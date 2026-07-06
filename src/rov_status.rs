use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};

/// The UDP port the ROV broadcasts status packets on.
pub const ROV_STATUS_UDP_PORT: u16 = 8500;
const ROV_STATUS_PACKET_ID: u8 = 0x03;
const ROV_STATUS_PACKET_TYPE: u8 = 0x01;
const ROV_STATUS_PACKET_HEADER_SIZE: usize = 12;

/// A single ROV status reading, decoded from a status UDP packet's JSON payload.
///
/// # Examples
///
/// ```
/// use third_eye_client::rov_status::{Imu, Status};
///
/// let status = Status {
///     pitch: 0.1,
///     roll: -0.2,
///     yaw: 3.0,
///     depth: 12.5,
///     lat: 45_123_456,
///     lon: 16_123_456,
///     temperature: 22.0,
///     batteries: Vec::new(),
///     imu: Imu::default(),
/// };
/// println!("depth: {} m", status.depth);
/// ```
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Status {
    /// Pitch angle in degrees.
    #[serde(rename = "pitch")]
    pub pitch: f32,
    /// Roll angle in degrees.
    #[serde(rename = "roll")]
    pub roll: f32,
    /// Yaw angle in degrees.
    #[serde(rename = "yaw")]
    pub yaw: f32,
    /// Depth in meters.
    #[serde(rename = "depth")]
    pub depth: f32,
    /// Latitude in degrees, scaled by 1e7 (as reported by the ROV).
    #[serde(rename = "lat")]
    pub lat: i32,
    /// Longitude in degrees, scaled by 1e7 (as reported by the ROV).
    #[serde(rename = "lon")]
    pub lon: i32,
    /// Internal temperature in degrees Celsius.
    #[serde(rename = "temperature")]
    pub temperature: f32,
    /// Battery readings; empty if the ROV did not report any.
    #[serde(rename = "batteries", default)]
    pub batteries: Vec<Battery>,
    /// Gyroscope reading; defaults to zero if the ROV did not report one.
    #[serde(rename = "imu", default)]
    pub imu: Imu,
}

/// A single battery's voltage, current, and remaining charge, as reported by the ROV.
///
/// # Examples
///
/// ```
/// use third_eye_client::rov_status::Battery;
///
/// let battery = Battery {
///     id: 0,
///     voltage: 16_800,
///     current: -1_250,
///     remaining: 87,
/// };
/// assert_eq!(battery.remaining, 87);
/// ```
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Battery {
    /// Battery index, as assigned by the ROV.
    #[serde(rename = "id")]
    pub id: u8,
    /// Voltage in millivolts.
    #[serde(rename = "volt")]
    pub voltage: u16,
    /// Current in milliamps; negative when discharging.
    #[serde(rename = "current")]
    pub current: i16,
    /// Remaining charge as a percentage (0-100).
    #[serde(rename = "remain")]
    pub remaining: u8,
}

/// Raw gyroscope readings from the ROV's IMU.
///
/// # Examples
///
/// ```
/// use third_eye_client::rov_status::Imu;
///
/// let imu = Imu::default();
/// assert_eq!(imu.gyro_x, 0);
///
/// let imu = Imu { gyro_x: 10, gyro_y: -5, gyro_z: 2 };
/// assert_eq!(imu.gyro_z, 2);
/// ```
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Imu {
    /// Angular rate around the X axis.
    #[serde(rename = "gx")]
    pub gyro_x: i16,
    /// Angular rate around the Y axis.
    #[serde(rename = "gy")]
    pub gyro_y: i16,
    /// Angular rate around the Z axis.
    #[serde(rename = "gz")]
    pub gyro_z: i16,
}

enum UdpStatusEvent {
    Status(Status),
    Error(String),
    Ended,
}

/// Tracks a background UDP listener for ROV status broadcasts and the latest
/// decoded [`Status`], polled from the UI thread via [`UdpStatusState::poll_events`].
///
/// # Examples
///
/// ```
/// use third_eye_client::rov_status::UdpStatusState;
///
/// let mut state = UdpStatusState::default();
/// assert!(!state.is_running());
/// assert_eq!(state.packets_received(), 0);
/// assert!(state.latest_status().is_none());
/// ```
#[derive(Default)]
pub struct UdpStatusState {
    event_rx: Option<Receiver<UdpStatusEvent>>,
    controller: Option<UdpStatusController>,
    latest_status: Option<Status>,
    status: String,
    packets_received: u64,
}

