//! In-process TCP forwarder that pins ROV video traffic to one network
//! interface.
//!
//! ffmpeg's RTSP demuxer cannot bind to an interface, so instead of steering it
//! with OS routes (which needs admin rights), ffmpeg connects to this
//! forwarder on `127.0.0.1` and the forwarder opens the real connection to the
//! ROV from a socket bound to the ROV interface.

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use reqwest::Url;
use socket2::{Domain, Protocol, Socket, Type};

#[cfg(target_os = "windows")]
use crate::network::{ipv4_interface_index, source_ipv4_for_host};
use crate::rov_status::bind_socket_to_interface;

const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
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
    /// Index of that interface, used on Windows to set `IP_UNICAST_IF` so the
    /// connection leaves through that adapter even when other routes
    /// overlap. Ignored elsewhere.
    pub interface_index: Option<u32>,
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
    /// Why the most recent attempt to reach the ROV failed; cleared by the
    /// next success.
    last_connect_error: Mutex<Option<String>>,
}

impl Stats {
    fn set_connect_error(&self, error: Option<String>) {
        *self
            .last_connect_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = error;
    }
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

/// Live sockets of every session, so teardown can shut them down: a thread
/// blocked in a read or write on a socket returns as soon as another thread
/// shuts that socket down. No read or write timeouts are used anywhere, which
/// matters on Windows, where a send that times out leaves the socket in an
/// indeterminate state.
#[derive(Default)]
struct Registry {
    stopped: bool,
    next_id: u64,
    sockets: Vec<(u64, TcpStream)>,
}

#[derive(Default)]
struct Shared {
    registry: Mutex<Registry>,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_stopped(&self) -> bool {
        self.lock().stopped
    }

    /// Registers a session's two sockets for teardown. Returns `None` once the
    /// forwarder has stopped (the session must not start).
    fn register(&self, client: &TcpStream, upstream: &TcpStream) -> Option<u64> {
        let handles = (client.try_clone().ok()?, upstream.try_clone().ok()?);
        let mut registry = self.lock();
        if registry.stopped {
            return None;
        }
        registry.next_id += 1;
        let id = registry.next_id;
        registry.sockets.push((id, handles.0));
        registry.sockets.push((id, handles.1));
        Some(id)
    }

    fn deregister(&self, id: u64) {
        self.lock().sockets.retain(|(sid, _)| *sid != id);
    }

