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

#[cfg(target_os = "windows")]
use crate::network::source_ipv4_for_host;
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
        counts
            .iter()
            .enumerate()
            .find_map(|(i, count)| {
                seen += count;
                (seen >= rank).then_some(if i == 0 { 0 } else { 1_u64 << i })
            })
            .unwrap_or(u64::MAX)
    }
}

#[derive(Default)]
struct Stats {
    connections: AtomicU64,
    active_connections: AtomicU64,
    connect_failures: AtomicU64,
    bytes_to_rov: AtomicU64,
    bytes_from_rov: AtomicU64,
    write_delay: Histogram,
}

/// Point-in-time copy of the forwarder's counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwarderStats {
    pub connections: u64,
    /// Sessions currently relaying (or connecting to the ROV).
    pub active_connections: u64,
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
            active_connections: self.stats.active_connections.load(Ordering::Relaxed),
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
        let _ = self.accept_thread.take().map(JoinHandle::join);
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
    let _active = ActiveSession::start(stats);
    let Ok(upstream) = connect_upstream(target) else {
        stats.connect_failures.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let _ = client.set_nodelay(true);
    let _ = upstream.set_nodelay(true);

    let _ = relay(client, upstream, stop, stats);
}

/// Counts a session as active for as long as it lives.
struct ActiveSession<'a>(&'a Stats);

impl<'a> ActiveSession<'a> {
    fn start(stats: &'a Stats) -> Self {
        stats.active_connections.fetch_add(1, Ordering::Relaxed);
        Self(stats)
    }
}

