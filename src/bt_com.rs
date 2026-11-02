//! Telling Windows' Bluetooth serial (COM) ports apart.
//!
//! Pairing a Bluetooth device makes Windows create two serial ports, both named
//! "Standard Serial over Bluetooth link (COMn)": an *outgoing* port that dials
//! the device when opened, and an *incoming* port that only waits for the
//! device to connect. Only the outgoing one is useful for reading a GPS, and the
//! incoming one often has the higher number.
//!
//! The ports can be told apart by their device instance ids:
//!
//! ```text
//! BTHENUM\{00001101-…}_LOCALMFG&005D\7&3611D67D&0&FCE815269F7D_C00000000   outgoing, device FC:E8:15:26:9F:7D
//! BTHENUM\{00001101-…}_LOCALMFG&0000\7&3611D67D&0&000000000000_00000002    incoming, no device
//! BTHENUM\DEV_FCE815269F7D\7&1C25B880&0&BLUETOOTHDEVICE_FCE815269F7D        the device itself, named "BY80125B39"
//! ```
//!
//! This module only parses those strings; reading them from Windows is done in
//! the Windows-only `winports` module.

use std::collections::HashMap;

/// The whole description of an incoming port, so callers can leave it out of
/// automatic selection. Other descriptions are `"<device name> (Bluetooth)"`, so
/// a device name cannot make one equal to this.
pub const INCOMING_DESCRIPTION: &str = "Bluetooth (incoming, not for GPS)";

/// Extracts the port name from a friendly name such as
/// `"Standard Serial over Bluetooth link (COM6)"`.
///
/// # Examples
///
/// ```
/// use third_eye_client::bt_com::com_port_name;
///
/// assert_eq!(
///     com_port_name("Standard Serial over Bluetooth link (COM6)").as_deref(),
///     Some("COM6")
/// );
/// assert_eq!(com_port_name("BY80125B39"), None);
/// ```
#[must_use]
pub fn com_port_name(friendly_name: &str) -> Option<String> {
    let open = friendly_name.rfind("(COM")?;
    let rest = &friendly_name[open + 1..];
    let close = rest.find(')')?;
    let name = &rest[..close];
    let digits = name.strip_prefix("COM")?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| name.to_string())
}

fn hex_address(text: &str) -> Option<&str> {
    (text.len() == 12 && text.bytes().all(|b| b.is_ascii_hexdigit())).then_some(text)
}

fn colon_address(hex: &str) -> String {
    let mut out = String::with_capacity(hex.len() + hex.len() / 2);
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        if i > 0 {
            out.push(':');
        }
        out.extend(pair.iter().map(|b| char::from(b.to_ascii_uppercase())));
    }
    out
}

/// For the instance id of a Bluetooth *device* (`BTHENUM\DEV_<address>\…`),
/// returns its address without separators, upper-case.
#[must_use]
pub fn device_address(instance_id: &str) -> Option<String> {
    let rest = instance_id.to_ascii_uppercase();
    let rest = rest.strip_prefix("BTHENUM\\DEV_")?;
    let address = rest.split('\\').next()?;
    hex_address(address).map(str::to_string)
}

/// For the instance id of a Bluetooth *serial port*
/// (`BTHENUM\{00001101-…}_…\…&<address>_<suffix>`), returns the address of the
/// device it belongs to, upper-case and without separators. An all-zero address
/// means the port is not tied to a device (an incoming port).
#[must_use]
pub fn serial_link_address(instance_id: &str) -> Option<String> {
    let upper = instance_id.to_ascii_uppercase();
    if !upper.starts_with("BTHENUM\\{00001101-") {
        return None;
    }
    let last = upper.rsplit('\\').next()?;
    let tail = last.rsplit('&').next()?;
    let (address, _suffix) = tail.split_once('_')?;
    hex_address(address).map(str::to_string)
}

