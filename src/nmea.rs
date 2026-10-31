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

use std::io::{BufRead, BufReader, Read};
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
///
/// # Returns
///
/// * `Vec<String>` - The port names currently visible to the OS; empty if
///   none are available.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::list_serial_ports;
///
/// let ports = list_serial_ports();
/// println!("number of serial ports found: {}", ports.len());
/// ```
pub fn list_serial_ports() -> Vec<String> {
    let ports = usable_serial_port_names(serialport::available_ports().unwrap_or_default());
    #[cfg(target_os = "macos")]
    let ports = {
        let shadowed = macos_shadowed_serial_nodes();
        ports
            .into_iter()
            .filter(|name| !shadowed.contains(name) && !is_macos_noise_port(name))
            .collect::<Vec<_>>()
    };
    ports
}

/// Port names from `ports`, minus the Linux placeholder UARTs.
fn usable_serial_port_names(ports: Vec<serialport::SerialPortInfo>) -> Vec<String> {
    ports
        .into_iter()
        .map(|p| p.port_name)
        .filter(|name| !is_linux_placeholder_uart(name))
        .collect()
}

/// Names of the ports in `ports` that look like Bluetooth SPP devices.
fn likely_bluetooth_port_names(ports: Vec<serialport::SerialPortInfo>) -> Vec<String> {
    ports
        .into_iter()
        .filter(is_likely_bluetooth_port)
        .map(|p| p.port_name)
        .collect()
}

/// macOS exposes every serial device twice (`/dev/cu.*` for outgoing use and
/// `/dev/tty.*`) and adds system Bluetooth ports that are never a GPS. Only the
/// `cu.` node of a real device is useful.
#[cfg(any(target_os = "macos", test))]
fn is_macos_noise_port(port_name: &str) -> bool {
    port_name.starts_with("/dev/tty.")
        || port_name.contains(".BLTH")
        || port_name.contains("Bluetooth-Incoming-Port")
        || port_name.contains("debug-console")
}

/// Maps a saved raw macOS serial node (`/dev/cu.NAME` or `/dev/tty.NAME`) to
/// the `bt://` entry for the same device in `candidates`, so selections saved
/// before devices were offered as `bt://` keep working.
#[must_use]
pub fn migrate_raw_bluetooth_port(selected: &str, candidates: &[String]) -> Option<String> {
    let node_name = selected
        .strip_prefix("/dev/cu.")
        .or_else(|| selected.strip_prefix("/dev/tty."))?;
    let sanitize = |text: &str| -> String {
        text.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    };
    let wanted = sanitize(node_name);
    candidates
        .iter()
        .find(|c| crate::bt_port::device_name(c).is_some_and(|name| sanitize(name) == wanted))
        .cloned()
}

static BLUETOOTH_SCAN_ENABLED: AtomicBool = AtomicBool::new(false);

/// Allows or forbids querying the OS Bluetooth stack when listing ports.
///
/// Enumerating Bluetooth devices can trigger the macOS permission prompt and
/// talks to `bluetoothd` on Linux, so the UI only enables it while the
/// Bluetooth GPS mode is selected.
pub fn set_bluetooth_scan_enabled(enabled: bool) {
    BLUETOOTH_SCAN_ENABLED.store(enabled, Ordering::Relaxed);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn bluetooth_scan_enabled() -> bool {
    BLUETOOTH_SCAN_ENABLED.load(Ordering::Relaxed)
}

/// `/dev/cu.*` nodes belonging to devices we offer as `bt://` ports instead,
/// so the same GPS isn't listed twice.
#[cfg(target_os = "macos")]
fn macos_shadowed_serial_nodes() -> std::collections::HashSet<String> {
    if !bluetooth_scan_enabled() {
        return std::collections::HashSet::new();
    }
    crate::iobt::list_paired_spp_devices()
        .into_iter()
        .filter_map(|d| d.serial_node)
        .collect()
}

/// `true` for the `/dev/ttyS<n>` nodes the Linux kernel pre-creates (often 32
/// of them) whether or not a UART exists behind them. They are never a GPS;
/// a real on-board UART can still be typed into the manual override field.
fn is_linux_placeholder_uart(port_name: &str) -> bool {
    cfg!(target_os = "linux") && port_name.starts_with("/dev/ttyS")
}

/// Returns only Bluetooth SPP serial port names, filtered per-platform:
///
/// - **Windows**: ports tagged `SerialPortType::BluetoothPort` by the driver.
/// - **macOS**: `/dev/cu.*` ports that are *not* `PciPort` (built-in). BT SPP
///   often shows as `Unknown` on macOS so we can't rely on the type alone.
///   Falls back to a direct `/dev/cu.*` filesystem scan when the `serialport`
///   crate misses Bluetooth ports (common on newer macOS).
/// - **Linux**: `/dev/rfcomm*` ports (BT RFCOMM channels).
///
/// Falls back to the full `list_serial_ports()` on unsupported platforms.
///
/// # Returns
///
/// * `Vec<String>` - The Bluetooth SPP port names currently visible to the
///   OS; empty if none are paired/available.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::list_bluetooth_ports;
///
/// let ports = list_bluetooth_ports();
/// println!("number of bluetooth ports found: {}", ports.len());
/// ```
pub fn list_bluetooth_ports() -> Vec<String> {
    #[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(unused_mut))]
    let mut filtered =
        likely_bluetooth_port_names(serialport::available_ports().unwrap_or_default());

    // Linux: paired SPP devices reachable through BlueZ (shown as
    // `bt://<address>`; no `rfcomm bind` needed).
    #[cfg(target_os = "linux")]
    if bluetooth_scan_enabled() {
        let devices = crate::bluez::cached_paired_spp_devices();
        filtered.extend(devices.iter().map(crate::bluez::SppDevice::port_name));
    }

    #[cfg(target_os = "macos")]
    let mut shadowed = std::collections::HashSet::<String>::new();
    #[cfg(target_os = "macos")]
    if bluetooth_scan_enabled() {
        let devices = crate::iobt::list_paired_spp_devices();
        shadowed = devices
            .iter()
            .filter_map(|d| d.serial_node.clone())
            .collect();
        filtered.retain(|port| !shadowed.contains(port));
        filtered.extend(devices.iter().map(crate::iobt::SppDevice::port_name));
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(entries) = std::fs::read_dir("/dev") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("cu.") {
                    continue;
                }
                let full = format!("/dev/{name}");
                if filtered.contains(&full) || shadowed.contains(&full) {
                    continue;
                }
                if is_likely_bluetooth_dev_name(&name) {
                    filtered.push(full);
                }
            }
        }
    }

    filtered
}

fn is_likely_bluetooth_port(port: &serialport::SerialPortInfo) -> bool {
    use serialport::SerialPortType;
    match &port.port_type {
        SerialPortType::BluetoothPort => true,
        #[cfg(target_os = "macos")]
        SerialPortType::Unknown => {
            // On macOS, /dev/cu.* Bluetooth SPP ports show as Unknown.
            // Exclude system and non-BT ports.
            let name = &port.port_name;
            name.starts_with("/dev/cu.")
                && !name.contains("usbmodem")
                && !name.contains("usbserial")
                && !name.contains("Bluetooth-Incoming")
                && name != "/dev/cu.BLTH"
        }
        #[cfg(target_os = "linux")]
        SerialPortType::Unknown => port.port_name.starts_with("/dev/rfcomm"),
        _ => false,
    }
}

/// Checks whether a `/dev/cu.*` entry name (without the `/dev/` prefix) looks
/// like a Bluetooth SPP device rather than a built-in or USB serial port.
#[cfg(target_os = "macos")]
fn is_likely_bluetooth_dev_name(name: &str) -> bool {
    // `name` is e.g. "cu.BY80125B39" (without /dev/ prefix).
    let suffix = name.strip_prefix("cu.").unwrap_or("");
    !suffix.is_empty()
        && !name.contains("usbmodem")
        && !name.contains("usbserial")
        && !name.contains("Bluetooth-Incoming")
        && suffix != "BLTH"
        // Exclude wlan/debug ports sometimes seen on macOS.
        && !name.contains("wlan")
        && !name.contains("debug")
}

