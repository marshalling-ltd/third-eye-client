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

// ---------------------------------------------------------------------------
// ROV link detection
// ---------------------------------------------------------------------------

/// What kind of link a network interface is. Declared in order of preference
/// for carrying ROV traffic: a cable beats Wi-Fi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkKind {
    Wired,
    Unknown,
    Wireless,
}

impl LinkKind {
    /// Short name for status messages.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Wired => "wired (USB Ethernet)",
            LinkKind::Wireless => "Wi-Fi",
            LinkKind::Unknown => "network",
        }
    }
}

/// A local interface with its IPv4 addresses, as seen at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    pub name: String,
    pub kind: LinkKind,
    /// `(address, netmask)` pairs. Empty for an active wired adapter that has
    /// no IPv4 address yet; may hold only a link-local one.
    pub ipv4: Vec<(std::net::Ipv4Addr, std::net::Ipv4Addr)>,
}

/// The interface chosen to reach the ROV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RovLink {
    pub interface: String,
    pub kind: LinkKind,
    /// This computer's address on the ROV's subnet, or `None` when the
    /// adapter is connected but has no address there yet.
    pub local_ip: Option<std::net::Ipv4Addr>,
}

fn on_subnet(ip: std::net::Ipv4Addr, mask: std::net::Ipv4Addr, other: std::net::Ipv4Addr) -> bool {
    let mask = u32::from(mask);
    (u32::from(ip) & mask) == (u32::from(other) & mask)
}

/// Picks the interface to reach `rov_ip` from a snapshot of local interfaces.
///
/// 1. Interfaces with an address on the ROV's subnet, preferring a cable over
///    Wi-Fi (ties go to the lowest interface name, so the choice is stable).
///    An unrelated link such as a phone hotspot is never chosen, because it
///    has no address on the ROV's subnet.
/// 2. Otherwise a wired adapter with no IPv4 address yet, or only a
///    link-local one (169.254.x.x, what an adapter without DHCP falls back
///    to): the cable is in but the adapter still needs an address on the
///    ROV's network.
/// 3. Otherwise `None`.
#[must_use]
pub fn select_rov_link(
    interfaces: &[InterfaceInfo],
    rov_ip: std::net::Ipv4Addr,
) -> Option<RovLink> {
    let mut on_rov_subnet: Vec<(LinkKind, &str, std::net::Ipv4Addr)> = interfaces
        .iter()
        .filter_map(|iface| {
            iface
                .ipv4
                .iter()
                .find(|(ip, mask)| *ip != rov_ip && on_subnet(*ip, *mask, rov_ip))
                .map(|(ip, _)| (iface.kind, iface.name.as_str(), *ip))
        })
        .collect();
    on_rov_subnet.sort();
    if let Some((kind, name, ip)) = on_rov_subnet.first() {
        return Some(RovLink {
            interface: (*name).to_owned(),
            kind: *kind,
            local_ip: Some(*ip),
        });
    }
    interfaces
        .iter()
        .filter(|iface| {
            iface.kind == LinkKind::Wired && iface.ipv4.iter().all(|(ip, _)| ip.is_link_local())
        })
        .min_by(|a, b| a.name.cmp(&b.name))
        .map(|iface| RovLink {
            interface: iface.name.clone(),
            kind: LinkKind::Wired,
            local_ip: None,
        })
}

/// Status text for the UI: what was found, or what to check when nothing was.
#[must_use]
pub fn describe_rov_link(
    link: Option<&RovLink>,
    rov_host: &str,
    interfaces: &[InterfaceInfo],
) -> String {
    match link {
        Some(RovLink {
            interface,
            kind,
            local_ip: Some(ip),
        }) => format!(
            "Connected to the ROV over {} ({interface}): this computer {ip} -> ROV {rov_host}.",
            kind.label()
        ),
        Some(RovLink { interface, .. }) => format!(
            "Cable detected on {interface}, but it has no address on the ROV's network \
             ({rov_host}). Press Recalibrate to set one up."
        ),
        None => match rov_host.parse::<std::net::Ipv4Addr>() {
            // The ROV address comes from the active device's configuration and
            // can be a host name or a routed (non-local) address; neither can
            // be matched to a cable or Wi-Fi network, and neither needs to be.
            Err(_) => format!(
                "The ROV address \"{rov_host}\" is a name, not an IP address, so the \
                 connection cannot be detected automatically. Using normal OS routing."
            ),
            Ok(ip) if !(ip.is_private() || ip.is_link_local()) => format!(
                "The ROV address {rov_host} is not on a local network, so no cable or Wi-Fi \
                 link is chosen. Using normal OS routing."
            ),
            Ok(_) => {
                let addresses: Vec<String> = interfaces
                    .iter()
                    .flat_map(|iface| {
                        iface
                            .ipv4
                            .iter()
                            .map(|(ip, _)| format!("{} {ip}", iface.name))
                    })
                    .collect();
                let seen = if addresses.is_empty() {
                    "This computer has no network addresses at all.".to_owned()
                } else {
                    format!(
                        "None of this computer's addresses ({}) are on the ROV's network \
                         ({rov_host}).",
                        addresses.join(", ")
                    )
                };
                format!(
                    "ROV not found. {seen} Check that the ROV is powered on, then join its \
                     Wi-Fi network or plug in the USB Ethernet cable."
                )
            }
        },
    }
}

