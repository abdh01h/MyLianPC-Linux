//! Pump and radiator-fan curves with a preset picker and a live curve preview.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, DropDown, Grid, Label, Orientation, SpinButton, ToggleButton,
};

use mylianpc_core::fan::{presets, CurvePoint, FanCurve, TemperatureSource};
use mylianpc_core::metrics;

use crate::client;
use crate::ui_kit::{
    self, card, field, label, page, primary_button, secondary_button, CurvePreview,
};
use crate::widgets::{message_label, metric, show_error, show_result};

const TARGETS: [&str; 2] = ["hydroshift:pump", "hydroshift:fans"];
const PRESETS: [(&str, &str); 5] = [
    ("Silent", "Quiet, ramps late"),
    ("Case fans", "Quiet chassis curve"),
    ("Balanced", "Recommended"),
    ("Performance", "Lower temperatures"),
    ("Maximum", "Full speed"),
];

pub fn build() -> gtk::Widget {
    let (scroller, content) = page(
        "Cooling",
        "The pump and radiator fans follow coolant, CPU or GPU temperature. Curves always reach 100 % by 90 °C.",
    );

    let info = match client::cooling() {
        Ok(info) => info,
        Err(error) => {
            let card = GtkBox::new(Orientation::Vertical, 4);
            card.add_css_class("card-error");
            card.append(&label("Cooling unavailable", "notice-title"));
            card.append(&ui_kit::wrapped(&error, "value-muted"));
            content.append(&card);
            return scroller.upcast();
        }
    };

    let (live_card, live_body) = card(Some("Live"));
    let live = label("Reading the cooler…", "row-subtitle");
    live.set_wrap(true);
    live_body.append(&live);
    content.append(&live_card);

    // -------------------------------------------------------------- presets
    let (preset_card, preset_body) = card(Some("Preset"));
    let preset_row = gtk::FlowBox::new();
    preset_row.set_selection_mode(gtk::SelectionMode::None);
    preset_row.set_column_spacing(8);
    preset_row.set_row_spacing(8);
    preset_row.set_max_children_per_line(5);
    let mut preset_buttons = Vec::new();
    let mut first: Option<ToggleButton> = None;
    for (index, (name, description)) in PRESETS.iter().enumerate() {
        let button = ToggleButton::new();
        button.add_css_class("preset");
        let body = GtkBox::new(Orientation::Vertical, 2);
        let title = label(name, "row-title");
        title.set_xalign(0.5);
        let detail = label(description, "field-hint");
        detail.set_xalign(0.5);
        body.append(&title);
        body.append(&detail);
        button.set_child(Some(&body));
        if let Some(first) = &first {
            button.set_group(Some(first));
        } else {
            first = Some(button.clone());
        }
        if index == 2 {
            button.set_active(true);
        }
        preset_row.insert(&button, -1);
        preset_buttons.push(button);
    }
    preset_body.append(&preset_row);
    let preset_controls = GtkBox::new(Orientation::Horizontal, 10);
    let preset_source = DropDown::from_strings(&["Coolant", "CPU package", "GPU core"]);
    let preset_source_field = field("Temperature source", &preset_source);
    preset_source_field.set_hexpand(true);
    preset_controls.append(&preset_source_field);
    let apply_preset = primary_button("Apply preset");
    apply_preset.set_valign(gtk::Align::End);
    preset_controls.append(&apply_preset);
    preset_body.append(&preset_controls);
    content.append(&preset_card);

    // --------------------------------------------------------------- custom
    let (form_card, form_body) = card(Some("Custom curve"));
    let source = DropDown::from_strings(&["Coolant", "CPU package", "GPU core"]);
    let count = SpinButton::with_range(2.0, 8.0, 1.0);
    count.set_value(5.0);
    let minimum = SpinButton::with_range(0.0, 100.0, 1.0);
    minimum.set_value(20.0);
    let hysteresis = SpinButton::with_range(0.0, 10.0, 0.5);
    hysteresis.set_value(3.0);
    let failsafe = SpinButton::with_range(50.0, 100.0, 1.0);
    failsafe.set_value(100.0);

    let defaults = info
        .curves
        .get(TARGETS[0])
        .or_else(|| info.defaults.get(TARGETS[0]))
        .cloned()
        .unwrap_or_else(|| presets::balanced(TemperatureSource::Coolant));
    count.set_value(defaults.points.len().clamp(2, 8) as f64);
    minimum.set_value(defaults.minimum_duty_percent as f64);
    hysteresis.set_value(defaults.hysteresis_c);
    failsafe.set_value(defaults.failsafe_duty_percent as f64);

    let source_row = GtkBox::new(Orientation::Horizontal, 10);
    let source_field = field("Temperature source", &source);
    source_field.set_hexpand(true);
    source_row.append(&source_field);
    let count_field = field("Points", &count);
    source_row.append(&count_field);
    form_body.append(&source_row);

    let limits = Grid::new();
    limits.set_column_spacing(10);
    limits.set_row_spacing(6);
    limits.attach(&field("Minimum duty %", &minimum), 0, 0, 1, 1);
    limits.attach(&field("Hysteresis °C", &hysteresis), 1, 0, 1, 1);
    limits.attach(&field("Failsafe %", &failsafe), 2, 0, 1, 1);
    form_body.append(&limits);

    let points_grid = Grid::new();
    points_grid.set_row_spacing(5);
    points_grid.set_column_spacing(10);
    points_grid.attach(&label("Temperature °C", "field-label"), 0, 0, 1, 1);
    points_grid.attach(&label("Duty %", "field-label"), 1, 0, 1, 1);
    let mut temperatures = Vec::new();
    let mut duties = Vec::new();
    for index in 0..8 {
        let temperature = SpinButton::with_range(0.0, 110.0, 1.0);
        let duty = SpinButton::with_range(0.0, 100.0, 1.0);
        let point = defaults.points.get(index).copied().unwrap_or(CurvePoint {
            temperature_c: 100.0,
            duty_percent: 100,
        });
        temperature.set_value(point.temperature_c);
        duty.set_value(point.duty_percent as f64);
        points_grid.attach(&temperature, 0, (index + 1) as i32, 1, 1);
        points_grid.attach(&duty, 1, (index + 1) as i32, 1, 1);
        temperatures.push(temperature);
        duties.push(duty);
    }
    let set_visible: Rc<dyn Fn(usize)> = {
        let temperatures = temperatures.clone();
        let duties = duties.clone();
        Rc::new(move |count: usize| {
            for index in 0..8 {
                let visible = index < count;
                temperatures[index].set_visible(visible);
                duties[index].set_visible(visible);
            }
        })
    };
    set_visible(count.value() as usize);
    {
        let set_visible = Rc::clone(&set_visible);
        count.connect_value_notify(move |count| set_visible(count.value() as usize));
    }
    form_body.append(&points_grid);

    let form_actions = ui_kit::toolbar();
    let apply_custom = primary_button("Apply custom curve");
    let restore = secondary_button("Restore Balanced");
    form_actions.insert(&apply_custom, -1);
    form_actions.insert(&restore, -1);
    form_body.append(&form_actions);
    let message = message_label();
    form_body.append(&message);

    let (preview_card, preview_body) = card(Some("Curve preview"));
    preview_card.set_valign(gtk::Align::Start);
    let (curve_preview, curve_area) = CurvePreview::new();
    preview_body.append(&curve_area);
    preview_body.append(&ui_kit::wrapped(
        "The violet marker is the current temperature. The curve rises immediately and only falls \
         after the hysteresis has been crossed.",
        "field-hint",
    ));
    let columns = ui_kit::adaptive_columns(860);
    columns.append(&form_card);
    columns.append(&preview_card);
    content.append(&columns);
    let curve_preview = Rc::new(curve_preview);

    let build_points: Rc<dyn Fn() -> Vec<CurvePoint>> = {
        let temperatures = temperatures.clone();
        let duties = duties.clone();
        let count = count.clone();
        Rc::new(move || {
            let count = count.value() as usize;
            (0..count)
                .map(|index| CurvePoint {
                    temperature_c: temperatures[index].value(),
                    duty_percent: duties[index].value().round() as i32,
                })
                .collect()
        })
    };
    let update_preview: Rc<dyn Fn(Option<f64>)> = {
        let build_points = Rc::clone(&build_points);
        let curve_preview = Rc::clone(&curve_preview);
        Rc::new(move |current| {
            curve_preview.update(build_points(), current);
        })
    };

    {
        let update_preview = Rc::clone(&update_preview);
        update_preview(None);
        let update_preview = Rc::clone(&update_preview);
        source.connect_selected_notify(move |source| {
            let selected = source.selected();
            let update_preview = Rc::clone(&update_preview);
            client::run_async(client::sensors, move |result| {
                let current = result
                    .as_ref()
                    .ok()
                    .and_then(|snapshot| live_temp_selected(snapshot, selected));
                update_preview(current);
            });
        });
    }
    for spin in temperatures.iter().chain(duties.iter()) {
        let update_preview = Rc::clone(&update_preview);
        spin.connect_value_changed(move |_| update_preview(None));
    }
    for spin in [&minimum, &hysteresis, &failsafe] {
        let update_preview = Rc::clone(&update_preview);
        spin.connect_value_changed(move |_| update_preview(None));
    }

    // ---------------------------------------------------------------- actions
    {
        let message = message.clone();
        let preset_buttons = preset_buttons.clone();
        let preset_source = preset_source.clone();
        apply_preset.connect_clicked(move |_| {
            let index = preset_buttons
                .iter()
                .position(|button| button.is_active())
                .unwrap_or(2);
            let source = match preset_source.selected() {
                0 => TemperatureSource::Coolant,
                1 => TemperatureSource::CpuPackage,
                _ => TemperatureSource::GpuCore,
            };
            let builder: fn(TemperatureSource) -> FanCurve = match index {
                0 => presets::silent,
                1 => presets::case_fans,
                2 => presets::balanced,
                3 => presets::performance,
                _ => presets::maximum,
            };
            let curves = TARGETS
                .iter()
                .map(|target| (target.to_string(), builder(source)))
                .collect();
            show_result(&message, client::set_cooling(curves));
        });
    }
    {
        let message = message.clone();
        let build_points = Rc::clone(&build_points);
        let source = source.clone();
        let minimum = minimum.clone();
        let hysteresis = hysteresis.clone();
        let failsafe = failsafe.clone();
        apply_custom.connect_clicked(move |_| {
            let source_value = match source.selected() {
                0 => TemperatureSource::Coolant,
                1 => TemperatureSource::CpuPackage,
                _ => TemperatureSource::GpuCore,
            };
            let curve = FanCurve {
                source: source_value,
                points: build_points(),
                minimum_duty_percent: minimum.value().round() as i32,
                hysteresis_c: hysteresis.value(),
                failsafe_duty_percent: failsafe.value().round() as i32,
            };
            let problems = curve.validate();
            if !problems.is_empty() {
                show_error(&message, problems.join("\n"));
                return;
            }
            let curves = TARGETS
                .iter()
                .map(|target| (target.to_string(), curve.clone()))
                .collect();
            show_result(&message, client::set_cooling(curves));
        });
    }
    {
        let message = message.clone();
        restore.connect_clicked(move |_| show_result(&message, client::restore_cooling()));
    }

    // ------------------------------------------------------------------ live
    {
        let busy = Rc::new(Cell::new(false));
        let refresh_live = {
            let live = live.clone();
            let source = source.clone();
            let update_preview = Rc::clone(&update_preview);
            let busy = Rc::clone(&busy);
            move || {
                if busy.replace(true) {
                    return;
                }
                let live = live.clone();
                let source = source.clone();
                let update_preview = Rc::clone(&update_preview);
                let busy = Rc::clone(&busy);
                client::run_async(client::sensors, move |result| {
                    busy.set(false);
                    apply_live(&live, &source, &result);
                    let current = result
                        .as_ref()
                        .ok()
                        .and_then(|snapshot| live_temp(snapshot, &source));
                    update_preview(current);
                });
            }
        };
        refresh_live();
        gtk::glib::timeout_add_seconds_local(2, move || {
            refresh_live();
            gtk::glib::ControlFlow::Continue
        });
    }

    scroller.upcast()
}

