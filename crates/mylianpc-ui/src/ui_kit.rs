//! Reusable UI building blocks and the small canvas previews.
//!
//! The visual language is defined in `style.css` and `docs/design-system.md`.
//! These helpers keep spacing, typography and component states consistent
//! across every page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, DrawingArea, Entry, Label, Orientation, ScrolledWindow};

use mylianpc_core::color::RgbColor;
use mylianpc_core::effect::{self, ZoneLighting};
use mylianpc_core::fan::CurvePoint;

pub const PAGE_X: i32 = 20;
pub const PAGE_TOP: i32 = 20;
pub const PAGE_BOTTOM: i32 = 24;
pub const SECTION_GAP: i32 = 14;
pub const CONTROL_GAP: i32 = 8;

pub fn label(text: &str, class: &str) -> Label {
    let label = Label::new(Some(text));
    label.set_xalign(0.0);
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}

pub fn wrapped(text: &str, class: &str) -> Label {
    let label = label(text, class);
    label.set_wrap(true);
    label.set_selectable(true);
    label
}

pub fn big_value(text: &str, class: &str) -> Label {
    label(text, class)
}

/// Page header + scrollable content column.
pub fn page(title: &str, subtitle: &str) -> (ScrolledWindow, GtkBox) {
    let root = GtkBox::new(Orientation::Vertical, 0);
    let header = GtkBox::new(Orientation::Vertical, 3);
    header.set_margin_top(PAGE_TOP);
    header.set_margin_start(PAGE_X);
    header.set_margin_end(PAGE_X);
    header.append(&label(title, "page-title"));
    // Page subtitles are chrome, not content: selectable labels are focusable
    // and GTK would focus the first one on startup, leaving its text selected.
    let subtitle_label = label(subtitle, "page-subtitle");
    subtitle_label.set_wrap(true);
    subtitle_label.set_selectable(false);
    header.append(&subtitle_label);
    root.append(&header);

    let content = GtkBox::new(Orientation::Vertical, SECTION_GAP);
    content.set_margin_top(16);
    content.set_margin_bottom(PAGE_BOTTOM);
    content.set_margin_start(PAGE_X);
    content.set_margin_end(PAGE_X);
    root.append(&content);

    let scroller = ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&root)
        .build();
    (scroller, content)
}

/// A card with an optional title. The returned box is the content area.
pub fn card(title: Option<&str>) -> (GtkBox, GtkBox) {
    let card = GtkBox::new(Orientation::Vertical, 10);
    card.add_css_class("card");
    if let Some(title) = title {
        card.append(&label(title, "card-title"));
    }
    let body = GtkBox::new(Orientation::Vertical, 8);
    card.append(&body);
    (card, body)
}

/// A wrapping action row: buttons flow to the next line when the window is
/// narrow instead of forcing the page wider.
pub fn toolbar() -> gtk::FlowBox {
    let box_ = gtk::FlowBox::new();
    box_.set_selection_mode(gtk::SelectionMode::None);
    box_.set_column_spacing(CONTROL_GAP as u32);
    box_.set_row_spacing(6);
    box_.set_max_children_per_line(8);
    box_.set_min_children_per_line(1);
    box_.set_halign(gtk::Align::Start);
    box_.set_valign(gtk::Align::Start);
    box_
}

/// Two side-by-side regions that automatically stack when the window is narrow.
pub fn adaptive_columns(threshold: i32) -> GtkBox {
    let container = GtkBox::new(Orientation::Horizontal, SECTION_GAP);
    let container_for_timer = container.clone();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        if container_for_timer.is_mapped() {
            let wanted = if container_for_timer.width() < threshold {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            };
            if container_for_timer.orientation() != wanted {
                container_for_timer.set_orientation(wanted);
            }
        }
        gtk::glib::ControlFlow::Continue
    });
    container
}


/// Two equal cards that reflow to one column when the window is narrow.
pub fn two_columns_flow() -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_max_children_per_line(2);
    flow.set_min_children_per_line(1);
    flow.set_column_spacing(SECTION_GAP as u32);
    flow.set_row_spacing(SECTION_GAP as u32);
    flow.set_valign(gtk::Align::Start);
    flow
}

pub fn field(label_text: &str, widget: &impl IsA<gtk::Widget>) -> GtkBox {
    let box_ = GtkBox::new(Orientation::Vertical, 4);
    let text = label(label_text, "field-label");
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    box_.append(&text);
    box_.append(widget);
    box_
}

pub fn field_with_hint(
    label_text: &str,
    hint: &str,
    widget: &impl IsA<gtk::Widget>,
) -> GtkBox {
    let box_ = field(label_text, widget);
    box_.append(&wrapped(hint, "field-hint"));
    box_
}

