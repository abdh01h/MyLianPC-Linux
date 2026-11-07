//! MyLianPC GTK4 UI.
//!
//! The window is a thin client: it talks to `mylianpc-service` over the Unix
//! socket and never opens a device node. It follows the system theme through
//! the semantic tokens in `style.css`, and it does not install a client-side
//! titlebar, so the desktop draws the window frame (KDE's Global Theme window
//! decoration, the X11 window manager, or GTK's default decoration).

mod client;
mod pages;
mod ui_kit;
mod widgets;

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Box as GtkBox, Button, DrawingArea, Image, Label,
    ListBox, ListBoxRow, Orientation, Overlay, SelectionMode, Stack,
};

fn main() -> gtk::glib::ExitCode {
    let page = std::env::var("MYLIANPC_PAGE").ok();
    let app_id = std::env::var("MYLIANPC_APP_ID")
        .unwrap_or_else(|_| "io.github.mylianpc.MyLianPC".to_string());
    let app = Application::builder()
        .application_id(&app_id)
        .build();
    app.connect_activate(move |app| build_ui(app, page.as_deref()));
    app.run()
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("style.css"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

const NAV: &[(&str, &str, &str)] = &[
    ("dashboard", "Dashboard", "view-grid-symbolic"),
    ("lighting", "Lighting", "applications-graphics-symbolic"),
    ("lcd", "LCD", "view-fullscreen-symbolic"),
    ("profiles", "Profiles", "folder-documents-symbolic"),
    ("cooling", "Cooling", "applications-science-symbolic"),
    ("devices", "Devices", "computer-symbolic"),
    ("settings", "Settings", "preferences-system-symbolic"),
    ("about", "About", "help-about-symbolic"),
];

struct Toast {
    label: Label,
}

impl Toast {
    fn show(&self, message: &str, error: bool) {
        self.label.remove_css_class("error-text");
        self.label.remove_css_class("success-text");
        self.label
            .add_css_class(if error { "error-text" } else { "success-text" });
        self.label.set_text(message);
        self.label.set_visible(true);
        let label = self.label.clone();
        gtk::glib::timeout_add_seconds_local_once(4, move || {
            label.set_visible(false);
        });
    }
}

fn build_ui(app: &Application, page: Option<&str>) {
    if let Some(window) = app.windows().first() {
        window.present();
        return;
    }
    // Wayland resolves the window icon from the desktop entry named after the
    // application id; X11 takes it from the default icon name.
    gtk::Window::set_default_icon_name("mylianpc");
    install_css();

    let stack = Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);
    let page_defs: [(&str, &str, gtk::Widget); 8] = [
        ("dashboard", "Dashboard", pages::dashboard::build()),
        ("lighting", "Lighting", pages::lighting::build()),
        ("lcd", "LCD", pages::lcd::build()),
        ("profiles", "Profiles", pages::profiles::build()),
        ("cooling", "Cooling", pages::cooling::build()),
        ("devices", "Devices", pages::devices::build()),
        ("settings", "Settings", pages::settings::build()),
        ("about", "About", pages::about::build()),
    ];
    for (name, title, widget) in &page_defs {
        if std::env::var_os("MYLIANPC_DEBUG_SIZES").is_some() {
            let (min, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
            eprintln!("page {name}: min width {min}, natural width {natural}");
        }
        stack.add_titled(widget, Some(name), title);
    }

    // ------------------------------------------------------------------ toast
    let toast = Rc::new(Toast {
        label: {
            let label = Label::new(None);
            label.add_css_class("toast");
            label.set_halign(gtk::Align::End);
            label.set_valign(gtk::Align::End);
            label.set_margin_end(20);
            label.set_margin_bottom(18);
            label.set_visible(false);
            label
        },
    });

    // ------------------------------------------------------------------ sidebar
    let sidebar = GtkBox::new(Orientation::Vertical, 10);
    sidebar.add_css_class("sidebar");
    sidebar.set_size_request(222, -1);

    let brand = GtkBox::new(Orientation::Horizontal, 10);
    brand.set_margin_start(6);
    brand.set_margin_bottom(10);
    let logo = DrawingArea::new();
    logo.set_content_width(32);
    logo.set_content_height(32);
    logo.set_draw_func(|_, context, width, height| {
        let center = width as f64 / 2.0;
        let radius = (width.min(height) as f64 / 2.0) - 1.0;
        context.set_source_rgb(0.02, 0.06, 0.09);
        context.arc(center, center, radius, 0.0, std::f64::consts::TAU);
        let _ = context.fill();
        context.set_source_rgb(0.0, 0.88, 0.69);
        context.set_line_width(3.0);
        context.arc(center, center, radius - 3.0, -2.2, 1.1);
        let _ = context.stroke();
        context.set_source_rgb(0.54, 0.42, 1.0);
        context.arc(center, center, radius - 3.0, 1.2, 3.4);
        let _ = context.stroke();
        context.set_source_rgb(0.91, 0.94, 0.97);
        context.arc(center, center, 4.0, 0.0, std::f64::consts::TAU);
        let _ = context.fill();
    });
    brand.append(&logo);
    let brand_text = GtkBox::new(Orientation::Vertical, 0);
    brand_text.append(&ui_kit::label("MyLianPC", "brand-title"));
    brand_text.append(&ui_kit::label("Hardware control", "brand-subtitle"));
    brand.append(&brand_text);
    sidebar.append(&brand);

    let nav = ListBox::new();
    nav.add_css_class("nav-list");
    nav.set_selection_mode(SelectionMode::Single);
    let mut rows = Vec::new();
    for (name, title, icon) in NAV {
        let row = ListBoxRow::new();
        let content = GtkBox::new(Orientation::Horizontal, 10);
        content.add_css_class("nav-row");
        let icon_slot = GtkBox::new(Orientation::Horizontal, 0);
        icon_slot.set_size_request(32, -1);
        let image = Image::from_icon_name(icon);
        image.set_pixel_size(20);
        image.set_halign(gtk::Align::Center);
        image.add_css_class("nav-icon");
        icon_slot.append(&image);
        content.append(&icon_slot);
        content.append(&ui_kit::label(title, ""));
        row.set_child(Some(&content));
        nav.append(&row);
        rows.push(((*name).to_string(), row));
    }
    sidebar.append(&nav);

    let spacer = GtkBox::new(Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    sidebar.append(&spacer);

    let footer = GtkBox::new(Orientation::Vertical, 6);
    footer.add_css_class("sidebar-footer");
    let footer_body = GtkBox::new(Orientation::Vertical, 6);
    footer.append(&footer_body);
    let status_row = GtkBox::new(Orientation::Horizontal, 7);
    let dot = Label::new(Some("●"));
    dot.add_css_class("error-text");
    let status_text = ui_kit::label("Connecting…", "row-subtitle");
    status_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    status_row.append(&dot);
    status_row.append(&status_text);
    footer_body.append(&status_row);
    let profile_text = ui_kit::label("Profile: —", "row-subtitle");
    profile_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    footer_body.append(&profile_text);
    let socket_text = ui_kit::label("", "field-hint");
    socket_text.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    socket_text.set_tooltip_text(Some(&mylianpc_core::ipc::socket_path().display().to_string()));
    footer_body.append(&socket_text);
    let reapply = Button::new();
    reapply.add_css_class("btn");
    reapply.add_css_class("btn-secondary");
    reapply.set_margin_top(4);
    let reapply_content = GtkBox::new(Orientation::Horizontal, 6);
    let refresh_icon = Image::from_icon_name("view-refresh-symbolic");
    refresh_icon.set_pixel_size(16);
    reapply_content.append(&refresh_icon);
    reapply_content.append(&ui_kit::label("Re-apply profile", ""));
    reapply.set_child(Some(&reapply_content));
    {
        let toast = Rc::clone(&toast);
        reapply.connect_clicked(move |_| {
            let toast = Rc::clone(&toast);
            toast.show("Re-applying the active profile…", false);
            client::run_async(client::reapply, move |result| match result {
                Ok(message) => toast.show(&message, false),
                Err(error) => toast.show(&error, true),
            });
        });
    }
    footer_body.append(&reapply);
    sidebar.append(&footer);
    {
        let dot = dot.clone();
        let status_text = status_text.clone();
        let profile_text = profile_text.clone();
        let socket_text = socket_text.clone();
        let busy = Rc::new(Cell::new(false));
        let refresh_footer = {
            let busy = Rc::clone(&busy);
            move || {
                if busy.replace(true) {
                    return;
                }
                let dot = dot.clone();
                let status_text = status_text.clone();
                let profile_text = profile_text.clone();
                let socket_text = socket_text.clone();
                let busy = Rc::clone(&busy);
                client::run_async(client::status, move |status| {
                    busy.set(false);
                    match status {
                        Ok(status) => {
                            dot.remove_css_class("error-text");
                            dot.remove_css_class("success-text");
                            dot.add_css_class("success-text");
                            status_text.set_text(&format!(
                                "Service {} · {} device(s)",
                                status.version,
                                status.devices.len()
                            ));
                            profile_text.set_text(&format!(
                                "Profile: {}",
                                status.active_profile_name.as_deref().unwrap_or("none")
                            ));
                        }
                        Err(_) => {
                            dot.remove_css_class("success-text");
                            dot.remove_css_class("error-text");
                            dot.add_css_class("error-text");
                            status_text.set_text("Service unavailable");
                            profile_text.set_text("Profile: —");
                        }
                    }
                    socket_text
                        .set_text(&mylianpc_core::ipc::socket_path().display().to_string());
                });
            }
        };
        refresh_footer();
        gtk::glib::timeout_add_seconds_local(2, move || {
            refresh_footer();
            gtk::glib::ControlFlow::Continue
        });
    }

    // --------------------------------------------------------------- content
    let overlay = Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.add_overlay(&toast.label);

    let main = GtkBox::new(Orientation::Horizontal, 0);
    if std::env::var_os("MYLIANPC_DEBUG_SIZES").is_some() {
        for (name, widget) in [
            ("sidebar", sidebar.clone().upcast::<gtk::Widget>()),
            ("stack", stack.clone().upcast::<gtk::Widget>()),
            ("overlay", overlay.clone().upcast::<gtk::Widget>()),
            ("toast", toast.label.clone().upcast::<gtk::Widget>()),
        ] {
            let (min, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
            eprintln!("{name}: min {min}, natural {natural}");
        }
    }
    main.append(&sidebar);
    main.append(&overlay);

    if std::env::var_os("MYLIANPC_DEBUG_SIZES").is_some() {
        let (min, natural, _, _) = main.measure(gtk::Orientation::Horizontal, -1);
        eprintln!("main: min {min}, natural {natural}");
    }
    let window = ApplicationWindow::builder()
        .application(app)
        .title("MyLianPC")
        .default_width(1150)
        .default_height(800)
        .child(&main)
        .build();
    window.add_css_class("mylianpc");
    window.set_resizable(true);
    if std::env::var_os("MYLIANPC_DEBUG_SIZES").is_some() {
        let (min, natural, _, _) = window.measure(gtk::Orientation::Horizontal, -1);
        eprintln!("window: min {min}, natural {natural}, resizable {}", window.is_resizable());
        let window_for_idle = window.clone();
        gtk::glib::idle_add_local_once(move || {
            let (min, natural, _, _) = window_for_idle.measure(gtk::Orientation::Horizontal, -1);
            eprintln!(
                "window after present: min {min}, natural {natural}, resizable {}, size {}x{}",
                window_for_idle.is_resizable(),
                window_for_idle.width(),
                window_for_idle.height()
            );
        });
    }
    set_tone(&window);
    if let Some(settings) = gtk::Settings::default() {
        let window_for_theme = window.clone();
        settings.connect_gtk_theme_name_notify(move |_| {
            let window = window_for_theme.clone();
            gtk::glib::idle_add_local_once(move || set_tone(&window));
        });
        let window_for_dark = window.clone();
        settings.connect_gtk_application_prefer_dark_theme_notify(move |_| {
            let window = window_for_dark.clone();
            gtk::glib::idle_add_local_once(move || set_tone(&window));
        });
    }

    // ------------------------------------------------------------ navigation
    let initial = page
        .filter(|page| NAV.iter().any(|(name, _, _)| name == page))
        .unwrap_or("dashboard");
    if let Some((_, row)) = rows.iter().find(|(name, _)| name == initial) {
        nav.select_row(Some(row));
        stack.set_visible_child_name(initial);
    }
    {
        let stack = stack.clone();
        let rows = rows.clone();
        nav.connect_row_selected(move |_, selected| {
            if let Some(selected) = selected {
                if let Some((name, _)) = rows.iter().find(|(_, row)| row == selected) {
                    stack.set_visible_child_name(name);
                }
            }
        });
    }

    // Ctrl+1 … Ctrl+8 jump between pages. Arrow keys change the focused select
    // (GtkDropDown) directly, whether its popup is open or closed.
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let rows = rows.clone();
        let window_for_keys = window.clone();
        key.connect_key_pressed(move |_, keyval, _code, state| {
            let modifiers = gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK;
            if !state.intersects(modifiers)
                && matches!(keyval, gtk::gdk::Key::Up | gtk::gdk::Key::Down)
            {
                if let Some(dropdown) = focused_dropdown(&window_for_keys) {
                    let delta = if keyval == gtk::gdk::Key::Up { -1 } else { 1 };
                    if step_dropdown(&dropdown, delta) {
                        return gtk::glib::Propagation::Stop;
                    }
                }
            }
            if !state.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return gtk::glib::Propagation::Proceed;
            }
            let index = match keyval {
                gtk::gdk::Key::_1 => 0,
                gtk::gdk::Key::_2 => 1,
                gtk::gdk::Key::_3 => 2,
                gtk::gdk::Key::_4 => 3,
                gtk::gdk::Key::_5 => 4,
                gtk::gdk::Key::_6 => 5,
                gtk::gdk::Key::_7 => 6,
                gtk::gdk::Key::_8 => 7,
                _ => return gtk::glib::Propagation::Proceed,
            };
            if let Some((_, row)) = rows.get(index) {
                nav.select_row(Some(row));
            }
            gtk::glib::Propagation::Stop
        });
    }
    window.add_controller(key);

    window.present();
    // GTK focuses the first focusable control while mapping, which leaves a
    // bright focus ring on an untouched card. Start with no control focused;
    // Tab and the shortcuts still work normally.
    let window_for_focus = window.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
        gtk::prelude::GtkWindowExt::set_focus(&window_for_focus, None::<&gtk::Widget>);
    });
}

