//! In-process TCP forwarder that pins ROV video traffic to one network
//! interface.
//!
//! ffmpeg's RTSP demuxer cannot bind to an interface, so instead of steering it
//! with OS routes (which needs admin rights), ffmpeg connects to this
//! forwarder on `127.0.0.1` and the forwarder opens the real connection to the
//! ROV from a socket bound to the ROV interface.

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reqwest::Url;
use socket2::{Domain, Protocol, Socket, Type};

use crate::rov_status::bind_socket_to_interface;

const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How often a blocked pump wakes up to notice a stop request.
const READ_POLL: Duration = Duration::from_millis(200);
const BUFFER_SIZE: usize = 16 * 1024;
const HISTOGRAM_BUCKETS: usize = 32;

/// Where and how the forwarder reaches the ROV.
#[derive(Debug, Clone)]
pub struct ForwarderTarget {
    /// The ROV's RTSP endpoint.
    pub addr: SocketAddr,
    /// Interface to pin the outgoing connection to (`IP_BOUND_IF` /
    /// `SO_BINDTODEVICE`). `None` leaves routing to the OS.
    pub interface: Option<String>,
    /// Source address to bind before connecting. This selects the source
    /// interface on Windows, where binding by name is not available.
    pub source_ip: Option<IpAddr>,
}

/// Log2 histogram of durations in microseconds, safe to update from many threads.
#[derive(Default)]
struct Histogram {
    buckets: [AtomicU64; HISTOGRAM_BUCKETS],
}

impl Histogram {
    fn record(&self, elapsed: Duration) {
        // Round up so a nonzero sub-microsecond write is not reported as 0 us.
        let micros = u64::try_from(elapsed.as_nanos().div_ceil(1_000)).unwrap_or(u64::MAX);
        let bucket = micros.bit_width().min(HISTOGRAM_BUCKETS as u32 - 1);
        self.buckets[bucket as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// Upper bound, in microseconds, of the bucket holding the given quantile.
    fn percentile_micros(&self, quantile: f64) -> u64 {
        let counts: Vec<u64> = self
            .buckets
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect();
        let total: u64 = counts.iter().sum();
        if total == 0 {
            return 0;
        }
        let rank = (total as f64 * quantile).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (i, count) in counts.iter().enumerate() {
            seen += count;
            if seen >= rank {
                return if i == 0 { 0 } else { 1_u64 << i };
            }
        }
        u64::MAX
    }
}

#[derive(Default)]
struct Stats {
    connections: AtomicU64,
    connect_failures: AtomicU64,
    bytes_to_rov: AtomicU64,
    bytes_from_rov: AtomicU64,
    write_delay: Histogram,
}

/// Point-in-time copy of the forwarder's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwarderStats {
    pub connections: u64,
    pub connect_failures: u64,
    pub bytes_to_rov: u64,
    pub bytes_from_rov: u64,
    /// Median time spent handing a chunk to the destination socket, rounded
    /// up to a power of two. This is the forwarder's own overhead, not the
    /// network round trip.
    pub write_p50_micros: u64,
    /// 99th percentile of the same measurement.
    pub write_p99_micros: u64,
}

/// A running forwarder. Dropping it stops the listener and all connections.
pub struct RtspForwarder {
    local_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    stats: Arc<Stats>,
    accept_thread: Option<JoinHandle<()>>,
}

impl RtspForwarder {
    /// Starts listening on a free `127.0.0.1` port.
    ///
    /// # Errors
    ///
    /// Returns an error if the local listener cannot be created.
    pub fn start(target: ForwarderTarget) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .context("failed to bind RTSP forwarder listener on 127.0.0.1")?;
        let local_addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Stats::default());

        let accept_thread = {
            let stop = Arc::clone(&stop);
            let stats = Arc::clone(&stats);
            thread::spawn(move || accept_loop(&listener, &target, &stop, &stats))
        };

        Ok(Self {
            local_addr,
            stop,
            stats,
            accept_thread: Some(accept_thread),
        })
    }

    /// The address ffmpeg should connect to.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    #[must_use]
    pub fn stats(&self) -> ForwarderStats {
        ForwarderStats {
            connections: self.stats.connections.load(Ordering::Relaxed),
            connect_failures: self.stats.connect_failures.load(Ordering::Relaxed),
            bytes_to_rov: self.stats.bytes_to_rov.load(Ordering::Relaxed),
            bytes_from_rov: self.stats.bytes_from_rov.load(Ordering::Relaxed),
            write_p50_micros: self.stats.write_delay.percentile_micros(0.5),
            write_p99_micros: self.stats.write_delay.percentile_micros(0.99),
        }
    }
}

impl Drop for RtspForwarder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the blocking `accept` so the thread can see the stop flag.
        let _ = TcpStream::connect_timeout(&self.local_addr, Duration::from_millis(500));
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
    }
}

fn accept_loop(
    listener: &TcpListener,
    target: &ForwarderTarget,
    stop: &Arc<AtomicBool>,
    stats: &Arc<Stats>,
) {
    for incoming in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(client) = incoming else { continue };
        let target = target.clone();
        let stop = Arc::clone(stop);
        let stats = Arc::clone(stats);
        thread::spawn(move || handle_client(client, &target, &stop, &stats));
    }
}

