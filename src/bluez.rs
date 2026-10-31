//! Linux Bluetooth SPP support through `BlueZ`.
//!
//! Paired devices that advertise the Serial Port Profile are exposed to the
//! rest of the app as pseudo-ports (see [`crate::bt_port`]), so no
//! `/dev/rfcomm*` node (and no `sudo rfcomm bind`) is required.
//!
//! Connecting does not use `BlueZ`'s profile machinery, which pages the device
//! and looks up its channel in one step and times out on some devices.
//! Instead the app:
//!
//! 1. brings the radio link up through `BlueZ` (as `bluetoothctl connect` does),
//! 2. asks the device for its serial channel with a small SDP query, and
//! 3. opens that RFCOMM channel with a plain socket.
//!
//! Pairing itself is done in the desktop's Bluetooth settings, which own the
//! pairing agent.

use std::collections::HashSet;
use std::io::{self, Read};
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use bluer::l2cap::SeqPacket;
use bluer::rfcomm::{SocketAddr, Stream};
use bluer::{Address, AddressType, Session, Uuid};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::runtime::{Builder, Runtime};

/// Serial Port Profile service class UUID (`00001101-0000-1000-8000-00805F9B34FB`).
const SPP_UUID: Uuid = Uuid::from_u128(0x0000_1101_0000_1000_8000_0080_5f9b_34fb);

/// How long a single `read` waits before reporting a timeout, so the worker
/// loop can observe its stop flag.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound for enumerating devices; this runs on the UI thread.
const LIST_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound for establishing the RFCOMM connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// L2CAP channel (PSM) of the Service Discovery Protocol.
const SDP_PSM: u16 = 1;

/// Upper bound for the SDP lookup of the serial channel.
const SDP_TIMEOUT: Duration = Duration::from_secs(8);

/// How often a connect attempt checks whether it was cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// How long a background device scan stays fresh.
const SCAN_TTL: Duration = Duration::from_secs(2);

/// A paired device that offers the Serial Port Profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SppDevice {
    /// Bluetooth address, e.g. `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    /// Friendly name reported by `BlueZ`.
    pub name: String,
}

impl SppDevice {
    /// The `bt://` entry for this device in the serial-port list.
    #[must_use]
    pub fn port_name(&self) -> String {
        crate::bt_port::port_name(&self.address, &self.name)
    }
}

fn runtime() -> Result<Runtime> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create Bluetooth runtime")
}

/// Lists paired devices that expose the Serial Port Profile.
///
/// # Returns
///
/// * `Result<Vec<SppDevice>>` - The devices; an error if `bluetoothd` or an
///   adapter is unavailable.
pub fn list_paired_spp_devices() -> Result<Vec<SppDevice>> {
    let rt = runtime()?;
    rt.block_on(async {
        tokio::time::timeout(LIST_TIMEOUT, list_async())
            .await
            .map_err(|_| anyhow!("timed out talking to bluetoothd"))?
    })
}

struct ScanCache {
    devices: Vec<SppDevice>,
    fetched: Option<Instant>,
    in_flight: bool,
}

static SCAN: Mutex<ScanCache> = Mutex::new(ScanCache {
    devices: Vec::new(),
    fetched: None,
    in_flight: false,
});

/// Returns the paired SPP devices from the last background scan and starts a
/// new scan when that result is stale.
///
/// Unlike [`list_paired_spp_devices`] this never waits on D-Bus, so it is safe
/// to call from the UI thread. The first call returns an empty list; callers
/// that poll pick the devices up once the scan has finished.
#[must_use]
pub fn cached_paired_spp_devices() -> Vec<SppDevice> {
    let mut cache = SCAN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stale = cache.fetched.is_none_or(|t| t.elapsed() >= SCAN_TTL);
    if stale && !cache.in_flight {
        cache.in_flight = true;
        thread::spawn(|| {
            let devices = list_paired_spp_devices().unwrap_or_default();
            let mut cache = SCAN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.devices = devices;
            cache.fetched = Some(Instant::now());
            cache.in_flight = false;
        });
    }
    cache.devices.clone()
}