/// Whether Windows `route PRINT <host>` output lists a /32 route for `host`
/// that leaves through the adapter whose address is `local_ip`. A route
/// through another adapter (for example a leftover one over Wi-Fi) does not
/// count.
///
/// # Examples
///
/// ```
/// use std::net::Ipv4Addr;
/// use third_eye_client::network::route_print_uses_interface;
///
/// let output = "  192.168.1.88  255.255.255.255  On-link  192.168.1.103  281";
/// assert!(route_print_uses_interface(output, "192.168.1.88", Ipv4Addr::new(192, 168, 1, 103)));
/// assert!(!route_print_uses_interface(output, "192.168.1.88", Ipv4Addr::new(192, 168, 1, 50)));
/// ```
#[must_use]
pub fn route_print_uses_interface(output: &str, host: &str, local_ip: std::net::Ipv4Addr) -> bool {
    let want = local_ip.to_string();
    output.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.len() >= 4
            && fields[0] == host
            && fields[1] == "255.255.255.255"
            && fields[3] == want
    })
}

/// Whether `interface` appears in `snapshot` (so it is up) with an address on
/// the ROV's subnet. Unlike an address lookup alone, this is false for an
/// unplugged adapter that kept its IPv4 address.
#[must_use]
pub fn snapshot_has_rov_subnet_address(
    snapshot: &[InterfaceInfo],
    interface: &str,
    rov_ip: std::net::Ipv4Addr,
) -> bool {
    snapshot
        .iter()
        .filter(|iface| iface.name == interface)
        .flat_map(|iface| iface.ipv4.iter())
        .any(|(ip, mask)| on_subnet(*ip, *mask, rov_ip))
}

/// Whether the RTSP source is this machine (loopback, such as the bundled
/// test server), and so unaffected by which cable or Wi-Fi link reaches the
/// ROV. The RTSP and HTTP endpoints are configured independently.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::rtsp_url_is_independent_of_rov_link;
///
/// assert!(rtsp_url_is_independent_of_rov_link("rtsp://admin:admin@127.0.0.1:8554/stream"));
/// assert!(rtsp_url_is_independent_of_rov_link("rtsp://localhost/stream"));
/// assert!(!rtsp_url_is_independent_of_rov_link("rtsp://192.168.1.88:8554/stream/0/0"));
/// ```
#[must_use]
pub fn rtsp_url_is_independent_of_rov_link(rtsp_url: &str) -> bool {
    let Some((host, _)) = parse_rtsp_host_port(rtsp_url) else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(|c| c == '[' || c == ']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
}

/// Which interfaces the app should remember after a detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkBinding {
    /// The ROV interface, even if it has no address yet (empty if none).
    pub detected_interface: String,
    /// The interface to bind HTTP and telemetry sockets to; empty means
    /// default OS routing. Only set when the interface has an address on
    /// the ROV's subnet, because binding to one without would fail.
    pub bound_interface: String,
}

/// Decides what to remember after detecting `link`. `current_bound` is the
/// binding from before; with no link it is dropped when that interface no
/// longer has an address on the ROV's subnet (cable pulled), but a still-valid
/// manual binding is kept.
#[must_use]
pub fn plan_link_binding(
    link: Option<&RovLink>,
    current_bound: &str,
    still_on_rov_subnet: &dyn Fn(&str) -> bool,
) -> LinkBinding {
    if let Some(link) = link {
        LinkBinding {
            detected_interface: link.interface.clone(),
            bound_interface: if link.local_ip.is_some() {
                link.interface.clone()
            } else {
                String::new()
            },
        }
    } else {
        let current = current_bound.trim();
        LinkBinding {
            detected_interface: String::new(),
            bound_interface: if current.is_empty() || !still_on_rov_subnet(current) {
                String::new()
            } else {
                current.to_owned()
            },
        }
    }
}

