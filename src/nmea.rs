//! Receives GPS coordinates from a phone app over TCP or Bluetooth.
//!
//! **TCP mode** (GPS2IP, `GPSd` Forwarder, `ShareGPS`): the laptop listens on
//! a TCP port and the phone app connects as a client, streaming NMEA sentences.
//!
//! **Bluetooth / serial mode** (Bluetooth GPS, GPS Share, any SPP app): the
//! Android phone is paired via Bluetooth and its RFCOMM/SPP channel appears
//! as a virtual serial port (`/dev/cu.*` on macOS, `COM*` on Windows). This
//! module opens the port at 9600 baud and reads NMEA sentences directly.
//!
//! Both modes share the same [`NmeaGpsState`] — only one can run at a time.

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};

/// Default TCP port used by GPS2IP (both iOS and Android).
pub const DEFAULT_NMEA_GPS_PORT: u16 = 11123;

/// Baud rate used by the vast majority of Bluetooth GPS apps on Android
/// (GPS Share, Bluetooth GPS, etc.). Classic NMEA-0183 is 4800 baud but
/// modern Android apps default to 9600.
pub const DEFAULT_BT_BAUD_RATE: u32 = 9600;

// ---------------------------------------------------------------------------
// Serial port enumeration
// ---------------------------------------------------------------------------

/// Returns all virtual serial / Bluetooth SPP port names visible to the OS.
///
/// On macOS the paired Bluetooth device shows up as `/dev/cu.<DeviceName>`.
/// On Windows it appears as a `COM` port number. The list is unsorted.
pub fn list_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}

/// Returns only Bluetooth SPP serial port names, filtered per-platform:
///
/// - **Windows**: ports tagged `SerialPortType::BluetoothPort` by the driver.
/// - **macOS**: `/dev/cu.*` ports that are *not* `PciPort` (built-in). BT SPP
///   often shows as `Unknown` on macOS so we can't rely on the type alone.
/// - **Linux**: `/dev/rfcomm*` ports (BT RFCOMM channels).
///
/// Falls back to the full `list_serial_ports()` on unsupported platforms.
pub fn list_bluetooth_ports() -> Vec<String> {
    let ports = serialport::available_ports().unwrap_or_default();
    let filtered: Vec<String> = ports
        .into_iter()
        .filter(is_likely_bluetooth_port)
        .map(|p| p.port_name)
        .collect();
    filtered
}

