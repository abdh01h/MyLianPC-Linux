//! Connection information and traffic accounting.
//!
//! Ported from `MyLianPC.Core/Network/{NetworkModels,TrafficMeter}.cs`. The
//! Linux collector lives in `mylianpc-hardware::network`; this module holds the
//! wire model and the pure rate/daily-total maths so it can be unit tested.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InterfaceInfo {
    pub name: String,
    pub mac: Option<String>,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub up: bool,
    pub link_speed_mbps: Option<u64>,
    pub wireless: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkSnapshot {
    pub time_utc: DateTime<Utc>,
    pub interfaces: Vec<InterfaceInfo>,
    /// Interface carrying the default route.
    pub primary: Option<String>,
    pub ssid: Option<String>,
    pub signal_dbm: Option<i32>,
    pub band: Option<String>,
    pub channel: Option<u32>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub download_bytes_per_second: f64,
    pub upload_bytes_per_second: f64,
    pub downloaded_today_gb: f64,
    pub uploaded_today_gb: f64,
    pub rx_bytes_total: u64,
    pub tx_bytes_total: u64,
}

impl Default for NetworkSnapshot {
    fn default() -> Self {
        Self {
            time_utc: Utc::now(),
            interfaces: Vec::new(),
            primary: None,
            ssid: None,
            signal_dbm: None,
            band: None,
            channel: None,
            gateway: None,
            dns: Vec::new(),
            download_bytes_per_second: 0.0,
            upload_bytes_per_second: 0.0,
            downloaded_today_gb: 0.0,
            uploaded_today_gb: 0.0,
            rx_bytes_total: 0,
            tx_bytes_total: 0,
        }
    }
}

/// Persisted daily totals.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TrafficState {
    pub day: NaiveDate,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

impl Default for TrafficState {
    fn default() -> Self {
        Self {
            day: Utc::now().date_naive(),
            rx_bytes: 0,
            tx_bytes: 0,
        }
    }
}

/// Tracks the counter deltas and today's totals across samples.
#[derive(Clone, Debug, Default)]
pub struct TrafficMeter {
    last: Option<(u64, u64, DateTime<Utc>)>,
    pub state: TrafficState,
}

impl TrafficMeter {
    pub fn new(state: TrafficState) -> Self {
        Self {
            last: None,
            state,
        }
    }

    /// Feeds cumulative interface counters. Returns the instantaneous rates.
    pub fn update(
        &mut self,
        rx_bytes: u64,
        tx_bytes: u64,
        now: DateTime<Utc>,
    ) -> (f64, f64) {
        if self.roll_day(now) {
            // Do not attribute the traffic between the last sample and midnight
            // to the new day; the next sample starts a fresh measurement.
            self.last = None;
        }

        let (down, up) = match self.last {
            Some((last_rx, last_tx, last_time)) => {
                let seconds = (now - last_time).num_milliseconds() as f64 / 1000.0;
                if seconds <= 0.0 {
                    (0.0, 0.0)
                } else {
                    let rx_delta = rx_bytes.saturating_sub(last_rx);
                    let tx_delta = tx_bytes.saturating_sub(last_tx);
                    self.state.rx_bytes = self.state.rx_bytes.saturating_add(rx_delta);
                    self.state.tx_bytes = self.state.tx_bytes.saturating_add(tx_delta);
                    (rx_delta as f64 / seconds, tx_delta as f64 / seconds)
                }
            }
            None => (0.0, 0.0),
        };
        self.last = Some((rx_bytes, tx_bytes, now));
        (down, up)
    }

    fn roll_day(&mut self, now: DateTime<Utc>) -> bool {
        let today = now.date_naive();
        if self.state.day != today {
            self.state.day = today;
            self.state.rx_bytes = 0;
            self.state.tx_bytes = 0;
            true
        } else {
            false
        }
    }

    pub fn downloaded_today_gb(&self) -> f64 {
        self.state.rx_bytes as f64 / 1_000_000_000.0
    }

    pub fn uploaded_today_gb(&self) -> f64 {
        self.state.tx_bytes as f64 / 1_000_000_000.0
    }
}

/// Human-readable byte rate, matching the Windows dashboard's style.
pub fn rate_text(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1_000_000_000.0 {
        format!("{:.2} GB/s", bytes_per_second / 1_000_000_000.0)
    } else if bytes_per_second >= 1_000_000.0 {
        format!("{:.2} MB/s", bytes_per_second / 1_000_000.0)
    } else if bytes_per_second >= 1_000.0 {
        format!("{:.1} KB/s", bytes_per_second / 1_000.0)
    } else {
        format!("{bytes_per_second:.0} B/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    #[test]
    fn first_sample_has_no_rate_and_no_totals() {
        let mut meter = TrafficMeter::default();
        let (down, up) = meter.update(1000, 500, at(100));
        assert_eq!((down, up), (0.0, 0.0));
        assert_eq!(meter.state.rx_bytes, 0);
    }

    #[test]
    fn deltas_become_rates_and_daily_totals() {
        let mut meter = TrafficMeter::default();
        meter.update(1000, 500, at(100));
        let (down, up) = meter.update(3000, 1500, at(102));
        assert_eq!(down, 1000.0);
        assert_eq!(up, 500.0);
        assert_eq!(meter.state.rx_bytes, 2000);
        assert_eq!(meter.state.tx_bytes, 1000);
    }

    #[test]
    fn counters_never_underflow_after_a_reset() {
        let mut meter = TrafficMeter::default();
        meter.update(10_000, 10_000, at(100));
        let (down, up) = meter.update(10, 20, at(101));
        assert_eq!((down, up), (0.0, 0.0));
        assert_eq!(meter.state.rx_bytes, 0);
    }

    #[test]
    fn a_new_day_resets_the_totals() {
        let mut meter = TrafficMeter::default();
        meter.state.day = at(0).date_naive();
        meter.state.rx_bytes = 5000;
        meter.update(1000, 1000, at(100));
        let next_day = at(100).date_naive().succ_opt().unwrap();
        meter.state.day = next_day;
        meter.update(2000, 2000, at(200));
        assert_eq!(meter.state.rx_bytes, 0);
    }

    #[test]
    fn rates_are_formatted_in_decimal_units() {
        assert_eq!(rate_text(500.0), "500 B/s");
        assert_eq!(rate_text(1500.0), "1.5 KB/s");
        assert_eq!(rate_text(2_500_000.0), "2.50 MB/s");
    }
}
