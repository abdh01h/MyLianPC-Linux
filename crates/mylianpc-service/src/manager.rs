//! The single owner of all hardware.
//!
//! Ported from `MyLianPC.Service/HardwareManager.cs`. Every loop that changes
//! hardware runs through this type, so no two code paths issue contradictory
//! commands.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use uuid::Uuid;

use mylianpc_core::device::{
    fit_lighting, ApplyReport, ApplyResult, DeviceState, DeviceStatus, ZoneInfo,
};
use mylianpc_core::effect::{LightingEffect, ZoneLighting};
use mylianpc_core::fan::{presets, FanCurve, FanCurveController, TemperatureSource};
use mylianpc_core::metrics;
use mylianpc_core::network::{NetworkSnapshot, TrafficState};
use mylianpc_core::profile::{CoolingSettings, HardwareProfile, LcdSettings, LightingSettings, ProfileStore};
use mylianpc_core::settings::DeviceSettings;
use mylianpc_hardware::network::NetworkMonitor;
use mylianpc_hardware::sensors::{SensorSampler, Snapshot};
use mylianpc_lcd::{encode_jpeg, encode_png, themes, FontCache, LcdMedia, LcdMetrics};

use crate::adapters::{
    effect_label, CorsairDramAdapter, Fusion2Adapter, HydroShiftAdapter, CORSAIR_DEVICE_ID,
    FANS_TARGET, FUSION2_DEVICE_ID, HYDROSHIFT_DEVICE_ID, PUMP_TARGET,
};
use crate::paths::ServicePaths;

#[derive(Clone, Debug, Serialize)]
pub struct ServiceStatus {
    pub version: String,
    pub started_utc: DateTime<Utc>,
    pub devices: Vec<DeviceStatus>,
    pub active_profile_id: Option<Uuid>,
    pub active_profile_name: Option<String>,
    pub last_apply: Option<ApplyReport>,
    pub warnings: Vec<String>,
    pub device_settings: DeviceSettings,
}

pub struct DeviceManager {
    pub paths: ServicePaths,
    pub profiles: ProfileStore,
    pub settings: DeviceSettings,
    pub fusion: Fusion2Adapter,
    pub cooler: HydroShiftAdapter,
    pub ram: CorsairDramAdapter,
    pub sensors: SensorSampler,
    pub network: NetworkMonitor,
    pub snapshot: Snapshot,
    pub network_snapshot: NetworkSnapshot,
    pub last_apply: Option<ApplyReport>,
    pub started_utc: DateTime<Utc>,
    fan_controllers: HashMap<String, (String, FanCurveController)>,
    lcd_version: u64,
    lcd_shown_version: u64,
    lcd_shown_generation: u64,
    lcd_last_jpeg: Option<Vec<u8>>,
    lcd_last_push: Option<Instant>,
    lcd_repush_at: Option<Instant>,
    lcd_frame: usize,
    lcd_next_due: Instant,
    fonts: FontCache,
    media_cache: HashMap<String, Arc<LcdMedia>>,
    last_network_sample: Instant,
    last_network_persist: Instant,
    was_fusion_connected: bool,
    was_cooler_connected: bool,
    was_ram_connected: bool,
    logs: Vec<String>,
}

impl DeviceManager {
    pub fn new(paths: ServicePaths) -> anyhow::Result<Self> {
        paths.ensure()?;
        let mut profiles = ProfileStore::new(paths.profiles());
        if let Err(error) = profiles.load() {
            eprintln!("profile store: {error}");
        }
        let settings = DeviceSettings::load(paths.devices());
        let transport = std::fs::read_to_string(paths.network())
            .ok()
            .and_then(|text| serde_json::from_str::<TrafficState>(&text).ok())
            .unwrap_or_default();
        let mut sensors = SensorSampler::new();
        let snapshot = sensors.sample();
        let mut network = NetworkMonitor::new(transport);
        let network_snapshot = network.sample();
        let mut cooler = HydroShiftAdapter::new();
        cooler.fan_lighting = settings.cooler_fan_lighting;
        let now = Instant::now();
        Ok(Self {
            paths,
            profiles,
            settings,
            fusion: Fusion2Adapter::new(),
            cooler,
            ram: CorsairDramAdapter::new(),
            sensors,
            network,
            snapshot,
            network_snapshot,
            last_apply: None,
            started_utc: Utc::now(),
            fan_controllers: HashMap::new(),
            lcd_version: 1,
            lcd_shown_version: 0,
            lcd_shown_generation: 0,
            lcd_last_jpeg: None,
            lcd_last_push: None,
            lcd_repush_at: None,
            lcd_frame: 0,
            lcd_next_due: now,
            fonts: FontCache::new(),
            media_cache: HashMap::new(),
            last_network_sample: now,
            last_network_persist: now,
            was_fusion_connected: false,
            was_cooler_connected: false,
            was_ram_connected: false,
            logs: Vec::new(),
        })
    }