fn is_likely_bluetooth_port(port: &serialport::SerialPortInfo) -> bool {
    use serialport::SerialPortType;
    match &port.port_type {
        SerialPortType::BluetoothPort => true,
        #[cfg(target_os = "macos")]
        SerialPortType::Unknown => {
            // On macOS, /dev/cu.* Bluetooth SPP ports show as Unknown.
            // Exclude /dev/cu.usbmodem* and /dev/cu.usbserial* (USB adapters).
            let name = &port.port_name;
            name.starts_with("/dev/cu.")
                && !name.contains("usbmodem")
                && !name.contains("usbserial")
                && !name.contains("Bluetooth-Incoming")
        }
        #[cfg(target_os = "linux")]
        SerialPortType::Unknown => port.port_name.starts_with("/dev/rfcomm"),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Public state
// ---------------------------------------------------------------------------

enum NmeaGpsEvent {
    Fix { lat: f64, lon: f64 },
    Status(String),
    Error(String),
    Ended,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[derive(Default)]
pub struct NmeaGpsState {
    event_rx: Option<Receiver<NmeaGpsEvent>>,
    controller: Option<NmeaGpsController>,
    latest_fix: Option<(f64, f64)>,
    status: String,
    fixes_received: u64,
    /// Unix-ms timestamp of the most recent Fix event.
    last_fix_at_ms: i64,
}

impl NmeaGpsState {
    /// Opens a Bluetooth (SPP) or wired serial port and starts reading NMEA
    /// sentences. `port_path` is e.g. `/dev/cu.GPS-SPPSlave` (macOS) or
    /// `COM5` (Windows). The baud rate defaults to [`DEFAULT_BT_BAUD_RATE`].
    pub fn start_bluetooth(&mut self, port_path: &str) -> Result<String> {
        let port_path = port_path.trim().to_owned();
        if port_path.is_empty() {
            anyhow::bail!("Serial port path must not be empty.");
        }
        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_flag);
        let path_clone = port_path.clone();
        let worker = thread::Builder::new()
            .name("nmea-bt".into())
            .spawn(move || nmea_serial_worker(path_clone, DEFAULT_BT_BAUD_RATE, worker_stop, tx))
            .context("failed to spawn Bluetooth GPS worker thread")?;

        self.event_rx = Some(rx);
        self.controller = Some(NmeaGpsController {
            stop_flag,
            worker: Some(worker),
        });
        self.latest_fix = None;
        self.fixes_received = 0;
        self.last_fix_at_ms = 0;
        self.status = format!("Connecting to Bluetooth GPS on {port_path}...");
        Ok(self.status.clone())
    }

    /// Connects as a TCP **client** to a phone running an NMEA server app.
    /// The laptop dials `host:port` and reads NMEA sentences from the
    /// connection. Automatically retries on disconnect.
    pub fn start_client(&mut self, host: &str, port: u16) -> Result<String> {
        let host = host.trim();
        if host.is_empty() {
            anyhow::bail!("Phone server host must not be empty.");
        }
        let addr = format!("{host}:{port}");
        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_flag);
        let addr_clone = addr.clone();
        let worker = thread::Builder::new()
            .name("nmea-tcp-client".into())
            .spawn(move || nmea_tcp_client_worker(addr_clone, worker_stop, tx))
            .context("failed to spawn NMEA TCP client worker thread")?;

        self.event_rx = Some(rx);
        self.controller = Some(NmeaGpsController {
            stop_flag,
            worker: Some(worker),
        });
        self.latest_fix = None;
        self.fixes_received = 0;
        self.last_fix_at_ms = 0;
        self.status = format!("Connecting to phone GPS server at {addr}...");
        Ok(self.status.clone())
    }

    /// Starts a TCP **server** (listener) that accepts connections from phone
    /// GPS apps like GPS2IP.
    pub fn start(&mut self, host: &str, port: u16) -> Result<String> {
        let host = host.trim();
        let bind_addr = if host.is_empty() {
            format!("0.0.0.0:{port}")
        } else {
            format!("{host}:{port}")
        };
        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_flag);
        let addr_clone = bind_addr.clone();
        let worker = thread::Builder::new()
            .name("nmea-gps".into())
            .spawn(move || nmea_tcp_worker(addr_clone, worker_stop, tx))
            .context("failed to spawn NMEA GPS worker thread")?;

        self.event_rx = Some(rx);
        self.controller = Some(NmeaGpsController {
            stop_flag,
            worker: Some(worker),
        });
        self.latest_fix = None;
        self.fixes_received = 0;
        self.last_fix_at_ms = 0;
        self.status = format!("Listening for phone GPS on {bind_addr}...");

        Ok(self.status.clone())
    }

    pub fn stop(&mut self) {
        if let Some(mut controller) = self.controller.take() {
            controller.stop();
            self.status = "NMEA GPS listener stopped.".to_owned();
        }
        self.event_rx = None;
        self.fixes_received = 0;
        self.last_fix_at_ms = 0;
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.controller.is_some()
    }

    /// Drains pending events from the worker thread. Call from the UI timer.
    pub fn poll_events(&mut self) -> bool {
        let mut got_fix = false;
        let mut disconnected = false;
        if let Some(rx) = &self.event_rx {
            loop {
                match rx.try_recv() {
                    Ok(NmeaGpsEvent::Fix { lat, lon }) => {
                        self.latest_fix = Some((lat, lon));
                        self.fixes_received = self.fixes_received.saturating_add(1);
                        self.last_fix_at_ms = now_ms();
                        self.status = format!(
                            "NMEA GPS fix: {lat:.6}, {lon:.6} ({} fixes)",
                            self.fixes_received
                        );
                        got_fix = true;
                    }
                    Ok(NmeaGpsEvent::Status(text)) => {
                        self.status = text;
                    }
                    Ok(NmeaGpsEvent::Error(text)) => {
                        self.status = text;
                    }
                    Ok(NmeaGpsEvent::Ended) => {
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
            if self.status.trim().is_empty() {
                self.status = "NMEA GPS connection ended.".to_owned();
            }
        }
        got_fix
    }

    #[must_use]
    pub fn latest_location(&self) -> Option<(f64, f64)> {
        self.latest_fix
    }

    #[must_use]
    pub fn status_text(&self) -> &str {
        &self.status
    }

    /// Directly overwrite the status string (used for non-event messages like
    /// the serial port list returned by `list_serial_ports`).
    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }

    #[must_use]
    pub fn fixes_received(&self) -> u64 {
        self.fixes_received
    }

    /// Returns `true` when the service is running and has received a fix
    /// within `stale_timeout_ms` milliseconds.
    #[must_use]
    pub fn has_recent_fix(&self, stale_timeout_ms: i64) -> bool {
        self.controller.is_some()
            && self.last_fix_at_ms > 0
            && (now_ms() - self.last_fix_at_ms) < stale_timeout_ms
    }
}

// ---------------------------------------------------------------------------
// Controller (stop + join)
// ---------------------------------------------------------------------------

struct NmeaGpsController {
    stop_flag: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl NmeaGpsController {
    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for NmeaGpsController {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

const TCP_READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Minimum change in degrees before we send a new Fix event (~0.1 m).
const DEDUP_THRESHOLD: f64 = 0.000001;

/// Returns true if the new position differs from the last sent position
/// by more than [`DEDUP_THRESHOLD`] in either axis.
fn position_changed(last: &Option<(f64, f64)>, lat: f64, lon: f64) -> bool {
    match last {
        None => true,
        Some((prev_lat, prev_lon)) => {
            (lat - prev_lat).abs() > DEDUP_THRESHOLD || (lon - prev_lon).abs() > DEDUP_THRESHOLD
        }
    }
}

fn nmea_tcp_worker(addr: String, stop: Arc<AtomicBool>, tx: mpsc::Sender<NmeaGpsEvent>) {
    let listener = match std::net::TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(err) => {
            let _ = tx.send(NmeaGpsEvent::Error(format!(
                "Failed to listen on {addr}: {err}"
            )));
            let _ = tx.send(NmeaGpsEvent::Ended);
            return;
        }
    };
    // Non-blocking accept so we can check the stop flag periodically.
    let _ = listener.set_nonblocking(true);
    let _ = tx.send(NmeaGpsEvent::Status(format!(
        "Listening for phone GPS on {addr}. Configure GPS2IP as TCP client pointing here."
    )));

    while !stop.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((stream, peer)) => {
                let _ = tx.send(NmeaGpsEvent::Status(format!(
                    "Phone GPS connected from {peer}."
                )));
                stream
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            Err(err) => {
                let _ = tx.send(NmeaGpsEvent::Error(format!(
                    "Accept failed on {addr}: {err}"
                )));
                let _ = tx.send(NmeaGpsEvent::Ended);
                return;
            }
        };

        let _ = stream.set_read_timeout(Some(TCP_READ_TIMEOUT));
        let reader = BufReader::new(stream);
        let mut last_sent: Option<(f64, f64)> = None;
        for line_result in reader.lines() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match line_result {
                Ok(line) => {
                    if let Some((lat, lon)) = parse_nmea_location(&line)
                        && position_changed(&last_sent, lat, lon)
                    {
                        last_sent = Some((lat, lon));
                        if tx.send(NmeaGpsEvent::Fix { lat, lon }).is_err() {
                            return;
                        }
                    }
                }
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // Read timeout — just loop and check stop flag.
                }
                Err(err) => {
                    let _ = tx.send(NmeaGpsEvent::Status(format!(
                        "Phone GPS disconnected ({err}). Waiting for reconnect on {addr}..."
                    )));
                    break;
                }
            }
        }
    }

    let _ = tx.send(NmeaGpsEvent::Ended);
}

/// Worker for TCP **client** mode: connects to the phone's NMEA server,
/// reads lines, and retries on disconnect until the stop flag is set.
fn nmea_tcp_client_worker(addr: String, stop: Arc<AtomicBool>, tx: mpsc::Sender<NmeaGpsEvent>) {
    while !stop.load(Ordering::Relaxed) {
        let stream = match std::net::TcpStream::connect_timeout(
            &addr.parse().unwrap_or_else(|_| {
                // Fallback: try to resolve via ToSocketAddrs.
                use std::net::ToSocketAddrs;
                addr.to_socket_addrs()
                    .ok()
                    .and_then(|mut iter| iter.next())
                    .unwrap_or_else(|| std::net::SocketAddr::from(([0, 0, 0, 0], 0)))
            }),
            Duration::from_secs(5),
        ) {
            Ok(s) => {
                let _ = tx.send(NmeaGpsEvent::Status(format!(
                    "Connected to phone GPS server at {addr}."
                )));
                s
            }
            Err(err) => {
                let _ = tx.send(NmeaGpsEvent::Status(format!(
                    "Cannot reach phone GPS server at {addr}: {err}. Retrying in 3s..."
                )));
                // Sleep in small increments so we can check the stop flag.
                for _ in 0..15 {
                    if stop.load(Ordering::Relaxed) {
                        let _ = tx.send(NmeaGpsEvent::Ended);
                        return;
                    }
                    thread::sleep(Duration::from_millis(200));
                }
                continue;
            }
        };

        let _ = stream.set_read_timeout(Some(TCP_READ_TIMEOUT));
        let reader = BufReader::new(stream);
        let mut last_sent: Option<(f64, f64)> = None;
        for line_result in reader.lines() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match line_result {
                Ok(line) => {
                    if let Some((lat, lon)) = parse_nmea_location(&line)
                        && position_changed(&last_sent, lat, lon)
                    {
                        last_sent = Some((lat, lon));
                        if tx.send(NmeaGpsEvent::Fix { lat, lon }).is_err() {
                            return;
                        }
                    }
                }
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // Read timeout — just loop and check stop flag.
                }
                Err(err) => {
                    let _ = tx.send(NmeaGpsEvent::Status(format!(
                        "Phone GPS server disconnected ({err}). Reconnecting to {addr}..."
                    )));
                    break;
                }
            }
        }
    }

    let _ = tx.send(NmeaGpsEvent::Ended);
}

