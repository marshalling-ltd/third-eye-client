//! Tests for the `network` module used by background ROV recalibration.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use third_eye_client::network::{
    RecalibrateResult, detect_rov_interface, parse_host_from_http_base,
};

// ---- parse_host_from_http_base -------------------------------------------

#[test]
fn parse_host_extracts_ip_from_full_url() {
    assert_eq!(
        parse_host_from_http_base("http://192.168.1.88"),
        Some("192.168.1.88".to_string())
    );
}

#[test]
fn parse_host_strips_port_and_path() {
    assert_eq!(
        parse_host_from_http_base("http://192.168.1.88:8080/v1/api"),
        Some("192.168.1.88".to_string())
    );
}

#[test]
fn parse_host_adds_scheme_when_missing() {
    assert_eq!(
        parse_host_from_http_base("192.168.1.88"),
        Some("192.168.1.88".to_string())
    );
}

#[test]
fn parse_host_trims_whitespace() {
    assert_eq!(
        parse_host_from_http_base("  http://10.0.0.1  "),
        Some("10.0.0.1".to_string())
    );
}

#[test]
fn parse_host_returns_none_for_empty_input() {
    assert_eq!(parse_host_from_http_base(""), None);
}

#[test]
fn parse_host_handles_hostname() {
    assert_eq!(
        parse_host_from_http_base("http://rov.local"),
        Some("rov.local".to_string())
    );
}

// ---- detect_rov_interface ------------------------------------------------

#[test]
fn detect_interface_returns_none_for_unreachable_subnet() {
    // 1.2.3.4 is guaranteed to not be on any local interface subnet.
    assert!(detect_rov_interface("1.2.3.4").is_none());
}

#[test]
fn detect_interface_returns_none_for_invalid_ip() {
    assert!(detect_rov_interface("not-an-ip").is_none());
}

// ---- RecalibrateResult channel -------------------------------------------

#[test]
fn recalibrate_result_round_trips_through_channel() {
    let (tx, rx) = mpsc::channel::<RecalibrateResult>();

    tx.send(RecalibrateResult {
        interface: "en10".to_string(),
        rov_info: "Detected wired ROV interface en10.".to_string(),
    })
    .unwrap();

    let received = rx.try_recv().unwrap();
    assert_eq!(received.interface, "en10");
    assert!(received.rov_info.contains("en10"));
}

#[test]
fn recalibrate_channel_is_empty_before_send() {
    let (_tx, rx) = mpsc::channel::<RecalibrateResult>();
    assert!(rx.try_recv().is_err());
}

#[test]
fn recalibrate_result_delivered_from_background_thread() {
    let (tx, rx) = mpsc::channel::<RecalibrateResult>();

    thread::spawn(move || {
        // Simulate the blocking recalibration work.
        let host = parse_host_from_http_base("http://1.2.3.4");
        let interface = host
            .as_deref()
            .and_then(detect_rov_interface)
            .unwrap_or_default();
        let rov_info = if interface.is_empty() {
            format!(
                "No dedicated wired ROV interface detected for {}. Using OS routing.",
                host.as_deref().unwrap_or("?")
            )
        } else {
            format!("Detected wired ROV interface {interface}.")
        };
        let _ = tx.send(RecalibrateResult {
            interface,
            rov_info,
        });
    });

    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("background thread should deliver result within 5 s");

    assert!(result.interface.is_empty());
    assert!(result.rov_info.contains("1.2.3.4"));
    assert!(result.rov_info.contains("No dedicated wired ROV interface"));
}
