//! Measures what the interface-bound RTSP forwarder costs, with no GUI and no
//! real ROV: runs the same RTSP session directly and through the forwarder
//! against any RTSP source (usually `chasing-simulator`) and reports time to
//! first frame, frame rate, stalls and control-channel round-trip times.

use std::env;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::Url;
use serde::Serialize;
use third_eye_client::rtsp_forwarder::{RtspForwarder, start_url_forwarder};

const DEFAULT_RTSP_PORT: u16 = 554;
/// A gap between frames longer than this counts as a stall.
const STALL_GAP: Duration = Duration::from_millis(200);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
const RTT_IO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Direct,
    Forwarded,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Direct => "direct",
            Mode::Forwarded => "forwarded",
        }
    }
}

struct Config {
    url: String,
    interface: Option<String>,
    modes: Vec<Mode>,
    runs: u32,
    duration: Duration,
    rtt_samples: u32,
    ffmpeg: PathBuf,
    json_out: Option<PathBuf>,
}

#[derive(Serialize, Clone)]
struct StreamRun {
    /// Process start to first decoded JPEG; `None` if no frame arrived.
    ttff_ms: Option<f64>,
    frames: u64,
    fps: f64,
    max_gap_ms: f64,
    stalls: u64,
    bytes: u64,
}

#[derive(Serialize, Clone, Copy)]
struct Percentiles {
    samples: usize,
    p50_us: u64,
    p99_us: u64,
    max_us: u64,
}

#[derive(Serialize, Default)]
struct ForwarderReport {
    connections: u64,
    connect_failures: u64,
    bytes_to_rov: u64,
    bytes_from_rov: u64,
    write_p50_micros: u64,
    write_p99_micros: u64,
}

#[derive(Serialize)]
struct ModeReport {
    mode: &'static str,
    runs: Vec<StreamRun>,
    ttff_median_ms: Option<f64>,
    fps_median: Option<f64>,
    max_gap_ms: f64,
    stalls_total: u64,
    rtt: Option<Percentiles>,
    forwarder: Option<ForwarderReport>,
}

#[derive(Serialize)]
struct Report {
    url: String,
    interface: Option<String>,
    os: &'static str,
    arch: &'static str,
    runs: u32,
    duration_secs: u64,
    modes: Vec<ModeReport>,
}

fn main() -> Result<()> {
    let config = parse_config()?;
    let mut rtt: Vec<(Mode, Vec<Duration>)> =
        config.modes.iter().map(|m| (*m, Vec::new())).collect();
    let mut stream_runs: Vec<(Mode, Vec<StreamRun>)> =
        config.modes.iter().map(|m| (*m, Vec::new())).collect();
    let mut forwarder_totals: Vec<ForwarderReport> = config
        .modes
        .iter()
        .map(|_| ForwarderReport::default())
        .collect();

    println!(
        "Probing {} ({} run(s) x {}s per mode)",
        redact(&config.url),
        config.runs,
        config.duration.as_secs()
    );
    for run in 0..config.runs {
        // Alternate which mode goes first so network warm-up favours neither.
        let mut order: Vec<usize> = (0..config.modes.len()).collect();
        if run % 2 == 1 {
            order.reverse();
        }
        for index in order {
            let mode = config.modes[index];
            let (session_url, guard) = open_session(&config, mode)?;
            println!("  run {}/{} {} ...", run + 1, config.runs, mode.label());

            match measure_rtt(&session_url, config.rtt_samples) {
                Ok(samples) => rtt[index].1.extend(samples),
                Err(err) => eprintln!("    RTT probe failed: {err:#}"),
            }
            match stream_run(&config, &session_url) {
                Ok(result) => stream_runs[index].1.push(result),
                Err(err) => eprintln!("    stream run failed: {err:#}"),
            }
            if let Some(guard) = guard {
                let stats = guard.stats();
                let total = &mut forwarder_totals[index];
                total.connections += stats.connections;
                total.connect_failures += stats.connect_failures;
                total.bytes_to_rov += stats.bytes_to_rov;
                total.bytes_from_rov += stats.bytes_from_rov;
                total.write_p50_micros = total.write_p50_micros.max(stats.write_p50_micros);
                total.write_p99_micros = total.write_p99_micros.max(stats.write_p99_micros);
            }
        }
    }

    let mut modes = Vec::new();
    for (index, mode) in config.modes.iter().enumerate() {
        let runs = std::mem::take(&mut stream_runs[index].1);
        let rtt_samples = std::mem::take(&mut rtt[index].1);
        let forwarder =
            (*mode == Mode::Forwarded).then(|| std::mem::take(&mut forwarder_totals[index]));
        modes.push(summarize(*mode, runs, &rtt_samples, forwarder));
    }
    let report = Report {
        url: redact(&config.url),
        interface: config.interface.clone(),
        os: env::consts::OS,
        arch: env::consts::ARCH,
        runs: config.runs,
        duration_secs: config.duration.as_secs(),
        modes,
    };

    print_summary(&report);
    if let Some(path) = &config.json_out {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        println!("\nJSON written to {}", path.display());
    }
    Ok(())
}

