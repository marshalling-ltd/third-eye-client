//! Records the ROV's RTSP stream untouched and keeps a timeline of telemetry
//! next to it.
//!
//! The video is copied as it arrives (`-c copy`), so there is no re-encode and
//! the quality is the ROV's own. Telemetry samples are stamped with the time
//! since recording started and, when recording stops, written as a sidecar
//! `.srt` (readable overlay) and `.telemetry.json` (machine-readable) and
//! muxed into the mp4 as a subtitle track.
//!
//! Video and telemetry arrive on separate clocks, so the stamps are the time
//! the app received each sample. [`Recorder::set_video_latency`] shifts the
//! telemetry to line it up with the video.

use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::rov_status::Status;

/// How long to wait for ffmpeg to finalise the mp4 after asking it to quit.
const FINALISE_TIMEOUT: Duration = Duration::from_secs(10);

/// One telemetry reading on the recording timeline.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Sample {
    /// Milliseconds since the recording started.
    pub t_ms: u64,
    pub depth_m: f32,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub roll_deg: f32,
    pub temperature_c: f32,
    pub battery_pct: Option<u8>,
    pub lat: f64,
    pub lon: f64,
}

impl Sample {
    #[must_use]
    pub fn from_status(t_ms: u64, status: &Status) -> Self {
        Self {
            t_ms,
            depth_m: status.depth,
            yaw_deg: status.yaw,
            pitch_deg: status.pitch,
            roll_deg: status.roll,
            temperature_c: status.temperature,
            battery_pct: status.batteries.iter().map(|b| b.remaining).min(),
            lat: f64::from(status.lat) / 1e7,
            lon: f64::from(status.lon) / 1e7,
        }
    }

    fn caption(&self) -> String {
        let battery = self
            .battery_pct
            .map_or_else(|| "--".to_owned(), |p| format!("{p}%"));
        format!(
            "Depth {:.1} m  Heading {:.0}°  Battery {battery}",
            self.depth_m,
            self.yaw_deg.rem_euclid(360.0)
        )
    }
}

/// `HH:MM:SS,mmm` as SubRip expects.
fn srt_time(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// Builds a SubRip file where each sample is shown until the next one, and
/// the last for `last_ms` more.
#[must_use]
pub fn samples_to_srt(samples: &[Sample], last_ms: u64) -> String {
    let mut out = String::new();
    for (i, sample) in samples.iter().enumerate() {
        let end = samples
            .get(i + 1)
            .map_or(sample.t_ms + last_ms, |next| next.t_ms);
        if end <= sample.t_ms {
            continue;
        }
        let _ = write!(
            out,
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_time(sample.t_ms),
            srt_time(end),
            sample.caption()
        );
    }
    out
}

/// ffmpeg arguments that copy `rtsp_url` into a crash-safe fragmented mp4.
fn record_args(rtsp_url: &str, out: &Path) -> Vec<String> {
    [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-rtsp_transport",
        "tcp",
        "-i",
        rtsp_url,
        "-map",
        "0:v",
        "-c",
        "copy",
        // Fragments are written as they arrive, so a crash or pulled cable
        // leaves a playable file instead of an unfinished one.
        "-movflags",
        "+frag_keyframe+empty_moov+default_base_moof",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain(std::iter::once(out.display().to_string()))
    .collect()
}

/// ffmpeg arguments that add `srt` to `video` as a text track.
fn mux_args(video: &Path, srt: &Path, out: &Path) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-i".into(),
        video.display().to_string(),
        "-i".into(),
        srt.display().to_string(),
        "-map".into(),
        "0".into(),
        "-map".into(),
        "1".into(),
        "-c".into(),
        "copy".into(),
        "-c:s".into(),
        "mov_text".into(),
        "-metadata:s:s:0".into(),
        "title=Telemetry".into(),
        out.display().to_string(),
    ]
}