    /// Marks the forwarder stopped and shuts down every live socket, which
    /// unblocks all reads and writes in progress.
    fn shutdown_all(&self) {
        let mut registry = self.lock();
        registry.stopped = true;
        for (_, socket) in registry.sockets.drain(..) {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

/// A running forwarder. Dropping it stops the listener and all connections.
pub struct RtspForwarder {
    local_addr: SocketAddr,
    shared: Arc<Shared>,
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
        let shared = Arc::new(Shared::default());
        let stats = Arc::new(Stats::default());

        let accept_thread = {
            let shared = Arc::clone(&shared);
            let stats = Arc::clone(&stats);
            thread::spawn(move || accept_loop(&listener, &target, &shared, &stats))
        };

        Ok(Self {
            local_addr,
            shared,
            stats,
            accept_thread: Some(accept_thread),
        })
    }

    /// The address ffmpeg should connect to.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Why the most recent attempt to reach the ROV failed (for example
    /// "failed to connect to ROV at 192.168.1.88:8554: Network is unreachable"),
    /// or `None` if the last attempt worked or none was made yet.
    #[must_use]
    pub fn last_connect_error(&self) -> Option<String> {
        self.stats
            .last_connect_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        self.shared.shutdown_all();
        // Wake the blocking `accept` so the thread can see that it stopped.
        let _ = TcpStream::connect_timeout(&self.local_addr, Duration::from_millis(500));
        let _ = self.accept_thread.take().map(JoinHandle::join);
    }
}

fn accept_loop(
    listener: &TcpListener,
    target: &ForwarderTarget,
    shared: &Arc<Shared>,
    stats: &Arc<Stats>,
) {
    for incoming in listener.incoming() {
        if shared.is_stopped() {
            break;
        }
        let Ok(client) = incoming else { continue };
        let target = target.clone();
        let shared = Arc::clone(shared);
        let stats = Arc::clone(stats);
        thread::spawn(move || handle_client(client, &target, &shared, &stats));
    }
}

fn handle_client(
    client: TcpStream,
    target: &ForwarderTarget,
    shared: &Arc<Shared>,
    stats: &Arc<Stats>,
) {
    stats.connections.fetch_add(1, Ordering::Relaxed);
    let _active = ActiveSession::start(stats);
    let upstream = match connect_upstream(target) {
        Ok(stream) => {
            stats.set_connect_error(None);
            stream
        }
        Err(err) => {
            stats.connect_failures.fetch_add(1, Ordering::Relaxed);
            stats.set_connect_error(Some(format!("{err:#}")));
            return;
        }
    };
    let _ = client.set_nodelay(true);
    let _ = upstream.set_nodelay(true);

    // Not registered when the forwarder stopped while connecting.
    let Some(id) = shared.register(&client, &upstream) else {
        return;
    };
    let _ = relay(client, upstream, stats);
    shared.deregister(id);
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

/// Runs both directions until each side has closed or a socket is shut down.
fn relay(client: TcpStream, upstream: TcpStream, stats: &Arc<Stats>) -> std::io::Result<()> {
    let client_rx = client.try_clone()?;
    let upstream_tx = upstream.try_clone()?;
    let to_rov = {
        let stats = Arc::clone(stats);
        thread::spawn(move || pump(client_rx, upstream_tx, &stats.bytes_to_rov, &stats))
    };
    pump(upstream, client, &stats.bytes_from_rov, stats);
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
    #[cfg(windows)]
    if let Some(index) = target.interface_index {
        set_unicast_if(&socket, index, target.addr.is_ipv6())?;
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

/// Forces outgoing packets of `socket` through the adapter with `index`
/// (`IP_UNICAST_IF` / `IPV6_UNICAST_IF`). Binding the adapter's source address
/// alone does not do this: with overlapping routes Windows may still pick
/// another adapter for the connection.
#[cfg(windows)]
fn set_unicast_if(socket: &Socket, index: u32, ipv6: bool) -> Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Networking::WinSock::{
        IP_UNICAST_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, SOCKET, SOCKET_ERROR, setsockopt,
    };

    // IPv4 takes the interface index in network byte order, IPv6 in host order.
    let (level, name, value) = if ipv6 {
        (IPPROTO_IPV6.0, IPV6_UNICAST_IF, index)
    } else {
        (IPPROTO_IP.0, IP_UNICAST_IF, index.to_be())
    };
    let bytes = value.to_ne_bytes();
    // SAFETY: the handle belongs to `socket`, which outlives the call, and
    // `bytes` is a valid buffer of the length passed.
    let result = unsafe {
        setsockopt(
            SOCKET(socket.as_raw_socket() as usize),
            level,
            name,
            Some(&bytes),
        )
    };
    if result == SOCKET_ERROR {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("IP_UNICAST_IF failed for interface index {index}"));
    }
    Ok(())
}

/// Copies `from` into `to` until either side closes or fails.
///
/// An orderly EOF only half-closes `to`, so the peer sees EOF while the
/// opposite direction can still finish. Any error (reset, failed write, or a
/// shutdown by the forwarder's teardown) shuts both sockets down in both
/// directions: the sibling pump shares these sockets and would otherwise keep
/// a dead session alive.
fn pump(mut from: TcpStream, mut to: TcpStream, byte_counter: &AtomicU64, stats: &Stats) {
    let mut buffer = [0_u8; BUFFER_SIZE];
    let mut orderly_eof = false;
    loop {
        let read = match from.read(&mut buffer) {
            Ok(0) => {
                orderly_eof = true;
                break;
            }
            Ok(n) => n,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let started = Instant::now();
        if to.write_all(&buffer[..read]).is_err() {
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
    // The adapter's index is also needed there, for `IP_UNICAST_IF`.
    #[cfg(target_os = "windows")]
    let (source_ip, interface_index) = {
        let std::net::IpAddr::V4(rov_ip) = addr.ip() else {
            anyhow::bail!("ROV host {host} is not an IPv4 address");
        };
        let ifaces = if_addrs::get_if_addrs().context("failed to enumerate network interfaces")?;
        let source = source_ipv4_for_host(&ifaces, interface, rov_ip).with_context(|| {
            format!("interface {interface:?} has no IPv4 address to send ROV traffic from")
        })?;
        let index = ipv4_interface_index(&ifaces, interface)
            .with_context(|| format!("cannot find the interface index of {interface:?}"))?;
        (Some(std::net::IpAddr::V4(source)), Some(index))
    };
    #[cfg(not(target_os = "windows"))]
    let (source_ip, interface_index) = (None, None);

    let forwarder = RtspForwarder::start(ForwarderTarget {
        addr,
        interface: Some(interface.to_owned()),
        interface_index,
        source_ip,
    })?;
    let local_url = rewrite_rtsp_url(url, forwarder.local_addr())
        .context("cannot point the stream URL at the forwarder")?;
    Ok(Some((local_url, forwarder)))
}

/// The `Host` header value a client would send for `url`: the host, plus the
/// port when it is not the scheme's default. Behind the forwarder ffmpeg
/// would send the forwarder's own address instead, which a ROV's HTTP server
/// may not accept, so the real value is passed explicitly.
///
/// # Examples
///
/// ```
/// use third_eye_client::rtsp_forwarder::http_host_header;
///
/// assert_eq!(http_host_header("http://192.168.1.88/v1/medias").as_deref(), Some("192.168.1.88"));
/// assert_eq!(http_host_header("http://192.168.1.88:8080/x").as_deref(), Some("192.168.1.88:8080"));
/// assert_eq!(http_host_header("http://rov.local:80/x").as_deref(), Some("rov.local"));
/// assert!(http_host_header("not a url").is_none());
/// ```
#[must_use]
pub fn http_host_header(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    // `port()` is `None` for the scheme's default port, so only an explicit,
    // non-default one is appended.
    Some(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
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
    fn host_header_keeps_explicit_non_default_ports_only() {
        let header = |url: &str| http_host_header(url);
        assert_eq!(
            header("http://192.168.1.88/v1/x").as_deref(),
            Some("192.168.1.88")
        );
        assert_eq!(
            header("http://192.168.1.88:80/x").as_deref(),
            Some("192.168.1.88")
        );
        assert_eq!(
            header("http://192.168.1.88:8080/x").as_deref(),
            Some("192.168.1.88:8080")
        );
        assert_eq!(
            header("https://rov.local:443/x").as_deref(),
            Some("rov.local")
        );
        assert_eq!(
            header("rtsp://10.0.0.5:8554/s").as_deref(),
            Some("10.0.0.5:8554")
        );
        assert!(header("not a url").is_none());
        assert!(header("mailto:rov@example.com").is_none());
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

    // ---- socket registry used for teardown ----------------------------------

    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    #[test]
    fn registry_refuses_sessions_after_shutdown() {
        let shared = Shared::default();
        assert!(!shared.is_stopped());
        shared.shutdown_all();
        assert!(shared.is_stopped());
        let (a, b) = tcp_pair();
        assert!(shared.register(&a, &b).is_none());
    }

    #[test]
    fn registry_hands_out_distinct_ids_and_forgets_finished_sessions() {
        let shared = Shared::default();
        let (a, b) = tcp_pair();
        let (c, d) = tcp_pair();
        let first = shared.register(&a, &b).unwrap();
        let second = shared.register(&c, &d).unwrap();
        assert_ne!(first, second);
        assert_eq!(shared.lock().sockets.len(), 4);
        shared.deregister(first);
        assert_eq!(shared.lock().sockets.len(), 2);

        // A finished session is no longer touched by the teardown.
        shared.shutdown_all();
        let mut a = a;
        assert!(a.write_all(b"still open").is_ok());
        assert!(shared.lock().sockets.is_empty());
    }

    #[test]
    fn a_session_that_connects_after_shutdown_is_dropped_without_relaying() {
        let rov = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let target = ForwarderTarget {
            addr: rov.local_addr().unwrap(),
            interface: None,
            interface_index: None,
            source_ip: None,
        };
        let shared = Arc::new(Shared::default());
        shared.shutdown_all();
        let stats = Arc::new(Stats::default());
        let (mut client, accepted) = tcp_pair();

        handle_client(accepted, &target, &shared, &stats);

        assert_eq!(stats.connections.load(Ordering::Relaxed), 1);
        assert_eq!(stats.active_connections.load(Ordering::Relaxed), 0);
        assert_eq!(stats.connect_failures.load(Ordering::Relaxed), 0);
        // The client's socket was closed, not relayed.
        let mut buf = [0_u8; 4];
        assert_eq!(client.read(&mut buf).unwrap_or(0), 0);
    }

    #[test]
    fn shutdown_all_unblocks_a_blocked_reader() {
        let shared = Shared::default();
        let (a, b) = tcp_pair();
        shared.register(&a, &b).unwrap();
        let mut blocked = a.try_clone().unwrap();
        let reader = thread::spawn(move || {
            let mut buf = [0_u8; 8];
            blocked.read(&mut buf).unwrap_or(0)
        });
        // Let the reader block first, then shut the session down.
        thread::sleep(Duration::from_millis(100));
        shared.shutdown_all();
        assert_eq!(reader.join().unwrap(), 0);
    }
}
