use std::env;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

const DEFAULT_TARGET_HOST: &str = "127.0.0.1";
const DEFAULT_TARGET_PORT: u16 = 8500;
const DEFAULT_INTERVAL_MS: u64 = 200;

const PACKET_ID_STATUS: u8 = 0x03;
const PACKET_TYPE_ROV_STATUS: u8 = 0x01;

#[derive(Debug)]
struct Config {
    target_host: String,
    target_port: u16,
    interval_ms: u64,
}

#[derive(Debug, Serialize)]
struct Status {
    pitch: f32,
    roll: f32,
    yaw: f32,
    depth: f32,
    lat: i32,
    lon: i32,
    temperature: f32,
    batteries: Vec<Battery>,
    imu: Imu,
}

#[derive(Debug, Serialize)]
struct Battery {
    id: u8,
    #[serde(rename = "volt")]
    voltage_mv: u16,
    current: i16,
    #[serde(rename = "remain")]
    remaining_pct: u8,
}

#[derive(Debug, Serialize)]
struct Imu {
    #[serde(rename = "gx")]
    gyro_x: i16,
    #[serde(rename = "gy")]
    gyro_y: i16,
    #[serde(rename = "gz")]
    gyro_z: i16,
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

    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .context("failed to bind local UDP socket for test server")?;
    socket
        .set_broadcast(true)
        .context("failed to enable UDP broadcast mode")?;

    println!(
        "Test UDP telemetry server started -> sending to {} every {}ms",
        destination, config.interval_ms
    );
    println!("Press Ctrl+C to stop.");

    let start = Instant::now();
    let mut seq: u64 = 0;
    loop {
        let status = build_test_status(start.elapsed(), seq);
        let packet = build_packet(&status)?;
        socket
            .send_to(&packet, destination)
            .with_context(|| format!("failed to send UDP packet to {destination}"))?;
        seq = seq.saturating_add(1);
        thread::sleep(Duration::from_millis(config.interval_ms));
    }
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

fn build_test_status(elapsed: Duration, seq: u64) -> Status {
    let t = elapsed.as_secs_f32();
    let pitch = 0.25 * (t * 1.3).sin();
    let roll = 0.35 * (t * 0.9).cos();
    let yaw = (t * 0.5).sin();
    let depth = 8.0 + (t * 0.4).sin() * 1.8;
    let lat = 451_234_567 + ((t * 8.0).sin() * 8_000.0) as i32;
    let lon = 161_234_567 + ((t * 7.0).cos() * 8_000.0) as i32;
    let temperature = 23.0 + (t * 0.2).sin() * 2.0;

    let battery_1_remain = ((100_i32 - (seq as i32 % 100)).max(1)) as u8;
    let battery_2_remain = ((95_i32 - (seq as i32 % 95)).max(1)) as u8;

    Status {
        pitch,
        roll,
        yaw,
        depth,
        lat,
        lon,
        temperature,
        batteries: vec![
            Battery {
                id: 1,
                voltage_mv: 16_200,
                current: -30,
                remaining_pct: battery_1_remain,
            },
            Battery {
                id: 2,
                voltage_mv: 15_980,
                current: -28,
                remaining_pct: battery_2_remain,
            },
        ],
        imu: Imu {
            gyro_x: (pitch * 100.0) as i16,
            gyro_y: (roll * 100.0) as i16,
            gyro_z: (yaw * 100.0) as i16,
        },
    }
}

fn build_packet(status: &Status) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(status).context("failed to serialize status JSON")?;
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.push(PACKET_ID_STATUS);
    packet.push(1);
    packet.extend_from_slice(&[0_u8, 0_u8]);
    packet.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    packet.push(PACKET_TYPE_ROV_STATUS);
    packet.extend_from_slice(&[0_u8, 0_u8, 0_u8]);
    packet.extend_from_slice(&payload);
    Ok(packet)
}