    pub fn log(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("{message}");
        let entry = format!("{} {message}", Local::now().format("%Y-%m-%d %H:%M:%S"));
        self.logs.push(entry.clone());
        if self.logs.len() > 500 {
            self.logs.drain(..100);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.paths.logs().join("service.log"))
        {
            use std::io::Write;
            let _ = writeln!(file, "{entry}");
        }
    }

    pub fn recent_logs(&self) -> &[String] {
        &self.logs
    }

    // ------------------------------------------------------------- lifecycle

    pub fn shutdown(&mut self) {
        self.log("Service stopping: handing the cooler a safe fixed speed");
        self.cooler.safe_defaults();
        self.save_network(true);
        let _ = self.settings.save(self.paths.devices());
    }

    // ----------------------------------------------------------------- loops

    pub fn tick_sensors(&mut self) {
        self.snapshot = self.sensors.sample();
        if let Some(telemetry) = self.cooler.telemetry().cloned() {
            self.snapshot
                .set_value(metrics::COOLANT_TEMP, Some(telemetry.coolant_c as f64));
            self.snapshot
                .set_value(metrics::PUMP_RPM, Some(telemetry.pump_rpm as f64));
            for (index, rpm) in telemetry.fan_rpm.iter().enumerate() {
                let key = match index {
                    0 => metrics::AIO_FAN1,
                    1 => metrics::AIO_FAN2,
                    _ => metrics::AIO_FAN3,
                };
                self.snapshot.set_value(key, Some(*rpm as f64));
            }
            if let Some(name) = self.snapshot.names.cooler.clone() {
                self.snapshot.names.cooler = Some(name);
            }
        }
    }

    pub fn tick_cooling(&mut self) {
        if !self.cooler.ensure_connected() {
            return;
        }
        self.cooler.poll_telemetry();
        self.tick_telemetry_into_snapshot();
        let snapshot = self.snapshot.clone();
        let curves = self.effective_curves();
        let duty_for = |manager: &mut Self, target: &str, curve: &FanCurve| -> i32 {
            let temperature = match curve.source {
                TemperatureSource::CpuPackage => snapshot.value(metrics::CPU_TEMP),
                TemperatureSource::GpuCore => snapshot.value(metrics::GPU_TEMP),
                TemperatureSource::Coolant => snapshot.value(metrics::COOLANT_TEMP),
            };
            let signature = serde_json::to_string(curve).unwrap_or_default();
            let replace = manager
                .fan_controllers
                .get(target)
                .map(|(existing, _)| existing != &signature)
                .unwrap_or(true);
            if replace {
                manager
                    .fan_controllers
                    .insert(target.to_string(), (signature, FanCurveController::new(curve.clone())));
            }
            manager
                .fan_controllers
                .get_mut(target)
                .map(|(_, controller)| controller.next(temperature))
                .unwrap_or(100)
        };
        let pump = duty_for(self, PUMP_TARGET, &curves[PUMP_TARGET]);
        let fans = duty_for(self, FANS_TARGET, &curves[FANS_TARGET]);
        let pump_rpm = pump_rpm_for_duty(pump);
        self.cooler.set_cooling(pump_rpm, fans);
    }

    fn tick_telemetry_into_snapshot(&mut self) {
        let Some(telemetry) = self.cooler.telemetry().cloned() else {
            return;
        };
        self.snapshot
            .set_value(metrics::COOLANT_TEMP, Some(telemetry.coolant_c as f64));
        self.snapshot
            .set_value(metrics::PUMP_RPM, Some(telemetry.pump_rpm as f64));
        for (index, rpm) in telemetry.fan_rpm.iter().enumerate() {
            let key = match index {
                0 => metrics::AIO_FAN1,
                1 => metrics::AIO_FAN2,
                _ => metrics::AIO_FAN3,
            };
            self.snapshot.set_value(key, Some(*rpm as f64));
        }
    }

