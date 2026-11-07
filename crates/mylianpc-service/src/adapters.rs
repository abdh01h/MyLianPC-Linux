//! Device adapters: the only code paths that turn a profile into hardware
//! commands. Ported from `MyLianPC.Hardware/Adapters/*.cs`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use mylianpc_core::color::RgbColor;
use mylianpc_core::device::{
    ApplyResult, DeviceState, DeviceStatus, FanChannelInfo, ZoneInfo,
};
use mylianpc_core::effect::{self, Animation, LightingEffect, ZoneLighting};
use mylianpc_hardware::corsair::{
    self, CorsairDram, CorsairHealth, CorsairMode,
};
use mylianpc_hardware::fusion2::{
    Fusion2Controller, Fusion2Effect, Fusion2Zone, B850_ZONES,
};
use mylianpc_hardware::hydroshift2::{self as hs, Telemetry};
use mylianpc_hardware::hydroshift_device::{HydroShiftDevice, HydroShiftError};
use mylianpc_hardware::inventory;
use mylianpc_hardware::smbus::Smbus;

pub const FUSION2_DEVICE_ID: &str = "gigabyte-fusion2";
pub const HYDROSHIFT_DEVICE_ID: &str = "lianli-hydroshift2";
pub const CORSAIR_DEVICE_ID: &str = "corsair-dram";

pub const RING_ZONE_ID: &str = "hydroshift:ring";
pub const PUMP_TARGET: &str = "hydroshift:pump";
pub const FANS_TARGET: &str = "hydroshift:fans";

const ALL_EFFECTS: &[LightingEffect] = &[
    LightingEffect::Off,
    LightingEffect::Static,
    LightingEffect::Breathing,
    LightingEffect::Flash,
    LightingEffect::ColorCycle,
    LightingEffect::Rainbow,
    LightingEffect::RainbowWave,
    LightingEffect::Wave,
    LightingEffect::Gradient,
];

pub fn effect_label(effect: LightingEffect) -> &'static str {
    match effect {
        LightingEffect::Off => "Off",
        LightingEffect::Static => "Static",
        LightingEffect::Breathing => "Breathing",
        LightingEffect::Flash => "Flash",
        LightingEffect::ColorCycle => "Color cycle",
        LightingEffect::Rainbow => "Rainbow",
        LightingEffect::RainbowWave => "Rainbow wave",
        LightingEffect::Wave => "Wave",
        LightingEffect::Gradient => "Gradient",
    }
}

// ---------------------------------------------------------------------------
// GIGABYTE Fusion2
// ---------------------------------------------------------------------------

struct ZoneAnimation {
    animation: Animation,
    frame: usize,
    next: Instant,
}

pub struct Fusion2Adapter {
    controller: Option<Fusion2Controller>,
    state: DeviceState,
    error: Option<String>,
    last_contact: Option<DateTime<Utc>>,
    next_attempt: Instant,
    animations: HashMap<String, ZoneAnimation>,
    direct_active: bool,
    firmware: Option<String>,
    description: Option<String>,
}

impl Default for Fusion2Adapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Fusion2Adapter {
    pub fn new() -> Self {
        Self {
            controller: None,
            state: DeviceState::Disconnected,
            error: None,
            last_contact: None,
            next_attempt: Instant::now(),
            animations: HashMap::new(),
            direct_active: false,
            firmware: None,
            description: None,
        }
    }

    pub fn connected(&self) -> bool {
        self.state == DeviceState::Connected && self.controller.is_some()
    }

    pub fn animating(&self) -> bool {
        !self.animations.is_empty()
    }

