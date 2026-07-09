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
