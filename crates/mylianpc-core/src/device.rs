//! Device models shared by the service and the UI.
//!
//! Ported from `MyLianPC.Core/Devices/DeviceModels.cs`. Adapters advertise exactly
//! what they can do; the UI only offers the advertised effects, zones and fans.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::effect::{LightingEffect, ZoneCapabilities, ZoneLighting};
use crate::color::RgbColor;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceState {
    #[default]
    Disconnected,
    Connected,
    /// Another process holds the device.
    Busy,
    /// The device stopped answering; usually needs a power cycle.
    Unresponsive,
    /// A required driver or component is missing.
    NeedsDriver,
    Unsupported,
    Error,
}

impl DeviceState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Disconnected => "Not found",
            Self::Busy => "In use",
            Self::Unresponsive => "Not responding",
            Self::NeedsDriver => "Driver missing",
            Self::Unsupported => "Unsupported",
            Self::Error => "Error",
        }
    }
}

/// A lighting zone and exactly what it can do on this hardware.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneInfo {
    pub id: String,
    pub device_id: String,
    pub name: String,
    pub led_count: usize,
    pub effects: Vec<LightingEffect>,
    pub max_colors: usize,
    pub has_brightness: bool,
    pub has_speed: bool,
    /// True if the zone's LED count is configurable by the user (plain ARGB headers).
    pub led_count_configurable: bool,
    pub note: Option<String>,
}

impl ZoneInfo {
    pub fn capabilities(&self) -> ZoneCapabilities {
        ZoneCapabilities {
            effects: self.effects.clone(),
            hardware_brightness: false,
            per_led_color: self.led_count > 1,
            led_count: self.led_count,
            max_colors: self.max_colors,
        }
    }
}

/// A fan channel on a device.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FanChannelInfo {
    pub id: String,
    pub name: String,
    /// Who drives this fan, e.g. "HydroShift II pump head".
    pub controller: String,
    pub rpm: Option<f64>,
    pub duty_percent: Option<i32>,
    pub controllable: bool,
    /// Motherboard headers are monitoring-only until the user opts in.
    pub control_enabled: bool,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub id: String,
    pub name: String,
    pub manufacturer: String,
    pub kind: String,
    pub state: DeviceState,
    pub firmware: Option<String>,
    pub location: Option<String>,
    pub zones: Vec<ZoneInfo>,
    pub fans: Vec<FanChannelInfo>,
    pub has_lcd: bool,
    pub capabilities: Vec<String>,
    pub dependencies: Vec<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub troubleshooting: Option<String>,
    pub last_contact_utc: Option<DateTime<Utc>>,
}

impl DeviceStatus {
    pub fn placeholder(id: &str, name: &str, manufacturer: &str, kind: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            manufacturer: manufacturer.to_string(),
            kind: kind.to_string(),
            state: DeviceState::Disconnected,
            firmware: None,
            location: None,
            zones: Vec::new(),
            fans: Vec::new(),
            has_lcd: false,
            capabilities: Vec::new(),
            dependencies: Vec::new(),
            warnings: Vec::new(),
            error: None,
            troubleshooting: None,
            last_contact_utc: None,
        }
    }
}

/// Result of applying one part of a profile to one device.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyResult {
    pub device_id: String,
    pub part: String,
    pub success: bool,
    pub message: String,
}

impl ApplyResult {
    pub fn ok(device_id: impl Into<String>, part: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            device_id: device_id.into(),
            part: part.into(),
            success: true,
            message: message.into(),
        }
    }

    pub fn failed(device_id: impl Into<String>, part: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            device_id: device_id.into(),
            part: part.into(),
            success: false,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyReport {
    pub profile_name: String,
    pub time_utc: DateTime<Utc>,
    pub results: Vec<ApplyResult>,
}

impl ApplyReport {
    pub fn empty(profile_name: impl Into<String>) -> Self {
        Self {
            profile_name: profile_name.into(),
            time_utc: Utc::now(),
            results: Vec::new(),
        }
    }

    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|result| result.success)
    }

    pub fn push(&mut self, result: ApplyResult) {
        self.results.push(result);
    }
}