fn live_temp(snapshot: &mylianpc_hardware::sensors::Snapshot, source: &DropDown) -> Option<f64> {
    live_temp_selected(snapshot, source.selected())
}

fn live_temp_selected(
    snapshot: &mylianpc_hardware::sensors::Snapshot,
    selected: u32,
) -> Option<f64> {
    match selected {
        1 => snapshot.values.get(metrics::CPU_TEMP).copied(),
        2 => snapshot.values.get(metrics::GPU_TEMP).copied(),
        _ => snapshot.values.get(metrics::COOLANT_TEMP).copied(),
    }
}

fn apply_live(
    label: &Label,
    source: &DropDown,
    result: &Result<mylianpc_hardware::sensors::Snapshot, String>,
) {
    match result {
        Ok(snapshot) => {
            label.set_text(&format!(
                "Coolant {} · pump {} · radiator {} / {} / {}\nCPU {} · GPU {} · source: {}",
                metric(snapshot, metrics::COOLANT_TEMP, "°C", 1),
                metric(snapshot, metrics::PUMP_RPM, "RPM", 0),
                metric(snapshot, metrics::AIO_FAN1, "RPM", 0),
                metric(snapshot, metrics::AIO_FAN2, "RPM", 0),
                metric(snapshot, metrics::AIO_FAN3, "RPM", 0),
                metric(snapshot, metrics::CPU_TEMP, "°C", 1),
                metric(snapshot, metrics::GPU_TEMP, "°C", 1),
                match source.selected() {
                    1 => "CPU package",
                    2 => "GPU core",
                    _ => "coolant",
                }
            ));
        }
        Err(error) => label.set_text(&format!("Readings unavailable: {error}")),
    }
}
