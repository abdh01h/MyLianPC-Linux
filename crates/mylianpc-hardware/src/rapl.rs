//! CPU package power from the Linux power-capping (RAPL) interface.
//!
//! AMD Zen exposes package energy through `intel-rapl`-compatible sysfs on the
//! kernels used here. The value is a cumulative micro-joule counter, so power is
//! the delta between two samples divided by the elapsed time.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Rapl {
    energy_path: Option<PathBuf>,
    max_range: u64,
    last: Option<(u64, Instant)>,
}

impl Default for Rapl {
    fn default() -> Self {
        Self::new()
    }
}

impl Rapl {
    pub fn new() -> Self {
        let root = Path::new("/sys/class/powercap");
        let mut energy_path = None;
        let mut max_range = 0;
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = fs::read_to_string(path.join("name")).unwrap_or_default();
                if name.trim() != "package-0" {
                    continue;
                }
                energy_path = Some(path.join("energy_uj"));
                max_range = fs::read_to_string(path.join("max_energy_range_uj"))
                    .ok()
                    .and_then(|text| text.trim().parse().ok())
                    .unwrap_or(0);
                break;
            }
        }
        Self {
            energy_path,
            max_range,
            last: None,
        }
    }

    pub fn available(&self) -> bool {
        self.energy_path.is_some()
    }

    /// Reads the counter and returns the average power since the previous call.
    pub fn sample(&mut self) -> Option<f64> {
        let path = self.energy_path.as_ref()?;
        let energy: u64 = fs::read_to_string(path).ok()?.trim().parse().ok()?;
        let now = Instant::now();
        let result = match self.last {
            Some((previous, at)) => {
                let seconds = now.duration_since(at).as_secs_f64();
                if seconds <= 0.0 {
                    None
                } else {
                    Some(power_watts(previous, energy, self.max_range, seconds))
                }
            }
            None => None,
        };
        self.last = Some((energy, now));
        result
    }
}

/// Converts a counter delta (with wrap-around) into watts.
pub fn power_watts(previous: u64, current: u64, max_range: u64, seconds: f64) -> f64 {
    let delta = if current >= previous {
        current - previous
    } else if max_range > 0 {
        max_range.saturating_sub(previous).saturating_add(current)
    } else {
        0
    };
    (delta as f64 / 1_000_000.0 / seconds).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_is_the_microjoule_delta_over_time() {
        // 20 J over 2 s = 10 W.
        assert!((power_watts(1_000_000, 21_000_000, 0, 2.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn counter_wrap_is_handled() {
        // 10 J before the wrap and 5 J after, 1 s apart.
        assert!((power_watts(90_000_000, 5_000_000, 100_000_000, 1.0) - 15.0).abs() < 1e-9);
    }

    #[test]
    fn zero_time_is_guarded_by_the_caller() {
        assert_eq!(power_watts(1, 2, 0, 1.0), 0.000001);
    }
}