/// Normalizes a serial/Bluetooth port name for comparison purposes. On
/// Windows, `COM` port names are case-insensitive; elsewhere the name (minus
/// surrounding whitespace) is already canonical.
#[must_use]
pub fn canonical_serial_port_name(port_name: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        return port_name.trim().to_ascii_uppercase();
    }
    #[cfg(not(target_os = "windows"))]
    {
        port_name.trim().to_owned()
    }
}

/// Extracts the numeric suffix from a Windows `COM<N>` port name.
#[cfg(target_os = "windows")]
#[must_use]
pub fn parse_windows_com_port_number(port_name: &str) -> Option<u32> {
    let upper = port_name.trim().to_ascii_uppercase();
    let suffix = upper.strip_prefix("COM")?;
    suffix.parse::<u32>().ok()
}

/// Picks a sensible default serial port from `candidates`. On Windows,
/// Bluetooth "outgoing" ports tend to be the highest-numbered `COM` device,
/// so that's preferred when parseable; otherwise the first candidate wins.
#[must_use]
pub fn pick_default_nmea_serial_port(candidates: &[String]) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        if let Some((_, port)) = candidates
            .iter()
            .filter_map(|port| parse_windows_com_port_number(port).map(|num| (num, port)))
            .max_by_key(|(num, _)| *num)
        {
            return Some(port.clone());
        }
    }
    candidates.first().cloned()
}

/// Finds `selected_port`'s index within `candidates`, comparing canonicalized
/// names so platform-specific casing differences don't cause a mismatch.
#[must_use]
pub fn find_nmea_serial_port_index(candidates: &[String], selected_port: &str) -> Option<usize> {
    let canonical_selected = canonical_serial_port_name(selected_port);
    candidates
        .iter()
        .position(|candidate| canonical_serial_port_name(candidate) == canonical_selected)
}