    pub fn tick_watch(&mut self) {
        let fusion = self.fusion.ensure_connected();
        let cooler = self.cooler.ensure_connected();
        let ram = self.ram.ensure_connected();
        let profile = self.profiles.active().cloned();

        if let Some(profile) = &profile {
            if profile.lighting.is_some() {
                if fusion && !self.was_fusion_connected {
                    let report = self.apply_lighting(profile, false, false, Some(FUSION2_DEVICE_ID));
                    self.log(format!(
                        "Fusion2 reconnected: {} lighting result(s)",
                        report.results.len()
                    ));
                }
                if ram && !self.was_ram_connected {
                    let report = self.apply_lighting(profile, false, false, Some(CORSAIR_DEVICE_ID));
                    self.log(format!(
                        "Corsair DRAM reconnected: {} lighting result(s)",
                        report.results.len()
                    ));
                } else if ram && self.ram.due_for_reapply() {
                    self.ram.clear_needs_reapply();
                    let report = self.apply_lighting(profile, true, false, Some(CORSAIR_DEVICE_ID));
                    self.log(format!(
                        "Corsair DRAM recovered: rewrote {} zone(s)",
                        report.results.len()
                    ));
                }
                if cooler && !self.was_cooler_connected {
                    let report = self.apply_lighting(profile, true, false, Some(HYDROSHIFT_DEVICE_ID));
                    self.log(format!(
                        "Cooler reconnected: {} lighting result(s)",
                        report.results.len()
                    ));
                }
            }
        }

        if let Some(reason) = self.cooler.fan_lighting_rejected.take() {
            self.log(format!("Radiator fan lighting turned off: {reason}"));
            self.settings.cooler_fan_lighting = false;
            let _ = self.settings.save(self.paths.devices());
        }
        self.was_fusion_connected = fusion;
        self.was_cooler_connected = cooler;
        self.was_ram_connected = ram;
    }

    /// Cheap high-frequency work: LED animation frames and the LCD.
    pub fn tick_fast(&mut self) {
        self.cooler.service_pending_fan();
        self.fusion.render_animations();
        self.tick_lcd();
        if self.last_network_sample.elapsed() >= Duration::from_secs(1) {
            self.network_snapshot = self.network.sample();
            self.last_network_sample = Instant::now();
        }
    }

    pub fn save_network(&mut self, force: bool) {
        if !force && self.last_network_persist.elapsed() < Duration::from_secs(30) {
            return;
        }
        if let Ok(text) = serde_json::to_string_pretty(self.network.state()) {
            let _ = mylianpc_core::settings::atomic_write(&self.paths.network(), &text);
        }
        self.last_network_persist = Instant::now();
    }

    // ------------------------------------------------------------- profiles

    pub fn zones(&self) -> Vec<ZoneInfo> {
        let devices = mylianpc_hardware::inventory::usb_devices();
        let present = |vid: &str, pid: &str| {
            devices
                .iter()
                .any(|device| device.vid.eq_ignore_ascii_case(vid) && device.pid.eq_ignore_ascii_case(pid))
        };
        let mut zones = Vec::new();
        if present("048d", "5711") {
            zones.extend(self.fusion.zones(&self.settings.zone_led_counts));
        }
        zones.extend(self.ram.zones());
        if present("1cbe", "a021") {
            zones.push(self.cooler.ring_zone());
        }
        zones
    }

    pub fn zone(&self, id: &str) -> Option<ZoneInfo> {
        self.zones().into_iter().find(|zone| zone.id == id)
    }

    pub fn lighting_get(&self) -> LightingSettings {
        self.profiles
            .active()
            .and_then(|profile| profile.lighting.clone())
            .unwrap_or_default()
    }

    fn ensure_active_profile(&mut self, name: &str) -> anyhow::Result<Uuid> {
        if let Some(profile) = self.profiles.active() {
            return Ok(profile.id);
        }
        // A stale "Current" profile may exist without being active; give the
        // working profile a unique name instead of failing the whole request.
        let mut unique = name.to_string();
        let mut suffix = 2;
        while self
            .profiles
            .snapshot()
            .profiles
            .iter()
            .any(|profile| profile.name.eq_ignore_ascii_case(&unique))
        {
            unique = format!("{name} {suffix}");
            suffix += 1;
        }
        let profile = HardwareProfile {
            id: Uuid::new_v4(),
            name: unique,
            lighting: Some(LightingSettings::default()),
            lcd: Some(LcdSettings::default()),
            cooling: Some(CoolingSettings::default()),
            modified_utc: Utc::now(),
        };
        let stored = self.profiles.upsert(profile)?;
        self.profiles.set_active(stored.id)?;
        Ok(stored.id)
    }

    fn update_active_profile(
        &mut self,
        change: impl FnOnce(&mut HardwareProfile),
    ) -> anyhow::Result<HardwareProfile> {
        let id = self.ensure_active_profile("Current")?;
        let mut profile = self.profiles.get(id)?.clone();
        change(&mut profile);
        Ok(self.profiles.upsert(profile)?)
    }