impl UdpStatusState {
    /// Binds a UDP socket and spawns a background thread that decodes incoming
    /// ROV status packets, replacing any previously running listener.
    ///
    /// # Arguments
    ///
    /// * `bind_host` - The local address to bind to (e.g. `"0.0.0.0"` or `"127.0.0.1"`).
    /// * `port` - The local UDP port to bind to.
    /// * `interface` - An optional network interface name to restrict the socket to.
    ///
    /// # Returns
    ///
    /// * `Result<String>` - A human-readable status message on success.
    ///
    /// # Errors
    ///
    /// Returns an error if `bind_host` is empty, or if the socket cannot be bound.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let mut state = UdpStatusState::default();
    /// let message = state.start("127.0.0.1", 0, None).unwrap();
    /// assert!(message.contains("Listening"));
    /// assert!(state.is_running());
    ///
    /// state.stop();
    /// assert!(!state.is_running());
    /// ```
    pub fn start(&mut self, bind_host: &str, port: u16, interface: Option<&str>) -> Result<String> {
        let bind_host = bind_host.trim();
        if bind_host.is_empty() {
            anyhow::bail!("UDP bind host cannot be empty");
        }
        let bind_addr = format!("{bind_host}:{port}");
        let socket = create_bound_udp_socket(bind_host, port, interface)
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
        self.status = format!("Listening for UDP ROV status broadcasts on {bind_addr}.");

        Ok(self.status.clone())
    }

    /// Stops the background listener thread, if one is running. Safe to call
    /// even if no listener is running.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let mut state = UdpStatusState::default();
    /// state.start("127.0.0.1", 0, None).unwrap();
    ///
    /// state.stop();
    /// assert!(!state.is_running());
    /// assert_eq!(state.status_text(), "ROV status listener stopped.");
    /// ```
    pub fn stop(&mut self) {
        if let Some(mut controller) = self.controller.take() {
            controller.stop();
            self.status = "ROV status listener stopped.".to_string();
        }
        self.event_rx = None;
    }

    /// Returns whether a background listener thread is currently running.
    ///
    /// # Returns
    ///
    /// * `bool` - `true` if [`UdpStatusState::start`] succeeded and
    ///   [`UdpStatusState::stop`] has not since been called.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let state = UdpStatusState::default();
    /// assert!(!state.is_running());
    /// ```
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.controller.is_some()
    }

    /// Drains events from the background listener thread, updating the latest
    /// status, packet count, and status text. Call this periodically from the
    /// UI thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let mut state = UdpStatusState::default();
    /// state.start("127.0.0.1", 0, None).unwrap();
    ///
    /// // No packets have arrived yet, but polling is always safe.
    /// state.poll_events();
    /// assert_eq!(state.packets_received(), 0);
    ///
    /// state.stop();
    /// ```
    pub fn poll_events(&mut self) {
        let mut disconnected = false;
        if let Some(rx) = &self.event_rx {
            loop {
                match rx.try_recv() {
                    Ok(UdpStatusEvent::Status(status)) => {
                        self.latest_status = Some(status);
                        self.packets_received = self.packets_received.saturating_add(1);
                        self.status = "Receiving ROV status packets.".to_string();
                    }
                    Ok(UdpStatusEvent::Error(err)) => {
                        self.status = err;
                    }
                    Ok(UdpStatusEvent::Ended) | Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
        }

        if disconnected {
            self.controller = None;
            self.event_rx = None;
            if self.status.trim().is_empty() || self.status == "Receiving ROV status packets." {
                self.status = "ROV status listener ended.".to_string();
            }
        }
    }

    /// Returns the current human-readable status message.
    ///
    /// # Returns
    ///
    /// * `&str` - The current status message.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let mut state = UdpStatusState::default();
    /// state.start("127.0.0.1", 0, None).unwrap();
    /// assert_eq!(
    ///     state.status_text(),
    ///     "Listening for UDP ROV status broadcasts on 127.0.0.1:0."
    /// );
    /// state.stop();
    /// ```
    #[must_use]
    pub fn status_text(&self) -> &str {
        &self.status
    }

    /// Overwrites the current status message.
    ///
    /// # Arguments
    ///
    /// * `text` - The new status message to display.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let mut state = UdpStatusState::default();
    /// state.set_status_text("Waiting for configuration.".to_string());
    /// assert_eq!(state.status_text(), "Waiting for configuration.");
    /// ```
    pub fn set_status_text(&mut self, text: String) {
        self.status = text;
    }

    /// Returns the number of status packets successfully received and decoded
    /// since the listener was last started.
    ///
    /// # Returns
    ///
    /// * `u64` - The number of packets received.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let state = UdpStatusState::default();
    /// assert_eq!(state.packets_received(), 0);
    /// ```
    #[must_use]
    pub fn packets_received(&self) -> u64 {
        self.packets_received
    }

    /// Returns the most recently decoded [`Status`], if any packets have been
    /// received yet.
    ///
    /// # Returns
    ///
    /// * `Option<&Status>` - The latest status, or `None` if no packet has
    ///   arrived since the listener was started.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::rov_status::UdpStatusState;
    ///
    /// let state = UdpStatusState::default();
    /// assert!(state.latest_status().is_none());
    /// ```
    #[must_use]
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
    let mut datagram = vec![0_u8; 65_507].into_boxed_slice();
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
                    "UDP receive failed on port {ROV_STATUS_UDP_PORT}: {err}"
                )));
                let _ = tx.send(UdpStatusEvent::Ended);
                return;
            }
        }
    }
    let _ = tx.send(UdpStatusEvent::Ended);
}

