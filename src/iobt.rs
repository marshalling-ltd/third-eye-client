//! macOS Bluetooth SPP support through `IOBluetooth`.
//!
//! The RFCOMM channel is opened by the app itself, so no `/dev/cu.*` node and
//! no "Connect" click in System Settings is needed: opening the baseband link
//! and the channel is what brings the device up.
//!
//! `IOBluetooth` only opens RFCOMM channels from a process's main thread, which
//! in the app belongs to the UI. Each [`IobtLink`] therefore runs the channel in
//! a helper child process (this executable started with [`HELPER_FLAG`], see
//! [`run_helper`]) and reads the bytes it forwards over a pipe.
//!
//! Access is gated by macOS privacy (TCC): the running bundle needs
//! `NSBluetoothAlwaysUsageDescription` in its `Info.plist`.

use std::ffi::c_void;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
use objc2_io_bluetooth::{IOBluetoothDevice, IOBluetoothRFCOMMChannel, IOBluetoothSDPUUID};

use crate::bt_port::normalize_address;

/// Serial Port Profile 16-bit service class id.
const SPP_UUID16: u16 = 0x1101;

/// How long a single `read` waits before reporting a timeout, so the worker
/// loop can observe its stop flag.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound for bringing up the link and opening the channel.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a stopped helper gets to close the link before it is killed.
const HELPER_STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the helper waits for an SDP answer when none is cached.
const SDP_WAIT_SECS: f64 = 3.0;

/// Run-loop slice between stop-flag checks in the helper.
const PUMP_SLICE_SECS: f64 = 0.1;

/// A paired device that is (or may be) a serial-port GPS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SppDevice {
    /// Normalized address, e.g. `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    /// Name reported by macOS.
    pub name: String,
    /// The `/dev/cu.*` node macOS created for this device, if any.
    pub serial_node: Option<String>,
}

impl SppDevice {
    /// The `bt://` entry for this device in the serial-port list.
    #[must_use]
    pub fn port_name(&self) -> String {
        crate::bt_port::port_name(&self.address, &self.name)
    }
}

enum Event {
    Data(Vec<u8>),
    Closed,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = Sender<Event>]
    #[name = "ThirdEyeRfcommDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(rfcommChannelData:data:length:))]
        fn channel_data(&self, _channel: *mut AnyObject, data: *mut c_void, length: usize) {
            if data.is_null() || length == 0 {
                return;
            }
            let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
            let _ = self.ivars().send(Event::Data(bytes.to_vec()));
        }

        #[unsafe(method(rfcommChannelOpenComplete:status:))]
        fn channel_open_complete(&self, _channel: *mut AnyObject, _status: i32) {}

        #[unsafe(method(rfcommChannelClosed:))]
        fn channel_closed(&self, _channel: *mut AnyObject) {
            let _ = self.ivars().send(Event::Closed);
        }
    }
);

impl Delegate {
    fn new(tx: Sender<Event>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(tx);
        unsafe { msg_send![super(this), init] }
    }
}

/// Runs this thread's run loop for `seconds`, delivering `IOBluetooth`
/// callbacks.
///
/// `runMode:beforeDate:` returns immediately when the run loop has no sources
/// attached (before a channel is open), so this sleeps off the remainder
/// instead of busy-spinning or cutting waits short.
fn pump(seconds: f64) {
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        let slice = remaining.min(Duration::from_secs_f64(PUMP_SLICE_SECS));
        let started = Instant::now();
        unsafe {
            let until = NSDate::dateWithTimeIntervalSinceNow(slice.as_secs_f64());
            NSRunLoop::currentRunLoop().runMode_beforeDate(NSDefaultRunLoopMode, &until);
        }
        if let Some(rest) = slice.checked_sub(started.elapsed()) {
            thread::sleep(rest);
        }
    }
}

/// Candidate `/dev/cu.*` node for a device name: macOS replaces characters
/// that are awkward in paths, so both the raw and a sanitized form are tried.
fn serial_node_for(name: &str) -> Option<String> {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    [name, sanitized.as_str()]
        .into_iter()
        .map(|n| format!("/dev/cu.{n}"))
        .find(|path| Path::new(path).exists())
}