/// Returns the URL to stream from and, for forwarded mode, the live forwarder.
fn open_session(config: &Config, mode: Mode) -> Result<(String, Option<RtspForwarder>)> {
    match mode {
        Mode::Direct => Ok((config.url.clone(), None)),
        Mode::Forwarded => {
            let started =
                start_url_forwarder(&config.url, config.interface.as_deref(), DEFAULT_RTSP_PORT)?;
            let Some((local_url, forwarder)) = started else {
                bail!(
                    "forwarded mode needs --interface and a non-loopback ROV address \
                     (loopback sources are never forwarded)"
                );
            };
            Ok((local_url, Some(forwarder)))
        }
    }
}

/// Round-trip time of `OPTIONS` requests on one RTSP connection: pure
/// control-channel latency, the clearest view of what a proxy hop adds.
fn measure_rtt(url: &str, samples: u32) -> Result<Vec<Duration>> {
    if samples == 0 {
        return Ok(Vec::new());
    }
    let mut parsed = Url::parse(url).context("invalid RTSP URL")?;
    let host = parsed
        .host_str()
        .context("RTSP URL has no host")?
        .to_owned();
    let port = parsed.port().unwrap_or(DEFAULT_RTSP_PORT);
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);

    let mut stream = TcpStream::connect((host.as_str(), port))
        .with_context(|| format!("cannot connect to {host}:{port}"))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(RTT_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(RTT_IO_TIMEOUT))?;

    let mut out = Vec::with_capacity(samples as usize);
    let mut buf = [0_u8; 2048];
    for cseq in 1..=samples {
        let request = format!(
            "OPTIONS {parsed} RTSP/1.0\r\nCSeq: {cseq}\r\nUser-Agent: forwarder-probe\r\n\r\n"
        );
        let started = Instant::now();
        stream.write_all(request.as_bytes())?;
        let mut response = Vec::new();
        while !response.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                bail!("connection closed while waiting for an RTSP response");
            }
            response.extend_from_slice(&buf[..n]);
        }
        out.push(started.elapsed());
    }
    Ok(out)
}

