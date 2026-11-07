//! Per-zone lighting editor with a live LED preview, native colour picker and
//! an explicit colour count (one colour by default, add more when needed).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, DropDown, Label, Orientation, Scale, StringList};

use mylianpc_core::color::RgbColor;
use mylianpc_core::device::ZoneInfo;
use mylianpc_core::effect::{LightingEffect, ZoneLighting};
use mylianpc_core::profile::LightingSettings;

use crate::client;
use crate::ui_kit::{
    self, card, field, label, page, primary_button, secondary_button, ColorField, LedPreview,
};
use crate::widgets::{message_label, show_error, show_result};

fn effect_name(effect: LightingEffect) -> &'static str {
    match effect {
        LightingEffect::Off => "Off",
        LightingEffect::Static => "Static",
        LightingEffect::Breathing => "Breathing",
        LightingEffect::Flash => "Flash",
        LightingEffect::ColorCycle => "Colour cycle",
        LightingEffect::Rainbow => "Rainbow",
        LightingEffect::RainbowWave => "Rainbow wave",
        LightingEffect::Wave => "Wave",
        LightingEffect::Gradient => "Gradient",
    }
}

fn effect_uses_colours(effect: LightingEffect) -> bool {
    !matches!(
        effect,
        LightingEffect::Off | LightingEffect::Rainbow | LightingEffect::RainbowWave
    )
}

/// Static uses only the first colour; the animated effects can use several.
fn effect_allows_multiple(effect: LightingEffect) -> bool {
    matches!(
        effect,
        LightingEffect::Breathing
            | LightingEffect::Flash
            | LightingEffect::ColorCycle
            | LightingEffect::Wave
            | LightingEffect::Gradient
    )
}

/// Trailing repeated colours add nothing to a gradient or wave; trimming them
/// stops the old always-six-fields UI from accumulating duplicates in profiles.
fn effective_colour_count(colors: &[RgbColor]) -> usize {
    let mut count = colors.len().max(1);
    while count > 1 && colors[count - 1] == colors[count - 2] {
        count -= 1;
    }
    count
}

struct State {
    zones: Vec<ZoneInfo>,
    lighting: LightingSettings,
    effects: Vec<LightingEffect>,
}

#[derive(Clone)]
struct ColourControls {
    fields: Vec<ColorField>,
    labels: Vec<Label>,
    section: GtkBox,
    add: Button,
    remove: Button,
    single: Button,
    count: Label,
    visible: Rc<Cell<usize>>,
}

impl ColourControls {
    fn new() -> Self {
        let fields: Vec<ColorField> = (0..6).map(|_| ColorField::new(RgbColor::WHITE)).collect();
        let labels: Vec<Label> = (0..6)
            .map(|index| label(&format!("Colour {}", index + 1), "field-label"))
            .collect();
        let section = GtkBox::new(Orientation::Vertical, 8);
        section.append(&ui_kit::separator());
        section.append(&label("Colours", "section-title"));
        let colour_list = GtkBox::new(Orientation::Vertical, 6);
        for (label, field) in labels.iter().zip(fields.iter()) {
            let row = GtkBox::new(Orientation::Horizontal, 8);
            label.set_width_chars(11);
            row.append(label);
            row.append(&field.row());
            colour_list.append(&row);
        }
        section.append(&colour_list);

        section.append(&label("Quick colours", "field-hint"));
        let palette_row = gtk::FlowBox::new();
        palette_row.set_selection_mode(gtk::SelectionMode::None);
        palette_row.set_column_spacing(4);
        palette_row.set_row_spacing(2);
        palette_row.set_max_children_per_line(8);
        palette_row.set_valign(gtk::Align::Start);
        for (name, hex) in [
            ("White", "#FFFFFF"),
            ("Warm", "#FFB06A"),
            ("Red", "#FF4D5E"),
            ("Green", "#2ECC71"),
            ("Teal", "#00E0B0"),
            ("Blue", "#3B82F6"),
            ("Violet", "#8A6CFF"),
            ("Off", "#000000"),
        ] {
            let button = Button::with_label(name);
            button.add_css_class("btn");
            button.add_css_class("btn-ghost");
            let field = fields[0].clone();
            let color = RgbColor::try_parse_hex(hex).unwrap_or(RgbColor::BLACK);
            button.connect_clicked(move |_| field.set_color(color));
            palette_row.insert(&button, -1);
        }
        section.append(&palette_row);

        let add_row = GtkBox::new(Orientation::Horizontal, 8);
        let add = secondary_button("+ Add colour");
        let remove = ui_kit::ghost_button("Remove last");
        let single = ui_kit::ghost_button("Use one colour");
        let count = label("1 colour", "field-hint");
        add_row.append(&add);
        add_row.append(&remove);
        add_row.append(&single);
        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        add_row.append(&spacer);
        add_row.append(&count);
        section.append(&add_row);

        let controls = Self {
            fields,
            labels,
            section,
            add,
            remove,
            single,
            count,
            visible: Rc::new(Cell::new(1)),
        };
        controls.set_count(1, 6);
        controls
    }