impl Drop for ActiveSession<'_> {
    fn drop(&mut self) {
        self.0.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Runs both directions until each side has closed or `stop` is set.
fn relay(
    client: TcpStream,
    upstream: TcpStream,
    stop: &Arc<AtomicBool>,
    stats: &Arc<Stats>,
) -> std::io::Result<()> {
    let client_rx = client.try_clone()?;
    let upstream_tx = upstream.try_clone()?;
    let to_rov = {
        let stop = Arc::clone(stop);
        let stats = Arc::clone(stats);
        thread::spawn(move || pump(client_rx, upstream_tx, &stats.bytes_to_rov, &stats, &stop))
    };
    pump(upstream, client, &stats.bytes_from_rov, stats, stop);
    let _ = to_rov.join();
    Ok(())
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

/// Copies `from` into `to` until either side closes or a stop is requested.
///
/// An orderly EOF only half-closes `to`, so the peer sees EOF while the
/// opposite direction can still finish. Any error (reset, failed write) or a
/// stop shuts both sockets down in both directions: the sibling pump shares
/// these sockets and would otherwise keep a dead session alive until the
/// forwarder is dropped.
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
    let mut orderly_eof = false;
    while !stop.load(Ordering::Relaxed) {
        let read = match from.read(&mut buffer) {
            Ok(0) => {
                orderly_eof = true;
                break;
            }
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
    if orderly_eof {
        let _ = to.shutdown(Shutdown::Write);
    } else {
        let _ = to.shutdown(Shutdown::Both);
        let _ = from.shutdown(Shutdown::Both);
    }
}

/// `write_all` that gives up when `stop` is set. A write timeout only means the
/// peer's buffer is full, so the bytes already written are kept and the rest
/// is retried. Returns `false` if the connection failed or a stop was requested.
fn write_all_until_stopped(to: &mut TcpStream, mut data: &[u8], stop: &AtomicBool) -> bool {
    while !data.is_empty() && !stop.load(Ordering::Relaxed) {
        match to.write(data) {
            Ok(n) if n > 0 => data = &data[n..],
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            // A closed or failed connection (a zero-byte write counts as one).
            _ => return false,
        }
    }
    data.is_empty()
}

/// Port the forwarder should dial: the one in the URL, else the scheme's
/// well-known port (80 for `http`), else `fallback` (e.g. 554 for `rtsp`).
pub fn forward_target_port(url: &Url, fallback: u16) -> u16 {
    url.port_or_known_default().unwrap_or(fallback)
}

/// Starts a forwarder for `url` (RTSP or HTTP) pinned to `rov_interface` and
/// returns the URL ffmpeg should use instead. `default_port` applies when the
/// URL names none. Returns `Ok(None)` when forwarding does not apply: no
/// interface is selected, or the URL points at this machine (the bundled test
/// server).
///
/// # Errors
///
/// Fails rather than falling back to unpinned routing, e.g. on Windows when
/// the selected adapter has no usable IPv4 source address.
pub fn start_url_forwarder(
    url: &str,
    rov_interface: Option<&str>,
    default_port: u16,
) -> Result<Option<(String, RtspForwarder)>> {
    use std::net::ToSocketAddrs;

    let Some(interface) = rov_interface else {
        return Ok(None);
    };
    let parsed = Url::parse(url).context("invalid stream URL")?;
    // The forwarder carries raw TCP, so TLS would be negotiated with the
    // loopback address and fail certificate validation.
    anyhow::ensure!(
        parsed.scheme() != "https",
        "HTTPS streams cannot go through the interface-bound forwarder"
    );
    let host = parsed.host_str().context("stream URL has no host")?;
    let port = forward_target_port(&parsed, default_port);
    let addr = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve ROV host {host}"))?
        .find(std::net::SocketAddr::is_ipv4)
        .with_context(|| format!("ROV host {host} has no IPv4 address"))?;
    if addr.ip().is_loopback() {
        return Ok(None);
    }

    // Windows cannot bind by interface name here, so pin the source address:
    // the one on the ROV's subnet, never an unrelated alias on the adapter.
    #[cfg(target_os = "windows")]
    let source_ip = {
        let std::net::IpAddr::V4(rov_ip) = addr.ip() else {
            anyhow::bail!("ROV host {host} is not an IPv4 address");
        };
        let ifaces = if_addrs::get_if_addrs().context("failed to enumerate network interfaces")?;
        let source = source_ipv4_for_host(&ifaces, interface, rov_ip).with_context(|| {
            format!("interface {interface:?} has no IPv4 address to send ROV traffic from")
        })?;
        Some(std::net::IpAddr::V4(source))
    };
    #[cfg(not(target_os = "windows"))]
    let source_ip = None;

    let forwarder = RtspForwarder::start(ForwarderTarget {
        addr,
        interface: Some(interface.to_owned()),
        source_ip,
    })?;
    let local_url = rewrite_rtsp_url(url, forwarder.local_addr())
        .context("cannot point the stream URL at the forwarder")?;
    Ok(Some((local_url, forwarder)))
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

    // ---- start_url_forwarder -----------------------------------------------

    #[test]
    fn forward_target_port_prefers_url_then_scheme_then_fallback() {
        let port = |u: &str| forward_target_port(&Url::parse(u).unwrap(), 554);
        assert_eq!(port("rtsp://10.0.0.1:8554/s"), 8554);
        assert_eq!(port("rtsp://10.0.0.1/s"), 554);
        assert_eq!(port("http://10.0.0.1/x"), 80);
        assert_eq!(port("http://10.0.0.1:8080/x"), 8080);
        assert_eq!(port("https://10.0.0.1/x"), 443);
    }

    #[test]
    fn forwarder_is_skipped_without_interface_or_for_loopback_hosts() {
        assert!(
            start_url_forwarder("rtsp://192.0.2.1:8554/s", None, 554)
                .unwrap()
                .is_none()
        );
        assert!(
            start_url_forwarder("rtsp://127.0.0.1:8554/s", Some("en0"), 554)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn forwarder_rejects_bad_or_https_urls() {
        assert!(start_url_forwarder("not a url", Some("en0"), 554).is_err());
        let result = start_url_forwarder("https://192.0.2.1/x", Some("en0"), 80);
        assert!(matches!(result, Err(ref err) if format!("{err:#}").contains("HTTPS")));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn forwarder_rewrites_url_to_loopback_keeping_credentials_and_path() {
        let (local_url, guard) = start_url_forwarder(
            "rtsp://admin:pw@192.0.2.1:8554/stream/0/0",
            Some("en0"),
            554,
        )
        .unwrap()
        .unwrap();
        let expected = format!(
            "rtsp://admin:pw@127.0.0.1:{}/stream/0/0",
            guard.local_addr().port()
        );
        assert_eq!(local_url, expected);
    }

    // ---- write_all_until_stopped -------------------------------------------

    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    /// Keeps writing 1 MiB chunks until `write_all_until_stopped` gives up.
    /// How much the OS buffers before a write blocks or fails differs a lot
    /// (Windows accepts very large writes at once), so the tests must not
    /// assume a size; they only require that it gives up in bounded time.
    fn write_until_it_gives_up(writer: &mut TcpStream, stop: &AtomicBool) {
        let chunk = vec![0_u8; 1024 * 1024];
        let deadline = Instant::now() + Duration::from_secs(20);
        while write_all_until_stopped(writer, &chunk, stop) && Instant::now() < deadline {}
        assert!(Instant::now() < deadline, "never gave up");
    }

    #[test]
    fn write_all_until_stopped_delivers_everything_to_a_reading_peer() {
        let (mut writer, mut reader) = tcp_pair();
        let data = vec![7_u8; 1_000_000];
        let sink = thread::spawn(move || {
            let mut received = Vec::new();
            reader.read_to_end(&mut received).unwrap();
            received.len()
        });
        assert!(write_all_until_stopped(
            &mut writer,
            &data,
            &AtomicBool::new(false)
        ));
        drop(writer);
        assert_eq!(sink.join().unwrap(), data.len());
    }

    #[test]
    fn write_all_until_stopped_retries_through_timeouts_until_stopped() {
        // The reader never reads, so the socket buffers fill and writes time out.
        let (mut writer, _reader) = tcp_pair();
        writer
            .set_write_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopper = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(300));
                stop.store(true, Ordering::Relaxed);
            })
        };
        write_until_it_gives_up(&mut writer, &stop);
        assert!(
            stop.load(Ordering::Relaxed),
            "gave up before the stop request"
        );
        stopper.join().unwrap();
    }

    #[test]
    fn write_all_until_stopped_gives_up_when_the_peer_resets() {
        let (mut writer, reader) = tcp_pair();
        socket2::SockRef::from(&reader)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        drop(reader);
        write_until_it_gives_up(&mut writer, &AtomicBool::new(false));
    }
}