/// Runs ffmpeg the way the app does and watches the JPEG frames it produces.
fn stream_run(config: &Config, url: &str) -> Result<StreamRun> {
    let started = Instant::now();
    let mut child = Command::new(&config.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error"])
        .args([
            "-rtsp_transport",
            "tcp",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
        ])
        .arg("-i")
        .arg(url)
        .args([
            "-vf",
            "fps=15,scale=960:-1",
            "-f",
            "mjpeg",
            "-q:v",
            "6",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start {}", config.ffmpeg.display()))?;
    let frames = spawn_frame_reader(&mut child);

    let result = collect_frames(&frames, started, config.duration);
    let _ = child.kill();
    let _ = child.wait();
    Ok(result)
}

/// Streams `(arrival time, frame size)` for every complete JPEG on ffmpeg's stdout.
fn spawn_frame_reader(child: &mut Child) -> Receiver<(Instant, usize)> {
    let (tx, rx) = mpsc::channel();
    let mut stdout = child.stdout.take().expect("stdout is piped");
    thread::spawn(move || {
        let mut chunk = [0_u8; 16 * 1024];
        let mut pending: Vec<u8> = Vec::new();
        while let Ok(n) = stdout.read(&mut chunk) {
            if n == 0 {
                break;
            }
            pending.extend_from_slice(&chunk[..n]);
            while let Some(len) = take_jpeg(&mut pending) {
                if tx.send((Instant::now(), len)).is_err() {
                    return;
                }
            }
        }
    });
    rx
}

/// Removes the first complete JPEG (SOI..EOI) from `buffer`, returning its size.
fn take_jpeg(buffer: &mut Vec<u8>) -> Option<usize> {
    let start = buffer.windows(2).position(|p| p == [0xFF, 0xD8])?;
    let end = buffer[start + 2..]
        .windows(2)
        .position(|p| p == [0xFF, 0xD9])?
        + start
        + 4;
    let len = end - start;
    buffer.drain(..end);
    Some(len)
}

fn collect_frames(
    frames: &Receiver<(Instant, usize)>,
    started: Instant,
    duration: Duration,
) -> StreamRun {
    let Ok((first_at, first_len)) = frames.recv_timeout(FIRST_FRAME_TIMEOUT) else {
        return StreamRun {
            ttff_ms: None,
            frames: 0,
            fps: 0.0,
            max_gap_ms: 0.0,
            stalls: 0,
            bytes: 0,
        };
    };
    let deadline = first_at + duration;
    let mut arrivals = vec![first_at];
    let mut bytes = first_len as u64;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match frames.recv_timeout(remaining) {
            Ok((at, len)) => {
                arrivals.push(at);
                bytes += len as u64;
            }
            Err(_) => break,
        }
    }
    // A stream that dies early shows up as a final gap up to the deadline.
    let last = *arrivals.last().expect("at least one frame");
    let tail_gap = deadline.saturating_duration_since(last);

    let mut max_gap = tail_gap;
    let mut stalls = u64::from(tail_gap > STALL_GAP);
    for pair in arrivals.windows(2) {
        let gap = pair[1] - pair[0];
        max_gap = max_gap.max(gap);
        stalls += u64::from(gap > STALL_GAP);
    }
    let span = (last - first_at).as_secs_f64();
    StreamRun {
        ttff_ms: Some((first_at - started).as_secs_f64() * 1000.0),
        frames: arrivals.len() as u64,
        fps: if span > 0.0 {
            f64::from(u32::try_from(arrivals.len() - 1).unwrap_or(u32::MAX)) / span
        } else {
            0.0
        },
        max_gap_ms: max_gap.as_secs_f64() * 1000.0,
        stalls,
        bytes,
    }
}

fn summarize(
    mode: Mode,
    runs: Vec<StreamRun>,
    rtt: &[Duration],
    forwarder: Option<ForwarderReport>,
) -> ModeReport {
    let ttffs: Vec<f64> = runs.iter().filter_map(|r| r.ttff_ms).collect();
    let fps: Vec<f64> = runs
        .iter()
        .filter(|r| r.frames > 1)
        .map(|r| r.fps)
        .collect();
    ModeReport {
        mode: mode.label(),
        ttff_median_ms: median(&ttffs),
        fps_median: median(&fps),
        max_gap_ms: runs.iter().map(|r| r.max_gap_ms).fold(0.0, f64::max),
        stalls_total: runs.iter().map(|r| r.stalls).sum(),
        rtt: percentiles(rtt),
        forwarder,
        runs,
    }
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 0 {
        f64::midpoint(sorted[mid - 1], sorted[mid])
    } else {
        sorted[mid]
    })
}

fn percentiles(samples: &[Duration]) -> Option<Percentiles> {
    if samples.is_empty() {
        return None;
    }
    let mut micros: Vec<u64> = samples
        .iter()
        .map(|d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
        .collect();
    micros.sort_unstable();
    let at = |percent: usize| micros[(micros.len() * percent / 100).min(micros.len() - 1)];
    Some(Percentiles {
        samples: micros.len(),
        p50_us: at(50),
        p99_us: at(99),
        max_us: *micros.last().expect("non-empty"),
    })
}

fn print_summary(report: &Report) {
    println!(
        "\n{} / {}  interface: {}",
        report.os,
        report.arch,
        report.interface.as_deref().unwrap_or("-")
    );
    println!(
        "{:<10} {:>12} {:>7} {:>11} {:>7} {:>10} {:>10}",
        "mode", "ttff median", "fps", "max gap", "stalls", "rtt p50", "rtt p99"
    );
    for m in &report.modes {
        println!(
            "{:<10} {:>12} {:>7} {:>11} {:>7} {:>10} {:>10}",
            m.mode,
            fmt_opt(m.ttff_median_ms, "ms", 0),
            fmt_opt(m.fps_median, "", 1),
            format!("{:.0} ms", m.max_gap_ms),
            m.stalls_total,
            m.rtt.map_or("-".to_owned(), |r| format!("{} us", r.p50_us)),
            m.rtt.map_or("-".to_owned(), |r| format!("{} us", r.p99_us)),
        );
    }
    let direct = report.modes.iter().find(|m| m.mode == "direct");
    let forwarded = report.modes.iter().find(|m| m.mode == "forwarded");
    if let (Some(d), Some(f)) = (direct, forwarded) {
        println!("\nforwarded minus direct:");
        if let (Some(a), Some(b)) = (f.ttff_median_ms, d.ttff_median_ms) {
            println!("  time to first frame  {:+.0} ms", a - b);
        }
        if let (Some(a), Some(b)) = (f.fps_median, d.fps_median) {
            println!("  frame rate           {:+.1} fps", a - b);
        }
        if let (Some(a), Some(b)) = (f.rtt, d.rtt) {
            println!(
                "  RTT p50 / p99        {:+} / {:+} us",
                signed(a.p50_us, b.p50_us),
                signed(a.p99_us, b.p99_us)
            );
        }
        println!(
            "  extra stalls         {:+}",
            i128::from(f.stalls_total) - i128::from(d.stalls_total)
        );
    }
    if let Some(f) = forwarded.and_then(|m| m.forwarder.as_ref()) {
        println!(
            "\nforwarder: {} connection(s), {} failed, write overhead p50 <= {} us / p99 <= {} us",
            f.connections, f.connect_failures, f.write_p50_micros, f.write_p99_micros
        );
    }
}

fn signed(a: u64, b: u64) -> i128 {
    i128::from(a) - i128::from(b)
}

fn fmt_opt(value: Option<f64>, unit: &str, decimals: usize) -> String {
    value.map_or("-".to_owned(), |v| {
        format!("{v:.decimals$} {unit}").trim_end().to_owned()
    })
}

/// Hides credentials so reports are safe to paste into a ticket.
fn redact(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut parsed) => {
            if !parsed.username().is_empty() || parsed.password().is_some() {
                let _ = parsed.set_username("***");
                let _ = parsed.set_password(None);
            }
            parsed.to_string()
        }
        Err(_) => "<invalid url>".to_owned(),
    }
}

