//! Well-known metric keys used by the dashboard, LCD themes and fan curves.
//!
//! Ported from `MyLianPC.Core/Sensors/SensorModels.cs`. Keep the strings stable:
//! profiles store them.

pub const CPU_TEMP: &str = "cpu.temp";
pub const CPU_LOAD: &str = "cpu.load";
pub const CPU_CLOCK: &str = "cpu.clock";
pub const CPU_POWER: &str = "cpu.power";

pub const GPU_TEMP: &str = "gpu.temp";
pub const GPU_LOAD: &str = "gpu.load";
pub const GPU_CLOCK: &str = "gpu.clock";
pub const GPU_POWER: &str = "gpu.power";
pub const GPU_FAN: &str = "gpu.fan";
pub const GPU_VRAM_USED: &str = "gpu.vram.used";
pub const GPU_VRAM_TOTAL: &str = "gpu.vram.total";
pub const GPU_VRAM_FREE: &str = "gpu.vram.free";

pub const RAM_LOAD: &str = "ram.load";
pub const RAM_USED_GB: &str = "ram.used";
pub const RAM_TOTAL_GB: &str = "ram.total";
pub const RAM_TEMP: &str = "ram.temp";
pub const RAM_SPEED: &str = "ram.speed";
pub const RAM_XMP: &str = "ram.xmp";

pub const STORAGE_TEMP: &str = "storage.temp";
pub const STORAGE_HEALTH: &str = "storage.health";

pub const COOLANT_TEMP: &str = "coolant.temp";
pub const PUMP_RPM: &str = "pump.rpm";
pub const AIO_FAN1: &str = "aio.fan1";
pub const AIO_FAN2: &str = "aio.fan2";
pub const AIO_FAN3: &str = "aio.fan3";

#[derive(Clone, Copy, Debug)]
pub struct MetricInfo {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub group: &'static str,
}

pub const ALL: &[MetricInfo] = &[
    MetricInfo { key: CPU_TEMP, label: "CPU temperature", unit: "°C", group: "CPU" },
    MetricInfo { key: CPU_LOAD, label: "CPU utilization", unit: "%", group: "CPU" },
    MetricInfo { key: CPU_CLOCK, label: "CPU clock", unit: "MHz", group: "CPU" },
    MetricInfo { key: CPU_POWER, label: "CPU power", unit: "W", group: "CPU" },
    MetricInfo { key: GPU_TEMP, label: "GPU temperature", unit: "°C", group: "GPU" },
    MetricInfo { key: GPU_LOAD, label: "GPU utilization", unit: "%", group: "GPU" },
    MetricInfo { key: GPU_CLOCK, label: "GPU clock", unit: "MHz", group: "GPU" },
    MetricInfo { key: GPU_POWER, label: "GPU power", unit: "W", group: "GPU" },
    MetricInfo { key: GPU_FAN, label: "GPU fan", unit: "RPM", group: "GPU" },
    MetricInfo { key: GPU_VRAM_TOTAL, label: "VRAM total", unit: "GB", group: "GPU" },
    MetricInfo { key: GPU_VRAM_USED, label: "VRAM used", unit: "GB", group: "GPU" },
    MetricInfo { key: GPU_VRAM_FREE, label: "VRAM free", unit: "GB", group: "GPU" },
    MetricInfo { key: RAM_LOAD, label: "Memory usage", unit: "%", group: "Memory" },
    MetricInfo { key: RAM_USED_GB, label: "Memory used", unit: "GB", group: "Memory" },
    MetricInfo { key: RAM_TEMP, label: "Memory temperature", unit: "°C", group: "Memory" },
    MetricInfo { key: RAM_SPEED, label: "Memory speed", unit: "MT/s", group: "Memory" },
    MetricInfo { key: RAM_XMP, label: "EXPO / XMP", unit: "", group: "Memory" },
    MetricInfo { key: COOLANT_TEMP, label: "Coolant temperature", unit: "°C", group: "Cooler" },
    MetricInfo { key: PUMP_RPM, label: "Pump speed", unit: "RPM", group: "Cooler" },
    MetricInfo { key: AIO_FAN1, label: "Radiator fan 1", unit: "RPM", group: "Cooler" },
    MetricInfo { key: AIO_FAN2, label: "Radiator fan 2", unit: "RPM", group: "Cooler" },
    MetricInfo { key: AIO_FAN3, label: "Radiator fan 3", unit: "RPM", group: "Cooler" },
    MetricInfo { key: STORAGE_TEMP, label: "Drive temperature", unit: "°C", group: "Storage" },
    MetricInfo { key: STORAGE_HEALTH, label: "Drive health", unit: "%", group: "Storage" },
];

pub fn find(key: &str) -> Option<&'static MetricInfo> {
    ALL.iter().find(|info| info.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_keys_are_unique() {
        let mut keys: Vec<&str> = ALL.iter().map(|info| info.key).collect();
        keys.sort_unstable();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn finds_a_known_metric() {
        assert_eq!(find(CPU_TEMP).map(|info| info.unit), Some("°C"));
        assert!(find("nope").is_none());
    }
}
