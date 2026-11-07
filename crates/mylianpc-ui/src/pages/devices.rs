//! Adapter status, capabilities and recovery actions.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{Box as GtkBox, FlowBox, Label, Orientation};

use crate::client;
use crate::ui_kit::{self, card, label, page, pill, pill_for_state, secondary_button};
use crate::widgets::{message_label, show_result};

pub fn build() -> gtk::Widget {
    let (scroller, content) = page(
        "Devices",
        "Everything MyLianPC owns, with the capabilities each adapter really supports.",
    );

    let toolbar = ui_kit::toolbar();
    let refresh = secondary_button("Refresh");
    let rescan = secondary_button("Rescan hardware");
    let reapply = secondary_button("Re-apply profile");
    toolbar.insert(&refresh, -1);
    toolbar.insert(&rescan, -1);
    toolbar.insert(&reapply, -1);
    content.append(&toolbar);
    let message = message_label();
    content.append(&message);

    let list = GtkBox::new(Orientation::Vertical, 14);
    content.append(&list);
    populate(&list, &message);

    {
        let list = list.clone();
        let message = message.clone();
        refresh.connect_clicked(move |_| populate(&list, &message));
    }
    {
        let list = list.clone();
        let message = message.clone();
        rescan.connect_clicked(move |_| {
            message.set_text("Rescanning hardware…");
            let list = list.clone();
            let message = message.clone();
            client::run_async(client::rescan, move |result| {
                show_result(&message, result);
                populate(&list, &message);
            });
        });
    }
    {
        let list = list.clone();
        let message = message.clone();
        reapply.connect_clicked(move |_| {
            message.set_text("Re-applying the active profile…");
            let list = list.clone();
            let message = message.clone();
            client::run_async(client::reapply, move |result| {
                show_result(&message, result);
                populate(&list, &message);
            });
        });
    }

    scroller.upcast()
}

fn clear(container: &GtkBox) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn populate(list: &GtkBox, message: &Label) {
    clear(list);
    let status = match client::status() {
        Ok(status) => status,
        Err(error) => {
            let card = GtkBox::new(Orientation::Vertical, 4);
            card.add_css_class("card-error");
            card.append(&label("Service unavailable", "notice-title"));
            card.append(&ui_kit::wrapped(&error, "value-muted"));
            list.append(&card);
            return;
        }
    };
    message.remove_css_class("error-text");
    message.set_text(&format!(
        "Service {} · active profile: {}",
        status.version,
        status.active_profile_name.as_deref().unwrap_or("none")
    ));

    for device in status.devices {
        let (card, body) = card(None);
        let head = GtkBox::new(Orientation::Horizontal, 8);
        let title = label(&device.name, "row-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        head.append(&title);
        head.append(&pill_for_state(&format!("{:?}", device.state).to_lowercase()));
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        head.append(&spacer);
        let kind = label(
            &format!("{} · {}", device.manufacturer, device.kind),
            "field-hint",
        );
        kind.set_ellipsize(gtk::pango::EllipsizeMode::End);
        head.append(&kind);
        body.append(&head);

        let mut facts: Vec<String> = Vec::new();
        if let Some(firmware) = &device.firmware {
            facts.push(format!("firmware {firmware}"));
        }
        if let Some(location) = &device.location {
            facts.push(location.clone());
        }
        facts.push(device.id.clone());
        body.append(&ui_kit::wrapped(&facts.join(" · "), "value-muted"));

        if !device.zones.is_empty() {
            body.append(&label("Zones", "field-label"));
            let zones = FlowBox::new();
            zones.set_selection_mode(gtk::SelectionMode::None);
            zones.set_column_spacing(6);
            zones.set_row_spacing(4);
            zones.set_max_children_per_line(4);
            for zone in &device.zones {
                zones.insert(
                    &pill(
                        &format!("{} · {} LEDs", zone.name, zone.led_count),
                        "pill-muted",
                    ),
                    -1,
                );
            }
            body.append(&zones);
        }
        if !device.fans.is_empty() {
            let fans = GtkBox::new(Orientation::Vertical, 3);
            fans.append(&label("Fans", "field-label"));
            for fan in &device.fans {
                fans.append(&label(
                    &format!(
                        "{} — {}{}",
                        fan.name,
                        fan.rpm
                            .map(|rpm| format!("{rpm:.0} RPM"))
                            .unwrap_or_else(|| "—".to_string()),
                        if fan.controllable { "" } else { " · monitor only" }
                    ),
                    "value-muted",
                ));
            }
            body.append(&fans);
        }
        if !device.capabilities.is_empty() {
            body.append(&label("Supports", "field-label"));
            let capabilities = FlowBox::new();
            capabilities.set_selection_mode(gtk::SelectionMode::None);
            capabilities.set_column_spacing(6);
            capabilities.set_row_spacing(4);
            capabilities.set_max_children_per_line(4);
            for capability in &device.capabilities {
                capabilities.insert(&pill(capability, "pill-accent"), -1);
            }
            body.append(&capabilities);
        }
        if !device.dependencies.is_empty() {
            body.append(&ui_kit::wrapped(
                &format!("Requires: {}", device.dependencies.join(" · ")),
                "field-hint",
            ));
        }
        for warning in &device.warnings {
            let notice = GtkBox::new(Orientation::Vertical, 2);
            notice.add_css_class("card-warning");
            notice.append(&ui_kit::wrapped(warning, "value-muted"));
            body.append(&notice);
        }
        if let Some(error) = &device.error {
            let notice = GtkBox::new(Orientation::Vertical, 2);
            notice.add_css_class("card-error");
            notice.append(&ui_kit::wrapped(error, "value-muted"));
            body.append(&notice);
        }
        if let Some(troubleshooting) = &device.troubleshooting {
            body.append(&ui_kit::wrapped(troubleshooting, "field-hint"));
        }
        list.append(&card);
    }
}
