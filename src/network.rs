//! Network detection and recalibration helpers.
//!
//! These functions are pure or `Send`-safe and live in the library crate so
//! they can be tested from `tests/`.

use reqwest::Url;

/// Result of a background ROV network recalibration.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::RecalibrateResult;
///
/// let result = RecalibrateResult {
///     interface: "en10".to_string(),
///     rov_info: "Detected ROV interface en10.".to_string(),
/// };
///
/// assert_eq!(result.interface, "en10");
/// assert!(result.rov_info.contains("en10"));
/// ```
pub struct RecalibrateResult {
    /// Detected interface name, or empty if none found.
    pub interface: String,
    /// Human-readable status summary for `rov_info`.
    pub rov_info: String,
}

/// Extracts the host from an HTTP base URL string.
///
/// If `base` has no scheme, `http://` is prepended so bare IP addresses such
/// as `"192.168.1.88"` are accepted.
///
/// # Arguments
///
/// * `base` - A full HTTP URL, hostname, or bare IP address.
///
/// # Returns
///
/// * `Option<String>` - The extracted host, or `None` if the input cannot be
///   parsed as a URL.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::parse_host_from_http_base;
///
/// assert_eq!(
///     parse_host_from_http_base("http://192.168.1.88"),
///     Some("192.168.1.88".to_string())
/// );
///
/// assert_eq!(
///     parse_host_from_http_base("192.168.1.88"),
///     Some("192.168.1.88".to_string())
/// );
///
/// assert_eq!(
///     parse_host_from_http_base("http://10.0.0.1:8080/v1/api"),
///     Some("10.0.0.1".to_string())
/// );
///
/// assert_eq!(parse_host_from_http_base(""), None);
/// ```
pub fn parse_host_from_http_base(base: &str) -> Option<String> {
    let normalized = if base.contains("://") {
        base.trim().to_owned()
    } else {
        format!("http://{}", base.trim())
    };
    Url::parse(&normalized)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

/// Extracts the host and port an RTSP URL will actually connect to.
///
/// Used to prime ARP for the RTSP source itself (not `rov_http_base`, which
/// may be a different, unrelated, or even unreachable endpoint — a bare
/// test RTSP server has no HTTP API to probe at all). Defaults to the
/// standard RTSP port 554 when the URL doesn't specify one.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::parse_rtsp_host_port;
///
/// assert_eq!(
///     parse_rtsp_host_port("rtsp://admin:admin@192.168.1.88:8554/stream/0/0"),
///     Some(("192.168.1.88".to_string(), 8554))
/// );
/// assert_eq!(
///     parse_rtsp_host_port("rtsp://192.168.1.88/stream"),
///     Some(("192.168.1.88".to_string(), 554))
/// );
/// assert_eq!(parse_rtsp_host_port("not a url"), None);
/// ```
#[must_use]
pub fn parse_rtsp_host_port(rtsp_url: &str) -> Option<(String, u16)> {
    let url = Url::parse(rtsp_url).ok()?;
    let host = url.host_str()?.to_owned();
    let port = url.port().unwrap_or(554);
    Some((host, port))
}

/// Finds the network interface that is on the same subnet as `rov_host`.
///
/// Uses `if-addrs` for cross-platform interface enumeration. On macOS, wired
/// links are preferred by inspecting `ifconfig` media types. On other
/// platforms, interface names that do not look wireless are preferred.
///
/// # Arguments
///
/// * `rov_host` - The ROV IPv4 address as a string.
///
/// # Returns
///
/// * `Option<String>` - The detected interface name, or `None` if no matching
///   interface is found or `rov_host` is not a valid IPv4 address.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::detect_rov_interface;
///
/// assert!(detect_rov_interface("not-an-ip").is_none());
/// assert!(detect_rov_interface("").is_none());
/// ```
///
/// No doctest is provided for successful detection because available network
/// interfaces depend on the host running the test.
pub fn detect_rov_interface(rov_host: &str) -> Option<String> {
    let rov_ip = rov_host.parse::<std::net::Ipv4Addr>().ok()?;
    let interfaces = if_addrs::get_if_addrs().ok()?;

    let candidates: Vec<String> = interfaces
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| {
            if let if_addrs::IfAddr::V4(v4) = iface.addr
                && v4.ip != rov_ip
            {
                let mask = u32::from(v4.netmask);
                if (u32::from(v4.ip) & mask) == (u32::from(rov_ip) & mask) {
                    return Some(iface.name);
                }
            }
            None
        })
        .collect();

    #[cfg(target_os = "macos")]
    {
        // Classify by media type rather than interface name so a wired `en0`
        // (desktop / Thunderbolt dock) is usable while a Wi-Fi adapter is not
        // mistaken for the ROV link. Read `ifconfig -a` only once.
        match macos_ifconfig_text() {
            Some(text) => candidates
                .iter()
                .find(|name| macos_interface_is_wired(&text, name.as_str()))
                .cloned()
                // No subnet-matching wired candidate: fall back to an active
                // wired adapter that has no IPv4 yet so recalibrate can assign
                // one.
                .or_else(|| select_active_macos_ethernet_interface(&text)),
            // `ifconfig` is unavailable: fall back to the first subnet match.
            None => candidates.into_iter().next(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        // No media metadata here; prefer an interface whose name does not look
        // wireless so the ROV route is not bound to Wi-Fi.
        prefer_wired_interface(&candidates)
    }
}

/// Checks whether `interface` currently holds an IPv4 address on the same
/// subnet as `rov_host`.
///
/// Unlike [`detect_rov_interface`] (which searches all interfaces for a
/// match), this checks one specific interface — used to verify a
/// previously-detected or user-selected interface is still valid.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::interface_has_rov_subnet_ipv4;
///
/// assert!(!interface_has_rov_subnet_ipv4("en0", "not-an-ip"));
/// assert!(!interface_has_rov_subnet_ipv4("nonexistent-iface-xyz", "192.168.1.88"));
/// ```
#[must_use]
pub fn interface_has_rov_subnet_ipv4(interface: &str, rov_host: &str) -> bool {
    let Ok(rov_ip) = rov_host.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return false;
    };
    interfaces.iter().any(|iface| {
        iface.name == interface
            && !iface.is_loopback()
            && matches!(&iface.addr, if_addrs::IfAddr::V4(v4) if {
                let mask = u32::from(v4.netmask);
                (u32::from(v4.ip) & mask) == (u32::from(rov_ip) & mask)
            })
    })
}

/// Returns the full `ifconfig -a` output on macOS.
///
/// This is used to inspect interface media types so wired Ethernet adapters can
/// be preferred over Wi-Fi.
///
/// # Returns
///
/// * `Option<String>` - The command output, or `None` if `ifconfig` cannot be
///   executed.
///
/// No doctest is provided because this function depends on the host operating
/// system and installed system tools.
#[cfg(target_os = "macos")]
fn macos_ifconfig_text() -> Option<String> {
    let output = std::process::Command::new("ifconfig")
        .arg("-a")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Returns whether a named macOS interface appears to be wired Ethernet.
///
/// The function inspects one interface block from `ifconfig -a` output. An
/// interface is considered wired when it has both a hardware `ether` address
/// and a `media:` line containing a wired `base` media type, such as
/// `1000baseT`.
///
/// # Arguments
///
/// * `ifconfig_text` - Text output from `ifconfig -a`.
/// * `name` - Interface name, for example `"en5"`.
///
/// # Returns
///
/// * `bool` - `true` if the named interface looks like wired Ethernet.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::macos_interface_is_wired;
///
/// let ifconfig = concat!(
///     "en5: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
///     "\tether ac:de:48:00:11:22\n",
///     "\tmedia: autoselect (1000baseT <full-duplex>)\n",
///     "\tstatus: active\n",
/// );
///
/// assert!(macos_interface_is_wired(ifconfig, "en5"));
/// assert!(!macos_interface_is_wired(ifconfig, "en9"));
/// ```
#[must_use]
pub fn macos_interface_is_wired(ifconfig_text: &str, name: &str) -> bool {
    let mut in_block = false;
    let mut has_ether = false;
    let mut wired_media = false;
    for line in ifconfig_text.lines() {
        if !line.starts_with('\t') && line.contains(": flags=") {
            in_block = line.split(':').next() == Some(name);
            continue;
        }
        if in_block {
            let trimmed = line.trim();
            if trimmed.starts_with("ether ") {
                has_ether = true;
            } else if trimmed.starts_with("media:") && trimmed.contains("base") {
                wired_media = true;
            }
        }
    }
    has_ether && wired_media
}

/// Picks a non-wireless interface from a list of candidates.
///
/// This is used on Linux and Windows, where macOS `ifconfig` media metadata is
/// not consulted. Interface names containing common wireless markers such as
/// `wlan`, `wlp`, `wifi`, `wi-fi`, `wireless`, or `wl` are skipped when
/// possible.
///
/// # Arguments
///
/// * `candidates` - Candidate interface names.
///
/// # Returns
///
/// * `Option<String>` - The preferred wired-looking interface, the first
///   candidate if all look wireless, or `None` if the list is empty.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::prefer_wired_interface;
///
/// let candidates = vec!["wlan0".to_string(), "eth0".to_string()];
/// assert_eq!(
///     prefer_wired_interface(&candidates),
///     Some("eth0".to_string())
/// );
///
/// let candidates = vec!["wlan0".to_string(), "wlp2s0".to_string()];
/// assert_eq!(
///     prefer_wired_interface(&candidates),
///     Some("wlan0".to_string())
/// );
///
/// assert_eq!(prefer_wired_interface(&[]), None);
/// ```
#[must_use]
pub fn prefer_wired_interface(candidates: &[String]) -> Option<String> {
    fn looks_wireless(name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        ["wlan", "wlp", "wifi", "wi-fi", "wireless", "wl"]
            .iter()
            .any(|pattern| lower.contains(pattern))
    }
    candidates
        .iter()
        .find(|name| !looks_wireless(name.as_str()))
        .or_else(|| candidates.first())
        .cloned()
}

/// Selects an active wired macOS `en*` adapter from `ifconfig -a` output.
///
/// This catches USB or Thunderbolt Ethernet adapters that are physically active
/// but do not yet have an IPv4 address. Selection is based on media type rather
/// than interface name, so a wired `en0` can be selected while Wi-Fi is ignored.
///
/// # Arguments
///
/// * `ifconfig_text` - Text output from `ifconfig -a`.
///
/// # Returns
///
/// * `Option<String>` - The first active wired `en*` interface, or `None` if no
///   suitable adapter is found.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::select_active_macos_ethernet_interface;
///
/// let ifconfig = concat!(
///     "en10: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
///     "\tether 11:22:33:44:55:66\n",
///     "\tmedia: autoselect (1000baseT <full-duplex>)\n",
///     "\tstatus: active\n",
/// );
///
/// assert_eq!(
///     select_active_macos_ethernet_interface(ifconfig),
///     Some("en10".to_string())
/// );
/// ```
#[must_use]
pub fn select_active_macos_ethernet_interface(ifconfig_text: &str) -> Option<String> {
    #[derive(Default)]
    struct Entry {
        name: String,
        has_ether: bool,
        active: bool,
        wired_media: bool,
    }

    fn finish(entry: &Entry) -> Option<String> {
        if entry.name.starts_with("en") && entry.has_ether && entry.active && entry.wired_media {
            Some(entry.name.clone())
        } else {
            None
        }
    }

    let mut current = Entry::default();
    for line in ifconfig_text.lines() {
        if !line.starts_with('\t') && line.contains(": flags=") {
            if let Some(name) = finish(&current) {
                return Some(name);
            }
            current = Entry {
                name: line.split(':').next().unwrap_or_default().to_string(),
                ..Entry::default()
            };
            continue;
        }

        let trimmed = line.trim();
        if trimmed.starts_with("ether ") {
            current.has_ether = true;
        } else if trimmed == "status: active" {
            current.active = true;
        } else if trimmed.starts_with("media:") && trimmed.contains("base") {
            current.wired_media = true;
        }
    }
    finish(&current)
}

/// Every non-loopback IPv4 address on this machine, paired with its
/// interface name and sorted by interface name for stable display.
///
/// Used by the Settings screen so the operator can see (and hand off) the
/// addresses another machine on the LAN would need to reach this one —
/// e.g. confirming which adapter is actually on the ROV's subnet.
#[must_use]
pub fn local_ipv4_addresses() -> Vec<(String, std::net::Ipv4Addr)> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    local_ipv4_addresses_from(&interfaces)
}

/// Pure helper behind [`local_ipv4_addresses`], over a supplied interface
/// list so it can be unit-tested on any platform.
fn local_ipv4_addresses_from(
    interfaces: &[if_addrs::Interface],
) -> Vec<(String, std::net::Ipv4Addr)> {
    let mut addrs: Vec<(String, std::net::Ipv4Addr)> = interfaces
        .iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match &iface.addr {
            if_addrs::IfAddr::V4(v4) => Some((iface.name.clone(), v4.ip)),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    addrs.sort();
    addrs
}

/// Formats interface/IP pairs (as returned by [`local_ipv4_addresses`]) into
/// a single display line for the Settings screen.
///
/// # Examples
///
/// ```
/// use std::net::Ipv4Addr;
/// use third_eye_client::network::format_local_ipv4_summary;
///
/// let addrs = vec![
///     ("en0".to_string(), Ipv4Addr::new(10, 0, 0, 2)),
///     ("en10".to_string(), Ipv4Addr::new(192, 168, 1, 103)),
/// ];
/// assert_eq!(
///     format_local_ipv4_summary(&addrs),
///     "en0: 10.0.0.2   \u{b7}   en10: 192.168.1.103"
/// );
/// assert_eq!(
///     format_local_ipv4_summary(&[]),
///     "No active network interfaces found."
/// );
/// ```
#[must_use]
pub fn format_local_ipv4_summary(addrs: &[(String, std::net::Ipv4Addr)]) -> String {
    if addrs.is_empty() {
        return "No active network interfaces found.".to_string();
    }
    addrs
        .iter()
        .map(|(name, ip)| format!("{name}: {ip}"))
        .collect::<Vec<_>>()
        .join("   \u{b7}   ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn make_iface(name: &str, addr: if_addrs::IfAddr) -> if_addrs::Interface {
        if_addrs::Interface {
            name: name.to_string(),
            addr,
            index: None,
            oper_status: if_addrs::IfOperStatus::Up,
            is_p2p: false,
            #[cfg(windows)]
            adapter_name: String::new(),
        }
    }

    fn v4_iface(name: &str, ip: [u8; 4]) -> if_addrs::Interface {
        make_iface(
            name,
            if_addrs::IfAddr::V4(if_addrs::Ifv4Addr {
                ip: Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
                netmask: Ipv4Addr::new(255, 255, 255, 0),
                prefixlen: 24,
                broadcast: None,
            }),
        )
    }

    fn v6_iface(name: &str) -> if_addrs::Interface {
        make_iface(
            name,
            if_addrs::IfAddr::V6(if_addrs::Ifv6Addr {
                ip: std::net::Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1),
                netmask: std::net::Ipv6Addr::new(0xffff, 0xffff, 0xffff, 0xffff, 0, 0, 0, 0),
                prefixlen: 64,
                broadcast: None,
            }),
        )
    }

    fn loopback_iface() -> if_addrs::Interface {
        make_iface(
            "lo0",
            if_addrs::IfAddr::V4(if_addrs::Ifv4Addr {
                ip: Ipv4Addr::LOCALHOST,
                netmask: Ipv4Addr::new(255, 0, 0, 0),
                prefixlen: 8,
                broadcast: None,
            }),
        )
    }

    // ---- local_ipv4_addresses_from -----------------------------------------

    #[test]
    fn local_ipv4_addresses_skips_loopback_and_ipv6() {
        let ifaces = vec![
            loopback_iface(),
            v6_iface("en0"),
            v4_iface("en0", [10, 0, 0, 2]),
        ];
        assert_eq!(
            local_ipv4_addresses_from(&ifaces),
            vec![("en0".to_string(), Ipv4Addr::new(10, 0, 0, 2))]
        );
    }

    #[test]
    fn local_ipv4_addresses_sorted_by_interface_name() {
        let ifaces = vec![
            v4_iface("en10", [192, 168, 1, 103]),
            v4_iface("en0", [10, 0, 0, 2]),
        ];
        assert_eq!(
            local_ipv4_addresses_from(&ifaces),
            vec![
                ("en0".to_string(), Ipv4Addr::new(10, 0, 0, 2)),
                ("en10".to_string(), Ipv4Addr::new(192, 168, 1, 103)),
            ]
        );
    }

    #[test]
    fn local_ipv4_addresses_empty_when_no_interfaces() {
        assert_eq!(local_ipv4_addresses_from(&[]), Vec::new());
    }

    // ---- local_ipv4_addresses (live wrapper) --------------------------------

    #[test]
    fn local_ipv4_addresses_returns_sorted_non_loopback() {
        // Exercises the live `if_addrs::get_if_addrs()` wrapper itself (not
        // just the pure `_from` helper above). The actual addresses depend
        // on the machine running the test, so this only checks the
        // invariants `local_ipv4_addresses_from` guarantees: sorted by
        // interface name, loopback excluded.
        let addrs = local_ipv4_addresses();
        let mut sorted = addrs.clone();
        sorted.sort();
        assert_eq!(addrs, sorted);
        assert!(addrs.iter().all(|(_, ip)| !ip.is_loopback()));
    }

    // ---- format_local_ipv4_summary ------------------------------------------

    #[test]
    fn format_local_ipv4_summary_joins_multiple_entries() {
        let addrs = vec![
            ("en0".to_string(), Ipv4Addr::new(10, 0, 0, 2)),
            ("en10".to_string(), Ipv4Addr::new(192, 168, 1, 103)),
        ];
        assert_eq!(
            format_local_ipv4_summary(&addrs),
            "en0: 10.0.0.2   \u{b7}   en10: 192.168.1.103"
        );
    }

    #[test]
    fn format_local_ipv4_summary_handles_empty() {
        assert_eq!(
            format_local_ipv4_summary(&[]),
            "No active network interfaces found."
        );
    }

    // ---- parse_rtsp_host_port -----------------------------------------------

    #[test]
    fn parse_rtsp_host_port_extracts_explicit_port() {
        assert_eq!(
            parse_rtsp_host_port("rtsp://admin:admin@192.168.1.88:8554/stream/0/0"),
            Some(("192.168.1.88".to_string(), 8554))
        );
    }

    #[test]
    fn parse_rtsp_host_port_defaults_to_554() {
        assert_eq!(
            parse_rtsp_host_port("rtsp://192.168.1.88/stream"),
            Some(("192.168.1.88".to_string(), 554))
        );
    }

    #[test]
    fn parse_rtsp_host_port_supports_hostnames() {
        assert_eq!(
            parse_rtsp_host_port("rtsp://rov.local:554/stream"),
            Some(("rov.local".to_string(), 554))
        );
    }

    #[test]
    fn parse_rtsp_host_port_rejects_invalid_url() {
        assert_eq!(parse_rtsp_host_port("not a url"), None);
        assert_eq!(parse_rtsp_host_port(""), None);
    }

    #[test]
    fn parse_rtsp_host_port_rejects_hostless_scheme() {
        // A scheme without "//" (e.g. "rtsp:stream") parses as opaque with no
        // host, unlike "rtsp://host/stream".
        assert_eq!(parse_rtsp_host_port("rtsp:stream"), None);
    }

    // ---- parse_host_from_http_base ----------------------------------------

    #[test]
    fn parse_host_full_url() {
        assert_eq!(
            parse_host_from_http_base("http://192.168.1.88"),
            Some("192.168.1.88".to_string())
        );
    }

    #[test]
    fn parse_host_bare_ip() {
        assert_eq!(
            parse_host_from_http_base("192.168.1.88"),
            Some("192.168.1.88".to_string())
        );
    }

    #[test]
    fn parse_host_with_port_and_path() {
        assert_eq!(
            parse_host_from_http_base("http://10.0.0.1:8080/v1/api"),
            Some("10.0.0.1".to_string())
        );
    }

    #[test]
    fn parse_host_whitespace() {
        assert_eq!(
            parse_host_from_http_base("  http://10.0.0.1  "),
            Some("10.0.0.1".to_string())
        );
    }

    #[test]
    fn parse_host_empty() {
        assert_eq!(parse_host_from_http_base(""), None);
    }

    #[test]
    fn parse_host_hostname() {
        assert_eq!(
            parse_host_from_http_base("http://rov.local"),
            Some("rov.local".to_string())
        );
    }

    // ---- detect_rov_interface (live system) --------------------------------

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn detect_interface_unreachable() {
        assert!(detect_rov_interface("1.2.3.4").is_none());
    }

    #[test]
    fn detect_interface_invalid_ip() {
        assert!(detect_rov_interface("not-an-ip").is_none());
    }

    #[test]
    fn detect_interface_empty() {
        assert!(detect_rov_interface("").is_none());
    }

    // ---- select_active_macos_ethernet_interface ---------------------------

    #[test]
    fn selects_active_wired_macos_adapter_without_ipv4() {
        let ifconfig = r"
en5: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 16000
	ether ac:de:48:00:11:22
	media: autoselect (100baseTX <full-duplex>)
	status: active
en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	ether be:74:bd:47:68:55
	inet 192.168.1.9 netmask 0xffffff00 broadcast 192.168.1.255
	media: autoselect
	status: active
";
        assert_eq!(
            select_active_macos_ethernet_interface(ifconfig),
            Some("en5".to_string())
        );
    }

    #[test]
    fn selects_rosetta_style_en10_adapter() {
        let ifconfig = r"
en10: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	ether 11:22:33:44:55:66
	media: autoselect (1000baseT <full-duplex>)
	status: active
";
        assert_eq!(
            select_active_macos_ethernet_interface(ifconfig),
            Some("en10".to_string())
        );
    }

    #[test]
    fn ignores_wifi_only_macos_adapter() {
        let ifconfig = r"
en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	ether be:74:bd:47:68:55
	inet 192.168.1.9 netmask 0xffffff00 broadcast 192.168.1.255
	media: autoselect
	status: active
";
        assert_eq!(select_active_macos_ethernet_interface(ifconfig), None);
    }

    #[test]
    fn ignores_inactive_wired_adapter() {
        let ifconfig = r"
en5: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 16000
	ether ac:de:48:00:11:22
	media: autoselect (100baseTX <full-duplex>)
	status: inactive
";
        assert_eq!(select_active_macos_ethernet_interface(ifconfig), None);
    }

    #[test]
    fn select_active_returns_none_for_empty_input() {
        assert_eq!(select_active_macos_ethernet_interface(""), None);
    }

    #[test]
    fn select_active_ignores_adapter_without_ether() {
        let ifconfig = r"
en7: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	media: autoselect (1000baseT <full-duplex>)
	status: active
";
        assert_eq!(select_active_macos_ethernet_interface(ifconfig), None);
    }

    #[test]
    fn select_active_ignores_non_wired_media_adapter() {
        // Has ether + active, but the media line is not a wired *base* type.
        let ifconfig = r"
en7: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
	ether ac:de:48:00:11:22
	media: autoselect
	status: active
";
        assert_eq!(select_active_macos_ethernet_interface(ifconfig), None);
    }

    #[test]
    fn select_active_returns_first_matching_adapter() {
        let ifconfig = r"
en5: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 16000
	ether ac:de:48:00:11:22
	media: autoselect (100baseTX <full-duplex>)
	status: active
en6: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 16000
	ether ac:de:48:00:33:44
	media: autoselect (1000baseT <full-duplex>)
	status: active
";
        assert_eq!(
            select_active_macos_ethernet_interface(ifconfig),
            Some("en5".to_string())
        );
    }

    #[test]
    fn selects_wired_en0_desktop_adapter() {
        // A wired en0 (desktop / Thunderbolt dock) advertises a base media
        // type, so it is now selectable rather than excluded by name.
        let ifconfig = concat!(
            "en0: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether be:74:bd:47:68:55\n",
            "\tinet 192.168.1.9 netmask 0xffffff00 broadcast 192.168.1.255\n",
            "\tmedia: autoselect (1000baseT <full-duplex>)\n",
            "\tstatus: active\n",
        );
        assert_eq!(
            select_active_macos_ethernet_interface(ifconfig),
            Some("en0".to_string())
        );
    }

    // ---- macos_interface_is_wired -----------------------------------------

    #[test]
    fn wired_classification_detects_base_media() {
        let ifconfig = concat!(
            "en5: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether ac:de:48:00:11:22\n",
            "\tmedia: autoselect (1000baseT <full-duplex>)\n",
            "\tstatus: active\n",
        );
        assert!(macos_interface_is_wired(ifconfig, "en5"));
    }

    #[test]
    fn wired_classification_rejects_wifi() {
        let ifconfig = concat!(
            "en0: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether be:74:bd:47:68:55\n",
            "\tmedia: autoselect\n",
            "\tstatus: active\n",
        );
        assert!(!macos_interface_is_wired(ifconfig, "en0"));
    }

    #[test]
    fn wired_classification_scopes_to_named_block() {
        // en0 is Wi-Fi (no base media); en5 is wired. The classifier must not
        // leak en5's media into en0's result.
        let ifconfig = concat!(
            "en0: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether be:74:bd:47:68:55\n",
            "\tmedia: autoselect\n",
            "\tstatus: active\n",
            "en5: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether ac:de:48:00:11:22\n",
            "\tmedia: autoselect (1000baseT <full-duplex>)\n",
            "\tstatus: active\n",
        );
        assert!(!macos_interface_is_wired(ifconfig, "en0"));
        assert!(macos_interface_is_wired(ifconfig, "en5"));
    }

    #[test]
    fn wired_classification_unknown_interface_is_false() {
        let ifconfig = concat!(
            "en5: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether ac:de:48:00:11:22\n",
            "\tmedia: autoselect (1000baseT <full-duplex>)\n",
        );
        assert!(!macos_interface_is_wired(ifconfig, "en9"));
    }

    // ---- prefer_wired_interface -------------------------------------------

    #[test]
    fn prefer_wired_skips_linux_wireless_names() {
        let candidates = vec!["wlan0".to_string(), "eth0".to_string()];
        assert_eq!(
            prefer_wired_interface(&candidates),
            Some("eth0".to_string())
        );
    }

    #[test]
    fn prefer_wired_skips_windows_wifi_name() {
        let candidates = vec!["Wi-Fi".to_string(), "Ethernet".to_string()];
        assert_eq!(
            prefer_wired_interface(&candidates),
            Some("Ethernet".to_string())
        );
    }

    #[test]
    fn prefer_wired_falls_back_to_first_when_all_wireless() {
        let candidates = vec!["wlan0".to_string(), "wlp2s0".to_string()];
        assert_eq!(
            prefer_wired_interface(&candidates),
            Some("wlan0".to_string())
        );
    }

    #[test]
    fn prefer_wired_returns_first_when_all_wired() {
        let candidates = vec!["eth0".to_string(), "eth1".to_string()];
        assert_eq!(
            prefer_wired_interface(&candidates),
            Some("eth0".to_string())
        );
    }

    #[test]
    fn prefer_wired_none_on_empty() {
        assert_eq!(prefer_wired_interface(&[]), None);
    }

    #[test]
    fn recalibrate_result_holds_fields() {
        let result = RecalibrateResult {
            interface: "en10".to_string(),
            rov_info: "Detected ROV interface en10.".to_string(),
        };
        assert_eq!(result.interface, "en10");
        assert!(result.rov_info.contains("en10"));
    }
}
