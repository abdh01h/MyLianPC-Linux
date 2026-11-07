//! SMBIOS type-17 memory information.
//!
//! Windows reads this through WMI (`Win32_PhysicalMemory`). On Linux the raw
//! SMBIOS table is available under `/sys/firmware/dmi/entries/17-*/raw` to
//! root, which is how the service runs. Fields used here are defined by the
//! SMBIOS 3.x specification (Memory Device, type 17).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

const TYPE_MEMORY_DEVICE: u8 = 0x11;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryModuleInfo {
    pub size_gb: f64,
    pub memory_type: String,
    pub rated_mtps: u32,
    pub configured_mtps: u32,
    pub manufacturer: Option<String>,
    pub part_number: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub modules: Vec<MemoryModuleInfo>,
    /// Configured speed reported by the modules (MT/s).
    pub configured_mtps: u32,
    /// Rated (XMP/EXPO) speed reported by the modules (MT/s).
    pub rated_mtps: u32,
    /// True when the configured speed matches the rated profile speed.
    pub xmp_expo_active: bool,
    pub summary: String,
}

impl MemoryInfo {
    pub fn read() -> Self {
        Self::read_from(Path::new("/sys/firmware/dmi/entries"))
    }

    pub fn read_from(entries: &Path) -> Self {
        let Ok(read_dir) = fs::read_dir(entries) else {
            return Self::default();
        };
        let mut modules = Vec::new();
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("17-") {
                continue;
            }
            let Ok(raw) = fs::read(entry.path().join("raw")) else {
                continue;
            };
            if let Some(module) = parse_memory_device(&raw) {
                modules.push(module);
            }
        }
        let mut info = Self {
            modules,
            ..Default::default()
        };
        info.finish();
        info
    }

    fn finish(&mut self) {
        if self.modules.is_empty() {
            return;
        }
        let count = self.modules.len();
        let total_gb: f64 = self.modules.iter().map(|module| module.size_gb).sum();
        self.configured_mtps = self
            .modules
            .iter()
            .map(|module| module.configured_mtps)
            .max()
            .unwrap_or(0);
        self.rated_mtps = self
            .modules
            .iter()
            .map(|module| module.rated_mtps)
            .max()
            .unwrap_or(0);
        self.xmp_expo_active = self.rated_mtps > 0
            && self.configured_mtps > 0
            && self.configured_mtps >= self.rated_mtps;
        let kind = self.modules[0].memory_type.clone();
        let per_module = if count == 1 {
            format!("{total_gb:.0} GB")
        } else {
            format!("{count} × {:.0} GB", total_gb / count as f64)
        };
        let speed = if self.configured_mtps > 0 {
            format!(" · {} MT/s", self.configured_mtps)
        } else {
            String::new()
        };
        self.summary = format!("{per_module} {kind}{speed}").trim().to_string();
    }
}

/// Parses one raw SMBIOS structure. Returns `None` for empty slots, unknown
/// sizes and structures that are not memory devices.
pub fn parse_memory_device(raw: &[u8]) -> Option<MemoryModuleInfo> {
    if raw.len() < 4 || raw[0] != TYPE_MEMORY_DEVICE {
        return None;
    }
    let length = raw[1] as usize;
    if length < 0x15 || raw.len() < length {
        return None;
    }

    let size = u16::from_le_bytes([raw[0x0C], raw[0x0D]]);
    if size == 0 || size == 0xFFFF {
        return None; // empty slot or unknown
    }
    let size_mb = if size == 0x7FFF {
        if length < 0x20 {
            return None;
        }
        let extended = u32::from_le_bytes([raw[0x1C], raw[0x1D], raw[0x1E], raw[0x1F]]);
        (extended & 0x7FFF_FFFF) as f64
    } else {
        size as f64
    };

    let memory_type = memory_type_name(raw[0x12]);
    // SMBIOS type 17: 0x12 type, 0x13..0x15 type detail, 0x15 speed,
    // 0x17 manufacturer string, 0x1A part number, 0x20 configured speed.
    let rated_mtps = if length >= 0x17 {
        u16::from_le_bytes([raw[0x15], raw[0x16]]) as u32
    } else {
        0
    };
    let configured_mtps = if length >= 0x22 {
        u16::from_le_bytes([raw[0x20], raw[0x21]]) as u32
    } else {
        0
    };
    let configured_mtps = if configured_mtps == 0 {
        rated_mtps
    } else {
        configured_mtps
    };

    let strings = parse_strings(raw, length);
    let string_at = |index: u8| -> Option<String> {
        if index == 0 {
            return None;
        }
        strings
            .get(index as usize - 1)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && value != "Unknown")
    };

    Some(MemoryModuleInfo {
        size_gb: size_mb / 1024.0,
        memory_type,
        rated_mtps,
        configured_mtps,
        manufacturer: if length >= 0x18 {
            string_at(raw[0x17])
        } else {
            None
        },
        part_number: if length >= 0x1B {
            string_at(raw[0x1A])
        } else {
            None
        },
    })
}

