//! Linux connection information and traffic sampling.
//!
//! Mirrors `MyLianPC.Service/NetworkMonitor.cs` with Linux sources:
//! `/proc/net/dev`, `/proc/net/route`, `getifaddrs(3)`, `/etc/resolv.conf` and
//! `iw` for the wireless link details.

use std::fs;
use std::process::Command;

use chrono::Utc;
use mylianpc_core::network::{InterfaceInfo, NetworkSnapshot, TrafficMeter, TrafficState};

pub struct NetworkMonitor {
    meter: TrafficMeter,
}

impl NetworkMonitor {
    pub fn new(state: TrafficState) -> Self {
        Self {
            meter: TrafficMeter::new(state),
        }
    }

    pub fn state(&self) -> &TrafficState {
        &self.meter.state
    }

    pub fn sample(&mut self) -> NetworkSnapshot {
        let counters = interface_counters();
        let route = default_route();
        let primary = route.as_ref().map(|route| route.0.clone());
        let (rx, tx) = primary
            .as_ref()
            .and_then(|name| counters.iter().find(|entry| &entry.0 == name))
            .map(|entry| (entry.1, entry.2))
            .unwrap_or_else(|| {
                counters
                    .iter()
                    .filter(|entry| entry.0 != "lo")
                    .fold((0, 0), |(rx, tx), entry| {
                        (rx + entry.1, tx + entry.2)
                    })
            });

        let now = Utc::now();
        let (down, up) = self.meter.update(rx, tx, now);
        let addresses = interface_addresses();
        let mut interfaces = interface_info(&addresses);
        let wireless = primary
            .as_ref()
            .map(|name| is_wireless(name))
            .unwrap_or(false);
        let wireless_details = if wireless {
            primary.as_deref().and_then(wifi_link)
        } else {
            None
        };
        let link_speed = primary
            .as_ref()
            .and_then(|name| link_speed(name))
            .or_else(|| wireless_details.as_ref().and_then(|details| details.bitrate_mbps));

        if let Some(primary_name) = &primary {
            if let Some(interface) = interfaces.iter_mut().find(|item| &item.name == primary_name) {
                interface.link_speed_mbps = link_speed;
            }
        }

        NetworkSnapshot {
            time_utc: now,
            interfaces,
            primary,
            ssid: wireless_details.as_ref().and_then(|details| details.ssid.clone()),
            signal_dbm: wireless_details.as_ref().and_then(|details| details.signal_dbm),
            band: wireless_details.as_ref().and_then(|details| details.band.clone()),
            channel: wireless_details.as_ref().and_then(|details| details.channel),
            gateway: route.map(|route| route.1),
            dns: dns_servers(),
            download_bytes_per_second: down,
            upload_bytes_per_second: up,
            downloaded_today_gb: self.meter.downloaded_today_gb(),
            uploaded_today_gb: self.meter.uploaded_today_gb(),
            rx_bytes_total: rx,
            tx_bytes_total: tx,
        }
    }
}

impl Default for NetworkMonitor {
    fn default() -> Self {
        Self::new(TrafficState::default())
    }
}

fn interface_counters() -> Vec<(String, u64, u64)> {
    let Ok(text) = fs::read_to_string("/proc/net/dev") else {
        return Vec::new();
    };
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let name = name.trim().to_string();
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if fields.len() < 9 {
                return None;
            }
            let rx = fields[0].parse().ok()?;
            let tx = fields[8].parse().ok()?;
            Some((name, rx, tx))
        })
        .collect()
}

/// Returns (interface, gateway) of the default route.
fn default_route() -> Option<(String, String)> {
    let text = fs::read_to_string("/proc/net/route").ok()?;
    let mut best: Option<(u32, String, String)> = None;
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 {
            continue;
        }
        if fields[1] != "00000000" {
            continue;
        }
        let flags = u32::from_str_radix(fields[3], 16).unwrap_or(0);
        if flags & 0x1 == 0 {
            continue;
        }
        let metric = fields[6].parse::<u32>().unwrap_or(u32::MAX);
        let gateway = ipv4_from_le_hex(fields[2]);
        match &best {
            Some((best_metric, _, _)) if *best_metric <= metric => {}
            _ => best = Some((metric, fields[0].to_string(), gateway)),
        }
    }
    best.map(|(_, interface, gateway)| (interface, gateway))
}

fn ipv4_from_le_hex(value: &str) -> String {
    let number = u32::from_str_radix(value, 16).unwrap_or(0);
    let bytes = number.to_le_bytes();
    format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])
}

fn interface_addresses() -> Vec<(String, String, bool)> {
    let mut result = Vec::new();
    // SAFETY: getifaddrs allocates the list and freeifaddrs releases it.
    unsafe {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return result;
        }
        let mut current = list;
        while !current.is_null() {
            let item = &*current;
            if !item.ifa_addr.is_null() {
                let family = (*item.ifa_addr).sa_family as i32;
                if let Some(name) = std::ffi::CStr::from_ptr(item.ifa_name)
                    .to_str()
                    .ok()
                    .map(str::to_string)
                {
                    if family == libc::AF_INET {
                        let address = &*(item.ifa_addr as *const libc::sockaddr_in);
                        let bytes = address.sin_addr.s_addr.to_ne_bytes();
                        result.push((
                            name,
                            format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3]),
                            false,
                        ));
                    } else if family == libc::AF_INET6 {
                        let address = &*(item.ifa_addr as *const libc::sockaddr_in6);
                        let bytes = address.sin6_addr.s6_addr;
                        let ip = std::net::Ipv6Addr::from(bytes);
                        if !ip.is_loopback() && !ip.is_unicast_link_local() {
                            result.push((name, ip.to_string(), true));
                        }
                    }
                }
            }
            current = item.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    result
}