    pub fn apply_zone(&mut self, zone_id: &str, lighting: ZoneLighting) -> ApplyReport {
        let zone = self.zone(zone_id);
        let Some(zone) = zone else {
            return ApplyReport {
                profile_name: "(none)".to_string(),
                time_utc: Utc::now(),
                results: vec![ApplyResult::failed("service", zone_id, "Unknown zone.")],
            };
        };
        let updated = self
            .update_active_profile(|profile| {
                let settings = profile.lighting.get_or_insert_with(LightingSettings::default);
                if settings.sync_all.is_some() {
                    settings.sync_all = Some(lighting.clone());
                } else {
                    settings.zones.insert(zone_id.to_string(), lighting.clone());
                }
            })
            .ok();
        let mut report = ApplyReport::empty(
            updated
                .as_ref()
                .map(|profile| profile.name.clone())
                .unwrap_or_else(|| "(none)".to_string()),
        );
        let (fitted, substitute) = fit_lighting(&lighting, &zone);
        let mut result = self.apply_zone_now(&zone, &fitted);
        if let Some(substitute) = substitute {
            result.message = format!(
                "{} is not available on this zone; {} was used instead. {}",
                effect_label(lighting.effect),
                effect_label(substitute),
                result.message
            );
        }
        report.push(result);
        self.last_apply = Some(report.clone());
        report
    }

    fn apply_zone_now(&mut self, zone: &ZoneInfo, lighting: &ZoneLighting) -> ApplyResult {
        match zone.device_id.as_str() {
            FUSION2_DEVICE_ID => {
                if self.fusion.ensure_connected() {
                    self.fusion.begin_batch();
                }
                let result = self.fusion.apply(&zone.id, lighting, zone.led_count);
                let mut commit = self.fusion.commit(
                    self.settings.save_motherboard_lighting_to_flash,
                );
                // The zone result is the meaningful one; the commit result is
                // reported separately by apply_lighting.
                if result.success {
                    commit.part = zone.name.clone();
                    commit.message = format!("{:?} applied", lighting.effect);
                    commit
                } else {
                    result
                }
            }
            CORSAIR_DEVICE_ID => self.ram.apply(&zone.id, lighting, true),
            HYDROSHIFT_DEVICE_ID => self.cooler.apply_ring(lighting),
            _ => ApplyResult::failed(&zone.device_id, &zone.name, "Unknown device"),
        }
    }

    pub fn apply_all_lighting(&mut self, lighting: LightingSettings) -> ApplyReport {
        let profile = self
            .update_active_profile(|profile| profile.lighting = Some(lighting))
            .ok();
        match profile {
            Some(profile) => self.apply_lighting(&profile, true, true, None),
            None => ApplyReport {
                profile_name: "(none)".to_string(),
                time_utc: Utc::now(),
                results: vec![ApplyResult::failed("service", "Lighting", "Could not store the profile.")],
            },
        }
    }

    pub fn lighting_reset(&mut self) -> ApplyReport {
        let mut lighting = self.lighting_get();
        let off = ZoneLighting {
            effect: LightingEffect::Off,
            colors: vec![mylianpc_core::color::RgbColor::BLACK],
            brightness: 0,
            speed: 50,
        };
        if let Some(sync) = lighting.sync_all.as_mut() {
            *sync = off.clone();
        }
        for value in lighting.zones.values_mut() {
            *value = off.clone();
        }
        if lighting.sync_all.is_none() && lighting.zones.is_empty() {
            lighting.sync_all = Some(off);
        }
        self.apply_all_lighting(lighting)
    }

    pub fn reapply(&mut self) -> ApplyReport {
        let profile = self.profiles.active().cloned();
        match profile {
            Some(profile) => self.apply_active(&profile, "reapply requested"),
            None => ApplyReport::empty("(none)"),
        }
    }

    pub fn activate_profile(&mut self, id: Uuid) -> anyhow::Result<ApplyReport> {
        self.profiles.set_active(id)?;
        let profile = self.profiles.get(id)?.clone();
        Ok(self.apply_active(&profile, "profile activated"))
    }

    pub fn apply_active(&mut self, profile: &HardwareProfile, reason: &str) -> ApplyReport {
        self.log(format!("Applying profile '{}' ({reason})", profile.name));

        // Push the LCD frame before the ring upload: the cooler acknowledges a
        // frame that arrives while its animation engine is switching and then
        // does not show it.
        if let Some(lcd) = &profile.lcd {
            if self.cooler.connected() {
                self.push_lcd_once(lcd);
            }
        }

        let mut report = self.apply_lighting(profile, true, true, None);
        self.fan_controllers.clear();
        self.lcd_version = self.lcd_version.wrapping_add(1);
        if let Some(lcd) = &profile.lcd {
            report.push(if self.cooler.connected() {
                ApplyResult::ok(HYDROSHIFT_DEVICE_ID, "LCD", "LCD layout scheduled")
            } else {
                ApplyResult::failed(
                    HYDROSHIFT_DEVICE_ID,
                    "LCD",
                    self.cooler
                        .status()
                        .error
                        .unwrap_or_else(|| "Cooler not connected".to_string()),
                )
            });
            let _ = lcd;
        }
        report.push(if self.cooler.connected() {
            ApplyResult::ok(
                HYDROSHIFT_DEVICE_ID,
                "Cooling",
                match &profile.cooling {
                    Some(cooling) if !cooling.curves.is_empty() => {
                        format!("{} curve(s) active", cooling.curves.len())
                    }
                    _ => "Using default (Balanced) curves".to_string(),
                },
            )
        } else {
            ApplyResult::failed(
                HYDROSHIFT_DEVICE_ID,
                "Cooling",
                "Cooler not reachable; curves will be applied when it reconnects",
            )
        });
        for result in report.results.iter().filter(|result| !result.success) {
            self.log(format!(
                "Apply {}/{} failed: {}",
                result.device_id, result.part, result.message
            ));
        }
        self.last_apply = Some(report.clone());
        report
    }

