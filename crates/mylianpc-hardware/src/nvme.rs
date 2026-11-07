//! NVMe drive inventory, temperature and remaining life.
//!
//! Windows reads drive health through WMI/StorageReliabilityCounter. On Linux
//! the equivalent is the NVMe SMART/health log page, fetched with the admin
//! ioctl on `/dev/nvmeX`. The service runs as root, so the ioctl is available
//! there; a permission failure simply leaves health unknown.

use std::fs;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use serde::{Deserialize, Serialize};

const NVME_IOCTL_ADMIN_CMD: libc::c_ulong = 0xC048_4E41;
const NVME_LOG_SMART: u32 = 0x02;
const NVME_NSID_ALL: u32 = 0xFFFF_FFFF;

#[repr(C)]
#[derive(Default)]
struct AdminCommand {
    opcode: u8,
    flags: u8,
    rsvd1: u16,
    nsid: u32,
    cdw2: u32,
    cdw3: u32,
    metadata: u64,
    addr: u64,
    metadata_len: u32,
    data_len: u32,
    cdw10: u32,
    cdw11: u32,
    cdw12: u32,
    cdw13: u32,
    cdw14: u32,
    cdw15: u32,
    timeout_ms: u32,
    rsvd2: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StorageDeviceInfo {
    pub name: String,
    pub temperature_celsius: Option<f64>,
    /// Remaining life in percent (100 = as new).
    pub health_percent: Option<f64>,
    /// The firmware's own verdict ("Healthy", "Warning", "Unhealthy").
    pub health_state: Option<String>,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    pub volumes: Option<String>,
}

/// Every NVMe controller with its namespace, temperature and SMART values.
pub fn storage_devices() -> Vec<StorageDeviceInfo> {
    let Ok(entries) = fs::read_dir("/sys/class/nvme") else {
        return Vec::new();
    };
    let mounts = mounts();
    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let controller = entry.path();
        let controller_name = entry.file_name().to_string_lossy().to_string();
        if !controller_name.starts_with("nvme") {
            continue;
        }
        let model = fs::read_to_string(controller.join("model"))
            .ok()
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| controller_name.clone());
        let temperature = controller_temperature(&controller);
        let (total_bytes, free_bytes, volumes) = namespace_usage(&controller, &mounts);
        let (health_percent, health_state) = smart_health(Path::new(&format!(
            "/dev/{controller_name}"
        )));
        devices.push(StorageDeviceInfo {
            name: model,
            temperature_celsius: temperature,
            health_percent,
            health_state,
            total_bytes,
            free_bytes,
            volumes,
        });
    }
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices
}

fn controller_temperature(controller: &Path) -> Option<f64> {
    // The controller's hwmon device is a direct child (`nvme0/hwmon3`); some
    // kernels use `device/hwmon`. Scan both, looking for the hwmon directory.
    let mut candidates = Vec::new();
    for root in [controller.to_path_buf(), controller.join("device")] {
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("hwmon") {
                    candidates.push(entry.path());
                }
            }
        }
        if let Ok(entries) = fs::read_dir(root.join("hwmon")) {
            for entry in entries.flatten() {
                candidates.push(entry.path());
            }
        }
    }
    for path in candidates {
        let label = fs::read_to_string(path.join("temp1_label")).unwrap_or_default();
        if label.trim() != "Composite" {
            continue;
        }
        if let Some(value) = fs::read_to_string(path.join("temp1_input"))
            .ok()
            .and_then(|text| text.trim().parse::<f64>().ok())
        {
            return Some(value / 1000.0);
        }
    }
    None
}

