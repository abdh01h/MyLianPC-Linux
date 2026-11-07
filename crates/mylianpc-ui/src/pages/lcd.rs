//! LCD theme, media and layout editor with an always-on live preview.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{
    Box as GtkBox, CheckButton, DropDown, Entry, Expander, Orientation, Picture,
    SpinButton, StringList, Switch, TextView,
};

use mylianpc_core::profile::{
    GaugeStyle, LcdMode, LcdSettings, LcdWidget, TemperatureUnit,
};

use crate::client;
use crate::ui_kit::{
    self, card, color_row, field, field_with_hint, label, page, primary_button, secondary_button,
    ColorField,
};
use crate::widgets::{message_label, show_error, show_result};

struct Controls {
    mode: DropDown,
    theme: DropDown,
    media: DropDown,
    background: ColorField,
    text: ColorField,
    accent: ColorField,
    secondary: ColorField,
    font: Entry,
    font_scale: gtk::Scale,
    brightness: gtk::Scale,
    rotation: DropDown,
    refresh: SpinButton,
    unit: DropDown,
    clock_24: Switch,
    gauge: DropDown,
    overlay: Switch,
    metrics: Vec<CheckButton>,
    widgets: TextView,
}

pub fn build() -> gtk::Widget {
    let (scroller, content) = page(
        "Cooler LCD",
        "Pick a theme, colours and layout. The preview uses the live sensor readings.",
    );

    let info = match client::lcd() {
        Ok(info) => info,
        Err(error) => {
            let card = GtkBox::new(Orientation::Vertical, 4);
            card.add_css_class("card-error");
            card.append(&label("LCD unavailable", "notice-title"));
            card.append(&ui_kit::wrapped(&error, "value-muted"));
            content.append(&card);
            return scroller.upcast();
        }
    };
    let settings = info.settings.clone();
    let theme_ids: Vec<String> = info.themes.iter().map(|theme| theme.id.clone()).collect();
    let theme_names: Vec<String> = info.themes.iter().map(|theme| theme.name.clone()).collect();
    let media_files = client::media_list().map(|media| media.files).unwrap_or_default();
    let mut media_names = vec!["(none)".to_string()];
    media_names.extend(media_files.iter().cloned());

    let mode = DropDown::from_strings(&["Theme", "Image", "Animation", "Off"]);
    mode.set_selected(match settings.mode {
        LcdMode::Theme => 0,
        LcdMode::Image => 1,
        LcdMode::Animation => 2,
        LcdMode::Off => 3,
    });
    let theme = DropDown::from_strings(
        &theme_names.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    theme.set_selected(
        theme_ids
            .iter()
            .position(|id| id == &settings.theme_id)
            .unwrap_or(0) as u32,
    );
    let media = DropDown::from_strings(
        &media_names.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    if let Some(file) = &settings.media_file {
        if let Some(index) = media_names.iter().position(|name| name == file) {
            media.set_selected(index as u32);
        }
    }
    let background = ColorField::new(settings.background);
    let text = ColorField::new(settings.text_color);
    let accent = ColorField::new(settings.accent);
    let secondary = ColorField::new(settings.secondary);
    let font = Entry::new();
    font.set_text(&settings.font_family);
    font.set_hexpand(true);
    let font_scale = gtk::Scale::with_range(Orientation::Horizontal, 0.5, 2.0, 0.05);
    font_scale.set_value(settings.font_scale.clamp(0.5, 2.0));
    font_scale.set_hexpand(true);
    let brightness = gtk::Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    brightness.set_value(settings.brightness.clamp(0, 100) as f64);
    brightness.set_hexpand(true);
    let rotation = DropDown::from_strings(&["0°", "90°", "180°", "270°"]);
    rotation.set_selected(match settings.rotation_degrees {
        90 => 1,
        180 => 2,
        270 => 3,
        _ => 0,
    });
    let refresh = SpinButton::with_range(500.0, 60_000.0, 250.0);
    refresh.set_value(settings.refresh_milliseconds as f64);
    let unit = DropDown::from_strings(&["Celsius", "Fahrenheit"]);
    unit.set_selected(if settings.unit == TemperatureUnit::Fahrenheit { 1 } else { 0 });
    let clock_24 = Switch::new();
    clock_24.set_active(settings.clock_24_hour);
    clock_24.set_halign(gtk::Align::Start);
    let gauge = DropDown::from_strings(&["Arc", "Ring", "Bar"]);
    gauge.set_selected(match settings.gauge_style {
        GaugeStyle::Arc => 0,
        GaugeStyle::Ring => 1,
        GaugeStyle::Bar => 2,
    });
    let overlay = Switch::new();
    overlay.set_active(settings.media_overlay);
    overlay.set_halign(gtk::Align::Start);

    let metrics: Vec<CheckButton> = mylianpc_core::metrics::ALL
        .iter()
        .map(|info| {
            let check = CheckButton::with_label(info.label);
            check.set_active(settings.metrics.iter().any(|key| key == info.key));
            check
        })
        .collect();
    let metrics_grid = gtk::Grid::new();
    metrics_grid.set_row_spacing(4);
    metrics_grid.set_column_spacing(14);
    for (index, check) in metrics.iter().enumerate() {
        metrics_grid.attach(check, (index % 2) as i32, (index / 2) as i32, 1, 1);
    }

    let widgets = TextView::new();
    widgets.set_wrap_mode(gtk::WrapMode::WordChar);
    widgets.set_monospace(true);
    widgets.set_size_request(-1, 110);
    widgets
        .buffer()
        .set_text(&serde_json::to_string_pretty(&settings.widgets).unwrap_or_else(|_| "[]".into()));

    let (content_card, content_body) = card(Some("Content"));
    let content_grid = gtk::Grid::new();
    content_grid.set_column_spacing(12);
    content_grid.set_row_spacing(10);
    content_grid.attach(&field("Mode", &mode), 0, 0, 1, 1);
    content_grid.attach(&field("Theme", &theme), 1, 0, 1, 1);
    let upload = secondary_button("Upload…");
    let media_row = GtkBox::new(Orientation::Horizontal, 8);
    media.set_hexpand(true);
    media_row.append(&media);
    media_row.append(&upload);
    content_grid.attach(&field_with_hint("Image or animation", "PNG, JPEG, BMP, GIF or WebP (up to 25 MB)", &media_row), 0, 1, 2, 1);
    let overlay_field = field_with_hint(
        "Overlay",
        "Draw the clock and CPU/GPU temperatures over images and animations",
        &overlay,
    );
    content_grid.attach(&overlay_field, 0, 2, 2, 1);
    content_body.append(&content_grid);

    let (appearance_card, appearance_body) = card(Some("Appearance"));
    let colours = GtkBox::new(Orientation::Vertical, 6);
    for (name, field) in [
        ("Background", &background),
        ("Text", &text),
        ("Accent", &accent),
        ("Secondary", &secondary),
    ] {
        colours.append(&color_row(name, field));
    }
    appearance_body.append(&colours);
    let font_row = GtkBox::new(Orientation::Horizontal, 10);
    let font_field = field("Font family", &font);
    font_field.set_hexpand(true);
    let scale_field = field("Text size", &font_scale);
    scale_field.set_hexpand(true);
    font_row.append(&font_field);
    font_row.append(&scale_field);
    appearance_body.append(&font_row);

    let (layout_card, layout_body) = card(Some("Layout and behaviour"));
    let layout_grid = gtk::Grid::new();
    layout_grid.set_column_spacing(12);
    layout_grid.set_row_spacing(10);
    layout_grid.attach(&field("Brightness", &brightness), 0, 0, 2, 1);
    layout_grid.attach(&field("Rotation", &rotation), 0, 1, 1, 1);
    layout_grid.attach(&field("Refresh (ms)", &refresh), 1, 1, 1, 1);
    layout_grid.attach(&field("Temperature unit", &unit), 0, 2, 1, 1);
    layout_grid.attach(&field("Gauge style", &gauge), 1, 2, 1, 1);
    layout_grid.attach(&field("24-hour clock", &clock_24), 0, 3, 2, 1);
    layout_body.append(&layout_grid);

    let metrics_expander = Expander::new(Some("Metrics for the multi-metric theme"));
    metrics_expander.set_child(Some(&metrics_grid));
    let custom_expander = Expander::new(Some("Custom layout widgets (JSON)"));
    custom_expander.set_child(Some(&widgets));

    let (preview_card, preview_body) = card(Some("Live preview"));
    preview_card.set_valign(gtk::Align::Start);
    preview_card.set_hexpand(true);
    let picture = Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_size_request(260, 260);
    let preview_frame = GtkBox::new(Orientation::Vertical, 0);
    preview_frame.add_css_class("preview-frame");
    preview_frame.append(&picture);
    preview_body.append(&preview_frame);
    let preview_message = message_label();
    preview_body.append(&preview_message);
    let preview_actions = ui_kit::toolbar();
    let preview_refresh = secondary_button("Refresh preview");
    let save = primary_button("Save and push");
    preview_actions.insert(&preview_refresh, -1);
    preview_actions.insert(&save, -1);
    preview_body.append(&preview_actions);
    preview_body.append(&ui_kit::wrapped(
        "The preview is rendered by the service with the same theme code that drives the panel.",
        "field-hint",
    ));

    let columns = GtkBox::new(Orientation::Vertical, 16);
    let left = GtkBox::new(Orientation::Vertical, 16);
    left.set_hexpand(true);
    left.append(&content_card);
    left.append(&appearance_card);
    left.append(&layout_card);
    left.append(&metrics_expander);
    left.append(&custom_expander);
    columns.append(&preview_card);
    columns.append(&left);
    content.append(&columns);

    let controls = Rc::new(Controls {
        mode,
        theme,
        media,
        background,
        text,
        accent,
        secondary,
        font,
        font_scale,
        brightness,
        rotation,
        refresh,
        unit,
        clock_24,
        gauge,
        overlay,
        metrics,
        widgets,
    });
    let theme_ids = Rc::new(theme_ids);

    let preview_generation = Rc::new(Cell::new(0u64));
    let refresh_preview: Rc<dyn Fn()> = {
        let picture = picture.clone();
        let preview_message = preview_message.clone();
        let controls = Rc::clone(&controls);
        let theme_ids = Rc::clone(&theme_ids);
        let generation = Rc::clone(&preview_generation);
        Rc::new(move || {
            let settings = match build_settings(&controls, theme_ids.as_slice(), false) {
                Ok(settings) => settings,
                Err(error) => {
                    preview_message.set_text(&error);
                    return;
                }
            };
            let current = generation.get().wrapping_add(1);
            generation.set(current);
            let picture = picture.clone();
            let preview_message = preview_message.clone();
            let generation = Rc::clone(&generation);
            client::run_async(
                move || client::preview_draft(&settings),
                move |result| {
                    if generation.get() != current {
                        return;
                    }
                    match result {
                        Ok(png) => {
                            match gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from(&png[..])) {
                                Ok(texture) => {
                                    picture.set_paintable(Some(&texture));
                                    preview_message.set_text("");
                                }
                                Err(error) => preview_message.set_text(&format!(
                                    "Could not decode the preview: {error}"
                                )),
                            }
                        }
                        Err(error) => {
                            preview_message.set_text(&format!("Preview unavailable: {error}"))
                        }
                    }
                },
            );
        })
    };
    // Debounce live edits so dragging a slider does not flood the service.
    let schedule_preview: Rc<dyn Fn()> = {
        let refresh_preview = Rc::clone(&refresh_preview);
        let pending = Rc::new(Cell::new(false));
        Rc::new(move || {
            if pending.replace(true) {
                return;
            }
            let pending = Rc::clone(&pending);
            let refresh_preview = Rc::clone(&refresh_preview);
            gtk::glib::timeout_add_local_once(Duration::from_millis(180), move || {
                pending.set(false);
                refresh_preview();
            });
        })
    };

    {
        let refresh_preview = refresh_preview.clone();
        picture.connect_map(move |_| refresh_preview());
    }
    {
        let refresh_preview = refresh_preview.clone();
        let picture = picture.clone();
        gtk::glib::timeout_add_seconds_local(5, move || {
            if picture.is_mapped() {
                refresh_preview();
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    {
        let controls = Rc::clone(&controls);
        let theme_ids = Rc::clone(&theme_ids);
        let message = preview_message.clone();
        let refresh_preview = refresh_preview.clone();
        save.connect_clicked(move |_| match build_settings(&controls, theme_ids.as_slice(), true) {
            Ok(settings) => {
                message.set_text("Saving and pushing…");
                let message = message.clone();
                let refresh_preview = Rc::clone(&refresh_preview);
                client::run_async(
                    move || {
                        let saved = client::set_lcd(settings)?;
                        let pushed = client::push_lcd();
                        Ok::<_, String>((saved, pushed))
                    },
                    move |result| {
                        refresh_preview();
                        match result {
                            Ok((saved, Ok(_))) => {
                                show_result(&message, Ok(format!("{saved} Pushed to the panel.")))
                            }
                            Ok((saved, Err(error))) => show_error(
                                &message,
                                format!("{saved} Push failed: {error}"),
                            ),
                            Err(error) => show_error(&message, error),
                        }
                    },
                );
            }
            Err(error) => show_error(&message, error),
        });
    }
    {
        let message = preview_message.clone();
        let refresh_preview = refresh_preview.clone();
        preview_refresh.connect_clicked(move |_| {
            refresh_preview();
            message.set_text("Preview refreshed.");
        });
    }
    {
        let controls = Rc::clone(&controls);
        let message = preview_message.clone();
        let schedule_preview = Rc::clone(&schedule_preview);
        upload.connect_clicked(move |_| {
            let schedule_for_dialog = Rc::clone(&schedule_preview);
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Choose an image or animation");
            let controls = Rc::clone(&controls);
            let message = message.clone();
            dialog.open(
                ui_kit::active_window().as_ref(),
                None::<&gtk::gio::Cancellable>,
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    let Some(path) = file.path() else {
                        show_error(&message, "The selected file has no local path.");
                        return;
                    };
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("media")
                        .to_string();
                    let data = match std::fs::read(&path) {
                        Ok(data) => data,
                        Err(error) => {
                            show_error(&message, format!("Could not read the file: {error}"));
                            return;
                        }
                    };
                    match client::media_upload(&name, &data) {
                        Ok(media) => {
                            let mut names = vec!["(none)".to_string()];
                            names.extend(media.files.iter().cloned());
                            controls.media.set_model(Some(&StringList::new(
                                &names.iter().map(String::as_str).collect::<Vec<_>>(),
                            )));
                            if let Some(stored) = &media.stored {
                                if let Some(index) = names.iter().position(|name| name == stored) {
                                    controls.media.set_selected(index as u32);
                                }
                            }
                            schedule_for_dialog();
                            show_result(
                                &message,
                                Ok("Uploaded. Choose Image or Animation, then save.".to_string()),
                            );
                        }
                        Err(error) => show_error(&message, error),
                    }
                },
            );
        });
    }
    // Every control updates the draft preview.
    for dropdown in [
        &controls.mode,
        &controls.theme,
        &controls.media,
        &controls.rotation,
        &controls.unit,
        &controls.gauge,
    ] {
        let schedule_preview = Rc::clone(&schedule_preview);
        dropdown.connect_selected_notify(move |_| schedule_preview());
    }
    for scale in [&controls.brightness, &controls.font_scale] {
        let schedule_preview = Rc::clone(&schedule_preview);
        scale.connect_value_changed(move |_| schedule_preview());
    }
    for control in [&controls.clock_24, &controls.overlay] {
        let schedule_preview = Rc::clone(&schedule_preview);
        control.connect_active_notify(move |_| schedule_preview());
    }
    for field in [
        &controls.background,
        &controls.text,
        &controls.accent,
        &controls.secondary,
    ] {
        let schedule_preview = Rc::clone(&schedule_preview);
        field.entry.connect_changed(move |_| schedule_preview());
    }
    {
        let schedule_preview = Rc::clone(&schedule_preview);
        controls.font.connect_changed(move |_| schedule_preview());
    }
    {
        let schedule_preview = Rc::clone(&schedule_preview);
        controls.refresh.connect_value_changed(move |_| schedule_preview());
    }
    for check in &controls.metrics {
        let schedule_preview = Rc::clone(&schedule_preview);
        check.connect_toggled(move |_| schedule_preview());
    }
    {
        let schedule_preview = Rc::clone(&schedule_preview);
        controls.widgets.buffer().connect_changed(move |_| schedule_preview());
    }

    scroller.upcast()
}

fn build_settings(
    controls: &Controls,
    theme_ids: &[String],
    validate: bool,
) -> Result<LcdSettings, String> {
    let buffer = controls.widgets.buffer();
    let widgets_text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
    let widgets: Vec<LcdWidget> = serde_json::from_str(widgets_text.as_str())
        .map_err(|error| format!("Custom layout JSON: {error}"))?;
    let metrics = controls
        .metrics
        .iter()
        .zip(mylianpc_core::metrics::ALL.iter())
        .filter(|(check, _)| check.is_active())
        .map(|(_, info)| info.key.to_string())
        .collect();
    let media_file = if controls.media.selected() == 0 {
        None
    } else {
        controls
            .media
            .selected_item()
            .and_then(|item| item.downcast::<gtk::StringObject>().ok())
            .map(|item| item.string().to_string())
    };
    let settings = LcdSettings {
        mode: match controls.mode.selected() {
            1 => LcdMode::Image,
            2 => LcdMode::Animation,
            3 => LcdMode::Off,
            _ => LcdMode::Theme,
        },
        theme_id: theme_ids
            .get(controls.theme.selected() as usize)
            .cloned()
            .unwrap_or_else(|| "minimal-temps".to_string()),
        background: controls.background.color().ok_or("Invalid background colour.")?,
        text_color: controls.text.color().ok_or("Invalid text colour.")?,
        accent: controls.accent.color().ok_or("Invalid accent colour.")?,
        secondary: controls.secondary.color().ok_or("Invalid secondary colour.")?,
        font_family: controls.font.text().to_string(),
        font_scale: controls.font_scale.value(),
        unit: if controls.unit.selected() == 1 {
            TemperatureUnit::Fahrenheit
        } else {
            TemperatureUnit::Celsius
        },
        clock_24_hour: controls.clock_24.is_active(),
        gauge_style: match controls.gauge.selected() {
            1 => GaugeStyle::Ring,
            2 => GaugeStyle::Bar,
            _ => GaugeStyle::Arc,
        },
        metrics,
        widgets,
        media_file,
        media_overlay: controls.overlay.is_active(),
        brightness: controls.brightness.value().round() as i32,
        rotation_degrees: [0, 90, 180, 270][controls.rotation.selected().min(3) as usize],
        refresh_milliseconds: controls.refresh.value().round() as u64,
    };
    if validate {
        let problems = settings.validate();
        if !problems.is_empty() {
            return Err(problems.join("\n"));
        }
    }
    Ok(settings)
}