/// Lists paired devices that look like serial-port devices: macOS created a
/// `/dev/cu.*` node for them, or an SPP service record is already cached.
///
/// The first call may show the macOS Bluetooth permission prompt.
///
/// # Returns
///
/// * `Vec<SppDevice>` - The devices; empty when none are paired or access is
///   denied.
#[must_use]
pub fn list_paired_spp_devices() -> Vec<SppDevice> {
    let Some(paired) = (unsafe { IOBluetoothDevice::pairedDevices() }) else {
        return Vec::new();
    };
    let uuid = unsafe { IOBluetoothSDPUUID::uuid16(SPP_UUID16) };
    paired
        .iter()
        .filter_map(|obj| obj.downcast::<IOBluetoothDevice>().ok())
        .filter_map(|device| unsafe {
            let address = normalize_address(&device.addressString()?.to_string())?;
            let name = device
                .nameOrAddress()
                .map_or_else(|| address.clone(), |n| n.to_string());
            let serial_node = serial_node_for(&name);
            let has_spp_record = uuid
                .as_deref()
                .is_some_and(|u| device.getServiceRecordForUUID(Some(u)).is_some());
            (serial_node.is_some() || has_spp_record).then_some(SppDevice {
                address,
                name,
                serial_node,
            })
        })
        .collect()
}

struct OpenChannel {
    device: Retained<IOBluetoothDevice>,
    channel: Retained<IOBluetoothRFCOMMChannel>,
    // The channel only holds a weak reference to its delegate.
    _delegate: Retained<Delegate>,
}

/// How many times to tear the link down and redial when the channel open is
/// rejected. A stale baseband link (left by a crashed process, or held by the
/// system) makes the first attempt fail; a fresh link fixes it.
const OPEN_ATTEMPTS: u32 = 3;

/// Settle time after closing a link before dialing again.
const REDIAL_PAUSE_SECS: f64 = 1.5;

fn open_channel(address: &str, tx: &Sender<Event>) -> Result<OpenChannel, String> {
    unsafe {
        // Use the instance from the paired list (as System Settings does), not
        // a fresh `deviceWithAddressString` one.
        let device = IOBluetoothDevice::pairedDevices()
            .into_iter()
            .flat_map(|array| array.iter().collect::<Vec<_>>())
            .filter_map(|obj| obj.downcast::<IOBluetoothDevice>().ok())
            .find(|d| {
                d.addressString()
                    .and_then(|a| normalize_address(&a.to_string()))
                    .as_deref()
                    == Some(address)
            })
            .ok_or_else(|| format!("{address} is not a paired device"))?;
        let uuid = IOBluetoothSDPUUID::uuid16(SPP_UUID16)
            .ok_or_else(|| "could not build the SPP UUID".to_string())?;

        let mut last_error = String::new();
        for attempt in 0..OPEN_ATTEMPTS {
            if attempt > 0 {
                // Drop whatever link state we inherited and dial fresh.
                device.closeConnection();
                pump(REDIAL_PAUSE_SECS);
            }

            // Bringing the baseband link up first is what makes SDP and the
            // channel open succeed.
            if !device.isConnected() {
                let status = device.openConnection();
                if status != 0 {
                    last_error = format!(
                        "could not reach {address}; is it powered on and in range? (IOReturn {status:#x})"
                    );
                    continue;
                }
            }

            let mut record = device.getServiceRecordForUUID(Some(&uuid));
            if record.is_none() {
                device.performSDPQuery(None);
                pump(SDP_WAIT_SECS);
                record = device.getServiceRecordForUUID(Some(&uuid));
            }
            let Some(record) = record else {
                last_error = format!("{address} offers no serial port service");
                continue;
            };
            let mut channel_id: u8 = 0;
            if record.getRFCOMMChannelID(&raw mut channel_id) != 0 {
                last_error = format!("no RFCOMM channel in the SPP record of {address}");
                continue;
            }

            let delegate = Delegate::new(tx.clone());
            let mut channel: Option<Retained<IOBluetoothRFCOMMChannel>> = None;
            let status = device.openRFCOMMChannelSync_withChannelID_delegate(
                Some(&mut channel),
                channel_id,
                Some(&delegate),
            );
            match channel {
                Some(channel) if status == 0 => {
                    return Ok(OpenChannel {
                        device,
                        channel,
                        _delegate: delegate,
                    });
                }
                _ => {
                    last_error = format!(
                        "could not open serial channel {channel_id} on {address} (IOReturn {status:#x})"
                    );
                }
            }
        }
        // Don't leave a half-open link behind: it makes the next attempt fail.
        device.closeConnection();
        Err(last_error)
    }
}

/// Command-line flag that makes the executable act as the Bluetooth helper.
pub const HELPER_FLAG: &str = "--bluetooth-helper";