/// Worker for Bluetooth / serial port NMEA. Opens the port once and reads
/// lines until the stop flag is set or an unrecoverable error occurs.
fn nmea_serial_worker(
    port_path: String,
    baud_rate: u32,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<NmeaGpsEvent>,
) {
    let port = match serialport::new(&port_path, baud_rate)
        .timeout(Duration::from_secs(2))
        .open()
    {
        Ok(p) => p,
        Err(err) => {
            let _ = tx.send(NmeaGpsEvent::Error(format!(
                "Failed to open Bluetooth port {port_path}: {err}. \
                 Make sure the device is paired and the port name is correct."
            )));
            let _ = tx.send(NmeaGpsEvent::Ended);
            return;
        }
    };
    let _ = tx.send(NmeaGpsEvent::Status(format!(
        "Bluetooth GPS connected on {port_path} ({baud_rate} baud). Waiting for fix..."
    )));

    let reader = BufReader::new(port);
    let mut last_sent: Option<(f64, f64)> = None;
    for line_result in reader.lines() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match line_result {
            Ok(line) => {
                if let Some((lat, lon)) = parse_nmea_location(&line)
                    && position_changed(&last_sent, lat, lon)
                {
                    last_sent = Some((lat, lon));
                    if tx.send(NmeaGpsEvent::Fix { lat, lon }).is_err() {
                        return;
                    }
                }
            }
            // A 2-second read timeout is normal on quiet Bluetooth links.
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock =>
            {
                // Just loop back and check the stop flag.
            }
            Err(err) => {
                let _ = tx.send(NmeaGpsEvent::Error(format!(
                    "Bluetooth GPS read error on {port_path}: {err}"
                )));
                break;
            }
        }
    }

    let _ = tx.send(NmeaGpsEvent::Ended);
}

