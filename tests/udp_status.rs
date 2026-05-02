use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};

use third_eye_client::rov_status::{UdpStatusState, parse_status_packet};

const TEST_PACKET_ID: u8 = 0x03;
const TEST_PACKET_TYPE: u8 = 0x01;
const TEST_PACKET_HEADER_SIZE: usize = 12;

fn sample_status_json() -> String {
    serde_json::json!({
        "pitch": 0.12_f32,
        "roll": -0.34_f32,
        "yaw": 1.57_f32,
        "depth": 12.5_f32,
        "lat": 451234567_i32,
        "lon": 161234567_i32,
        "temperature": 23.4_f32,
        "batteries": [
            {
                "id": 1_u8,
                "volt": 16200_u16,
                "current": -30_i16,
                "remain": 81_u8
            }
        ],
        "imu": {
            "gx": 11_i16,
            "gy": -12_i16,
            "gz": 13_i16
        }
    })
    .to_string()
}

fn build_status_packet(payload_json: &str, little_endian_len: bool) -> Vec<u8> {
    let payload = payload_json.as_bytes();
    let mut packet = Vec::with_capacity(TEST_PACKET_HEADER_SIZE + payload.len());
    packet.push(TEST_PACKET_ID);
    packet.push(1_u8);
    packet.extend_from_slice(&[0_u8, 0_u8]);
    let payload_len = payload.len() as u32;
    if little_endian_len {
        packet.extend_from_slice(&payload_len.to_le_bytes());
    } else {
        packet.extend_from_slice(&payload_len.to_be_bytes());
    }
    packet.push(TEST_PACKET_TYPE);
    packet.extend_from_slice(&[0_u8, 0_u8, 0_u8]);
    packet.extend_from_slice(payload);
    packet
}

fn reserve_udp_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("should reserve local UDP port");
    socket
        .local_addr()
        .expect("reserved socket should have address")
        .port()
}

#[test]
fn parse_status_packet_deserializes_expected_values() {
    let payload_json = sample_status_json();
    let packet = build_status_packet(&payload_json, true);

    let status = parse_status_packet(&packet).expect("packet should parse");
    assert!((status.pitch - 0.12).abs() < f32::EPSILON);
    assert!((status.roll + 0.34).abs() < f32::EPSILON);
    assert!((status.yaw - 1.57).abs() < f32::EPSILON);
    assert!((status.depth - 12.5).abs() < f32::EPSILON);
    assert_eq!(status.lat, 451_234_567);
    assert_eq!(status.lon, 161_234_567);
    assert!((status.temperature - 23.4).abs() < f32::EPSILON);
    assert_eq!(status.batteries.len(), 1);
    assert_eq!(status.batteries[0].id, 1);
    assert_eq!(status.batteries[0].voltage, 16_200);
    assert_eq!(status.batteries[0].current, -30);
    assert_eq!(status.batteries[0].remaining, 81);
    assert_eq!(status.imu.gyro_x, 11);
    assert_eq!(status.imu.gyro_y, -12);
    assert_eq!(status.imu.gyro_z, 13);
}

#[test]
fn parse_status_packet_supports_big_endian_payload_length() {
    let payload_json = sample_status_json();
    let packet = build_status_packet(&payload_json, false);

    let status = parse_status_packet(&packet).expect("packet should parse");
    assert_eq!(status.lat, 451_234_567);
    assert_eq!(status.lon, 161_234_567);
}

#[test]
fn udp_listener_receives_and_deserializes_status() {
    let port = reserve_udp_port();
    let mut listener = UdpStatusState::default();
    listener
        .start("127.0.0.1", port, None)
        .expect("listener should start on reserved port");

    let sender = UdpSocket::bind("127.0.0.1:0").expect("sender should bind");
    let packet = build_status_packet(&sample_status_json(), true);
    sender
        .send_to(&packet, format!("127.0.0.1:{port}"))
        .expect("sender should send packet");

    let start = Instant::now();
    while listener.packets_received() == 0 && start.elapsed() < Duration::from_secs(2) {
        listener.poll_events();
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        listener.packets_received() > 0,
        "no packet was received in time"
    );
    let status = listener
        .latest_status()
        .expect("latest status should be populated");
    assert_eq!(status.lat, 451_234_567);
    assert_eq!(status.lon, 161_234_567);
    assert!((status.depth - 12.5).abs() < f32::EPSILON);
    assert_eq!(status.batteries[0].remaining, 81);

    listener.stop();
}