/// Sums the size of every namespace and the free space of its mounted
/// partitions.
fn namespace_usage(
    controller: &Path,
    mounts: &[(String, String)],
) -> (Option<u64>, Option<u64>, Option<String>) {
    let Ok(entries) = fs::read_dir(controller) else {
        return (None, None, None);
    };
    let mut total = 0u64;
    let mut free = 0u64;
    let mut have_total = false;
    let mut have_free = false;
    let mut volumes = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("nvme") || !name.contains('n') {
            continue;
        }
        let namespace = entry.path();
        if let Some(bytes) = fs::read_to_string(namespace.join("size"))
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
        {
            total = total.saturating_add(bytes.saturating_mul(512));
            have_total = true;
        }
        // Mounted partitions of this namespace, plus the namespace itself.
        let mut candidates = vec![format!("/dev/{name}")];
        if let Ok(partitions) = fs::read_dir(&namespace) {
            for partition in partitions.flatten() {
                let partition_name = partition.file_name().to_string_lossy().to_string();
                if partition_name.starts_with(&format!("{name}p")) {
                    candidates.push(format!("/dev/{partition_name}"));
                }
            }
        }
        for (source, mountpoint) in mounts {
            if candidates.iter().any(|candidate| candidate == source) {
                if let Some((blocks, available, fragment)) = statvfs(mountpoint) {
                    free = free.saturating_add(available.saturating_mul(fragment));
                    let _ = blocks;
                    have_free = true;
                    if !volumes.contains(mountpoint) {
                        volumes.push(mountpoint.clone());
                    }
                }
            }
        }
    }
    (
        have_total.then_some(total),
        have_free.then_some(free),
        (!volumes.is_empty()).then(|| volumes.join(", ")),
    )
}

fn statvfs(path: &str) -> Option<(u64, u64, u64)> {
    use std::ffi::CString;
    let path = CString::new(path).ok()?;
    // SAFETY: `stat` is zeroed and filled by the kernel; the path is valid.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut stat) != 0 {
            return None;
        }
        Some((stat.f_blocks, stat.f_bavail, stat.f_frsize))
    }
}

fn mounts() -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string("/proc/self/mounts") else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let source = fields.next()?.to_string();
            let target = fields.next()?.to_string();
            Some((source, target.replace("\\040", " ")))
        })
        .collect()
}

/// Reads the NVMe SMART/health log page. Returns (remaining life, state).
pub fn smart_health(device: &Path) -> (Option<f64>, Option<String>) {
    let Ok(file) = fs::File::open(device) else {
        return (None, None);
    };
    let mut log = [0u8; 512];
    let mut command = AdminCommand {
        opcode: 0x02, // Get Log Page
        nsid: NVME_NSID_ALL,
        addr: log.as_mut_ptr() as u64,
        data_len: log.len() as u32,
        cdw10: NVME_LOG_SMART | ((log.len() as u32 / 4 - 1) << 16),
        ..Default::default()
    };
    // SAFETY: the command points at a live 512-byte buffer.
    let result = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            NVME_IOCTL_ADMIN_CMD,
            &mut command as *mut AdminCommand,
        )
    };
    if result != 0 {
        return (None, None);
    }
    (
        Some(remaining_life(&log)),
        Some(health_state(&log)),
    )
}

/// Remaining life from the SMART log (100% - percentage used).
pub fn remaining_life(log: &[u8]) -> f64 {
    let used = log.get(5).copied().unwrap_or(0).min(100);
    (100 - used) as f64
}

/// The firmware verdict from the critical-warning byte and percentage used.
pub fn health_state(log: &[u8]) -> String {
    let critical = log.first().copied().unwrap_or(0) & 0x0F;
    let used = log.get(5).copied().unwrap_or(0);
    if critical != 0 || used >= 100 {
        "Unhealthy".to_string()
    } else if used >= 80 {
        "Warning".to_string()
    } else {
        "Healthy".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_command_layout_matches_the_kernel_header() {
        assert_eq!(std::mem::size_of::<AdminCommand>(), 72);
        assert_eq!(NVME_IOCTL_ADMIN_CMD, 0xC048_4E41);
    }

    #[test]
    fn smart_log_percentage_used_becomes_remaining_life() {
        let mut log = [0u8; 512];
        log[5] = 12;
        assert_eq!(remaining_life(&log), 88.0);
        assert_eq!(health_state(&log), "Healthy");
    }

    #[test]
    fn critical_warnings_are_unhealthy_and_high_wear_warns() {
        let mut log = [0u8; 512];
        log[0] = 0x04;
        assert_eq!(health_state(&log), "Unhealthy");
        let mut worn = [0u8; 512];
        worn[5] = 85;
        assert_eq!(health_state(&worn), "Warning");
    }
}