// ---------------------------------------------------------------------------
// NMEA sentence parsing
// ---------------------------------------------------------------------------

/// Attempts to parse a lat/lon fix from a single NMEA sentence.
/// Supports `$GPGGA`, `$GNGGA`, `$GPRMC`, and `$GNRMC`.
#[must_use]
pub fn parse_nmea_location(line: &str) -> Option<(f64, f64)> {
    let line = line.trim();
    if !line.starts_with('$') {
        return None;
    }
    // Strip optional checksum (*XX)
    let body = line.split('*').next()?;
    let fields: Vec<&str> = body.split(',').collect();
    let sentence_type = fields.first()?;

    if sentence_type.ends_with("GGA") && fields.len() >= 6 {
        // $xxGGA,time,lat,N/S,lon,E/W,quality,...
        let quality = fields.get(6).and_then(|s| s.parse::<u8>().ok());
        if quality == Some(0) {
            return None; // no fix
        }
        let lat = parse_nmea_coordinate(fields[2], fields[3])?;
        let lon = parse_nmea_coordinate(fields[4], fields[5])?;
        return Some((lat, lon));
    }

    if sentence_type.ends_with("RMC") && fields.len() >= 6 {
        // $xxRMC,time,status,lat,N/S,lon,E/W,...
        let status = fields.get(2).copied().unwrap_or("");
        if status != "A" {
            return None; // V = void / no fix
        }
        let lat = parse_nmea_coordinate(fields[3], fields[4])?;
        let lon = parse_nmea_coordinate(fields[5], fields[6])?;
        return Some((lat, lon));
    }

    None
}

