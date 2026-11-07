//! Thin IPC client. The UI never opens a device node.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use base64::Engine;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use uuid::Uuid;

use mylianpc_core::device::{ApplyReport, DeviceStatus, ZoneInfo};
use mylianpc_core::effect::ZoneLighting;
use mylianpc_core::fan::FanCurve;
use mylianpc_core::ipc::{socket_path, Request, Response};
use mylianpc_core::network::NetworkSnapshot;
use mylianpc_core::profile::{
    HardwareProfile, LcdSettings, LightingSettings, ProfileDocument,
};
use mylianpc_core::settings::DeviceSettings;
use mylianpc_hardware::sensors::Snapshot;

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct ServiceStatus {
    pub version: String,
    pub devices: Vec<DeviceStatus>,
    #[serde(default)]
    pub active_profile_id: Option<Uuid>,
    #[serde(default)]
    pub active_profile_name: Option<String>,
    #[serde(default)]
    pub last_apply: Option<ApplyReport>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub device_settings: DeviceSettings,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ThemeInfo {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LcdInfo {
    pub settings: LcdSettings,
    pub themes: Vec<ThemeInfo>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CoolingInfo {
    pub curves: BTreeMap<String, FanCurve>,
    pub defaults: BTreeMap<String, FanCurve>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MediaInfo {
    pub files: Vec<String>,
    #[serde(default)]
    pub stored: Option<String>,
}

/// Runs a blocking IPC call off the GTK main thread and delivers its result on
/// the main context. The window no longer freezes while the service samples
/// sensors or applies a profile.
pub fn run_async<T, F, D>(work: F, done: D)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    let mut done = Some(done);
    gtk4::glib::timeout_add_local(Duration::from_millis(25), move || {
        match receiver.try_recv() {
            Ok(value) => {
                if let Some(done) = done.take() {
                    done(value);
                }
                gtk4::glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => gtk4::glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => gtk4::glib::ControlFlow::Break,
        }
    });
}

pub fn send(request: &Request) -> Result<Response, String> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let line = serde_json::to_string(request).map_err(|error| error.to_string())?;
    writeln!(stream, "{line}").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|error| error.to_string())?;
    serde_json::from_str::<Response>(&reply).map_err(|error| error.to_string())
}

fn unwrap_error(response: Response) -> Result<Response, String> {
    match response {
        Response::Error { message } => Err(message),
        other => Ok(other),
    }
}

fn fetch<T: DeserializeOwned>(
    request: Request,
    pick: impl FnOnce(Response) -> Option<serde_json::Value>,
) -> Result<T, String> {
    let response = unwrap_error(send(&request)?)?;
    let value = pick(response).ok_or_else(|| "unexpected response from service".to_string())?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

pub fn status() -> Result<ServiceStatus, String> {
    fetch(Request::Status, |response| match response {
        Response::Status { service } => Some(service),
        _ => None,
    })
}

pub fn zones() -> Result<Vec<ZoneInfo>, String> {
    fetch(Request::Zones, |response| match response {
        Response::Zones { zones } => Some(zones),
        _ => None,
    })
}

pub fn lighting() -> Result<LightingSettings, String> {
    fetch(Request::LightingGet, |response| match response {
        Response::Lighting { lighting } => Some(lighting),
        _ => None,
    })
}

pub fn sensors() -> Result<Snapshot, String> {
    fetch(Request::Sensors, |response| match response {
        Response::Sensors { snapshot } => Some(snapshot),
        _ => None,
    })
}

pub fn network() -> Result<NetworkSnapshot, String> {
    fetch(Request::Network, |response| match response {
        Response::Network { snapshot } => Some(snapshot),
        _ => None,
    })
}

pub fn profiles() -> Result<ProfileDocument, String> {
    fetch(Request::Profiles, |response| match response {
        Response::Profiles { document } => Some(document),
        _ => None,
    })
}

pub fn lcd() -> Result<LcdInfo, String> {
    fetch(Request::LcdGet, |response| match response {
        Response::Lcd { settings, themes } => Some(serde_json::json!({
            "settings": settings,
            "themes": themes,
        })),
        _ => None,
    })
}

pub fn cooling() -> Result<CoolingInfo, String> {
    fetch(Request::CoolingGet, |response| match response {
        Response::Cooling { curves, defaults } => Some(serde_json::json!({
            "curves": curves,
            "defaults": defaults,
        })),
        _ => None,
    })
}

pub fn settings() -> Result<DeviceSettings, String> {
    fetch(Request::SettingsGet, |response| match response {
        Response::Settings { settings } => Some(settings),
        _ => None,
    })
}

pub fn diagnostics() -> Result<String, String> {
    fetch(Request::Diagnostics, |response| match response {
        Response::Diagnostics { text } => Some(serde_json::Value::String(text)),
        _ => None,
    })
}


/// Renders the unsaved form values, so the preview follows the controls.
pub fn preview_draft(settings: &LcdSettings) -> Result<Vec<u8>, String> {
    let response = unwrap_error(send(&Request::LcdPreviewDraft {
        settings: settings.clone(),
    })?)?;
    match response {
        Response::Preview { png_base64 } => base64::engine::general_purpose::STANDARD
            .decode(png_base64)
            .map_err(|error| error.to_string()),
        _ => Err("unexpected response from service".to_string()),
    }
}

/// Sends an action and returns its confirmation message.
pub fn action(request: Request) -> Result<String, String> {
    match unwrap_error(send(&request)?)? {
        Response::Ok { message } => Ok(message),
        Response::Report { report } => {
            let report: ApplyReport =
                serde_json::from_value(report).map_err(|error| error.to_string())?;
            let failures: Vec<String> = report
                .results
                .iter()
                .filter(|result| !result.success)
                .map(|result| format!("{}: {}", result.part, result.message))
                .collect();
            if failures.is_empty() {
                Ok(format!(
                    "{} applied ({} part(s))",
                    report.profile_name,
                    report.results.len()
                ))
            } else {
                Err(failures.join("\n"))
            }
        }
        Response::Profile { profile } => {
            let profile: HardwareProfile =
                serde_json::from_value(profile).map_err(|error| error.to_string())?;
            Ok(format!("Profile '{}' stored.", profile.name))
        }
        Response::Media { files, stored } => {
            let _ = files;
            Ok(stored
                .map(|name| format!("Stored media file {name}."))
                .unwrap_or_else(|| "Media list updated.".to_string()))
        }
        other => Ok(format!("{other:?}")),
    }
}

pub fn apply_zone(zone_id: &str, lighting: ZoneLighting) -> Result<String, String> {
    action(Request::LightingApplyZone {
        zone_id: zone_id.to_string(),
        lighting,
    })
}

pub fn apply_all(lighting: LightingSettings) -> Result<String, String> {
    action(Request::LightingApplyAll { lighting })
}

pub fn set_lcd(settings: LcdSettings) -> Result<String, String> {
    action(Request::LcdSet { settings })
}

pub fn push_lcd() -> Result<String, String> {
    action(Request::LcdPush)
}

pub fn set_cooling(curves: BTreeMap<String, FanCurve>) -> Result<String, String> {
    action(Request::CoolingSet { curves })
}

pub fn restore_cooling() -> Result<String, String> {
    action(Request::CoolingRestore)
}

pub fn update_settings(settings: DeviceSettings) -> Result<String, String> {
    action(Request::SettingsUpdate { settings })
}

pub fn activate_profile(id: Uuid) -> Result<String, String> {
    action(Request::ProfileActivate { id })
}

pub fn duplicate_profile(id: Uuid, name: &str) -> Result<String, String> {
    action(Request::ProfileDuplicate {
        id,
        name: name.to_string(),
    })
}

pub fn rename_profile(id: Uuid, name: &str) -> Result<String, String> {
    action(Request::ProfileRename {
        id,
        name: name.to_string(),
    })
}

pub fn delete_profile(id: Uuid) -> Result<String, String> {
    action(Request::ProfileDelete { id })
}

pub fn reset_profiles() -> Result<String, String> {
    action(Request::ProfilesReset)
}

pub fn save_profile(profile: HardwareProfile) -> Result<String, String> {
    action(Request::ProfileUpsert { profile })
}

pub fn import_profile(json: &str) -> Result<String, String> {
    action(Request::ProfileImport {
        json: json.to_string(),
    })
}

pub fn export_profile(id: Uuid) -> Result<String, String> {
    match unwrap_error(send(&Request::ProfileExport { id })?)? {
        Response::Export { json } => Ok(json),
        _ => Err("unexpected response from service".to_string()),
    }
}

pub fn media_upload(file_name: &str, data: &[u8]) -> Result<MediaInfo, String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    let response = unwrap_error(send(&Request::MediaUpload {
        file_name: file_name.to_string(),
        base64: encoded,
    })?)?;
    match response {
        Response::Media { files, stored } => {
            let files = serde_json::from_value(files).map_err(|error| error.to_string())?;
            Ok(MediaInfo { files, stored })
        }
        _ => Err("unexpected response from service".to_string()),
    }
}

pub fn media_list() -> Result<MediaInfo, String> {
    let response = unwrap_error(send(&Request::MediaList)?)?;
    match response {
        Response::Media { files, stored } => {
            let files = serde_json::from_value(files).map_err(|error| error.to_string())?;
            Ok(MediaInfo { files, stored })
        }
        _ => Err("unexpected response from service".to_string()),
    }
}

pub fn rescan() -> Result<String, String> {
    action(Request::Rescan)
}

pub fn reapply() -> Result<String, String> {
    action(Request::Reapply)
}

pub fn reset_lighting() -> Result<String, String> {
    action(Request::LightingReset)
}