pub fn primary_button(text: &str) -> Button {
    let button = Button::with_label(text);
    button.add_css_class("btn");
    button.add_css_class("btn-primary");
    button
}

pub fn secondary_button(text: &str) -> Button {
    let button = Button::with_label(text);
    button.add_css_class("btn");
    button.add_css_class("btn-secondary");
    button
}

pub fn ghost_button(text: &str) -> Button {
    let button = Button::with_label(text);
    button.add_css_class("btn");
    button.add_css_class("btn-ghost");
    button
}

pub fn danger_button(text: &str) -> Button {
    let button = Button::with_label(text);
    button.add_css_class("btn");
    button.add_css_class("btn-danger");
    button
}

pub fn pill(text: &str, kind: &str) -> Label {
    let label = Label::new(Some(text));
    label.add_css_class("pill");
    label.add_css_class(kind);
    label.set_halign(gtk::Align::Start);
    label
}

pub fn pill_for_state(state: &str) -> Label {
    match state {
        "connected" => pill("Connected", "pill-ok"),
        "not-found" | "disconnected" => pill("Not found", "pill-muted"),
        "busy" => pill("In use", "pill-warn"),
        "needs-driver" => pill("Driver missing", "pill-warn"),
        "unresponsive" => pill("Not responding", "pill-error"),
        "error" => pill("Error", "pill-error"),
        _ => pill("Unknown", "pill-muted"),
    }
}

fn application() -> Option<gtk::Application> {
    gtk::gio::Application::default().and_then(|app| app.downcast::<gtk::Application>().ok())
}

/// The visible application window, used to parent native dialogs.
pub fn active_window() -> Option<gtk::Window> {
    application().and_then(|app| app.active_window())
}

pub fn separator() -> GtkBox {
    let line = GtkBox::new(Orientation::Vertical, 0);
    line.add_css_class("divider");
    line.set_margin_top(4);
    line.set_margin_bottom(4);
    line
}

// ---------------------------------------------------------------------------
// Colour field with a native picker
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ColorField {
    pub entry: Entry,
    pub button: Button,
    preview: DrawingArea,
    current: Rc<Cell<RgbColor>>,
}

impl ColorField {
    pub fn new(initial: RgbColor) -> Self {
        let current = Rc::new(Cell::new(initial));
        let preview = DrawingArea::new();
        preview.set_content_width(22);
        preview.set_content_height(22);
        {
            let current = Rc::clone(&current);
            preview.set_draw_func(move |_, context, width, height| {
                let color = current.get();
                context.set_source_rgb(
                    color.r as f64 / 255.0,
                    color.g as f64 / 255.0,
                    color.b as f64 / 255.0,
                );
                context.rectangle(0.0, 0.0, width as f64, height as f64);
                let _ = context.fill();
            });
        }
        let button = Button::new();
        button.add_css_class("color-swatch");
        button.set_child(Some(&preview));
        button.set_tooltip_text(Some("Choose a colour"));
        let entry = Entry::new();
        entry.set_text(&initial.to_hex());
        entry.set_width_chars(8);
        entry.set_max_length(7);
        let field = Self {
            entry,
            button,
            preview,
            current,
        };
        {
            let field = field.clone();
            field.entry.connect_changed(move |entry| {
                if let Some(color) = RgbColor::try_parse_hex(&entry.text()) {
                    field.current.set(color);
                    field.preview.queue_draw();
                }
            });
        }
        {
            let button = field.button.clone();
            let field = field.clone();
            button.connect_clicked(move |_| {
                let color = field.current.get();
                let rgba = gtk::gdk::RGBA::new(
                    color.r as f32 / 255.0,
                    color.g as f32 / 255.0,
                    color.b as f32 / 255.0,
                    1.0,
                );
                #[allow(deprecated)]
                {
                    let parent = active_window();
                    let dialog = gtk::ColorChooserDialog::new(
                        Some("Pick a Color"),
                        parent.as_ref(),
                    );
                    dialog.set_modal(true);
                    // Show the palette and the colour editor together; the
                    // editor is hidden by default, which leaves only a short
                    // palette and forces a scrollbar for the full controls.
                    dialog.set_show_editor(true);
                    // GtkColorChooserDialog ignores set_default_size (it
                    // always maps at its tiny natural size, which scrolls the
                    // editor), so force a size that fits the full editor.
                    dialog.set_size_request(450, 460);
                    dialog.set_resizable(true);
                    dialog.set_rgba(&rgba);
                    // GTK owns windows through the application; a local-only
                    // dialog would be destroyed as soon as this handler returns.
                    if let Some(app) = application() {
                        app.add_window(&dialog);
                    }
                    let field = field.clone();
                    dialog.connect_response(move |dialog, response| {
                        if response == gtk::ResponseType::Ok {
                            let rgba = dialog.rgba();
                            field.set_color(RgbColor::new(
                                (rgba.red() * 255.0).round() as u8,
                                (rgba.green() * 255.0).round() as u8,
                                (rgba.blue() * 255.0).round() as u8,
                            ));
                        }
                        dialog.close();
                    });
                    dialog.present();
                }
            });
        }
        field
    }

