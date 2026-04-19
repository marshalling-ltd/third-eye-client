use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const ROV_STATUS_UDP_PORT: u16 = 8500;
const ROV_STATUS_PACKET_ID: u8 = 0x03;
const ROV_STATUS_PACKET_TYPE: u8 = 0x01;
const ROV_STATUS_PACKET_HEADER_SIZE: usize = 12;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Status {
    #[serde(rename = "pitch")]
    pub pitch: f32,
    #[serde(rename = "roll")]
    pub roll: f32,
    #[serde(rename = "yaw")]
    pub yaw: f32,
    #[serde(rename = "depth")]
    pub depth: f32,
    #[serde(rename = "lat")]
    pub lat: i32,
    #[serde(rename = "lon")]
    pub lon: i32,
    #[serde(rename = "temperature")]
    pub temperature: f32,
    #[serde(rename = "batteries", default)]
    pub batteries: Vec<Battery>,
    #[serde(rename = "imu", default)]
    pub imu: Imu,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Battery {
    #[serde(rename = "id")]
    pub id: u8,
    #[serde(rename = "volt")]
    pub voltage: u16,
    #[serde(rename = "current")]
    pub current: i16,
    #[serde(rename = "remain")]
    pub remaining: u8,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Imu {
    #[serde(rename = "gx")]
    pub gyro_x: i16,
    #[serde(rename = "gy")]
    pub gyro_y: i16,
    #[serde(rename = "gz")]
    pub gyro_z: i16,
}

enum UdpStatusEvent {
    Status(Status),
    Error(String),
    Ended,
}

#[derive(Default)]
pub struct UdpStatusState {
    event_rx: Option<Receiver<UdpStatusEvent>>,
    controller: Option<UdpStatusController>,
    latest_status: Option<Status>,
    status: String,
    packets_received: u64,
}

impl UdpStatusState {
    pub fn start(&mut self, bind_host: &str, port: u16) -> Result<String> {
        let bind_host = bind_host.trim();
        if bind_host.is_empty() {
            anyhow::bail!("UDP bind host cannot be empty");
        }
        let bind_addr = format!("{bind_host}:{port}");
        let socket = UdpSocket::bind((bind_host, port))
            .with_context(|| format!("failed to bind UDP {bind_addr}"))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .context("failed to set UDP read timeout")?;

        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop_flag = Arc::clone(&stop_flag);
        let worker = thread::spawn(move || udp_status_worker_loop(socket, worker_stop_flag, tx));

        self.event_rx = Some(rx);
        self.controller = Some(UdpStatusController {
            stop_flag,
            worker: Some(worker),
        });
        self.latest_status = None;
        self.packets_received = 0;
        self.status = format!("Listening for UDP ROV status broadcasts on {}.", bind_addr);

        Ok(self.status.clone())
    }

    pub fn stop(&mut self) {
        if let Some(mut controller) = self.controller.take() {
            controller.stop();
            self.status = "ROV status listener stopped.".to_owned();
        }
        self.event_rx = None;
    }

    pub fn is_running(&self) -> bool {
        self.controller.is_some()
    }

    pub fn poll_events(&mut self) {
        let mut disconnected = false;
        if let Some(rx) = &self.event_rx {
            loop {
                match rx.try_recv() {
                    Ok(UdpStatusEvent::Status(status)) => {
                        self.latest_status = Some(status);
                        self.packets_received = self.packets_received.saturating_add(1);
                        self.status = "Receiving ROV status packets.".to_owned();
                    }
                    Ok(UdpStatusEvent::Error(err)) => {
                        self.status = err;
                    }
                    Ok(UdpStatusEvent::Ended) => {
                        disconnected = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if disconnected {
            self.controller = None;
            self.event_rx = None;
            if self.status.trim().is_empty() || self.status == "Receiving ROV status packets." {
                self.status = "ROV status listener ended.".to_owned();
            }
        }
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn set_status_text(&mut self, text: String) {
        self.status = text;
    }

    pub fn packets_received(&self) -> u64 {
        self.packets_received
    }

    pub fn latest_status(&self) -> Option<&Status> {
        self.latest_status.as_ref()
    }
}

struct UdpStatusController {
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl UdpStatusController {
    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for UdpStatusController {
    fn drop(&mut self) {
        self.stop();
    }
}

fn udp_status_worker_loop(
    socket: UdpSocket,
    stop_flag: Arc<AtomicBool>,
    tx: mpsc::Sender<UdpStatusEvent>,
) {
    let mut datagram = [0_u8; 65_507];
    while !stop_flag.load(Ordering::Relaxed) {
        match socket.recv_from(&mut datagram) {
            Ok((bytes_received, _source)) => match parse_status_packet(&datagram[..bytes_received])
            {
                Ok(status) => {
                    if tx.send(UdpStatusEvent::Status(status)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    if tx
                        .send(UdpStatusEvent::Error(format!(
                            "Failed to parse status packet: {err:#}"
                        )))
                        .is_err()
                    {
                        return;
                    }
                }
            },
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut => {}
            Err(err) => {
                let _ = tx.send(UdpStatusEvent::Error(format!(
                    "UDP receive failed on port {}: {}",
                    ROV_STATUS_UDP_PORT, err
                )));
                let _ = tx.send(UdpStatusEvent::Ended);
                return;
            }
        }
    }
    let _ = tx.send(UdpStatusEvent::Ended);
}

pub fn parse_status_packet(datagram: &[u8]) -> Result<Status> {
    if datagram.len() < ROV_STATUS_PACKET_HEADER_SIZE {
        anyhow::bail!(
            "packet too short: got {} bytes, need at least {}",
            datagram.len(),
            ROV_STATUS_PACKET_HEADER_SIZE
        );
    }
    let packet_id = datagram[0];
    if packet_id != ROV_STATUS_PACKET_ID {
        anyhow::bail!(
            "unexpected packet id 0x{packet_id:02x} (expected 0x{ROV_STATUS_PACKET_ID:02x})"
        );
    }
    let payload_type = datagram[8];
    if payload_type != ROV_STATUS_PACKET_TYPE {
        anyhow::bail!(
            "unexpected packet type 0x{payload_type:02x} (expected 0x{ROV_STATUS_PACKET_TYPE:02x})"
        );
    }

    let payload_len_le = u32::from_le_bytes([datagram[4], datagram[5], datagram[6], datagram[7]]);
    let payload_len_be = u32::from_be_bytes([datagram[4], datagram[5], datagram[6], datagram[7]]);
    let payload_len = if ROV_STATUS_PACKET_HEADER_SIZE + payload_len_le as usize <= datagram.len() {
        payload_len_le as usize
    } else if ROV_STATUS_PACKET_HEADER_SIZE + payload_len_be as usize <= datagram.len() {
        payload_len_be as usize
    } else {
        anyhow::bail!(
            "payload length mismatch: header says le={}, be={}, datagram={}",
            payload_len_le,
            payload_len_be,
            datagram.len()
        );
    };

    let payload =
        &datagram[ROV_STATUS_PACKET_HEADER_SIZE..(ROV_STATUS_PACKET_HEADER_SIZE + payload_len)];
    serde_json::from_slice(payload).context("invalid JSON payload for ROV status")
}