/// Returns the ports in `current` that were not present in `baseline`,
/// comparing canonicalized names and preserving `current`'s order. Used by the
/// "pair in OS settings" flow to spot the serial port a freshly paired device
/// creates.
///
/// # Arguments
///
/// * `baseline` - Port names captured before the user started pairing.
/// * `current` - Port names visible now.
///
/// # Returns
///
/// * `Vec<String>` - Ports that appeared since `baseline`.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::new_serial_ports;
///
/// let before = vec!["/dev/cu.A".to_string()];
/// let now = vec!["/dev/cu.A".to_string(), "/dev/cu.B".to_string()];
/// assert_eq!(new_serial_ports(&before, &now), vec!["/dev/cu.B".to_string()]);
/// ```
#[must_use]
pub fn new_serial_ports(baseline: &[String], current: &[String]) -> Vec<String> {
    current
        .iter()
        .filter(|port| find_nmea_serial_port_index(baseline, port).is_none())
        .cloned()
        .collect()
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

/// Tracks a background GPS listener (TCP server, TCP client, or Bluetooth/
/// serial) and the latest decoded fix, polled from the UI thread via
/// [`NmeaGpsState::poll_events`]. Only one connection mode can be active at
/// a time.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::NmeaGpsState;
///
/// let state = NmeaGpsState::default();
/// assert!(!state.is_running());
/// assert_eq!(state.fixes_received(), 0);
/// assert!(state.latest_location().is_none());
/// ```
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
    ///
    /// # Arguments
    ///
    /// * `port_path` - The serial/SPP port to open.
    /// * `protocol` - Which sentence format to parse fixes from.
    ///
    /// # Returns
    ///
    /// * `Result<String>` - A human-readable status message on success.
    ///
    /// # Errors
    ///
    /// Returns an error if `port_path` is empty or the worker thread cannot
    /// be spawned.
    pub fn start_bluetooth(&mut self, port_path: &str, protocol: GpsProtocol) -> Result<String> {
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
            .spawn(move || {
                nmea_serial_worker(path_clone, DEFAULT_BT_BAUD_RATE, worker_stop, tx, protocol);
            })
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
    ///
    /// # Arguments
    ///
    /// * `host` - The phone's address to dial.
    /// * `port` - The phone's NMEA server TCP port.
    /// * `protocol` - Which sentence format to parse fixes from.
    ///
    /// # Returns
    ///
    /// * `Result<String>` - A human-readable status message on success.
    ///
    /// # Errors
    ///
    /// Returns an error if `host` is empty or the worker thread cannot be
    /// spawned.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::{GpsProtocol, NmeaGpsState};
    ///
    /// let mut state = NmeaGpsState::default();
    /// // Port 0 has nothing listening, so this connects and immediately
    /// // starts retrying in the background — enough to exercise the API.
    /// state.start_client("127.0.0.1", 0, GpsProtocol::Nmea).unwrap();
    /// assert!(state.is_running());
    ///
    /// state.stop();
    /// assert!(!state.is_running());
    /// ```
    pub fn start_client(&mut self, host: &str, port: u16, protocol: GpsProtocol) -> Result<String> {
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
            .spawn(move || nmea_tcp_client_worker(addr_clone, worker_stop, tx, protocol))
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
    ///
    /// # Arguments
    ///
    /// * `host` - The local address to bind to (empty binds to all interfaces).
    /// * `port` - The local TCP port to listen on.
    /// * `protocol` - Which sentence format to parse fixes from.
    ///
    /// # Returns
    ///
    /// * `Result<String>` - A human-readable status message on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the worker thread cannot be spawned.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::{GpsProtocol, NmeaGpsState};
    ///
    /// let mut state = NmeaGpsState::default();
    /// let message = state.start("127.0.0.1", 0, GpsProtocol::Nmea).unwrap();
    /// assert!(message.contains("Listening"));
    /// assert!(state.is_running());
    ///
    /// state.stop();
    /// assert!(!state.is_running());
    /// ```
    pub fn start(&mut self, host: &str, port: u16, protocol: GpsProtocol) -> Result<String> {
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
            .spawn(move || nmea_tcp_worker(addr_clone, worker_stop, tx, protocol))
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

    /// Stops the background listener thread, if one is running, and resets
    /// the fix count. Safe to call even if no listener is running.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::{GpsProtocol, NmeaGpsState};
    ///
    /// let mut state = NmeaGpsState::default();
    /// state.start("127.0.0.1", 0, GpsProtocol::Nmea).unwrap();
    ///
    /// state.stop();
    /// assert!(!state.is_running());
    /// assert_eq!(state.status_text(), "NMEA GPS listener stopped.");
    /// ```
    pub fn stop(&mut self) {
        if let Some(mut controller) = self.controller.take() {
            controller.stop();
            self.status = "NMEA GPS listener stopped.".to_string();
        }
        self.event_rx = None;
        self.fixes_received = 0;
        self.last_fix_at_ms = 0;
    }

    /// Returns whether a background listener thread is currently running.
    ///
    /// # Returns
    ///
    /// * `bool` - `true` if a `start*` method succeeded and [`NmeaGpsState::stop`]
    ///   has not since been called.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::NmeaGpsState;
    ///
    /// let state = NmeaGpsState::default();
    /// assert!(!state.is_running());
    /// ```
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.controller.is_some()
    }

    /// Drains pending events from the worker thread. Call from the UI timer.
    ///
    /// # Returns
    ///
    /// * `bool` - `true` if a new GPS fix was recorded during this call.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::NmeaGpsState;
    ///
    /// let mut state = NmeaGpsState::default();
    /// // No listener has been started, so there is nothing to poll.
    /// assert!(!state.poll_events());
    /// ```
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
                    Ok(NmeaGpsEvent::Status(text) | NmeaGpsEvent::Error(text)) => {
                        self.status = text;
                    }
                    Ok(NmeaGpsEvent::Ended) | Err(TryRecvError::Disconnected) => {
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
            if self.status.trim().is_empty() {
                self.status = "NMEA GPS connection ended.".to_string();
            }
        }
        got_fix
    }

    /// Returns the most recently decoded fix, if any has been received yet.
    ///
    /// # Returns
    ///
    /// * `Option<(f64, f64)>` - The latest `(latitude, longitude)`, or `None`
    ///   if no fix has arrived since the listener was started.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::NmeaGpsState;
    ///
    /// let state = NmeaGpsState::default();
    /// assert!(state.latest_location().is_none());
    /// ```
    #[must_use]
    pub fn latest_location(&self) -> Option<(f64, f64)> {
        self.latest_fix
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
    /// use third_eye_client::nmea::NmeaGpsState;
    ///
    /// let state = NmeaGpsState::default();
    /// assert_eq!(state.status_text(), "");
    /// ```
    #[must_use]
    pub fn status_text(&self) -> &str {
        &self.status
    }

    /// Directly overwrite the status string (used for non-event messages like
    /// the serial port list returned by `list_serial_ports`).
    ///
    /// # Arguments
    ///
    /// * `status` - The new status message to display.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::NmeaGpsState;
    ///
    /// let mut state = NmeaGpsState::default();
    /// state.set_status("Waiting for configuration.".to_string());
    /// assert_eq!(state.status_text(), "Waiting for configuration.");
    /// ```
    pub fn set_status(&mut self, status: String) {
        self.status = status;
    }

    /// Returns the number of GPS fixes received since the listener was last
    /// started.
    ///
    /// # Returns
    ///
    /// * `u64` - The number of fixes received.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::NmeaGpsState;
    ///
    /// let state = NmeaGpsState::default();
    /// assert_eq!(state.fixes_received(), 0);
    /// ```
    #[must_use]
    pub fn fixes_received(&self) -> u64 {
        self.fixes_received
    }

    /// Returns `true` when the service is running and has received a fix
    /// within `stale_timeout_ms` milliseconds.
    ///
    /// # Arguments
    ///
    /// * `stale_timeout_ms` - How old (in milliseconds) the most recent fix
    ///   may be before it's considered stale.
    ///
    /// # Returns
    ///
    /// * `bool` - `true` if running and the last fix is within the timeout.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::NmeaGpsState;
    ///
    /// let state = NmeaGpsState::default();
    /// assert!(!state.has_recent_fix(5_000));
    /// ```
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

/// How many times to retry opening a Bluetooth serial port before giving up.
/// macOS may keep the RFCOMM channel reserved for a moment after the previous
/// session closes.
const BT_OPEN_RETRIES: u32 = 4;

/// Delay between Bluetooth port open retries.
const BT_OPEN_RETRY_DELAY: Duration = Duration::from_millis(800);

/// Settle time after closing a Bluetooth serial port. Gives the macOS
/// Bluetooth stack time to fully release the RFCOMM channel.
const BT_CLOSE_SETTLE: Duration = Duration::from_millis(500);

/// Seconds to wait before retrying the full open cycle when the device is
/// unreachable (e.g. powered off or out of range).
const BT_RECONNECT_DELAY_SECS: u32 = 5;

/// Minimum change in degrees before we send a new Fix event (~0.1 m).
const DEDUP_THRESHOLD: f64 = 0.000_001;

/// Returns true if the new position differs from the last sent position
/// by more than [`DEDUP_THRESHOLD`] in either axis.
fn position_changed(last: Option<&(f64, f64)>, lat: f64, lon: f64) -> bool {
    match last {
        None => true,
        Some((prev_lat, prev_lon)) => {
            (lat - prev_lat).abs() > DEDUP_THRESHOLD || (lon - prev_lon).abs() > DEDUP_THRESHOLD
        }
    }
}

fn nmea_tcp_worker(
    addr: String,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<NmeaGpsEvent>,
    protocol: GpsProtocol,
) {
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
                    if let Some((lat, lon)) = parse_gps_location(&line, protocol)
                        && position_changed(last_sent.as_ref(), lat, lon)
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
fn nmea_tcp_client_worker(
    addr: String,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<NmeaGpsEvent>,
    protocol: GpsProtocol,
) {
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
                    if let Some((lat, lon)) = parse_gps_location(&line, protocol)
                        && position_changed(last_sent.as_ref(), lat, lon)
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

/// Opens the byte source for `port_path`: an RFCOMM link (`BlueZ` on Linux,
/// `IOBluetooth` on macOS) for `bt://` names, otherwise a serial port at `baud_rate`.
#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux")),
    allow(unused_variables)
)]
fn open_nmea_link(
    port_path: &str,
    baud_rate: u32,
    stop: &AtomicBool,
) -> Result<Box<dyn Read + Send>, String> {
    #[cfg(target_os = "linux")]
    if let Some(address) = crate::bt_port::parse_port_name(port_path) {
        return boxed_link(crate::bluez::connect(&address, stop));
    }
    #[cfg(target_os = "macos")]
    if let Some(address) = crate::bt_port::parse_port_name(port_path) {
        return boxed_link(crate::iobt::connect(&address, stop));
    }
    boxed_link(
        serialport::new(port_path, baud_rate)
            .timeout(Duration::from_secs(2))
            .open(),
    )
}

/// Turns the outcome of opening a link into a boxed reader, or its error text.
fn boxed_link<L, E>(result: Result<L, E>) -> Result<Box<dyn Read + Send>, String>
where
    L: Read + Send + 'static,
    E: std::fmt::Display,
{
    match result {
        Ok(link) => Ok(Box::new(link)),
        Err(err) => Err(format!("{err:#}")),
    }
}

/// Worker for Bluetooth / serial port NMEA. Maintains a persistent connection
/// by automatically reconnecting when the link drops. Only exits when the
/// stop flag is set.
fn nmea_serial_worker(
    port_path: String,
    baud_rate: u32,
    stop: Arc<AtomicBool>,
    tx: mpsc::Sender<NmeaGpsEvent>,
    protocol: GpsProtocol,
) {
    nmea_serial_worker_with(&port_path, baud_rate, &stop, &tx, protocol, |path, baud| {
        open_nmea_link(path, baud, &stop)
    });
}

/// Body of [`nmea_serial_worker`] with the link opener injected, so tests can
/// drive it without a real serial port or Bluetooth device.
fn nmea_serial_worker_with(
    port_path: &str,
    baud_rate: u32,
    stop: &AtomicBool,
    tx: &mpsc::Sender<NmeaGpsEvent>,
    protocol: GpsProtocol,
    open: impl Fn(&str, u32) -> Result<Box<dyn Read + Send>, String>,
) {
    let mut last_sent: Option<(f64, f64)> = None;

    // Outer reconnect loop — keeps trying to (re)open the port until stopped.
    while !stop.load(Ordering::Relaxed) {
        let port = {
            let mut last_err = String::new();
            let mut opened = None;
            for attempt in 0..BT_OPEN_RETRIES {
                if stop.load(Ordering::Relaxed) {
                    let _ = tx.send(NmeaGpsEvent::Ended);
                    return;
                }
                match open(port_path, baud_rate) {
                    Ok(p) => {
                        opened = Some(p);
                        break;
                    }
                    Err(err) => {
                        last_err = err;
                        eprintln!("third-eye-client: opening {port_path} failed: {last_err}");
                        if attempt + 1 < BT_OPEN_RETRIES {
                            let _ = tx.send(NmeaGpsEvent::Status(format!(
                                "Could not open {port_path}: {last_err}. Retrying ({}/{BT_OPEN_RETRIES})...",
                                attempt + 1
                            )));
                            thread::sleep(BT_OPEN_RETRY_DELAY);
                        }
                    }
                }
            }
            if let Some(p) = opened {
                p
            } else {
                let _ = tx.send(NmeaGpsEvent::Status(format!(
                    "Cannot open {port_path}: {last_err}. \
                     Retrying in {BT_RECONNECT_DELAY_SECS}s..."
                )));
                for _ in 0..(BT_RECONNECT_DELAY_SECS * 5) {
                    if stop.load(Ordering::Relaxed) {
                        let _ = tx.send(NmeaGpsEvent::Ended);
                        return;
                    }
                    thread::sleep(Duration::from_millis(200));
                }
                continue;
            }
        };

        let _ = tx.send(NmeaGpsEvent::Status(format!(
            "Bluetooth GPS connected on {port_path} ({baud_rate} baud). Waiting for fix..."
        )));

        let mut reader = BufReader::new(port);
        match pump_nmea_lines(&mut reader, stop, tx, protocol, &mut last_sent) {
            ReadLoopExit::Stopped => break,
            ReadLoopExit::Disconnected => {}
            ReadLoopExit::Error(err) => {
                let _ = tx.send(NmeaGpsEvent::Status(format!(
                    "Bluetooth GPS disconnected ({err}). Reconnecting to {port_path}..."
                )));
            }
        }

        drop(reader);
        thread::sleep(BT_CLOSE_SETTLE);
    }

    let _ = tx.send(NmeaGpsEvent::Ended);
}

/// Why [`pump_nmea_lines`] stopped reading.
enum ReadLoopExit {
    /// The stop flag was set, or the event channel was closed; the worker
    /// should shut down rather than reconnect.
    Stopped,
    /// The reader reached EOF (`Ok(0)`); the caller should reconnect.
    Disconnected,
    /// A non-timeout read error occurred; the caller should reconnect.
    Error(std::io::Error),
}

/// Reads NMEA/TAIP lines from `reader`, emitting deduplicated
/// [`NmeaGpsEvent::Fix`] events on `tx` until the stop flag is observed, EOF is
/// reached, or a read error occurs.
///
/// Read timeouts (`TimedOut` / `WouldBlock`) are ignored so a blocking serial
/// port with a short timeout still observes the stop flag promptly. `last_sent`
/// carries dedup state across reconnects. Extracted from [`nmea_serial_worker`]
/// behind a [`BufRead`] seam so it can be unit-tested with an in-memory reader.
fn pump_nmea_lines<R: BufRead>(
    reader: &mut R,
    stop: &AtomicBool,
    tx: &mpsc::Sender<NmeaGpsEvent>,
    protocol: GpsProtocol,
    last_sent: &mut Option<(f64, f64)>,
) -> ReadLoopExit {
    let mut line_buf = String::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return ReadLoopExit::Stopped;
        }
        line_buf.clear();
        match reader.read_line(&mut line_buf) {
            Ok(0) => return ReadLoopExit::Disconnected,
            Ok(_) => {
                if let Some((lat, lon)) = parse_gps_location(&line_buf, protocol)
                    && position_changed(last_sent.as_ref(), lat, lon)
                {
                    *last_sent = Some((lat, lon));
                    if tx.send(NmeaGpsEvent::Fix { lat, lon }).is_err() {
                        return ReadLoopExit::Stopped;
                    }
                }
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(err) => return ReadLoopExit::Error(err),
        }
    }
}

/// GPS protocol selector for worker threads.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::GpsProtocol;
///
/// assert_eq!(GpsProtocol::default(), GpsProtocol::Nmea);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpsProtocol {
    #[default]
    Nmea,
    Taip,
}

impl GpsProtocol {
    /// Parses the protocol from a config string: `"0"` = NMEA, `"1"` = TAIP.
    ///
    /// # Arguments
    ///
    /// * `value` - The saved config string.
    ///
    /// # Returns
    ///
    /// * `Self` - The corresponding protocol; defaults to [`GpsProtocol::Nmea`]
    ///   for any value other than `"1"`.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::GpsProtocol;
    ///
    /// assert_eq!(GpsProtocol::from_config("1"), GpsProtocol::Taip);
    /// assert_eq!(GpsProtocol::from_config("0"), GpsProtocol::Nmea);
    /// assert_eq!(GpsProtocol::from_config("garbage"), GpsProtocol::Nmea);
    /// ```
    #[must_use]
    pub fn from_config(value: &str) -> Self {
        match value.trim() {
            "1" => Self::Taip,
            _ => Self::Nmea,
        }
    }

    /// Returns the config string for persistence.
    ///
    /// # Returns
    ///
    /// * `&'static str` - `"0"` for NMEA, `"1"` for TAIP.
    ///
    /// # Examples
    ///
    /// ```
    /// use third_eye_client::nmea::GpsProtocol;
    ///
    /// assert_eq!(GpsProtocol::Nmea.as_config_str(), "0");
    /// assert_eq!(GpsProtocol::Taip.as_config_str(), "1");
    /// ```
    #[must_use]
    pub fn as_config_str(self) -> &'static str {
        match self {
            Self::Nmea => "0",
            Self::Taip => "1",
        }
    }
}

