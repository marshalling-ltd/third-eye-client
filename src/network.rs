//! Network detection and recalibration helpers.
//!
//! These functions are pure or `Send`-safe and live in the library crate so
//! they can be tested from `tests/`.

use reqwest::Url;

/// Result of a background ROV network recalibration.
pub struct RecalibrateResult {
    /// Detected interface name, or empty if none found.
    pub interface: String,
    /// Human-readable status summary for `rov_info`.
    pub rov_info: String,
}

/// Extracts the host from an HTTP base URL string.
///
/// Prepends `http://` if no scheme is present so bare IPs like
/// `"192.168.1.88"` work correctly.
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

/// Finds the network interface that is on the same subnet as `rov_host`.
///
/// Uses `if-addrs` for cross-platform interface enumeration.  On macOS the
/// WiFi adapter (`en0`) is excluded so that wired USB-ethernet adapters are
/// preferred; on other platforms the first matching non-loopback interface
/// is returned.
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
        // Prefer a wired interface over WiFi (en0).
        candidates
            .iter()
            .find(|name| name.as_str() != "en0")
            .cloned()
            .or_else(|| {
                // No wired interface has IPv4 on the ROV subnet — look for
                // an active wired adapter so recalibrate can assign an IP.
                detect_active_macos_ethernet_interface()
            })
    }

    #[cfg(not(target_os = "macos"))]
    candidates.into_iter().next()
}

#[cfg(target_os = "macos")]
fn detect_active_macos_ethernet_interface() -> Option<String> {
    let output = std::process::Command::new("ifconfig")
        .arg("-a")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    select_active_macos_ethernet_interface(&text)
}

/// Selects an active wired macOS `en*` adapter from `ifconfig -a` output.
///
/// This catches USB Ethernet adapters that are physically active but do not
/// have an IPv4 address yet. WiFi (`en0` on normal macOS installs) is not
/// selected here; ROV WiFi should use normal OS routing instead.
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
        if entry.name.starts_with("en")
            && entry.name != "en0"
            && entry.has_ether
            && entry.active
            && entry.wired_media
        {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
