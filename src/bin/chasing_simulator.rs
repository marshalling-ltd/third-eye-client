use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use third_eye_client::simulator::http::{self, DEFAULT_HTTP_PORT};
use third_eye_client::simulator::rtsp::{self, DEFAULT_RTSP_PATH, DEFAULT_RTSP_PORT, RtspConfig};
use third_eye_client::simulator::udp::{
    self, DEFAULT_INTERVAL_MS, DEFAULT_TARGET_HOST, DEFAULT_TARGET_PORT,
};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Lets the worker threads wind down (and reap mediamtx/ffmpeg) on Ctrl+C or SIGTERM.
fn install_signal_handlers() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

struct Config {
    udp_target: String,
    udp_port: u16,
    interval_ms: u64,
    bind: String,
    rtsp_port: u16,
    http_port: u16,
    samples: PathBuf,
    video: Option<PathBuf>,
    ffmpeg: PathBuf,
    mediamtx: PathBuf,
}

fn main() -> Result<()> {
    let config = parse_config()?;
    let udp_dest: SocketAddr = format!("{}:{}", config.udp_target, config.udp_port)
        .parse()
        .with_context(|| {
            format!(
                "invalid UDP target {}:{}",
                config.udp_target, config.udp_port
            )
        })?;
    let video = match &config.video {
        Some(v) => v.clone(),
        None => rtsp::find_sample_video(&config.samples)?,
    };

    println!("Chasing ROV simulator");
    println!(
        "  UDP telemetry -> {udp_dest} every {}ms",
        config.interval_ms
    );
    println!(
        "  RTSP          -> rtsp://admin:admin@127.0.0.1:{}/{DEFAULT_RTSP_PATH}  ({})",
        config.rtsp_port,
        video.display()
    );
    println!(
        "  HTTP camera   -> http://127.0.0.1:{}  (media from {})",
        config.http_port,
        config.samples.display()
    );
    println!("Press Ctrl+C to stop.");

    install_signal_handlers();
    let interval = Duration::from_millis(config.interval_ms);
    let rtsp_config = RtspConfig {
        ffmpeg: config.ffmpeg.clone(),
        mediamtx: config.mediamtx.clone(),
        file: video,
        bind_host: config.bind.clone(),
        port: config.rtsp_port,
        path: DEFAULT_RTSP_PATH.to_owned(),
    };

    let tasks: Vec<(&str, thread::JoinHandle<Result<()>>)> = vec![
        (
            "udp",
            thread::spawn(move || udp::run(udp_dest, interval, &STOP)),
        ),
        (
            "rtsp",
            thread::spawn(move || rtsp::run(&rtsp_config, &STOP)),
        ),
        (
            "http",
            thread::spawn(move || {
                http::run(&config.bind, config.http_port, &config.samples, &STOP)
            }),
        ),
    ];

    // Any task failing (e.g. port in use, ffmpeg missing) takes the whole simulator down.
    while !STOP.load(Ordering::Relaxed) {
        if let Some(name) = tasks.iter().find(|(_, h)| h.is_finished()).map(|(n, _)| *n) {
            STOP.store(true, Ordering::Relaxed);
            let mut failure = None;
            for (n, handle) in tasks {
                if let Ok(Err(e)) = handle.join() {
                    failure.get_or_insert(e.context(format!("{n} task failed")));
                }
            }
            return Err(
                failure.unwrap_or_else(|| anyhow::anyhow!("{name} task exited unexpectedly"))
            );
        }
        thread::sleep(Duration::from_millis(200));
    }
    for (_, handle) in tasks {
        let _ = handle.join();
    }
    println!("Stopped.");
    Ok(())
}

fn parse_config() -> Result<Config> {
    let mut c = Config {
        udp_target: DEFAULT_TARGET_HOST.to_owned(),
        udp_port: DEFAULT_TARGET_PORT,
        interval_ms: DEFAULT_INTERVAL_MS,
        bind: "0.0.0.0".to_owned(),
        rtsp_port: DEFAULT_RTSP_PORT,
        http_port: DEFAULT_HTTP_PORT,
        samples: PathBuf::from("samples"),
        video: None,
        ffmpeg: PathBuf::from("ffmpeg"),
        mediamtx: PathBuf::from("mediamtx"),
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .with_context(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "--udp-target" => c.udp_target = value("--udp-target")?,
            "--udp-port" => {
                c.udp_port = value("--udp-port")?.parse().context("invalid --udp-port")?;
            }
            "--interval-ms" => {
                c.interval_ms = value("--interval-ms")?
                    .parse()
                    .context("invalid --interval-ms")?;
            }
            "--bind" => c.bind = value("--bind")?,
            "--rtsp-port" => {
                c.rtsp_port = value("--rtsp-port")?
                    .parse()
                    .context("invalid --rtsp-port")?;
            }
            "--http-port" => {
                c.http_port = value("--http-port")?
                    .parse()
                    .context("invalid --http-port")?;
            }
            "--samples" => c.samples = value("--samples")?.into(),
            "--video" => c.video = Some(value("--video")?.into()),
            "--mediamtx" => c.mediamtx = value("--mediamtx")?.into(),
            "--ffmpeg" => c.ffmpeg = value("--ffmpeg")?.into(),
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => anyhow::bail!("unknown argument: {arg} (use --help for usage)"),
        }
    }
    if c.interval_ms == 0 {
        anyhow::bail!("--interval-ms must be > 0");
    }
    Ok(c)
}

fn print_help() {
    println!("Usage: cargo run --features test-tools --bin chasing-simulator -- [options]");
    println!();
    println!("Simulates a Chasing ROV: UDP telemetry, RTSP video and the camera HTTP API.");
    println!();
    println!("Options:");
    println!("  --udp-target <HOST>    UDP telemetry destination (default: {DEFAULT_TARGET_HOST})");
    println!("  --udp-port <PORT>      UDP telemetry port (default: {DEFAULT_TARGET_PORT})");
    println!("  --interval-ms <MS>     Telemetry interval (default: {DEFAULT_INTERVAL_MS})");
    println!("  --bind <ADDR>          Bind address for RTSP/HTTP (default: 0.0.0.0)");
    println!("  --rtsp-port <PORT>     RTSP port (default: {DEFAULT_RTSP_PORT})");
    println!("  --http-port <PORT>     HTTP port (default: {DEFAULT_HTTP_PORT})");
    println!("  --samples <DIR>        Samples directory (default: samples)");
    println!("  --video <FILE>         Video to stream over RTSP (default: first .mp4 in samples)");
    println!("  --ffmpeg <PATH>        ffmpeg binary (default: ffmpeg from PATH)");
    println!(
        "  --mediamtx <PATH>      mediamtx binary used as RTSP server (default: mediamtx from PATH)"
    );
    println!("  -h, --help             Show this help");
}
