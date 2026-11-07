//! Temperature-to-duty fan curves with the safety rules.
//!
//! Ported from `MyLianPC.Core/Cooling/FanCurve.cs` and `CoolingPresets.cs`.
//! These invariants are not negotiable:
//!
//! * every curve reaches 100% by 90 °C;
//! * a missing or implausible reading commands the failsafe duty (100%);
//! * duty never falls below the curve minimum;
//! * duty rises immediately but falls only after the temperature drops by the
//!   hysteresis amount.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemperatureSource {
    CpuPackage,
    GpuCore,
    Coolant,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub temperature_c: f64,
    pub duty_percent: i32,
}

impl CurvePoint {
    pub const fn new(temperature_c: f64, duty_percent: i32) -> Self {
        Self {
            temperature_c,
            duty_percent,
        }
    }
}

/// Above this temperature every curve must command 100%.
pub const CRITICAL_TEMPERATURE_C: f64 = 90.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FanCurve {
    pub source: TemperatureSource,
    pub points: Vec<CurvePoint>,
    /// Lowest duty the curve may command. Adapters raise this to the hardware
    /// minimum if it is higher.
    pub minimum_duty_percent: i32,
    /// Degrees the temperature must drop before duty may decrease.
    pub hysteresis_c: f64,
    pub failsafe_duty_percent: i32,
}

impl FanCurve {
    pub fn new(source: TemperatureSource, points: Vec<CurvePoint>) -> Self {
        Self {
            source,
            points,
            minimum_duty_percent: 20,
            hysteresis_c: 3.0,
            failsafe_duty_percent: 100,
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.points.len() < 2 {
            problems.push("A curve needs at least two points.".to_string());
        }
        if !(0..=100).contains(&self.minimum_duty_percent) {
            problems.push("Minimum duty must be between 0 and 100%.".to_string());
        }
        if !(50..=100).contains(&self.failsafe_duty_percent) {
            problems.push("Failsafe duty must be between 50 and 100%.".to_string());
        }
        if !(0.0..=10.0).contains(&self.hysteresis_c) {
            problems.push("Hysteresis must be between 0 and 10 °C.".to_string());
        }

        for (i, p) in self.points.iter().enumerate() {
            if !(0.0..=110.0).contains(&p.temperature_c) {
                problems.push(format!(
                    "Point {}: temperature {} °C is out of range (0-110).",
                    i + 1,
                    p.temperature_c
                ));
            }
            if !(0..=100).contains(&p.duty_percent) {
                problems.push(format!(
                    "Point {}: duty {}% is out of range (0-100).",
                    i + 1,
                    p.duty_percent
                ));
            }
            if p.duty_percent < self.minimum_duty_percent {
                problems.push(format!(
                    "Point {}: duty {}% is below the minimum of {}%.",
                    i + 1,
                    p.duty_percent,
                    self.minimum_duty_percent
                ));
            }
            if i > 0 {
                let prev = self.points[i - 1];
                if p.temperature_c <= prev.temperature_c {
                    problems.push(format!("Point {}: temperatures must strictly increase.", i + 1));
                }
                if p.duty_percent < prev.duty_percent {
                    problems.push(format!(
                        "Point {}: duty must not decrease as temperature rises.",
                        i + 1
                    ));
                }
            }
        }

        if self.points.len() >= 2
            && self
                .points
                .iter()
                .all(|p| (0..=100).contains(&p.duty_percent))
        {
            let at_critical = self.interpolate(CRITICAL_TEMPERATURE_C);
            if at_critical < 100 {
                problems.push(format!(
                    "The curve must reach 100% by {CRITICAL_TEMPERATURE_C} °C (it commands {at_critical}%)."
                ));
            }
        }
        problems
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }

    /// Stateless duty for a temperature, clamped to the minimum.
    /// A missing or implausible reading returns the failsafe duty.
    pub fn evaluate(&self, temperature_c: Option<f64>) -> i32 {
        let Some(t) = temperature_c else {
            return self.failsafe_duty_percent;
        };
        if !t.is_finite() || !(-20.0..=150.0).contains(&t) {
            return self.failsafe_duty_percent;
        }
        if t >= CRITICAL_TEMPERATURE_C {
            return 100;
        }
        self.minimum_duty_percent.max(self.interpolate(t))
    }

    fn interpolate(&self, t: f64) -> i32 {
        let Some(first) = self.points.first() else {
            return self.failsafe_duty_percent;
        };
        if t <= first.temperature_c {
            return first.duty_percent;
        }
        for pair in self.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t <= b.temperature_c {
                let f = (t - a.temperature_c) / (b.temperature_c - a.temperature_c);
                return (a.duty_percent as f64
                    + f * (b.duty_percent - a.duty_percent) as f64)
                    .round() as i32;
            }
        }
        // Past the last point the curve holds its last duty; Validate() rejects
        // curves below 100% at the critical temperature.
        self.points.last().map(|p| p.duty_percent).unwrap_or(self.failsafe_duty_percent)
    }
}

