//! Effects, zone capabilities and software-generated animation frames.
//!
//! Ported from `MyLianPC.Core/Lighting/LightingModels.cs` and
//! `EffectFrames.cs`. Frame sequences are at most [`MAX_FRAMES`] and loop
//! seamlessly, because the cooler's ring uploads a fixed-size frame table.

use serde::{Deserialize, Serialize};

use crate::color::RgbColor;

/// Effects the application knows about. An adapter advertises the subset its
/// hardware really implements; the UI must not offer anything outside that set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LightingEffect {
    Off,
    Static,
    Breathing,
    ColorCycle,
    Rainbow,
    RainbowWave,
    Wave,
    Gradient,
    Flash,
}

/// What one lighting zone can actually do, as reported by its adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneCapabilities {
    pub effects: Vec<LightingEffect>,
    pub hardware_brightness: bool,
    pub per_led_color: bool,
    pub led_count: usize,
    pub max_colors: usize,
}

impl ZoneCapabilities {
    pub fn supports(&self, effect: LightingEffect) -> bool {
        self.effects.contains(&effect)
    }
}

/// The requested lighting for one zone. Speed and brightness are percentages.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneLighting {
    pub effect: LightingEffect,
    pub colors: Vec<RgbColor>,
    pub brightness: i32,
    pub speed: i32,
}

impl Default for ZoneLighting {
    fn default() -> Self {
        Self {
            effect: LightingEffect::Static,
            colors: vec![RgbColor::WHITE],
            brightness: 100,
            speed: 50,
        }
    }
}

impl ZoneLighting {
    /// Checks the request against what the zone supports.
    /// Returns human-readable problems; an empty list means valid.
    pub fn validate(&self, caps: &ZoneCapabilities) -> Vec<String> {
        let mut problems = Vec::new();
        if !caps.supports(self.effect) {
            problems.push(format!(
                "Effect '{:?}' is not supported by this zone.",
                self.effect
            ));
        }
        if !(0..=100).contains(&self.brightness) {
            problems.push("Brightness must be between 0 and 100.".to_string());
        }
        if !(0..=100).contains(&self.speed) {
            problems.push("Speed must be between 0 and 100.".to_string());
        }
        let colour_free = matches!(
            self.effect,
            LightingEffect::Off | LightingEffect::Rainbow | LightingEffect::RainbowWave
        );
        if !colour_free && self.colors.is_empty() {
            problems.push("At least one color is required for this effect.".to_string());
        }
        if self.colors.len() > caps.max_colors {
            problems.push(format!(
                "This zone accepts at most {} color(s).",
                caps.max_colors
            ));
        }
        problems
    }
}

/// A generated animation: one frame per entry, each frame has one colour per LED.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Animation {
    pub frames: Vec<Vec<RgbColor>>,
    pub frame_interval_ms: u32,
}

/// Maximum frames the ring upload accepts. Do not raise without hardware evidence.
pub const MAX_FRAMES: usize = 48;

/// Effect period in ms for a speed percentage: 0% = 6 s, 100% = 0.8 s.
pub fn period_ms(speed_percent: i32) -> u32 {
    (6000.0 - speed_percent.clamp(0, 100) as f64 / 100.0 * 5200.0) as u32
}

/// Generates the frame sequence for an effect on a device without a built-in
/// effect engine, and for UI previews.
pub fn generate(zone: &ZoneLighting, led_count: usize) -> Animation {
    let led_count = led_count.max(1);
    let colors = if zone.colors.is_empty() {
        vec![RgbColor::WHITE]
    } else {
        zone.colors.clone()
    };
    let interval = (period_ms(zone.speed) / MAX_FRAMES as u32).clamp(20, 250);
    let brightness = zone.brightness;

    let animated = matches!(
        zone.effect,
        LightingEffect::Breathing
            | LightingEffect::Flash
            | LightingEffect::ColorCycle
            | LightingEffect::Rainbow
            | LightingEffect::RainbowWave
            | LightingEffect::Wave
    );
    if !animated {
        return Animation {
            frames: vec![frame(zone.effect, led_count, 0, &colors, brightness)],
            frame_interval_ms: 100,
        };
    }
    Animation {
        frames: (0..MAX_FRAMES)
            .map(|phase| frame(zone.effect, led_count, phase, &colors, brightness))
            .collect(),
        frame_interval_ms: interval,
    }
}