/// Builds the interface snapshot from `if-addrs` output. `kind_of` classifies
/// each interface by name (`None` = unknown); loopback and IPv6-only
/// interfaces are skipped.
#[must_use]
pub fn interfaces_from(
    ifaces: &[if_addrs::Interface],
    kind_of: &dyn Fn(&str) -> LinkKind,
) -> Vec<InterfaceInfo> {
    let mut out: Vec<InterfaceInfo> = Vec::new();
    // An adapter keeps its address after the cable is pulled; only links that
    // are operationally up (carrier present) can carry ROV traffic.
    for iface in ifaces.iter().filter(|i| !i.is_loopback() && i.is_oper_up()) {
        let if_addrs::IfAddr::V4(v4) = &iface.addr else {
            continue;
        };
        match out.iter_mut().find(|known| known.name == iface.name) {
            Some(known) => known.ipv4.push((v4.ip, v4.netmask)),
            None => out.push(InterfaceInfo {
                name: iface.name.clone(),
                kind: kind_of(&iface.name),
                ipv4: vec![(v4.ip, v4.netmask)],
            }),
        }
    }
    out
}

/// Sorted `(interface, address, netmask)` list, cheap to compute (no
/// subprocess). Comparing two of these tells whether the network changed.
#[must_use]
pub fn network_signature() -> NetworkSignature {
    signature_from(&if_addrs::get_if_addrs().unwrap_or_default())
}

