//! Local IPC contract between the service and the UI.
//!
//! Newline-delimited JSON over a Unix domain socket. The service validates
//! every request; the UI never opens a device node itself.
//!
//! The wire format is intentionally small and additive: new requests are new
//! enum variants. Unknown fields are ignored by serde, and unknown commands
//! produce an `Error` response rather than closing the connection.

#![allow(clippy::items_after_test_module)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::effect::ZoneLighting;
use crate::fan::FanCurve;
use crate::profile::{HardwareProfile, LcdSettings, LightingSettings};
use crate::settings::DeviceSettings;

pub const APP_DIR: &str = "mylianpc";
pub const SOCKET_NAME: &str = "mylianpc.sock";
pub const SOCKET_ENV: &str = "MYLIANPC_SOCKET";
/// Where the installed system service listens.
pub const SYSTEM_SOCKET: &str = "/run/mylianpc/mylianpc.sock";

/// Requests accepted by the service.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Request {
    /// Liveness/version check.
    Ping,
    /// Read-only hardware inventory.
    Inventory,
    /// Fusion2 identification query (no lighting change).
    Fusion2,
    /// Shared hardware profiles (read-only snapshot).
    Profiles,
    /// Mapped sensor readings (read-only).
    Sensors,
    /// Corsair DRAM RGB detection over SMBus (read-only).
    Ram,
    /// Full device/status report.
    Status,
    /// Every lighting zone currently advertised by the adapters.
    Zones,
    /// Lighting settings of the active profile.
    LightingGet,
    /// Applies one zone now and records it in the active profile.
    LightingApplyZone { zone_id: String, lighting: ZoneLighting },
    /// Applies a complete lighting block now and records it in the active profile.
    LightingApplyAll { lighting: LightingSettings },
    /// Turns every zone off and clears the active profile's lighting overrides.
    LightingReset,
    /// Re-applies the active profile.
    Reapply,

    /// Returns one profile.
    ProfileGet { id: Uuid },
    /// Creates or updates a profile.
    ProfileUpsert { profile: HardwareProfile },
    /// Deletes a profile.
    ProfileDelete { id: Uuid },
    /// Duplicates a profile under a new name.
    ProfileDuplicate { id: Uuid, name: String },
    /// Renames a profile.
    ProfileRename { id: Uuid, name: String },
    /// Activates a profile and applies it.
    ProfileActivate { id: Uuid },
    /// Imports an exported profile.
    ProfileImport { json: String },
    /// Exports a profile as JSON.
    ProfileExport { id: Uuid },
    /// Restores the shipped example profiles.
    ProfilesReset,

    /// Cooling curves of the active profile and the built-in defaults.
    CoolingGet,
    /// Replaces the active profile's cooling curves.
    CoolingSet { curves: BTreeMap<String, FanCurve> },
    /// Restores the default Balanced curves and releases fan control.
    CoolingRestore,

    /// LCD settings of the active profile and the theme catalogue.
    LcdGet,
    /// Stores new LCD settings in the active profile and schedules a refresh.
    LcdSet { settings: LcdSettings },
    /// Pushes the current LCD layout to the panel immediately.
    LcdPush,
    /// Renders the current LCD layout as a PNG preview.
    LcdPreview,
    /// Renders an unsaved draft of the LCD layout as a PNG preview.
    LcdPreviewDraft { settings: LcdSettings },
    /// Validates and stores an image or animation for the LCD.
    MediaUpload { file_name: String, base64: String },
    /// Lists stored LCD media.
    MediaList,

    /// Machine-wide device settings.
    SettingsGet,
    /// Replaces the machine-wide device settings.
    SettingsUpdate { settings: DeviceSettings },
    /// Connection information and traffic totals.
    Network,
    /// Reconnects every device and re-applies the active profile.
    Rescan,
    /// Human-readable diagnostics report.
    Diagnostics,
}

/// Responses. `*` payloads stay JSON values so this crate does not depend on
/// the hardware crate; the UI deserializes them into the concrete types.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Response {
    Pong { version: String },
    Inventory { inventory: serde_json::Value },
    Fusion2 { info: serde_json::Value },
    Profiles { document: serde_json::Value },
    Sensors { snapshot: serde_json::Value },
    Ram { buses: serde_json::Value },
    Status { service: serde_json::Value },
    Zones { zones: serde_json::Value },
    Lighting { lighting: serde_json::Value },
    Report { report: serde_json::Value },
    Profile { profile: serde_json::Value },
    Cooling { curves: serde_json::Value, defaults: serde_json::Value },
    Lcd { settings: serde_json::Value, themes: serde_json::Value },
    Preview { png_base64: String },
    Media { files: serde_json::Value, stored: Option<String> },
    Settings { settings: serde_json::Value },
    Network { snapshot: serde_json::Value },
    Diagnostics { text: String },
    Export { json: String },
    Ok { message: String },
    Error { message: String },
}