    pub fn set_color(&self, color: RgbColor) {
        self.current.set(color);
        self.entry.set_text(&color.to_hex());
        self.preview.queue_draw();
    }

    pub fn color(&self) -> Option<RgbColor> {
        RgbColor::try_parse_hex(&self.entry.text())
    }

    pub fn row(&self) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 6);
        row.append(&self.button);
        row.append(&self.entry);
        row
    }
}

/// A compact colour row: swatch + hex entry + a palette shortcut.
pub fn color_row(text: &str, field: &ColorField) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 8);
    let label = label(text, "field-label");
    label.set_width_chars(12);
    row.append(&label);
    row.append(&field.row());
    row
}

// ---------------------------------------------------------------------------
// LED strip preview
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct LedPreview {
    model: Rc<RefCell<(ZoneLighting, Vec<RgbColor>)>>,
    area: DrawingArea,
}

impl LedPreview {
    pub fn new() -> (Self, DrawingArea) {
        let model = Rc::new(RefCell::new((
            ZoneLighting::default(),
            vec![RgbColor::BLACK; 12],
        )));
        let area = DrawingArea::new();
        area.set_content_height(58);
        area.set_hexpand(true);
        {
            let model = Rc::clone(&model);
            area.set_draw_func(move |_, context, width, height| {
                let (lighting, frame) = model.borrow().clone();
                let count = frame.len().max(1);
                let margin = 8.0;
                let usable = width as f64 - margin * 2.0;
                let gap = 2.0;
                let cell = (usable / count as f64).max(3.0);
                let bar_height = (height as f64 - 18.0).max(12.0);
                let y = 6.0;
                context.set_source_rgba(1.0, 1.0, 1.0, 0.06);
                context.rectangle(margin, y, usable, bar_height);
                let _ = context.fill();
                for (index, color) in frame.iter().enumerate() {
                    let x = margin + index as f64 * cell;
                    context.set_source_rgb(
                        color.r as f64 / 255.0,
                        color.g as f64 / 255.0,
                        color.b as f64 / 255.0,
                    );
                    context.rectangle(x, y, (cell - gap).max(1.0), bar_height);
                    let _ = context.fill();
                }
                context.set_source_rgba(1.0, 1.0, 1.0, 0.55);
                context.set_font_size(11.0);
                context.move_to(
                    margin,
                    height as f64 - 4.0,
                );
                let _ = context.show_text(&format!(
                    "{} LEDs · {} · {}% brightness",
                    count, format_effect(lighting.effect), lighting.brightness
                ));
            });
        }
        (
            Self {
                model,
                area: area.clone(),
            },
            area,
        )
    }

    pub fn update(&self, lighting: ZoneLighting, led_count: usize) {
        let frame = effect::preview_frame(&lighting, led_count);
        *self.model.borrow_mut() = (lighting, frame);
        self.area.queue_draw();
    }
}

fn format_effect(effect: mylianpc_core::effect::LightingEffect) -> &'static str {
    use mylianpc_core::effect::LightingEffect;
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

// ---------------------------------------------------------------------------
// Fan-curve preview
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CurvePreview {
    model: Rc<RefCell<(Vec<CurvePoint>, Option<f64>)>>,
    area: DrawingArea,
}