    pub fn next_animation_delay(&self) -> Duration {
        if self.animations.is_empty() {
            return Duration::from_millis(1000);
        }
        let next = self
            .animations
            .values()
            .map(|animation| animation.next)
            .min()
            .unwrap_or_else(Instant::now);
        next.saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(5), Duration::from_millis(250))
    }

    pub fn ensure_connected(&mut self) -> bool {
        if self.connected() {
            return true;
        }
        if Instant::now() < self.next_attempt {
            return false;
        }
        match Fusion2Controller::open() {
            Ok(mut controller) => {
                if let Err(error) = controller.initialize() {
                    self.fail(DeviceState::Error, error.to_string());
                    return false;
                }
                self.firmware = Some(controller.info().firmware.clone());
                self.description = Some(controller.info().description.clone());
                self.controller = Some(controller);
                self.state = DeviceState::Connected;
                self.error = None;
                self.last_contact = Some(Utc::now());
                true
            }
            Err(error) => {
                self.fail(DeviceState::Disconnected, format!("{error:#}"));
                false
            }
        }
    }

    fn fail(&mut self, state: DeviceState, error: String) {
        self.state = state;
        self.error = Some(error);
        self.controller = None;
        self.animations.clear();
        self.next_attempt = Instant::now() + Duration::from_secs(10);
    }

    pub fn zones(&self, led_counts: &BTreeMap<String, usize>) -> Vec<ZoneInfo> {
        B850_ZONES
            .iter()
            .map(|zone| ZoneInfo {
                id: zone.id.to_string(),
                device_id: FUSION2_DEVICE_ID.to_string(),
                name: zone.name.to_string(),
                led_count: if zone.addressable {
                    led_counts
                        .get(zone.id)
                        .copied()
                        .unwrap_or(32)
                        .clamp(1, 256)
                } else {
                    1
                },
                effects: ALL_EFFECTS.to_vec(),
                max_colors: if zone.addressable { 6 } else { 1 },
                has_brightness: true,
                has_speed: true,
                led_count_configurable: zone.addressable,
                note: zone.addressable.then(|| {
                    "Gradient, rainbow, rainbow wave and wave are drawn per LED; set the LED count. \
                     They are re-applied at every start and are not stored in flash."
                        .to_string()
                }),
            })
            .collect()
    }

    fn zone(zone_id: &str) -> Option<&'static Fusion2Zone> {
        B850_ZONES.iter().find(|zone| zone.id == zone_id)
    }

    pub fn begin_batch(&mut self) {
        self.animations.clear();
        if let Some(controller) = self.controller.as_mut() {
            if let Err(error) = controller.begin_batch() {
                self.fail(DeviceState::Error, error.to_string());
            }
        }
    }

    pub fn apply(&mut self, zone_id: &str, lighting: &ZoneLighting, led_count: usize) -> ApplyResult {
        let Some(zone) = Self::zone(zone_id) else {
            return ApplyResult::failed(FUSION2_DEVICE_ID, zone_id, "Unknown zone.");
        };
        if !self.ensure_connected() {
            return ApplyResult::failed(
                FUSION2_DEVICE_ID,
                zone.name,
                self.error.clone().unwrap_or_else(|| "Not connected.".to_string()),
            );
        }
        let result = self.apply_inner(zone, lighting, led_count);
        match result {
            Ok(()) => {
                self.last_contact = Some(Utc::now());
                ApplyResult::ok(
                    FUSION2_DEVICE_ID,
                    zone.name,
                    format!("{} applied", effect_label(lighting.effect)),
                )
            }
            Err(error) => {
                self.fail(DeviceState::Error, error);
                ApplyResult::failed(
                    FUSION2_DEVICE_ID,
                    zone.name,
                    self.error.clone().unwrap_or_default(),
                )
            }
        }
    }

    fn apply_inner(
        &mut self,
        zone: &Fusion2Zone,
        lighting: &ZoneLighting,
        led_count: usize,
    ) -> Result<(), String> {
        let controller = self.controller.as_mut().ok_or("not connected")?;
        let color = lighting.colors.first().copied().unwrap_or(RgbColor::WHITE);
        match lighting.effect {
            LightingEffect::Off => controller
                .set_effect(zone, Fusion2Effect::Static, RgbColor::BLACK, 0, 0, false)
                .map_err(|error| error.to_string())?,
            LightingEffect::Static => controller
                .set_effect(
                    zone,
                    Fusion2Effect::Static,
                    color,
                    lighting.brightness,
                    lighting.speed,
                    false,
                )
                .map_err(|error| error.to_string())?,
            LightingEffect::Breathing => controller
                .set_effect(
                    zone,
                    Fusion2Effect::Pulse,
                    color,
                    lighting.brightness,
                    lighting.speed,
                    false,
                )
                .map_err(|error| error.to_string())?,
            LightingEffect::Flash => controller
                .set_effect(
                    zone,
                    Fusion2Effect::Blinking,
                    color,
                    lighting.brightness,
                    lighting.speed,
                    false,
                )
                .map_err(|error| error.to_string())?,
            LightingEffect::ColorCycle => controller
                .set_effect(
                    zone,
                    Fusion2Effect::ColorCycle,
                    color,
                    lighting.brightness,
                    lighting.speed,
                    false,
                )
                .map_err(|error| error.to_string())?,
            LightingEffect::Rainbow | LightingEffect::RainbowWave | LightingEffect::Wave
                if zone.addressable =>
            {
                let animation = effect::generate(lighting, led_count.clamp(1, 256));
                let interval = animation.frame_interval_ms.clamp(40, 250);
                controller
                    .set_direct(zone, &animation.frames[0])
                    .map_err(|error| error.to_string())?;
                self.direct_active = true;
                self.animations.insert(
                    zone.id.to_string(),
                    ZoneAnimation {
                        animation: Animation {
                            frames: animation.frames,
                            frame_interval_ms: interval,
                        },
                        frame: 1,
                        next: Instant::now() + Duration::from_millis(interval as u64),
                    },
                );
            }
            LightingEffect::Rainbow | LightingEffect::RainbowWave | LightingEffect::Wave => {
                controller
                    .set_effect(
                        zone,
                        Fusion2Effect::ColorCycle,
                        color,
                        lighting.brightness,
                        lighting.speed,
                        false,
                    )
                    .map_err(|error| error.to_string())?;
            }
            LightingEffect::Gradient if zone.addressable => {
                let animation = effect::generate(lighting, led_count.clamp(1, 256));
                controller
                    .set_direct(zone, &animation.frames[0])
                    .map_err(|error| error.to_string())?;
                self.direct_active = true;
            }
            LightingEffect::Gradient => {
                controller
                    .set_effect(
                        zone,
                        Fusion2Effect::Static,
                        color,
                        lighting.brightness,
                        lighting.speed,
                        false,
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    pub fn commit(&mut self, save_to_flash: bool) -> ApplyResult {
        if !self.ensure_connected() {
            return ApplyResult::failed(
                FUSION2_DEVICE_ID,
                "Commit",
                self.error.clone().unwrap_or_else(|| "Not connected.".to_string()),
            );
        }
        let Some(controller) = self.controller.as_mut() else {
            return ApplyResult::failed(FUSION2_DEVICE_ID, "Commit", "Not connected.");
        };
        if let Err(error) = controller.apply() {
            self.fail(DeviceState::Error, error.to_string());
            return ApplyResult::failed(FUSION2_DEVICE_ID, "Commit", error.to_string());
        }
        let saved = save_to_flash && !self.direct_active;
        if saved {
            if let Err(error) = controller.save_to_flash() {
                self.fail(DeviceState::Error, error.to_string());
                return ApplyResult::failed(FUSION2_DEVICE_ID, "Commit", error.to_string());
            }
        }
        self.direct_active = false;
        ApplyResult::ok(
            FUSION2_DEVICE_ID,
            "Motherboard lighting",
            if saved {
                "Applied and saved to controller flash"
            } else {
                "Applied"
            },
        )
    }

    pub fn render_animations(&mut self) {
        if self.animations.is_empty() {
            return;
        }
        let Some(controller) = self.controller.as_mut() else {
            return;
        };
        let now = Instant::now();
        let due: Vec<String> = self
            .animations
            .iter()
            .filter(|(_, state)| now >= state.next)
            .map(|(id, _)| id.clone())
            .collect();
        for zone_id in due {
            let Some(zone) = Self::zone(&zone_id) else {
                self.animations.remove(&zone_id);
                continue;
            };
            let Some(state) = self.animations.get(&zone_id) else {
                continue;
            };
            let frames = &state.animation.frames;
            let frame = frames[state.frame % frames.len()].clone();
            let interval = state.animation.frame_interval_ms;
            if let Err(error) = controller.set_direct(zone, &frame) {
                self.fail(DeviceState::Error, error.to_string());
                return;
            }
            if let Some(state) = self.animations.get_mut(&zone_id) {
                state.frame = state.frame.wrapping_add(1);
                state.next = now + Duration::from_millis(interval as u64);
            }
        }
    }

    pub fn reopen(&mut self, full_reset: bool) {
        self.controller = None;
        self.state = DeviceState::Disconnected;
        self.next_attempt = Instant::now();
        if !full_reset {
            if let Ok(mut controller) = Fusion2Controller::open() {
                if controller.reassert().is_ok() {
                    self.firmware = Some(controller.info().firmware.clone());
                    self.description = Some(controller.info().description.clone());
                    self.controller = Some(controller);
                    self.state = DeviceState::Connected;
                    self.error = None;
                    self.last_contact = Some(Utc::now());
                    return;
                }
            }
        }
        self.ensure_connected();
    }

    pub fn status(&self, led_counts: &BTreeMap<String, usize>) -> DeviceStatus {
        let name = self
            .description
            .as_deref()
            .map(|description| format!("RGB Fusion 2 ({description})"))
            .unwrap_or_else(|| "RGB Fusion 2".to_string());
        let mut status =
            DeviceStatus::placeholder(FUSION2_DEVICE_ID, &name, "GIGABYTE", "Motherboard lighting");
        status.state = self.state;
        status.firmware = self.firmware.clone();
        status.location = Some("USB HID 048D:5711".to_string());
        status.zones = self.zones(led_counts);
        status.capabilities = vec![
            "3 ARGB headers (case lighting and fans)".to_string(),
            "Chipset accent".to_string(),
            "LED_C".to_string(),
            "Hardware effects stored in flash".to_string(),
        ];
        status.dependencies = vec!["None (HID, works for standard users)".to_string()];
        status.warnings = vec![
            "GIGABYTE Control Center and Windows Dynamic Lighting cannot run on Linux; nothing else should repaint the controller."
                .to_string(),
        ];
        status.error = self.error.clone();
        status.troubleshooting = (self.state == DeviceState::Error)
            .then(|| "Click Rescan. If it keeps failing, re-plug the board's USB header.".to_string());
        status.last_contact_utc = self.last_contact;
        status
    }
}

// ---------------------------------------------------------------------------
// Lian Li HydroShift II
// ---------------------------------------------------------------------------

struct PendingFan {
    frames: Vec<Vec<u8>>,
    interval: u8,
    next: Instant,
    remaining: u8,
}

pub struct HydroShiftAdapter {
    device: Option<HydroShiftDevice>,
    state: DeviceState,
    error: Option<String>,
    next_attempt: Instant,
    last_contact: Option<DateTime<Utc>>,
    telemetry: Option<Telemetry>,
    generation: u64,
    pub fan_lighting: bool,
    pub fan_lighting_rejected: Option<String>,
    lighting_uploads: u32,
    firmware: Option<String>,
    last_pump_rpm: Option<i32>,
    last_fan_percent: Option<i32>,
    last_cooling_write: Option<Instant>,
    pending_fan: Option<PendingFan>,
}

impl Default for HydroShiftAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl HydroShiftAdapter {
    pub fn new() -> Self {
        Self {
            device: None,
            state: DeviceState::Disconnected,
            error: None,
            next_attempt: Instant::now(),
            last_contact: None,
            telemetry: None,
            generation: 0,
            fan_lighting: false,
            fan_lighting_rejected: None,
            lighting_uploads: 0,
            firmware: None,
            last_pump_rpm: None,
            last_fan_percent: None,
            last_cooling_write: None,
            pending_fan: None,
        }
    }

    pub fn connected(&self) -> bool {
        self.state == DeviceState::Connected && self.device.is_some()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn telemetry(&self) -> Option<&Telemetry> {
        self.telemetry.as_ref()
    }

    pub fn lighting_upload_in_progress(&self) -> bool {
        self.lighting_uploads > 0
    }

    pub fn ensure_connected(&mut self) -> bool {
        if self.connected() && !self.device.as_ref().map(|device| device.unresponsive()).unwrap_or(true) {
            return true;
        }
        if Instant::now() < self.next_attempt {
            return false;
        }
        match HydroShiftDevice::open() {
            Ok(mut device) => match device.initialize() {
                Ok(()) => {
                    self.firmware = device.firmware().map(str::to_string);
                    self.telemetry = device.read_telemetry().ok().flatten();
                    self.device = Some(device);
                    self.state = DeviceState::Connected;
                    self.error = None;
                    self.last_contact = Some(Utc::now());
                    self.last_pump_rpm = None;
                    self.last_fan_percent = None;
                    self.generation = self.generation.wrapping_add(1);
                    true
                }
                Err(error) => {
                    self.fail(error);
                    false
                }
            },
            Err(HydroShiftError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ResourceBusy | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                self.fail_state(
                    DeviceState::Busy,
                    format!("The cooler is in use by another program or not accessible: {error}"),
                    Duration::from_secs(15),
                );
                false
            }
            Err(error) => {
                self.fail(error);
                false
            }
        }
    }

    fn fail(&mut self, error: HydroShiftError) {
        let state = match error {
            HydroShiftError::Unresponsive => DeviceState::Unresponsive,
            _ => DeviceState::Error,
        };
        self.fail_state(state, error.to_string(), Duration::from_secs(15));
    }

    fn fail_state(&mut self, state: DeviceState, error: String, retry: Duration) {
        self.state = state;
        self.error = Some(error);
        self.device = None;
        self.pending_fan = None;
        self.next_attempt = Instant::now() + retry;
    }

    pub fn reconnect(&mut self) {
        self.device = None;
        self.pending_fan = None;
        self.state = DeviceState::Disconnected;
        self.next_attempt = Instant::now();
    }

    /// Re-sends an acknowledged combined ring+fans packet after the firmware's
    /// animation switch has settled. Called from the fast service loop so the
    /// apply request does not block the UI for seconds.
    pub fn service_pending_fan(&mut self) {
        let Some(pending) = self.pending_fan.as_mut() else {
            return;
        };
        if Instant::now() < pending.next {
            return;
        }
        let Some(device) = self.device.as_mut() else {
            self.pending_fan = None;
            return;
        };
        let ok = device
            .set_ring_with_retry(&pending.frames, pending.interval, false)
            .unwrap_or(false);
        eprintln!(
            "HydroShift: fan refresh ack={ok} ({} remaining)",
            pending.remaining.saturating_sub(1)
        );
        if ok && pending.remaining > 1 {
            pending.remaining -= 1;
            pending.next = Instant::now() + Duration::from_millis(800);
        } else {
            self.pending_fan = None;
        }
    }

    fn with_device<T>(&mut self, operation: impl FnOnce(&mut HydroShiftDevice) -> Result<T, HydroShiftError>) -> Option<T> {
        if !self.ensure_connected() {
            return None;
        }
        let result = self.device.as_mut().map(operation);
        match result {
            Some(Ok(value)) => {
                self.last_contact = Some(Utc::now());
                Some(value)
            }
            Some(Err(error)) => {
                self.fail(error);
                None
            }
            None => None,
        }
    }

    pub fn ring_zone(&self) -> ZoneInfo {
        ZoneInfo {
            id: RING_ZONE_ID.to_string(),
            device_id: HYDROSHIFT_DEVICE_ID.to_string(),
            name: "Pump head ring".to_string(),
            led_count: hs::RING_LED_COUNT,
            effects: ALL_EFFECTS.to_vec(),
            max_colors: 6,
            has_brightness: true,
            has_speed: true,
            led_count_configurable: false,
            note: Some("Animations are uploaded once and looped by the cooler itself.".to_string()),
        }
    }

    pub fn apply_ring(&mut self, lighting: &ZoneLighting) -> ApplyResult {
        if !self.ensure_connected() {
            return ApplyResult::failed(
                HYDROSHIFT_DEVICE_ID,
                "Pump ring lighting",
                self.error.clone().unwrap_or_else(|| "Cooler not connected.".to_string()),
            );
        }
        // The fan count comes from telemetry. Refresh it if the cache is empty
        // so a slow first reply cannot silently skip the fan upload.
        if self.telemetry.is_none() {
            let _ = self.poll_telemetry();
        }
        let animation = effect::generate(lighting, hs::RING_LED_COUNT);
        let frames: Vec<Vec<u8>> = animation
            .frames
            .iter()
            .map(|frame| {
                frame
                    .iter()
                    .flat_map(|color| [color.r, color.g, color.b])
                    .collect()
            })
            .collect();
        self.lighting_uploads += 1;
        let result = (|| {
            // Stop the running animation first; otherwise the firmware
            // acknowledges a new PushRgb without showing it.
            if let Some(device) = self.device.as_mut() {
                device.try_wake();
            }
            let interval = animation.frame_interval_ms.clamp(1, 255) as u8;
            let ring_ok = self
                .device
                .as_mut()
                .map(|device| device.set_ring(&frames, interval).unwrap_or(false))
                .unwrap_or(false);
            let ring_error = self
                .error
                .clone()
                .unwrap_or_else(|| "The cooler did not acknowledge the ring upload.".to_string());

            let fans = self
                .telemetry
                .as_ref()
                .and_then(|telemetry| telemetry.lighting_fan_count)
                .filter(|count| (1..=4).contains(count))
                .unwrap_or(0);
            if fans == 0 {
                eprintln!("HydroShift: ring ack={ring_ok}, no lighting fans reported");
                return (ring_ok, ring_error, None);
            }
            if ring_ok {
                // Give the animation engine time to switch away from the
                // ring-only upload before the combined ring+fans packet.
                let settle = (animation.frame_interval_ms * 2).clamp(800, 1200);
                std::thread::sleep(Duration::from_millis(settle as u64));
            }
            if !self.fan_lighting {
                let (off_frames, off_interval) = with_fans(&animation, fans, false);
                let fans_off = self
                    .device
                    .as_mut()
                    .map(|device| {
                        device
                            .set_ring_with_retry(&off_frames, off_interval, false)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                eprintln!("HydroShift: ring ack={ring_ok}, fan-off ack={fans_off} for {fans} fans");
                return (
                    ring_ok,
                    ring_error.clone(),
                    Some(if fans_off {
                        format!("{} applied to the ring; radiator fan LEDs off", effect_label(lighting.effect))
                    } else {
                        format!("{} applied", effect_label(lighting.effect))
                    }),
                );
            }
            let (fan_frames, fan_interval) = with_fans(&animation, fans, true);
            let fans_ok = self
                .device
                .as_mut()
                .map(|device| {
                    device
                        .set_ring_with_retry(&fan_frames, fan_interval, false)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if fans_ok {
                // The firmware sometimes acknowledges a combined upload that
                // arrives mid-switch and then ignores it. Queue two more
                // sends for the fast loop instead of blocking this apply.
                self.pending_fan = Some(PendingFan {
                    frames: fan_frames,
                    interval: fan_interval,
                    next: Instant::now() + Duration::from_millis(800),
                    remaining: 2,
                });
            }
            eprintln!(
                "HydroShift: ring ack={ring_ok}, fan ack={fans_ok} for {fans} fans"
            );
            if fans_ok {
                (
                    true,
                    ring_error,
                    Some(format!(
                        "{} applied to the ring and {fans} radiator fans",
                        effect_label(lighting.effect)
                    )),
                )
            } else {
                self.fan_lighting = false;
                self.fan_lighting_rejected =
                    Some(format!("The cooler did not accept lighting for {fans} fans."));
                (
                    ring_ok,
                    ring_error.clone(),
                    Some(if ring_ok {
                        format!("{} applied", effect_label(lighting.effect))
                    } else {
                        ring_error
                    }),
                )
            }
        })();
        self.lighting_uploads = self.lighting_uploads.saturating_sub(1);
        let (ok, message, extra) = result;
        ApplyResult {
            device_id: HYDROSHIFT_DEVICE_ID.to_string(),
            part: "Pump ring lighting".to_string(),
            success: ok,
            message: extra.unwrap_or(message),
        }
    }

    pub fn rearm_lcd(&mut self) -> bool {
        self.with_device(|device| Ok(device.try_wake())).unwrap_or(false)
    }

    pub fn show_jpeg(&mut self, jpeg: &[u8]) -> bool {
        self.with_device(|device| device.show_jpeg(jpeg))
            .unwrap_or(false)
    }

    pub fn set_lcd_brightness(&mut self, percent: i32) -> bool {
        self.with_device(|device| device.set_brightness(percent))
            .unwrap_or(false)
    }

    pub fn set_lcd_rotation(&mut self, degrees: i32) -> bool {
        self.with_device(|device| device.set_rotation(degrees / 90))
            .unwrap_or(false)
    }

    pub fn poll_telemetry(&mut self) -> Option<Telemetry> {
        let telemetry = self
            .with_device(|device| device.read_telemetry())
            .flatten();
        if let Some(telemetry) = telemetry.clone() {
            self.telemetry = Some(telemetry);
        }
        telemetry
    }

    /// Pump RPM (1600-2500) and one duty for all three radiator fans. The
    /// setpoint is repeated every five seconds because the firmware falls back
    /// to its own curve after about ten seconds.
    pub fn set_cooling(&mut self, pump_rpm: i32, fan_percent: i32) -> bool {
        let changed = self.last_pump_rpm != Some(pump_rpm)
            || self.last_fan_percent != Some(fan_percent);
        if !changed
            && self
                .last_cooling_write
                .map(|at| at.elapsed() < Duration::from_secs(5))
                .unwrap_or(false)
        {
            return true;
        }
        let sent = self
            .with_device(|device| {
                device.set_pump_and_fans(pump_rpm, fan_percent, fan_percent, fan_percent, 20)
            })
            .unwrap_or(false);
        if sent {
            self.last_pump_rpm = Some(pump_rpm);
            self.last_fan_percent = Some(fan_percent);
            self.last_cooling_write = Some(Instant::now());
        }
        sent
    }

    pub fn safe_defaults(&mut self) {
        self.last_pump_rpm = None;
        self.last_fan_percent = None;
        if let Some(device) = self.device.as_mut() {
            let _ = device.safe_defaults();
        }
    }

    pub fn status(&self) -> DeviceStatus {
        let mut status = DeviceStatus::placeholder(
            HYDROSHIFT_DEVICE_ID,
            "HydroShift II LCD-C",
            "Lian Li",
            "AIO cooler",
        );
        status.state = self.state;
        status.firmware = self.firmware.clone();
        status.location = Some("USB 1CBE:A021".to_string());
        status.has_lcd = true;
        status.zones = vec![self.ring_zone()];
        let telemetry = self.telemetry.as_ref();
        status.fans = vec![
            FanChannelInfo {
                id: PUMP_TARGET.to_string(),
                name: "Pump".to_string(),
                controller: "HydroShift II pump head".to_string(),
                rpm: telemetry.map(|telemetry| telemetry.pump_rpm as f64),
                duty_percent: None,
                controllable: true,
                control_enabled: true,
                note: Some("1600-2500 RPM".to_string()),
            },
            FanChannelInfo {
                id: FANS_TARGET.to_string(),
                name: "Radiator fans (3)".to_string(),
                controller: "HydroShift II pump head".to_string(),
                rpm: telemetry.map(|telemetry| {
                    let values: Vec<f64> = telemetry
                        .fan_rpm
                        .iter()
                        .filter(|rpm| **rpm > 0)
                        .map(|rpm| *rpm as f64)
                        .collect();
                    if values.is_empty() {
                        0.0
                    } else {
                        values.iter().sum::<f64>() / values.len() as f64
                    }
                }),
                duty_percent: None,
                controllable: true,
                control_enabled: true,
                note: None,
            },
        ];
        status.capabilities = vec![
            "LCD 480×480 (JPEG frames)".to_string(),
            "Pump speed".to_string(),
            "Radiator fan speed".to_string(),
            "Coolant temperature".to_string(),
            "Pump ring 24 LEDs".to_string(),
        ];
        status.dependencies = vec!["usbfs (kernel USB)".to_string()];
        status.warnings = if self.fan_lighting {
            vec![format!(
                "Radiator fan lighting is on (experimental): the fans follow the pump ring ({} fans reported).",
                telemetry
                    .and_then(|telemetry| telemetry.lighting_fan_count)
                    .unwrap_or(0)
            )]
        } else {
            vec![
                "Radiator fan LEDs are switched off. Settings → 'Light the radiator fans with the pump ring' makes them follow the ring instead."
                    .to_string(),
            ]
        };
        status.error = self.error.clone();
        status.troubleshooting = match self.state {
            DeviceState::Busy => Some("Close any other program that owns the cooler.".to_string()),
            DeviceState::Unresponsive => Some(
                "Shut the PC down, switch the PSU off for 15 seconds, then start it again."
                    .to_string(),
            ),
            DeviceState::Disconnected => {
                Some("Check the cooler's internal USB cable to the motherboard USB 2.0 header.".to_string())
            }
            _ => None,
        };
        status.last_contact_utc = self.last_contact;
        status
    }
}

/// Ring frames followed by the same 24 colours for every fan, or black when the
/// fan LEDs are off. Frames are thinned so the animation fits the controller's
/// 4 KB decode cache.
fn with_fans(animation: &Animation, fans: u8, lit: bool) -> (Vec<Vec<u8>>, u8) {
    let frame_bytes = hs::RING_LED_COUNT * 3 + fans as usize * hs::FAN_LED_COUNT * 3;
    let max_frames = (hs::MAX_RAW_LIGHTING_BYTES / frame_bytes).max(1);
    let step = animation.frames.len().div_ceil(max_frames).max(1);
    let black = vec![0u8; hs::FAN_LED_COUNT * 3];
    let frames: Vec<Vec<u8>> = animation
        .frames
        .iter()
        .step_by(step)
        .map(|frame| {
            let ring: Vec<u8> = frame
                .iter()
                .flat_map(|color| [color.r, color.g, color.b])
                .collect();
            let fan: &[u8] = if lit { &ring } else { &black };
            let mut out = Vec::with_capacity(ring.len() + fans as usize * fan.len());
            out.extend_from_slice(&ring);
            for _ in 0..fans {
                out.extend_from_slice(fan);
            }
            out
        })
        .collect();
    let interval = (animation.frame_interval_ms * step as u32).clamp(1, 255) as u8;
    (frames, interval)
}

// ---------------------------------------------------------------------------
// Corsair DRAM
// ---------------------------------------------------------------------------

pub struct CorsairDramAdapter {
    bus: Option<Smbus>,
    bus_number: Option<u32>,
    modules: Vec<CorsairDram>,
    state: DeviceState,
    error: Option<String>,
    next_attempt: Instant,
    next_health: Instant,
    last_contact: Option<DateTime<Utc>>,
    applied: HashMap<String, ZoneLighting>,
    reported: HashSet<u8>,
    pub needs_reapply: bool,
    next_reapply: Instant,
    reapply_delay: Duration,
}

impl Default for CorsairDramAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CorsairDramAdapter {
    pub fn new() -> Self {
        Self {
            bus: None,
            bus_number: None,
            modules: Vec::new(),
            state: DeviceState::Disconnected,
            error: None,
            next_attempt: Instant::now(),
            next_health: Instant::now(),
            last_contact: None,
            applied: HashMap::new(),
            reported: HashSet::new(),
            needs_reapply: false,
            next_reapply: Instant::now(),
            reapply_delay: Duration::from_secs(60),
        }
    }

    pub fn connected(&self) -> bool {
        !self.modules.is_empty() && self.state == DeviceState::Connected
    }

    pub fn bus_number(&self) -> Option<u32> {
        self.bus_number
    }

    pub fn modules(&self) -> &[CorsairDram] {
        &self.modules
    }

    pub fn due_for_reapply(&self) -> bool {
        self.needs_reapply && Instant::now() >= self.next_reapply
    }

    pub fn clear_needs_reapply(&mut self) {
        self.needs_reapply = false;
        self.next_reapply = Instant::now() + self.reapply_delay;
        self.reapply_delay = (self.reapply_delay * 2).min(Duration::from_secs(600));
    }

    pub fn ensure_connected(&mut self) -> bool {
        if !self.modules.is_empty() {
            if Instant::now() >= self.next_health {
                self.check_health();
            }
            return self.state == DeviceState::Connected;
        }
        if Instant::now() < self.next_attempt {
            return false;
        }
        self.next_attempt = Instant::now() + Duration::from_secs(30);

        let adapters: Vec<_> = inventory::i2c_adapters()
            .into_iter()
            .filter(|adapter| adapter.name.contains("SMBus PIIX4"))
            .collect();
        for adapter in adapters {
            let Ok(bus) = Smbus::open(adapter.number) else {
                continue;
            };
            let modules = corsair::detect_and_recover_on(&bus);
            if modules.is_empty() {
                continue;
            }
            for module in &modules {
                if module.recovered {
                    self.needs_reapply = true;
                    self.applied.clear();
                }
            }
            self.bus_number = Some(adapter.number);
            self.bus = Some(bus);
            self.modules = modules;
            self.state = DeviceState::Connected;
            self.error = None;
            self.last_contact = Some(Utc::now());
            self.next_health = Instant::now() + Duration::from_secs(30);
            return true;
        }
        self.state = DeviceState::Disconnected;
        self.error = Some("No Corsair DRAM RGB controllers answered on the SMBus.".to_string());
        false
    }

    fn check_health(&mut self) {
        let Some(bus) = self.bus.as_ref() else {
            return;
        };
        let mut retry_soon = false;
        for module in &mut self.modules {
            match module.check_health(bus) {
                CorsairHealth::Normal => {
                    self.reported.remove(&module.address);
                }
                CorsairHealth::Recovered => {
                    self.reported.remove(&module.address);
                    self.applied.clear();
                    self.needs_reapply = true;
                }
                CorsairHealth::Stalled => {
                    self.applied.clear();
                    self.needs_reapply = true;
                    retry_soon = true;
                    self.reported.insert(module.address);
                }
                CorsairHealth::Unreadable => {
                    self.needs_reapply = true;
                    retry_soon = true;
                    self.reported.insert(module.address);
                }
            }
        }
        self.next_health =
            Instant::now() + if retry_soon { Duration::from_secs(10) } else { Duration::from_secs(30) };
    }

    pub fn rescan(&mut self) {
        self.modules.clear();
        self.bus = None;
        self.bus_number = None;
        self.next_attempt = Instant::now();
        self.next_health = Instant::now();
        self.applied.clear();
        self.reported.clear();
        self.needs_reapply = false;
        self.next_reapply = Instant::now();
        self.reapply_delay = Duration::from_secs(60);
    }

    pub fn invalidate(&mut self) {
        self.applied.clear();
        for module in &mut self.modules {
            module.static_mode_set = false;
        }
    }

    fn zone_id(module: &CorsairDram) -> String {
        format!("corsair-dram:{:02X}", module.address)
    }

    pub fn zones(&self) -> Vec<ZoneInfo> {
        self.modules
            .iter()
            .enumerate()
            .map(|(index, module)| ZoneInfo {
                id: Self::zone_id(module),
                device_id: CORSAIR_DEVICE_ID.to_string(),
                name: format!("Memory module {}", index + 1),
                led_count: corsair::LED_COUNT,
                effects: ALL_EFFECTS.to_vec(),
                max_colors: corsair::LED_COUNT,
                has_brightness: true,
                has_speed: true,
                led_count_configurable: false,
                note: Some(
                    "Settings are stored on the module and survive reboots without software."
                        .to_string(),
                ),
            })
            .collect()
    }

    pub fn apply(&mut self, zone_id: &str, lighting: &ZoneLighting, force: bool) -> ApplyResult {
        if !self.ensure_connected() {
            return ApplyResult::failed(
                CORSAIR_DEVICE_ID,
                zone_id,
                self.error.clone().unwrap_or_else(|| "Not connected.".to_string()),
            );
        }
        let Some(index) = self
            .modules
            .iter()
            .position(|module| Self::zone_id(module) == zone_id)
        else {
            return ApplyResult::failed(CORSAIR_DEVICE_ID, zone_id, "Unknown memory module.");
        };
        let name = format!("Memory module at 0x{:02X}", self.modules[index].address);
        if !force
            && self
                .applied
                .get(zone_id)
                .map(|previous| same(previous, lighting))
                .unwrap_or(false)
        {
            return ApplyResult::ok(CORSAIR_DEVICE_ID, name, "Already applied");
        }

        let color1 = lighting.colors.first().copied().unwrap_or(RgbColor::WHITE);
        let color2 = lighting.colors.get(1).copied().unwrap_or(color1);
        let brightness = (lighting.brightness.clamp(0, 100) * 255 / 100) as u8;
        let speed = if lighting.speed < 34 {
            0
        } else if lighting.speed < 67 {
            1
        } else {
            2
        };
        enum Outcome {
            NoBus,
            Unreadable(u8),
            RecoverFailed(u8),
            Done {
                ok: bool,
                address: u8,
                kept_old: bool,
                recovered: bool,
            },
        }

        // Take the bus out temporarily so the mutable module borrow and the bus
        // borrow never overlap.
        let bus = self.bus.take();
        let outcome = match bus.as_ref() {
            None => Outcome::NoBus,
            Some(bus) => {
                let module = &mut self.modules[index];
                let mut recovered = false;
                // Re-read the device info before every write. A module can drop
                // into the abnormal firmware mode between periodic health
                // checks; using cached protocol 4 would write colours that the
                // module acknowledges but ignores, leaving stale "red edges"
                // until the next health check.
                if !module.refresh(bus) {
                    Outcome::Unreadable(module.address)
                } else if module.protocol_version < 4 {
                    if !module.recover(bus).unwrap_or(false) {
                        Outcome::RecoverFailed(module.address)
                    } else {
                        module.static_mode_set = false;
                        module.refresh(bus);
                        recovered = true;
                        let ok =
                            write_lighting(module, bus, lighting, color1, color2, brightness, speed)
                                .unwrap_or(false);
                        Outcome::Done {
                            ok,
                            address: module.address,
                            kept_old: module.last_write_stored == Some(false),
                            recovered,
                        }
                    }
                } else {
                    let mut ok =
                        write_lighting(module, bus, lighting, color1, color2, brightness, speed)
                            .unwrap_or(false);
                    if !ok
                        && module.last_write_stored == Some(false)
                        && module.recover(bus).unwrap_or(false)
                    {
                        module.static_mode_set = false;
                        module.refresh(bus);
                        recovered = true;
                        ok = write_lighting(module, bus, lighting, color1, color2, brightness, speed)
                            .unwrap_or(false);
                    }
                    Outcome::Done {
                        ok,
                        address: module.address,
                        kept_old: module.last_write_stored == Some(false),
                        recovered,
                    }
                }
            }
        };
        self.bus = bus;

        match outcome {
            Outcome::NoBus => {
                ApplyResult::failed(CORSAIR_DEVICE_ID, name, "SMBus is not open.")
            }
            Outcome::Unreadable(address) => {
                self.needs_reapply = true;
                ApplyResult::failed(
                    CORSAIR_DEVICE_ID,
                    name,
                    format!(
                        "Memory module at 0x{address:02X} did not return a verifiable device-info block; it was left untouched and will be retried."
                    ),
                )
            }
            Outcome::RecoverFailed(address) => {
                self.needs_reapply = true;
                ApplyResult::failed(
                    CORSAIR_DEVICE_ID,
                    name,
                    format!(
                        "Memory module at 0x{address:02X} is in an abnormal firmware mode and ignores lighting writes. Click Rescan; if that does not help, shut down and switch the power supply off for 15 seconds."
                    ),
                )
            }
            Outcome::Done {
                ok,
                address,
                kept_old,
                recovered,
            } => {
                if recovered {
                    self.applied.clear();
                }
                if ok {
                    self.applied.insert(zone_id.to_string(), lighting.clone());
                    self.last_contact = Some(Utc::now());
                    self.reapply_delay = Duration::from_secs(60);
                    self.reported.remove(&address);
                    ApplyResult::ok(
                        CORSAIR_DEVICE_ID,
                        name,
                        format!(
                            "{} stored on the module{}",
                            effect_label(lighting.effect),
                            if recovered {
                                " (recovered from abnormal firmware mode)"
                            } else {
                                ""
                            }
                        ),
                    )
                } else {
                    self.needs_reapply = true;
                    self.next_reapply = Instant::now() + Duration::from_secs(30);
                    self.reported.insert(address);
                    ApplyResult::failed(
                        CORSAIR_DEVICE_ID,
                        name,
                        if kept_old {
                            "The module acknowledged the colours but kept its previous ones. Rewriting automatically; click Rescan if it stays wrong."
                        } else {
                            "The module rejected the configuration (checksum mismatch). Another program may be using the SMBus."
                        },
                    )
                }
            }
        }
    }

    pub fn status(&self) -> DeviceStatus {
        let mut status = DeviceStatus::placeholder(
            CORSAIR_DEVICE_ID,
            "Vengeance RGB DDR5",
            "Corsair",
            "Memory lighting",
        );
        status.state = self.state;
        status.firmware = self.modules.first().map(|module| module.firmware.clone());
        status.location = self.bus_number.map(|bus| format!("SMBus i2c-{bus}"));
        status.zones = self.zones();
        status.capabilities = vec![
            format!("{} module(s) × {} LEDs", self.modules.len(), corsair::LED_COUNT),
            "Hardware effects stored on the module".to_string(),
        ];
        status.dependencies = vec![
            "i2c-dev / piix4_smbus (acpi_enforce_resources=lax)".to_string(),
            "No other program writing the SMBus".to_string(),
        ];
        status.warnings = self
            .modules
            .iter()
            .filter(|module| module.device_info_valid && module.protocol_version < 4)
            .map(|module| {
                format!(
                    "Memory module at 0x{:02X} reports an abnormal firmware state (protocol {}) and ignores lighting writes. Click Rescan to recover it.",
                    module.address, module.protocol_version
                )
            })
            .collect();
        status.error = self.error.clone();
        status.troubleshooting = (self.state == DeviceState::Error).then(|| {
            "Close other SMBus users, then click Rescan.".to_string()
        });
        status.last_contact_utc = self.last_contact;
        status
    }
}

fn same(a: &ZoneLighting, b: &ZoneLighting) -> bool {
    a.effect == b.effect && a.brightness == b.brightness && a.speed == b.speed && a.colors == b.colors
}

fn write_lighting(
    module: &mut CorsairDram,
    bus: &Smbus,
    lighting: &ZoneLighting,
    color1: RgbColor,
    color2: RgbColor,
    brightness: u8,
    speed: u8,
) -> std::io::Result<bool> {
    match lighting.effect {
        LightingEffect::Off => module.set_static_colors(bus, &[RgbColor::BLACK]),
        LightingEffect::Static => module.set_static_colors(
            bus,
            &[color1.with_brightness(lighting.brightness)],
        ),
        LightingEffect::Gradient => {
            let animation = effect::generate(lighting, corsair::LED_COUNT);
            module.set_static_colors(bus, &animation.frames[0])
        }
        LightingEffect::Breathing | LightingEffect::Flash => module.set_effect(
            bus,
            CorsairMode::ColorPulse,
            speed,
            0,
            false,
            brightness,
            color1,
            color2,
        ),
        LightingEffect::ColorCycle => module.set_effect(
            bus,
            CorsairMode::ColorShift,
            speed,
            0,
            lighting.colors.len() < 2,
            brightness,
            color1,
            color2,
        ),
        LightingEffect::Rainbow => module.set_effect(
            bus,
            CorsairMode::RainbowWave,
            speed,
            1,
            false,
            255,
            color1,
            color2,
        ),
        LightingEffect::RainbowWave => module.set_effect(
            bus,
            CorsairMode::Rainbow,
            speed,
            1,
            false,
            255,
            color1,
            color2,
        ),
        LightingEffect::Wave => module.set_effect(
            bus,
            CorsairMode::ColorWave,
            speed,
            1,
            false,
            brightness,
            color1,
            color2,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_fans_thins_frames_to_fit_the_decode_cache() {
        let animation = Animation {
            frames: vec![vec![RgbColor::WHITE; hs::RING_LED_COUNT]; 48],
            frame_interval_ms: 100,
        };
        let (frames, interval) = with_fans(&animation, 3, true);
        assert!(!frames.is_empty());
        assert!(frames.len() * frames[0].len() <= hs::MAX_RAW_LIGHTING_BYTES);
        assert_eq!(frames[0].len(), (hs::RING_LED_COUNT + 3 * hs::FAN_LED_COUNT) * 3);
        assert!(interval as u32 >= animation.frame_interval_ms);
    }

    #[test]
    fn with_fans_blackens_the_fan_leds_when_turned_off() {
        let animation = Animation {
            frames: vec![vec![RgbColor::WHITE; hs::RING_LED_COUNT]],
            frame_interval_ms: 100,
        };
        let (frames, _) = with_fans(&animation, 1, false);
        let ring_bytes = hs::RING_LED_COUNT * 3;
        assert!(frames[0][..ring_bytes].iter().all(|value| *value == 255));
        assert!(frames[0][ring_bytes..].iter().all(|value| *value == 0));
    }

    #[test]
    fn zone_ids_and_effects_are_stable() {
        assert_eq!(FUSION2_DEVICE_ID, "gigabyte-fusion2");
        assert_eq!(RING_ZONE_ID, "hydroshift:ring");
        assert_eq!(ALL_EFFECTS.len(), 9);
    }
}