    pub fn apply_lighting(
        &mut self,
        profile: &HardwareProfile,
        force: bool,
        persist: bool,
        only: Option<&str>,
    ) -> ApplyReport {
        let mut report = ApplyReport::empty(profile.name.clone());
        let Some(lighting) = &profile.lighting else {
            return report;
        };
        if force && matches!(only, None | Some(FUSION2_DEVICE_ID)) {
            self.fusion.reopen(false);
        }
        if matches!(only, None | Some(FUSION2_DEVICE_ID)) {
            self.fusion.begin_batch();
        }
        let mut touched_fusion = false;
        let mut zones = self.zones();
        // The cooler waits for its animation engine before it updates the
        // radiator-fan LEDs. Do it last so that wait never delays the headers
        // or the memory modules.
        zones.sort_by_key(|zone| zone.device_id == HYDROSHIFT_DEVICE_ID);
        for zone in zones {
            if let Some(only) = only {
                if zone.device_id != only {
                    continue;
                }
            }
            let Some(requested) = lighting.for_zone(&zone.id) else {
                continue;
            };
            let (fitted, substitute) = fit_lighting(requested, &zone);
            let mut result = match zone.device_id.as_str() {
                FUSION2_DEVICE_ID => {
                    touched_fusion = true;
                    self.fusion.apply(&zone.id, &fitted, zone.led_count)
                }
                CORSAIR_DEVICE_ID => self.ram.apply(&zone.id, &fitted, force),
                HYDROSHIFT_DEVICE_ID => self.cooler.apply_ring(&fitted),
                _ => ApplyResult::failed(&zone.device_id, &zone.name, "Unknown device"),
            };
            result.part = zone.name.clone();
            if let Some(substitute) = substitute {
                result.message = format!(
                    "{} is not available on this zone; {} was used instead. {}",
                    effect_label(requested.effect),
                    effect_label(substitute),
                    result.message
                )
                .trim()
                .to_string();
            }
            report.push(result);
        }
        if touched_fusion {
            report.push(
                self.fusion
                    .commit(persist && self.settings.save_motherboard_lighting_to_flash),
            );
        }
        if let Some(reason) = self.cooler.fan_lighting_rejected.take() {
            self.log(format!("Radiator fan lighting turned off: {reason}"));
            self.settings.cooler_fan_lighting = false;
            let _ = self.settings.save(self.paths.devices());
        }
        report
    }

    pub fn rescan(&mut self) {
        self.ram.rescan();
        self.fusion.reopen(true);
        self.cooler.reconnect();
        self.was_fusion_connected = false;
        self.was_cooler_connected = false;
        self.was_ram_connected = false;
        self.tick_watch();
    }

    // -------------------------------------------------------------- cooling

    pub fn cooling_defaults(&self) -> BTreeMap<String, FanCurve> {
        BTreeMap::from([
            (PUMP_TARGET.to_string(), presets::balanced(TemperatureSource::Coolant)),
            (FANS_TARGET.to_string(), presets::balanced(TemperatureSource::Coolant)),
        ])
    }

    pub fn effective_curves(&self) -> BTreeMap<String, FanCurve> {
        let mut curves = BTreeMap::from([
            (PUMP_TARGET.to_string(), presets::balanced(TemperatureSource::Coolant)),
            (FANS_TARGET.to_string(), presets::balanced(TemperatureSource::Coolant)),
        ]);
        if let Some(cooling) = self.profiles.active().and_then(|profile| profile.cooling.as_ref()) {
            for (target, curve) in &cooling.curves {
                if curve.is_valid() {
                    curves.insert(target.clone(), curve.clone());
                }
            }
        }
        curves
    }

    pub fn cooling_set(&mut self, curves: BTreeMap<String, FanCurve>) -> anyhow::Result<()> {
        for (target, curve) in &curves {
            let problems = curve.validate();
            if !problems.is_empty() {
                anyhow::bail!("Cooling '{}': {}", target, problems.join(" "));
            }
        }
        self.update_active_profile(|profile| {
            profile.cooling = Some(CoolingSettings { curves });
        })?;
        self.fan_controllers.clear();
        Ok(())
    }