/// `true` if a device's advertised service UUIDs include the Serial Port Profile.
fn offers_spp(uuids: Option<&HashSet<Uuid>>) -> bool {
    uuids.is_some_and(|uuids| uuids.contains(&SPP_UUID))
}

async fn list_async() -> Result<Vec<SppDevice>> {
    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    let mut devices = Vec::new();
    for address in adapter.device_addresses().await? {
        let device = adapter.device(address)?;
        if !device.is_paired().await.unwrap_or(false) {
            continue;
        }
        if !offers_spp(device.uuids().await.ok().flatten().as_ref()) {
            continue;
        }
        let name = device.alias().await.unwrap_or_else(|_| address.to_string());
        devices.push(SppDevice {
            address: address.to_string(),
            name,
        });
    }
    Ok(devices)
}

/// A connected RFCOMM stream that can be read synchronously.
///
/// Reads time out with [`io::ErrorKind::TimedOut`] after [`READ_TIMEOUT`],
/// matching the behavior of a serial port opened with a timeout.
pub struct BluezLink {
    // Field order matters: the stream must drop before the runtime that drives it.
    stream: Stream,
    _session: Session,
    rt: Runtime,
}

/// Connects to the SPP service of a paired device.
///
/// # Arguments
///
/// * `address` - Bluetooth address, e.g. `AA:BB:CC:DD:EE:FF`.
/// * `stop` - Set to abandon the attempt promptly (within about 100 ms).
///
/// # Returns
///
/// * `Result<BluezLink>` - A readable link, or an error describing why the
///   connection failed (device off, out of range, not paired, cancelled, ...).
pub fn connect(address: &str, stop: &AtomicBool) -> Result<BluezLink> {
    let address = Address::from_str(address).map_err(|e| anyhow!("invalid address: {e}"))?;
    let rt = runtime()?;
    let (session, stream) = rt.block_on(async {
        tokio::select! {
            result = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(address)) => {
                result.map_err(|_| anyhow!("timed out connecting to {address}"))?
            }
            () = wait_for_stop(stop) => Err(anyhow!("connection attempt cancelled")),
        }
    })?;
    Ok(BluezLink {
        stream,
        _session: session,
        rt,
    })
}

async fn wait_for_stop(stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(CANCEL_POLL).await;
    }
}

async fn connect_async(address: Address) -> Result<(Session, Stream)> {
    let session = Session::new().await.context("connecting to bluetoothd")?;
    let adapter = session
        .default_adapter()
        .await
        .context("finding the Bluetooth adapter")?;
    // Only power the adapter on when it is off; redundant mode changes can
    // fail on some controllers and would abort an otherwise good connect.
    if !adapter.is_powered().await? {
        adapter
            .set_powered(true)
            .await
            .context("powering on the adapter")?;
    }
    let device = adapter.device(address)?;

    ensure_link(&device).await?;

    let channel = tokio::time::timeout(SDP_TIMEOUT, find_spp_channel(address))
        .await
        .map_err(|_| anyhow!("timed out asking {address} for its serial channel"))??;
    let stream = Stream::connect(SocketAddr::new(address, channel))
        .await
        .with_context(|| format!("opening serial channel {channel} on {address}"))?;
    Ok((session, stream))
}

/// Asks the device, over SDP, which RFCOMM channel carries its Serial Port
/// Profile. `BlueZ`'s own profile machinery does this too, but it pages the
/// device in the same step and times out on some devices.
async fn find_spp_channel(address: Address) -> Result<u8> {
    let sdp = SeqPacket::connect(bluer::l2cap::SocketAddr::new(
        address,
        AddressType::BrEdr,
        SDP_PSM,
    ))
    .await
    .context("opening an SDP connection")?;
    sdp.send(&sdp_serial_channel_request(1))
        .await
        .context("sending the SDP request")?;
    let mut buf = vec![0u8; sdp.recv_mtu().unwrap_or(4096).max(512)];
    let n = sdp.recv(&mut buf).await.context("reading the SDP answer")?;
    parse_rfcomm_channel(&buf[..n])
        .ok_or_else(|| anyhow!("{address} does not offer a serial port service"))
}

