//! Read-only Linux hardware inventory.
//!
//! Everything here reads sysfs (and opens device nodes read-only) so the port
//! can be validated before any hardware write exists. There are deliberately
//! no write helpers in this module.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FUSION2_VID: &str = "048d";
pub const FUSION2_PID: &str = "5711";
pub const HYDROSHIFT_VID: &str = "1cbe";
pub const HYDROSHIFT_PID: &str = "a021";
pub const CORSAIR_VID: &str = "1b1c";
pub const CORSAIR_DRAM_PID: &str = "0901";

const SMBUS_PCI: &str = "0000:00:14.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbEndpoint {
    pub address: u8,
    /// `Bulk`, `Interrupt`, `Isochronous` or `Control`.
    pub kind: String,
    pub max_packet: u16,
    pub interval: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbInterface {
    pub number: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub driver: Option<String>,
    pub endpoints: Vec<UsbEndpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsbDevice {
    pub vid: String,
    pub pid: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub bus: u8,
    pub dev: u8,
    pub sysfs: String,
    pub interfaces: Vec<UsbInterface>,
}

impl UsbDevice {
    pub fn id(&self) -> String {
        format!("{}:{}", self.vid, self.pid)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HidRawDevice {
    pub node: String,
    pub name: String,
    pub hid_id: String,
    pub phys: String,
    pub uniq: String,
    pub vid: Option<String>,
    pub pid: Option<String>,
    pub readable: bool,
    pub open_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct I2cAdapter {
    pub number: u32,
    pub name: String,
    pub path: String,
    pub readable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmbusStatus {
    pub pci_address: String,
    pub pci_id: Option<String>,
    pub driver: Option<String>,
    pub module_loaded: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwmonSensor {
    pub kind: String,
    pub label: String,
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwmonDevice {
    pub name: String,
    pub path: String,
    pub sensors: Vec<HwmonSensor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmiInfo {
    pub board_vendor: Option<String>,
    pub board_name: Option<String>,
    pub product_name: Option<String>,
    pub bios_version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Targets {
    pub fusion2: Option<UsbDevice>,
    pub hydroship: Option<UsbDevice>,
    pub corsair_usb: Option<UsbDevice>,
    pub it87: Option<HwmonDevice>,
    pub smbus_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub dmi: DmiInfo,
    pub usb: Vec<UsbDevice>,
    pub hidraw: Vec<HidRawDevice>,
    pub i2c: Vec<I2cAdapter>,
    pub smbus: SmbusStatus,
    pub hwmon: Vec<HwmonDevice>,
    pub targets: Targets,
}

impl Inventory {
    pub fn collect() -> Self {
        let usb = usb_devices();
        let hidraw = hidraw_devices();
        let i2c = i2c_adapters();
        let smbus = smbus_status();
        let hwmon = hwmon_devices();

        let find = |vid: &str, pid: &str| {
            usb.iter()
                .find(|d| d.vid.eq_ignore_ascii_case(vid) && d.pid.eq_ignore_ascii_case(pid))
                .cloned()
        };

        let targets = Targets {
            fusion2: find(FUSION2_VID, FUSION2_PID),
            hydroship: find(HYDROSHIFT_VID, HYDROSHIFT_PID),
            corsair_usb: find(CORSAIR_VID, CORSAIR_DRAM_PID),
            it87: hwmon.iter().find(|h| h.name == "it87").cloned(),
            smbus_ready: smbus.driver.is_some(),
        };

        Self {
            dmi: dmi_info(),
            usb,
            hidraw,
            i2c,
            smbus,
            hwmon,
            targets,
        }
    }
}

// ---------------------------------------------------------------------------
// sysfs helpers
// ---------------------------------------------------------------------------

fn read_trim(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_u8_dec(path: impl AsRef<Path>) -> Option<u8> {
    read_trim(path)?.parse().ok()
}

fn read_hex_u8(path: impl AsRef<Path>) -> Option<u8> {
    u8::from_str_radix(&read_trim(path)?, 16).ok()
}

fn read_hex_u16(path: impl AsRef<Path>) -> Option<u16> {
    u16::from_str_radix(&read_trim(path)?, 16).ok()
}

fn link_name(path: impl AsRef<Path>) -> Option<String> {
    fs::read_link(path).ok().and_then(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
    })
}

fn open_error(path: &str) -> Option<String> {
    match fs::File::open(path) {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// USB / HID
// ---------------------------------------------------------------------------

pub fn usb_devices() -> Vec<UsbDevice> {
    let root = Path::new("/sys/bus/usb/devices");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut devices: Vec<UsbDevice> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let vid = read_trim(path.join("idVendor"))?;
            let pid = read_trim(path.join("idProduct"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            Some(UsbDevice {
                vid,
                pid,
                manufacturer: read_trim(path.join("manufacturer")),
                product: read_trim(path.join("product")),
                serial: read_trim(path.join("serial")),
                bus: read_u8_dec(path.join("busnum")).unwrap_or(0),
                dev: read_u8_dec(path.join("devnum")).unwrap_or(0),
                sysfs: path.to_string_lossy().into_owned(),
                interfaces: usb_interfaces(root, &name),
            })
        })
        .collect();

    devices.sort_by_key(|device| (device.bus, device.dev));
    devices
}

fn usb_interfaces(root: &Path, device: &str) -> Vec<UsbInterface> {
    let prefix = format!("{device}:");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut interfaces: Vec<UsbInterface> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) {
                return None;
            }
            let path = entry.path();
            Some(UsbInterface {
                number: read_hex_u8(path.join("bInterfaceNumber")).unwrap_or(0),
                class: read_hex_u8(path.join("bInterfaceClass")).unwrap_or(0),
                subclass: read_hex_u8(path.join("bInterfaceSubClass")).unwrap_or(0),
                protocol: read_hex_u8(path.join("bInterfaceProtocol")).unwrap_or(0),
                driver: link_name(path.join("driver")),
                endpoints: usb_endpoints(&path),
            })
        })
        .collect();
    interfaces.sort_by_key(|i| i.number);
    interfaces
}

fn usb_endpoints(interface: &Path) -> Vec<UsbEndpoint> {
    let Ok(entries) = fs::read_dir(interface) else {
        return Vec::new();
    };
    let mut endpoints: Vec<UsbEndpoint> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("ep_") {
                return None;
            }
            let path = entry.path();
            Some(UsbEndpoint {
                address: read_hex_u8(path.join("bEndpointAddress")).unwrap_or(0),
                kind: read_trim(path.join("type")).unwrap_or_default(),
                max_packet: read_hex_u16(path.join("wMaxPacketSize")).unwrap_or(0),
                interval: read_hex_u8(path.join("bInterval")).unwrap_or(0),
            })
        })
        .collect();
    endpoints.sort_by_key(|e| e.address);
    endpoints
}

pub fn hidraw_devices() -> Vec<HidRawDevice> {
    let root = Path::new("/sys/class/hidraw");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut devices: Vec<HidRawDevice> = entries
        .flatten()
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let node = format!("/dev/{name}");
            let uevent = read_trim(path.join("device/uevent")).unwrap_or_default();
            let map = parse_uevent(&uevent);
            let (vid, pid) = parse_hid_id(map.get("HID_ID"));
            let error = open_error(&node);
            HidRawDevice {
                node,
                name: map.get("HID_NAME").cloned().unwrap_or_default(),
                hid_id: map.get("HID_ID").cloned().unwrap_or_default(),
                phys: map.get("HID_PHYS").cloned().unwrap_or_default(),
                uniq: map.get("HID_UNIQ").cloned().unwrap_or_default(),
                vid,
                pid,
                readable: error.is_none(),
                open_error: error,
            }
        })
        .collect();

    devices.sort_by(|a, b| a.node.cmp(&b.node));
    devices
}

fn parse_uevent(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn parse_hid_id(text: Option<&String>) -> (Option<String>, Option<String>) {
    let Some(text) = text else {
        return (None, None);
    };
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() < 3 {
        return (None, None);
    }
    let parse = |s: &str| {
        u16::from_str_radix(s, 16)
            .ok()
            .map(|v| format!("{v:04x}"))
    };
    (parse(parts[1]), parse(parts[2]))
}

// ---------------------------------------------------------------------------
// I2C / SMBus
// ---------------------------------------------------------------------------

pub fn i2c_adapters() -> Vec<I2cAdapter> {
    let root = Path::new("/sys/class/i2c-dev");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut adapters: Vec<I2cAdapter> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.strip_prefix("i2c-")?.parse().ok()?;
            let path = format!("/dev/{name}");
            Some(I2cAdapter {
                number,
                name: read_trim(entry.path().join("name")).unwrap_or_default(),
                readable: open_error(&path).is_none(),
                path,
            })
        })
        .collect();

    adapters.sort_by_key(|a| a.number);
    adapters
}

pub fn smbus_status() -> SmbusStatus {
    let pci = PathBuf::from("/sys/bus/pci/devices").join(SMBUS_PCI);
    let driver = link_name(pci.join("driver"));
    let module_loaded = Path::new("/sys/module/i2c_piix4").exists();
    let pci_id = read_trim(pci.join("uevent"))
        .and_then(|uevent| parse_uevent(&uevent).get("PCI_ID").cloned());

    let detail = match (&driver, module_loaded) {
        (Some(driver), _) => format!("SMBus controller is bound to {driver}."),
        (None, true) => "i2c_piix4 is loaded but the SMBus controller has no driver bound. \
                         RAM RGB cannot be reached until this is fixed (check dmesg for an \
                         ACPI/opregion conflict)."
            .to_string(),
        (None, false) => "i2c_piix4 is not loaded. Load it (modprobe i2c-piix4) before attempting SMBus work."
            .to_string(),
    };

    SmbusStatus {
        pci_address: SMBUS_PCI.to_string(),
        pci_id,
        driver,
        module_loaded,
        detail,
    }
}

// ---------------------------------------------------------------------------
// hwmon / DMI
// ---------------------------------------------------------------------------

pub fn hwmon_devices() -> Vec<HwmonDevice> {
    let root = Path::new("/sys/class/hwmon");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut devices: Vec<HwmonDevice> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = read_trim(path.join("name"))?;
            Some(HwmonDevice {
                name,
                path: path.to_string_lossy().into_owned(),
                sensors: hwmon_sensors(&path),
            })
        })
        .collect();

    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices
}

fn hwmon_sensors(path: &Path) -> Vec<HwmonSensor> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };

    let mut sensors: Vec<HwmonSensor> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let (kind, unit, scale) = classify_hwmon(&file)?;
            let raw: f64 = read_trim(entry.path())?.parse().ok()?;
            let label = file
                .replace("_input", "_label");
            let label = read_trim(path.join(&label)).unwrap_or_else(|| {
                file.trim_end_matches("_input").to_string()
            });
            Some(HwmonSensor {
                kind: kind.to_string(),
                label,
                value: raw * scale,
                unit: unit.to_string(),
            })
        })
        .collect();

    sensors.sort_by(|a, b| (&a.kind, &a.label).cmp(&(&b.kind, &b.label)));
    sensors
}