fn ffmpeg_command(ffmpeg_bin: &Path, args: &[String]) -> Command {
    let mut command = Command::new(ffmpeg_bin);
    command.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Files produced by a finished recording.
#[derive(Clone, Debug)]
pub struct RecordingFiles {
    /// The mp4, with the telemetry subtitle track when muxing succeeded.
    pub video: PathBuf,
    pub srt: PathBuf,
    pub json: PathBuf,
    /// Set when the telemetry could not be muxed into the mp4; the video and
    /// sidecars are still good.
    pub mux_error: Option<String>,
}

/// A recording in progress.
pub struct Recorder {
    ffmpeg_bin: PathBuf,
    child: Child,
    started: Instant,
    video_path: PathBuf,
    samples: Vec<Sample>,
    latency_ms: u64,
    finalise_timeout: Duration,
}

impl Recorder {
    /// Starts copying `rtsp_url` into `video_path`.
    pub fn start(ffmpeg_bin: PathBuf, rtsp_url: &str, video_path: PathBuf) -> Result<Self> {
        if let Some(dir) = video_path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let child = ffmpeg_command(&ffmpeg_bin, &record_args(rtsp_url, &video_path))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to spawn ffmpeg for recording")?;
        Ok(Self {
            ffmpeg_bin,
            child,
            started: Instant::now(),
            video_path,
            samples: Vec::new(),
            latency_ms: 0,
            finalise_timeout: FINALISE_TIMEOUT,
        })
    }

    /// How far the video lags the telemetry. Telemetry is moved this much
    /// later so it lines up with what the video shows.
    pub fn set_video_latency(&mut self, latency: Duration) {
        self.latency_ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX);
    }

    /// Records the telemetry that just arrived.
    pub fn push(&mut self, status: &Status) {
        let elapsed = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.samples.push(Sample::from_status(
            elapsed.saturating_add(self.latency_ms),
            status,
        ));
    }

    /// `true` while ffmpeg is still running (it exits on its own if the
    /// stream drops).
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Stops ffmpeg, writes the sidecars and muxes the telemetry track.
    pub fn stop(mut self) -> Result<RecordingFiles> {
        // `q` makes ffmpeg finish the file cleanly; killing it would not.
        // Taking stdin also closes the pipe once the closure returns.
        let _ = self
            .child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(b"q\n"));
        let deadline = Instant::now() + self.finalise_timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        if !self.video_path.is_file() {
            bail!("ffmpeg produced no video file; the stream may not have connected");
        }

        let srt = self.video_path.with_extension("srt");
        let json = self.video_path.with_extension("telemetry.json");
        std::fs::write(&srt, samples_to_srt(&self.samples, 1000))
            .with_context(|| format!("cannot write {}", srt.display()))?;
        std::fs::write(&json, serde_json::to_vec_pretty(&self.samples)?)
            .with_context(|| format!("cannot write {}", json.display()))?;

        let mux_error = if self.samples.is_empty() {
            None
        } else {
            self.mux(&srt).err().map(|e| format!("{e:#}"))
        };
        Ok(RecordingFiles {
            video: self.video_path,
            srt,
            json,
            mux_error,
        })
    }

    fn mux(&self, srt: &Path) -> Result<()> {
        let tmp = self.video_path.with_extension("muxed.mp4");
        let status = ffmpeg_command(&self.ffmpeg_bin, &mux_args(&self.video_path, srt, &tmp))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to run ffmpeg to add the telemetry track")?;
        if !status.success() {
            let _ = std::fs::remove_file(&tmp);
            bail!("ffmpeg could not add the telemetry track ({status})");
        }
        std::fs::rename(&tmp, &self.video_path)
            .context("cannot replace the recording with the muxed file")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rov_status::{Battery, Imu};

    fn status(depth: f32, yaw: f32, batteries: &[u8]) -> Status {
        Status {
            pitch: 1.0,
            roll: 2.0,
            yaw,
            depth,
            lat: 457_000_000,
            lon: 160_000_000,
            temperature: 20.0,
            batteries: batteries
                .iter()
                .enumerate()
                .map(|(i, &remaining)| Battery {
                    id: i as u8,
                    voltage: 16_000,
                    current: -1000,
                    remaining,
                })
                .collect(),
            imu: Imu::default(),
        }
    }

    #[test]
    fn srt_time_formats_hours_minutes_seconds_millis() {
        assert_eq!(srt_time(0), "00:00:00,000");
        assert_eq!(srt_time(3_723_456), "01:02:03,456");
    }

    #[test]
    fn sample_uses_lowest_battery_and_scaled_position() {
        let s = Sample::from_status(10, &status(5.0, 90.0, &[80, 60]));
        assert_eq!(s.battery_pct, Some(60));
        assert!((s.lat - 45.7).abs() < 1e-9);
        assert!((s.lon - 16.0).abs() < 1e-9);
        assert_eq!(
            Sample::from_status(0, &status(0.0, 0.0, &[])).battery_pct,
            None
        );
    }

    #[test]
    fn srt_shows_each_sample_until_the_next() {
        let samples = [
            Sample::from_status(0, &status(1.0, 10.0, &[90])),
            Sample::from_status(500, &status(2.5, 370.0, &[])),
        ];
        let srt = samples_to_srt(&samples, 1000);
        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:00,500\nDepth 1.0 m  Heading 10°  Battery 90%\n\n\
             2\n00:00:00,500 --> 00:00:01,500\nDepth 2.5 m  Heading 10°  Battery --\n\n"
        );
    }

    #[test]
    fn srt_skips_samples_that_would_show_for_no_time() {
        let samples = [
            Sample::from_status(100, &status(1.0, 0.0, &[])),
            Sample::from_status(100, &status(2.0, 0.0, &[])),
        ];
        let srt = samples_to_srt(&samples, 1000);
        assert!(!srt.contains("Depth 1.0"));
        assert!(srt.contains("Depth 2.0"));
    }

    #[test]
    fn record_args_copy_video_into_a_fragmented_mp4() {
        let args = record_args("rtsp://127.0.0.1:9/s", Path::new("/tmp/a.mp4"));
        assert!(args.windows(2).any(|w| w == ["-c", "copy"]));
        assert!(args.iter().any(|a| a.contains("frag_keyframe")));
        assert_eq!(args.last().map(String::as_str), Some("/tmp/a.mp4"));
    }
}

