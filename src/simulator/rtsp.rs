//! RTSP video source. `mediamtx` acts as the RTSP server (ffmpeg cannot serve
//! RTSP on its own, `-rtsp_flags listen` is input-only) while ffmpeg loops a
//! sample file into it, so any number of clients can connect.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

pub const DEFAULT_RTSP_PORT: u16 = 8554;
pub const DEFAULT_RTSP_PATH: &str = "stream/0/0";

const RESTART_DELAY: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(200);
const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

pub struct RtspConfig {
    pub ffmpeg: PathBuf,
    pub mediamtx: PathBuf,
    pub file: PathBuf,
    pub bind_host: String,
    pub port: u16,
    pub path: String,
}

/// Kills and reaps the child on drop so a failing simulator never leaves
/// orphaned `mediamtx`/`ffmpeg` processes behind.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_server(config: &RtspConfig) -> Result<ChildGuard> {
    // mediamtx only accepts publishers on configured paths, hence the
    // `all_others` entry. Everything but RTSP is switched off.
    let yaml = format!(
        "logLevel: warn\nrtspAddress: \"{}:{}\"\nrtmp: no\nhls: no\nwebrtc: no\nsrt: no\napi: no\nmetrics: no\nplayback: no\npprof: no\npaths:\n  all_others:\n",
        config.bind_host, config.port
    );
    let conf_path = std::env::temp_dir().join(format!(
        "chasing-simulator-mediamtx-{}.yml",
        std::process::id()
    ));
    std::fs::write(&conf_path, yaml).context("failed to write mediamtx config")?;
    let child = Command::new(&config.mediamtx)
        .arg(&conf_path)
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "failed to start mediamtx at {} (install it, e.g. `brew install mediamtx`, or pass --mediamtx)",
                config.mediamtx.display()
            )
        });
    // mediamtx has read the file by the time its port opens; a leftover tmp
    // file on failure is harmless.
    Ok(ChildGuard(child?))
}

fn spawn_publisher(config: &RtspConfig) -> Result<ChildGuard> {
    let url = format!("rtsp://127.0.0.1:{}/{}", config.port, config.path);
    let child = Command::new(&config.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-re",
            "-stream_loop",
            "-1",
            "-i",
        ])
        .arg(&config.file)
        .args([
            "-an",
            "-c:v",
            "copy",
            "-f",
            "rtsp",
            "-rtsp_transport",
            "tcp",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start ffmpeg at {}", config.ffmpeg.display()))?;
    Ok(ChildGuard(child))
}

fn wait_for_port(port: u16, server: &mut ChildGuard, stop: &AtomicBool) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + SERVER_STARTUP_TIMEOUT;
    while !stop.load(Ordering::Relaxed) {
        if let Some(status) = server.0.try_wait()? {
            anyhow::bail!("mediamtx exited during startup ({status}); is port {port} in use?");
        }
        if TcpStream::connect_timeout(&addr, POLL_INTERVAL).is_ok() {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "mediamtx did not open port {port}"
        );
        thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}

/// Runs the RTSP server and keeps the ffmpeg publisher alive until `stop` is set.
pub fn run(config: &RtspConfig, stop: &AtomicBool) -> Result<()> {
    let mut server = spawn_server(config)?;
    wait_for_port(config.port, &mut server, stop)?;

    while !stop.load(Ordering::Relaxed) {
        let mut publisher = spawn_publisher(config)?;
        while !stop.load(Ordering::Relaxed) {
            if server.0.try_wait()?.is_some() {
                anyhow::bail!("mediamtx exited unexpectedly");
            }
            if publisher.0.try_wait()?.is_some() {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
        thread::sleep(RESTART_DELAY);
    }
    Ok(())
}

/// First `.mp4` file in `dir` (sorted by name).
pub fn find_sample_video(dir: &Path) -> Result<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read samples dir {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")))
        .collect();
    files.sort();
    files
        .into_iter()
        .next()
        .with_context(|| format!("no .mp4 files in {}", dir.display()))
}