/// Creates a UDP socket optionally bound to a specific network interface.
///
/// On macOS this sets `IP_BOUND_IF` via `socket2` so the socket only sends
/// and receives on the named interface — no host routes or ARP hacks needed.
fn create_bound_udp_socket(
    bind_host: &str,
    port: u16,
    interface: Option<&str>,
) -> Result<UdpSocket> {
    let addr: std::net::SocketAddr = format!("{bind_host}:{port}")
        .parse()
        .with_context(|| format!("invalid bind address {bind_host}:{port}"))?;
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create UDP socket")?;
    socket
        .set_reuse_address(true)
        .context("failed to set SO_REUSEADDR")?;

    if let Some(iface) = interface {
        bind_socket_to_interface(&socket, iface)?;
    }

    socket
        .bind(&addr.into())
        .with_context(|| format!("failed to bind UDP {addr}"))?;
    Ok(socket.into())
}

/// Binds a `socket2::Socket` to a named network interface.
///
/// On macOS/iOS this uses `IP_BOUND_IF` (via `bind_device_by_index_v4`).
/// On Linux this uses `SO_BINDTODEVICE` (via `bind_device`).
#[allow(unused_variables)] // `iface` unused on unsupported platforms
fn bind_socket_to_interface(socket: &Socket, iface: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let index = interface_name_to_index(iface)?;
        socket
            .bind_device_by_index_v4(Some(index))
            .with_context(|| format!("IP_BOUND_IF failed for interface {iface} (index {index})"))?;
    }
    #[cfg(target_os = "linux")]
    {
        socket
            .bind_device(Some(iface.as_bytes()))
            .with_context(|| format!("SO_BINDTODEVICE failed for interface {iface}"))?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Interface binding is not supported on this platform (e.g. Windows).
        // Bind to 0.0.0.0 instead — the socket will receive on all interfaces.
        let _ = iface;
    }
    Ok(())
}

/// Resolves a network interface name (e.g. `"en10"`) to its OS index.
///
/// No doctest is provided since valid interface names are specific to the
/// host machine running the docs build.
#[cfg(target_os = "macos")]
pub fn interface_name_to_index(name: &str) -> Result<std::num::NonZeroU32> {
    let c_name =
        std::ffi::CString::new(name).context("interface name contains interior NUL byte")?;
    // SAFETY: `if_nametoindex` is a POSIX function that accepts a C string.
    let index = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
    if index == 0 {
        anyhow::bail!(
            "interface {name:?} not found (if_nametoindex returned 0, errno={})",
            std::io::Error::last_os_error()
        );
    }
    Ok(std::num::NonZeroU32::new(index)
        .expect("if_nametoindex returned non-zero but NonZeroU32::new failed"))
}

