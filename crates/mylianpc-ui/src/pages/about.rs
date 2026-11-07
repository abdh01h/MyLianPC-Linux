//! About page: version, licence and project links.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::LinkButton;

use crate::client;
use crate::ui_kit::{self, card, label, page, pill, wrapped};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "https://github.com/abdh01h/MyLianPC-Linux";
const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

pub fn build() -> gtk::Widget {
    let (scroller, content) = page("About", "Version, licence and project information.");

    let (identity_card, identity) = card(None);
    identity.append(&label("MyLianPC Linux", "about-name"));
    identity.append(&wrapped(
        "Native hardware control for the GIGABYTE B850 AORUS ELITE WIFI7, Lian Li HydroShift II \
         LCD-C and Corsair Vengeance RGB DDR5. The service owns the hardware; this window is a \
         thin client.",
        "value-muted",
    ));
    let version = label(
        &format!("Window {VERSION} · checking the service…"),
        "mono",
    );
    version.set_selectable(true);
    identity.append(&version);
    identity.append(&pill("GPL-3.0-or-later", "pill-accent"));
    content.append(&identity_card);

    let columns = ui_kit::two_columns_flow();

    let (license_card, license_body) = card(Some("License"));
    license_body.append(&wrapped(
        "MyLianPC Linux is free software: you can run it for any purpose, study it, share it and \
         change it.",
        "value-muted",
    ));
    license_body.append(&wrapped(
        "If you distribute it or a modified version, it must stay under the GNU General Public \
         License version 3 (or later) and the corresponding source must be available. Nobody can \
         turn the project or a published update into closed-source software. There is no \
         warranty.",
        "value-muted",
    ));
    let license_link = LinkButton::with_label(LICENSE_URL, "Read the GNU GPL v3");
    style_link(&license_link);
    license_body.append(&license_link);
    license_card.set_hexpand(true);
    columns.insert(&license_card, -1);

    let (links_card, links_body) = card(Some("Project"));
    links_body.append(&wrapped(
        "The source, documentation and issue tracker live on GitHub.",
        "value-muted",
    ));
    let links = ui_kit::toolbar();
    for (uri, text) in [
        (REPO, "Source code"),
        ("https://github.com/abdh01h/MyLianPC-Linux/issues", "Report an issue"),
        ("https://github.com/abdh01h/MyLianPC-Linux#readme", "Documentation"),
    ] {
        let link = LinkButton::with_label(uri, text);
        style_link(&link);
        links.insert(&link, -1);
    }
    links_body.append(&links);
    links_card.set_hexpand(true);
    columns.insert(&links_card, -1);
    content.append(&columns);

    let (service_card, service_body) = card(Some("Service"));
    let status_line = label("Checking service…", "value-muted");
    status_line.set_wrap(true);
    status_line.set_selectable(true);
    service_body.append(&status_line);
    let socket = label(
        &format!("Socket: {}", mylianpc_core::ipc::socket_path().display()),
        "field-hint",
    );
    socket.set_selectable(true);
    service_body.append(&socket);
    service_body.append(&wrapped(
        "The service is the only process that touches hardware. This window reads and writes \
         through the local Unix socket and never opens a device node.",
        "field-hint",
    ));
    content.append(&service_card);

    client::run_async(client::status, move |result| match result {
        Ok(status) => status_line.set_text(&format!(
            "Window {VERSION} · service {} · {} device(s) · active profile: {}",
            status.version,
            status.devices.len(),
            status.active_profile_name.as_deref().unwrap_or("none")
        )),
        Err(error) => {
            status_line.add_css_class("error-text");
            status_line.set_text(&format!(
                "Service unreachable ({error}). Start it with: sudo systemctl start mylianpc"
            ));
        }
    });

    scroller.upcast()
}

fn style_link(link: &LinkButton) {
    link.add_css_class("btn");
    link.add_css_class("btn-secondary");
    link.set_halign(gtk::Align::Start);
    link.set_valign(gtk::Align::Center);
}
