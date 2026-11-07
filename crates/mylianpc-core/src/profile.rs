//! Hardware profiles, LCD settings and the atomic JSON store.
//!
//! Ported from `MyLianPC.Core/Profiles/{HardwareProfile,DefaultProfiles,ProfileStore}.cs`.
//! Profiles are shared by the whole machine; only the last activated profile is
//! remembered per user. Fields are additive and defaulted so older files load.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::color::RgbColor;
use crate::effect::{LightingEffect, ZoneLighting};
use crate::fan::{presets, FanCurve, TemperatureSource};
use crate::metrics;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LcdMode {
    #[default]
    Theme,
    Image,
    Animation,
    Off,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GaugeStyle {
    #[default]
    Arc,
    Ring,
    Bar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LcdWidgetType {
    #[default]
    Metric,
    Gauge,
    Clock,
    Date,
    Text,
    Bar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemperatureUnit {
    #[default]
    Celsius,
    Fahrenheit,
}

/// One movable element of the custom LCD layout. Coordinates are panel pixels (0-480).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LcdWidget {
    #[serde(rename = "type")]
    pub kind: LcdWidgetType,
    pub metric: String,
    pub label: Option<String>,
    pub text: Option<String>,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub color: Option<RgbColor>,
}

impl Default for LcdWidget {
    fn default() -> Self {
        Self {
            kind: LcdWidgetType::Metric,
            metric: metrics::CPU_TEMP.to_string(),
            label: None,
            text: None,
            x: 240.0,
            y: 240.0,
            size: 48.0,
            color: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LcdSettings {
    pub mode: LcdMode,
    pub theme_id: String,
    pub background: RgbColor,
    pub text_color: RgbColor,
    pub accent: RgbColor,
    pub secondary: RgbColor,
    pub font_family: String,
    pub font_scale: f64,
    pub unit: TemperatureUnit,
    pub clock_24_hour: bool,
    pub gauge_style: GaugeStyle,
    /// Metrics shown by the multi-metric and dashboard themes (in order).
    pub metrics: Vec<String>,
    pub widgets: Vec<LcdWidget>,
    /// Image or GIF stored by the service in its media folder (file name only).
    pub media_file: Option<String>,
    /// Draw the clock and CPU temperature over images/animations.
    pub media_overlay: bool,
    pub brightness: i32,
    pub rotation_degrees: i32,
    pub refresh_milliseconds: u64,
}

impl Default for LcdSettings {
    fn default() -> Self {
        Self {
            mode: LcdMode::Theme,
            theme_id: "minimal-temps".to_string(),
            background: RgbColor::new(0x05, 0x07, 0x0C),
            text_color: RgbColor::new(0xE8, 0xEC, 0xF4),
            accent: RgbColor::new(0x00, 0xE0, 0xB0),
            secondary: RgbColor::new(0x7A, 0x5C, 0xFF),
            // The Windows default was Segoe UI; Linux uses a font that is
            // normally installed. The renderer falls back to DejaVu Sans when a
            // profile still names Segoe UI.
            font_family: "DejaVu Sans".to_string(),
            font_scale: 1.0,
            unit: TemperatureUnit::Celsius,
            clock_24_hour: true,
            gauge_style: GaugeStyle::Arc,
            metrics: default_dashboard_metrics(),
            widgets: default_widgets(),
            media_file: None,
            media_overlay: false,
            brightness: 100,
            rotation_degrees: 0,
            refresh_milliseconds: 1000,
        }
    }
}

impl LcdSettings {
    pub const MAX_WIDGETS: usize = 16;

    /// A media file name that cannot escape the service's media folder.
    pub fn media_file_name(&self) -> Option<&str> {
        self.media_file
            .as_deref()
            .map(str::trim)
            .filter(|name| {
                !name.is_empty()
                    && !name.contains('/')
                    && !name.contains('\\')
                    && !name.contains(':')
                    && !name.contains("..")
            })
    }

    /// Clamps a draft from an IPC client before it reaches the renderer. This
    /// is used for previews only; saved settings are validated when stored.
    pub fn preview_safe(&self) -> Self {
        let mut safe = self.clone();
        safe.brightness = safe.brightness.clamp(0, 100);
        safe.rotation_degrees = match safe.rotation_degrees {
            90 => 90,
            180 => 180,
            270 => 270,
            _ => 0,
        };
        safe.refresh_milliseconds = safe.refresh_milliseconds.clamp(500, 60_000);
        safe.font_scale = if safe.font_scale.is_finite() {
            safe.font_scale.clamp(0.5, 2.0)
        } else {
            1.0
        };
        safe.widgets.truncate(Self::MAX_WIDGETS);
        for widget in &mut safe.widgets {
            widget.x = widget.x.clamp(0.0, 480.0);
            widget.y = widget.y.clamp(0.0, 480.0);
            widget.size = widget.size.clamp(8.0, 300.0);
        }
        safe.metrics.retain(|key| crate::metrics::find(key).is_some());
        safe.media_file = self.media_file_name().map(str::to_string);
        safe
    }

    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !(0..=100).contains(&self.brightness) {
            problems.push("LCD brightness must be between 0 and 100.".to_string());
        }
        if !matches!(self.rotation_degrees, 0 | 90 | 180 | 270) {
            problems.push("LCD rotation must be 0, 90, 180 or 270.".to_string());
        }
        if !(500..=60_000).contains(&self.refresh_milliseconds) {
            problems.push("LCD refresh must be between 500 ms and 60 s.".to_string());
        }
        if !(0.5..=2.0).contains(&self.font_scale) {
            problems.push("LCD font scale must be between 0.5 and 2.".to_string());
        }
        if self.widgets.len() > Self::MAX_WIDGETS {
            problems.push(format!(
                "The custom layout supports at most {} widgets.",
                Self::MAX_WIDGETS
            ));
        }
        if self.widgets.iter().any(|widget| {
            !(0.0..=480.0).contains(&widget.x)
                || !(0.0..=480.0).contains(&widget.y)
                || !(8.0..=300.0).contains(&widget.size)
        }) {
            problems.push(
                "Widget positions must be within the panel (0-480) and sizes 8-300.".to_string(),
            );
        }
        if self.media_file.is_some() && self.media_file_name().is_none() {
            problems.push("Invalid media file name.".to_string());
        }
        let needs_media = matches!(self.mode, LcdMode::Image | LcdMode::Animation);
        if needs_media
            && self
                .media_file
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
        {
            problems.push("Choose an image or animation file for this LCD mode.".to_string());
        }
        problems
    }
}

fn default_dashboard_metrics() -> Vec<String> {
    metrics::ALL
        .iter()
        .filter(|info| info.group != "Cooler")
        .map(|info| info.key.to_string())
        .collect()
}

fn default_widgets() -> Vec<LcdWidget> {
    vec![
        LcdWidget {
            kind: LcdWidgetType::Clock,
            x: 240.0,
            y: 120.0,
            size: 56.0,
            ..Default::default()
        },
        LcdWidget {
            kind: LcdWidgetType::Gauge,
            metric: metrics::CPU_TEMP.to_string(),
            label: Some("CPU".to_string()),
            x: 160.0,
            y: 270.0,
            size: 110.0,
            ..Default::default()
        },
        LcdWidget {
            kind: LcdWidgetType::Gauge,
            metric: metrics::GPU_TEMP.to_string(),
            label: Some("GPU".to_string()),
            x: 320.0,
            y: 270.0,
            size: 110.0,
            ..Default::default()
        },
        LcdWidget {
            kind: LcdWidgetType::Metric,
            metric: metrics::COOLANT_TEMP.to_string(),
            label: Some("Coolant".to_string()),
            x: 240.0,
            y: 400.0,
            size: 30.0,
            ..Default::default()
        },
    ]
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LightingSettings {
    /// When set, every zone uses this and `zones` is ignored.
    pub sync_all: Option<ZoneLighting>,
    /// Per-zone settings keyed by stable zone id (e.g. `fusion2:argb1`).
    pub zones: BTreeMap<String, ZoneLighting>,
}

impl LightingSettings {
    pub fn for_zone(&self, zone_id: &str) -> Option<&ZoneLighting> {
        self.sync_all.as_ref().or_else(|| self.zones.get(zone_id))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CoolingSettings {
    /// Curves keyed by fan target id (e.g. `hydroshift:pump`, `hydroshift:fans`).
    pub curves: BTreeMap<String, FanCurve>,
}

/// A machine-wide hardware configuration. Any section may be absent, meaning
/// "leave that part of the hardware alone" when the profile is applied.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub lighting: Option<LightingSettings>,
    #[serde(default)]
    pub lcd: Option<LcdSettings>,
    #[serde(default)]
    pub cooling: Option<CoolingSettings>,
    #[serde(default = "now_utc")]
    pub modified_utc: DateTime<Utc>,
}

impl HardwareProfile {
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.name.trim().is_empty() {
            problems.push("Profile name is required.".to_string());
        } else if self.name.chars().count() > 64 {
            problems.push("Profile name must be 64 characters or fewer.".to_string());
        }
        if let Some(cooling) = &self.cooling {
            for (target, curve) in &cooling.curves {
                for problem in curve.validate() {
                    problems.push(format!("Cooling '{target}': {problem}"));
                }
            }
        }
        if let Some(lcd) = &self.lcd {
            problems.extend(lcd.validate());
        }
        problems
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }
}

fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

/// Example profiles seeded on first run. Zone and fan target ids match the adapters.
/// Work is the default active profile.
pub fn default_profiles() -> Vec<HardwareProfile> {
    vec![
        make_profile(
            "Work",
            static_zone("#FFFFFF", 60),
            presets::balanced,
            "minimal-temps",
            100,
        ),
        make_profile(
            "Silent",
            static_zone("#3050FF", 40),
            presets::silent,
            "minimal-clock",
            100,
        ),
        make_profile(
            "Night Mode",
            static_zone("#FF3000", 15),
            presets::silent,
            "minimal-clock",
            20,
        ),
        make_profile(
            "Maximum Cooling",
            static_zone("#00A0FF", 100),
            presets::maximum,
            "multi-metric",
            100,
        ),
    ]
}

/// The shipped profile document. Work is active from the first start.
pub fn seeded_document() -> ProfileDocument {
    let profiles = default_profiles();
    let active = profiles
        .iter()
        .find(|profile| profile.name == "Work")
        .map(|profile| profile.id);
    ProfileDocument {
        profiles,
        active_profile_id: active,
        ..Default::default()
    }
}
fn static_zone(hex: &str, brightness: i32) -> ZoneLighting {
    ZoneLighting {
        effect: LightingEffect::Static,
        colors: vec![RgbColor::parse_hex(hex).expect("built-in colour")],
        brightness,
        speed: 50,
    }
}

fn make_profile(
    name: &str,
    lighting: ZoneLighting,
    cooling: fn(TemperatureSource) -> FanCurve,
    theme: &str,
    lcd_brightness: i32,
) -> HardwareProfile {
    let mut curves = BTreeMap::new();
    curves.insert(
        "hydroshift:fans".to_string(),
        cooling(TemperatureSource::Coolant),
    );
    curves.insert(
        "hydroshift:pump".to_string(),
        cooling(TemperatureSource::Coolant),
    );

    HardwareProfile {
        id: Uuid::new_v4(),
        name: name.to_string(),
        lighting: Some(LightingSettings {
            sync_all: Some(lighting),
            zones: BTreeMap::new(),
        }),
        lcd: Some(LcdSettings {
            theme_id: theme.to_string(),
            brightness: lcd_brightness,
            ..Default::default()
        }),
        cooling: Some(CoolingSettings { curves }),
        modified_utc: Utc::now(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileDocument {
    pub schema_version: i32,
    pub active_profile_id: Option<Uuid>,
    /// Which profile each user last activated (keyed by user name or uid).
    pub active_by_user: BTreeMap<String, Uuid>,
    pub profiles: Vec<HardwareProfile>,
}

impl Default for ProfileDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            active_profile_id: None,
            active_by_user: BTreeMap::new(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum ProfileError {
    Validation(Vec<String>),
    NotFound(Uuid),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(problems) => {
                write!(f, "profile is invalid: {}", problems.join(" "))
            }
            Self::NotFound(id) => write!(f, "profile {id} not found"),
            Self::Io(error) => write!(f, "profile storage: {error}"),
            Self::Json(error) => write!(f, "profile JSON: {error}"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl From<std::io::Error> for ProfileError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ProfileError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Machine-wide profile storage. The service is the only writer; desktop
/// clients go through IPC. Writes are atomic so a crash never leaves a torn file.
#[derive(Debug)]
pub struct ProfileStore {
    path: PathBuf,
    doc: ProfileDocument,
}

impl ProfileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            doc: ProfileDocument::default(),
        }
    }

    /// `MYLIANPC_DATA_DIR` → `$XDG_DATA_HOME/mylianpc` → `$HOME/.local/share/mylianpc`
    /// → `/var/lib/mylianpc`.
    pub fn default_path() -> PathBuf {
        if let Ok(dir) = std::env::var("MYLIANPC_DATA_DIR") {
            if !dir.trim().is_empty() {
                return PathBuf::from(dir).join("profiles.json");
            }
        }
        if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
            if !dir.trim().is_empty() {
                return PathBuf::from(dir).join("mylianpc").join("profiles.json");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            if !home.trim().is_empty() {
                return PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("mylianpc")
                    .join("profiles.json");
            }
        }
        PathBuf::from("/var/lib/mylianpc/profiles.json")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn snapshot(&self) -> &ProfileDocument {
        &self.doc
    }

    pub fn active(&self) -> Option<&HardwareProfile> {
        self.doc
            .profiles
            .iter()
            .find(|profile| Some(profile.id) == self.doc.active_profile_id)
    }

    pub fn active_for(&self, user: Option<&str>) -> Option<&HardwareProfile> {
        if let Some(user) = user {
            if let Some(id) = self.doc.active_by_user.get(user) {
                if let Some(profile) = self.doc.profiles.iter().find(|p| p.id == *id) {
                    return Some(profile);
                }
            }
        }
        self.active()
    }

    /// Loads from disk. Missing file seeds the defaults; a corrupt file is
    /// preserved as `.corrupt` and the defaults are used.
    pub fn load(&mut self) -> Result<(), ProfileError> {
        if !self.path.exists() {
            self.doc = seeded_document();
            self.save()?;
            return Ok(());
        }

        match fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str::<ProfileDocument>(&text) {
                Ok(doc) => {
                    self.doc = doc;
                    Ok(())
                }
                Err(_) => {
                    let mut corrupt = self.path.clone().into_os_string();
                    corrupt.push(".corrupt");
                    let _ = fs::copy(&self.path, PathBuf::from(corrupt));
                    self.doc = seeded_document();
                    self.save()?;
                    Ok(())
                }
            },
            Err(error) => Err(ProfileError::Io(error)),
        }
    }

    pub fn get(&self, id: Uuid) -> Result<&HardwareProfile, ProfileError> {
        self.doc
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or(ProfileError::NotFound(id))
    }

    pub fn upsert(&mut self, profile: HardwareProfile) -> Result<HardwareProfile, ProfileError> {
        let problems = profile.validate();
        if !problems.is_empty() {
            return Err(ProfileError::Validation(problems));
        }
        if self.doc.profiles.iter().any(|other| {
            other.id != profile.id && other.name.eq_ignore_ascii_case(&profile.name)
        }) {
            return Err(ProfileError::Validation(vec![format!(
                "A profile named '{}' already exists.",
                profile.name
            )]));
        }

        let mut stamped = profile;
        stamped.modified_utc = Utc::now();
        if let Some(existing) = self
            .doc
            .profiles
            .iter_mut()
            .find(|existing| existing.id == stamped.id)
        {
            *existing = stamped.clone();
        } else {
            self.doc.profiles.push(stamped.clone());
        }
        self.save()?;
        Ok(stamped)
    }

    pub fn duplicate(
        &mut self,
        id: Uuid,
        new_name: &str,
    ) -> Result<HardwareProfile, ProfileError> {
        let mut copy = self.get(id)?.clone();
        copy.id = Uuid::new_v4();
        copy.name = new_name.to_string();
        self.upsert(copy)
    }

    pub fn rename(&mut self, id: Uuid, new_name: &str) -> Result<HardwareProfile, ProfileError> {
        let mut profile = self.get(id)?.clone();
        profile.name = new_name.to_string();
        self.upsert(profile)
    }

    pub fn delete(&mut self, id: Uuid) -> Result<(), ProfileError> {
        self.get(id)?;
        self.doc.profiles.retain(|profile| profile.id != id);
        if self.doc.active_profile_id == Some(id) {
            self.doc.active_profile_id = None;
        }
        self.doc.active_by_user.retain(|_, active| *active != id);
        self.save()
    }

    pub fn set_active(&mut self, id: Uuid) -> Result<(), ProfileError> {
        self.get(id)?;
        self.doc.active_profile_id = Some(id);
        self.save()
    }

    pub fn set_active_for(
        &mut self,
        user: Option<&str>,
        id: Uuid,
    ) -> Result<(), ProfileError> {
        self.get(id)?;
        self.doc.active_profile_id = Some(id);
        if let Some(user) = user {
            if !user.is_empty() {
                self.doc.active_by_user.insert(user.to_string(), id);
            }
        }
        self.save()
    }

    /// Replaces every profile with the shipped examples. Work becomes active
    /// again. Used by "Reset profiles" in the UI.
    pub fn reset_defaults(&mut self) -> Result<(), ProfileError> {
        self.doc = seeded_document();
        self.save()
    }

    pub fn export(&self, id: Uuid) -> Result<String, ProfileError> {
        Ok(serde_json::to_string_pretty(self.get(id)?)?)
    }

    /// Imports a profile exported by [`ProfileStore::export`]. It always gets a
    /// fresh id; a clashing name gets a numeric suffix.
    pub fn import(&mut self, json: &str) -> Result<HardwareProfile, ProfileError> {
        let mut profile: HardwareProfile = serde_json::from_str(json)?;
        let original = profile.name.clone();
        let mut name = original.clone();
        let mut suffix = 2;
        while self
            .doc
            .profiles
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&name))
        {
            name = format!("{original} ({suffix})");
            suffix += 1;
        }
        profile.id = Uuid::new_v4();
        profile.name = name;
        self.upsert(profile)
    }

    fn save(&self) -> Result<(), ProfileError> {
        let json = serde_json::to_string_pretty(&self.doc)?;
        atomic_write(&self.path, &json)?;
        Ok(())
    }
}

fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, ProfileStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let mut store = ProfileStore::new(path);
        store.load().unwrap();
        (dir, store)
    }

    #[test]
    fn seeds_defaults_when_the_file_is_missing() {
        let (dir, store) = temp_store();
        let names: Vec<&str> = store
            .snapshot()
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert_eq!(names, ["Work", "Silent", "Night Mode", "Maximum Cooling"]);
        assert_eq!(
            store.active().map(|profile| profile.name.as_str()),
            Some("Work")
        );
        assert!(dir.path().join("profiles.json").exists());
        assert!(store.snapshot().profiles.iter().all(|p| p.is_valid()));
    }

    #[test]
    fn profiles_round_trip_through_json() {
        let profile = default_profiles().remove(0);
        let json = serde_json::to_string_pretty(&profile).unwrap();
        let back: HardwareProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, profile.id);
        assert_eq!(back.name, profile.name);
        assert_eq!(
            back.lcd.as_ref().map(|lcd| lcd.theme_id.as_str()),
            Some("minimal-temps")
        );
    }

    #[test]
    fn rejects_duplicate_names_case_insensitively() {
        let (_dir, mut store) = temp_store();
        let existing = store.snapshot().profiles[0].clone();
        let mut clash = existing.clone();
        clash.id = Uuid::new_v4();
        clash.name = existing.name.to_uppercase();
        assert!(matches!(
            store.upsert(clash),
            Err(ProfileError::Validation(_))
        ));
    }

    #[test]
    fn delete_clears_active_and_user_choice() {
        let (_dir, mut store) = temp_store();
        let id = store.snapshot().profiles[0].id;
        store.set_active_for(Some("abdh"), id).unwrap();
        store.delete(id).unwrap();
        assert_eq!(store.snapshot().active_profile_id, None);
        assert!(store.snapshot().active_by_user.is_empty());
    }

    #[test]
    fn corrupt_file_is_preserved_and_defaults_are_reseeded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        fs::write(&path, "not json").unwrap();
        let mut store = ProfileStore::new(&path);
        store.load().unwrap();
        assert!(path.with_extension("json.corrupt").exists());
        assert_eq!(store.snapshot().profiles.len(), 4);
    }

    #[test]
    fn import_suffixes_a_clashing_name() {
        let (_dir, mut store) = temp_store();
        let json = store.export(store.snapshot().profiles[0].id).unwrap();
        let imported = store.import(&json).unwrap();
        assert_eq!(imported.name, "Work (2)");
        assert_ne!(imported.id, store.snapshot().profiles[0].id);
    }

    #[test]
    fn lcd_validation_rejects_bad_geometry_and_media() {
        let mut lcd = LcdSettings::default();
        lcd.widgets[0].x = 999.0;
        lcd.mode = LcdMode::Image;
        let problems = lcd.validate();
        assert!(problems.iter().any(|p| p.contains("Widget positions")));
        assert!(problems.iter().any(|p| p.contains("image or animation")));
    }

    #[test]
    fn preview_safe_clamps_drafts_and_rejects_unsafe_media_names() {
        let lcd = LcdSettings {
            brightness: 500,
            rotation_degrees: 45,
            refresh_milliseconds: 10,
            font_scale: f64::NAN,
            metrics: vec!["cpu.temp".to_string(), "not.a.metric".to_string()],
            widgets: vec![LcdWidget::default(); LcdSettings::MAX_WIDGETS + 4],
            media_file: Some("../../etc/passwd".to_string()),
            ..Default::default()
        };

        let safe = lcd.preview_safe();
        assert_eq!(safe.brightness, 100);
        assert_eq!(safe.rotation_degrees, 0);
        assert_eq!(safe.refresh_milliseconds, 500);
        assert_eq!(safe.font_scale, 1.0);
        assert_eq!(safe.metrics, vec!["cpu.temp".to_string()]);
        assert_eq!(safe.widgets.len(), LcdSettings::MAX_WIDGETS);
        assert_eq!(safe.media_file, None);

        for bad in ["/etc/passwd", "..\\x", "a:b", ".."] {
            let lcd = LcdSettings {
                media_file: Some(bad.to_string()),
                ..Default::default()
            };
            assert!(lcd.preview_safe().media_file.is_none(), "{bad}");
            assert!(!lcd.validate().is_empty(), "{bad}");
        }
    }

    #[test]
    fn default_profiles_cover_every_preset_shape() {
        let profiles = default_profiles();
        assert_eq!(profiles.len(), 4);
        let silent = profiles.iter().find(|p| p.name == "Silent").unwrap();
        assert_eq!(
            crate::fan::presets::name_of(
                &silent.cooling.as_ref().unwrap().curves["hydroshift:fans"]
            ),
            Some("Silent")
        );
        assert!(profiles.iter().all(|p| p.is_valid()));
    }

    #[test]
    fn zone_lookup_prefers_sync_all() {
        let zone = ZoneLighting {
            effect: LightingEffect::Static,
            colors: vec![RgbColor::WHITE],
            brightness: 100,
            speed: 50,
        };
        let settings = LightingSettings {
            sync_all: Some(zone.clone()),
            zones: BTreeMap::new(),
        };
        assert_eq!(settings.for_zone("anything").unwrap().effect, LightingEffect::Static);
    }
}
