//! Prints how this machine would reach the ROV, for checking detection on a
//! real network without starting the GUI:
//!
//! ```sh
//! ROV_HOST=192.168.1.88 cargo test --test rov_link_diagnostic -- --nocapture
//! ```
//!
//! It only reports; it passes on any machine.

use std::net::Ipv4Addr;

use third_eye_client::network::{
    LinkKind, describe_rov_link, network_signature, select_rov_link, snapshot_interfaces,
};

const DEFAULT_ROV_HOST: &str = "192.168.1.88";

fn kind_name(kind: LinkKind) -> &'static str {
    match kind {
        LinkKind::Wired => "wired",
        LinkKind::Wireless => "wireless",
        LinkKind::Unknown => "unknown",
    }
}

#[test]
fn report_how_this_machine_reaches_the_rov() {
    let host = std::env::var("ROV_HOST").unwrap_or_else(|_| DEFAULT_ROV_HOST.to_owned());
    let rov_ip: Ipv4Addr = host
        .parse()
        .unwrap_or_else(|_| DEFAULT_ROV_HOST.parse().expect("default is valid"));

    let interfaces = snapshot_interfaces();
    println!("\nInterfaces (name, guessed kind, IPv4 address/netmask):");
    for iface in &interfaces {
        let addrs: Vec<String> = iface
            .ipv4
            .iter()
            .map(|(ip, mask)| format!("{ip}/{mask}"))
            .collect();
        println!(
            "  {:<24} {:<9} {}",
            iface.name,
            kind_name(iface.kind),
            if addrs.is_empty() {
                "(no IPv4 yet)".to_owned()
            } else {
                addrs.join(", ")
            }
        );
    }

    let link = select_rov_link(&interfaces, rov_ip);
    println!("\nROV {rov_ip}: {link:?}");
    println!(
        "{}",
        describe_rov_link(link.as_ref(), &rov_ip.to_string(), &interfaces)
    );
    println!(
        "\nChange-detection signature has {} address(es).",
        network_signature().len()
    );

    assert!(
        interfaces.iter().all(|i| !i.name.is_empty()),
        "interface names must not be empty"
    );
}
