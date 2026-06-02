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

    // On macOS prefer any interface over en0 (en0 is typically WiFi;
    // wired USB-ethernet adapters appear as en5, en6, etc.).
    #[cfg(target_os = "macos")]
    {
        candidates
            .iter()
            .find(|name| name.as_str() != "en0")
            .cloned()
    }

    #[cfg(not(target_os = "macos"))]
    candidates.into_iter().next()
}
