use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use third_eye_client::rtsp_forwarder::{ForwarderTarget, RtspForwarder};

/// Echo server on loopback, standing in for the ROV.
fn spawn_echo_server() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0_u8; 4096];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 || stream.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

fn target(addr: SocketAddr) -> ForwarderTarget {
    ForwarderTarget {
        addr,
        interface: None,
        interface_index: None,
        source_ip: None,
    }
}

fn connect(forwarder: &RtspForwarder) -> TcpStream {
    let stream = TcpStream::connect(forwarder.local_addr()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

/// Polls until `condition` holds; panics after `timeout`. Counters are updated
/// by the forwarder's threads just after the bytes are delivered, so a test
/// that has read its reply cannot assume they are already visible.
fn wait_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Asserts the peer closed the connection (EOF or a reset). A read timeout or
/// any other error fails: it would mean the connection is still open.
fn assert_closed(stream: &mut TcpStream) {
    let mut buf = [0_u8; 16];
    match stream.read(&mut buf) {
        Ok(0) => {}
        Ok(n) => panic!("expected the connection to close, got {n} bytes"),
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe
            ) => {}
        Err(err) => panic!("connection did not close: {err}"),
    }
}

#[test]
fn forwards_bytes_in_both_directions() {
    let forwarder = RtspForwarder::start(target(spawn_echo_server())).unwrap();
    let mut client = connect(&forwarder);

    client
        .write_all(b"DESCRIBE rtsp://rov/ RTSP/1.0\r\n\r\n")
        .unwrap();
    let mut reply = [0_u8; 33];
    client.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"DESCRIBE rtsp://rov/ RTSP/1.0\r\n\r\n");

    wait_until("byte counters", Duration::from_secs(2), || {
        let stats = forwarder.stats();
        stats.bytes_to_rov == 33 && stats.bytes_from_rov == 33
    });
    let stats = forwarder.stats();
    assert_eq!(stats.connections, 1);
    assert_eq!(stats.connect_failures, 0);
}

#[test]
fn large_payload_survives_intact() {
    let forwarder = RtspForwarder::start(target(spawn_echo_server())).unwrap();
    let mut client = connect(&forwarder);
    let payload: Vec<u8> = (0..1_000_000_u32).map(|i| (i % 251) as u8).collect();

    let mut writer = client.try_clone().unwrap();
    let to_send = payload.clone();
    let sender = thread::spawn(move || writer.write_all(&to_send).unwrap());

    let mut received = vec![0_u8; payload.len()];
    client.read_exact(&mut received).unwrap();
    sender.join().unwrap();
    assert_eq!(received, payload);
}

#[test]
fn the_last_connect_error_is_reported_and_cleared_by_a_later_success() {
    let addr = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap();
    let forwarder = RtspForwarder::start(target(addr)).unwrap();
    assert!(forwarder.last_connect_error().is_none());

    // Nothing listens at the ROV address yet: the failure is explained.
    let mut client = connect(&forwarder);
    assert_closed(&mut client);
    wait_until("connect error", Duration::from_secs(2), || {
        forwarder.last_connect_error().is_some()
    });
    let error = forwarder.last_connect_error().unwrap();
    assert!(error.contains(&addr.to_string()), "error was: {error}");

    // The ROV comes up on that address: the next connection clears the error.
    let rov = TcpListener::bind(addr).unwrap();
    thread::spawn(move || {
        let _keep_open = rov.accept();
        thread::sleep(Duration::from_secs(2));
    });
    let _second = connect(&forwarder);
    wait_until("error cleared", Duration::from_secs(2), || {
        forwarder.last_connect_error().is_none()
    });
}

#[test]
fn closing_the_rov_side_closes_the_client() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"bye").unwrap();
    });

    let forwarder = RtspForwarder::start(target(addr)).unwrap();
    let mut client = connect(&forwarder);
    let mut all = Vec::new();
    client.read_to_end(&mut all).unwrap();
    assert_eq!(all, b"bye");
}

#[test]
fn unreachable_rov_counts_a_failure_and_closes_the_client() {
    // Grab a free port, then release it so nothing is listening there.
    let addr = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap();
    let forwarder = RtspForwarder::start(target(addr)).unwrap();
    let mut client = connect(&forwarder);

    assert_closed(&mut client);
    wait_until("connect failure count", Duration::from_secs(2), || {
        forwarder.stats().connect_failures == 1
    });
}