fn parse_config() -> Result<Config> {
    let mut url = None;
    let mut interface = None;
    let mut modes = vec![Mode::Direct, Mode::Forwarded];
    let mut runs = 5;
    let mut duration_secs = 10;
    let mut rtt_samples = 200;
    let mut ffmpeg = PathBuf::from("ffmpeg");
    let mut json_out = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .with_context(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "--url" => url = Some(value("--url")?),
            "--interface" => interface = Some(value("--interface")?),
            "--mode" => {
                modes = match value("--mode")?.as_str() {
                    "direct" => vec![Mode::Direct],
                    "forwarded" => vec![Mode::Forwarded],
                    "both" => vec![Mode::Direct, Mode::Forwarded],
                    other => bail!("unknown --mode {other} (use direct, forwarded or both)"),
                };
            }
            "--runs" => runs = value("--runs")?.parse().context("invalid --runs")?,
            "--duration-secs" => {
                duration_secs = value("--duration-secs")?
                    .parse()
                    .context("invalid --duration-secs")?;
            }
            "--rtt-samples" => {
                rtt_samples = value("--rtt-samples")?
                    .parse()
                    .context("invalid --rtt-samples")?;
            }
            "--ffmpeg" => ffmpeg = value("--ffmpeg")?.into(),
            "--json-out" => json_out = Some(value("--json-out")?.into()),
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => bail!("unknown argument: {arg} (use --help for usage)"),
        }
    }
    let url = url.context("--url is required (use --help for usage)")?;
    if runs == 0 || duration_secs == 0 {
        bail!("--runs and --duration-secs must be > 0");
    }
    if modes.contains(&Mode::Forwarded) && interface.is_none() {
        bail!("--interface is required for forwarded mode (e.g. --interface en0)");
    }
    Ok(Config {
        url,
        interface,
        modes,
        runs,
        duration: Duration::from_secs(duration_secs),
        rtt_samples,
        ffmpeg,
        json_out,
    })
}