/// Parses a raw ROV status UDP datagram into a [`Status`].
///
/// The wire format is a 12-byte header followed by a JSON payload:
/// `[id: u8, _: u8, _: u8, _: u8, payload_len: u32, type: u8, _: u8, _: u8, _: u8, payload: [u8]]`,
/// where `id` must be `0x03` and `type` must be `0x01`. `payload_len` is
/// accepted as either little- or big-endian, whichever is consistent with the
/// datagram's actual length.
///
/// # Arguments
///
/// * `datagram` - The raw bytes received from the UDP socket.
///
/// # Returns
///
/// * `Result<Status>` - The decoded status on success.
///
/// # Errors
///
/// Returns an error if the datagram is shorter than the header, has an
/// unexpected packet id or type, the payload length doesn't fit the datagram,
/// or the payload is not valid JSON for [`Status`].
///
/// # Examples
///
/// ```
/// use third_eye_client::rov_status::parse_status_packet;
///
/// let payload = br#"{"pitch":0.0,"roll":0.0,"yaw":0.0,"depth":0.0,"lat":0,"lon":0,"temperature":20.0}"#;
///
/// let mut packet = vec![0x03_u8, 0x01, 0x00, 0x00];
/// packet.extend_from_slice(&(payload.len() as u32).to_le_bytes());
/// packet.push(0x01); // payload type
/// packet.extend_from_slice(&[0x00, 0x00, 0x00]);
/// packet.extend_from_slice(payload);
///
/// let status = parse_status_packet(&packet).unwrap();
/// assert!((status.temperature - 20.0).abs() < f32::EPSILON);
/// ```
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_status_json() -> String {
        serde_json::json!({
            "pitch": 0.0_f32, "roll": 0.0_f32, "yaw": 0.0_f32,
            "depth": 0.0_f32, "lat": 0_i32, "lon": 0_i32,
            "temperature": 20.0_f32
        })
        .to_string()
    }

    fn build_packet(payload_json: &str) -> Vec<u8> {
        let payload = payload_json.as_bytes();
        let mut packet = Vec::with_capacity(ROV_STATUS_PACKET_HEADER_SIZE + payload.len());
        packet.push(ROV_STATUS_PACKET_ID);
        packet.push(1);
        packet.extend_from_slice(&[0, 0]);
        packet.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        packet.push(ROV_STATUS_PACKET_TYPE);
        packet.extend_from_slice(&[0, 0, 0]);
        packet.extend_from_slice(payload);
        packet
    }

    #[test]
    fn parse_minimal_valid_packet() {
        let packet = build_packet(&minimal_status_json());
        let status = parse_status_packet(&packet).unwrap();
        assert!((status.temperature - 20.0).abs() < f32::EPSILON);
        assert!(status.batteries.is_empty());
    }

    #[test]
    fn parse_truncated_packet() {
        assert!(parse_status_packet(&[0x03, 0x00]).is_err());
    }

    #[test]
    fn parse_empty_packet() {
        assert!(parse_status_packet(&[]).is_err());
    }

    #[test]
    fn parse_wrong_packet_id() {
        let mut packet = build_packet(&minimal_status_json());
        packet[0] = 0xFF;
        assert!(parse_status_packet(&packet).is_err());
    }

    #[test]
    fn parse_wrong_packet_type() {
        let mut packet = build_packet(&minimal_status_json());
        packet[8] = 0xFF;
        assert!(parse_status_packet(&packet).is_err());
    }

    #[test]
    fn parse_missing_optional_fields() {
        let json = serde_json::json!({
            "pitch": 0.1_f32, "roll": 0.2_f32, "yaw": 1.0_f32,
            "depth": 5.0_f32, "lat": 451_234_567_i32, "lon": 161_234_567_i32,
            "temperature": 22.5_f32
        })
        .to_string();
        let packet = build_packet(&json);
        let status = parse_status_packet(&packet).unwrap();
        assert_eq!(status.lat, 451_234_567);
        assert!(status.batteries.is_empty());
        assert_eq!(status.imu.gyro_x, 0);
    }

    #[test]
    fn parse_payload_length_mismatch() {
        let mut packet = build_packet(&minimal_status_json());
        packet[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_status_packet(&packet).is_err());
    }
}