#[test]
fn dropping_the_forwarder_frees_the_port_and_ends_connections() {
    let forwarder = RtspForwarder::start(target(spawn_echo_server())).unwrap();
    let local = forwarder.local_addr();
    let mut client = connect(&forwarder);
    client.write_all(b"x").unwrap();
    let mut byte = [0_u8; 1];
    client.read_exact(&mut byte).unwrap();

    drop(forwarder);

    assert_closed(&mut client);
    assert!(
        TcpStream::connect_timeout(&local, Duration::from_millis(500)).is_err(),
        "listener should be gone"
    );
}

/// Round-trips a few bytes through a forwarder built from `forwarder_target`.
fn echoes_through(forwarder_target: ForwarderTarget) {
    let forwarder = RtspForwarder::start(forwarder_target).unwrap();
    let mut client = connect(&forwarder);
    client.write_all(b"ping").unwrap();
    let mut reply = [0_u8; 4];
    client.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"ping");
}

/// Asserts a forwarder whose upstream setup fails counts it and drops the client.
fn fails_to_reach_rov(forwarder_target: ForwarderTarget) {
    let forwarder = RtspForwarder::start(forwarder_target).unwrap();
    let mut client = connect(&forwarder);
    assert_closed(&mut client);
    wait_until("connect failure count", Duration::from_secs(2), || {
        forwarder.stats().connect_failures == 1
    });
}

#[test]
fn binding_to_a_local_source_address_still_connects() {
    echoes_through(ForwarderTarget {
        source_ip: Some(Ipv4Addr::LOCALHOST.into()),
        ..target(spawn_echo_server())
    });
}

#[test]
fn binding_to_an_address_this_machine_does_not_own_fails_cleanly() {
    // 203.0.113.0/24 is reserved for documentation and never assigned locally.
    fails_to_reach_rov(ForwarderTarget {
        source_ip: Some(Ipv4Addr::new(203, 0, 113, 9).into()),
        ..target(spawn_echo_server())
    });
}

/// Runs the IPv6 upstream setup whether or not the host supports IPv6: nothing
/// listens on `[::1]:1`, so the session fails either way and is counted.
#[test]
fn an_unreachable_ipv6_rov_counts_a_failure() {
    fails_to_reach_rov(target("[::1]:1".parse().unwrap()));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn pinning_to_a_missing_interface_fails_cleanly() {
    fails_to_reach_rov(ForwarderTarget {
        interface: Some("nonexistent9".to_owned()),
        ..target(spawn_echo_server())
    });
}

#[cfg(target_os = "macos")]
#[test]
fn pinning_to_the_loopback_interface_connects() {
    echoes_through(ForwarderTarget {
        interface: Some("lo0".to_owned()),
        ..target(spawn_echo_server())
    });
}

/// Closes `stream` with a TCP reset instead of an orderly shutdown.
fn close_with_reset(stream: TcpStream) {
    socket2::SockRef::from(&stream)
        .set_linger(Some(Duration::ZERO))
        .unwrap();
    drop(stream);
}

#[test]
fn forwards_to_an_ipv6_rov() {
    // Skip on hosts without IPv6 loopback.
    let Ok(listener) = TcpListener::bind("[::1]:0") else {
        return;
    };
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0_u8; 4];
        stream.read_exact(&mut buf).unwrap();
        stream.write_all(&buf).unwrap();
    });
    echoes_through(target(addr));
}

#[test]
fn a_rov_that_resets_the_connection_ends_the_session() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        close_with_reset(stream);
    });
    let forwarder = RtspForwarder::start(target(addr)).unwrap();
    let mut client = connect(&forwarder);
    assert_closed(&mut client);

    // The client-to-ROV direction must also give up once the ROV is gone,
    // instead of waiting forever or crashing.
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.write_all(b"after the reset").is_ok() {
        assert!(Instant::now() < deadline, "client side never closed");
        thread::sleep(Duration::from_millis(100));
    }
}