fn interface_info(addresses: &[(String, String, bool)]) -> Vec<InterfaceInfo> {
    let Ok(entries) = fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut interfaces = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "lo" {
            continue;
        }
        let path = entry.path();
        let mac = fs::read_to_string(path.join("address"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| value != "00:00:00:00:00:00");
        let state = fs::read_to_string(path.join("operstate"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let ipv4 = addresses
            .iter()
            .filter(|(interface, _, v6)| interface == &name && !v6)
            .map(|(_, address, _)| address.clone())
            .collect();
        let ipv6 = addresses
            .iter()
            .filter(|(interface, _, v6)| interface == &name && *v6)
            .map(|(_, address, _)| address.clone())
            .collect();
        interfaces.push(InterfaceInfo {
            name: name.clone(),
            mac,
            ipv4,
            ipv6,
            up: state == "up",
            link_speed_mbps: if state == "up" { link_speed(&name) } else { None },
            wireless: is_wireless(&name),
        });
    }
    interfaces.sort_by(|a, b| b.up.cmp(&a.up).then_with(|| a.name.cmp(&b.name)));
    interfaces
}

fn is_wireless(name: &str) -> bool {
    std::path::Path::new(&format!("/sys/class/net/{name}/wireless")).exists()
}

fn link_speed(name: &str) -> Option<u64> {
    let value = fs::read_to_string(format!("/sys/class/net/{name}/speed")).ok()?;
    let value = value.trim().parse::<i64>().ok()?;
    (value > 0).then_some(value as u64)
}

fn dns_servers() -> Vec<String> {
    let Ok(text) = fs::read_to_string("/etc/resolv.conf") else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            match fields.next() {
                Some("nameserver") => fields.next().map(str::to_string),
                _ => None,
            }
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
struct WifiLink {
    ssid: Option<String>,
    signal_dbm: Option<i32>,
    band: Option<String>,
    channel: Option<u32>,
    bitrate_mbps: Option<u64>,
}

fn wifi_link(interface: &str) -> Option<WifiLink> {
    let output = Command::new("iw")
        .args(["dev", interface, "link"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_iw_link(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_iw_link(text: &str) -> WifiLink {
    let mut link = WifiLink::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("SSID:") {
            link.ssid = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("signal:") {
            link.signal_dbm = value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok());
        } else if let Some(value) = line.strip_prefix("freq:") {
            if let Ok(mhz) = value.trim().parse::<u32>() {
                let (band, channel) = band_and_channel(mhz);
                link.band = Some(band);
                link.channel = channel;
            }
        } else if line.contains("bitrate:") {
            if let Some(value) = line.split("bitrate:").nth(1) {
                let number = value
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<f64>().ok());
                if let Some(number) = number {
                    link.bitrate_mbps = Some(number.round() as u64);
                }
            }
        }
    }
    link
}

fn band_and_channel(frequency_mhz: u32) -> (String, Option<u32>) {
    match frequency_mhz {
        2400..=2500 => (
            "2.4 GHz".to_string(),
            Some(if frequency_mhz == 2484 {
                14
            } else {
                (frequency_mhz.saturating_sub(2407)) / 5
            }),
        ),
        4900..=5950 => (
            "5 GHz".to_string(),
            Some(frequency_mhz.saturating_sub(5000) / 5),
        ),
        5951..=7150 => (
            "6 GHz".to_string(),
            Some(frequency_mhz.saturating_sub(5950) / 5),
        ),
        _ => ("Unknown".to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_route_parsing_uses_little_endian_gateway_bytes() {
        assert_eq!(ipv4_from_le_hex("0102A8C0"), "192.168.2.1");
    }

    #[test]
    fn wifi_channels_match_the_common_bands() {
        assert_eq!(band_and_channel(2437), ("2.4 GHz".to_string(), Some(6)));
        assert_eq!(band_and_channel(2484), ("2.4 GHz".to_string(), Some(14)));
        assert_eq!(band_and_channel(5180), ("5 GHz".to_string(), Some(36)));
        assert_eq!(band_and_channel(6115), ("6 GHz".to_string(), Some(33)));
    }

    #[test]
    fn iw_link_parsing_extracts_ssid_signal_frequency_and_rate() {
        let link = parse_iw_link(
            "Connected to 11:22:33:44:55:66 (on wlp7s0)\n\
             \tSSID: MyNetwork\n\
             \tfreq: 5180\n\
             \tsignal: -53 dBm\n\
             \ttx bitrate: 866.7 MBit/s\n",
        );
        assert_eq!(link.ssid.as_deref(), Some("MyNetwork"));
        assert_eq!(link.signal_dbm, Some(-53));
        assert_eq!(link.band.as_deref(), Some("5 GHz"));
        assert_eq!(link.channel, Some(36));
        assert_eq!(link.bitrate_mbps, Some(867));
    }
}