    pub fn cooling_restore(&mut self) -> anyhow::Result<()> {
        self.update_active_profile(|profile| profile.cooling = None)?;
        self.fan_controllers.clear();
        Ok(())
    }

    // ------------------------------------------------------------------ LCD

    pub fn lcd_settings(&self) -> LcdSettings {
        self.profiles
            .active()
            .and_then(|profile| profile.lcd.clone())
            .unwrap_or_default()
    }

    pub fn lcd_set(&mut self, settings: LcdSettings) -> anyhow::Result<()> {
        let problems = settings.validate();
        if !problems.is_empty() {
            anyhow::bail!("{}", problems.join(" "));
        }
        self.update_active_profile(|profile| profile.lcd = Some(settings))?;
        self.lcd_version = self.lcd_version.wrapping_add(1);
        self.lcd_next_due = Instant::now();
        Ok(())
    }

    pub fn lcd_themes(&self) -> Vec<mylianpc_lcd::LcdTheme> {
        themes()
    }

    pub fn lcd_push_now(&mut self) -> anyhow::Result<()> {
        if !self.cooler.connected() {
            anyhow::bail!("the cooler is not connected");
        }
        let settings = self.lcd_settings();
        if !self.push_lcd_once(&settings) {
            anyhow::bail!("the panel did not acknowledge the frame");
        }
        self.lcd_version = self.lcd_version.wrapping_add(1);
        self.lcd_next_due = Instant::now();
        Ok(())
    }

    pub fn lcd_preview(&mut self) -> Vec<u8> {
        let settings = self.lcd_settings();
        self.lcd_preview_with(&settings)
    }

    /// Renders an arbitrary (possibly unsaved) LCD layout with the current
    /// sensor readings.
    pub fn lcd_preview_with(&mut self, settings: &LcdSettings) -> Vec<u8> {
        let settings = settings.preview_safe();
        let metrics = self.lcd_metrics();
        let media = if matches!(
            settings.mode,
            mylianpc_core::profile::LcdMode::Image | mylianpc_core::profile::LcdMode::Animation
        ) {
            self.media_for(&settings)
        } else {
            None
        };
        let frame = media.as_ref().and_then(|media| media.frames.first());
        let pixmap = mylianpc_lcd::render(&settings, &metrics, frame, &mut self.fonts);
        let pixmap = mylianpc_lcd::preview_transform(
            &pixmap,
            settings.brightness,
            settings.rotation_degrees,
        );
        encode_png(&pixmap)
    }

    fn lcd_metrics(&self) -> LcdMetrics {
        LcdMetrics::new(self.snapshot.values.clone(), Local::now())
    }

    fn media_for(&mut self, settings: &LcdSettings) -> Option<Arc<LcdMedia>> {
        let file = settings.media_file_name()?.to_string();
        if let Some(media) = self.media_cache.get(&file) {
            return Some(Arc::clone(media));
        }
        let media_dir = self.paths.media();
        let path = media_dir.join(&file);
        if path.parent() != Some(media_dir.as_path()) {
            return None;
        }
        let data = std::fs::read(path).ok()?;
        let media = Arc::new(LcdMedia::load(&data).ok()?);
        self.media_cache.insert(file, Arc::clone(&media));
        Some(media)
    }

    pub fn store_media(&mut self, file_name: &str, data: &[u8]) -> anyhow::Result<String> {
        if data.len() > 25 * 1024 * 1024 {
            anyhow::bail!("Files up to 25 MB are supported.");
        }
        let extension = PathBuf::from(file_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !mylianpc_lcd::media::SUPPORTED_EXTENSIONS.contains(&extension.as_str()) {
            anyhow::bail!("Unsupported file type '.{extension}'. Use PNG, JPEG, BMP, GIF or WebP.");
        }
        LcdMedia::load(data).map_err(anyhow::Error::msg)?;
        let name = format!(
            "{}-{}.{}",
            Utc::now().timestamp_millis(),
            data.len(),
            extension
        );
        let path = self.paths.media().join(&name);
        std::fs::write(&path, data)?;
        Ok(name)
    }

    pub fn media_files(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.paths.media()) else {
            return Vec::new();
        };
        let mut files: Vec<String> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .collect();
        files.sort();
        files
    }

