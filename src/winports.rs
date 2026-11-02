//! Reads Windows' Bluetooth device entries, for naming Bluetooth COM ports.
//!
//! See [`crate::bt_com`] for what the entries look like and how they are used.

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_ALLCLASSES, DIGCF_PRESENT, HDEVINFO, SETUP_DI_REGISTRY_PROPERTY, SP_DEVINFO_DATA,
    SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
    SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
};
use windows::core::w;

fn utf16_to_string(units: &[u16]) -> String {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn instance_id(set: HDEVINFO, data: &SP_DEVINFO_DATA) -> Option<String> {
    let mut buf = [0u16; 512];
    unsafe { SetupDiGetDeviceInstanceIdW(set, data, Some(&mut buf), None) }.ok()?;
    Some(utf16_to_string(&buf))
}

fn string_property(
    set: HDEVINFO,
    data: &SP_DEVINFO_DATA,
    property: SETUP_DI_REGISTRY_PROPERTY,
) -> Option<String> {
    let mut buf = [0u8; 1024];
    unsafe { SetupDiGetDeviceRegistryPropertyW(set, data, property, None, Some(&mut buf), None) }
        .ok()?;
    let units: Vec<u16> = buf
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let text = utf16_to_string(&units);
    (!text.trim().is_empty()).then_some(text)
}

/// Lists the `BTHENUM` devices Windows currently has (the paired Bluetooth
/// devices and their serial ports) as `(device instance id, friendly name)` pairs.
///
/// # Returns
///
/// * `Vec<(String, String)>` - Empty if the entries cannot be read.
#[must_use]
pub fn bluetooth_pnp_entries() -> Vec<(String, String)> {
    let Ok(set) = (unsafe {
        SetupDiGetClassDevsW(None, w!("BTHENUM"), None, DIGCF_PRESENT | DIGCF_ALLCLASSES)
    }) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    let mut index = 0;
    loop {
        let mut data = SP_DEVINFO_DATA {
            cbSize: u32::try_from(size_of::<SP_DEVINFO_DATA>()).unwrap_or(0),
            ..Default::default()
        };
        if unsafe { SetupDiEnumDeviceInfo(set, index, &raw mut data) }.is_err() {
            break;
        }
        index += 1;
        let name = string_property(set, &data, SPDRP_FRIENDLYNAME)
            .or_else(|| string_property(set, &data, SPDRP_DEVICEDESC));
        if let (Some(id), Some(name)) = (instance_id(set, &data), name) {
            entries.push((id, name));
        }
    }
    let _ = unsafe { SetupDiDestroyDeviceInfoList(set) };
    entries
}