fn handle_client(
    client: TcpStream,
    target: &ForwarderTarget,
    stop: &Arc<AtomicBool>,
    stats: &Arc<Stats>,
) {
    stats.connections.fetch_add(1, Ordering::Relaxed);
    let Ok(upstream) = connect_upstream(target) else {
        stats.connect_failures.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let _ = client.set_nodelay(true);
    let _ = upstream.set_nodelay(true);

    let (Ok(client_rx), Ok(upstream_tx)) = (client.try_clone(), upstream.try_clone()) else {
        return;
    };
    let to_rov = {
        let stop = Arc::clone(stop);
        let stats = Arc::clone(stats);
        thread::spawn(move || pump(client_rx, upstream_tx, &stats.bytes_to_rov, &stats, &stop))
    };
    pump(upstream, client, &stats.bytes_from_rov, stats, stop);
    let _ = to_rov.join();
}

fn connect_upstream(target: &ForwarderTarget) -> Result<TcpStream> {
    let domain = match target.addr {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))
        .context("failed to create upstream socket")?;
    if let Some(interface) = &target.interface {
        bind_socket_to_interface(&socket, interface)?;
    }
    if let Some(ip) = target.source_ip {
        socket
            .bind(&SocketAddr::new(ip, 0).into())
            .with_context(|| format!("failed to bind upstream socket to {ip}"))?;
    }
    socket
        .connect_timeout(&target.addr.into(), UPSTREAM_CONNECT_TIMEOUT)
        .with_context(|| format!("failed to connect to ROV at {}", target.addr))?;
    Ok(socket.into())
}

/// Copies `from` into `to` until either side closes or a stop is requested,
/// then half-closes `to` so the peer sees EOF.
fn pump(
    mut from: TcpStream,
    mut to: TcpStream,
    byte_counter: &AtomicU64,
    stats: &Stats,
    stop: &AtomicBool,
) {
    let _ = from.set_read_timeout(Some(READ_POLL));
    // A peer that stops reading must not pin this thread: writes time out too,
    // so the stop flag is re-checked every `READ_POLL`.
    let _ = to.set_write_timeout(Some(READ_POLL));
    let mut buffer = [0_u8; BUFFER_SIZE];
    while !stop.load(Ordering::Relaxed) {
        let read = match from.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                continue;
            }
            Err(_) => break,
        };
        let started = Instant::now();
        if !write_all_until_stopped(&mut to, &buffer[..read], stop) {
            break;
        }
        stats.write_delay.record(started.elapsed());
        byte_counter.fetch_add(read as u64, Ordering::Relaxed);
    }
    let _ = to.shutdown(Shutdown::Write);
    let _ = from.shutdown(Shutdown::Read);
}

/// `write_all` that gives up when `stop` is set. A write timeout only means the
/// peer's buffer is full, so the bytes already written are kept and the rest
/// is retried. Returns `false` if the connection failed or a stop was requested.
fn write_all_until_stopped(to: &mut TcpStream, mut data: &[u8], stop: &AtomicBool) -> bool {
    while !data.is_empty() {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        match to.write(data) {
            Ok(0) => return false,
            Ok(n) => data = &data[n..],
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => return false,
        }
    }
    true
}

/// Points a URL (RTSP or HTTP) at the forwarder, keeping credentials, path
/// and query.
///
/// Returns `None` if `rtsp_url` is not a valid URL. Note that for HTTP the
/// `Host` header then names the forwarder rather than the ROV.
///
/// # Examples
///
/// ```
/// use std::net::SocketAddr;
/// use third_eye_client::rtsp_forwarder::rewrite_rtsp_url;
///
/// let local: SocketAddr = "127.0.0.1:40123".parse().unwrap();
/// assert_eq!(
///     rewrite_rtsp_url("rtsp://admin:admin@192.168.1.88:8554/stream/0/0", local).as_deref(),
///     Some("rtsp://admin:admin@127.0.0.1:40123/stream/0/0"),
/// );
/// assert!(rewrite_rtsp_url("not a url", local).is_none());
/// ```
#[must_use]
pub fn rewrite_rtsp_url(rtsp_url: &str, local: SocketAddr) -> Option<String> {
    let mut url = Url::parse(rtsp_url).ok()?;
    url.set_ip_host(local.ip()).ok()?;
    url.set_port(Some(local.port())).ok()?;
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_histogram_reports_zero() {
        let h = Histogram::default();
        assert_eq!(h.percentile_micros(0.5), 0);
        assert_eq!(h.percentile_micros(0.99), 0);
    }

    #[test]
    fn sub_microsecond_samples_round_up_to_a_nonzero_bound() {
        let h = Histogram::default();
        h.record(Duration::from_nanos(300));
        assert!(h.percentile_micros(0.5) >= 1);
        let zero = Histogram::default();
        zero.record(Duration::ZERO);
        assert_eq!(zero.percentile_micros(0.5), 0);
    }

    #[test]
    fn percentiles_split_fast_and_slow_samples() {
        let h = Histogram::default();
        for _ in 0..98 {
            h.record(Duration::from_micros(3));
        }
        h.record(Duration::from_micros(900));
        h.record(Duration::from_micros(900));
        let p50 = h.percentile_micros(0.5);
        let p99 = h.percentile_micros(0.99);
        assert!((3..=4).contains(&p50), "p50 was {p50}");
        assert!((900..=1024).contains(&p99), "p99 was {p99}");
    }

    #[test]
    fn huge_durations_land_in_the_last_bucket() {
        let h = Histogram::default();
        h.record(Duration::from_secs(10_000_000));
        assert!(h.percentile_micros(0.5) > 0);
    }

    #[test]
    fn rewrite_keeps_credentials_path_and_query() {
        let local: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        assert_eq!(
            rewrite_rtsp_url("http://10.0.0.5/v1/medias/a.mp4/download?x=1", local).as_deref(),
            Some("http://127.0.0.1:4000/v1/medias/a.mp4/download?x=1")
        );
    }

    #[test]
    fn rewrite_rejects_urls_that_cannot_carry_a_host() {
        let local: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        assert!(rewrite_rtsp_url("mailto:rov@example.com", local).is_none());
        assert!(rewrite_rtsp_url("", local).is_none());
    }
}