    fn push_lcd_once(&mut self, settings: &LcdSettings) -> bool {
        self.cooler
            .set_lcd_brightness(if settings.mode == mylianpc_core::profile::LcdMode::Off {
                0
            } else {
                settings.brightness
            });
        self.cooler.set_lcd_rotation(settings.rotation_degrees);
        if !self.cooler.rearm_lcd() {
            return false;
        }
        let metrics = self.lcd_metrics();
        let media = if matches!(
            settings.mode,
            mylianpc_core::profile::LcdMode::Image | mylianpc_core::profile::LcdMode::Animation
        ) {
            self.media_for(settings)
        } else {
            None
        };
        let frame = media
            .as_ref()
            .and_then(|media| media.frames.first());
        let pixmap = mylianpc_lcd::render(settings, &metrics, frame, &mut self.fonts);
        let jpeg = encode_jpeg(&pixmap, 150_000);
        if !jpeg.is_empty() && self.cooler.show_jpeg(&jpeg) {
            self.log("LCD: frame acknowledged by the panel");
            true
        } else {
            self.log("LCD: the panel did not acknowledge the frame");
            false
        }
    }

    fn tick_lcd(&mut self) {
        if !self.cooler.connected() {
            self.lcd_shown_generation = 0;
            return;
        }
        let settings = self.lcd_settings();
        let generation = self.cooler.generation();
        if self.lcd_version != self.lcd_shown_version
            || self.lcd_shown_generation != generation
        {
            self.log(format!(
                "LCD: showing {}",
                if settings.mode == mylianpc_core::profile::LcdMode::Theme {
                    settings.theme_id.clone()
                } else {
                    format!("{:?}", settings.mode)
                }
            ));
            self.cooler
                .set_lcd_brightness(if settings.mode == mylianpc_core::profile::LcdMode::Off {
                    0
                } else {
                    settings.brightness
                });
            self.cooler.set_lcd_rotation(settings.rotation_degrees);
            self.lcd_shown_version = self.lcd_version;
            self.lcd_shown_generation = generation;
            self.lcd_last_jpeg = None;
            self.lcd_frame = 0;
            self.lcd_next_due = Instant::now();
            // The ring animation was uploaded just before this loop saw the
            // new version; send one more frame once it has settled.
            self.lcd_repush_at = Some(Instant::now() + Duration::from_secs(2));
        }
        if Instant::now() < self.lcd_next_due {
            return;
        }
        if self.cooler.lighting_upload_in_progress() {
            self.lcd_next_due = Instant::now() + Duration::from_millis(100);
            return;
        }

        let media = if matches!(
            settings.mode,
            mylianpc_core::profile::LcdMode::Image | mylianpc_core::profile::LcdMode::Animation
        ) {
            self.media_for(&settings)
        } else {
            None
        };
        let frame_index = self.lcd_frame;
        let media_frame = media.as_ref().and_then(|media| {
            if media.frames.is_empty() {
                None
            } else {
                Some(&media.frames[frame_index % media.frames.len()])
            }
        });
        let metrics = self.lcd_metrics();
        let pixmap = mylianpc_lcd::render(&settings, &metrics, media_frame, &mut self.fonts);
        let jpeg = encode_jpeg(&pixmap, 150_000);
        if jpeg.is_empty() {
            self.lcd_next_due = Instant::now() + Duration::from_secs(1);
            return;
        }
        let due = self.lcd_last_jpeg.as_ref().map(|last| last != &jpeg).unwrap_or(true)
            || self
                .lcd_last_push
                .map(|at| at.elapsed() > Duration::from_secs(30))
                .unwrap_or(true)
            || self
                .lcd_repush_at
                .map(|at| Instant::now() >= at)
                .unwrap_or(false);
        if due {
            if self.cooler.show_jpeg(&jpeg) {
                self.lcd_last_jpeg = Some(jpeg);
                self.lcd_last_push = Some(Instant::now());
                if self
                    .lcd_repush_at
                    .map(|at| Instant::now() >= at)
                    .unwrap_or(false)
                {
                    self.lcd_repush_at = None;
                }
            } else {
                self.log("LCD: the panel did not acknowledge the frame");
            }
        }

        let delay = match &media {
            Some(media) if media.is_animated() && !media.durations_ms.is_empty() => {
                media.durations_ms[frame_index % media.durations_ms.len()]
            }
            _ if settings.mode == mylianpc_core::profile::LcdMode::Off => 5000,
            _ => settings.refresh_milliseconds,
        };
        if media.as_ref().map(|media| media.is_animated()).unwrap_or(false) {
            self.lcd_frame = self.lcd_frame.wrapping_add(1);
        }
        self.lcd_next_due = Instant::now() + Duration::from_millis(delay);
    }

    // -------------------------------------------------------------- settings

    pub fn settings_get(&self) -> DeviceSettings {
        self.settings.clone()
    }

