//! Sensor sampling from Linux `hwmon`, `/proc`, DRM sysfs, NVML and RAPL.
//!
//! This maps the Linux sources onto the metric keys the profiles and LCD
//! themes use. Everything is best-effort: a missing source simply leaves that
//! value absent. Reading a sensor never writes anything.
//!
//! The Windows service used LibreHardwareMonitor over PawnIO; the Linux sources
//! are `k10temp`/`amdgpu`/`nvme`/`spd5118` hwmon devices, `/proc`, NVML for the
//! NVIDIA GPU, RAPL for CPU package power and the NVMe SMART log for drive life.

use std::collections::BTreeMap;
use std::fs;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use mylianpc_core::metrics;

use crate::inventory::{self, HwmonDevice};
use crate::nvme::{self, StorageDeviceInfo};
use crate::nvml::Nvml;
use crate::rapl::Rapl;
use crate::smbios::MemoryInfo;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reading {
    pub key: String,
    pub label: String,
    pub unit: String,
    pub group: String,
    pub value: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Names {
    pub cpu: Option<String>,
    pub gpu: Option<String>,
    pub memory: Option<String>,
    pub storage: Option<String>,
    pub cooler: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub time_utc: DateTime<Utc>,
    pub values: BTreeMap<String, f64>,
    pub readings: Vec<Reading>,
    pub names: Names,
    /// Every physical drive with its temperature, remaining life and capacity.
    #[serde(default)]
    pub storage: Vec<StorageDeviceInfo>,
}

impl Snapshot {
    pub fn value(&self, key: &str) -> Option<f64> {
        self.values.get(key).copied()
    }

    /// Adds or replaces one reading (used to merge the cooler telemetry).
    pub fn set_value(&mut self, key: &str, value: Option<f64>) {
        let Some(value) = value else {
            return;
        };
        self.values.insert(key.to_string(), value);
        let reading = reading(key, value);
        if let Some(existing) = self.readings.iter_mut().find(|item| item.key == key) {
            *existing = reading;
        } else {
            self.readings.push(reading);
        }
    }
}

pub struct SensorSampler {
    previous_cpu: Option<(u64, u64)>,
    rapl: Rapl,
    nvml: Nvml,
    memory: MemoryInfo,
    storage: Vec<StorageDeviceInfo>,
    storage_updated: Option<Instant>,
}

impl Default for SensorSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl SensorSampler {
    pub fn new() -> Self {
        let mut rapl = Rapl::new();
        // Prime the cumulative energy counter so the first sample already has a
        // power reading.
        let _ = rapl.sample();
        Self {
            previous_cpu: cpu_times(),
            rapl,
            nvml: Nvml::new(),
            memory: MemoryInfo::read(),
            storage: Vec::new(),
            storage_updated: None,
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        let hwmon = inventory::hwmon_devices();
        let mut values = BTreeMap::new();
        let mut readings = Vec::new();

        let cpu_temp = hwmon
            .iter()
            .find(|device| device.name == "k10temp")
            .and_then(|device| {
                sensor(device, "Tctl")
                    .or_else(|| sensor(device, "Tccd1"))
                    .or_else(|| first_of_kind(device, "temperature"))
            });
        record(&mut values, &mut readings, metrics::CPU_TEMP, cpu_temp);

        let cpu_clock = cpu_clock_mhz();
        record(&mut values, &mut readings, metrics::CPU_CLOCK, cpu_clock);

        let current_cpu = cpu_times();
        let cpu_load = current_cpu.map(|current| cpu_load_percent(self.previous_cpu, current));
        if let Some(current) = current_cpu {
            self.previous_cpu = Some(current);
        }
        record(&mut values, &mut readings, metrics::CPU_LOAD, cpu_load);

        let cpu_power = self.rapl.sample();
        record(&mut values, &mut readings, metrics::CPU_POWER, cpu_power);

        let nvidia = self.nvml.sample();
        let amdgpu = hwmon.iter().find(|device| device.name == "amdgpu");
        let (gpu_name, gpu_temp, gpu_load, gpu_clock, gpu_power, gpu_fan, vram) = match &nvidia {
            Some(gpu) => (
                Some(gpu.name.clone()),
                gpu.temperature_c,
                gpu.load_percent,
                gpu.clock_mhz,
                gpu.power_w,
                gpu.fan_rpm,
                (
                    gpu.vram_total_gb,
                    gpu.vram_used_gb,
                    gpu.vram_free_gb,
                ),
            ),
            None => {
                let temp = amdgpu.and_then(|device| {
                    sensor(device, "edge")
                        .or_else(|| sensor(device, "junction"))
                        .or_else(|| first_of_kind(device, "temperature"))
                });
                (
                    gpu_name_from_drm(),
                    temp,
                    gpu_busy_percent(),
                    amdgpu.and_then(|device| first_of_kind(device, "frequency")),
                    amdgpu.and_then(|device| first_of_kind(device, "power")),
                    amdgpu.and_then(|device| first_of_kind(device, "fan")),
                    (None, None, None),
                )
            }
        };
        record(&mut values, &mut readings, metrics::GPU_TEMP, gpu_temp);
        record(&mut values, &mut readings, metrics::GPU_LOAD, gpu_load);
        record(&mut values, &mut readings, metrics::GPU_CLOCK, gpu_clock);
        record(&mut values, &mut readings, metrics::GPU_POWER, gpu_power);
        record(&mut values, &mut readings, metrics::GPU_FAN, gpu_fan);
        record(
            &mut values,
            &mut readings,
            metrics::GPU_VRAM_TOTAL,
            vram.0.map(round1),
        );
        record(
            &mut values,
            &mut readings,
            metrics::GPU_VRAM_USED,
            vram.1.map(round1),
        );
        record(
            &mut values,
            &mut readings,
            metrics::GPU_VRAM_FREE,
            vram.2.map(round1),
        );

        let memory = meminfo();
        if let Some((used, total)) = memory {
            record(&mut values, &mut readings, metrics::RAM_USED_GB, Some(used));
            record(
                &mut values,
                &mut readings,
                metrics::RAM_TOTAL_GB,
                Some(total),
            );
            let load = if total > 0.0 { used / total * 100.0 } else { 0.0 };
            record(&mut values, &mut readings, metrics::RAM_LOAD, Some(load));
        }

        let ram_temp = average_dimms(&hwmon);
        record(&mut values, &mut readings, metrics::RAM_TEMP, ram_temp);
        if self.memory.configured_mtps > 0 {
            record(
                &mut values,
                &mut readings,
                metrics::RAM_SPEED,
                Some(self.memory.configured_mtps as f64),
            );
        }
        if !self.memory.modules.is_empty() {
            record(
                &mut values,
                &mut readings,
                metrics::RAM_XMP,
                Some(if self.memory.xmp_expo_active { 1.0 } else { 0.0 }),
            );
        }

        if self
            .storage_updated
            .map(|at| at.elapsed() >= Duration::from_secs(15))
            .unwrap_or(true)
        {
            self.storage = nvme::storage_devices();
            self.storage_updated = Some(Instant::now());
        }
        let drive_temps: Vec<f64> = self
            .storage
            .iter()
            .filter_map(|drive| drive.temperature_celsius)
            .filter(|value| *value > 0.0 && *value < 150.0)
            .collect();
        record(
            &mut values,
            &mut readings,
            metrics::STORAGE_TEMP,
            drive_temps.iter().copied().reduce(f64::max),
        );
        let drive_health: Vec<f64> = self
            .storage
            .iter()
            .filter_map(|drive| drive.health_percent)
            .collect();
        record(
            &mut values,
            &mut readings,
            metrics::STORAGE_HEALTH,
            drive_health.iter().copied().reduce(f64::min),
        );

        let memory_label = if !self.memory.summary.is_empty() {
            Some(self.memory.summary.clone())
        } else {
            memory.map(|(_, total)| format!("{total:.0} GB"))
        };
        let names = Names {
            cpu: cpu_model(),
            gpu: gpu_name,
            memory: memory_label,
            storage: storage_summary(&self.storage),
            cooler: hydroship_present().then(|| "Lian Li HydroShift II LCD-C".to_string()),
        };

        Snapshot {
            time_utc: Utc::now(),
            values,
            readings,
            names,
            storage: self.storage.clone(),
        }
    }
}

fn record(
    values: &mut BTreeMap<String, f64>,
    readings: &mut Vec<Reading>,
    key: &str,
    value: Option<f64>,
) {
    if let Some(value) = value {
        values.insert(key.to_string(), value);
        readings.push(reading(key, value));
    }
}

fn reading(key: &str, value: f64) -> Reading {
    match metrics::find(key) {
        Some(info) => Reading {
            key: key.to_string(),
            label: info.label.to_string(),
            unit: info.unit.to_string(),
            group: info.group.to_string(),
            value: Some(value),
        },
        None => Reading {
            key: key.to_string(),
            label: key.to_string(),
            unit: String::new(),
            group: String::new(),
            value: Some(value),
        },
    }
}

fn sensor(device: &HwmonDevice, label: &str) -> Option<f64> {
    device
        .sensors
        .iter()
        .find(|sensor| sensor.label.eq_ignore_ascii_case(label))
        .map(|sensor| sensor.value)
}

fn first_of_kind(device: &HwmonDevice, kind: &str) -> Option<f64> {
    device
        .sensors
        .iter()
        .find(|sensor| sensor.kind == kind)
        .map(|sensor| sensor.value)
}

fn average_dimms(hwmon: &[HwmonDevice]) -> Option<f64> {
    let temps: Vec<f64> = hwmon
        .iter()
        .filter(|device| device.name == "spd5118")
        .filter_map(|device| first_of_kind(device, "temperature"))
        .filter(|value| *value > 0.0 && *value < 150.0)
        .collect();
    if temps.is_empty() {
        None
    } else {
        Some(temps.iter().sum::<f64>() / temps.len() as f64)
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn cpu_times() -> Option<(u64, u64)> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|field| field.parse().ok())
        .collect();
    if fields.len() < 4 {
        return None;
    }
    let total: u64 = fields.iter().sum();
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

fn cpu_load_percent(previous: Option<(u64, u64)>, current: (u64, u64)) -> f64 {
    let (total, idle) = current;
    if total == 0 {
        return 0.0;
    }
    let (delta_total, delta_idle) = match previous {
        Some((previous_total, previous_idle)) => {
            if total <= previous_total {
                return 0.0;
            }
            (
                total - previous_total,
                idle.saturating_sub(previous_idle),
            )
        }
        None => (total, idle),
    };
    if delta_total == 0 {
        return 0.0;
    }
    (100.0 * (1.0 - delta_idle as f64 / delta_total as f64)).clamp(0.0, 100.0)
}

fn cpu_clock_mhz() -> Option<f64> {
    let text = fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut total = 0.0f64;
    let mut count = 0usize;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("cpu MHz") {
            if let Some(value) = rest
                .split(':')
                .nth(1)
                .and_then(|value| value.trim().parse::<f64>().ok())
            {
                total += value;
                count += 1;
            }
        }
    }
    (count > 0 && total > 0.0).then(|| total / count as f64)
}

fn cpu_model() -> Option<String> {
    let text = fs::read_to_string("/proc/cpuinfo").ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("model name"))
        .and_then(|rest| rest.split(':').nth(1))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn meminfo() -> Option<(f64, f64)> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    let mut total_kb = 0.0f64;
    let mut available_kb = 0.0f64;

    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("MemTotal:") => {
                if let Some(value) = parts.next().and_then(|value| value.parse().ok()) {
                    total_kb = value;
                }
            }
            Some("MemAvailable:") => {
                if let Some(value) = parts.next().and_then(|value| value.parse().ok()) {
                    available_kb = value;
                }
            }
            _ => {}
        }
    }

    if total_kb <= 0.0 {
        return None;
    }
    Some((
        (total_kb - available_kb) / 1024.0 / 1024.0,
        total_kb / 1024.0 / 1024.0,
    ))
}