fn parse_strings(raw: &[u8], offset: usize) -> Vec<String> {
    if offset >= raw.len() {
        return Vec::new();
    }
    let mut strings = Vec::new();
    let mut current = Vec::new();
    for byte in &raw[offset..] {
        if *byte == 0 {
            if current.is_empty() {
                break;
            }
            strings.push(String::from_utf8_lossy(&current).to_string());
            current.clear();
        } else {
            current.push(*byte);
        }
    }
    strings
}

fn memory_type_name(code: u8) -> String {
    match code {
        0x12 => "DDR".to_string(),
        0x13 => "DDR2".to_string(),
        0x18 => "DDR3".to_string(),
        0x1A => "DDR4".to_string(),
        0x22 => "DDR5".to_string(),
        0x23 => "LPDDR5".to_string(),
        other => format!("type 0x{other:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_device(size_mb: u16, rated: u16, configured: u16) -> Vec<u8> {
        let mut raw = vec![0u8; 0x28];
        raw[0] = TYPE_MEMORY_DEVICE;
        raw[1] = 0x28;
        raw[0x0C..0x0E].copy_from_slice(&size_mb.to_le_bytes());
        raw[0x12] = 0x22; // DDR5
        raw[0x15..0x17].copy_from_slice(&rated.to_le_bytes());
        raw[0x17] = 1; // manufacturer
        raw[0x1A] = 2; // part number
        raw[0x20..0x22].copy_from_slice(&configured.to_le_bytes());
        raw.extend_from_slice(b"Kingston\0KF560C36-16\0\0");
        raw
    }

    #[test]
    fn parses_a_populated_ddr5_module() {
        let module = parse_memory_device(&memory_device(16384, 6000, 6000)).expect("parses");
        assert_eq!(module.size_gb, 16.0);
        assert_eq!(module.memory_type, "DDR5");
        assert_eq!(module.rated_mtps, 6000);
        assert_eq!(module.configured_mtps, 6000);
        assert_eq!(module.manufacturer.as_deref(), Some("Kingston"));
        assert_eq!(module.part_number.as_deref(), Some("KF560C36-16"));
    }

    #[test]
    fn empty_slots_are_ignored() {
        assert!(parse_memory_device(&memory_device(0, 6000, 4800)).is_none());
    }

    #[test]
    fn xmp_is_active_only_when_the_configured_speed_matches_the_profile() {
        let mut info = MemoryInfo {
            modules: vec![MemoryModuleInfo {
                size_gb: 16.0,
                memory_type: "DDR5".to_string(),
                rated_mtps: 6000,
                configured_mtps: 4800,
                ..Default::default()
            }],
            ..Default::default()
        };
        info.finish();
        assert!(!info.xmp_expo_active);
        assert_eq!(info.summary, "16 GB DDR5 · 4800 MT/s");

        info.modules[0].configured_mtps = 6000;
        info.finish();
        assert!(info.xmp_expo_active);
    }

    #[test]
    fn old_layout_without_configured_speed_uses_the_rated_speed() {
        let mut raw = memory_device(8192, 3200, 0);
        raw[1] = 0x1C;
        raw.truncate(0x1C);
        raw.extend_from_slice(b"Micron\0MT16\0\0");
        let module = parse_memory_device(&raw).expect("parses");
        assert_eq!(module.configured_mtps, 3200);
    }
}