/// Maps a requested effect onto what a zone really supports.
///
/// Returns the fitted lighting (clamped brightness/speed and the colour list cut
/// to what the zone accepts) and, when the requested effect had to be replaced,
/// the effect that was used instead.
pub fn fit_lighting(
    requested: &ZoneLighting,
    zone: &ZoneInfo,
) -> (ZoneLighting, Option<LightingEffect>) {
    let max_colors = zone.max_colors.max(1);
    let mut colors: Vec<RgbColor> = requested.colors.iter().take(max_colors).copied().collect();
    if colors.is_empty() {
        colors.push(RgbColor::WHITE);
    }
    let mut fitted = ZoneLighting {
        effect: requested.effect,
        colors,
        brightness: requested.brightness.clamp(0, 100),
        speed: requested.speed.clamp(0, 100),
    };

    if zone.effects.contains(&requested.effect) {
        return (fitted, None);
    }

    let fallbacks: &[LightingEffect] = match requested.effect {
        LightingEffect::Wave => &[
            LightingEffect::Rainbow,
            LightingEffect::ColorCycle,
            LightingEffect::Static,
        ],
        LightingEffect::Rainbow => &[LightingEffect::ColorCycle, LightingEffect::Static],
        LightingEffect::RainbowWave => &[
            LightingEffect::Rainbow,
            LightingEffect::ColorCycle,
            LightingEffect::Static,
        ],
        LightingEffect::Gradient => &[LightingEffect::Static],
        LightingEffect::Flash => &[LightingEffect::Breathing, LightingEffect::Static],
        _ => &[LightingEffect::Static],
    };
    if let Some(substitute) = fallbacks
        .iter()
        .copied()
        .find(|effect| zone.effects.contains(effect))
    {
        fitted.effect = substitute;
        (fitted, Some(substitute))
    } else {
        (fitted, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(effects: &[LightingEffect], max_colors: usize) -> ZoneInfo {
        ZoneInfo {
            id: "z".to_string(),
            device_id: "d".to_string(),
            name: "Zone".to_string(),
            led_count: 12,
            effects: effects.to_vec(),
            max_colors,
            has_brightness: true,
            has_speed: true,
            led_count_configurable: false,
            note: None,
        }
    }

    #[test]
    fn fit_keeps_a_supported_effect_and_clamps_values() {
        let requested = ZoneLighting {
            effect: LightingEffect::Static,
            colors: vec![RgbColor::WHITE; 4],
            brightness: 150,
            speed: -5,
        };
        let (fitted, substitute) = fit_lighting(&requested, &zone(&[LightingEffect::Static], 2));
        assert_eq!(fitted.effect, LightingEffect::Static);
        assert_eq!(fitted.brightness, 100);
        assert_eq!(fitted.speed, 0);
        assert_eq!(fitted.colors.len(), 2);
        assert_eq!(substitute, None);
    }

    #[test]
    fn fit_replaces_an_unsupported_effect_with_the_documented_fallback() {
        let requested = ZoneLighting {
            effect: LightingEffect::RainbowWave,
            colors: vec![RgbColor::WHITE],
            brightness: 50,
            speed: 50,
        };
        let (fitted, substitute) = fit_lighting(
            &requested,
            &zone(&[LightingEffect::Static, LightingEffect::ColorCycle], 1),
        );
        assert_eq!(fitted.effect, LightingEffect::ColorCycle);
        assert_eq!(substitute, Some(LightingEffect::ColorCycle));
    }

    #[test]
    fn fit_adds_a_white_colour_when_none_was_requested() {
        let requested = ZoneLighting {
            effect: LightingEffect::Static,
            colors: Vec::new(),
            brightness: 100,
            speed: 50,
        };
        let (fitted, _) = fit_lighting(&requested, &zone(&[LightingEffect::Static], 1));
        assert_eq!(fitted.colors, vec![RgbColor::WHITE]);
    }

    #[test]
    fn states_round_trip_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&DeviceState::NeedsDriver).unwrap(),
            r#""needs-driver""#
        );
    }
}
