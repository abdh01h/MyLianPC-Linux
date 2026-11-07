//! Shared hardware profiles with per-row actions and clear active state.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{Box as GtkBox, Entry, Label, Orientation};
use uuid::Uuid;

use mylianpc_core::profile::{CoolingSettings, HardwareProfile, ProfileDocument};

use crate::client;
use crate::ui_kit::{
    self, card, danger_button, label, page, pill, primary_button, secondary_button,
};
use crate::widgets::{message_label, show_error, show_result};
use crate::ui_kit::prompt_text;

struct State {
    document: ProfileDocument,
}

pub fn build() -> gtk::Widget {
    let (scroller, content) = page(
        "Profiles",
        "One shared list for the whole machine. Activating a profile applies its lighting, LCD layout and cooling curves.",
    );

    let document = match client::profiles() {
        Ok(document) => document,
        Err(error) => {
            let card = GtkBox::new(Orientation::Vertical, 4);
            card.add_css_class("card-error");
            card.append(&label("Profiles unavailable", "notice-title"));
            card.append(&ui_kit::wrapped(&error, "value-muted"));
            content.append(&card);
            return scroller.upcast();
        }
    };
    let state = Rc::new(RefCell::new(State { document }));

    let active = label("", "value-muted");
    content.append(&active);
    let message = message_label();

    let (save_card, save_body) = card(Some("Save the current settings"));
    let save_row = GtkBox::new(Orientation::Horizontal, 10);
    let name = Entry::new();
    name.set_placeholder_text(Some("New profile name"));
    name.set_hexpand(true);
    let save = primary_button("Save as new profile");
    save_row.append(&name);
    save_row.append(&save);
    save_body.append(&save_row);
    save_body.append(&ui_kit::wrapped(
        "Captures the active lighting, LCD layout and cooling curves.",
        "field-hint",
    ));
    content.append(&save_card);

    let (list_card, list_body) = card(Some("Saved profiles"));
    let list = GtkBox::new(Orientation::Vertical, 6);
    list.add_css_class("profile-list");
    list_body.append(&list);
    let toolbar = ui_kit::toolbar();
    let import = secondary_button("Import…");
    let export_all = secondary_button("Export all…");
    let reset = secondary_button("Restore examples");
    let refresh = secondary_button("Reload");
    toolbar.insert(&import, -1);
    toolbar.insert(&export_all, -1);
    toolbar.insert(&reset, -1);
    toolbar.insert(&refresh, -1);
    list_body.append(&toolbar);
    list_body.append(&message);
    content.append(&list_card);

    rebuild(&list, &state, &active, &message);
    {
        let state = Rc::clone(&state);
        let list = list.clone();
        let active = active.clone();
        let message = message.clone();
        refresh.connect_clicked(move |_| {
            match client::profiles() {
                Ok(document) => {
                    state.borrow_mut().document = document;
                    rebuild(&list, &state, &active, &message);
                    show_result(&message, Ok("Profiles reloaded.".to_string()));
                }
                Err(error) => show_error(&message, error),
            }
        });
    }
    {
        let state = Rc::clone(&state);
        let list = list.clone();
        let active = active.clone();
        let message = message.clone();
        let import = import.clone();
        import.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Import a profile");
            let message = message.clone();
            let state = Rc::clone(&state);
            let list = list.clone();
            let active = active.clone();
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
                    match std::fs::read_to_string(&path) {
                        Ok(json) => {
                            let outcome = client::import_profile(&json);
                            if let Ok(document) = client::profiles() {
                                state.borrow_mut().document = document;
                            }
                            rebuild(&list, &state, &active, &message);
                            show_result(&message, outcome);
                        }
                        Err(error) => {
                            show_error(&message, format!("Could not read the file: {error}"))
                        }
                    }
                },
            );
        });
    }
    {
        let message = message.clone();
        export_all.connect_clicked(move |_| {
            let document = match client::profiles() {
                Ok(document) => document,
                Err(error) => {
                    show_error(&message, error);
                    return;
                }
            };
            let json = match serde_json::to_string_pretty(&document) {
                Ok(json) => json,
                Err(error) => {
                    show_error(&message, format!("Could not serialize profiles: {error}"));
                    return;
                }
            };
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Export all profiles");
            dialog.set_initial_name(Some(&format!(
                "mylianpc-profiles-{}.json",
                chrono::Local::now().format("%Y-%m-%d")
            )));
            let message = message.clone();
            dialog.save(
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
                    match std::fs::write(&path, json) {
                        Ok(()) => show_result(
                            &message,
                            Ok(format!(
                                "All profiles exported to {}.",
                                path.display()
                            )),
                        ),
                        Err(error) => {
                            show_error(&message, format!("Could not write: {error}"))
                        }
                    }
                },
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let list = list.clone();
        let active = active.clone();
        let message = message.clone();
        reset.connect_clicked(move |_| {
            ui_kit::confirm(
                "Restore the example profiles?",
                "The current profile list is replaced by the four shipped examples. Work becomes active again.",
                "Restore",
                true,
                {
                    let state = Rc::clone(&state);
                    let list = list.clone();
                    let active = active.clone();
                    let message = message.clone();
                    move || {
                        let outcome = client::reset_profiles();
                        if let Ok(document) = client::profiles() {
                            state.borrow_mut().document = document;
                        }
                        rebuild(&list, &state, &active, &message);
                        show_result(&message, outcome);
                    }
                },
            );
        });
    }
    {
        let state = Rc::clone(&state);
        let list = list.clone();
        let active = active.clone();
        let message = message.clone();
        save.connect_clicked(move |_| {
            let name_text = name.text().to_string();
            if name_text.trim().is_empty() {
                show_error(&message, "Enter a name for the new profile.");
                return;
            }
            let lighting = match client::lighting() {
                Ok(lighting) => lighting,
                Err(error) => {
                    show_error(&message, error);
                    return;
                }
            };
            let lcd = match client::lcd() {
                Ok(lcd) => lcd.settings,
                Err(error) => {
                    show_error(&message, error);
                    return;
                }
            };
            let cooling = match client::cooling() {
                Ok(cooling) if !cooling.curves.is_empty() => cooling.curves,
                Ok(cooling) => cooling.defaults,
                Err(error) => {
                    show_error(&message, error);
                    return;
                }
            };
            let profile = HardwareProfile {
                id: Uuid::new_v4(),
                name: name_text.trim().to_string(),
                lighting: Some(lighting),
                lcd: Some(lcd),
                cooling: Some(CoolingSettings { curves: cooling }),
                modified_utc: Utc::now(),
            };
            let outcome = client::save_profile(profile);
            if let Ok(document) = client::profiles() {
                state.borrow_mut().document = document;
            }
            rebuild(&list, &state, &active, &message);
            show_result(&message, outcome);
        });
    }

    scroller.upcast()
}

fn rebuild(list: &GtkBox, state: &Rc<RefCell<State>>, active_label: &Label, message: &Label) {
    let message = (*message).clone();
    while let Some(row) = list.first_child() {
        list.remove(&row);
    }
    let active_id = state.borrow().document.active_profile_id;
    let count = state.borrow().document.profiles.len();
    let active_name = active_id
        .and_then(|id| {
            state
                .borrow()
                .document
                .profiles
                .iter()
                .find(|profile| profile.id == id)
                .map(|profile| profile.name.clone())
        })
        .unwrap_or_else(|| "none".to_string());
    active_label.set_text(&format!("Active profile: {active_name} · {count} saved"));

    let profiles = state.borrow().document.profiles.clone();
    for profile in profiles {
        let body = GtkBox::new(Orientation::Vertical, 8);
        body.add_css_class("row-padded");
        body.add_css_class("profile-row");
        let head = GtkBox::new(Orientation::Horizontal, 8);
        let title_label = label(&profile.name, "row-title");
        title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title_label.set_hexpand(true);
        head.append(&title_label);
        if Some(profile.id) == active_id {
            head.append(&pill("In use", "pill-ok"));
        }
        let metadata = GtkBox::new(Orientation::Vertical, 2);
        metadata.append(&ui_kit::wrapped(
            &format!(
                "{} · {} cooling curve(s) · modified {}",
                profile
                    .lcd
                    .as_ref()
                    .map(|lcd| lcd.theme_id.as_str())
                    .unwrap_or("no LCD layout"),
                profile
                    .cooling
                    .as_ref()
                    .map(|cooling| cooling.curves.len())
                    .unwrap_or(0),
                profile
                    .modified_utc
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
            ),
            "row-subtitle",
        ));
        let actions = GtkBox::new(Orientation::Horizontal, 6);
        let activate = if Some(profile.id) == active_id {
            let button = secondary_button("Active");
            button.set_sensitive(false);
            button
        } else {
            primary_button("Activate")
        };
        let duplicate = ui_kit::ghost_button("Duplicate");
        let rename = ui_kit::ghost_button("Rename");
        let export = ui_kit::ghost_button("Export");
        let delete = danger_button("Delete");
        actions.append(&activate);
        actions.append(&duplicate);
        actions.append(&rename);
        actions.append(&export);
        actions.append(&delete);
        body.append(&head);
        body.append(&metadata);
        body.append(&actions);
        list.append(&body);

        let list_for_activate = list.clone();
        let state_for_activate = Rc::clone(state);
        let active_for_activate = active_label.clone();
        let id = profile.id;
        let name = profile.name.clone();
        {
            let list = list_for_activate.clone();
            let state = Rc::clone(&state_for_activate);
            let active = active_for_activate.clone();
            let message = message.clone();
            activate.connect_clicked(move |_| {
                message.set_text("Applying profile…");
                let state = Rc::clone(&state);
                let list = list.clone();
                let active = active.clone();
                let message = message.clone();
                client::run_async(
                    move || client::activate_profile(id),
                    move |outcome| {
                        if let Ok(document) = client::profiles() {
                            state.borrow_mut().document = document;
                        }
                        rebuild(&list, &state, &active, &message);
                        show_result(&message, outcome);
                    },
                );
            });
        }
        {
            let list = list_for_activate.clone();
            let state = Rc::clone(&state_for_activate);
            let active = active_for_activate.clone();
            let message = message.clone();
            let name = name.clone();
            duplicate.connect_clicked(move |_| {
                prompt_text("Duplicate profile", &format!("{name} copy"), {
                    let state = Rc::clone(&state);
                    let list = list.clone();
                    let active = active.clone();
                    let message = message.clone();
                    move |new_name| {
                        let outcome = client::duplicate_profile(id, new_name.trim());
                        if let Ok(document) = client::profiles() {
                            state.borrow_mut().document = document;
                        }
                        rebuild(&list, &state, &active, &message);
                        show_result(&message, outcome);
                    }
                });
            });
        }
        {
            let list = list_for_activate.clone();
            let state = Rc::clone(&state_for_activate);
            let active = active_for_activate.clone();
            let message = message.clone();
            let name = name.clone();
            rename.connect_clicked(move |_| {
                prompt_text("Rename profile", &name, {
                    let state = Rc::clone(&state);
                    let list = list.clone();
                    let active = active.clone();
                    let message = message.clone();
                    move |new_name| {
                        let outcome = client::rename_profile(id, new_name.trim());
                        if let Ok(document) = client::profiles() {
                            state.borrow_mut().document = document;
                        }
                        rebuild(&list, &state, &active, &message);
                        show_result(&message, outcome);
                    }
                });
            });
        }
        {
            let message = message.clone();
            let name = name.clone();
            export.connect_clicked(move |_| {
                let json = match client::export_profile(id) {
                    Ok(json) => json,
                    Err(error) => {
                        show_error(&message, error);
                        return;
                    }
                };
                let dialog = gtk::FileDialog::new();
                dialog.set_title("Export the profile");
                dialog.set_initial_name(Some(&format!(
                    "{}.json",
                    name.replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
                        .trim()
                )));
                let message_for_save = message.clone();
                dialog.save(
                    ui_kit::active_window().as_ref(),
                    None::<&gtk::gio::Cancellable>,
                    move |result| {
                        let Ok(file) = result else {
                            return;
                        };
                        let Some(path) = file.path() else {
                            show_error(&message_for_save, "The selected file has no local path.");
                            return;
                        };
                        match std::fs::write(&path, json) {
                            Ok(()) => show_result(
                                &message_for_save,
                                Ok(format!("Exported to {}.", path.display())),
                            ),
                            Err(error) => {
                                show_error(&message_for_save, format!("Could not write: {error}"))
                            }
                        }
                    },
                );
            });
        }
        {
            let state = Rc::clone(&state_for_activate);
            let list = list_for_activate.clone();
            let active = active_for_activate.clone();
            let message = message.clone();
            let name = name.clone();
            delete.connect_clicked(move |_| {
                ui_kit::confirm(
                    &format!("Delete the profile '{name}'?"),
                    "This removes it from the shared list. Other profiles are untouched.",
                    "Delete",
                    true,
                    {
                        let state = Rc::clone(&state);
                        let list = list.clone();
                        let active = active.clone();
                        let message = message.clone();
                        move || {
                            let outcome = client::delete_profile(id);
                            if let Ok(document) = client::profiles() {
                                state.borrow_mut().document = document;
                            }
                            rebuild(&list, &state, &active, &message);
                            show_result(&message, outcome);
                        }
                    },
                );
            });
        }
    }
}
