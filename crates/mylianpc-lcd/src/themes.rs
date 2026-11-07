//! LCD themes.
//!
//! Ported from `MyLianPC.Lcd/LcdRenderer.cs`. The layout coordinates, colours
//! and formatting match the Windows renderer; the drawing API is the local
//! tiny-skia canvas.

use std::collections::BTreeMap;

use chrono::{DateTime, Local, Timelike};
use mylianpc_core::color::RgbColor;
use mylianpc_core::metrics;
use mylianpc_core::profile::{
    GaugeStyle, LcdMode, LcdSettings, LcdWidgetType, TemperatureUnit,
};
use serde::{Deserialize, Serialize};
use tiny_skia::Pixmap;

use crate::render::{Align, Canvas, FontCache, Weight, CENTER, SIZE};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LcdMetrics {
    pub values: BTreeMap<String, f64>,
    pub time: DateTime<Local>,
}

impl LcdMetrics {
    pub fn new(values: BTreeMap<String, f64>, time: DateTime<Local>) -> Self {
        Self { values, time }
    }

    pub fn get(&self, key: &str) -> Option<f64> {
        self.values.get(key).copied()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LcdTheme {
    pub id: String,
    pub name: String,
    pub description: String,
}

pub fn themes() -> Vec<LcdTheme> {
    [
        ("minimal-temps", "Minimal temperatures", "Large CPU and GPU temperatures with loads"),
        ("overview", "System overview", "Temperatures, loads, coolant, pump, fans, RAM and clock"),
        ("gaming-dashboard", "Gaming dashboard", "Temperatures, loads and clocks at a glance"),
        ("gauges", "Circular gauges", "Concentric CPU/GPU temperature arcs"),
        ("cyberpunk", "Cyberpunk", "Neon yellow and magenta HUD"),
        ("hud-toxic", "Toxic HUD", "Acid green and spring green HUD"),
        ("hud-synthwave", "Synthwave HUD", "Violet, pink and cyan HUD"),
        ("hud-ice", "Ice HUD", "Frost blue HUD"),
        ("hud-ember", "Ember HUD", "Amber and red HUD"),
        ("neon", "Dark neon", "Glowing ring that shifts with CPU temperature"),
        ("cpu-gpu", "CPU & GPU monitor", "Side-by-side CPU and GPU details"),
        ("media", "Image / animation", "Your picture or GIF, optional clock overlay"),
        ("minimal-clock", "Minimal clock", "Time, date and a quiet temperature line"),
        ("analog-clock", "Analog clock", "Clock face with the CPU and GPU temperatures underneath"),
        ("cooling", "Cooling focus", "Coolant temperature, pump and radiator fan speeds"),
        ("load-bars", "Load bars", "Vertical CPU, GPU and RAM load bars"),
        ("power", "Power draw", "CPU and GPU package power with loads"),
        ("terminal", "Status list", "A compact readout of every important value"),
        ("split", "Diagonal split", "CPU in one corner, GPU in the other"),
        ("multi-metric", "Multi-metric", "Six tiles with the metrics you pick"),
        ("custom", "Custom layout", "Your own movable widgets"),
    ]
    .into_iter()
    .map(|(id, name, description)| LcdTheme {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
    })
    .collect()
}

/// Renders one frame. `media` is the current image/animation frame, already
/// 480×480.
pub fn render(
    settings: &LcdSettings,
    metrics: &LcdMetrics,
    media: Option<&Pixmap>,
    fonts: &mut FontCache,
) -> Pixmap {
    let mut canvas = Canvas::new(
        fonts,
        &settings.font_family,
        settings.font_scale,
        settings.background,
    );
    let theme = match settings.mode {
        LcdMode::Image | LcdMode::Animation => "media",
        LcdMode::Off => "off",
        LcdMode::Theme => settings.theme_id.as_str(),
    };
    match theme {
        "off" => canvas.clear(RgbColor::BLACK),
        "overview" => overview(&mut canvas, settings, metrics),
        "gaming-dashboard" => gaming_dashboard(&mut canvas, settings, metrics),
        "gauges" => gauges(&mut canvas, settings, metrics),
        "cyberpunk" => hud(&mut canvas, settings, metrics, &CYBER_HUD),
        "hud-toxic" => hud(&mut canvas, settings, metrics, &TOXIC_HUD),
        "hud-synthwave" => hud(&mut canvas, settings, metrics, &SYNTHWAVE_HUD),
        "hud-ice" => hud(&mut canvas, settings, metrics, &ICE_HUD),
        "hud-ember" => hud(&mut canvas, settings, metrics, &EMBER_HUD),
        "neon" => neon(&mut canvas, settings, metrics),
        "cpu-gpu" => cpu_gpu(&mut canvas, settings, metrics),
        "media" => media_theme(&mut canvas, settings, metrics, media),
        "minimal-clock" => minimal_clock(&mut canvas, settings, metrics),
        "analog-clock" => analog_clock(&mut canvas, settings, metrics),
        "cooling" => cooling(&mut canvas, settings, metrics),
        "load-bars" => load_bars(&mut canvas, settings, metrics),
        "power" => power(&mut canvas, settings, metrics),
        "terminal" => terminal(&mut canvas, settings, metrics),
        "split" => split(&mut canvas, settings, metrics),
        "multi-metric" => multi_metric(&mut canvas, settings, metrics),
        "custom" => custom(&mut canvas, settings, metrics),
        _ => minimal_temps(&mut canvas, settings, metrics),
    }
    canvas.mask_circle();
    canvas.pixmap
}

// ---------------------------------------------------------------- formatting

pub fn format_temp(celsius: Option<f64>, unit: TemperatureUnit) -> String {
    match celsius {
        None => "—°".to_string(),
        Some(value) if unit == TemperatureUnit::Fahrenheit => {
            format!("{:.0}°", value * 9.0 / 5.0 + 32.0)
        }
        Some(value) => format!("{value:.0}°"),
    }
}

pub fn clock_text(time: DateTime<Local>, twenty_four_hour: bool, seconds: bool) -> String {
    if twenty_four_hour {
        if seconds {
            time.format("%H:%M:%S").to_string()
        } else {
            time.format("%H:%M").to_string()
        }
    } else {
        let hour = time.hour() % 12;
        let hour = if hour == 0 { 12 } else { hour };
        let suffix = if time.hour() < 12 { "AM" } else { "PM" };
        if seconds {
            format!("{hour}:{:02}:{:02} {suffix}", time.minute(), time.second())
        } else {
            format!("{hour}:{:02} {suffix}", time.minute())
        }
    }
}

/// Formats one metric with the unit and precision the dashboard and LCD use.
pub fn format_metric(metric: &str, value: Option<f64>, unit: TemperatureUnit) -> String {
    let Some(value) = value else {
        return "—".to_string();
    };
    if metric == metrics::RAM_XMP {
        return if value >= 0.5 { "On" } else { "Off" }.to_string();
    }
    match metrics::find(metric).map(|info| info.unit) {
        Some("°C") => format_temp(Some(value), unit),
        Some("%") => format!("{value:.0}%"),
        Some("MHz") => {
            if value >= 1000.0 {
                format!("{:.2} GHz", value / 1000.0)
            } else {
                format!("{value:.0} MHz")
            }
        }
        Some("W") => format!("{value:.0} W"),
        Some("RPM") => format!("{value:.0} rpm"),
        Some("GB") => format!("{value:.1} GB"),
        Some("MT/s") => format!("{value:.0} MT/s"),
        _ => format!("{value:.1}"),
    }
}

fn temp(metrics: &LcdMetrics, unit: TemperatureUnit, key: &str) -> String {
    format_temp(metrics.get(key), unit)
}

fn fmt(metrics: &LcdMetrics, unit: TemperatureUnit, key: &str) -> String {
    format_metric(key, metrics.get(key), unit)
}

fn number(value: Option<f64>) -> String {
    value.map(|value| format!("{value:.0}")).unwrap_or_else(|| "—".to_string())
}

fn fraction(value: Option<f64>, max: f64) -> Option<f64> {
    value.map(|value| (value / max).clamp(0.0, 1.0))
}

fn clock(metrics: &LcdMetrics, settings: &LcdSettings, seconds: bool) -> String {
    clock_text(metrics.time, settings.clock_24_hour, seconds)
}

fn aio_fan_rpm(metrics: &LcdMetrics) -> Option<f64> {
    let values: Vec<f64> = [
        metrics::AIO_FAN1,
        metrics::AIO_FAN2,
        metrics::AIO_FAN3,
    ]
    .iter()
    .filter_map(|key| metrics.get(key))
    .filter(|value| *value > 0.0)
    .collect();
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

// ------------------------------------------------------------------ themes

fn minimal_temps(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center(&format!("CPU  {}", fmt(m, s.unit, metrics::CPU_LOAD)), CENTER, 100.0, 32.0, s.accent, Weight::Bold, 255, 0.0);
    c.center(&temp(m, s.unit, metrics::CPU_TEMP), CENTER, 205.0, 118.0, fg, Weight::Bold, 255, 0.0);
    c.line(120.0, 240.0, 360.0, 240.0, fg, 3.0, 70);
    c.center(&temp(m, s.unit, metrics::GPU_TEMP), CENTER, 350.0, 118.0, fg, Weight::Bold, 255, 0.0);
    c.center(&format!("GPU  {}", fmt(m, s.unit, metrics::GPU_LOAD)), CENTER, 392.0, 32.0, s.secondary, Weight::Bold, 255, 0.0);
    c.center(
        &format!("H₂O {}  ·  {}", temp(m, s.unit, metrics::COOLANT_TEMP), clock(m, s, false)),
        CENTER,
        432.0,
        26.0,
        fg,
        Weight::Bold,
        255,
        190.0,
    );
}

fn overview(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center(&clock(m, s, false), CENTER, 84.0, 38.0, fg, Weight::Bold, 255, 0.0);
    c.center(
        &m.time.format("%a %-d %b").to_string(),
        CENTER,
        112.0,
        20.0,
        fg,
        Weight::Bold,
        255,
        170.0,
    );
    c.line(CENTER, 140.0, CENTER, 228.0, fg, 2.0, 50);
    c.center(&temp(m, s.unit, metrics::CPU_TEMP), 158.0, 192.0, 72.0, s.accent, Weight::Bold, 255, 150.0);
    c.center(&temp(m, s.unit, metrics::GPU_TEMP), 322.0, 192.0, 72.0, s.secondary, Weight::Bold, 255, 150.0);
    c.center(&format!("CPU {}", fmt(m, s.unit, metrics::CPU_LOAD)), 158.0, 224.0, 22.0, fg, Weight::Bold, 220, 150.0);
    c.center(&format!("GPU {}", fmt(m, s.unit, metrics::GPU_LOAD)), 322.0, 224.0, 22.0, fg, Weight::Bold, 220, 150.0);
    c.stat(112.0, 292.0, &temp(m, s.unit, metrics::COOLANT_TEMP), "COOLANT", fg, s.accent);
    c.stat(CENTER, 292.0, &number(m.get(metrics::PUMP_RPM)), "PUMP RPM", fg, fg);
    c.stat(368.0, 292.0, &number(aio_fan_rpm(m)), "FAN RPM", fg, s.secondary);
    c.center(
        &format!("RAM {}  ·  {}", fmt(m, s.unit, metrics::RAM_USED_GB), fmt(m, s.unit, metrics::RAM_LOAD)),
        CENTER,
        360.0,
        22.0,
        fg,
        Weight::Bold,
        220,
        0.0,
    );
    c.bar(130.0, 380.0, 220.0, 12.0, fraction(m.get(metrics::RAM_LOAD), 100.0), s.accent);
    c.center(
        &format!("{}  ·  {}", fmt(m, s.unit, metrics::CPU_CLOCK), fmt(m, s.unit, metrics::CPU_POWER)),
        CENTER,
        424.0,
        22.0,
        fg,
        Weight::Bold,
        255,
        0.0,
    );
}

fn gaming_dashboard(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center(&clock(m, s, false), CENTER, 78.0, 34.0, fg, Weight::Bold, 255, 200.0);
    c.gauge(150.0, 186.0, 78.0, fraction(m.get(metrics::CPU_TEMP), 100.0), s.accent, false);
    c.gauge(330.0, 186.0, 78.0, fraction(m.get(metrics::GPU_TEMP), 100.0), s.secondary, false);
    c.center(&temp(m, s.unit, metrics::CPU_TEMP), 150.0, 203.0, 50.0, fg, Weight::Bold, 255, 120.0);
    c.center(&temp(m, s.unit, metrics::GPU_TEMP), 330.0, 203.0, 50.0, fg, Weight::Bold, 255, 120.0);
    c.center("CPU", 150.0, 238.0, 22.0, s.accent, Weight::Bold, 255, 0.0);
    c.center("GPU", 330.0, 238.0, 22.0, s.secondary, Weight::Bold, 255, 0.0);
    c.bar_row("CPU", fraction(m.get(metrics::CPU_LOAD), 100.0), &fmt(m, s.unit, metrics::CPU_LOAD), 296.0, s.accent, fg);
    c.bar_row("GPU", fraction(m.get(metrics::GPU_LOAD), 100.0), &fmt(m, s.unit, metrics::GPU_LOAD), 334.0, s.secondary, fg);
    c.bar_row("RAM", fraction(m.get(metrics::RAM_LOAD), 100.0), &fmt(m, s.unit, metrics::RAM_LOAD), 372.0, fg, fg);
    c.center(
        &format!(
            "{}  ·  {}  ·  H₂O {}",
            fmt(m, s.unit, metrics::CPU_CLOCK),
            fmt(m, s.unit, metrics::CPU_POWER),
            temp(m, s.unit, metrics::COOLANT_TEMP)
        ),
        CENTER,
        420.0,
        22.0,
        fg,
        Weight::Bold,
        255,
        190.0,
    );
}

fn gauges(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.arc(CENTER, CENTER, 215.0, 20.0, fraction(m.get(metrics::CPU_TEMP), 100.0), s.accent);
    c.arc(CENTER, CENTER, 186.0, 20.0, fraction(m.get(metrics::GPU_TEMP), 100.0), s.secondary);
    c.center(&clock(m, s, false), CENTER, 142.0, 28.0, fg, Weight::Bold, 255, 170.0);
    c.center(&temp(m, s.unit, metrics::CPU_TEMP), CENTER, 234.0, 96.0, fg, Weight::Bold, 255, 260.0);
    c.center(&format!("CPU  {}", fmt(m, s.unit, metrics::CPU_LOAD)), CENTER, 272.0, 28.0, s.accent, Weight::Bold, 255, 0.0);
    c.center(
        &format!("GPU {}  ·  {}", temp(m, s.unit, metrics::GPU_TEMP), fmt(m, s.unit, metrics::GPU_LOAD)),
        CENTER,
        318.0,
        28.0,
        s.secondary,
        Weight::Bold,
        255,
        280.0,
    );
    c.center(&format!("H₂O {}", temp(m, s.unit, metrics::COOLANT_TEMP)), CENTER, 356.0, 28.0, fg, Weight::Bold, 255, 220.0);
}

struct HudPalette {
    title: &'static str,
    frame: RgbColor,
    cpu: RgbColor,
    gpu: RgbColor,
    detail: RgbColor,
    footer: RgbColor,
}

const CYBER_HUD: HudPalette = HudPalette {
    title: "// SYS.STATUS",
    frame: RgbColor::new(0xFC, 0xEE, 0x0A),
    cpu: RgbColor::new(0xFC, 0xEE, 0x0A),
    gpu: RgbColor::new(0xFF, 0x2A, 0x6D),
    detail: RgbColor::new(0x05, 0xD9, 0xE8),
    footer: RgbColor::new(0xFF, 0x2A, 0x6D),
};
const TOXIC_HUD: HudPalette = HudPalette {
    title: "// TOX.LOAD",
    frame: RgbColor::new(0xA8, 0xFF, 0x00),
    cpu: RgbColor::new(0xD8, 0xFF, 0x2E),
    gpu: RgbColor::new(0x00, 0xFF, 0xA3),
    detail: RgbColor::new(0x7B, 0xFF, 0x3A),
    footer: RgbColor::new(0x00, 0xFF, 0xA3),
};
const SYNTHWAVE_HUD: HudPalette = HudPalette {
    title: "// SYNTH.WAVE",
    frame: RgbColor::new(0xB9, 0x67, 0xFF),
    cpu: RgbColor::new(0xFF, 0x71, 0xCE),
    gpu: RgbColor::new(0x01, 0xCD, 0xFE),
    detail: RgbColor::new(0x05, 0xFF, 0xA1),
    footer: RgbColor::new(0xFF, 0xFB, 0x96),
};
const ICE_HUD: HudPalette = HudPalette {
    title: "// CRYO.CORE",
    frame: RgbColor::new(0x7F, 0xE7, 0xFF),
    cpu: RgbColor::new(0xD8, 0xF6, 0xFF),
    gpu: RgbColor::new(0x4F, 0xB6, 0xFF),
    detail: RgbColor::new(0x9B, 0xE7, 0xFF),
    footer: RgbColor::new(0x6A, 0xD3, 0xFF),
};
const EMBER_HUD: HudPalette = HudPalette {
    title: "// THERM.EMBR",
    frame: RgbColor::new(0xFF, 0x9E, 0x2C),
    cpu: RgbColor::new(0xFF, 0xC4, 0x6B),
    gpu: RgbColor::new(0xFF, 0x4D, 0x3D),
    detail: RgbColor::new(0xFF, 0xB2, 0x7A),
    footer: RgbColor::new(0xFF, 0xD2, 0x4A),
};

fn hud(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics, p: &HudPalette) {
    let fg = s.text_color;
    // A light grid.
    for position in (0..=SIZE as i32).step_by(24) {
        let position = position as f32;
        c.line(position, 0.0, position, SIZE as f32, p.detail, 1.0, 28);
        c.line(0.0, position, SIZE as f32, position, p.detail, 1.0, 28);
    }
    let frame = [
        (62.0, 118.0),
        (418.0, 118.0),
        (440.0, 140.0),
        (440.0, 318.0),
        (418.0, 340.0),
        (62.0, 340.0),
        (40.0, 318.0),
        (40.0, 140.0),
    ];
    c.fill_polygon(&frame, RgbColor::BLACK, 170);
    c.stroke_polygon(&frame, 3.0, p.frame, 255);
    c.center(p.title, CENTER, 98.0, 24.0, p.frame, Weight::Bold, 255, 0.0);
    c.line(CENTER, 150.0, CENTER, 312.0, p.detail, 2.0, 90);
    c.center(&temp(m, s.unit, metrics::CPU_TEMP), 140.0, 238.0, 100.0, p.cpu, Weight::Bold, 255, 180.0);
    c.center(&temp(m, s.unit, metrics::GPU_TEMP), 340.0, 238.0, 100.0, p.gpu, Weight::Bold, 255, 180.0);
    c.center(&format!("CPU {}", fmt(m, s.unit, metrics::CPU_LOAD)), 140.0, 280.0, 26.0, p.detail, Weight::Bold, 255, 180.0);
    c.center(&format!("GPU {}", fmt(m, s.unit, metrics::GPU_LOAD)), 340.0, 280.0, 26.0, p.detail, Weight::Bold, 255, 180.0);
    c.bar(80.0, 306.0, 120.0, 10.0, fraction(m.get(metrics::CPU_LOAD), 100.0), p.cpu);
    c.bar(280.0, 306.0, 120.0, 10.0, fraction(m.get(metrics::GPU_LOAD), 100.0), p.gpu);
    c.center(&clock(m, s, true), CENTER, 394.0, 44.0, p.detail, Weight::Bold, 255, 0.0);
    c.center(
        &format!("H₂O {}  ·  {}", temp(m, s.unit, metrics::COOLANT_TEMP), fmt(m, s.unit, metrics::PUMP_RPM)),
        CENTER,
        430.0,
        22.0,
        p.footer,
        Weight::Bold,
        255,
        0.0,
    );
    let _ = fg;
}

fn neon(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    let t = fraction(m.get(metrics::CPU_TEMP), 100.0).unwrap_or(0.0);
    let ring = RgbColor::lerp(s.accent, RgbColor::new(0xFF, 0x20, 0x50), ((t - 0.45) / 0.4).clamp(0.0, 1.0));
    for index in (1..=6).rev() {
        c.stroke_circle(CENTER, CENTER, 205.0, index as f32 * 6.0, ring, (18 + (6 - index) * 6) as u8);
    }
    c.stroke_circle(CENTER, CENTER, 205.0, 4.0, ring, 255);
    c.center(&clock(m, s, false), CENTER, 140.0, 30.0, fg, Weight::Bold, 255, 200.0);
    c.center(&temp(m, s.unit, metrics::CPU_TEMP), CENTER, 264.0, 124.0, fg, Weight::Bold, 255, 300.0);
    c.center(&format!("CPU  {}", fmt(m, s.unit, metrics::CPU_LOAD)), CENTER, 306.0, 30.0, ring, Weight::Bold, 255, 0.0);
    c.center(
        &format!(
            "GPU {}   ·   H₂O {}",
            temp(m, s.unit, metrics::GPU_TEMP),
            temp(m, s.unit, metrics::COOLANT_TEMP)
        ),
        CENTER,
        352.0,
        26.0,
        s.secondary,
        Weight::Bold,
        255,
        270.0,
    );
}

fn cpu_gpu(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.line(CENTER, 105.0, CENTER, 350.0, fg, 2.0, 60);
    let column = |c: &mut Canvas, cx: f32, title: &str, temp_key: &str, load_key: &str, clock_key: &str, power_key: &str, color: RgbColor| {
        c.center(title, cx, 124.0, 30.0, color, Weight::Bold, 255, 0.0);
        c.center(&temp(m, s.unit, temp_key), cx, 202.0, 76.0, fg, Weight::Bold, 255, 170.0);
        c.center(&fmt(m, s.unit, load_key), cx, 248.0, 34.0, color, Weight::Bold, 255, 170.0);
        c.center(&fmt(m, s.unit, clock_key), cx, 288.0, 24.0, fg, Weight::Bold, 210, 170.0);
        c.center(&fmt(m, s.unit, power_key), cx, 322.0, 24.0, fg, Weight::Bold, 210, 170.0);
    };
    column(c, 135.0, "CPU", metrics::CPU_TEMP, metrics::CPU_LOAD, metrics::CPU_CLOCK, metrics::CPU_POWER, s.accent);
    column(c, 345.0, "GPU", metrics::GPU_TEMP, metrics::GPU_LOAD, metrics::GPU_CLOCK, metrics::GPU_POWER, s.secondary);
    c.center(
        &format!("RAM {}  ·  H₂O {}", fmt(m, s.unit, metrics::RAM_LOAD), temp(m, s.unit, metrics::COOLANT_TEMP)),
        CENTER,
        398.0,
        26.0,
        fg,
        Weight::Bold,
        255,
        200.0,
    );
}

fn media_theme(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics, media: Option<&Pixmap>) {
    let fg = s.text_color;
    match media {
        Some(media) => c.draw_media(media),
        None => c.center("No image selected", CENTER, CENTER, 28.0, fg, Weight::Bold, 160, 0.0),
    }
    if s.media_overlay {
        c.fill_rect(0.0, 320.0, SIZE as f32, 160.0, RgbColor::BLACK, 130);
        c.center(&clock(m, s, false), CENTER, 384.0, 52.0, RgbColor::WHITE, Weight::Bold, 255, 0.0);
        c.center(
            &format!(
                "CPU {}   GPU {}",
                temp(m, s.unit, metrics::CPU_TEMP),
                temp(m, s.unit, metrics::GPU_TEMP)
            ),
            CENTER,
            422.0,
            26.0,
            RgbColor::WHITE,
            Weight::Bold,
            255,
            230.0,
        );
    }
}

fn minimal_clock(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center(&clock(m, s, false), CENTER, 240.0, 136.0, fg, Weight::Bold, 255, 0.0);
    c.center(
        &m.time.format("%A, %-d %B").to_string(),
        CENTER,
        292.0,
        30.0,
        s.accent,
        Weight::Bold,
        255,
        0.0,
    );
    c.center(
        &format!(
            "CPU {}   GPU {}",
            temp(m, s.unit, metrics::CPU_TEMP),
            temp(m, s.unit, metrics::GPU_TEMP)
        ),
        CENTER,
        352.0,
        28.0,
        fg,
        Weight::Bold,
        255,
        200.0,
    );
    c.center(&format!("H₂O {}", temp(m, s.unit, metrics::COOLANT_TEMP)), CENTER, 392.0, 26.0, s.secondary, Weight::Bold, 255, 0.0);
}

fn analog_clock(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    let (cy, radius) = (210.0f32, 148.0f32);
    c.stroke_circle(CENTER, cy, radius, 2.0, fg, 60);
    for index in 0..12 {
        let angle = index as f64 * std::f64::consts::PI / 6.0;
        let x0 = CENTER + (angle.sin() * (radius as f64 - 16.0)) as f32;
        let y0 = cy - (angle.cos() * (radius as f64 - 16.0)) as f32;
        let x1 = CENTER + (angle.sin() * (radius as f64 - 5.0)) as f32;
        let y1 = cy - (angle.cos() * (radius as f64 - 5.0)) as f32;
        c.line(x0, y0, x1, y1, fg, if index % 3 == 0 { 5.0 } else { 2.0 }, if index % 3 == 0 { 220 } else { 120 });
    }
    let time = m.time;
    let hour_angle = ((time.hour() % 12) as f64 + time.minute() as f64 / 60.0) * std::f64::consts::PI / 6.0;
    let minute_angle = (time.minute() as f64 + time.second() as f64 / 60.0) * std::f64::consts::PI / 30.0;
    let second_angle = time.second() as f64 * std::f64::consts::PI / 30.0;
    c.line(CENTER, cy, CENTER + (hour_angle.sin() * 84.0) as f32, cy - (hour_angle.cos() * 84.0) as f32, fg, 9.0, 255);
    c.line(CENTER, cy, CENTER + (minute_angle.sin() * 124.0) as f32, cy - (minute_angle.cos() * 124.0) as f32, fg, 5.0, 255);
    c.line(CENTER, cy, CENTER + (second_angle.sin() * 132.0) as f32, cy - (second_angle.cos() * 132.0) as f32, s.accent, 2.0, 255);
    c.fill_circle(CENTER, cy, 7.0, s.accent, 255);
    c.center(
        &format!(
            "{} CPU   {} GPU",
            temp(m, s.unit, metrics::CPU_TEMP),
            temp(m, s.unit, metrics::GPU_TEMP)
        ),
        CENTER,
        406.0,
        26.0,
        fg,
        Weight::Bold,
        255,
        220.0,
    );
    c.center(
        &time.format("%a %-d %b").to_string(),
        CENTER,
        438.0,
        20.0,
        s.secondary,
        Weight::Bold,
        255,
        200.0,
    );
}

fn cooling(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center("COOLING", CENTER, 56.0, 22.0, s.secondary, Weight::Bold, 255, 0.0);
    c.arc(CENTER, 212.0, 118.0, 18.0, fraction(m.get(metrics::COOLANT_TEMP), 60.0), s.accent);
    c.center(&temp(m, s.unit, metrics::COOLANT_TEMP), CENTER, 244.0, 88.0, fg, Weight::Bold, 255, 180.0);
    c.center("COOLANT", CENTER, 276.0, 20.0, s.accent, Weight::Bold, 255, 0.0);
    c.stat(134.0, 358.0, &number(m.get(metrics::PUMP_RPM)), "PUMP RPM", fg, fg);
    c.stat(346.0, 358.0, &number(aio_fan_rpm(m)), "FAN RPM", fg, s.secondary);
    c.center(&clock(m, s, false), CENTER, 430.0, 24.0, fg, Weight::Bold, 255, 200.0);
}

fn load_bars(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center(&clock(m, s, false), CENTER, 88.0, 26.0, fg, Weight::Bold, 255, 200.0);
    let column = |c: &mut Canvas, cx: f32, label: &str, metric: &str, color: RgbColor| {
        let filled = fraction(m.get(metric), 100.0).unwrap_or(0.0);
        c.center(&fmt(m, s.unit, metric), cx, 172.0, 34.0, color, Weight::Bold, 255, 112.0);
        c.fill_round_rect(cx - 20.0, 196.0, 40.0, 160.0, 20.0, fg, 30);
        if filled > 0.0 {
            let top = 356.0 - 160.0 * filled as f32;
            c.fill_round_rect(cx - 20.0, top, 40.0, 356.0 - top, 20.0, color, 255);
        }
        c.center(label, cx, 392.0, 24.0, color, Weight::Bold, 255, 120.0);
    };
    column(c, 148.0, "CPU", metrics::CPU_LOAD, s.accent);
    column(c, 240.0, "GPU", metrics::GPU_LOAD, s.secondary);
    column(c, 332.0, "RAM", metrics::RAM_LOAD, fg);
    c.center(
        &format!("{}  ·  {}", fmt(m, s.unit, metrics::CPU_CLOCK), fmt(m, s.unit, metrics::CPU_POWER)),
        CENTER,
        432.0,
        20.0,
        fg,
        Weight::Bold,
        170,
        220.0,
    );
}

fn power(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center("POWER DRAW", CENTER, 92.0, 24.0, s.secondary, Weight::Bold, 255, 0.0);
    c.line(CENTER, 148.0, CENTER, 282.0, fg, 2.0, 60);
    c.center(&fmt(m, s.unit, metrics::CPU_POWER), 150.0, 222.0, 64.0, s.accent, Weight::Bold, 255, 152.0);
    c.center("CPU", 150.0, 262.0, 24.0, s.accent, Weight::Bold, 255, 0.0);
    c.center(&fmt(m, s.unit, metrics::GPU_POWER), 330.0, 222.0, 64.0, s.secondary, Weight::Bold, 255, 152.0);
    c.center("GPU", 330.0, 262.0, 24.0, s.secondary, Weight::Bold, 255, 0.0);
    c.bar_row("CPU", fraction(m.get(metrics::CPU_LOAD), 100.0), &fmt(m, s.unit, metrics::CPU_LOAD), 324.0, s.accent, fg);
    c.bar_row("GPU", fraction(m.get(metrics::GPU_LOAD), 100.0), &fmt(m, s.unit, metrics::GPU_LOAD), 366.0, s.secondary, fg);
    c.center(
        &format!("H₂O {}   ·   {} rpm", temp(m, s.unit, metrics::COOLANT_TEMP), number(m.get(metrics::PUMP_RPM))),
        CENTER,
        426.0,
        20.0,
        fg,
        Weight::Bold,
        180,
        230.0,
    );
}

fn terminal(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.center("> system.status", CENTER, 86.0, 22.0, s.accent, Weight::Bold, 255, 0.0);
    c.line(84.0, 104.0, 396.0, 104.0, fg, 1.0, 70);
    let mut y = 146.0;
    let row = |c: &mut Canvas, label: &str, value: String, color: RgbColor, y: &mut f32| {
        c.text(label, 104.0, *y, 22.0, fg, Weight::Bold, 205, Align::Left, 0.0);
        c.text(&value, 376.0, *y, 22.0, color, Weight::Bold, 255, Align::Right, 0.0);
        *y += 33.0;
    };
    row(c, "cpu temp", temp(m, s.unit, metrics::CPU_TEMP), s.accent, &mut y);
    row(c, "gpu temp", temp(m, s.unit, metrics::GPU_TEMP), s.secondary, &mut y);
    row(c, "cpu load", fmt(m, s.unit, metrics::CPU_LOAD), fg, &mut y);
    row(c, "gpu load", fmt(m, s.unit, metrics::GPU_LOAD), fg, &mut y);
    row(
        c,
        "ram",
        format!("{} / {}", fmt(m, s.unit, metrics::RAM_USED_GB), fmt(m, s.unit, metrics::RAM_TOTAL_GB)),
        fg,
        &mut y,
    );
    row(c, "coolant", temp(m, s.unit, metrics::COOLANT_TEMP), s.accent, &mut y);
    row(c, "pump", number(m.get(metrics::PUMP_RPM)), s.secondary, &mut y);
    c.center(&clock(m, s, true), CENTER, 430.0, 22.0, fg, Weight::Bold, 255, 200.0);
}

fn split(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    c.line(46.0, 396.0, 434.0, 84.0, fg, 2.0, 55);
    c.center(&temp(m, s.unit, metrics::CPU_TEMP), 144.0, 168.0, 84.0, s.accent, Weight::Bold, 255, 176.0);
    c.center(&format!("CPU   {}", fmt(m, s.unit, metrics::CPU_LOAD)), 144.0, 206.0, 24.0, fg, Weight::Bold, 255, 176.0);
    c.center(&fmt(m, s.unit, metrics::CPU_CLOCK), 144.0, 240.0, 20.0, fg, Weight::Bold, 190, 176.0);
    c.center(&temp(m, s.unit, metrics::GPU_TEMP), 336.0, 300.0, 84.0, s.secondary, Weight::Bold, 255, 176.0);
    c.center(&format!("GPU   {}", fmt(m, s.unit, metrics::GPU_LOAD)), 336.0, 338.0, 24.0, fg, Weight::Bold, 255, 176.0);
    c.center(&fmt(m, s.unit, metrics::GPU_CLOCK), 336.0, 372.0, 20.0, fg, Weight::Bold, 190, 176.0);
    c.center(&clock(m, s, false), CENTER, 432.0, 24.0, fg, Weight::Bold, 255, 200.0);
}

fn multi_metric(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    let fg = s.text_color;
    let columns = [158.0, 322.0];
    let rows = [152.0, 256.0, 360.0];
    for (index, metric) in s.metrics.iter().take(6).enumerate() {
        let cx = columns[index % 2];
        let cy = rows[index / 2];
        let color = if index % 2 == 0 { s.accent } else { s.secondary };
        c.fill_round_rect(cx - 76.0, cy - 46.0, 152.0, 92.0, 16.0, s.text_color, 20);
        c.center(&fmt(m, s.unit, metric), cx, cy + 10.0, 40.0, fg, Weight::Bold, 255, 140.0);
        let label = metrics::find(metric)
            .map(|info| {
                info.label
                    .replace(" temperature", "")
                    .replace(" utilization", " load")
            })
            .unwrap_or_else(|| metric.clone());
        c.center(&label, cx, cy + 35.0, 18.0, color, Weight::Bold, 255, 140.0);
    }
    c.center(&clock(m, s, false), CENTER, 78.0, 30.0, fg, Weight::Bold, 255, 170.0);
}

fn custom(c: &mut Canvas, s: &LcdSettings, m: &LcdMetrics) {
    for widget in s.widgets.iter().take(LcdSettings::MAX_WIDGETS) {
        let color = widget.color.unwrap_or(s.text_color);
        match widget.kind {
            LcdWidgetType::Clock => c.center(
                &clock(m, s, false),
                widget.x,
                widget.y + widget.size * 0.35,
                widget.size,
                color,
                Weight::Bold,
                255,
                0.0,
            ),
            LcdWidgetType::Date => c.center(
                &m.time.format("%a %-d %b").to_string(),
                widget.x,
                widget.y + widget.size * 0.35,
                widget.size,
                color,
                Weight::Bold,
                255,
                0.0,
            ),
            LcdWidgetType::Text => c.center(
                widget.text.as_deref().unwrap_or(""),
                widget.x,
                widget.y + widget.size * 0.35,
                widget.size,
                color,
                Weight::Bold,
                255,
                0.0,
            ),
            LcdWidgetType::Metric => {
                c.center(
                    &fmt(m, s.unit, &widget.metric),
                    widget.x,
                    widget.y + widget.size * 0.35,
                    widget.size,
                    color,
                    Weight::Bold,
                    255,
                    0.0,
                );
                if let Some(label) = &widget.label {
                    if !label.is_empty() {
                        c.center(
                            label,
                            widget.x,
                            widget.y + widget.size * 0.35 + widget.size * 0.6,
                            widget.size * 0.45,
                            widget.color.unwrap_or(s.accent),
                            Weight::Bold,
                            255,
                            0.0,
                        );
                    }
                }
            }
            LcdWidgetType::Gauge => {
                let filled = fraction(m.get(&widget.metric), 100.0);
                let gauge_color = widget.color.unwrap_or(s.accent);
                if s.gauge_style == GaugeStyle::Bar {
                    c.bar(
                        widget.x - widget.size / 2.0,
                        widget.y,
                        widget.size,
                        12.0,
                        filled,
                        gauge_color,
                    );
                } else {
                    c.gauge(
                        widget.x,
                        widget.y,
                        widget.size / 2.0,
                        filled,
                        gauge_color,
                        s.gauge_style == GaugeStyle::Ring,
                    );
                }
                c.center(
                    &fmt(m, s.unit, &widget.metric),
                    widget.x,
                    widget.y + widget.size * 0.12,
                    widget.size * 0.32,
                    s.text_color,
                    Weight::Bold,
                    255,
                    widget.size * 0.8,
                );
                if let Some(label) = &widget.label {
                    if !label.is_empty() {
                        c.center(
                            label,
                            widget.x,
                            widget.y + widget.size * 0.32,
                            widget.size * 0.16,
                            gauge_color,
                            Weight::Bold,
                            255,
                            0.0,
                        );
                    }
                }
            }
            LcdWidgetType::Bar => {
                c.bar(
                    widget.x - widget.size,
                    widget.y,
                    widget.size * 2.0,
                    (widget.size / 6.0).max(6.0),
                    fraction(m.get(&widget.metric), 100.0),
                    widget.color.unwrap_or(s.accent),
                );
                if let Some(label) = &widget.label {
                    c.center(
                        &format!("{label} {}", fmt(m, s.unit, &widget.metric)),
                        widget.x,
                        widget.y - 8.0,
                        (widget.size / 4.0).max(14.0),
                        s.text_color,
                        Weight::Bold,
                        255,
                        0.0,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn metrics() -> LcdMetrics {
        LcdMetrics::new(
            BTreeMap::from([
                (metrics::CPU_TEMP.to_string(), 47.0),
                (metrics::GPU_TEMP.to_string(), 38.0),
                (metrics::COOLANT_TEMP.to_string(), 29.0),
                (metrics::CPU_LOAD.to_string(), 12.0),
                (metrics::GPU_LOAD.to_string(), 3.0),
                (metrics::RAM_LOAD.to_string(), 41.0),
                (metrics::PUMP_RPM.to_string(), 2450.0),
                (metrics::AIO_FAN1.to_string(), 1180.0),
            ]),
            Local.with_ymd_and_hms(2026, 11, 7, 17, 4, 0).unwrap(),
        )
    }

    #[test]
    fn catalogue_has_every_theme_from_the_windows_app() {
        assert_eq!(themes().len(), 21);
        assert_eq!(themes()[0].id, "minimal-temps");
        assert!(themes().iter().any(|theme| theme.id == "custom"));
    }

    #[test]
    fn temperature_and_clock_formatting_match_the_reference() {
        assert_eq!(format_temp(Some(47.4), TemperatureUnit::Celsius), "47°");
        assert_eq!(format_temp(None, TemperatureUnit::Celsius), "—°");
        assert_eq!(
            format_temp(Some(38.0), TemperatureUnit::Fahrenheit),
            "100°"
        );
        let time = Local.with_ymd_and_hms(2026, 11, 7, 17, 4, 9).unwrap();
        assert_eq!(clock_text(time, true, false), "17:04");
        assert_eq!(clock_text(time, false, false), "5:04 PM");
        assert_eq!(clock_text(time, false, true), "5:04:09 PM");
    }

    #[test]
    fn metric_formatting_uses_the_documented_units() {
        assert_eq!(format_metric(metrics::CPU_CLOCK, Some(4850.0), TemperatureUnit::Celsius), "4.85 GHz");
        assert_eq!(format_metric(metrics::GPU_POWER, Some(31.4), TemperatureUnit::Celsius), "31 W");
        assert_eq!(format_metric(metrics::RAM_XMP, Some(1.0), TemperatureUnit::Celsius), "On");
        assert_eq!(format_metric(metrics::RAM_XMP, Some(0.0), TemperatureUnit::Celsius), "Off");
        assert_eq!(format_metric(metrics::CPU_TEMP, None, TemperatureUnit::Celsius), "—");
    }

    #[test]
    fn every_theme_renders_to_a_valid_frame() {
        let mut fonts = FontCache::new();
        let mut settings = LcdSettings::default();
        for theme in themes() {
            settings.theme_id = theme.id.clone();
            let frame = render(&settings, &metrics(), None, &mut fonts);
            assert_eq!(frame.width(), SIZE);
            assert_eq!(frame.height(), SIZE);
        }
    }
}