/// Parses an NMEA coordinate value (`DDMM.MMMM` or `DDDMM.MMMM`) with a
/// hemisphere indicator (`N`/`S`/`E`/`W`) into a signed decimal-degree f64.
fn parse_nmea_coordinate(value: &str, hemisphere: &str) -> Option<f64> {
    if value.is_empty() || hemisphere.is_empty() {
        return None;
    }
    let dot_pos = value.find('.')?;
    if dot_pos < 2 {
        return None;
    }
    let degree_digits = dot_pos - 2;
    let degrees: f64 = value[..degree_digits].parse().ok()?;
    let minutes: f64 = value[degree_digits..].parse().ok()?;
    let mut decimal = degrees + minutes / 60.0;

    match hemisphere {
        "S" | "W" => decimal = -decimal,
        "N" | "E" => {}
        _ => return None,
    }

    if decimal.is_finite() && (-180.0..=180.0).contains(&decimal) {
        Some(decimal)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gga_valid() {
        let line = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47";
        let (lat, lon) = parse_nmea_location(line).unwrap();
        assert!((lat - 48.1173).abs() < 0.001);
        assert!((lon - 11.516_667).abs() < 0.001);
    }

    #[test]
    fn parse_gga_no_fix() {
        let line = "$GPGGA,123519,4807.038,N,01131.000,E,0,00,,,,,,,*42";
        assert!(parse_nmea_location(line).is_none());
    }

    #[test]
    fn parse_rmc_valid() {
        let line = "$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A";
        let (lat, lon) = parse_nmea_location(line).unwrap();
        assert!((lat - 48.1173).abs() < 0.001);
        assert!((lon - 11.516_667).abs() < 0.001);
    }

    #[test]
    fn parse_rmc_void() {
        let line = "$GPRMC,123519,V,,,,,,,230394,,,N*53";
        assert!(parse_nmea_location(line).is_none());
    }

    #[test]
    fn parse_southern_hemisphere() {
        let line = "$GPGGA,120000,3348.123,S,15112.456,E,1,05,1.0,10.0,M,0.0,M,,*00";
        let (lat, lon) = parse_nmea_location(line).unwrap();
        assert!(lat < 0.0);
        assert!(lon > 0.0);
    }

    #[test]
    fn parse_gngga() {
        let line = "$GNGGA,120000,4807.038,N,01131.000,E,1,12,0.5,100.0,M,47.0,M,,*00";
        let (lat, lon) = parse_nmea_location(line).unwrap();
        assert!((lat - 48.1173).abs() < 0.001);
        assert!((lon - 11.516_667).abs() < 0.001);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_nmea_location("hello world").is_none());
        assert!(parse_nmea_location("").is_none());
        assert!(parse_nmea_location("$GPVTG,054.7,T,034.4,M,005.5,N,010.2,K*48").is_none());
    }
}
