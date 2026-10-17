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
    // Let the socket buffers fill so the forwarder is blocked mid-write.
    thread::sleep(Duration::from_millis(500));

    drop(forwarder);
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
