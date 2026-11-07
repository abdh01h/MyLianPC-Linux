//! Machine-wide device settings and per-user interface preferences.
//!
//! Ported from `MyLianPC.Core/Settings/{DeviceSettings,UserPreferences}.cs`.
//! Device settings live next to the profiles and are owned by the service;
//! preferences only affect the UI and are stored per user.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::color::RgbColor;

/// Atomically replace a text file: write a sibling `.tmp`, fsync it, then rename.
pub fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Machine-wide hardware configuration that is not part of a profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceSettings {
    /// LED count per addressable zone id (e.g. `fusion2:argb1`).
    pub zone_led_counts: BTreeMap<String, usize>,
    /// Motherboard fan header ids the user explicitly opted into controlling.
    pub controlled_motherboard_fans: Vec<String>,
    /// Save motherboard effects to the controller's flash so they survive power loss.
    pub save_motherboard_lighting_to_flash: bool,
    /// Also light the radiator fans wired to the pump head (experimental).
    pub cooler_fan_lighting: bool,
    /// Sensor polling interval (ms).
    pub sensor_interval_ms: u64,
}

impl Default for DeviceSettings {
    fn default() -> Self {
        Self {
            zone_led_counts: BTreeMap::new(),
            controlled_motherboard_fans: Vec::new(),
            save_motherboard_lighting_to_flash: true,
            cooler_fan_lighting: true,
            sensor_interval_ms: 1000,
        }
    }
}

impl DeviceSettings {
    pub fn normalized(mut self) -> Self {
        self.sensor_interval_ms = self.sensor_interval_ms.clamp(500, 10_000);
        self.zone_led_counts = self
            .zone_led_counts
            .into_iter()
            .map(|(key, value)| (key, value.clamp(1, 256)))
            .collect();
        self.controlled_motherboard_fans.sort();
        self.controlled_motherboard_fans.dedup();
        self
    }

    pub fn load(path: impl AsRef<Path>) -> Self {
        read_json::<DeviceSettings>(path.as_ref())
            .unwrap_or_default()
            .normalized()
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        atomic_write(
            path.as_ref(),
            &serde_json::to_string_pretty(self).map_err(std::io::Error::other)?,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppearanceMode {
    Dark,
    Light,
    #[default]
    System,
}

/// Per-user interface preferences. Nothing here may change hardware state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UserPreferences {
    pub appearance: AppearanceMode,
    pub temperature_unit: crate::profile::TemperatureUnit,
    pub minimize_to_tray: bool,
    pub dashboard_refresh_ms: u64,
    pub ui_scale: f64,
    pub dashboard_metrics: Vec<String>,
    pub saved_colors: Vec<RgbColor>,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            appearance: AppearanceMode::System,
            temperature_unit: crate::profile::TemperatureUnit::Celsius,
            minimize_to_tray: true,
            dashboard_refresh_ms: 1000,
            ui_scale: 1.0,
            dashboard_metrics: crate::metrics::ALL
                .iter()
                .filter(|info| info.group != "Cooler")
                .map(|info| info.key.to_string())
                .collect(),
            saved_colors: Vec::new(),
        }
    }
}

impl UserPreferences {
    pub const MIN_UI_SCALE: f64 = 0.8;
    pub const MAX_UI_SCALE: f64 = 1.4;

    pub fn normalized(mut self) -> Self {
        self.dashboard_refresh_ms = self.dashboard_refresh_ms.clamp(250, 10_000);
        self.ui_scale = if self.ui_scale.is_finite() {
            (self.ui_scale * 100.0).round().clamp(
                Self::MIN_UI_SCALE * 100.0,
                Self::MAX_UI_SCALE * 100.0,
            ) / 100.0
        } else {
            1.0
        };
        self
    }

    pub fn load(path: impl AsRef<Path>) -> Self {
        read_json::<UserPreferences>(path.as_ref())
            .unwrap_or_default()
            .normalized()
    }

    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        atomic_write(
            path.as_ref(),
            &serde_json::to_string_pretty(self).map_err(std::io::Error::other)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_settings_normalize_ranges() {
        let settings = DeviceSettings {
            sensor_interval_ms: 50,
            zone_led_counts: BTreeMap::from([("a".to_string(), 0), ("b".to_string(), 999)]),
            ..Default::default()
        }
        .normalized();
        assert_eq!(settings.sensor_interval_ms, 500);
        assert_eq!(settings.zone_led_counts["a"], 1);
        assert_eq!(settings.zone_led_counts["b"], 256);
    }

    #[test]
    fn preferences_clamp_refresh_and_scale() {
        let prefs = UserPreferences {
            dashboard_refresh_ms: 10,
            ui_scale: 9.0,
            ..Default::default()
        }
        .normalized();
        assert_eq!(prefs.dashboard_refresh_ms, 250);
        assert_eq!(prefs.ui_scale, 1.4);
    }

    #[test]
    fn settings_round_trip_and_tolerate_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let settings = DeviceSettings {
            cooler_fan_lighting: true,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        assert!(DeviceSettings::load(&path).cooler_fan_lighting);

        fs::write(&path, r#"{"cooler_fan_lighting":true,"future":1}"#).unwrap();
        assert!(DeviceSettings::load(&path).cooler_fan_lighting);
    }

    #[test]
    fn corrupt_preferences_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        fs::write(&path, "not json").unwrap();
        let prefs = UserPreferences::load(&path);
        assert_eq!(prefs.dashboard_refresh_ms, 1000);
    }
}