/// After the ROV resets, an idle client must not keep the session (and its
/// threads and sockets) alive: both directions are torn down.
#[test]
fn a_rov_reset_releases_the_session_while_the_client_stays_idle() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        close_with_reset(stream);
    });
    let forwarder = RtspForwarder::start(target(addr)).unwrap();
    let mut client = connect(&forwarder);
    assert_closed(&mut client);

    // The client sends nothing and stays connected.
    wait_until("session release", Duration::from_secs(2), || {
        forwarder.stats().active_connections == 0
    });
    assert_eq!(forwarder.stats().connections, 1);
}

#[test]
fn a_client_that_resets_the_connection_ends_the_session() {
    let (seen_eof, eof) = mpsc::channel::<bool>();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut sink = Vec::new();
        seen_eof
            .send(stream.read_to_end(&mut sink).is_ok())
            .unwrap();
    });
    let forwarder = RtspForwarder::start(target(addr)).unwrap();
    let client = connect(&forwarder);
    wait_until("upstream connection", Duration::from_secs(2), || {
        forwarder.stats().connections == 1
    });
    close_with_reset(client);

    assert!(
        eof.recv_timeout(Duration::from_secs(5)).unwrap(),
        "ROV side should see the session end"
    );
}

/// A peer that stops reading must not keep forwarder threads and sockets alive
/// after shutdown.
#[test]
fn dropping_the_forwarder_closes_sockets_even_when_the_rov_stops_reading() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let rov_addr = listener.local_addr().unwrap();
    let (start_reading, may_read) = mpsc::channel::<()>();
    let rov = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Do not read until told to, so the forwarder's writes back up.
        may_read.recv().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut sink = Vec::new();
        stream.read_to_end(&mut sink)
    });

    let forwarder = RtspForwarder::start(target(rov_addr)).unwrap();
    let mut client = connect(&forwarder);
    thread::spawn(move || {
        let chunk = vec![0_u8; 64 * 1024];
        while client.write_all(&chunk).is_ok() {}
    });
    // Wait until the socket buffers are full and the forwarder is stuck
    // mid-write: no more bytes reach the ROV side for a while.
    let mut last = 0;
    let mut stalled_since = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(20);
    while stalled_since.elapsed() < Duration::from_millis(700) {
        assert!(Instant::now() < deadline, "forwarder never stalled");
        thread::sleep(Duration::from_millis(50));
        let now = forwarder.stats().bytes_to_rov;
        if now != last {
            last = now;
            stalled_since = Instant::now();
        }
    }

    drop(forwarder);
    // Give the stuck write time to time out and see the stop request while the
    // ROV is still not reading, so it cannot finish the write by luck.
    thread::sleep(Duration::from_millis(600));
    start_reading.send(()).unwrap();

    let result = rov.join().unwrap();
    assert!(
        result.is_ok(),
        "ROV side should see the connection closed, got {result:?}"
    );
}

/// Overhead measurement for the TE-2 go/no-go. Run with
/// `cargo test --test rtsp_forwarder -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark: prints numbers instead of asserting a platform-dependent bound"]
fn bench_added_latency() {
    const ROUNDS: usize = 5_000;
    let echo = spawn_echo_server();
    let forwarder = RtspForwarder::start(target(echo)).unwrap();

    let round_trips = |addr: SocketAddr| {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_nodelay(true).unwrap();
        let mut samples = Vec::with_capacity(ROUNDS);
        let msg = [0xAB_u8; 1200];
        let mut back = [0_u8; 1200];
        for _ in 0..ROUNDS {
            let t = Instant::now();
            stream.write_all(&msg).unwrap();
            stream.read_exact(&mut back).unwrap();
            samples.push(t.elapsed());
        }
        samples.sort();
        samples
    };

    let direct = round_trips(echo);
    let via = round_trips(forwarder.local_addr());
    let pct = |s: &[Duration], percent: usize| s[(s.len() * percent / 100).min(s.len() - 1)];
    println!(
        "direct   p50 {:?}  p99 {:?}\nforwarded p50 {:?}  p99 {:?}",
        pct(&direct, 50),
        pct(&direct, 99),
        pct(&via, 50),
        pct(&via, 99)
    );
    let stats = forwarder.stats();
    println!(
        "forwarder write overhead p50 <= {}us  p99 <= {}us",
        stats.write_p50_micros, stats.write_p99_micros
    );
}
