//! Linux Bluetooth SPP support through `BlueZ` (D-Bus).
//!
//! Paired devices that advertise the Serial Port Profile are exposed to the
//! rest of the app as pseudo-ports (see [`crate::bt_port`]), so no
//! `/dev/rfcomm*` node (and no `sudo rfcomm bind`) is required. Pairing itself
//! is still done by the desktop's Bluetooth settings, which own the pairing
//! agent.

use std::collections::HashSet;
use std::io::{self, Read};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bluer::rfcomm::{Profile, Role, Stream};
use bluer::{Address, Session, Uuid};
use futures::StreamExt;
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

/// `true` if a device's advertised service UUIDs include the Serial Port Profile.
fn offers_spp(uuids: Option<&HashSet<Uuid>>) -> bool {
    uuids.is_some_and(|uuids| uuids.contains(&SPP_UUID))
}

/// The profile we register so `BlueZ` hands us the connected RFCOMM socket.
fn spp_profile() -> Profile {
    Profile {
        uuid: SPP_UUID,
        name: Some("Third Eye GPS".to_string()),
        role: Some(Role::Client),
        require_authentication: Some(false),
        require_authorization: Some(false),
        auto_connect: Some(false),
        ..Profile::default()
    }
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
///
/// # Returns
///
/// * `Result<BluezLink>` - A readable link, or an error describing why the
///   connection failed (device off, out of range, not paired, ...).
pub fn connect(address: &str) -> Result<BluezLink> {
    let address = Address::from_str(address).map_err(|e| anyhow!("invalid address: {e}"))?;
    let rt = runtime()?;
    let (session, stream) = rt.block_on(async {
        tokio::time::timeout(CONNECT_TIMEOUT, connect_async(address))
            .await
            .map_err(|_| anyhow!("timed out connecting to {address}"))?
    })?;
    Ok(BluezLink {
        stream,
        _session: session,
        rt,
    })
}

async fn connect_async(address: Address) -> Result<(Session, Stream)> {
    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;
    let device = adapter.device(address)?;

    let mut handle = session.register_profile(spp_profile()).await?;

    // BlueZ only completes ConnectProfile once we accept the incoming
    // NewConnection, so both futures must make progress together.
    let (connected, request) = tokio::join!(device.connect_profile(&SPP_UUID), handle.next());
    connected?;
    let request = request.ok_or_else(|| anyhow!("BlueZ closed the profile before connecting"))?;
    let stream = request.accept()?;
    Ok((session, stream))
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
    fn spp_profile_is_an_unauthenticated_client() {
        let profile = spp_profile();
        assert_eq!(profile.uuid, SPP_UUID);
        assert_eq!(profile.role, Some(Role::Client));
        assert_eq!(profile.require_authentication, Some(false));
        assert_eq!(profile.auto_connect, Some(false));
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
        let err = connect("not-an-address").err().expect("must fail");
        assert!(err.to_string().contains("invalid address"), "{err}");
    }

    #[test]
    fn connect_to_an_unreachable_device_fails_instead_of_hanging() {
        // Without bluetoothd this fails at the D-Bus session; with it, BlueZ
        // rejects the unknown device. Either way: an error, not a hang.
        assert!(connect("00:00:00:00:00:01").is_err());
    }

    #[test]
    fn listing_devices_does_not_panic_without_bluetoothd() {
        // Err without a daemon, Ok(possibly empty) with one.
        let _ = list_paired_spp_devices();
    }
}