/// Builds the dropdown descriptions for Windows' Bluetooth serial ports.
///
/// # Arguments
///
/// * `entries` - `(device instance id, friendly name)` pairs of the `BTHENUM`
///   devices Windows lists.
///
/// # Returns
///
/// * `HashMap<String, String>` - Description per port name (`"COM6"`):
///   `"<device name> (Bluetooth)"` for an outgoing port, and
///   `"Bluetooth (incoming, not for GPS)"` for an incoming one.
///
/// # Examples
///
/// ```
/// use third_eye_client::bt_com::describe_bluetooth_ports;
///
/// let entries = vec![
///     (
///         "BTHENUM\\{00001101-0000-1000-8000-00805F9B34FB}_LOCALMFG&005D\\7&3611D67D&0&FCE815269F7D_C00000000".to_string(),
///         "Standard Serial over Bluetooth link (COM6)".to_string(),
///     ),
///     (
///         "BTHENUM\\DEV_FCE815269F7D\\7&1C25B880&0&BLUETOOTHDEVICE_FCE815269F7D".to_string(),
///         "BY80125B39".to_string(),
///     ),
/// ];
/// let descriptions = describe_bluetooth_ports(&entries);
/// assert_eq!(descriptions["COM6"], "BY80125B39 (Bluetooth)");
/// ```
#[must_use]
pub fn describe_bluetooth_ports(entries: &[(String, String)]) -> HashMap<String, String> {
    let names: HashMap<String, &str> = entries
        .iter()
        .filter_map(|(id, name)| device_address(id).map(|address| (address, name.trim())))
        .collect();

    entries
        .iter()
        .filter_map(|(id, friendly)| {
            let port = com_port_name(friendly)?;
            let address = serial_link_address(id)?;
            let description = if address.bytes().all(|b| b == b'0') {
                INCOMING_DESCRIPTION.to_string()
            } else {
                match names.get(&address).filter(|name| !name.is_empty()) {
                    Some(name) => format!("{name} (Bluetooth)"),
                    None => format!("Bluetooth device {}", colon_address(&address)),
                }
            };
            Some((crate::nmea::canonical_serial_port_name(&port), description))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTGOING: &str = "BTHENUM\\{00001101-0000-1000-8000-00805F9B34FB}_LOCALMFG&005D\\7&3611D67D&0&FCE815269F7D_C00000000";
    const INCOMING: &str = "BTHENUM\\{00001101-0000-1000-8000-00805F9B34FB}_LOCALMFG&0000\\7&3611D67D&0&000000000000_00000002";
    const DEVICE: &str = "BTHENUM\\DEV_FCE815269F7D\\7&1C25B880&0&BLUETOOTHDEVICE_FCE815269F7D";
    const LINK_NAME: &str = "Standard Serial over Bluetooth link";

    fn entries() -> Vec<(String, String)> {
        vec![
            (INCOMING.to_string(), format!("{LINK_NAME} (COM7)")),
            (OUTGOING.to_string(), format!("{LINK_NAME} (COM6)")),
            (DEVICE.to_string(), "BY80125B39".to_string()),
            (
                "USB\\VID_19D1&PID_0001&MI_06\\6&17910EBA&1&0006".to_string(),
                "USB Serial Device (COM4)".to_string(),
            ),
        ]
    }

    #[test]
    fn com_port_name_reads_the_port_from_the_friendly_name() {
        assert_eq!(
            com_port_name("USB Serial Device (COM10)").as_deref(),
            Some("COM10")
        );
        assert_eq!(com_port_name("Thing (LPT1)"), None);
        assert_eq!(com_port_name("Thing (COM)"), None);
        assert_eq!(com_port_name("no port here"), None);
    }

    #[test]
    fn addresses_are_read_from_instance_ids() {
        assert_eq!(device_address(DEVICE).as_deref(), Some("FCE815269F7D"));
        assert_eq!(device_address(OUTGOING), None);
        assert_eq!(
            serial_link_address(OUTGOING).as_deref(),
            Some("FCE815269F7D")
        );
        assert_eq!(
            serial_link_address(INCOMING).as_deref(),
            Some("000000000000")
        );
        assert_eq!(serial_link_address(DEVICE), None);
        assert_eq!(
            serial_link_address("USB\\VID_19D1&PID_0001&MI_06\\6&17910EBA&1&0006"),
            None
        );
    }

    #[test]
    fn outgoing_ports_are_named_after_the_device_and_incoming_ones_are_marked() {
        let descriptions = describe_bluetooth_ports(&entries());
        let key = |port: &str| crate::nmea::canonical_serial_port_name(port);
        assert_eq!(descriptions[&key("COM6")], "BY80125B39 (Bluetooth)");
        assert_eq!(descriptions[&key("COM7")], INCOMING_DESCRIPTION);
        // USB serial ports are not Bluetooth links and are left to other code.
        assert!(!descriptions.contains_key(&key("COM4")));
    }

    #[test]
    fn an_unknown_device_falls_back_to_its_address() {
        let only_link = vec![(OUTGOING.to_string(), format!("{LINK_NAME} (COM6)"))];
        let descriptions = describe_bluetooth_ports(&only_link);
        let key = crate::nmea::canonical_serial_port_name("COM6");
        assert_eq!(descriptions[&key], "Bluetooth device FC:E8:15:26:9F:7D");
    }

    #[test]
    fn instance_ids_are_matched_case_insensitively() {
        assert_eq!(
            serial_link_address(&OUTGOING.to_ascii_lowercase()).as_deref(),
            Some("FCE815269F7D")
        );
        assert_eq!(
            device_address(&DEVICE.to_ascii_lowercase()).as_deref(),
            Some("FCE815269F7D")
        );
    }
}
