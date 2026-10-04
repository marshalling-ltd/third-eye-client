//! UDP telemetry generator that mimics the ROV status broadcast.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

pub const DEFAULT_TARGET_HOST: &str = "127.0.0.1";
pub const DEFAULT_TARGET_PORT: u16 = 8500;
pub const DEFAULT_INTERVAL_MS: u64 = 200;

const PACKET_ID_STATUS: u8 = 0x03;
const PACKET_TYPE_ROV_STATUS: u8 = 0x01;

/// Sends synthetic status packets to `destination` every `interval` until
/// `stop` is set (pass a never-set flag to run forever).
pub fn run(destination: SocketAddr, interval: Duration, stop: &AtomicBool) -> Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .context("failed to bind local UDP socket for test server")?;
    socket
        .set_broadcast(true)
        .context("failed to enable UDP broadcast mode")?;

    let start = Instant::now();
    let mut seq: u64 = 0;
    while !stop.load(Ordering::Relaxed) {
        let status = build_test_status(start.elapsed(), seq);
        let packet = build_packet(&status)?;
        socket
            .send_to(&packet, destination)
            .with_context(|| format!("failed to send UDP packet to {destination}"))?;
        seq = seq.saturating_add(1);
        thread::sleep(interval);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
    pub depth: f32,
    pub lat: i32,
    pub lon: i32,
    pub temperature: f32,
    pub batteries: Vec<Battery>,
    pub imu: Imu,
}

#[derive(Debug, Serialize)]
pub struct Battery {
    pub id: u8,
    #[serde(rename = "volt")]
    pub voltage_mv: u16,
    pub current: i16,
    #[serde(rename = "remain")]
    pub remaining_pct: u8,
}

#[allow(clippy::struct_field_names)]
#[derive(Debug, Serialize)]
pub struct Imu {
    #[serde(rename = "gx")]
    pub gyro_x: i16,
    #[serde(rename = "gy")]
    pub gyro_y: i16,
    #[serde(rename = "gz")]
    pub gyro_z: i16,
}

pub fn build_test_status(elapsed: Duration, seq: u64) -> Status {
    let t = elapsed.as_secs_f32();
    let pitch = 0.25 * (t * 1.3).sin();
    let roll = 0.35 * (t * 0.9).cos();
    let yaw = (t * 0.5).sin();
    let depth = 8.0 + (t * 0.4).sin() * 1.8;

    #[allow(clippy::cast_possible_truncation)]
    let lat = 451_234_567 + ((t * 8.0).sin() * 8_000.0) as i32;

    #[allow(clippy::cast_possible_truncation)]
    let lon = 161_234_567 + ((t * 7.0).cos() * 8_000.0) as i32;

    let temperature = 23.0 + (t * 0.2).sin() * 2.0;

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let battery_1_remain = (100_i32 - (seq as i32 % 100)).max(1) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let battery_2_remain = (95_i32 - (seq as i32 % 95)).max(1) as u8;

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
            #[allow(clippy::cast_possible_truncation)]
            gyro_x: (pitch * 100.0) as i16,
            #[allow(clippy::cast_possible_truncation)]
            gyro_y: (roll * 100.0) as i16,
            #[allow(clippy::cast_possible_truncation)]
            gyro_z: (yaw * 100.0) as i16,
        },
    }
}

pub fn build_packet(status: &Status) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(status).context("failed to serialize status JSON")?;
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.push(PACKET_ID_STATUS);
    packet.push(1);
    packet.extend_from_slice(&[0_u8, 0_u8]);
    packet.extend_from_slice(&u32::try_from(payload.len())?.to_le_bytes());
    packet.push(PACKET_TYPE_ROV_STATUS);
    packet.extend_from_slice(&[0_u8, 0_u8, 0_u8]);
    packet.extend_from_slice(&payload);
    Ok(packet)
}
