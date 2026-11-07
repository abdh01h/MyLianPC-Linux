//! Unix-socket IPC server. Every request is re-validated here.

#![allow(clippy::items_after_test_module)]

use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

use base64::Engine;
use mylianpc_core::ipc::{Request, Response};
use serde::Serialize;

use crate::manager::DeviceManager;

pub fn handle_request(
    manager: &Arc<Mutex<DeviceManager>>,
    request: Request,
) -> Response {
    match request {
        Request::Ping => Response::Pong {
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        Request::Inventory => match to_value(&mylianpc_hardware::inventory::Inventory::collect()) {
            Ok(inventory) => Response::Inventory { inventory },
            Err(message) => Response::error(message),
        },
        Request::Fusion2 => with_manager(manager, |_| {
            // Serialize with the adapter's writes: opening a second HID handle
            // and writing feature reports while a packet is in flight would
            // corrupt the controller's report stream.
            let result = mylianpc_hardware::fusion2::find_vendor_hidraw().and_then(|hid| {
                mylianpc_hardware::fusion2::query_info(&hid, 0x5711)
            });
            match result.and_then(|info| serde_json::to_value(info).map_err(anyhow::Error::from)) {
                Ok(info) => Response::Fusion2 { info },
                Err(error) => Response::error(format!("Fusion2 query failed: {error:#}")),
            }
        }),
        Request::Profiles => with_manager(manager, |manager| {
            match to_value(manager.profiles.snapshot()) {
                Ok(document) => Response::Profiles { document },
                Err(message) => Response::error(message),
            }
        }),
        Request::Sensors => with_manager(manager, |manager| match to_value(&manager.snapshot) {
            Ok(snapshot) => Response::Sensors { snapshot },
            Err(message) => Response::error(message),
        }),
        Request::Ram => with_manager(manager, |_| {
            match to_value(&mylianpc_hardware::corsair::detect_all()) {
                Ok(buses) => Response::Ram { buses },
                Err(message) => Response::error(message),
            }
        }),
        Request::Status => with_manager(manager, |manager| match to_value(&manager.status()) {
            Ok(service) => Response::Status { service },
            Err(message) => Response::error(message),
        }),
        Request::Zones => with_manager(manager, |manager| match to_value(&manager.zones()) {
            Ok(zones) => Response::Zones { zones },
            Err(message) => Response::error(message),
        }),
        Request::LightingGet => with_manager(manager, |manager| {
            match to_value(&manager.lighting_get()) {
                Ok(lighting) => Response::Lighting { lighting },
                Err(message) => Response::error(message),
            }
        }),
        Request::LightingApplyZone { zone_id, lighting } => with_manager(manager, |manager| {
            let report = manager.apply_zone(&zone_id, lighting);
            match to_value(&report) {
                Ok(report) => Response::Report { report },
                Err(message) => Response::error(message),
            }
        }),
        Request::LightingApplyAll { lighting } => with_manager(manager, |manager| {
            let report = manager.apply_all_lighting(lighting);
            match to_value(&report) {
                Ok(report) => Response::Report { report },
                Err(message) => Response::error(message),
            }
        }),
        Request::LightingReset => with_manager(manager, |manager| {
            let report = manager.lighting_reset();
            match to_value(&report) {
                Ok(report) => Response::Report { report },
                Err(message) => Response::error(message),
            }
        }),
        Request::Reapply => with_manager(manager, |manager| {
            let report = manager.reapply();
            match to_value(&report) {
                Ok(report) => Response::Report { report },
                Err(message) => Response::error(message),
            }
        }),
        Request::ProfileGet { id } => with_manager(manager, |manager| {
            match manager.profiles.get(id) {
                Ok(profile) => match to_value(profile) {
                    Ok(profile) => Response::Profile { profile },
                    Err(message) => Response::error(message),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfileUpsert { profile } => with_manager(manager, |manager| {
            match manager.profiles.upsert(profile) {
                Ok(profile) => match to_value(&profile) {
                    Ok(profile) => Response::Profile { profile },
                    Err(message) => Response::error(message),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfileDelete { id } => with_manager(manager, |manager| {
            match manager.profiles.delete(id) {
                Ok(()) => Response::Ok {
                    message: "Profile deleted.".to_string(),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfileDuplicate { id, name } => with_manager(manager, |manager| {
            match manager.profiles.duplicate(id, &name) {
                Ok(profile) => match to_value(&profile) {
                    Ok(profile) => Response::Profile { profile },
                    Err(message) => Response::error(message),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfileRename { id, name } => with_manager(manager, |manager| {
            match manager.profiles.rename(id, &name) {
                Ok(profile) => match to_value(&profile) {
                    Ok(profile) => Response::Profile { profile },
                    Err(message) => Response::error(message),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfileActivate { id } => with_manager(manager, |manager| {
            match manager.activate_profile(id) {
                Ok(report) => match to_value(&report) {
                    Ok(report) => Response::Report { report },
                    Err(message) => Response::error(message),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfileImport { json } => with_manager(manager, |manager| {
            match manager.profiles.import(&json) {
                Ok(profile) => match to_value(&profile) {
                    Ok(profile) => Response::Profile { profile },
                    Err(message) => Response::error(message),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfileExport { id } => with_manager(manager, |manager| {
            match manager.profiles.export(id) {
                Ok(json) => Response::Export { json },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::ProfilesReset => with_manager(manager, |manager| {
            match manager.profiles.reset_defaults() {
                Ok(()) => Response::Ok {
                    message: "Example profiles restored.".to_string(),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::CoolingGet => with_manager(manager, |manager| {
            let curves = manager
                .profiles
                .active()
                .and_then(|profile| profile.cooling.clone())
                .map(|cooling| cooling.curves)
                .unwrap_or_default();
            match (to_value(&curves), to_value(&manager.cooling_defaults())) {
                (Ok(curves), Ok(defaults)) => Response::Cooling { curves, defaults },
                (Err(message), _) | (_, Err(message)) => Response::error(message),
            }
        }),
        Request::CoolingSet { curves } => with_manager(manager, |manager| {
            match manager.cooling_set(curves) {
                Ok(()) => Response::Ok {
                    message: "Cooling curves stored.".to_string(),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::CoolingRestore => with_manager(manager, |manager| {
            match manager.cooling_restore() {
                Ok(()) => Response::Ok {
                    message: "Cooling restored to the Balanced defaults.".to_string(),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::LcdGet => with_manager(manager, |manager| {
            match (
                to_value(&manager.lcd_settings()),
                to_value(&manager.lcd_themes()),
            ) {
                (Ok(settings), Ok(themes)) => Response::Lcd { settings, themes },
                (Err(message), _) | (_, Err(message)) => Response::error(message),
            }
        }),
        Request::LcdSet { settings } => with_manager(manager, |manager| {
            match manager.lcd_set(settings) {
                Ok(()) => Response::Ok {
                    message: "LCD settings stored.".to_string(),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::LcdPush => with_manager(manager, |manager| match manager.lcd_push_now() {
            Ok(()) => Response::Ok {
                message: "LCD frame pushed.".to_string(),
            },
            Err(error) => Response::error(format!("LCD push failed: {error}")),
        }),
        Request::LcdPreview => with_manager(manager, |manager| {
            preview_response(manager.lcd_preview())
        }),
        Request::LcdPreviewDraft { settings } => with_manager(manager, |manager| {
            preview_response(manager.lcd_preview_with(&settings))
        }),
        Request::MediaUpload { file_name, base64 } => with_manager(manager, |manager| {
            match base64::engine::general_purpose::STANDARD.decode(base64) {
                Ok(data) => match manager.store_media(&file_name, &data) {
                    Ok(name) => Response::Media {
                        files: serde_json::json!(manager.media_files()),
                        stored: Some(name),
                    },
                    Err(error) => Response::error(error.to_string()),
                },
                Err(error) => Response::error(format!("Invalid base64 data: {error}")),
            }
        }),
        Request::MediaList => with_manager(manager, |manager| Response::Media {
            files: serde_json::json!(manager.media_files()),
            stored: None,
        }),
        Request::SettingsGet => with_manager(manager, |manager| {
            match to_value(&manager.settings_get()) {
                Ok(settings) => Response::Settings { settings },
                Err(message) => Response::error(message),
            }
        }),
        Request::SettingsUpdate { settings } => with_manager(manager, |manager| {
            match manager.settings_update(settings) {
                Ok(()) => Response::Ok {
                    message: "Device settings stored.".to_string(),
                },
                Err(error) => Response::error(error.to_string()),
            }
        }),
        Request::Network => with_manager(manager, |manager| {
            match to_value(&manager.network_snapshot) {
                Ok(snapshot) => Response::Network { snapshot },
                Err(message) => Response::error(message),
            }
        }),
        Request::Rescan => with_manager(manager, |manager| {
            manager.rescan();
            Response::Ok {
                message: "Devices rescanned.".to_string(),
            }
        }),
        Request::Diagnostics => with_manager(manager, |manager| Response::Diagnostics {
            text: manager.diagnostics(),
        }),
    }
}

fn preview_response(png: Vec<u8>) -> Response {
    if png.is_empty() {
        Response::error("Could not render the LCD preview.")
    } else {
        Response::Preview {
            png_base64: base64::engine::general_purpose::STANDARD.encode(png),
        }
    }
}

fn with_manager(
    manager: &Arc<Mutex<DeviceManager>>,
    f: impl FnOnce(&mut DeviceManager) -> Response,
) -> Response {
    match manager.lock() {
        Ok(mut manager) => f(&mut manager),
        Err(error) => Response::error(format!("service state lock poisoned: {error}")),
    }
}

fn to_value<T: Serialize>(value: &T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| format!("serializing response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> (tempfile::TempDir, Arc<Mutex<DeviceManager>>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::ServicePaths {
            root: dir.path().to_path_buf(),
        };
        let manager = DeviceManager::new(paths).expect("manager");
        (dir, Arc::new(Mutex::new(manager)))
    }

    #[test]
    fn read_only_requests_serialize() {
        let (_dir, manager) = manager();
        for request in [
            Request::Status,
            Request::Zones,
            Request::LightingGet,
            Request::Profiles,
            Request::Sensors,
            Request::CoolingGet,
            Request::LcdGet,
            Request::SettingsGet,
            Request::Network,
            Request::Diagnostics,
            Request::MediaList,
        ] {
            let response = handle_request(&manager, request);
            assert!(
                !matches!(response, Response::Error { .. }),
                "request failed: {response:?}"
            );
        }
    }

    #[test]
    fn lcd_preview_is_a_png() {
        let (_dir, manager) = manager();
        match handle_request(&manager, Request::LcdPreview) {
            Response::Preview { png_base64 } => {
                let png = base64::engine::general_purpose::STANDARD
                    .decode(png_base64)
                    .unwrap();
                assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn zones_include_the_motherboard_and_the_pump_ring() {
        let (_dir, manager) = manager();
        match handle_request(&manager, Request::Zones) {
            Response::Zones { zones } => {
                let zones: Vec<mylianpc_core::device::ZoneInfo> =
                    serde_json::from_value(zones).unwrap();
                assert!(zones.iter().any(|zone| zone.id == "fusion2:argb1"));
                assert!(zones.iter().any(|zone| zone.id == "hydroshift:ring"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn lcd_settings_and_cooling_curves_are_stored_without_hardware() {
        let (_dir, manager) = manager();
        let settings = mylianpc_core::profile::LcdSettings {
            theme_id: "overview".to_string(),
            brightness: 35,
            ..Default::default()
        };
        assert!(matches!(
            handle_request(&manager, Request::LcdSet { settings }),
            Response::Ok { .. }
        ));
        let curves = std::collections::BTreeMap::from([(
            "hydroshift:fans".to_string(),
            mylianpc_core::fan::presets::silent(mylianpc_core::fan::TemperatureSource::Coolant),
        )]);
        assert!(matches!(
            handle_request(&manager, Request::CoolingSet { curves }),
            Response::Ok { .. }
        ));
        match handle_request(&manager, Request::Profiles) {
            Response::Profiles { document } => {
                let document: mylianpc_core::profile::ProfileDocument =
                    serde_json::from_value(document).unwrap();
                let active = document
                    .profiles
                    .iter()
                    .find(|profile| Some(profile.id) == document.active_profile_id)
                    .expect("working profile");
                assert_eq!(active.lcd.as_ref().unwrap().theme_id, "overview");
                assert!(active.cooling.as_ref().unwrap().curves.contains_key("hydroshift:fans"));
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn lcd_draft_preview_does_not_change_the_saved_settings() {
        let (_dir, manager) = manager();
        let saved = match handle_request(&manager, Request::LcdGet) {
            Response::Lcd { settings, .. } => settings,
            other => panic!("unexpected response: {other:?}"),
        };
        let mut draft: mylianpc_core::profile::LcdSettings =
            serde_json::from_value(saved.clone()).unwrap();
        draft.theme_id = "neon".to_string();
        draft.brightness = 33;
        match handle_request(&manager, Request::LcdPreviewDraft { settings: draft }) {
            Response::Preview { png_base64 } => assert!(!png_base64.is_empty()),
            other => panic!("unexpected response: {other:?}"),
        }
        match handle_request(&manager, Request::LcdGet) {
            Response::Lcd { settings, .. } => {
                let after: mylianpc_core::profile::LcdSettings =
                    serde_json::from_value(settings).unwrap();
                let before: mylianpc_core::profile::LcdSettings =
                    serde_json::from_value(saved).unwrap();
                assert_eq!(after.theme_id, before.theme_id);
                assert_eq!(after.brightness, before.brightness);
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn media_upload_is_validated_and_stored() {
        let (_dir, manager) = manager();
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut png = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut png),
                image::ImageFormat::Png,
            )
            .unwrap();
        let response = handle_request(
            &manager,
            Request::MediaUpload {
                file_name: "dot.png".to_string(),
                base64: base64::engine::general_purpose::STANDARD.encode(&png),
            },
        );
        match response {
            Response::Media { files, stored } => {
                assert!(stored.is_some());
                let files: Vec<String> = serde_json::from_value(files).unwrap();
                assert_eq!(files.len(), 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }
        let bad = handle_request(
            &manager,
            Request::MediaUpload {
                file_name: "bad.exe".to_string(),
                base64: base64::engine::general_purpose::STANDARD.encode(b"nope"),
            },
        );
        assert!(matches!(bad, Response::Error { .. }));
    }

    #[test]
    fn invalid_requests_produce_an_error_response() {
        let (_dir, manager) = manager();
        let response = handle_request(
            &manager,
            Request::ProfileDelete {
                id: uuid::Uuid::new_v4(),
            },
        );
        assert!(matches!(response, Response::Error { .. }));
    }
}

/// One client connection: newline-delimited JSON in, newline-delimited JSON out.
pub fn serve_connection(
    stream: std::os::unix::net::UnixStream,
    manager: Arc<Mutex<DeviceManager>>,
) -> anyhow::Result<()> {
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle_request(&manager, request),
            Err(error) => Response::error(format!("invalid request: {error}")),
        };
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
    }
    Ok(())
}