impl CurvePreview {
    pub fn new() -> (Self, DrawingArea) {
        let model = Rc::new(RefCell::new((Vec::<CurvePoint>::new(), None::<f64>)));
        let area = DrawingArea::new();
        area.set_content_height(150);
        area.set_hexpand(true);
        {
            let model = Rc::clone(&model);
            area.set_draw_func(move |_, context, width, height| {
                let (points, current) = model.borrow().clone();
                let width = width as f64;
                let height = height as f64;
                let left = 34.0;
                let right = 10.0;
                let top = 10.0;
                let bottom = 22.0;
                let plot_w = (width - left - right).max(1.0);
                let plot_h = (height - top - bottom).max(1.0);
                context.set_source_rgba(1.0, 1.0, 1.0, 0.08);
                context.set_line_width(1.0);
                for step in 0..=4 {
                    let y = top + plot_h * step as f64 / 4.0;
                    context.move_to(left, y);
                    context.line_to(left + plot_w, y);
                    let _ = context.stroke();
                }
                context.set_source_rgba(1.0, 1.0, 1.0, 0.45);
                context.set_font_size(10.0);
                for (label, value) in [("100%", 0.0), ("50%", 0.5), ("0%", 1.0)] {
                    context.move_to(4.0, top + plot_h * value + 3.0);
                    let _ = context.show_text(label);
                }
                if points.len() < 2 {
                    return;
                }
                let max_temp = 100.0f64;
                let point = |temperature: f64, duty: i32| {
                    (
                        left + (temperature / max_temp).clamp(0.0, 1.0) * plot_w,
                        top + (1.0 - duty as f64 / 100.0) * plot_h,
                    )
                };
                context.set_source_rgba(0.0, 0.88, 0.69, 0.16);
                context.move_to(left, top + plot_h);
                for p in &points {
                    let (x, y) = point(p.temperature_c, p.duty_percent);
                    context.line_to(x, y);
                }
                let (last_x, _) = point(points.last().unwrap().temperature_c, 0);
                context.line_to(last_x.min(left + plot_w), top + plot_h);
                context.close_path();
                let _ = context.fill();
                context.set_source_rgb(0.0, 0.88, 0.69);
                context.set_line_width(2.0);
                for (index, p) in points.iter().enumerate() {
                    let (x, y) = point(p.temperature_c, p.duty_percent);
                    if index == 0 {
                        context.move_to(x, y);
                    } else {
                        context.line_to(x, y);
                    }
                }
                let _ = context.stroke();
                for p in &points {
                    let (x, y) = point(p.temperature_c, p.duty_percent);
                    context.arc(x, y, 3.0, 0.0, std::f64::consts::TAU);
                    let _ = context.fill();
                }
                if let Some(current) = current {
                    let x = left + (current / max_temp).clamp(0.0, 1.0) * plot_w;
                    context.set_source_rgba(0.55, 0.42, 1.0, 0.85);
                    context.set_line_width(1.5);
                    context.move_to(x, top);
                    context.line_to(x, top + plot_h);
                    let _ = context.stroke();
                }
                context.set_source_rgba(1.0, 1.0, 1.0, 0.45);
                context.set_font_size(10.0);
                for temperature in [0, 25, 50, 75, 100] {
                    let x = left + (temperature as f64 / max_temp) * plot_w;
                    let text = format!("{temperature}°");
                    let width = text.len() as f64 * 5.0;
                    context.move_to(x - width / 2.0, height - 5.0);
                    let _ = context.show_text(&text);
                }
            });
        }
        (
            Self {
                model,
                area: area.clone(),
            },
            area,
        )
    }

    pub fn update(&self, points: Vec<CurvePoint>, current: Option<f64>) {
        *self.model.borrow_mut() = (points, current);
        self.area.queue_draw();
    }
}

// ---------------------------------------------------------------------------
// Small dialogs
// ---------------------------------------------------------------------------

pub fn prompt_text(title: &str, initial: &str, on_ok: impl Fn(String) + 'static) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .resizable(false)
        .default_width(380)
        .build();
    let root = GtkBox::new(Orientation::Vertical, 12);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let entry = Entry::new();
    entry.set_text(initial);
    entry.set_activates_default(true);
    root.append(&entry);
    let buttons = GtkBox::new(Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = secondary_button("Cancel");
    let ok = primary_button("OK");
    ok.set_receives_default(true);
    buttons.append(&cancel);
    buttons.append(&ok);
    root.append(&buttons);
    window.set_child(Some(&root));

    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    let window_for_ok = window.clone();
    let entry_for_ok = entry.clone();
    let on_ok = Rc::new(on_ok);
    ok.connect_clicked(move |_| {
        let text = entry_for_ok.text().to_string();
        window_for_ok.close();
        on_ok(text);
    });
    entry.grab_focus();
    window.present();
}

pub fn confirm(
    title: &str,
    detail: &str,
    action: &str,
    destructive: bool,
    on_confirm: impl Fn() + 'static,
) {
    let dialog = gtk::AlertDialog::builder()
        .message(title)
        .detail(detail)
        .buttons(["Cancel", action])
        .cancel_button(0)
        .default_button(0)
        .build();
    let on_confirm = Rc::new(on_confirm);
    dialog.choose(
        active_window().as_ref(),
        None::<&gtk::gio::Cancellable>,
        move |result| {
            if let Ok(choice) = result {
                if choice == 1 {
                    let _ = destructive;
                    on_confirm();
                }
            }
        },
    );
}