fn print_help() {
    println!(
        "Usage: cargo run --release --features test-tools --bin forwarder-probe -- --url <RTSP_URL> --interface <IFACE> [options]"
    );
    println!();
    println!("Runs the same RTSP session directly and through the interface-bound forwarder");
    println!("and compares time to first frame, frame rate, stalls and RTSP round-trip times.");
    println!();
    println!("Options:");
    println!(
        "  --url <URL>             RTSP source, e.g. rtsp://admin:admin@192.168.1.88:8554/stream/0/0"
    );
    println!(
        "  --interface <IFACE>     Interface the forwarder pins to (en0, eth1, ...); required unless --mode direct"
    );
    println!("  --mode <MODE>           direct | forwarded | both (default: both)");
    println!("  --runs <N>              Runs per mode, alternating order (default: 5)");
    println!(
        "  --duration-secs <S>     Streaming time per run after the first frame (default: 10)"
    );
    println!("  --rtt-samples <N>       OPTIONS round trips per run (default: 200)");
    println!("  --ffmpeg <PATH>         ffmpeg binary (default: ffmpeg from PATH)");
    println!("  --json-out <FILE>       Also write the full report as JSON");
    println!("  -h, --help              Show this help");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend_from_slice(payload);
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    #[test]
    fn take_jpeg_extracts_frames_and_keeps_the_remainder() {
        let mut buffer = vec![0x00, 0x01];
        buffer.extend(jpeg(&[1, 2, 3]));
        buffer.extend(jpeg(&[4]));
        buffer.extend_from_slice(&[0xFF, 0xD8, 9]);
        assert_eq!(take_jpeg(&mut buffer), Some(7));
        assert_eq!(take_jpeg(&mut buffer), Some(5));
        assert_eq!(take_jpeg(&mut buffer), None);
        assert_eq!(buffer, vec![0xFF, 0xD8, 9]);
    }

    #[test]
    fn median_handles_empty_odd_and_even() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
    }

    #[test]
    fn percentiles_report_p50_p99_and_max() {
        assert!(percentiles(&[]).is_none());
        let samples: Vec<Duration> = (1..=100).map(Duration::from_micros).collect();
        let p = percentiles(&samples).unwrap();
        assert_eq!(
            (p.samples, p.p50_us, p.p99_us, p.max_us),
            (100, 51, 100, 100)
        );
    }

    #[test]
    fn redact_hides_credentials_only_when_present() {
        assert_eq!(
            redact("rtsp://admin:secret@10.0.0.1:8554/s"),
            "rtsp://***@10.0.0.1:8554/s"
        );
        assert_eq!(redact("rtsp://10.0.0.1:8554/s"), "rtsp://10.0.0.1:8554/s");
        assert_eq!(redact("not a url"), "<invalid url>");
    }

    #[test]
    fn signed_and_fmt_opt_format_values() {
        assert_eq!(signed(5, 8), -3);
        assert_eq!(fmt_opt(Some(12.345), "ms", 1), "12.3 ms");
        assert_eq!(fmt_opt(Some(2.0), "", 0), "2");
        assert_eq!(fmt_opt(None, "ms", 0), "-");
    }

    #[test]
    fn collect_frames_computes_rate_gaps_and_stalls() {
        let started = Instant::now();
        let (tx, rx) = mpsc::channel();
        let t0 = started + Duration::from_millis(500);
        // 4 frames: 100 ms apart, then a 300 ms gap (one stall).
        for ms in [0, 100, 200, 500] {
            tx.send((t0 + Duration::from_millis(ms), 1000)).unwrap();
        }
        drop(tx);
        let run = collect_frames(&rx, started, Duration::from_millis(500));
        assert_eq!(run.frames, 4);
        assert_eq!(run.bytes, 4000);
        assert_eq!(run.stalls, 1);
        assert!(
            (run.max_gap_ms - 300.0).abs() < 1.0,
            "gap {}",
            run.max_gap_ms
        );
        assert!((run.fps - 6.0).abs() < 0.1, "fps {}", run.fps);
        assert!(run.ttff_ms.unwrap() >= 499.0);
    }

    #[test]
    fn collect_frames_with_no_frames_reports_no_first_frame() {
        let (tx, rx) = mpsc::channel::<(Instant, usize)>();
        drop(tx);
        let run = collect_frames(&rx, Instant::now(), Duration::from_millis(10));
        assert_eq!((run.ttff_ms, run.frames), (None, 0));
    }

    #[test]
    fn summarize_builds_medians_and_totals() {
        let run = |ttff: f64, fps: f64, stalls: u32| StreamRun {
            ttff_ms: Some(ttff),
            frames: 10,
            fps,
            max_gap_ms: 50.0 + f64::from(stalls),
            stalls: u64::from(stalls),
            bytes: 1,
        };
        let report = summarize(
            Mode::Forwarded,
            vec![run(100.0, 15.0, 0), run(300.0, 14.0, 2)],
            &[Duration::from_micros(10), Duration::from_micros(30)],
            Some(ForwarderReport::default()),
        );
        assert_eq!(report.mode, "forwarded");
        assert_eq!(report.ttff_median_ms, Some(200.0));
        assert_eq!(report.stalls_total, 2);
        assert!((report.max_gap_ms - 52.0).abs() < f64::EPSILON);
        assert_eq!(report.rtt.unwrap().samples, 2);
        assert!(report.forwarder.is_some());
    }

    #[test]
    fn measure_rtt_times_options_round_trips_against_a_fake_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 512];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0
                    || stream
                        .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n")
                        .is_err()
                {
                    break;
                }
            }
        });
        let url = format!("rtsp://user:pw@127.0.0.1:{port}/stream");
        assert_eq!(measure_rtt(&url, 5).unwrap().len(), 5);
        assert_eq!(
            measure_rtt(&url, 0).unwrap(),
            [] as [std::time::Duration; 0]
        );
    }

    #[test]
    fn measure_rtt_fails_when_the_server_closes_the_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || drop(listener.accept().unwrap()));
        let err = measure_rtt(&format!("rtsp://127.0.0.1:{port}/s"), 1).unwrap_err();
        assert!(format!("{err:#}").contains("closed") || format!("{err:#}").contains("reset"));
    }

    #[test]
    fn open_session_direct_keeps_the_url_and_forwarded_needs_a_remote_host() {
        let config = |url: &str| Config {
            url: url.to_owned(),
            interface: Some("en0".to_owned()),
            modes: vec![Mode::Forwarded],
            runs: 1,
            duration: Duration::from_secs(1),
            rtt_samples: 0,
            ffmpeg: PathBuf::from("ffmpeg"),
            json_out: None,
        };
        let (url, guard) = open_session(&config("rtsp://10.0.0.1/s"), Mode::Direct).unwrap();
        assert_eq!((url.as_str(), guard.is_none()), ("rtsp://10.0.0.1/s", true));
        let Err(err) = open_session(&config("rtsp://127.0.0.1:8554/s"), Mode::Forwarded) else {
            panic!("loopback must not be forwarded");
        };
        assert!(format!("{err:#}").contains("--interface"));
    }
}