fn signature_from(ifaces: &[if_addrs::Interface]) -> NetworkSignature {
    let mut sig: Vec<_> = ifaces
        .iter()
        .filter(|i| !i.is_loopback() && i.is_oper_up())
        .filter_map(|i| match &i.addr {
            if_addrs::IfAddr::V4(v4) => Some((i.name.clone(), v4.ip, v4.netmask)),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    sig.sort();
    sig
}

/// Output of [`network_signature`]: `(interface, address, netmask)`, sorted.
pub type NetworkSignature = Vec<(String, std::net::Ipv4Addr, std::net::Ipv4Addr)>;

/// How many quiet ticks pass between re-checks while the ROV is not reachable,
/// to catch a cable that is plugged in but has no address yet.
pub const RETRY_EVERY_TICKS: u32 = 5;

/// Decides, tick by tick, when the ROV link should be detected again.
///
/// A change in the network is acted on one tick later, so a cable that is
/// still getting its address is not judged half-way. While the ROV link is
/// missing it also asks for a re-check every [`RETRY_EVERY_TICKS`] ticks.
#[derive(Debug, Default)]
pub struct LinkWatcher {
    signature: NetworkSignature,
    recheck: bool,
    stable_ticks: u32,
}

impl LinkWatcher {
    /// Records `signature` as already handled (call after detecting).
    pub fn reset(&mut self, signature: NetworkSignature) {
        self.signature = signature;
        self.recheck = false;
        self.stable_ticks = 0;
    }

    /// Feeds the current signature; returns `true` when the link should be
    /// detected again now.
    pub fn tick(&mut self, now: NetworkSignature, link_ready: bool) -> bool {
        if now != self.signature {
            self.signature = now;
            self.recheck = true;
            self.stable_ticks = 0;
            return false;
        }
        self.stable_ticks = self.stable_ticks.saturating_add(1);
        let retry = !link_ready && self.stable_ticks.is_multiple_of(RETRY_EVERY_TICKS);
        let due = self.recheck || retry;
        self.recheck = false;
        due
    }
}

/// Classifies interfaces using the media information in `ifconfig -a` output
/// (macOS): a `baseT`-style medium means a cable, anything else is Wi-Fi.
/// With no `ifconfig` text the kind is unknown. A cable that is plugged in but
/// has no IPv4 yet is invisible to `if-addrs`, so it is added from the text.
#[must_use]
pub fn interfaces_with_media(
    ifaces: &[if_addrs::Interface],
    ifconfig: Option<&str>,
) -> Vec<InterfaceInfo> {
    let Some(text) = ifconfig else {
        return interfaces_from(ifaces, &|_| LinkKind::Unknown);
    };
    let mut out = interfaces_from(ifaces, &|name| {
        if macos_interface_is_wired(text, name) {
            LinkKind::Wired
        } else {
            LinkKind::Wireless
        }
    });
    for name in active_macos_ethernet_interfaces(text) {
        if !out.iter().any(|i| i.name == name) {
            out.push(InterfaceInfo {
                name,
                kind: LinkKind::Wired,
                ipv4: Vec::new(),
            });
        }
    }
    out
}

/// Classifies interfaces by name (Linux, Windows): names that look wireless
/// are Wi-Fi, everything else is treated as a cable.
#[must_use]
pub fn interfaces_by_name(ifaces: &[if_addrs::Interface]) -> Vec<InterfaceInfo> {
    interfaces_from(ifaces, &|name| {
        if looks_wireless(name) {
            LinkKind::Wireless
        } else {
            LinkKind::Wired
        }
    })
}

/// Snapshot of this machine's interfaces with their link kinds. On macOS this
/// runs `ifconfig`, so call it when the network changed, not on a fast timer.
#[must_use]
pub fn snapshot_interfaces() -> Vec<InterfaceInfo> {
    let ifaces = if_addrs::get_if_addrs().unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        interfaces_with_media(&ifaces, macos_ifconfig_text().as_deref())
    }
    #[cfg(not(target_os = "macos"))]
    {
        interfaces_by_name(&ifaces)
    }
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

/// Whether an interface name looks like a wireless adapter (`wlan0`, `wlp3s0`,
/// `Wi-Fi`, ...). A heuristic for platforms without media information.
fn looks_wireless(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["wlan", "wlp", "wifi", "wi-fi", "wireless", "wl"]
        .iter()
        .any(|pattern| lower.contains(pattern))
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
    active_macos_ethernet_interfaces(ifconfig_text)
        .into_iter()
        .next()
}

/// Every active wired macOS `en*` adapter in `ifconfig -a` output, in the
/// order they appear. See [`select_active_macos_ethernet_interface`] for how
/// an adapter qualifies.
///
/// # Examples
///
/// ```
/// use third_eye_client::network::active_macos_ethernet_interfaces;
///
/// let ifconfig = concat!(
///     "en5: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
///     "\tether 11:22:33:44:55:66\n",
///     "\tmedia: autoselect (1000baseT <full-duplex>)\n",
///     "\tstatus: active\n",
///     "en7: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
///     "\tether 11:22:33:44:55:77\n",
///     "\tmedia: autoselect (100baseTX <full-duplex>)\n",
///     "\tstatus: active\n",
/// );
///
/// assert_eq!(active_macos_ethernet_interfaces(ifconfig), ["en5", "en7"]);
/// ```
#[must_use]
pub fn active_macos_ethernet_interfaces(ifconfig_text: &str) -> Vec<String> {
    #[derive(Default)]
    struct Entry {
        name: String,
        has_ether: bool,
        active: bool,
        wired_media: bool,
    }

    fn finish(entry: &Entry, found: &mut Vec<String>) {
        if entry.name.starts_with("en") && entry.has_ether && entry.active && entry.wired_media {
            found.push(entry.name.clone());
        }
    }

    let mut found = Vec::new();
    let mut current = Entry::default();
    for line in ifconfig_text.lines() {
        if !line.starts_with('\t') && line.contains(": flags=") {
            finish(&current, &mut found);
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
    finish(&current, &mut found);
    found
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

/// Picks the local IPv4 address on `interface` to use as the source address
/// for connections to `host`.
///
/// Prefers the address on the same subnet as `host`, so an unrelated address
/// or alias listed first on the adapter is never chosen. If `host` is not on
/// any subnet of the interface (off-subnet, reached through a gateway), falls
/// back to the interface's first IPv4 address. Returns `None` when the
/// interface has no IPv4 address at all.
///
/// Pure over the supplied interface list so it can be unit-tested on any
/// platform.
#[must_use]
pub fn source_ipv4_for_host(
    interfaces: &[if_addrs::Interface],
    interface: &str,
    host: std::net::Ipv4Addr,
) -> Option<std::net::Ipv4Addr> {
    let candidates: Vec<&if_addrs::Ifv4Addr> = interfaces
        .iter()
        .filter(|iface| iface.name == interface && !iface.is_loopback())
        .filter_map(|iface| match &iface.addr {
            if_addrs::IfAddr::V4(v4) => Some(v4),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    candidates
        .iter()
        .find(|v4| {
            let mask = u32::from(v4.netmask);
            (u32::from(v4.ip) & mask) == (u32::from(host) & mask)
        })
        .or(candidates.first())
        .map(|v4| v4.ip)
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

    // ---- source_ipv4_for_host ----------------------------------------------

    #[test]
    fn source_ipv4_prefers_address_on_hosts_subnet() {
        let ifaces = vec![
            v4_iface("eth1", [169, 254, 7, 7]),
            v4_iface("eth1", [192, 168, 1, 103]),
        ];
        assert_eq!(
            source_ipv4_for_host(&ifaces, "eth1", Ipv4Addr::new(192, 168, 1, 88)),
            Some(Ipv4Addr::new(192, 168, 1, 103))
        );
    }

    #[test]
    fn source_ipv4_falls_back_to_first_address_for_off_subnet_host() {
        let ifaces = vec![
            v4_iface("eth1", [10, 0, 0, 5]),
            v4_iface("eth1", [10, 0, 1, 5]),
        ];
        assert_eq!(
            source_ipv4_for_host(&ifaces, "eth1", Ipv4Addr::new(192, 168, 1, 88)),
            Some(Ipv4Addr::new(10, 0, 0, 5))
        );
    }

    #[test]
    fn source_ipv4_ignores_other_interfaces_ipv6_and_loopback() {
        let ifaces = vec![
            v4_iface("wifi0", [192, 168, 1, 50]),
            v6_iface("eth1"),
            loopback_iface(),
        ];
        assert_eq!(
            source_ipv4_for_host(&ifaces, "eth1", Ipv4Addr::new(192, 168, 1, 88)),
            None
        );
        assert_eq!(
            source_ipv4_for_host(&ifaces, "missing", Ipv4Addr::new(192, 168, 1, 88)),
            None
        );
    }

    // ---- ROV link detection ------------------------------------------------

    fn info(name: &str, kind: LinkKind, addrs: &[[u8; 4]]) -> InterfaceInfo {
        InterfaceInfo {
            name: name.to_owned(),
            kind,
            ipv4: addrs
                .iter()
                .map(|a| {
                    (
                        Ipv4Addr::new(a[0], a[1], a[2], a[3]),
                        Ipv4Addr::new(255, 255, 255, 0),
                    )
                })
                .collect(),
        }
    }

    const ROV: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 88);

    #[test]
    fn link_prefers_a_cable_over_wifi_on_the_rov_subnet() {
        let ifaces = [
            info("en0", LinkKind::Wireless, &[[192, 168, 1, 50]]),
            info("en7", LinkKind::Wired, &[[192, 168, 1, 103]]),
        ];
        let link = select_rov_link(&ifaces, ROV).unwrap();
        assert_eq!(link.interface, "en7");
        assert_eq!(link.kind, LinkKind::Wired);
        assert_eq!(link.local_ip, Some(Ipv4Addr::new(192, 168, 1, 103)));
    }

    #[test]
    fn link_uses_wifi_when_it_is_the_only_path() {
        let ifaces = [info("en0", LinkKind::Wireless, &[[192, 168, 1, 50]])];
        let link = select_rov_link(&ifaces, ROV).unwrap();
        assert_eq!(
            (link.interface.as_str(), link.kind),
            ("en0", LinkKind::Wireless)
        );
    }

    #[test]
    fn link_ignores_a_phone_hotspot_while_the_rov_is_on_usb() {
        // Scenario C: internet through an iPhone hotspot, ROV through USB.
        let ifaces = [
            info("en0", LinkKind::Wireless, &[[172, 20, 10, 4]]),
            info("en7", LinkKind::Wired, &[[192, 168, 1, 103]]),
        ];
        assert_eq!(select_rov_link(&ifaces, ROV).unwrap().interface, "en7");
    }

    #[test]
    fn link_finds_nothing_when_only_a_hotspot_is_connected() {
        let ifaces = [info("en0", LinkKind::Wireless, &[[172, 20, 10, 4]])];
        assert!(select_rov_link(&ifaces, ROV).is_none());
    }

    #[test]
    fn link_never_picks_an_interface_that_owns_the_rov_address() {
        let ifaces = [info("en0", LinkKind::Wired, &[[192, 168, 1, 88]])];
        assert!(select_rov_link(&ifaces, ROV).is_none());
    }

    #[test]
    fn link_uses_the_address_on_the_rov_subnet_among_several() {
        let ifaces = [info(
            "eth1",
            LinkKind::Wired,
            &[[169, 254, 7, 7], [192, 168, 1, 103]],
        )];
        let link = select_rov_link(&ifaces, ROV).unwrap();
        assert_eq!(link.local_ip, Some(Ipv4Addr::new(192, 168, 1, 103)));
    }

    #[test]
    fn link_reports_a_plugged_in_adapter_that_has_no_address_yet() {
        let ifaces = [
            info("en0", LinkKind::Wireless, &[[172, 20, 10, 4]]),
            info("en7", LinkKind::Wired, &[]),
        ];
        let link = select_rov_link(&ifaces, ROV).unwrap();
        assert_eq!((link.interface.as_str(), link.local_ip), ("en7", None));
    }

    #[test]
    fn several_adapters_awaiting_an_address_resolve_to_the_lowest_name() {
        let ifaces = [
            info("en9", LinkKind::Wired, &[]),
            info("en6", LinkKind::Wired, &[[169, 254, 3, 3]]),
            info("en0", LinkKind::Wireless, &[]),
        ];
        let mut reversed = ifaces.clone();
        reversed.reverse();
        for order in [&ifaces, &reversed] {
            let link = select_rov_link(order, ROV).unwrap();
            assert_eq!((link.interface.as_str(), link.local_ip), ("en6", None));
        }
    }

    #[test]
    fn link_prefers_a_ready_adapter_over_one_waiting_for_an_address() {
        let ifaces = [
            info("en5", LinkKind::Wired, &[]),
            info("en0", LinkKind::Wireless, &[[192, 168, 1, 50]]),
        ];
        assert_eq!(select_rov_link(&ifaces, ROV).unwrap().interface, "en0");
    }

    #[test]
    fn link_choice_is_stable_between_equal_candidates() {
        let a = [
            info("eth2", LinkKind::Wired, &[[192, 168, 1, 5]]),
            info("eth1", LinkKind::Wired, &[[192, 168, 1, 6]]),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(select_rov_link(&a, ROV), select_rov_link(&b, ROV));
        assert_eq!(select_rov_link(&a, ROV).unwrap().interface, "eth1");
    }

    #[test]
    fn link_labels_and_order_put_cables_first() {
        assert!(LinkKind::Wired < LinkKind::Unknown && LinkKind::Unknown < LinkKind::Wireless);
        assert!(LinkKind::Wired.label().contains("USB"));
        assert_eq!(LinkKind::Wireless.label(), "Wi-Fi");
        assert_eq!(LinkKind::Unknown.label(), "network");
    }

    #[test]
    fn describe_link_explains_named_and_routed_rov_addresses() {
        let ifaces = [info("en0", LinkKind::Wireless, &[[10, 0, 0, 2]])];
        let named = describe_rov_link(None, "rov.example.com", &ifaces);
        assert!(named.contains("name") && named.contains("OS routing"));
        assert!(!named.contains("ROV not found"));
        let routed = describe_rov_link(None, "203.0.113.7", &ifaces);
        assert!(routed.contains("not on a local network") && routed.contains("OS routing"));
        // Private and link-local addresses are the ones worth looking for.
        for local in ["192.168.4.1", "10.1.2.3", "172.16.0.9", "169.254.1.1"] {
            assert!(
                describe_rov_link(None, local, &ifaces).contains("ROV not found"),
                "{local}"
            );
        }
    }

    #[test]
    fn describe_link_covers_ready_waiting_and_missing() {
        let ready = RovLink {
            interface: "en7".into(),
            kind: LinkKind::Wired,
            local_ip: Some(Ipv4Addr::new(192, 168, 1, 103)),
        };
        let text = describe_rov_link(Some(&ready), "192.168.1.88", &[]);
        assert!(text.contains("wired") && text.contains("en7") && text.contains("192.168.1.103"));

        let waiting = RovLink {
            local_ip: None,
            ..ready
        };
        assert!(describe_rov_link(Some(&waiting), "192.168.1.88", &[]).contains("Recalibrate"));

        let ifaces = [info("en0", LinkKind::Wireless, &[[172, 20, 10, 4]])];
        let missing = describe_rov_link(None, "192.168.1.88", &ifaces);
        assert!(missing.contains("ROV not found") && missing.contains("en0 172.20.10.4"));
        assert!(missing.contains("powered on") && missing.contains("Wi-Fi"));
        assert!(describe_rov_link(None, "192.168.1.88", &[]).contains("no network addresses"));
    }

    #[test]
    fn snapshot_groups_addresses_by_interface_and_skips_loopback_and_ipv6() {
        let ifaces = vec![
            v4_iface("eth1", [169, 254, 7, 7]),
            v4_iface("eth1", [192, 168, 1, 103]),
            v6_iface("eth1"),
            v6_iface("v6only"),
            loopback_iface(),
            v4_iface("wlan0", [10, 0, 0, 2]),
        ];
        let kind = |name: &str| {
            if name.starts_with("wlan") {
                LinkKind::Wireless
            } else {
                LinkKind::Wired
            }
        };
        let snapshot = interfaces_from(&ifaces, &kind);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].name, "eth1");
        assert_eq!(snapshot[0].ipv4.len(), 2);
        assert_eq!(snapshot[1].kind, LinkKind::Wireless);
    }

    #[test]
    fn binding_follows_a_ready_link_and_waits_for_an_address() {
        let ready = RovLink {
            interface: "en7".into(),
            kind: LinkKind::Wired,
            local_ip: Some(Ipv4Addr::new(192, 168, 1, 103)),
        };
        let never = |_: &str| false;
        let b = plan_link_binding(Some(&ready), "old0", &never);
        assert_eq!(
            (b.detected_interface.as_str(), b.bound_interface.as_str()),
            ("en7", "en7")
        );

        let waiting = RovLink {
            local_ip: None,
            ..ready
        };
        let b = plan_link_binding(Some(&waiting), "old0", &never);
        assert_eq!(
            (b.detected_interface.as_str(), b.bound_interface.as_str()),
            ("en7", "")
        );
    }

    #[test]
    fn binding_without_a_link_drops_dead_interfaces_but_keeps_valid_ones() {
        let gone = |_: &str| false;
        let b = plan_link_binding(None, "en7", &gone);
        assert_eq!(
            (b.detected_interface.as_str(), b.bound_interface.as_str()),
            ("", "")
        );
        let alive = |name: &str| name == "en7";
        assert_eq!(
            plan_link_binding(None, " en7 ", &alive).bound_interface,
            "en7"
        );
        assert_eq!(plan_link_binding(None, "", &alive).bound_interface, "");
    }

    #[test]
    fn media_classification_tells_cables_from_wifi_and_finds_addressless_cables() {
        let ifconfig = concat!(
            "en0: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether aa:bb:cc:00:00:01\n",
            "\tmedia: autoselect\n",
            "\tstatus: active\n",
            "en5: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether aa:bb:cc:00:00:03\n",
            "\tmedia: autoselect (100baseTX <full-duplex>)\n",
            "\tstatus: active\n",
            "en7: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n",
            "\tether aa:bb:cc:00:00:02\n",
            "\tmedia: autoselect (1000baseT <full-duplex>)\n",
            "\tstatus: active\n",
        );
        let ifaces = vec![
            v4_iface("en0", [192, 168, 1, 50]),
            v4_iface("en7", [192, 168, 1, 103]),
        ];
        let out = interfaces_with_media(&ifaces, Some(ifconfig));
        let kind = |n: &str| out.iter().find(|i| i.name == n).map(|i| i.kind);
        assert_eq!(kind("en0"), Some(LinkKind::Wireless));
        assert_eq!(kind("en7"), Some(LinkKind::Wired));
        let waiting = out
            .iter()
            .find(|i| i.name == "en5")
            .expect("addressless cable listed");
        assert!(waiting.ipv4.is_empty() && waiting.kind == LinkKind::Wired);

        // Without ifconfig text nothing can be classified.
        let unknown = interfaces_with_media(&ifaces, None);
        assert!(unknown.iter().all(|i| i.kind == LinkKind::Unknown));
    }

    #[test]
    fn name_classification_treats_wireless_looking_names_as_wifi() {
        let ifaces = vec![
            v4_iface("wlan0", [10, 0, 0, 2]),
            v4_iface("Wi-Fi", [10, 0, 1, 2]),
            v4_iface("enp3s0", [10, 0, 2, 2]),
            v4_iface("Ethernet 2", [10, 0, 3, 2]),
        ];
        let kinds: Vec<LinkKind> = interfaces_by_name(&ifaces).iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            [
                LinkKind::Wireless,
                LinkKind::Wireless,
                LinkKind::Wired,
                LinkKind::Wired
            ]
        );
    }

    #[test]
    fn live_snapshot_and_signature_agree_with_each_other() {
        // Runs the real OS calls: only checks they work and are consistent.
        let snapshot = snapshot_interfaces();
        let signature = network_signature();
        assert!(
            signature.windows(2).all(|w| w[0] <= w[1]),
            "signature is sorted"
        );
        assert!(
            signature
                .iter()
                .all(|(name, _, _)| snapshot.iter().any(|i| &i.name == name)),
            "every addressed interface is in the snapshot"
        );
    }

    #[test]
    fn links_without_carrier_are_ignored_even_if_they_keep_their_address() {
        let mut down = v4_iface("eth1", [192, 168, 1, 103]);
        down.oper_status = if_addrs::IfOperStatus::Down;
        let wifi = v4_iface("wlan0", [10, 0, 0, 2]);
        let ifaces = vec![down.clone(), wifi.clone()];
        let snapshot = interfaces_by_name(&ifaces);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].name, "wlan0");
        // The signature changes when the cable is pulled, so it is noticed.
        let plugged = signature_from(&[v4_iface("eth1", [192, 168, 1, 103]), wifi]);
        assert_ne!(plugged, signature_from(&ifaces));
    }

    #[test]
    fn a_cable_with_only_a_link_local_address_awaits_configuration() {
        let ifaces = [
            info("en0", LinkKind::Wireless, &[[172, 20, 10, 4]]),
            info("en7", LinkKind::Wired, &[[169, 254, 12, 34]]),
        ];
        let link = select_rov_link(&ifaces, ROV).unwrap();
        assert_eq!((link.interface.as_str(), link.local_ip), ("en7", None));
        // A real address elsewhere on the cable is not "awaiting".
        let configured = [info(
            "en7",
            LinkKind::Wired,
            &[[169, 254, 1, 1], [10, 9, 9, 9]],
        )];
        assert!(select_rov_link(&configured, ROV).is_none());
    }

    #[test]
    fn every_active_wired_adapter_is_listed_not_just_the_first() {
        let block = |name: &str, mac: u8| {
            format!(
                "{name}: flags=8863<UP,BROADCAST,RUNNING> mtu 1500\n\tether aa:bb:cc:00:00:{mac:02x}\n\tmedia: autoselect (1000baseT <full-duplex>)\n\tstatus: active\n"
            )
        };
        let ifconfig = format!("{}{}", block("en6", 6), block("en9", 9));
        assert_eq!(active_macos_ethernet_interfaces(&ifconfig), ["en6", "en9"]);
        assert_eq!(
            active_macos_ethernet_interfaces(""),
            [] as [std::string::String; 0]
        );

        // An addressed dock listed first must not hide an addressless ROV adapter.
        let ifaces = vec![v4_iface("en6", [192, 168, 50, 5])];
        let out = interfaces_with_media(&ifaces, Some(&ifconfig));
        assert!(out.iter().any(|i| i.name == "en6" && !i.ipv4.is_empty()));
        assert!(out.iter().any(|i| i.name == "en9" && i.ipv4.is_empty()));
    }

    #[test]
    fn route_print_must_leave_through_the_selected_adapter() {
        let output = "Active Routes:\n  Network Destination   Netmask   Gateway   Interface  Metric\n  \
                      192.168.1.88  255.255.255.255  On-link  192.168.1.50  306\n";
        let ethernet = Ipv4Addr::new(192, 168, 1, 103);
        let wifi = Ipv4Addr::new(192, 168, 1, 50);
        assert!(route_print_uses_interface(output, "192.168.1.88", wifi));
        assert!(!route_print_uses_interface(
            output,
            "192.168.1.88",
            ethernet
        ));
        assert!(!route_print_uses_interface(output, "192.168.1.99", wifi));
        assert!(!route_print_uses_interface("", "192.168.1.88", wifi));
    }

    #[test]
    fn binding_is_only_valid_while_the_adapter_is_up_with_a_rov_subnet_address() {
        let snapshot = [
            info("en7", LinkKind::Wired, &[[192, 168, 1, 103]]),
            info("en0", LinkKind::Wireless, &[[10, 0, 0, 2]]),
        ];
        assert!(snapshot_has_rov_subnet_address(&snapshot, "en7", ROV));
        assert!(!snapshot_has_rov_subnet_address(&snapshot, "en0", ROV));
        // An unplugged adapter is not in the snapshot even if it kept its address.
        assert!(!snapshot_has_rov_subnet_address(&snapshot, "en9", ROV));
        assert!(!snapshot_has_rov_subnet_address(&[], "en7", ROV));
    }

    #[test]
    fn loopback_rtsp_sources_do_not_depend_on_the_rov_link() {
        for url in [
            "rtsp://admin:admin@127.0.0.1:8554/stream",
            "rtsp://localhost/stream",
            "rtsp://LOCALHOST:8554/s",
            "rtsp://[::1]:8554/s",
            "rtsp://0.0.0.0:8554/s",
        ] {
            assert!(rtsp_url_is_independent_of_rov_link(url), "{url}");
        }
        for url in [
            "rtsp://192.168.1.88:8554/s",
            "rtsp://rov.local/s",
            "not a url",
            "",
        ] {
            assert!(!rtsp_url_is_independent_of_rov_link(url), "{url}");
        }
    }

    #[test]
    fn watcher_acts_one_tick_after_a_change_and_only_once() {
        let a = signature_from(&[v4_iface("wlan0", [10, 0, 0, 2])]);
        let b = signature_from(&[
            v4_iface("wlan0", [10, 0, 0, 2]),
            v4_iface("eth1", [192, 168, 1, 103]),
        ]);
        let mut w = LinkWatcher::default();
        w.reset(a.clone());
        assert!(!w.tick(a.clone(), true), "nothing changed");
        assert!(!w.tick(b.clone(), true), "change seen, let it settle");
        assert!(w.tick(b.clone(), true), "settled: detect now");
        assert!(!w.tick(b, true), "and only once");
    }

    #[test]
    fn watcher_waits_out_a_network_that_keeps_changing() {
        let sig = |n: u8| signature_from(&[v4_iface("eth1", [169, 254, 1, n])]);
        let mut w = LinkWatcher::default();
        w.reset(sig(1));
        assert!(!w.tick(sig(2), true));
        assert!(!w.tick(sig(3), true), "still changing");
        assert!(w.tick(sig(3), true));
    }

    #[test]
    fn watcher_retries_periodically_only_while_the_rov_is_missing() {
        let sig = signature_from(&[v4_iface("wlan0", [172, 20, 10, 4])]);
        let mut w = LinkWatcher::default();
        w.reset(sig.clone());
        let due: Vec<bool> = (0..RETRY_EVERY_TICKS * 2)
            .map(|_| w.tick(sig.clone(), false))
            .collect();
        assert_eq!(due.iter().filter(|d| **d).count(), 2);
        assert!(due[(RETRY_EVERY_TICKS - 1) as usize]);

        w.reset(sig.clone());
        assert!((0..RETRY_EVERY_TICKS * 2).all(|_| !w.tick(sig.clone(), true)));
    }

    #[test]
    fn signature_changes_when_an_address_appears_and_ignores_order() {
        let a = vec![
            v4_iface("eth1", [192, 168, 1, 103]),
            v4_iface("wlan0", [10, 0, 0, 2]),
        ];
        let mut reordered = a.clone();
        reordered.reverse();
        assert_eq!(signature_from(&a), signature_from(&reordered));

        let mut plugged = a.clone();
        plugged.push(v4_iface("eth2", [192, 168, 1, 104]));
        assert_ne!(signature_from(&a), signature_from(&plugged));
        assert_eq!(
            signature_from(&[loopback_iface(), v6_iface("x")]),
            [] as [(std::string::String, std::net::Ipv4Addr, std::net::Ipv4Addr); 0]
        );
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