/// Builds an SDP `ServiceSearchAttributeRequest` for the Serial Port Profile,
/// asking for its `ProtocolDescriptorList` (which holds the RFCOMM channel).
fn sdp_serial_channel_request(transaction_id: u16) -> Vec<u8> {
    let mut pdu = vec![0x06];
    pdu.extend_from_slice(&transaction_id.to_be_bytes());
    let params: [u8; 13] = [
        0x35, 0x03, 0x19, 0x11, 0x01, // service search pattern: UUID16 0x1101
        0xff, 0xff, // maximum attribute byte count
        0x35, 0x03, 0x09, 0x00, 0x04, // attribute list: ProtocolDescriptorList (0x0004)
        0x00, // no continuation
    ];
    pdu.extend_from_slice(&u16::try_from(params.len()).unwrap_or(0).to_be_bytes());
    pdu.extend_from_slice(&params);
    pdu
}

/// Extracts the RFCOMM channel from an SDP `ServiceSearchAttributeResponse`.
///
/// The channel sits in the `ProtocolDescriptorList` as `UUID16 0x0003`
/// (RFCOMM) followed by a one-byte unsigned integer.
fn parse_rfcomm_channel(response: &[u8]) -> Option<u8> {
    // PDU id 0x07, transaction id, parameter length, attribute lists byte count.
    if response.len() < 7 || response[0] != 0x07 {
        return None;
    }
    response[7..]
        .windows(5)
        .find(|w| w[..4] == [0x19, 0x00, 0x03, 0x08])
        .map(|w| w[4])
}

/// Brings the radio link to `device` up the way `bluetoothctl connect` does.
///
/// Starting our profile on a cold link makes `BlueZ` page the device and look
/// up its serial channel in one step, which times out on some devices. Opening
/// the link first (and letting `BlueZ` resolve the services) does not. The
/// "no profile to connect to" error that `Device.Connect` returns for a
/// serial-only device is expected: the link still comes up.
async fn ensure_link(device: &bluer::Device) -> Result<()> {
    if device.is_connected().await? {
        return Ok(());
    }
    let _ = device.connect().await;
    if device.is_connected().await? {
        Ok(())
    } else {
        Err(anyhow!(
            "could not reach the device; is it powered on, in range and not connected to another computer?"
        ))
    }
}

/// Reads from `stream` on `rt`, reporting [`io::ErrorKind::TimedOut`] if
/// nothing arrives within `timeout`.
fn read_with_timeout<S: AsyncRead + Unpin>(
    rt: &Runtime,
    stream: &mut S,
    buf: &mut [u8],
    timeout: Duration,
) -> io::Result<usize> {
    rt.block_on(async {
        match tokio::time::timeout(timeout, stream.read(buf)).await {
            Ok(result) => result,
            Err(_) => Err(io::ErrorKind::TimedOut.into()),
        }
    })
}