/// Runs the recorder against a stand-in `ffmpeg` shell script, so the process
/// handling is covered without ffmpeg or an RTSP source.
#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use crate::rov_status::Imu;
    use std::os::unix::fs::PermissionsExt;

    fn status() -> Status {
        Status {
            pitch: 0.0,
            roll: 0.0,
            yaw: 90.0,
            depth: 4.0,
            lat: 0,
            lon: 0,
            temperature: 15.0,
            batteries: vec![],
            imu: Imu::default(),
        }
    }

    /// Writes a script that acts as ffmpeg. `record` is the shell run for the
    /// recording call (output path in `$out`), `mux` for the telemetry call.
    fn fake_ffmpeg(name: &str, record: &str, mux: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("te-rec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("ffmpeg");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nfor out; do :; done\ncase \"$*\" in\n*mov_text*) {mux} ;;\n*) {record} ;;\nesac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, dir.join("dive.mp4"))
    }

    const RECORD_OK: &str = "printf video > \"$out\"; read line; exit 0";
    const MUX_OK: &str = "printf muxed > \"$out\"; exit 0";

    #[test]
    fn records_writes_sidecars_and_muxes_the_telemetry_track() {
        let (ffmpeg, video) = fake_ffmpeg("ok", RECORD_OK, MUX_OK);
        let mut rec = Recorder::start(ffmpeg, "rtsp://x", video.clone()).unwrap();
        assert!(rec.is_running());
        rec.push(&status());
        std::thread::sleep(Duration::from_millis(20));
        assert!(rec.elapsed() >= Duration::from_millis(20));
        let files = rec.stop().unwrap();
        assert!(files.mux_error.is_none(), "{:?}", files.mux_error);
        assert_eq!(std::fs::read_to_string(&files.video).unwrap(), "muxed");
        assert!(
            std::fs::read_to_string(&files.srt)
                .unwrap()
                .contains("Depth 4.0 m")
        );
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&files.json).unwrap()).unwrap();
        assert_eq!(json.as_array().map(Vec::len), Some(1));
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }

    #[test]
    fn a_failed_mux_keeps_the_video_and_reports_the_error() {
        let (ffmpeg, video) = fake_ffmpeg("mux-fail", RECORD_OK, "exit 1");
        let mut rec = Recorder::start(ffmpeg, "rtsp://x", video.clone()).unwrap();
        rec.push(&status());
        let files = rec.stop().unwrap();
        assert!(files.mux_error.is_some());
        assert_eq!(std::fs::read_to_string(&files.video).unwrap(), "video");
        assert!(!files.video.with_extension("muxed.mp4").exists());
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }

    #[test]
    fn without_telemetry_the_mux_is_skipped() {
        let (ffmpeg, video) = fake_ffmpeg("no-samples", RECORD_OK, "exit 1");
        let rec = Recorder::start(ffmpeg, "rtsp://x", video.clone()).unwrap();
        let files = rec.stop().unwrap();
        assert!(files.mux_error.is_none());
        assert_eq!(std::fs::read_to_string(&files.video).unwrap(), "video");
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }

    #[test]
    fn video_latency_moves_telemetry_later() {
        let (ffmpeg, video) = fake_ffmpeg("latency", RECORD_OK, MUX_OK);
        let mut rec = Recorder::start(ffmpeg, "rtsp://x", video.clone()).unwrap();
        rec.set_video_latency(Duration::from_secs(2));
        rec.push(&status());
        assert!(rec.samples[0].t_ms >= 2000);
        let _ = rec.stop();
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }

    #[test]
    fn stop_fails_when_ffmpeg_never_wrote_a_file() {
        let (ffmpeg, video) = fake_ffmpeg("no-file", "exit 1", MUX_OK);
        let mut rec = Recorder::start(ffmpeg, "rtsp://x", video.clone()).unwrap();
        // The script exits at once, as ffmpeg does when the stream is down.
        let deadline = Instant::now() + Duration::from_secs(30);
        while rec.is_running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!rec.is_running());
        let err = rec.stop().unwrap_err();
        assert!(format!("{err:#}").contains("no video file"));
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }

    #[test]
    fn stop_kills_ffmpeg_that_ignores_the_quit_request() {
        let (ffmpeg, video) =
            fake_ffmpeg("stuck", "printf video > \"$out\"; exec sleep 30", MUX_OK);
        let mut rec = Recorder::start(ffmpeg, "rtsp://x", video.clone()).unwrap();
        // Let the script write its file before it is asked to stop.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !video.is_file() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        rec.finalise_timeout = Duration::from_millis(200);
        let started = Instant::now();
        let files = rec.stop().unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(std::fs::read_to_string(&files.video).unwrap(), "video");
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }

    #[test]
    fn start_fails_when_the_output_folder_cannot_be_created() {
        let blocker = std::env::temp_dir().join(format!("te-rec-blocker-{}", std::process::id()));
        std::fs::write(&blocker, "a file, not a folder").unwrap();
        let result = Recorder::start(
            PathBuf::from("ffmpeg"),
            "rtsp://x",
            blocker.join("sub").join("a.mp4"),
        );
        let err = result.err().expect("start should fail");
        assert!(format!("{err:#}").contains("cannot create"));
        let _ = std::fs::remove_file(blocker);
    }

    #[test]
    fn stop_reports_a_sidecar_that_cannot_be_written() {
        for (name, blocked) in [
            ("srt-blocked", "dive.srt"),
            ("json-blocked", "dive.telemetry.json"),
        ] {
            let (ffmpeg, video) = fake_ffmpeg(name, RECORD_OK, MUX_OK);
            let rec = Recorder::start(ffmpeg, "rtsp://x", video.clone()).unwrap();
            // A folder where the file should go makes the write fail.
            std::fs::create_dir(video.with_file_name(blocked)).unwrap();
            let err = rec.stop().unwrap_err();
            assert!(
                format!("{err:#}").contains("cannot write"),
                "{name}: {err:#}"
            );
            let _ = std::fs::remove_dir_all(video.parent().unwrap());
        }
    }

    #[test]
    fn a_path_without_a_parent_folder_is_not_created() {
        let (ffmpeg, dir) = fake_ffmpeg("no-parent", "exit 0", MUX_OK);
        let rec = Recorder::start(ffmpeg, "rtsp://x", PathBuf::from("/")).unwrap();
        assert!(rec.stop().is_err());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn start_fails_when_ffmpeg_cannot_be_run() {
        let video =
            std::env::temp_dir().join(format!("te-rec-missing-{}/a.mp4", std::process::id()));
        let result = Recorder::start(
            PathBuf::from("/nonexistent/ffmpeg"),
            "rtsp://x",
            video.clone(),
        );
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(video.parent().unwrap());
    }
}
