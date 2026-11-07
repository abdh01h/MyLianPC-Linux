//! Service settings, header LED counts and diagnostics.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Expander, Grid, Orientation, ScrolledWindow, SpinButton, Switch, TextView,
};


use crate::client;
use crate::ui_kit::{
    self, card, field_with_hint, label, page, primary_button, secondary_button,
};
use crate::widgets::{message_label, show_error, show_result};

pub fn build() -> gtk::Widget {
    let (scroller, content) = page(
        "Settings",
        "Service behaviour, ARGB header LED counts and diagnostics.",
    );

    let settings = match client::settings() {
        Ok(settings) => settings,
        Err(error) => {
            let card = GtkBox::new(Orientation::Vertical, 4);
            card.add_css_class("card-error");
            card.append(&label("Settings unavailable", "notice-title"));
            card.append(&ui_kit::wrapped(&error, "value-muted"));
            content.append(&card);
            return scroller.upcast();
        }
    };
    let zones = client::zones().unwrap_or_default();
    let message = message_label();

    let (service_card, service_body) = card(Some("Service"));
    let sensor_interval = SpinButton::with_range(500.0, 10_000.0, 100.0);
    sensor_interval.set_value(settings.sensor_interval_ms as f64);
    sensor_interval.set_halign(gtk::Align::Start);
    sensor_interval.set_width_chars(6);
    service_body.append(&field_with_hint(
        "Sensor interval (ms)",
        "How often CPU, GPU, memory and storage readings are refreshed (500–10000 ms).",
        &sensor_interval,
    ));
    let save_flash = Switch::new();
    save_flash.set_active(settings.save_motherboard_lighting_to_flash);
    save_flash.set_halign(gtk::Align::Start);
    service_body.append(&field_with_hint(
        "Save motherboard lighting to controller flash",
        "Effects survive a power cycle without software. Per-LED gradients are never stored in flash.",
        &save_flash,
    ));
    let fan_lighting = Switch::new();
    fan_lighting.set_active(settings.cooler_fan_lighting);
    fan_lighting.set_halign(gtk::Align::Start);
    service_body.append(&field_with_hint(
        "Light the radiator fans with the pump ring (experimental)",
        "When off, the radiator fan LEDs stay dark. The service turns this off automatically if the cooler rejects it.",
        &fan_lighting,
    ));
    let apply = primary_button("Apply settings");
    let apply_row = ui_kit::toolbar();
    apply_row.insert(&apply, -1);
    service_body.append(&apply_row);
    service_body.append(&message);

    let (led_card, led_body) = card(Some("ARGB header LED counts"));
    let led_grid = Grid::new();
    led_grid.set_column_spacing(12);
    led_grid.set_row_spacing(8);
    let mut led_counts = Vec::new();
    let addressable: Vec<_> = zones
        .iter()
        .filter(|zone| zone.led_count_configurable)
        .collect();
    if addressable.is_empty() {
        led_body.append(&ui_kit::wrapped(
            "No configurable ARGB headers were reported.",
            "field-hint",
        ));
    } else {
        for zone in addressable {
            let spin = SpinButton::with_range(1.0, 256.0, 1.0);
            spin.set_width_chars(4);
            spin.set_value(
                settings
                    .zone_led_counts
                    .get(&zone.id)
                    .copied()
                    .unwrap_or(zone.led_count)
                    .clamp(1, 256) as f64,
            );
            let row = GtkBox::new(Orientation::Horizontal, 10);
            let name = label(&zone.name, "value-muted");
            name.set_hexpand(true);
            row.append(&name);
            row.append(&spin);
            led_counts.push((zone.id.clone(), spin.clone()));
            led_grid.attach(&row, 0, led_counts.len() as i32 - 1, 1, 1);
        }
        led_body.append(&led_grid);
        led_body.append(&ui_kit::wrapped(
            "Used for per-LED effects (gradient, rainbow, rainbow wave and wave).",
            "field-hint",
        ));
    }
    let settings_columns = ui_kit::two_columns_flow();
    service_card.set_hexpand(true);
    led_card.set_hexpand(true);
    settings_columns.insert(&service_card, -1);
    settings_columns.insert(&led_card, -1);
    content.append(&settings_columns);

    let diagnostics = TextView::new();
    diagnostics.set_monospace(true);
    diagnostics.set_wrap_mode(gtk::WrapMode::WordChar);
    diagnostics.set_size_request(-1, 300);
    let diagnostics_scroller = ScrolledWindow::builder()
        .min_content_height(220)
        .child(&diagnostics)
        .build();
    let diagnostics_expander = Expander::new(Some("Diagnostics report"));
    let diagnostics_body = GtkBox::new(Orientation::Vertical, 8);
    diagnostics_body.append(&diagnostics_scroller);
    let diagnostics_buttons = ui_kit::toolbar();
    let refresh = secondary_button("Refresh diagnostics");
    let copy = secondary_button("Copy to clipboard");
    diagnostics_buttons.insert(&refresh, -1);
    diagnostics_buttons.insert(&copy, -1);
    diagnostics_body.append(&diagnostics_buttons);
    let diagnostics_message = message_label();
    diagnostics_body.append(&diagnostics_message);
    diagnostics_expander.set_child(Some(&diagnostics_body));
    content.append(&diagnostics_expander);

    {
        let message = message.clone();
        let sensor_interval = sensor_interval.clone();
        let save_flash = save_flash.clone();
        let fan_lighting = fan_lighting.clone();
        let led_counts = led_counts.clone();
        apply.connect_clicked(move |_| {
            // Start from the stored settings so unrelated fields (e.g. the
            // controlled-fan list) are not reset to their defaults.
            let mut update = client::settings().unwrap_or_default();
            update.sensor_interval_ms = sensor_interval.value().round() as u64;
            update.save_motherboard_lighting_to_flash = save_flash.is_active();
            update.cooler_fan_lighting = fan_lighting.is_active();
            for (zone_id, spin) in &led_counts {
                update
                    .zone_led_counts
                    .insert(zone_id.clone(), spin.value().round() as usize);
            }
            message.set_text("Applying settings…");
            let message_for_done = message.clone();
            let save_flash = save_flash.clone();
            let fan_lighting = fan_lighting.clone();
            client::run_async(
                move || {
                    let result = client::update_settings(update);
                    let current = client::settings().ok();
                    (result, current)
                },
                move |(result, current)| {
                    if let Some(current) = current {
                        save_flash.set_active(current.save_motherboard_lighting_to_flash);
                        fan_lighting.set_active(current.cooler_fan_lighting);
                    }
                    show_result(&message_for_done, result);
                },
            );
        });
    }
    {
        let diagnostics = diagnostics.clone();
        let message = diagnostics_message.clone();
        refresh.connect_clicked(move |_| match client::diagnostics() {
            Ok(text) => diagnostics.buffer().set_text(&text),
            Err(error) => show_error(&message, error),
        });
    }
    {
        let diagnostics = diagnostics.clone();
        let message = diagnostics_message.clone();
        copy.connect_clicked(move |_| {
            let buffer = diagnostics.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
                show_result(&message, Ok("Diagnostics copied.".to_string()));
            } else {
                show_error(&message, "No display clipboard is available.");
            }
        });
    }

    if let Ok(text) = client::diagnostics() {
        diagnostics.buffer().set_text(&text);
    }

    scroller.upcast()
}