/// Runs the Bluetooth helper: opens the device's serial channel and writes the
/// bytes it sends to stdout until stdin closes or the channel drops.
///
/// `IOBluetooth` only opens RFCOMM channels from a process's main thread, and
/// the app's main thread belongs to the UI, so the app runs this in a child
/// process (see [`connect`]). Status goes to stderr as lines: `ready` once the
/// channel is open, or `error <message>`.
///
/// # Arguments
///
/// * `address` - Bluetooth address of the paired device.
///
/// # Returns
///
/// * `i32` - Process exit code: 0 after a clean stop or a lost link, non-zero
///   if the channel could not be opened.
#[must_use]
pub fn run_helper(address: &str) -> i32 {
    use std::io::Write;

    let Some(address) = normalize_address(address) else {
        eprintln!("error invalid Bluetooth address");
        return 2;
    };

    // The parent closing our stdin (or dying) is the signal to shut down.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut sink = [0u8; 64];
            while io::stdin().read(&mut sink).is_ok_and(|n| n > 0) {}
            stop.store(true, Ordering::Relaxed);
        });
    }

    let (tx, rx) = mpsc::channel();
    let open = match open_channel(&address, &tx) {
        Ok(open) => open,
        Err(message) => {
            eprintln!("error {message}");
            return 1;
        }
    };
    eprintln!("ready");

    let mut out = io::stdout().lock();
    let mut link_lost = false;
    while !stop.load(Ordering::Relaxed) && !link_lost && unsafe { open.channel.isOpen() } {
        unsafe {
            let until = NSDate::dateWithTimeIntervalSinceNow(PUMP_SLICE_SECS);
            NSRunLoop::currentRunLoop().runMode_beforeDate(NSDefaultRunLoopMode, &until);
        }
        let mut got_any = false;
        while let Ok(event) = rx.try_recv() {
            got_any = true;
            match event {
                Event::Data(bytes) => {
                    if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
                        // The app went away.
                        link_lost = true;
                        break;
                    }
                }
                Event::Closed => link_lost = true,
            }
        }
        if !got_any {
            // With no run-loop source the call above returns at once.
            thread::sleep(Duration::from_millis(5));
        }
    }
    unsafe {
        open.channel.closeChannel();
        open.device.closeConnection();
    }
    0
}

/// A connected RFCOMM channel that can be read synchronously.
///
/// The channel lives in a helper child process; this reads the bytes it
/// forwards. Reads time out with [`io::ErrorKind::TimedOut`] after two
/// seconds, like a serial port opened with a timeout. `Ok(0)` (EOF) means the
/// link was lost. Dropping the link stops the helper, which closes the channel
/// and the baseband connection.
pub struct IobtLink {
    rx: Receiver<Event>,
    pending: Vec<u8>,
    closed: bool,
    child: Child,
    stdin: Option<ChildStdin>,
}

/// Connects to the SPP service of a paired device by starting a helper
/// process (this executable with [`HELPER_FLAG`]).
///
/// # Arguments
///
/// * `address` - Bluetooth address, e.g. `AA:BB:CC:DD:EE:FF`.
///
/// # Returns
///
/// * `Result<IobtLink>` - A readable link, or an error saying why the device
///   could not be reached.
pub fn connect(address: &str) -> Result<IobtLink> {
    let address = normalize_address(address).ok_or_else(|| anyhow!("invalid Bluetooth address"))?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg(HELPER_FLAG)
        .arg(&address)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("could not start the Bluetooth helper: {e}"))?;
    let stdin = child.stdin.take();
    let (Some(mut stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return Err(anyhow!("could not attach to the Bluetooth helper"));
    };

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(Event::Data(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
        let _ = tx.send(Event::Closed);
    });

    let (ready_tx, ready_rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line == "ready" {
                let _ = ready_tx.send(Ok(()));
            } else if let Some(message) = line.strip_prefix("error ") {
                let _ = ready_tx.send(Err(message.to_string()));
            }
        }
        let _ = ready_tx.send(Err("the Bluetooth helper exited unexpectedly".to_string()));
    });

    let mut link = IobtLink {
        rx,
        pending: Vec::new(),
        closed: false,
        child,
        stdin,
    };
    match ready_rx.recv_timeout(CONNECT_TIMEOUT) {
        Ok(Ok(())) => Ok(link),
        Ok(Err(message)) => {
            link.shutdown();
            Err(anyhow!(message))
        }
        Err(_) => {
            link.shutdown();
            Err(anyhow!("timed out connecting to the Bluetooth device"))
        }
    }
}

impl IobtLink {
    /// Asks the helper to stop (by closing its stdin) and reaps it, killing it
    /// if it does not exit in time.
    fn shutdown(&mut self) {
        self.stdin = None;
        let deadline = Instant::now() + HELPER_STOP_TIMEOUT;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Read for IobtLink {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            if self.closed {
                return Ok(0);
            }
            match self.rx.recv_timeout(READ_TIMEOUT) {
                Ok(Event::Data(bytes)) => self.pending = bytes,
                Ok(Event::Closed) | Err(RecvTimeoutError::Disconnected) => {
                    self.closed = true;
                    return Ok(0);
                }
                Err(RecvTimeoutError::Timeout) => return Err(io::ErrorKind::TimedOut.into()),
            }
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }
}

impl Drop for IobtLink {
    fn drop(&mut self) {
        self.shutdown();
    }
}