/// Applies a curve with hysteresis: duty rises immediately but only falls once
/// the temperature drops by [`FanCurve::hysteresis_c`].
#[derive(Clone, Debug)]
pub struct FanCurveController {
    curve: FanCurve,
    temp_at_last_duty: Option<f64>,
    last_duty: Option<i32>,
}

impl FanCurveController {
    pub fn new(curve: FanCurve) -> Self {
        Self {
            curve,
            temp_at_last_duty: None,
            last_duty: None,
        }
    }

    pub fn curve(&self) -> &FanCurve {
        &self.curve
    }

    pub fn next(&mut self, temperature_c: Option<f64>) -> i32 {
        let target = self.curve.evaluate(temperature_c);
        if temperature_c.is_none() || self.last_duty.is_none() || target >= self.last_duty.unwrap() {
            self.last_duty = Some(target);
            self.temp_at_last_duty = temperature_c;
            return target;
        }
        if let (Some(t0), Some(t)) = (self.temp_at_last_duty, temperature_c) {
            if t0 - t >= self.curve.hysteresis_c {
                self.last_duty = Some(target);
                self.temp_at_last_duty = temperature_c;
            }
        }
        self.last_duty.unwrap()
    }
}

pub mod presets {
    use super::{CurvePoint, FanCurve, TemperatureSource};

    fn p(temperature_c: f64, duty_percent: i32) -> CurvePoint {
        CurvePoint::new(temperature_c, duty_percent)
    }

    pub fn silent(source: TemperatureSource) -> FanCurve {
        let points = if source == TemperatureSource::Coolant {
            vec![p(25.0, 25), p(30.0, 30), p(34.0, 45), p(38.0, 70), p(42.0, 100)]
        } else {
            vec![p(30.0, 25), p(50.0, 30), p(65.0, 45), p(75.0, 70), p(85.0, 100)]
        };
        FanCurve::new(source, points)
    }

    /// Chassis fans: hold the hardware's lowest duty through normal idle and
    /// light-load temperatures, then ramp quickly once the CPU is genuinely hot.
    pub fn case_fans(source: TemperatureSource) -> FanCurve {
        let points = if source == TemperatureSource::Coolant {
            vec![p(25.0, 20), p(40.0, 25), p(45.0, 45), p(50.0, 70), p(54.0, 100)]
        } else {
            vec![p(30.0, 20), p(60.0, 25), p(70.0, 40), p(80.0, 70), p(85.0, 100)]
        };
        FanCurve::new(source, points)
    }

    pub fn balanced(source: TemperatureSource) -> FanCurve {
        let points = if source == TemperatureSource::Coolant {
            vec![p(25.0, 30), p(30.0, 40), p(34.0, 55), p(38.0, 80), p(40.0, 100)]
        } else {
            vec![p(30.0, 30), p(50.0, 40), p(65.0, 60), p(75.0, 80), p(85.0, 100)]
        };
        FanCurve::new(source, points)
    }

    pub fn performance(source: TemperatureSource) -> FanCurve {
        let points = if source == TemperatureSource::Coolant {
            vec![p(25.0, 45), p(30.0, 60), p(34.0, 80), p(37.0, 100)]
        } else {
            vec![p(30.0, 45), p(50.0, 60), p(65.0, 80), p(75.0, 100)]
        };
        FanCurve::new(source, points)
    }