/// One representative frame for the UI preview. Animated effects use a mid
/// phase so breathing is not shown as black.
pub fn preview_frame(zone: &ZoneLighting, led_count: usize) -> Vec<RgbColor> {
    let led_count = led_count.max(1);
    let colors = if zone.colors.is_empty() {
        vec![RgbColor::WHITE]
    } else {
        zone.colors.clone()
    };
    let phase = if zone.effect == LightingEffect::Breathing {
        MAX_FRAMES / 4
    } else {
        0
    };
    frame(zone.effect, led_count, phase, &colors, zone.brightness)
}

fn frame(
    effect: LightingEffect,
    led_count: usize,
    phase: usize,
    colors: &[RgbColor],
    brightness: i32,
) -> Vec<RgbColor> {
    let led_count = led_count.max(1);
    let colors = if colors.is_empty() {
        &[RgbColor::WHITE][..]
    } else {
        colors
    };
    let fill = |color: RgbColor| vec![color.with_brightness(brightness); led_count];
    match effect {
        LightingEffect::Off => vec![RgbColor::BLACK; led_count],
        LightingEffect::Static => fill(colors[0]),
        LightingEffect::Gradient => (0..led_count)
            .map(|index| {
                let t = if led_count == 1 {
                    0.0
                } else {
                    index as f64 / (led_count - 1) as f64
                };
                sample(colors, t).with_brightness(brightness)
            })
            .collect(),
        LightingEffect::Breathing => {
            let level =
                0.5 - 0.5 * (std::f64::consts::TAU * phase as f64 / MAX_FRAMES as f64).cos();
            let color = colors[phase * colors.len() / MAX_FRAMES % colors.len()];
            vec![
                RgbColor::new(
                    (color.r as f64 * level) as u8,
                    (color.g as f64 * level) as u8,
                    (color.b as f64 * level) as u8,
                )
                .with_brightness(brightness);
                led_count
            ]
        }
        LightingEffect::Flash => {
            let color = if phase % 12 < 3 {
                colors[phase / 12 % colors.len()].with_brightness(brightness)
            } else {
                RgbColor::BLACK
            };
            vec![color; led_count]
        }
        LightingEffect::ColorCycle => {
            if colors.len() >= 2 {
                let color = sample_cyclic(colors, phase as f64 / MAX_FRAMES as f64)
                    .with_brightness(brightness);
                vec![color; led_count]
            } else {
                (0..led_count)
                    .map(|_| {
                        RgbColor::from_hsv(360.0 * phase as f64 / MAX_FRAMES as f64, 1.0, 1.0)
                            .with_brightness(brightness)
                    })
                    .collect()
            }
        }
        LightingEffect::Rainbow => (0..led_count)
            .map(|index| {
                RgbColor::from_hsv(
                    360.0 * index as f64 / led_count as f64
                        - 360.0 * phase as f64 / MAX_FRAMES as f64,
                    1.0,
                    1.0,
                )
                .with_brightness(brightness)
            })
            .collect(),
        LightingEffect::RainbowWave => (0..led_count)
            .map(|index| {
                let wave = index as f64 / led_count as f64 - phase as f64 / MAX_FRAMES as f64;
                let level = 0.25 + 0.75 * (0.5 - 0.5 * (std::f64::consts::TAU * wave).cos());
                RgbColor::from_hsv(360.0 * index as f64 / led_count as f64, 1.0, level)
                    .with_brightness(brightness)
            })
            .collect(),
        LightingEffect::Wave => {
            let wave_colors: Vec<RgbColor> = if colors.len() >= 2 {
                colors.to_vec()
            } else {
                vec![colors[0], RgbColor::BLACK]
            };
            (0..led_count)
                .map(|index| {
                    sample_cyclic(
                        &wave_colors,
                        (index as f64 / led_count as f64 + phase as f64 / MAX_FRAMES as f64) % 1.0,
                    )
                    .with_brightness(brightness)
                })
                .collect()
        }
    }
}


