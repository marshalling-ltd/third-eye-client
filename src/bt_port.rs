//! Naming of Bluetooth SPP devices as pseudo serial ports.
//!
//! On Linux (`BlueZ`) and macOS (`IOBluetooth`) the app opens the RFCOMM
//! channel itself instead of going through a `/dev` node. Such devices are
//! offered in the serial-port list as `bt://AA:BB:CC:DD:EE:FF <Name>`; the name
//! is for display only and just the address is used to connect.

/// Prefix that marks a port name as a Bluetooth device rather than a tty.
pub const PORT_PREFIX: &str = "bt://";

/// Normalizes a Bluetooth address to upper-case, colon-separated form.
/// Accepts `:` or `-` separators in any case.
///
/// # Returns
///
/// * `Option<String>` - The normalized address, or `None` if `text` is not
///   six hex byte pairs.
///
/// # Examples
///
/// ```
/// use third_eye_client::bt_port::normalize_address;
///
/// assert_eq!(
///     normalize_address("fc-e8-15-26-9f-7d").as_deref(),
///     Some("FC:E8:15:26:9F:7D")
/// );
/// assert_eq!(normalize_address("nonsense"), None);
/// ```
#[must_use]
pub fn normalize_address(text: &str) -> Option<String> {
    let parts: Vec<&str> = text.trim().split([':', '-']).collect();
    if parts.len() != 6
        || parts
            .iter()
            .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(parts.join(":").to_ascii_uppercase())
}

/// Builds the pseudo-port name for a device: the address, then the device name
/// so the port list is readable.
///
/// # Examples
///
/// ```
/// use third_eye_client::bt_port::port_name;
///
/// assert_eq!(
///     port_name("AA:BB:CC:DD:EE:FF", "GPS Puck"),
///     "bt://AA:BB:CC:DD:EE:FF GPS Puck"
/// );
/// ```
#[must_use]
pub fn port_name(address: &str, name: &str) -> String {
    format!("{PORT_PREFIX}{address} {name}")
}

/// Extracts the normalized Bluetooth address from a `bt://` pseudo-port name;
/// anything after the address (the device name) is ignored.
///
/// # Returns
///
/// * `Option<String>` - The address, or `None` if `port` is not a well-formed
///   `bt://` name.
///
/// # Examples
///
/// ```
/// use third_eye_client::bt_port::parse_port_name;
///
/// assert_eq!(
///     parse_port_name("bt://AA:BB:CC:DD:EE:FF GPS Puck").as_deref(),
///     Some("AA:BB:CC:DD:EE:FF")
/// );
/// assert_eq!(parse_port_name("/dev/rfcomm0"), None);
/// ```
#[must_use]
pub fn parse_port_name(port: &str) -> Option<String> {
    let rest = port.trim().strip_prefix(PORT_PREFIX)?;
    normalize_address(rest.split_whitespace().next()?)
}

/// Extracts the device name from a `bt://` pseudo-port name (the text after
/// the address), if there is one.
///
/// # Examples
///
/// ```
/// use third_eye_client::bt_port::device_name;
///
/// assert_eq!(device_name("bt://AA:BB:CC:DD:EE:FF GPS Puck"), Some("GPS Puck"));
/// assert_eq!(device_name("bt://AA:BB:CC:DD:EE:FF"), None);
/// ```
#[must_use]
pub fn device_name(port: &str) -> Option<&str> {
    let rest = port.trim().strip_prefix(PORT_PREFIX)?;
    let (_, name) = rest.split_once(char::is_whitespace)?;
    let name = name.trim();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_name_round_trips() {
        let port = port_name("AA:BB:CC:DD:EE:FF", "GPS Puck");
        assert_eq!(parse_port_name(&port).as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(
            parse_port_name("bt://AA:BB:CC:DD:EE:FF").as_deref(),
            Some("AA:BB:CC:DD:EE:FF")
        );
    }

    #[test]
    fn parse_port_name_rejects_other_ports_and_bad_addresses() {
        assert_eq!(parse_port_name("/dev/rfcomm0"), None);
        assert_eq!(parse_port_name("bt://"), None);
        assert_eq!(parse_port_name("bt://AA:BB:CC"), None);
        assert_eq!(parse_port_name("bt://GG:BB:CC:DD:EE:FF"), None);
        assert_eq!(parse_port_name("COM5"), None);
    }

    #[test]
    fn device_name_returns_text_after_address() {
        assert_eq!(
            device_name("bt://AA:BB:CC:DD:EE:FF  My GPS "),
            Some("My GPS")
        );
        assert_eq!(device_name("bt://AA:BB:CC:DD:EE:FF"), None);
        assert_eq!(device_name("/dev/cu.X"), None);
    }

    #[test]
    fn normalize_address_accepts_both_separators() {
        assert_eq!(
            normalize_address(" fc-e8-15-26-9f-7d ").as_deref(),
            Some("FC:E8:15:26:9F:7D")
        );
        assert_eq!(
            normalize_address("fc:e8:15:26:9f:7d").as_deref(),
            Some("FC:E8:15:26:9F:7D")
        );
        assert_eq!(normalize_address("fc:e8:15:26:9f"), None);
    }
}