    pub fn maximum(source: TemperatureSource) -> FanCurve {
        let mut curve = FanCurve::new(
            source,
            if source == TemperatureSource::Coolant {
                vec![p(20.0, 100), p(60.0, 100)]
            } else {
                vec![p(20.0, 100), p(90.0, 100)]
            },
        );
        curve.minimum_duty_percent = 100;
        curve
    }

    /// Every preset with the name the UI shows, in button order.
    pub type PresetFn = fn(TemperatureSource) -> FanCurve;

    pub const ALL: [(&str, PresetFn); 5] = [
        ("Silent", silent),
        ("Case fans", case_fans),
        ("Balanced", balanced),
        ("Performance", performance),
        ("Maximum cooling", maximum),
    ];

    /// Name of the built-in preset this curve came from, or `None` when edited.
    /// Only the points are compared, matching the original behaviour.
    pub fn name_of(curve: &FanCurve) -> Option<&'static str> {
        ALL.iter()
            .find(|(_, make)| make(curve.source).points == curve.points)
            .map(|(name, _)| *name)
    }
}

#[cfg(test)]
mod tests {
    use super::presets;
    use super::*;

    fn balanced_curve() -> FanCurve {
        presets::balanced(TemperatureSource::CpuPackage)
    }

    #[test]
    fn all_presets_are_valid() {
        for source in [
            TemperatureSource::CpuPackage,
            TemperatureSource::GpuCore,
            TemperatureSource::Coolant,
        ] {
            for (name, make) in presets::ALL {
                let curve = make(source);
                assert!(
                    curve.is_valid(),
                    "{name} for {source:?}: {:?}",
                    curve.validate()
                );
            }
        }
    }

    #[test]
    fn critical_temperature_commands_full_duty() {
        let curve = balanced_curve();
        assert_eq!(curve.evaluate(Some(90.0)), 100);
        assert_eq!(curve.evaluate(Some(99.0)), 100);
    }

    #[test]
    fn missing_or_implausible_reading_is_failsafe() {
        let curve = balanced_curve();
        for reading in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-30.0), Some(200.0)] {
            assert_eq!(curve.evaluate(reading), curve.failsafe_duty_percent, "{reading:?}");
        }
    }

    #[test]
    fn duty_never_below_minimum() {
        let mut curve = balanced_curve();
        curve.minimum_duty_percent = 40;
        assert!(curve.evaluate(Some(20.0)) >= 40);
    }

    #[test]
    fn interpolation_matches_the_original() {
        let curve = presets::silent(TemperatureSource::CpuPackage);
        // 80 °C sits halfway between (75, 70) and (85, 100).
        assert_eq!(curve.evaluate(Some(80.0)), 85);
        assert_eq!(curve.evaluate(Some(30.0)), 25);
    }

    #[test]
    fn hysteresis_holds_falling_duty() {
        let mut controller = FanCurveController::new(presets::silent(TemperatureSource::CpuPackage));
        let high = controller.next(Some(80.0));
        assert_eq!(high, 85);
        // 79 °C is only 1 °C below the last reading: hold the duty.
        assert_eq!(controller.next(Some(79.0)), high);
        // 76 °C is 4 °C below: allow the lower target.
        let lower = controller.next(Some(76.0));
        assert!(lower < high);
    }

    #[test]
    fn preset_identity_ignores_minimum_and_hysteresis() {
        let mut curve = presets::silent(TemperatureSource::CpuPackage);
        curve.minimum_duty_percent = 25;
        curve.hysteresis_c = 5.0;
        assert_eq!(
            presets::name_of(&curve),
            Some("Silent")
        );
        curve.points[0].duty_percent = 26;
        assert_eq!(presets::name_of(&curve), None);
    }

    #[test]
    fn invalid_curves_are_rejected() {
        let mut curve = balanced_curve();
        curve.points = vec![CurvePoint::new(50.0, 40)];
        assert!(!curve.is_valid());

        let mut curve = balanced_curve();
        curve.points = vec![
            CurvePoint::new(50.0, 40),
            CurvePoint::new(50.0, 50),
            CurvePoint::new(60.0, 70),
        ];
        assert!(curve.validate().iter().any(|p| p.contains("strictly increase")));
    }
}