fn focused_dropdown(window: &ApplicationWindow) -> Option<gtk::DropDown> {
    let mut widget = gtk::prelude::GtkWindowExt::focus(window)?;
    loop {
        if let Ok(dropdown) = widget.clone().downcast::<gtk::DropDown>() {
            return Some(dropdown);
        }
        widget = widget.parent()?;
    }
}

fn step_dropdown(dropdown: &gtk::DropDown, delta: i32) -> bool {
    let Some(model) = dropdown.model() else {
        return false;
    };
    let count = model.n_items();
    if count == 0 {
        return false;
    }
    let current = dropdown.selected();
    dropdown.set_selected(stepped_selection(current, count, delta));
    true
}

fn stepped_selection(current: u32, count: u32, delta: i32) -> u32 {
    if count == 0 {
        return 0;
    }
    let current = if current >= count { 0 } else { current as i64 };
    (current + delta as i64).rem_euclid(count as i64) as u32
}

fn set_tone(window: &ApplicationWindow) {
    window.remove_css_class("dark-tone");
    window.remove_css_class("light-tone");
    window.add_css_class(if is_dark(window) { "dark-tone" } else { "light-tone" });
}

#[allow(deprecated)]
fn is_dark(window: &ApplicationWindow) -> bool {
    if let Some(color) = window.style_context().lookup_color("theme_bg_color") {
        let luminance =
            0.2126 * color.red() + 0.7152 * color.green() + 0.0722 * color.blue();
        return luminance < 0.5;
    }
    if let Some(settings) = gtk::Settings::default() {
        if settings.is_gtk_application_prefer_dark_theme() {
            return true;
        }
        if let Some(name) = settings.gtk_theme_name() {
            return name.to_lowercase().contains("dark");
        }
    }
    false
}


#[cfg(test)]
mod tests {
    use super::stepped_selection;

    #[test]
    fn arrow_selection_wraps_at_both_ends() {
        assert_eq!(stepped_selection(0, 3, -1), 2);
        assert_eq!(stepped_selection(2, 3, 1), 0);
        assert_eq!(stepped_selection(1, 3, 1), 2);
        assert_eq!(stepped_selection(1, 3, -1), 0);
        assert_eq!(stepped_selection(u32::MAX, 3, 1), 1);
        assert_eq!(stepped_selection(0, 0, 1), 0);
    }
}