    pub fn settings_update(&mut self, settings: DeviceSettings) -> anyhow::Result<()> {
        let previous_fan_lighting = self.settings.cooler_fan_lighting;
        self.settings = settings.normalized();
        self.settings.save(self.paths.devices())?;
        if self.settings.cooler_fan_lighting != previous_fan_lighting {
            self.cooler.fan_lighting = self.settings.cooler_fan_lighting;
            if let Some(profile) = self.profiles.active().cloned() {
                if profile.lighting.is_some() {
                    self.apply_lighting(&profile, true, false, Some(HYDROSHIFT_DEVICE_ID));
                }
            }
        }
        if let Some(profile) = self.profiles.active().cloned() {
            if profile.lighting.is_some() {
                self.apply_lighting(&profile, false, false, Some(FUSION2_DEVICE_ID));
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- status

    pub fn status(&self) -> ServiceStatus {
        let mut warnings = Vec::new();
        if !self.fusion.connected() {
            warnings.push("The motherboard RGB controller is not connected.".to_string());
        }
        if !self.cooler.connected() {
            warnings.push("The HydroShift II cooler is not connected.".to_string());
        }
        if self.ram.modules().is_empty() {
            warnings.push("No Corsair DRAM RGB modules were found on the SMBus.".to_string());
        }
        ServiceStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_utc: self.started_utc,
            devices: vec![
                self.cooler.status(),
                self.fusion.status(&self.settings.zone_led_counts),
                self.ram.status(),
                self.sensors_status(),
            ],
            active_profile_id: self.profiles.active().map(|profile| profile.id),
            active_profile_name: self.profiles.active().map(|profile| profile.name.clone()),
            last_apply: self.last_apply.clone(),
            warnings,
            device_settings: self.settings.clone(),
        }
    }

    fn sensors_status(&self) -> DeviceStatus {
        let mut status = DeviceStatus::placeholder(
            "sensors",
            "Hardware sensors",
            "Motherboard",
            "Sensors and fan headers",
        );
        status.state = if self.snapshot.value(metrics::CPU_TEMP).is_some() {
            DeviceState::Connected
        } else {
            DeviceState::Error
        };
        status.location = Some("hwmon, /proc, NVML, RAPL".to_string());
        status.capabilities = vec![
            "CPU/GPU temperature, load, clock, power".to_string(),
            "Memory usage, temperature, speed and EXPO/XMP".to_string(),
            "Motherboard and NVMe temperatures".to_string(),
            "NVMe remaining life".to_string(),
        ];
        status.dependencies = vec!["Linux hwmon and NVML".to_string()];
        if !inventory_has_it87() {
            status.warnings.push(
                "The it87 driver is not loaded: motherboard fan headers are unavailable on this Linux build."
                    .to_string(),
            );
        }
        status.last_contact_utc = Some(self.snapshot.time_utc);
        status
    }

    pub fn diagnostics(&self) -> String {
        let mut text = String::new();
        text.push_str("MyLianPC Linux diagnostics\n");
        text.push_str(&format!("Version: {}\n", env!("CARGO_PKG_VERSION")));
        text.push_str(&format!("Started: {}\n", self.started_utc));
        text.push_str(&format!("Data folder: {}\n", self.paths.root.display()));
        text.push_str(&format!(
            "Active profile: {}\n\n",
            self.profiles
                .active()
                .map(|profile| profile.name.as_str())
                .unwrap_or("(none)")
        ));
        for device in self.status().devices {
            text.push_str(&format!(
                "{}: {} ({})\n",
                device.name,
                device.state.label(),
                device.id
            ));
            if let Some(error) = device.error {
                text.push_str(&format!("  error: {error}\n"));
            }
            for zone in device.zones {
                text.push_str(&format!(
                    "  zone {} · {} LEDs · {} effects\n",
                    zone.name,
                    zone.led_count,
                    zone.effects.len()
                ));
            }
            for fan in device.fans {
                text.push_str(&format!(
                    "  fan {} · {} rpm\n",
                    fan.name,
                    fan.rpm.map(|rpm| format!("{rpm:.0}")).unwrap_or_else(|| "—".to_string())
                ));
            }
        }
        text.push_str("\nSensors\n");
        for reading in &self.snapshot.readings {
            if let Some(value) = reading.value {
                text.push_str(&format!(
                    "  {:<22} {:>8.2} {}\n",
                    reading.label, value, reading.unit
                ));
            }
        }
        text.push_str("\nRecent log\n");
        for line in self.logs.iter().rev().take(80).rev() {
            text.push_str(&format!("  {line}\n"));
        }
        text
    }
}

pub fn pump_rpm_for_duty(duty: i32) -> i32 {
    1600 + duty.clamp(0, 100) * 9
}

fn inventory_has_it87() -> bool {
    mylianpc_hardware::inventory::hwmon_devices()
        .iter()
        .any(|device| device.name.starts_with("it87"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pump_duty_maps_to_the_1600_2500_range() {
        assert_eq!(pump_rpm_for_duty(0), 1600);
        assert_eq!(pump_rpm_for_duty(50), 2050);
        assert_eq!(pump_rpm_for_duty(100), 2500);
        assert_eq!(pump_rpm_for_duty(999), 2500);
    }
}
