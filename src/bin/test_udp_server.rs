use std::env;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::{Context, Result};
use third_eye_client::simulator::udp::{
    self, DEFAULT_INTERVAL_MS, DEFAULT_TARGET_HOST, DEFAULT_TARGET_PORT,
};

#[derive(Debug)]
struct Config {
    target_host: String,
    target_port: u16,
    interval_ms: u64,
}

fn main() -> Result<()> {
    let config = parse_config()?;
    let destination: SocketAddr = format!("{}:{}", config.target_host, config.target_port)
        .parse()
        .with_context(|| {
            format!(
                "invalid target address {}:{}",
                config.target_host, config.target_port
            )
        })?;

    println!(
        "Test UDP telemetry server started -> sending to {} every {}ms",
        destination, config.interval_ms
    );
    println!("Press Ctrl+C to stop.");

    udp::run(
        destination,
        Duration::from_millis(config.interval_ms),
        &AtomicBool::new(false),
    )
}

fn parse_config() -> Result<Config> {
    let mut target_host = DEFAULT_TARGET_HOST.to_owned();
    let mut target_port = DEFAULT_TARGET_PORT;
    let mut interval_ms = DEFAULT_INTERVAL_MS;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => {
                let value = args.next().context("missing value for --host")?;
                target_host = value;
            }
            "--port" => {
                let value = args.next().context("missing value for --port")?;
                target_port = value
                    .parse::<u16>()
                    .with_context(|| format!("invalid --port value: {value}"))?;
            }
            "--interval-ms" => {
                let value = args.next().context("missing value for --interval-ms")?;
                interval_ms = value
                    .parse::<u64>()
                    .with_context(|| format!("invalid --interval-ms value: {value}"))?;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => anyhow::bail!("unknown argument: {arg} (use --help for usage)"),
        }
    }

    if interval_ms == 0 {
        anyhow::bail!("--interval-ms must be > 0");
    }

    Ok(Config {
        target_host,
        target_port,
        interval_ms,
    })
}

fn print_help() {
    println!("Usage: cargo run --features test-tools --bin test_udp_server -- [options]");
    println!();
    println!("Options:");
    println!("  --host <HOST>           Destination host/IP (default: 127.0.0.1)");
    println!("  --port <PORT>           Destination UDP port (default: 8500)");
    println!("  --interval-ms <MILLIS>  Send interval in milliseconds (default: 200)");
    println!("  -h, --help              Show this help");
}
