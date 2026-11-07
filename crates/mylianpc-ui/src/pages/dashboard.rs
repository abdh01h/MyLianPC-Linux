//! Live dashboard: the health of the service and the most important readings.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Label, Orientation, ProgressBar};

use mylianpc_core::color::RgbColor;
use mylianpc_core::metrics;
use mylianpc_core::network::NetworkSnapshot;
use mylianpc_hardware::sensors::Snapshot;

use crate::client;
use crate::ui_kit::{self, big_value, card, label, page, pill, wrapped};
use crate::widgets::{metric, percent, rate_text, Sparkline};

struct MetricCard {
    name: Label,
    value: Label,
    detail: Label,
    bar: ProgressBar,
}

impl MetricCard {
    fn new(title: &str, value_class: &str) -> (GtkBox, Self) {
        let (card, body) = card(Some(title));
        let name = label("", "value-muted");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let value = big_value("—", value_class);
        let detail = wrapped("", "value-muted");
        let bar = ProgressBar::new();
        bar.set_show_text(false);
        bar.set_hexpand(true);
        body.append(&name);
        body.append(&value);
        body.append(&detail);
        body.append(&bar);
        (
            card,
            Self {
                name,
                value,
                detail,
                bar,
            },
        )
    }
}

struct Dashboard {
    status: Label,
    devices: gtk::FlowBox,
    warnings: GtkBox,
    cpu: MetricCard,
    gpu: MetricCard,
    memory: MetricCard,
    storage: MetricCard,
    cooler: MetricCard,
    network: MetricCard,
    cpu_history: Sparkline,
    gpu_history: Sparkline,
    network_history: Sparkline,
}

pub fn build() -> gtk::Widget {
    let (scroller, content) = page(
        "Dashboard",
        "Live readings and the state of every device MyLianPC controls.",
    );

    let status_card = GtkBox::new(Orientation::Vertical, 8);
    status_card.add_css_class("card");
    let status_header = GtkBox::new(Orientation::Horizontal, 8);
    status_header.append(&label("System health", "card-title"));
    let status = label("Connecting to the service…", "row-subtitle");
    status.set_hexpand(true);
    status_header.append(&status);
    status_card.append(&status_header);
    let devices = gtk::FlowBox::new();
    devices.set_selection_mode(gtk::SelectionMode::None);
    devices.set_column_spacing(6);
    devices.set_row_spacing(6);
    devices.set_max_children_per_line(6);
    devices.set_valign(gtk::Align::Start);
    status_card.append(&devices);
    let warnings = GtkBox::new(Orientation::Vertical, 6);
    status_card.append(&warnings);
    content.append(&status_card);

    let (cpu_card, cpu) = MetricCard::new("Processor", "stat-value");
    let (gpu_card, gpu) = MetricCard::new("Graphics", "stat-value-violet");
    gpu.bar.add_css_class("violet");
    let (memory_card, memory) = MetricCard::new("Memory", "stat-value");
    let (storage_card, storage) = MetricCard::new("Storage", "stat-value");
    let (cooler_card, cooler) = MetricCard::new("Cooler", "stat-value");
    let (network_card, network) = MetricCard::new("Connection", "stat-value");
    network.bar.set_visible(false);

    let grid = ui_kit::two_columns_flow();
    for card in [&cpu_card, &gpu_card, &memory_card, &storage_card, &cooler_card, &network_card] {
        card.set_hexpand(true);
        grid.insert(card, -1);
    }
    content.append(&grid);

    let (cpu_history, cpu_area) = Sparkline::new(RgbColor::new(0x00, 0xE0, 0xB0), Some(100.0));
    let (gpu_history, gpu_area) = Sparkline::new(RgbColor::new(0x8A, 0x6C, 0xFF), Some(100.0));
    let (network_history, network_area) = Sparkline::new(RgbColor::new(0x3B, 0x82, 0xF6), None);
    let (temp_card, temp_body) = card(Some("Temperature history"));
    let legend = GtkBox::new(Orientation::Horizontal, 14);
    legend.append(&label("CPU", "stat-label accent-text"));
    legend.append(&label("GPU", "stat-label violet-text"));
    temp_body.append(&legend);
    temp_body.append(&cpu_area);
    temp_body.append(&gpu_area);
    let (net_card, net_body) = card(Some("Download rate"));
    net_body.append(&network_area);
    let charts = ui_kit::two_columns_flow();
    charts.insert(&temp_card, -1);
    charts.insert(&net_card, -1);
    content.append(&charts);

    let dashboard = Rc::new(Dashboard {
        status,
        devices,
        warnings,
        cpu,
        gpu,
        memory,
        storage,
        cooler,
        network,
        cpu_history,
        gpu_history,
        network_history,
    });

    let busy = Rc::new(Cell::new(false));
    let start_refresh = {
        let dashboard = Rc::clone(&dashboard);
        let busy = Rc::clone(&busy);
        move || {
            if busy.replace(true) {
                return;
            }
            let dashboard = Rc::clone(&dashboard);
            let busy = Rc::clone(&busy);
            client::run_async(fetch_data, move |data| {
                busy.set(false);
                apply(&dashboard, data);
            });
        }
    };
    start_refresh();
    {
        let start_refresh = start_refresh.clone();
        gtk::glib::timeout_add_seconds_local(1, move || {
            start_refresh();
            gtk::glib::ControlFlow::Continue
        });
    }

    scroller.upcast()
}