/// Colour at position `t` in [0, 1] along a multi-stop gradient.
pub fn sample(stops: &[RgbColor], t: f64) -> RgbColor {
    if stops.is_empty() {
        return RgbColor::BLACK;
    }
    if stops.len() == 1 {
        return stops[0];
    }
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let i = (t as usize).min(stops.len() - 2);
    RgbColor::lerp(stops[i], stops[i + 1], t - i as f64)
}

fn sample_cyclic(stops: &[RgbColor], t: f64) -> RgbColor {
    if stops.is_empty() {
        return RgbColor::BLACK;
    }
    if stops.len() == 1 {
        return stops[0];
    }
    let wrapped = (t % 1.0 + 1.0) % 1.0;
    let scaled = wrapped * stops.len() as f64;
    let i = (scaled as usize) % stops.len();
    RgbColor::lerp(
        stops[i],
        stops[(i + 1) % stops.len()],
        scaled - scaled.floor(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(effect: LightingEffect, colors: Vec<RgbColor>) -> ZoneLighting {
        ZoneLighting {
            effect,
            colors,
            brightness: 100,
            speed: 50,
        }
    }

    #[test]
    fn period_matches_the_original_range() {
        assert_eq!(period_ms(0), 6000);
        assert_eq!(period_ms(100), 800);
        assert_eq!(period_ms(-10), 6000);
        assert_eq!(period_ms(200), 800);
    }

    #[test]
    fn animated_effects_fit_the_upload_limit() {
        for effect in [
            LightingEffect::Breathing,
            LightingEffect::Flash,
            LightingEffect::ColorCycle,
            LightingEffect::Rainbow,
            LightingEffect::RainbowWave,
            LightingEffect::Wave,
        ] {
            let animation = generate(&zone(effect, vec![RgbColor::WHITE]), 24);
            assert_eq!(animation.frames.len(), MAX_FRAMES, "{effect:?}");
            assert!(animation.frames.iter().all(|frame| frame.len() == 24));
            assert!((20..=250).contains(&animation.frame_interval_ms));
        }
    }

    #[test]
    fn static_effects_produce_one_frame() {
        for effect in [
            LightingEffect::Off,
            LightingEffect::Static,
            LightingEffect::Gradient,
        ] {
            let animation = generate(&zone(effect, vec![RgbColor::new(10, 20, 30)]), 12);
            assert_eq!(animation.frames.len(), 1, "{effect:?}");
            assert_eq!(animation.frames[0].len(), 12);
        }
    }

    #[test]
    fn brightness_is_applied_to_generated_frames() {
        let mut z = zone(LightingEffect::Static, vec![RgbColor::new(200, 100, 50)]);
        z.brightness = 50;
        let animation = generate(&z, 4);
        assert_eq!(animation.frames[0][0], RgbColor::new(100, 50, 25));
    }

    #[test]
    fn rainbow_spreads_across_the_strip() {
        let animation = generate(&zone(LightingEffect::Rainbow, vec![]), 12);
        assert_ne!(animation.frames[0][0], animation.frames[0][11]);
        assert_eq!(animation.frames[0].len(), 12);
    }

    #[test]
    fn gradient_samples_endpoints() {
        let stops = vec![RgbColor::new(0, 0, 0), RgbColor::new(255, 255, 255)];
        assert_eq!(sample(&stops, 0.0), stops[0]);
        assert_eq!(sample(&stops, 1.0), stops[1]);
        assert_eq!(sample(&stops, 0.5), RgbColor::new(128, 128, 128));
    }

    #[test]
    fn validation_matches_the_original_rules() {
        let caps = ZoneCapabilities {
            effects: vec![LightingEffect::Static, LightingEffect::Rainbow],
            hardware_brightness: false,
            per_led_color: true,
            led_count: 24,
            max_colors: 2,
        };
        assert!(zone(LightingEffect::Static, vec![RgbColor::WHITE])
            .validate(&caps)
            .is_empty());
        assert!(zone(LightingEffect::Rainbow, vec![])
            .validate(&caps)
            .is_empty());
        assert_eq!(
            zone(LightingEffect::Static, vec![])
                .validate(&caps)
                .len(),
            1
        );
        assert_eq!(
            zone(
                LightingEffect::Static,
                vec![RgbColor::WHITE, RgbColor::WHITE, RgbColor::WHITE]
            )
            .validate(&caps)
            .len(),
            1
        );
    }
}