impl Read for BluezLink {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Self { stream, rt, .. } = self;
        read_with_timeout(rt, stream, buf, READ_TIMEOUT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn spp_device_port_name_combines_address_and_name() {
        let device = SppDevice {
            address: "AA:BB:CC:DD:EE:FF".to_string(),
            name: "GPS Puck".to_string(),
        };
        assert_eq!(device.port_name(), "bt://AA:BB:CC:DD:EE:FF GPS Puck");
    }

    #[test]
    fn spp_uuid_is_the_serial_port_profile() {
        assert_eq!(SPP_UUID.to_string(), "00001101-0000-1000-8000-00805f9b34fb");
    }

    #[test]
    fn offers_spp_requires_the_spp_uuid() {
        let with: HashSet<Uuid> = [SPP_UUID, Uuid::from_u128(1)].into_iter().collect();
        let without: HashSet<Uuid> = [Uuid::from_u128(1)].into_iter().collect();
        assert!(offers_spp(Some(&with)));
        assert!(!offers_spp(Some(&without)));
        assert!(!offers_spp(None));
    }

    #[test]
    fn sdp_request_asks_for_the_serial_port_protocol_list() {
        assert_eq!(
            sdp_serial_channel_request(1),
            [
                0x06, 0x00, 0x01, 0x00, 0x0d, 0x35, 0x03, 0x19, 0x11, 0x01, 0xff, 0xff, 0x35, 0x03,
                0x09, 0x00, 0x04, 0x00
            ]
        );
    }

    #[test]
    fn rfcomm_channel_is_read_from_the_protocol_list() {
        // ServiceSearchAttributeResponse with L2CAP, then RFCOMM channel 2.
        let response = [
            0x07, 0x00, 0x01, 0x00, 0x18, 0x00, 0x15, 0x35, 0x13, 0x35, 0x11, 0x09, 0x00, 0x04,
            0x35, 0x0c, 0x35, 0x03, 0x19, 0x01, 0x00, 0x35, 0x05, 0x19, 0x00, 0x03, 0x08, 0x02,
            0x00,
        ];
        assert_eq!(parse_rfcomm_channel(&response), Some(2));
    }

    #[test]
    fn rfcomm_channel_is_absent_from_errors_and_other_records() {
        // An SDP error response.
        assert_eq!(parse_rfcomm_channel(&[0x01, 0, 1, 0, 2, 0, 3]), None);
        // Truncated.
        assert_eq!(parse_rfcomm_channel(&[0x07, 0, 1]), None);
        // A record without an RFCOMM entry (L2CAP only).
        let l2cap_only = [
            0x07, 0x00, 0x01, 0x00, 0x11, 0x00, 0x0e, 0x35, 0x0c, 0x35, 0x0a, 0x09, 0x00, 0x04,
            0x35, 0x03, 0x19, 0x01, 0x00, 0x00,
        ];
        assert_eq!(parse_rfcomm_channel(&l2cap_only), None);
    }

    #[test]
    fn read_with_timeout_returns_available_bytes() {
        let rt = runtime().unwrap();
        let (mut tx, mut rx) = tokio::io::duplex(64);
        rt.block_on(tx.write_all(b"$GPGGA\n")).unwrap();
        let mut buf = [0u8; 16];
        let n = read_with_timeout(&rt, &mut rx, &mut buf, Duration::from_secs(1)).unwrap();
        assert_eq!(&buf[..n], b"$GPGGA\n");
    }

    #[test]
    fn read_with_timeout_reports_timeout_when_idle() {
        let rt = runtime().unwrap();
        let (_tx, mut rx) = tokio::io::duplex(64);
        let mut buf = [0u8; 16];
        let err = read_with_timeout(&rt, &mut rx, &mut buf, Duration::from_millis(30)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn read_with_timeout_reports_eof_when_peer_closes() {
        let rt = runtime().unwrap();
        let (tx, mut rx) = tokio::io::duplex(64);
        drop(tx);
        let mut buf = [0u8; 16];
        let n = read_with_timeout(&rt, &mut rx, &mut buf, Duration::from_secs(1)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn connect_rejects_invalid_addresses() {
        let err = connect("not-an-address", &AtomicBool::new(false))
            .err()
            .expect("must fail");
        assert!(err.to_string().contains("invalid address"), "{err}");
    }

    #[test]
    fn connect_to_an_unreachable_device_fails_instead_of_hanging() {
        // Without bluetoothd this fails at the D-Bus session; with it, BlueZ
        // rejects the unknown device. Either way: an error, not a hang.
        assert!(connect("00:00:00:00:00:01", &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn a_cancelled_connect_returns_promptly() {
        let started = Instant::now();
        assert!(connect("00:00:00:00:00:01", &AtomicBool::new(true)).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn wait_for_stop_returns_once_the_flag_is_set() {
        let rt = runtime().unwrap();
        let stop = AtomicBool::new(true);
        rt.block_on(wait_for_stop(&stop));
    }

    #[test]
    fn cached_listing_never_blocks_and_fills_in_later() {
        let started = Instant::now();
        let _ = cached_paired_spp_devices();
        assert!(started.elapsed() < Duration::from_millis(500));
        // Let the background scan finish; a later call must not panic or block.
        thread::sleep(Duration::from_millis(300));
        let _ = cached_paired_spp_devices();
    }

    #[test]
    fn listing_devices_does_not_panic_without_bluetoothd() {
        // Err without a daemon, Ok(possibly empty) with one.
        let _ = list_paired_spp_devices();
    }
}