fn clear_flow(container: &gtk::FlowBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn clear(container: &GtkBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

struct DashboardData {
    status: Result<client::ServiceStatus, String>,
    sensors: Result<Snapshot, String>,
    network: Result<NetworkSnapshot, String>,
}

fn fetch_data() -> DashboardData {
    DashboardData {
        status: client::status(),
        sensors: client::sensors(),
        network: client::network(),
    }
}

fn apply(dashboard: &Dashboard, data: DashboardData) {
    let status = match data.status {
        Ok(status) => status,
        Err(error) => {
            dashboard
                .status
                .set_text(&format!("Service unavailable — {error}"));
            dashboard.status.add_css_class("error-text");
            clear_flow(&dashboard.devices);
            clear(&dashboard.warnings);
            let notice = wrapped(
                "Start the service with: sudo systemctl start mylianpc",
                "row-subtitle",
            );
            dashboard.warnings.append(&notice);
            return;
        }
    };
    dashboard.status.set_text(&format!(
        "Service {} · {} device(s)",
        status.version,
        status.devices.len()
    ));
    dashboard.status.remove_css_class("error-text");
    clear_flow(&dashboard.devices);
    dashboard.devices.insert(
        &pill(
            &format!(
                "Profile: {}",
                status.active_profile_name.as_deref().unwrap_or("none")
            ),
            "pill-accent",
        ),
        -1,
    );
    for device in &status.devices {
        let short = match device.id.as_str() {
            "lianli-hydroshift2" => "Cooler",
            "gigabyte-fusion2" => "Fusion2",
            "corsair-dram" => "RAM",
            "sensors" => "Sensors",
            other => other,
        };
        let state = format!("{:?}", device.state).to_lowercase();
        dashboard.devices.insert(
            &pill(
                &format!("{short} · {}", state_label(&state)),
                if state == "connected" {
                    "pill-ok"
                } else {
                    "pill-warn"
                },
            ),
            -1,
        );
    }
    clear(&dashboard.warnings);
    for warning in &status.warnings {
        let notice = GtkBox::new(Orientation::Vertical, 2);
        notice.add_css_class("card-warning");
        notice.append(&wrapped(warning, "value-muted"));
        dashboard.warnings.append(&notice);
    }

    let snapshot = match data.sensors {
        Ok(snapshot) => snapshot,
        Err(error) => {
            dashboard
                .cooler
                .detail
                .set_text(&format!("Readings unavailable: {error}"));
            return;
        }
    };

    dashboard
        .cpu
        .name
        .set_text(snapshot.names.cpu.as_deref().unwrap_or("CPU"));
    dashboard.cpu.value.set_text(&metric(&snapshot, metrics::CPU_TEMP, "°C", 1));
    dashboard.cpu.detail.set_text(&format!(
        "{} load · {} · {}",
        metric(&snapshot, metrics::CPU_LOAD, "%", 0),
        metric(&snapshot, metrics::CPU_CLOCK, "MHz", 0),
        metric(&snapshot, metrics::CPU_POWER, "W", 1)
    ));
    dashboard
        .cpu
        .bar
        .set_fraction(percent(&snapshot, metrics::CPU_LOAD) / 100.0);
    if let Some(value) = snapshot.values.get(metrics::CPU_TEMP) {
        dashboard.cpu_history.push(*value);
    }

    dashboard
        .gpu
        .name
        .set_text(snapshot.names.gpu.as_deref().unwrap_or("GPU"));
    dashboard.gpu.value.set_text(&metric(&snapshot, metrics::GPU_TEMP, "°C", 1));
    dashboard.gpu.detail.set_text(&format!(
        "{} load · {} · {} · VRAM {} / {}",
        metric(&snapshot, metrics::GPU_LOAD, "%", 0),
        metric(&snapshot, metrics::GPU_CLOCK, "MHz", 0),
        metric(&snapshot, metrics::GPU_POWER, "W", 1),
        metric(&snapshot, metrics::GPU_VRAM_USED, "GB", 1),
        metric(&snapshot, metrics::GPU_VRAM_TOTAL, "GB", 1)
    ));
    dashboard
        .gpu
        .bar
        .set_fraction(percent(&snapshot, metrics::GPU_LOAD) / 100.0);
    if let Some(value) = snapshot.values.get(metrics::GPU_TEMP) {
        dashboard.gpu_history.push(*value);
    }

    dashboard
        .memory
        .name
        .set_text(snapshot.names.memory.as_deref().unwrap_or("Memory"));
    dashboard
        .memory
        .value
        .set_text(&metric(&snapshot, metrics::RAM_LOAD, "%", 0));
    dashboard.memory.detail.set_text(&format!(
        "{} used of {} · {} · {}{}",
        metric(&snapshot, metrics::RAM_USED_GB, "GB", 1),
        metric(&snapshot, metrics::RAM_TOTAL_GB, "GB", 1),
        metric(&snapshot, metrics::RAM_TEMP, "°C", 1),
        metric(&snapshot, metrics::RAM_SPEED, "MT/s", 0),
        snapshot
            .values
            .get(metrics::RAM_XMP)
            .map(|value| if *value >= 0.5 { " · EXPO on" } else { " · EXPO off" })
            .unwrap_or("")
    ));
    dashboard
        .memory
        .bar
        .set_fraction(percent(&snapshot, metrics::RAM_LOAD) / 100.0);

    dashboard
        .storage
        .value
        .set_text(&metric(&snapshot, metrics::STORAGE_TEMP, "°C", 1));
    let health = snapshot
        .values
        .get(metrics::STORAGE_HEALTH)
        .copied()
        .unwrap_or(0.0);
    dashboard.storage.name.set_text(&if health > 0.0 {
        format!("{health:.0}% remaining life")
    } else {
        "life unknown".to_string()
    });
    dashboard.storage.detail.set_text(&if snapshot.storage.is_empty() {
        "No NVMe drives reported".to_string()
    } else {
        snapshot
            .storage
            .iter()
            .map(|drive| {
                format!(
                    "{} · {}{}",
                    drive.name,
                    drive
                        .temperature_celsius
                        .map(|value| format!("{value:.0} °C"))
                        .unwrap_or_else(|| "—".to_string()),
                    drive
                        .free_bytes
                        .map(|bytes| format!(" · {:.0} GB free", bytes as f64 / 1e9))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    dashboard.storage.bar.set_fraction(health / 100.0);

    dashboard
        .cooler
        .value
        .set_text(&metric(&snapshot, metrics::COOLANT_TEMP, "°C", 1));
    dashboard.cooler.name.set_text(
        snapshot
            .names
            .cooler
            .as_deref()
            .unwrap_or("Cooler not detected"),
    );
    dashboard.cooler.detail.set_text(&format!(
        "Pump {} · radiator fans {} / {} / {}",
        metric(&snapshot, metrics::PUMP_RPM, "RPM", 0),
        metric(&snapshot, metrics::AIO_FAN1, "RPM", 0),
        metric(&snapshot, metrics::AIO_FAN2, "RPM", 0),
        metric(&snapshot, metrics::AIO_FAN3, "RPM", 0)
    ));
    let pump_fraction = snapshot
        .values
        .get(metrics::PUMP_RPM)
        .map(|rpm| ((rpm - 1600.0) / 900.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    dashboard.cooler.bar.set_fraction(pump_fraction);

    if let Ok(network) = data.network {
        dashboard
            .network
            .value
            .set_text(&rate_text(network.download_bytes_per_second));
        let ssid = network
            .ssid
            .as_deref()
            .or(network.primary.as_deref())
            .unwrap_or("network");
        dashboard.network.name.set_text(&format!(
            "↓ download · {ssid} · ↑ {}",
            rate_text(network.upload_bytes_per_second)
        ));
        let ipv4 = network
            .interfaces
            .iter()
            .find(|interface| Some(&interface.name) == network.primary.as_ref())
            .and_then(|interface| interface.ipv4.first())
            .cloned()
            .unwrap_or_else(|| "—".to_string());
        dashboard.network.detail.set_text(&format!(
            "IPv4 {ipv4} · gateway {} · DNS {}\n{:.2} GB downloaded · {:.2} GB uploaded today",
            network.gateway.as_deref().unwrap_or("—"),
            if network.dns.is_empty() {
                "—".to_string()
            } else {
                network.dns.join(", ")
            },
            network.downloaded_today_gb,
            network.uploaded_today_gb
        ));
        dashboard
            .network_history
            .push(network.download_bytes_per_second);
    } else {
        dashboard.network.value.set_text("—");
        dashboard
            .network
            .name
            .set_text("Connection information unavailable");
        dashboard.network.detail.set_text("");
    }
}

fn state_label(state: &str) -> &'static str {
    match state {
        "connected" => "connected",
        "busy" => "in use",
        "needs-driver" => "driver missing",
        "unresponsive" => "not responding",
        "error" => "error",
        _ => "not found",
    }
}