// ---------------------------------------------------------------------------
// GPS sentence parsing (NMEA + TAIP)
// ---------------------------------------------------------------------------

/// Attempts to parse a lat/lon fix from a line, trying the protocol-specific
/// parser first. Falls back to the other protocol if the first fails.
///
/// # Arguments
///
/// * `line` - A single GPS sentence (NMEA or TAIP).
/// * `protocol` - Which parser to try first.
///
/// # Returns
///
/// * `Option<(f64, f64)>` - The `(latitude, longitude)` in decimal degrees, or
///   `None` if neither parser recognizes the line or reports no fix.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::{GpsProtocol, parse_gps_location};
///
/// let line = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47";
/// let (lat, lon) = parse_gps_location(line, GpsProtocol::Nmea).unwrap();
/// assert!((lat - 48.1173).abs() < 0.001);
/// assert!((lon - 11.516_667).abs() < 0.001);
/// ```
#[must_use]
pub fn parse_gps_location(line: &str, protocol: GpsProtocol) -> Option<(f64, f64)> {
    match protocol {
        GpsProtocol::Nmea => parse_nmea_location(line).or_else(|| parse_taip_location(line)),
        GpsProtocol::Taip => parse_taip_location(line).or_else(|| parse_nmea_location(line)),
    }
}