fn classify_hwmon(file: &str) -> Option<(&'static str, &'static str, f64)> {
    if !file.ends_with("_input") {
        return None;
    }
    if file.starts_with("temp") {
        Some(("temperature", "°C", 0.001))
    } else if file.starts_with("fan") {
        Some(("fan", "RPM", 1.0))
    } else if file.starts_with("in") {
        Some(("voltage", "V", 0.001))
    } else if file.starts_with("power") {
        Some(("power", "W", 0.000_001))
    } else if file.starts_with("curr") {
        Some(("current", "A", 0.001))
    } else if file.starts_with("freq") {
        Some(("frequency", "MHz", 0.000_001))
    } else {
        None
    }
}

pub fn dmi_info() -> DmiInfo {
    let root = Path::new("/sys/class/dmi/id");
    DmiInfo {
        board_vendor: read_trim(root.join("board_vendor")),
        board_name: read_trim(root.join("board_name")),
        product_name: read_trim(root.join("product_name")),
        bios_version: read_trim(root.join("bios_version")),
    }
}

// ---------------------------------------------------------------------------
// tests (pure parsing, no hardware access)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uevent_parses_key_values() {
        let map = parse_uevent("DRIVER=hid-generic\nHID_NAME=GIGABYTE Device\nHID_ID=0003:0000048D:00005711\n");
        assert_eq!(map.get("HID_NAME").unwrap(), "GIGABYTE Device");
        assert_eq!(map.get("HID_ID").unwrap(), "0003:0000048D:00005711");
    }

    #[test]
    fn hid_id_splits_vid_and_pid() {
        let text = "0003:0000048D:00005711".to_string();
        assert_eq!(
            parse_hid_id(Some(&text)),
            (Some("048d".to_string()), Some("5711".to_string()))
        );
        assert_eq!(parse_hid_id(None), (None, None));
    }

    #[test]
    fn hwmon_classification_covers_the_used_sensors() {
        assert_eq!(classify_hwmon("temp1_input"), Some(("temperature", "°C", 0.001)));
        assert_eq!(classify_hwmon("fan1_input"), Some(("fan", "RPM", 1.0)));
        assert_eq!(classify_hwmon("power1_input"), Some(("power", "W", 0.000_001)));
        assert_eq!(classify_hwmon("temp1_max"), None);
    }
}