impl Response {
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

/// Socket path used by both sides.
///
/// Resolution order:
/// 1. `MYLIANPC_SOCKET` (explicit development override; the systemd unit also
///    sets it);
/// 2. the installed service socket `/run/mylianpc/mylianpc.sock`, when it is
///    present — the UI does not inherit the unit's environment, so this path
///    must be discovered without it;
/// 3. `$XDG_RUNTIME_DIR/mylianpc/mylianpc.sock` (per-user development);
/// 4. `/tmp/mylianpc.sock`.
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var(SOCKET_ENV) {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    let system = PathBuf::from(SYSTEM_SOCKET);
    if system.exists() {
        return system;
    }
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.trim().is_empty() {
            return PathBuf::from(runtime).join(APP_DIR).join(SOCKET_NAME);
        }
    }
    PathBuf::from("/tmp").join(SOCKET_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_wire_format_is_tagged() {
        assert_eq!(
            serde_json::to_string(&Request::Inventory).unwrap(),
            r#"{"command":"inventory"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"command":"ping"}"#).unwrap().command_name(),
            "ping"
        );
    }

    #[test]
    fn complex_requests_round_trip() {
        let request = Request::LightingApplyZone {
            zone_id: "fusion2:argb1".to_string(),
            lighting: ZoneLighting {
                effect: crate::effect::LightingEffect::Static,
                colors: vec![crate::color::RgbColor::new(1, 2, 3)],
                brightness: 60,
                speed: 40,
            },
        };
        let json = serde_json::to_string(&request).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        match back {
            Request::LightingApplyZone { zone_id, lighting } => {
                assert_eq!(zone_id, "fusion2:argb1");
                assert_eq!(lighting.colors[0], crate::color::RgbColor::new(1, 2, 3));
            }
            other => panic!("wrong request: {other:?}"),
        }
    }

    #[test]
    fn lcd_draft_request_round_trips() {
        let settings = crate::profile::LcdSettings {
            theme_id: "neon".to_string(),
            brightness: 42,
            ..Default::default()
        };
        let json = serde_json::to_string(&Request::LcdPreviewDraft {
            settings: settings.clone(),
        })
        .unwrap();
        match serde_json::from_str::<Request>(&json).unwrap() {
            Request::LcdPreviewDraft { settings: back } => {
                assert_eq!(back.theme_id, "neon");
                assert_eq!(back.brightness, 42);
            }
            other => panic!("wrong request: {other:?}"),
        }
    }

    #[test]
    fn response_round_trips() {
        let response = Response::error("no device");
        let json = serde_json::to_string(&response).unwrap();
        match serde_json::from_str::<Response>(&json).unwrap() {
            Response::Error { message } => assert_eq!(message, "no device"),
            other => panic!("wrong response: {other:?}"),
        }
    }

    #[test]
    fn socket_path_honours_the_constants() {
        assert_eq!(super::APP_DIR, "mylianpc");
        assert_eq!(super::SOCKET_NAME, "mylianpc.sock");
    }
}

impl Request {
    /// Small helper used by tests and logging.
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Inventory => "inventory",
            Self::Fusion2 => "fusion2",
            Self::Profiles => "profiles",
            Self::Sensors => "sensors",
            Self::Ram => "ram",
            Self::Status => "status",
            Self::Zones => "zones",
            Self::LightingGet => "lighting-get",
            Self::LightingApplyZone { .. } => "lighting-apply-zone",
            Self::LightingApplyAll { .. } => "lighting-apply-all",
            Self::LightingReset => "lighting-reset",
            Self::Reapply => "reapply",
            Self::ProfileGet { .. } => "profile-get",
            Self::ProfileUpsert { .. } => "profile-upsert",
            Self::ProfileDelete { .. } => "profile-delete",
            Self::ProfileDuplicate { .. } => "profile-duplicate",
            Self::ProfileRename { .. } => "profile-rename",
            Self::ProfileActivate { .. } => "profile-activate",
            Self::ProfileImport { .. } => "profile-import",
            Self::ProfileExport { .. } => "profile-export",
            Self::ProfilesReset => "profiles-reset",
            Self::CoolingGet => "cooling-get",
            Self::CoolingSet { .. } => "cooling-set",
            Self::CoolingRestore => "cooling-restore",
            Self::LcdGet => "lcd-get",
            Self::LcdSet { .. } => "lcd-set",
            Self::LcdPush => "lcd-push",
            Self::LcdPreview => "lcd-preview",
            Self::LcdPreviewDraft { .. } => "lcd-preview-draft",
            Self::MediaUpload { .. } => "media-upload",
            Self::MediaList => "media-list",
            Self::SettingsGet => "settings-get",
            Self::SettingsUpdate { .. } => "settings-update",
            Self::Network => "network",
            Self::Rescan => "rescan",
            Self::Diagnostics => "diagnostics",
        }
    }
}