fn gpu_busy_percent() -> Option<f64> {
    let entries = fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let path = entry.path().join("device/gpu_busy_percent");
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(value) = text.trim().parse::<f64>() {
                return Some(value);
            }
        }
    }
    None
}

fn gpu_name_from_drm() -> Option<String> {
    let entries = fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        for file in ["device/product_name", "device/name"] {
            if let Ok(text) = fs::read_to_string(entry.path().join(file)) {
                let name = text.trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn hydroship_present() -> bool {
    inventory::usb_devices().iter().any(|device| {
        device.vid.eq_ignore_ascii_case("1cbe") && device.pid.eq_ignore_ascii_case("a021")
    })
}

fn storage_summary(devices: &[StorageDeviceInfo]) -> Option<String> {
    if devices.is_empty() {
        return None;
    }
    let total: u64 = devices
        .iter()
        .filter_map(|device| device.total_bytes)
        .sum();
    let count = devices.len();
    Some(if total > 0 {
        format!(
            "{count} drive{} · {}",
            if count == 1 { "" } else { "s" },
            capacity_text(total)
        )
    } else {
        format!("{count} drive{}", if count == 1 { "" } else { "s" })
    })
}

/// Capacity the way the label on the box reads it (decimal units).
fn capacity_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 999.5 {
        format!("{:.2} TB", gb / 1000.0)
    } else if gb >= 1.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> HwmonDevice {
        HwmonDevice {
            name: "k10temp".to_string(),
            path: String::new(),
            sensors: vec![
                crate::inventory::HwmonSensor {
                    kind: "temperature".to_string(),
                    label: "Tctl".to_string(),
                    value: 42.5,
                    unit: "°C".to_string(),
                },
                crate::inventory::HwmonSensor {
                    kind: "temperature".to_string(),
                    label: "Tccd1".to_string(),
                    value: 38.0,
                    unit: "°C".to_string(),
                },
            ],
        }
    }

    #[test]
    fn finds_sensors_by_label_and_kind() {
        let device = device();
        assert_eq!(sensor(&device, "Tctl"), Some(42.5));
        assert_eq!(sensor(&device, "tctl"), Some(42.5));
        assert_eq!(first_of_kind(&device, "temperature"), Some(42.5));
        assert_eq!(sensor(&device, "missing"), None);
    }

    #[test]
    fn cpu_load_uses_the_delta_once_available() {
        assert_eq!(cpu_load_percent(None, (100, 40)), 60.0);
        assert_eq!(cpu_load_percent(Some((100, 40)), (200, 140)), 0.0);
        assert_eq!(cpu_load_percent(Some((100, 40)), (200, 60)), 80.0);
        assert_eq!(cpu_load_percent(Some((100, 40)), (100, 40)), 0.0);
    }

    #[test]
    fn readings_carry_metric_metadata() {
        let reading = reading(metrics::CPU_TEMP, 42.5);
        assert_eq!(reading.label, "CPU temperature");
        assert_eq!(reading.unit, "°C");
        assert_eq!(reading.group, "CPU");
    }

    #[test]
    fn dimm_temperatures_are_averaged() {
        let dimm = |value| HwmonDevice {
            name: "spd5118".to_string(),
            path: String::new(),
            sensors: vec![crate::inventory::HwmonSensor {
                kind: "temperature".to_string(),
                label: String::new(),
                value,
                unit: "°C".to_string(),
            }],
        };
        assert_eq!(average_dimms(&[dimm(30.0), dimm(40.0)]), Some(35.0));
        assert_eq!(average_dimms(&[]), None);
    }

    #[test]
    fn capacity_text_uses_decimal_units() {
        assert_eq!(capacity_text(1_000_000_000_000), "1.00 TB");
        assert_eq!(capacity_text(500_000_000_000), "500 GB");
    }
}