    fn set_count(&self, count: usize, max: usize) {
        let count = count.clamp(1, max.max(1));
        self.visible.set(count);
        for index in 0..self.fields.len() {
            let visible = index < count;
            self.labels[index].set_visible(visible);
            self.fields[index].entry.set_visible(visible);
            self.fields[index].button.set_visible(visible);
        }
    }

    fn sync(&self, effect: LightingEffect, max_colors: usize) {
        let uses = effect_uses_colours(effect);
        self.section.set_visible(uses);
        let visible = self.visible.get();
        self.add
            .set_sensitive(uses && effect_allows_multiple(effect) && visible < max_colors);
        self.remove.set_sensitive(uses && visible > 1);
        self.single.set_sensitive(uses && visible > 1);
        self.count.set_text(&format!(
            "{visible} of {max_colors} colour{}",
            if max_colors == 1 { "" } else { "s" }
        ));
    }
}

pub fn build() -> gtk::Widget {
    let (scroller, content) = page(
        "Lighting",
        "Choose a zone, an effect and colours. Changes reach the hardware immediately.",
    );

    let (zones, lighting) = match (client::zones(), client::lighting()) {
        (Ok(zones), Ok(lighting)) => (zones, lighting),
        (Err(error), _) | (_, Err(error)) => {
            content.append(&error_card(&error));
            return scroller.upcast();
        }
    };
    if zones.is_empty() {
        content.append(&error_card(
            "No lighting zones are advertised by the connected adapters.",
        ));
        return scroller.upcast();
    }

    let state = Rc::new(RefCell::new(State {
        effects: zones.first().map(|zone| zone.effects.clone()).unwrap_or_default(),
        zones,
        lighting,
    }));

    let zone_dropdown = DropDown::from_strings(&[] as &[&str]);
    zone_dropdown.set_hexpand(true);
    let effect_dropdown = DropDown::from_strings(&[] as &[&str]);
    effect_dropdown.set_hexpand(true);
    let colours = ColourControls::new();
    let brightness = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    let speed = Scale::with_range(Orientation::Horizontal, 0.0, 100.0, 1.0);
    brightness.set_hexpand(true);
    speed.set_hexpand(true);
    brightness.set_value(100.0);
    speed.set_value(50.0);
    let brightness_value = label("100%", "value");
    let speed_value = label("50%", "value");
    let note = ui_kit::wrapped("", "field-hint");
    let message = message_label();
    let (preview, preview_area) = LedPreview::new();
    let (preview_card, preview_body) = card(Some("Preview"));
    preview_card.set_valign(gtk::Align::Start);

    {
        let state = state.borrow();
        let names: Vec<String> = state
            .zones
            .iter()
            .map(|zone| format!("{} · {} LEDs", zone.name, zone.led_count))
            .collect();
        zone_dropdown.set_model(Some(&StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        zone_dropdown.set_selected(0);
    }

    let (settings_card, settings_body) = card(Some("Zone settings"));
    let basics = GtkBox::new(Orientation::Horizontal, 10);
    let zone_field = field("Zone", &zone_dropdown);
    zone_field.set_hexpand(true);
    let effect_field = field("Effect", &effect_dropdown);
    effect_field.set_hexpand(true);
    basics.append(&zone_field);
    basics.append(&effect_field);
    settings_body.append(&basics);
    settings_body.append(&colours.section);

    settings_body.append(&ui_kit::separator());
    let brightness_row = GtkBox::new(Orientation::Vertical, 3);
    let brightness_head = GtkBox::new(Orientation::Horizontal, 8);
    brightness_head.append(&label("Brightness", "field-label"));
    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    brightness_head.append(&spacer);
    brightness_head.append(&brightness_value);
    brightness_row.append(&brightness_head);
    brightness_row.append(&brightness);
    settings_body.append(&brightness_row);
    let speed_row = GtkBox::new(Orientation::Vertical, 3);
    let speed_head = GtkBox::new(Orientation::Horizontal, 8);
    speed_head.append(&label("Speed", "field-label"));
    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    speed_head.append(&spacer);
    speed_head.append(&speed_value);
    speed_row.append(&speed_head);
    speed_row.append(&speed);
    settings_body.append(&speed_row);
    settings_body.append(&note);

    preview_body.append(&preview_area);
    preview_body.append(&ui_kit::separator());
    preview_body.append(&message);
    let actions = ui_kit::toolbar();
    let all = primary_button("Apply to every zone");
    let apply = secondary_button("Apply to this zone");
    let off = secondary_button("Turn this zone off");
    let reset = ui_kit::danger_button("Turn every zone off");
    let reload = ui_kit::ghost_button("Reload");
    actions.insert(&apply, -1);
    actions.insert(&all, -1);
    actions.insert(&off, -1);
    actions.insert(&reload, -1);
    actions.insert(&reset, -1);
    preview_body.append(&actions);
    preview_body.append(&ui_kit::wrapped(
        "Animations are drawn by the device itself where it supports them; \
         rainbow, rainbow wave and wave on the ARGB headers are streamed per LED.",
        "field-hint",
    ));

    let columns = ui_kit::adaptive_columns(860);
    columns.append(&settings_card);
    columns.append(&preview_card);
    content.append(&columns);

    load_zone(
        &state,
        0,
        &effect_dropdown,
        &colours,
        &brightness,
        &speed,
        &brightness_value,
        &speed_value,
        &note,
    );
    refresh_preview(
        &state,
        &zone_dropdown,
        &effect_dropdown,
        &colours,
        &brightness,
        &speed,
        &preview,
    );

    {
        let state = Rc::clone(&state);
        let effect_dropdown = effect_dropdown.clone();
        let colours = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let brightness_value = brightness_value.clone();
        let speed_value = speed_value.clone();
        let note = note.clone();
        let preview = preview.clone();
        zone_dropdown.connect_selected_notify(move |dropdown| {
            let index = dropdown.selected() as usize;
            load_zone(
                &state,
                index,
                &effect_dropdown,
                &colours,
                &brightness,
                &speed,
                &brightness_value,
                &speed_value,
                &note,
            );
            refresh_preview(
                &state,
                dropdown,
                &effect_dropdown,
                &colours,
                &brightness,
                &speed,
                &preview,
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let colours = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let preview = preview.clone();
        effect_dropdown.connect_selected_notify(move |effect| {
            let Some(zone) = selected_zone(&state, &zone_dropdown) else {
                return;
            };
            let effect_value = selected_effect(&state, effect);
            if !effect_allows_multiple(effect_value) {
                colours.set_count(1, zone.max_colors);
                if let Some(first) = colours.fields[0].color() {
                    colours.fields[0].set_color(first);
                }
            }
            colours.sync(effect_value, zone.max_colors);
            refresh_preview(
                &state,
                &zone_dropdown,
                effect,
                &colours,
                &brightness,
                &speed,
                &preview,
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours_for_add = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let preview = preview.clone();
        colours.add.connect_clicked(move |_| {
            let Some(zone) = selected_zone(&state, &zone_dropdown) else {
                return;
            };
            let visible = colours_for_add.visible.get();
            if visible >= zone.max_colors {
                return;
            }
            if let Some(previous) = colours_for_add.fields[visible - 1].color() {
                colours_for_add.fields[visible].set_color(previous);
            }
            colours_for_add.set_count(visible + 1, zone.max_colors);
            let effect = selected_effect(&state, &effect_dropdown);
            colours_for_add.sync(effect, zone.max_colors);
            refresh_preview(
                &state,
                &zone_dropdown,
                &effect_dropdown,
                &colours_for_add,
                &brightness,
                &speed,
                &preview,
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours_for_remove = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let preview = preview.clone();
        colours.remove.connect_clicked(move |_| {
            let Some(zone) = selected_zone(&state, &zone_dropdown) else {
                return;
            };
            let visible = colours_for_remove.visible.get();
            if visible <= 1 {
                return;
            }
            colours_for_remove.set_count(visible - 1, zone.max_colors);
            let effect = selected_effect(&state, &effect_dropdown);
            colours_for_remove.sync(effect, zone.max_colors);
            refresh_preview(
                &state,
                &zone_dropdown,
                &effect_dropdown,
                &colours_for_remove,
                &brightness,
                &speed,
                &preview,
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours_for_single = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let preview = preview.clone();
        colours.single.connect_clicked(move |_| {
            let Some(zone) = selected_zone(&state, &zone_dropdown) else {
                return;
            };
            colours_for_single.set_count(1, zone.max_colors);
            let effect = selected_effect(&state, &effect_dropdown);
            colours_for_single.sync(effect, zone.max_colors);
            refresh_preview(
                &state,
                &zone_dropdown,
                &effect_dropdown,
                &colours_for_single,
                &brightness,
                &speed,
                &preview,
            );
        });
    }
    for field in &colours.fields {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let preview = preview.clone();
        field.entry.connect_changed(move |_| {
            refresh_preview(
                &state,
                &zone_dropdown,
                &effect_dropdown,
                &colours,
                &brightness,
                &speed,
                &preview,
            );
        });
    }
    {
        let value = brightness_value.clone();
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours = colours.clone();
        let speed = speed.clone();
        let preview = preview.clone();
        let brightness_scale = brightness.clone();
        brightness.connect_value_changed(move |scale| {
            value.set_text(&format!("{:.0}%", scale.value()));
            refresh_preview(
                &state,
                &zone_dropdown,
                &effect_dropdown,
                &colours,
                &brightness_scale,
                &speed,
                &preview,
            );
        });
    }
    {
        let value = speed_value.clone();
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours = colours.clone();
        let brightness = brightness.clone();
        let speed_scale = speed.clone();
        let preview = preview.clone();
        speed.connect_value_changed(move |scale| {
            value.set_text(&format!("{:.0}%", scale.value()));
            refresh_preview(
                &state,
                &zone_dropdown,
                &effect_dropdown,
                &colours,
                &brightness,
                &speed_scale,
                &preview,
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let message = message.clone();
        apply.connect_clicked(move |_| {
            let Some(zone) = selected_zone(&state, &zone_dropdown) else {
                show_error(&message, "Choose a zone first.");
                return;
            };
            let lighting =
                read_controls(&state, &effect_dropdown, &colours, &brightness, &speed);
            let zone_id = zone.id.clone();
            message.set_text("Applying…");
            let message = message.clone();
            client::run_async(
                move || client::apply_zone(&zone_id, lighting),
                move |result| show_result(&message, result),
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let message = message.clone();
        all.connect_clicked(move |_| {
            if selected_zone(&state, &zone_dropdown).is_none() {
                show_error(&message, "Choose a zone first.");
                return;
            }
            let lighting =
                read_controls(&state, &effect_dropdown, &colours, &brightness, &speed);
            message.set_text("Applying to every zone…");
            let message = message.clone();
            client::run_async(
                move || {
                    client::apply_all(LightingSettings {
                        sync_all: Some(lighting),
                        zones: Default::default(),
                    })
                },
                move |result| show_result(&message, result),
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let speed = speed.clone();
        let message = message.clone();
        off.connect_clicked(move |_| {
            let Some(zone) = selected_zone(&state, &zone_dropdown) else {
                show_error(&message, "Choose a zone first.");
                return;
            };
            let lighting = ZoneLighting {
                effect: LightingEffect::Off,
                colors: vec![RgbColor::BLACK],
                brightness: 0,
                speed: speed.value().round() as i32,
            };
            let zone_id = zone.id.clone();
            let message = message.clone();
            message.set_text("Turning the zone off…");
            client::run_async(
                move || client::apply_zone(&zone_id, lighting),
                move |result| show_result(&message, result),
            );
        });
    }
    {
        let message = message.clone();
        reset.connect_clicked(move |_| {
            ui_kit::confirm(
                "Turn every lighting zone off?",
                "This applies Off to the motherboard headers, both memory modules and the pump ring.",
                "Turn off",
                true,
                {
                    let message = message.clone();
                    move || {
                        message.set_text("Turning every zone off…");
                        let message = message.clone();
                        client::run_async(client::reset_lighting, move |result| {
                            show_result(&message, result)
                        });
                    }
                },
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let zone_dropdown = zone_dropdown.clone();
        let effect_dropdown = effect_dropdown.clone();
        let colours = colours.clone();
        let brightness = brightness.clone();
        let speed = speed.clone();
        let brightness_value = brightness_value.clone();
        let speed_value = speed_value.clone();
        let note = note.clone();
        let preview = preview.clone();
        let message = message.clone();
        reload.connect_clicked(move |_| match (client::zones(), client::lighting()) {
            (Ok(zones), Ok(lighting)) => {
                let selected = zone_dropdown.selected() as usize;
                {
                    let mut state = state.borrow_mut();
                    state.zones = zones;
                    state.lighting = lighting;
                    let names: Vec<String> = state
                        .zones
                        .iter()
                        .map(|zone| format!("{} · {} LEDs", zone.name, zone.led_count))
                        .collect();
                    zone_dropdown.set_model(Some(&StringList::new(
                        &names.iter().map(String::as_str).collect::<Vec<_>>(),
                    )));
                    zone_dropdown.set_selected(selected.min(names.len().saturating_sub(1)) as u32);
                }
                load_zone(
                    &state,
                    selected,
                    &effect_dropdown,
                    &colours,
                    &brightness,
                    &speed,
                    &brightness_value,
                    &speed_value,
                    &note,
                );
                refresh_preview(
                    &state,
                    &zone_dropdown,
                    &effect_dropdown,
                    &colours,
                    &brightness,
                    &speed,
                    &preview,
                );
                show_result(&message, Ok("Lighting reloaded.".to_string()));
            }
            (Err(error), _) | (_, Err(error)) => show_error(&message, error),
        });
    }

    scroller.upcast()
}

fn error_card(message: &str) -> GtkBox {
    let card = GtkBox::new(Orientation::Vertical, 4);
    card.add_css_class("card-error");
    card.append(&label("Cannot edit lighting", "notice-title"));
    card.append(&ui_kit::wrapped(message, "value-muted"));
    card
}

fn set_effects(dropdown: &DropDown, effects: &[LightingEffect]) {
    let names: Vec<&str> = effects.iter().map(|effect| effect_name(*effect)).collect();
    dropdown.set_model(Some(&StringList::new(&names)));
    dropdown.set_selected(0);
}

fn selected_effect(state: &Rc<RefCell<State>>, dropdown: &DropDown) -> LightingEffect {
    let state = state.borrow();
    state
        .effects
        .get(dropdown.selected() as usize)
        .copied()
        .unwrap_or(LightingEffect::Static)
}

fn selected_zone(state: &Rc<RefCell<State>>, dropdown: &DropDown) -> Option<ZoneInfo> {
    let state = state.borrow();
    state.zones.get(dropdown.selected() as usize).cloned()
}

#[allow(clippy::too_many_arguments)]
fn load_zone(
    state: &Rc<RefCell<State>>,
    index: usize,
    effect_dropdown: &DropDown,
    colours: &ColourControls,
    brightness: &Scale,
    speed: &Scale,
    brightness_value: &Label,
    speed_value: &Label,
    note: &Label,
) {
    let Some((zone, requested)) = ({
        let state = state.borrow();
        state.zones.get(index).cloned().map(|zone| {
            let requested = state
                .lighting
                .for_zone(&zone.id)
                .cloned()
                .unwrap_or_default();
            (zone, requested)
        })
    }) else {
        return;
    };
    {
        let mut state = state.borrow_mut();
        state.effects = zone.effects.clone();
    }
    set_effects(effect_dropdown, &zone.effects);
    if let Some(position) = zone
        .effects
        .iter()
        .position(|effect| *effect == requested.effect)
    {
        effect_dropdown.set_selected(position as u32);
    }

    let mut count = effective_colour_count(&requested.colors);
    if !effect_allows_multiple(requested.effect) {
        count = 1;
    }
    count = count.min(zone.max_colors).max(1);
    colours.set_count(count, zone.max_colors);
    colours.sync(requested.effect, zone.max_colors);
    for index in 0..colours.fields.len() {
        let colour = requested
            .colors
            .get(index)
            .copied()
            .or_else(|| requested.colors.first().copied())
            .unwrap_or(RgbColor::WHITE);
        colours.fields[index].set_color(colour);
    }
    brightness.set_value(requested.brightness.clamp(0, 100) as f64);
    speed.set_value(requested.speed.clamp(0, 100) as f64);
    brightness_value.set_text(&format!("{}%", requested.brightness.clamp(0, 100)));
    speed_value.set_text(&format!("{}%", requested.speed.clamp(0, 100)));
    note.set_text(&format!(
        "{} LEDs · up to {} colours.{}",
        zone.led_count,
        zone.max_colors,
        zone.note
            .as_deref()
            .map(|note| format!(" {note}"))
            .unwrap_or_default()
    ));
}

#[allow(clippy::too_many_arguments)]
fn read_controls(
    state: &Rc<RefCell<State>>,
    effect_dropdown: &DropDown,
    colours: &ColourControls,
    brightness: &Scale,
    speed: &Scale,
) -> ZoneLighting {
    let effect = selected_effect(state, effect_dropdown);
    let count = if effect_allows_multiple(effect) {
        colours.visible.get()
    } else {
        1
    };
    let mut selected: Vec<RgbColor> = colours
        .fields
        .iter()
        .take(count)
        .filter_map(ColorField::color)
        .collect();
    if selected.is_empty() {
        selected.push(RgbColor::WHITE);
    }
    while selected.len() > 1 && selected[selected.len() - 1] == selected[selected.len() - 2] {
        selected.pop();
    }
    ZoneLighting {
        effect,
        colors: selected,
        brightness: brightness.value().round() as i32,
        speed: speed.value().round() as i32,
    }
}

#[allow(clippy::too_many_arguments)]
fn refresh_preview(
    state: &Rc<RefCell<State>>,
    zone_dropdown: &DropDown,
    effect_dropdown: &DropDown,
    colours: &ColourControls,
    brightness: &Scale,
    speed: &Scale,
    preview: &LedPreview,
) {
    let Some(zone) = selected_zone(state, zone_dropdown) else {
        return;
    };
    let lighting = read_controls(state, effect_dropdown, colours, brightness, speed);
    preview.update(lighting, zone.led_count);
}