/// Attempts to parse a lat/lon fix from a single NMEA sentence.
/// Supports `$GPGGA`, `$GNGGA`, `$GPRMC`, and `$GNRMC`.
///
/// # Arguments
///
/// * `line` - A single NMEA sentence, e.g. `"$GPGGA,...*47"`.
///
/// # Returns
///
/// * `Option<(f64, f64)>` - The `(latitude, longitude)` in decimal degrees, or
///   `None` if the sentence isn't a supported type or reports no fix.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::parse_nmea_location;
///
/// let line = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47";
/// let (lat, lon) = parse_nmea_location(line).unwrap();
/// assert!((lat - 48.1173).abs() < 0.001);
/// assert!((lon - 11.516_667).abs() < 0.001);
/// ```
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
    // NMEA fields are ASCII; reject non-ASCII so the byte-index slicing below
    // can never split a multi-byte UTF-8 char (which would panic).
    if !value.is_ascii() {
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
// TAIP sentence parsing
// ---------------------------------------------------------------------------

/// Attempts to parse a lat/lon fix from a TAIP RPV (Position/Velocity) sentence.
///
/// Format: `>RPV{time5}{±lat8}{±lon9}{spd3}{hdg3}{fix1}{age1}[;ID=...][;*xx]<`
///
/// Latitude is `±DD.DDDDD` (sign + 2-digit degrees + 5-digit decimal fraction).
/// Longitude is `±DDD.DDDDD` (sign + 3-digit degrees + 5-digit decimal fraction).
/// Fix mode `9` means no fix.
///
/// # Arguments
///
/// * `line` - A single TAIP RPV sentence, e.g. `">RPV...<"`.
///
/// # Returns
///
/// * `Option<(f64, f64)>` - The `(latitude, longitude)` in decimal degrees, or
///   `None` if the sentence isn't a recognized RPV sentence or reports no fix.
///
/// # Examples
///
/// ```
/// use third_eye_client::nmea::parse_taip_location;
///
/// let line = ">RPV15714+3739438-1220384601512612<";
/// let (lat, lon) = parse_taip_location(line).unwrap();
/// assert!((lat - 37.39438).abs() < 0.00001);
/// assert!((lon - (-122.03846)).abs() < 0.00001);
/// ```
#[must_use]
pub fn parse_taip_location(line: &str) -> Option<(f64, f64)> {
    let line = line.trim();

    // Strip optional `>` / `<` delimiters.
    let body = line.strip_prefix('>').unwrap_or(line).split('<').next()?;

    // Must start with RPV.
    let data = body.strip_prefix("RPV")?;

    // Strip optional `;ID=...` and `;*xx` suffixes — only keep the fixed-length data.
    let data = data.split(';').next()?;

    // TAIP data is ASCII; reject non-ASCII so the fixed byte-offset slices below
    // cannot split a multi-byte UTF-8 char (which would panic).
    if !data.is_ascii() {
        return None;
    }

    // Minimum data length: time(5) + lat(8) + lon(9) + spd(3) + hdg(3) + fix(1) + age(1) = 30
    if data.len() < 30 {
        return None;
    }

    // Fix mode at offset 25 (after time5 + lat8 + lon9 + spd3 = 25, then hdg3 = 28, fix at 28).
    // Offsets: time=[0..5], lat=[5..13], lon=[13..22], spd=[22..25], hdg=[25..28], fix=[28], age=[29]
    let fix_mode = data.as_bytes().get(28)?;
    if *fix_mode == b'9' {
        return None; // no fix
    }

    let lat_str = &data[5..13]; // ±DDFFFFF (8 chars)
    let lon_str = &data[13..22]; // ±DDDFFFFF (9 chars)

    let lat = parse_taip_coordinate(lat_str, 2)?;
    let lon = parse_taip_coordinate(lon_str, 3)?;

    if lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
    {
        Some((lat, lon))
    } else {
        None
    }
}

/// Parses a TAIP coordinate: sign + `degree_digits` digits of degrees + 5 digits
/// of decimal fraction → `±DD.DDDDD` or `±DDD.DDDDD`.
fn parse_taip_coordinate(s: &str, degree_digits: usize) -> Option<f64> {
    // ASCII guard: the byte-index slicing below would panic on a multi-byte char.
    if !s.is_ascii() {
        return None;
    }
    if s.len() < 1 + degree_digits + 5 {
        return None;
    }
    let sign = match s.as_bytes()[0] {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let degrees: f64 = s[1..=degree_digits].parse().ok()?;
    let fraction: f64 = s[1 + degree_digits..].parse().ok()?;
    let scale = 10_f64.powi(s[(1 + degree_digits)..].len() as i32);
    Some(sign * (degrees + fraction / scale))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- NMEA tests -------------------------------------------------------

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

    // ---- TAIP tests -------------------------------------------------------

    #[test]
    fn taip_valid_rpv() {
        // >RPV15714+3739438-1220384601512612<
        let line = ">RPV15714+3739438-1220384601512612<";
        let (lat, lon) = parse_taip_location(line).unwrap();
        assert!((lat - 37.39438).abs() < 0.00001, "lat={lat}");
        assert!((lon - (-122.03846)).abs() < 0.00001, "lon={lon}");
    }

    #[test]
    fn taip_with_id_and_checksum() {
        let line = ">RPV15714+3739438-1220384601512612;ID=1234;*7F<";
        let (lat, lon) = parse_taip_location(line).unwrap();
        assert!((lat - 37.39438).abs() < 0.00001);
        assert!((lon - (-122.03846)).abs() < 0.00001);
    }

    #[test]
    fn taip_no_fix_mode_9() {
        let line = ">RPV15714+3739438-1220384601512692<";
        assert!(parse_taip_location(line).is_none());
    }

    #[test]
    fn taip_negative_latitude() {
        // Southern hemisphere: lat = -33.86000
        let line = ">RPV00000-3386000+1511200000000012<";
        let (lat, lon) = parse_taip_location(line).unwrap();
        assert!(lat < 0.0, "lat should be negative: {lat}");
        assert!((lat - (-33.86)).abs() < 0.001);
        assert!(lon > 0.0);
    }

    #[test]
    fn taip_too_short() {
        assert!(parse_taip_location(">RPV123<").is_none());
        assert!(parse_taip_location("").is_none());
        assert!(parse_taip_location("RPV").is_none());
    }

    #[test]
    fn taip_without_delimiters() {
        // Some devices send without > <
        let line = "RPV15714+3739438-1220384601512612";
        let (lat, lon) = parse_taip_location(line).unwrap();
        assert!((lat - 37.39438).abs() < 0.00001);
        assert!((lon - (-122.03846)).abs() < 0.00001);
    }

    #[test]
    fn taip_rejects_non_rpv() {
        assert!(parse_taip_location(">RAL15714+3739438-1220384601512612<").is_none());
        assert!(parse_taip_location("hello world").is_none());
    }

    #[test]
    fn taip_zero_coordinates() {
        let line = ">RPV00000+0000000+0000000000000012<";
        let (lat, lon) = parse_taip_location(line).unwrap();
        assert!((lat).abs() < 0.001);
        assert!((lon).abs() < 0.001);
    }

    #[test]
    fn taip_location_rejects_non_ascii_without_panic() {
        // 'é' (2 bytes) straddles the fixed slice boundary at byte 13, which
        // would panic the byte-index slicing before the ASCII guard was added.
        let line = ">RPV15714+373943\u{e9}2038400000581234<";
        assert!(parse_taip_location(line).is_none());
    }

    // ---- Unified parser tests ---------------------------------------------

    #[test]
    fn gps_location_nmea_protocol_parses_nmea() {
        let line = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47";
        assert!(parse_gps_location(line, GpsProtocol::Nmea).is_some());
    }

    #[test]
    fn gps_location_taip_protocol_parses_taip() {
        let line = ">RPV15714+3739438-1220384601512612<";
        assert!(parse_gps_location(line, GpsProtocol::Taip).is_some());
    }

    #[test]
    fn gps_location_fallback_nmea_to_taip() {
        // TAIP input with NMEA protocol selected — should fall back.
        let line = ">RPV15714+3739438-1220384601512612<";
        assert!(parse_gps_location(line, GpsProtocol::Nmea).is_some());
    }

    #[test]
    fn gps_location_fallback_taip_to_nmea() {
        // NMEA input with TAIP protocol selected — should fall back.
        let line = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47";
        assert!(parse_gps_location(line, GpsProtocol::Taip).is_some());
    }

    #[test]
    fn gps_location_garbage() {
        assert!(parse_gps_location("hello", GpsProtocol::Nmea).is_none());
        assert!(parse_gps_location("hello", GpsProtocol::Taip).is_none());
    }

    // ---- GpsProtocol tests ------------------------------------------------

    #[test]
    fn protocol_from_config() {
        assert_eq!(GpsProtocol::from_config("0"), GpsProtocol::Nmea);
        assert_eq!(GpsProtocol::from_config("1"), GpsProtocol::Taip);
        assert_eq!(GpsProtocol::from_config(""), GpsProtocol::Nmea);
        assert_eq!(GpsProtocol::from_config("garbage"), GpsProtocol::Nmea);
    }

    #[test]
    fn protocol_config_roundtrip() {
        assert_eq!(
            GpsProtocol::from_config(GpsProtocol::Nmea.as_config_str()),
            GpsProtocol::Nmea
        );
        assert_eq!(
            GpsProtocol::from_config(GpsProtocol::Taip.as_config_str()),
            GpsProtocol::Taip
        );
    }

    #[test]
    fn protocol_default_is_nmea() {
        assert_eq!(GpsProtocol::default(), GpsProtocol::Nmea);
    }

    // ---- private coordinate parsers ---------------------------------------

    #[test]
    fn nmea_coordinate_rejects_bad_input() {
        assert!(parse_nmea_coordinate("", "N").is_none());
        assert!(parse_nmea_coordinate("4807.038", "").is_none());
        // Dot too early: no room for the two-digit minutes field.
        assert!(parse_nmea_coordinate("1.5", "N").is_none());
        // Invalid hemisphere indicator.
        assert!(parse_nmea_coordinate("4807.038", "X").is_none());
    }

    #[test]
    fn nmea_coordinate_parses_every_hemisphere() {
        let n = parse_nmea_coordinate("4807.038", "N").unwrap();
        assert!((n - 48.1173).abs() < 0.001);
        let s = parse_nmea_coordinate("4807.038", "S").unwrap();
        assert!((s + 48.1173).abs() < 0.001);
        let e = parse_nmea_coordinate("01131.000", "E").unwrap();
        assert!((e - 11.516_667).abs() < 0.001);
        let w = parse_nmea_coordinate("01131.000", "W").unwrap();
        assert!((w + 11.516_667).abs() < 0.001);
    }

    #[test]
    fn nmea_coordinate_rejects_out_of_range() {
        // Degrees + minutes well outside [-180, 180].
        assert!(parse_nmea_coordinate("999999.000", "N").is_none());
    }

    #[test]
    fn nmea_coordinate_rejects_non_ascii_without_panic() {
        // 'é' (2 bytes) makes the `find('.')`-derived offset land off a char
        // boundary; the parser must reject it rather than panic.
        assert!(parse_nmea_coordinate("48\u{e9}7.038", "N").is_none());
    }

    #[test]
    fn nmea_location_rejects_non_ascii_coordinate_without_panic() {
        let line = "$GPGGA,123519,48\u{e9}7.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47";
        assert!(parse_nmea_location(line).is_none());
    }

    #[test]
    fn nmea_location_requires_dollar_prefix() {
        assert!(parse_nmea_location("GPGGA,123519,4807.038,N,01131.000,E,1").is_none());
    }

    #[test]
    fn nmea_location_rejects_short_sentence() {
        assert!(parse_nmea_location("$GPGGA,123519,4807.038").is_none());
    }

    #[test]
    fn taip_coordinate_rejects_bad_sign_and_length() {
        // Missing leading sign.
        assert!(parse_taip_coordinate("3739438", 2).is_none());
        // Too short for sign + degrees + 5-digit fraction.
        assert!(parse_taip_coordinate("+37", 2).is_none());
    }

    #[test]
    fn taip_coordinate_rejects_non_ascii_without_panic() {
        // 'é' (2 bytes) makes the byte-index slices land off a char boundary.
        assert!(parse_taip_coordinate("+3\u{e9}9438", 2).is_none());
    }

    // ---- position_changed -------------------------------------------------

    #[test]
    fn position_changed_detects_movement() {
        assert!(position_changed(None, 1.0, 2.0));
        let last = (1.0, 2.0);
        assert!(!position_changed(Some(&last), 1.0, 2.0));
        // Below the dedup threshold in both axes.
        assert!(!position_changed(Some(&last), 1.000_000_1, 2.000_000_1));
        // Above the threshold in latitude, then in longitude.
        assert!(position_changed(Some(&last), 1.01, 2.0));
        assert!(position_changed(Some(&last), 1.0, 2.01));
    }

    #[test]
    fn now_ms_is_positive() {
        assert!(now_ms() > 0);
    }

    // ---- NmeaGpsState defaults & accessors --------------------------------

    #[test]
    fn state_default_is_idle() {
        let state = NmeaGpsState::default();
        assert!(!state.is_running());
        assert!(state.latest_location().is_none());
        assert_eq!(state.status_text(), "");
        assert_eq!(state.fixes_received(), 0);
        assert!(!state.has_recent_fix(10_000));
    }

    #[test]
    fn set_status_overwrites_text() {
        let mut state = NmeaGpsState::default();
        state.set_status("hello".to_string());
        assert_eq!(state.status_text(), "hello");
    }

    #[test]
    fn start_client_rejects_empty_host() {
        let mut state = NmeaGpsState::default();
        assert!(state.start_client("   ", 1234, GpsProtocol::Nmea).is_err());
    }

    #[test]
    fn start_bluetooth_rejects_empty_path() {
        let mut state = NmeaGpsState::default();
        assert!(state.start_bluetooth("  ", GpsProtocol::Nmea).is_err());
    }

    // ---- poll_events via injected events ----------------------------------

    #[test]
    fn poll_events_records_fix() {
        let mut state = NmeaGpsState::default();
        let (tx, rx) = mpsc::channel();
        tx.send(NmeaGpsEvent::Fix {
            lat: 12.5,
            lon: -3.25,
        })
        .unwrap();
        state.event_rx = Some(rx);
        assert!(state.poll_events());
        let (lat, lon) = state.latest_location().expect("fix recorded");
        assert!((lat - 12.5).abs() < f64::EPSILON);
        assert!((lon + 3.25).abs() < f64::EPSILON);
        assert_eq!(state.fixes_received(), 1);
        assert!(state.status_text().contains("fix"));
    }

    #[test]
    fn poll_events_counts_multiple_fixes() {
        let mut state = NmeaGpsState::default();
        let (tx, rx) = mpsc::channel();
        tx.send(NmeaGpsEvent::Fix { lat: 1.0, lon: 1.0 }).unwrap();
        tx.send(NmeaGpsEvent::Fix { lat: 2.0, lon: 2.0 }).unwrap();
        state.event_rx = Some(rx);
        assert!(state.poll_events());
        assert_eq!(state.fixes_received(), 2);
        let (lat, _) = state.latest_location().unwrap();
        assert!((lat - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn poll_events_applies_status_then_keeps_it_on_end() {
        let mut state = NmeaGpsState::default();
        let (tx, rx) = mpsc::channel();
        tx.send(NmeaGpsEvent::Status("connected".to_string()))
            .unwrap();
        tx.send(NmeaGpsEvent::Ended).unwrap();
        state.event_rx = Some(rx);
        assert!(!state.poll_events());
        assert_eq!(state.status_text(), "connected");
        assert!(!state.is_running());
        assert!(state.event_rx.is_none());
    }

    #[test]
    fn poll_events_reports_error_text() {
        let mut state = NmeaGpsState::default();
        let (tx, rx) = mpsc::channel();
        tx.send(NmeaGpsEvent::Error("boom".to_string())).unwrap();
        state.event_rx = Some(rx);
        assert!(!state.poll_events());
        assert_eq!(state.status_text(), "boom");
    }

    #[test]
    fn poll_events_handles_disconnect_with_empty_status() {
        let mut state = NmeaGpsState::default();
        let (tx, rx) = mpsc::channel::<NmeaGpsEvent>();
        drop(tx);
        state.event_rx = Some(rx);
        assert!(!state.poll_events());
        assert_eq!(state.status_text(), "NMEA GPS connection ended.");
        assert!(state.event_rx.is_none());
    }

    #[test]
    fn poll_events_without_channel_is_noop() {
        let mut state = NmeaGpsState::default();
        assert!(!state.poll_events());
    }

    // ---- controller lifecycle & has_recent_fix ----------------------------

    #[test]
    fn has_recent_fix_requires_running_and_fresh_timestamp() {
        let mut state = NmeaGpsState {
            controller: Some(NmeaGpsController {
                stop_flag: Arc::new(AtomicBool::new(false)),
                worker: None,
            }),
            ..Default::default()
        };
        assert!(state.is_running());
        // Running but no fix yet.
        assert!(!state.has_recent_fix(10_000));
        state.last_fix_at_ms = now_ms();
        assert!(state.has_recent_fix(10_000));
        // Stale timestamp falls outside the window.
        state.last_fix_at_ms = now_ms() - 20_000;
        assert!(!state.has_recent_fix(10_000));
    }

    #[test]
    fn stop_clears_controller_and_resets_counters() {
        let mut state = NmeaGpsState {
            controller: Some(NmeaGpsController {
                stop_flag: Arc::new(AtomicBool::new(false)),
                worker: None,
            }),
            fixes_received: 7,
            last_fix_at_ms: now_ms(),
            ..Default::default()
        };
        state.stop();
        assert!(!state.is_running());
        assert_eq!(state.fixes_received(), 0);
        assert_eq!(state.status_text(), "NMEA GPS listener stopped.");
    }

    // ---- serial port helpers ----------------------------------------------

    #[test]
    fn list_helpers_do_not_panic() {
        let _ = list_serial_ports();
        let _ = list_bluetooth_ports();
    }

    #[test]
    fn is_likely_bluetooth_port_classifies_types() {
        use serialport::{SerialPortInfo, SerialPortType};
        let bt = SerialPortInfo {
            port_name: "/dev/cu.GPS".to_string(),
            port_type: SerialPortType::BluetoothPort,
        };
        assert!(is_likely_bluetooth_port(&bt));
        let pci = SerialPortInfo {
            port_name: "/dev/cu.serial".to_string(),
            port_type: SerialPortType::PciPort,
        };
        assert!(!is_likely_bluetooth_port(&pci));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_unknown_cu_port_is_bluetooth() {
        use serialport::{SerialPortInfo, SerialPortType};
        let unknown = SerialPortInfo {
            port_name: "/dev/cu.BY80125B39".to_string(),
            port_type: SerialPortType::Unknown,
        };
        assert!(is_likely_bluetooth_port(&unknown));
        let usb = SerialPortInfo {
            port_name: "/dev/cu.usbmodem1234".to_string(),
            port_type: SerialPortType::Unknown,
        };
        assert!(!is_likely_bluetooth_port(&usb));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_bluetooth_dev_name_filter() {
        assert!(is_likely_bluetooth_dev_name("cu.BY80125B39"));
        assert!(!is_likely_bluetooth_dev_name("cu.usbmodem1234"));
        assert!(!is_likely_bluetooth_dev_name("cu.usbserial-10"));
        assert!(!is_likely_bluetooth_dev_name("cu.Bluetooth-Incoming-Port"));
        assert!(!is_likely_bluetooth_dev_name("cu.BLTH"));
        assert!(!is_likely_bluetooth_dev_name("cu."));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn canonical_serial_port_name_trims_but_preserves_case() {
        assert_eq!(canonical_serial_port_name("  /dev/cu.Foo  "), "/dev/cu.Foo");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn canonical_serial_port_name_uppercases_on_windows() {
        assert_eq!(canonical_serial_port_name("  com3  "), "COM3");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_com_port_number_parses_suffix() {
        assert_eq!(parse_windows_com_port_number("COM3"), Some(3));
        assert_eq!(parse_windows_com_port_number("com12"), Some(12));
        assert_eq!(parse_windows_com_port_number("USB0"), None);
        assert_eq!(parse_windows_com_port_number("COM"), None);
    }

    #[test]
    fn pick_default_serial_port_returns_first_when_empty_or_single() {
        assert_eq!(pick_default_nmea_serial_port(&[]), None);
        assert_eq!(
            pick_default_nmea_serial_port(&["/dev/cu.a".to_string()]),
            Some("/dev/cu.a".to_string())
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn pick_default_serial_port_prefers_highest_com_number_on_windows() {
        let candidates = vec!["COM3".to_string(), "COM12".to_string(), "COM5".to_string()];
        assert_eq!(
            pick_default_nmea_serial_port(&candidates),
            Some("COM12".to_string())
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn pick_default_serial_port_uses_first_candidate_off_windows() {
        let candidates = vec!["/dev/cu.b".to_string(), "/dev/cu.a".to_string()];
        assert_eq!(
            pick_default_nmea_serial_port(&candidates),
            Some("/dev/cu.b".to_string())
        );
    }

    #[test]
    fn find_serial_port_index_matches_canonicalized_name() {
        let candidates = vec!["/dev/cu.a".to_string(), "/dev/cu.b".to_string()];
        assert_eq!(
            find_nmea_serial_port_index(&candidates, "  /dev/cu.b  "),
            Some(1)
        );
        assert_eq!(
            find_nmea_serial_port_index(&candidates, "/dev/cu.missing"),
            None
        );
    }

    // ---- worker integration (real localhost sockets) ---------------------

    #[test]
    fn tcp_server_worker_reports_fix() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};
        use std::time::{Duration, Instant};

        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        let mut state = NmeaGpsState::default();
        state.start("127.0.0.1", port, GpsProtocol::Nmea).unwrap();
        assert!(state.is_running());

        // The worker binds asynchronously; retry connecting briefly.
        let mut stream = None;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                stream = Some(s);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let mut stream = stream.expect("listener should accept a connection");
        stream
            .write_all(b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47\n")
            .unwrap();
        stream.flush().unwrap();

        let start = Instant::now();
        while state.latest_location().is_none() && start.elapsed() < Duration::from_secs(3) {
            state.poll_events();
            thread::sleep(Duration::from_millis(20));
        }
        let (lat, lon) = state
            .latest_location()
            .expect("server worker should report a fix");
        assert!((lat - 48.1173).abs() < 0.01, "lat={lat}");
        assert!((lon - 11.516_667).abs() < 0.01, "lon={lon}");
        state.stop();
        assert!(!state.is_running());
    }

    #[test]
    fn tcp_client_worker_reads_fix() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Stand in for the phone's NMEA server: accept once, stream a sentence.
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(
                    b"$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A\n",
                );
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(300));
            }
        });

        let mut state = NmeaGpsState::default();
        state
            .start_client("127.0.0.1", port, GpsProtocol::Nmea)
            .unwrap();

        let start = Instant::now();
        while state.latest_location().is_none() && start.elapsed() < Duration::from_secs(3) {
            state.poll_events();
            thread::sleep(Duration::from_millis(20));
        }
        state.stop();
        let _ = handle.join();
        assert!(
            state.latest_location().is_some(),
            "client worker should read at least one fix"
        );
    }

    // ---- pump_nmea_lines (serial read-loop seam) --------------------------

    #[test]
    fn pump_emits_fix_event_for_nmea_line() {
        use std::io::Cursor;
        let data = b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47\n".to_vec();
        let mut reader = Cursor::new(data);
        let stop = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let mut last_sent = None;
        let exit = pump_nmea_lines(&mut reader, &stop, &tx, GpsProtocol::Nmea, &mut last_sent);
        assert!(matches!(exit, ReadLoopExit::Disconnected));
        let NmeaGpsEvent::Fix { lat, lon } = rx.try_recv().expect("a fix event") else {
            panic!("expected a Fix event");
        };
        assert!((lat - 48.1173).abs() < 0.01, "lat={lat}");
        assert!((lon - 11.516_667).abs() < 0.01, "lon={lon}");
        assert!(last_sent.is_some());
    }

    #[test]
    fn pump_dedups_repeated_positions() {
        use std::io::Cursor;
        let line = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47\n";
        let data = format!("{line}{line}{line}").into_bytes();
        let mut reader = Cursor::new(data);
        let stop = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let mut last_sent = None;
        let exit = pump_nmea_lines(&mut reader, &stop, &tx, GpsProtocol::Nmea, &mut last_sent);
        assert!(matches!(exit, ReadLoopExit::Disconnected));
        // Three identical sentences collapse to a single Fix event.
        assert!(matches!(rx.try_recv(), Ok(NmeaGpsEvent::Fix { .. })));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn pump_reports_disconnect_on_eof() {
        use std::io::Cursor;
        let mut reader = Cursor::new(Vec::new());
        let stop = AtomicBool::new(false);
        let (tx, _rx) = mpsc::channel();
        let mut last_sent = None;
        let exit = pump_nmea_lines(&mut reader, &stop, &tx, GpsProtocol::Nmea, &mut last_sent);
        assert!(matches!(exit, ReadLoopExit::Disconnected));
    }

    #[test]
    fn pump_stops_without_reading_when_flag_preset() {
        use std::io::Cursor;
        let data = b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47\n".to_vec();
        let mut reader = Cursor::new(data);
        let stop = AtomicBool::new(true); // already stopped
        let (tx, rx) = mpsc::channel();
        let mut last_sent = None;
        let exit = pump_nmea_lines(&mut reader, &stop, &tx, GpsProtocol::Nmea, &mut last_sent);
        assert!(matches!(exit, ReadLoopExit::Stopped));
        assert!(rx.try_recv().is_err());
        assert!(last_sent.is_none());
        assert_eq!(reader.position(), 0, "must not read when already stopped");
    }

    // ---- Bluetooth scan flag / port opening / serial worker ----------------

    #[test]
    fn bluetooth_scan_flag_round_trips() {
        set_bluetooth_scan_enabled(true);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(bluetooth_scan_enabled());
        set_bluetooth_scan_enabled(false);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(!bluetooth_scan_enabled());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn listing_bluetooth_ports_with_scan_enabled_does_not_panic() {
        // Without bluetoothd the BlueZ lookup fails and contributes nothing.
        set_bluetooth_scan_enabled(true);
        let _ = list_bluetooth_ports();
        set_bluetooth_scan_enabled(false);
    }

    #[test]
    fn usable_serial_port_names_keeps_real_ports() {
        let info = |name: &str| serialport::SerialPortInfo {
            port_name: name.to_string(),
            port_type: serialport::SerialPortType::Unknown,
        };
        let names = usable_serial_port_names(vec![info("/dev/ttyUSB0"), info("/dev/ttyS0")]);
        assert!(names.contains(&"/dev/ttyUSB0".to_string()));
        // The kernel's placeholder UARTs only disappear on Linux.
        assert_eq!(
            names.contains(&"/dev/ttyS0".to_string()),
            !cfg!(target_os = "linux")
        );
    }

    #[test]
    fn likely_bluetooth_port_names_keeps_only_bluetooth_ports() {
        let ports = vec![
            serialport::SerialPortInfo {
                port_name: "COM5".to_string(),
                port_type: serialport::SerialPortType::BluetoothPort,
            },
            serialport::SerialPortInfo {
                port_name: "COM1".to_string(),
                port_type: serialport::SerialPortType::PciPort,
            },
        ];
        assert_eq!(likely_bluetooth_port_names(ports), vec!["COM5".to_string()]);
    }

    #[test]
    fn boxed_link_passes_a_reader_through() {
        let mut link = boxed_link(Ok::<_, String>(std::io::Cursor::new(b"$GPGGA".to_vec())))
            .expect("an Ok result yields a reader");
        let mut text = String::new();
        link.read_to_string(&mut text).unwrap();
        assert_eq!(text, "$GPGGA");
    }

    #[test]
    fn boxed_link_reports_the_error_text() {
        let err = boxed_link(Err::<std::io::Cursor<Vec<u8>>, _>("device unreachable"))
            .err()
            .expect("an Err result yields its message");
        assert_eq!(err, "device unreachable");
    }

    #[test]
    fn open_nmea_link_fails_for_a_missing_serial_port() {
        assert!(
            open_nmea_link(
                "/dev/definitely-not-a-serial-port",
                9600,
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn open_nmea_link_fails_for_an_unreachable_bluetooth_device() {
        assert!(
            open_nmea_link(
                "bt://00:00:00:00:00:01 Nobody",
                9600,
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }

    #[test]
    fn serial_worker_reports_unreachable_port_and_stops_on_request() {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let worker = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                nmea_serial_worker_with("gps", 9600, &stop, &tx, GpsProtocol::Nmea, |_, _| {
                    Err("device unreachable".to_string())
                });
            })
        };
        let first = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a status event");
        assert!(
            matches!(first, NmeaGpsEvent::Status(ref s) if s.contains("Retrying") && s.contains("device unreachable")),
            "status must carry the failure reason"
        );
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        let mut ended = false;
        while let Ok(event) = rx.try_recv() {
            ended |= matches!(event, NmeaGpsEvent::Ended);
        }
        assert!(ended, "worker must report that it ended");
    }

    #[test]
    fn serial_worker_streams_a_fix_then_reconnects_after_the_link_ends() {
        use std::io::Cursor;
        use std::sync::atomic::AtomicUsize;

        let stop = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let opens = AtomicUsize::new(0);
        let sentence = b"$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,47.0,M,,*47\n";
        nmea_serial_worker_with("gps", 9600, &stop, &tx, GpsProtocol::Nmea, |_, _| {
            let link: Box<dyn Read + Send> = if opens.fetch_add(1, Ordering::Relaxed) == 0 {
                // First link: one fix, then the device goes away (EOF).
                Box::new(Cursor::new(sentence.to_vec()))
            } else {
                // The worker reconnects; ask it to stop on the second link.
                stop.store(true, Ordering::Relaxed);
                Box::new(Cursor::new(Vec::new()))
            };
            Ok(link)
        });

        assert_eq!(opens.load(Ordering::Relaxed), 2, "must reconnect once");
        let events: Vec<NmeaGpsEvent> = rx.try_iter().collect();
        assert!(events.iter().any(
            |e| matches!(e, NmeaGpsEvent::Status(s) if s.contains("connected on gps (9600 baud)"))
        ));
        assert!(
            events.iter().any(
                |e| matches!(e, NmeaGpsEvent::Fix { lat, .. } if (lat - 48.1173).abs() < 0.01)
            )
        );
        assert!(matches!(events.last(), Some(NmeaGpsEvent::Ended)));
    }

    #[test]
    fn start_bluetooth_spawns_a_worker_that_stop_joins() {
        let mut state = NmeaGpsState::default();
        let status = state
            .start_bluetooth("/dev/definitely-not-a-serial-port", GpsProtocol::Nmea)
            .expect("worker starts even if the port is unreachable");
        assert!(status.contains("Connecting to Bluetooth GPS"), "{status}");
        assert!(state.is_running());
        // Wait until the worker really tried to open the port and reported why
        // it failed; stopping earlier would race with its first attempt.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !state.status_text().contains("Could not open")
            && std::time::Instant::now() < deadline
        {
            state.poll_events();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            state.status_text().contains("Could not open"),
            "{}",
            state.status_text()
        );
        state.stop();
        assert!(!state.is_running());
    }

    // ---- macOS noise / migration -------------------------------------------

    #[test]
    fn macos_noise_ports_are_recognised() {
        assert!(is_macos_noise_port("/dev/tty.BY80125B39"));
        assert!(is_macos_noise_port("/dev/cu.BLTH"));
        assert!(is_macos_noise_port("/dev/cu.Bluetooth-Incoming-Port"));
        assert!(!is_macos_noise_port("/dev/cu.BY80125B39"));
        assert!(!is_macos_noise_port("/dev/cu.usbserial-1410"));
    }

    #[test]
    fn migrate_raw_port_maps_to_matching_bt_entry() {
        let candidates = vec![
            "/dev/cu.usbserial-1".to_string(),
            "bt://FC:E8:15:26:9F:7D BY80125B39".to_string(),
        ];
        assert_eq!(
            migrate_raw_bluetooth_port("/dev/tty.BY80125B39", &candidates).as_deref(),
            Some("bt://FC:E8:15:26:9F:7D BY80125B39")
        );
        assert_eq!(
            migrate_raw_bluetooth_port("/dev/cu.BY80125B39", &candidates).as_deref(),
            Some("bt://FC:E8:15:26:9F:7D BY80125B39")
        );
        assert_eq!(
            migrate_raw_bluetooth_port("/dev/cu.Other", &candidates),
            None
        );
        assert_eq!(migrate_raw_bluetooth_port("COM7", &candidates), None);
    }

    // ---- is_linux_placeholder_uart -----------------------------------------

    #[cfg(target_os = "linux")]
    #[test]
    fn placeholder_uarts_are_filtered_on_linux_only() {
        assert!(is_linux_placeholder_uart("/dev/ttyS0"));
        assert!(is_linux_placeholder_uart("/dev/ttyS31"));
        assert!(!is_linux_placeholder_uart("/dev/ttyUSB0"));
        assert!(!is_linux_placeholder_uart("/dev/rfcomm0"));
    }

    // ---- new_serial_ports --------------------------------------------------

    #[test]
    fn new_serial_ports_returns_only_added_ports() {
        let before = vec!["/dev/cu.A".to_string(), "/dev/cu.B".to_string()];
        let now = vec![
            "/dev/cu.A".to_string(),
            "/dev/cu.C".to_string(),
            "/dev/cu.B".to_string(),
        ];
        assert_eq!(new_serial_ports(&before, &now), vec!["/dev/cu.C"]);
    }

    #[test]
    fn new_serial_ports_empty_when_unchanged_or_removed() {
        let before = vec!["/dev/cu.A".to_string(), "/dev/cu.B".to_string()];
        assert_eq!(
            new_serial_ports(&before, &before),
            [] as [std::string::String; 0]
        );
        assert_eq!(
            new_serial_ports(&before, &before[..1]),
            [] as [std::string::String; 0]
        );
    }

    #[test]
    fn new_serial_ports_treats_empty_baseline_as_all_new() {
        let now = vec!["/dev/rfcomm0".to_string()];
        assert_eq!(new_serial_ports(&[], &now), now);
    }
}
