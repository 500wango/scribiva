//! Environment checks for the UI (replaces `scribiva doctor`). Reports tool
//! presence and states only: no window titles, no transcript text, no token.

use super::{audio, autostart, clipboard, focus, proc, setup, shell};
use crate::storage;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    /// `ok`, `warn` or `fail`.
    pub status: &'static str,
    pub detail: String,
}

fn check(id: &'static str, status: &'static str, detail: impl Into<String>) -> Check {
    Check {
        id,
        status,
        detail: detail.into(),
    }
}

fn tool(id: &'static str, program: &str, missing: &'static str, hint: &str) -> Check {
    match proc::which(program) {
        Some(p) => check(id, "ok", p.to_string_lossy()),
        None => check(id, missing, format!("{program} not found ({hint})")),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostics {
    pub session: String,
    pub desktop: String,
    pub mic_node: Option<String>,
    pub service_url: String,
    pub token_storage: &'static str,
    pub checks: Vec<Check>,
}

pub fn collect(config_dir: &Path, mic_device: &str, service_url: &str) -> Diagnostics {
    let x11 = focus::is_x11();
    let mut checks = vec![
        tool("pw-record", "pw-record", "fail", "install pipewire-bin"),
        tool("pw-dump", "pw-dump", "fail", "install pipewire-bin"),
    ];
    let devices = audio::device_list(mic_device);
    checks.push(match &devices.error {
        Some(e) => check("capture-devices", "fail", e.clone()),
        None if devices.devices.is_empty() => {
            check("capture-devices", "fail", "no audio source found")
        }
        None => check(
            "capture-devices",
            "ok",
            format!(
                "{} source(s); active: {}",
                devices.devices.len(),
                devices.active.as_deref().unwrap_or("system default")
            ),
        ),
    });
    if x11 {
        checks.push(tool(
            "xprop",
            "xprop",
            "fail",
            "install x11-utils; needed to see the focused window",
        ));
        checks.push(tool("xclip", "xclip", "fail", "install xclip"));
        checks.push(tool(
            "xdotool",
            "xdotool",
            "warn",
            "install xdotool; without it text is only copied",
        ));
    } else {
        checks.push(tool("wl-copy", "wl-copy", "fail", "install wl-clipboard"));
        checks.push(tool(
            "wl-paste",
            "wl-paste",
            "warn",
            "install wl-clipboard; needed to restore the clipboard",
        ));
        checks.push(match clipboard::eitype_path() {
            Some(p) => check("eitype", "ok", p.to_string_lossy()),
            None => check(
                "eitype",
                "warn",
                "eitype not found; used only when the GNOME extension is unavailable",
            ),
        });
    }
    let ext = setup::extension_status(&setup::System);
    let ext_state = if ext.active {
        "ok"
    } else if x11 {
        "warn"
    } else {
        "fail"
    };
    checks.push(check(
        "gnome-extension",
        ext_state,
        format!(
            "installed: {}, enabled: {}, active: {}, version: {}{}",
            ext.installed,
            ext.enabled,
            ext.active,
            ext.version,
            if ext.enabled && !ext.active {
                " (log out and back in)"
            } else {
                ""
            }
        ),
    ));
    let hotkey = setup::hotkey_status(&setup::System, &setup::toggle_command());
    checks.push(if hotkey.installed {
        check("shortcut", "ok", hotkey.label.unwrap_or_default())
    } else if hotkey.broken {
        check(
            "shortcut",
            "warn",
            "a Scribiva shortcut runs an outdated command; bind it again",
        )
    } else {
        check("shortcut", "warn", "no GNOME shortcut set up")
    });
    let backend = storage::token_backend(config_dir);
    let secret_service = shell::name_has_owner(shell::SECRETS_BUS_NAME)
        || shell::name_activatable(shell::SECRETS_BUS_NAME);
    checks.push(if backend == "file" {
        check(
            "token-storage",
            "warn",
            "no Secret Service: the sign-in token is in a private file (0600) in the config directory",
        )
    } else if secret_service {
        check("token-storage", "ok", "Secret Service")
    } else {
        check(
            "token-storage",
            "warn",
            "no Secret Service found; signing in stores the token in a private file (0600)",
        )
    });
    checks.push(check(
        "autostart",
        if autostart::installed() { "ok" } else { "warn" },
        if autostart::installed() {
            "starts at login"
        } else {
            "not starting at login"
        },
    ));
    Diagnostics {
        session: setup::session_type(),
        desktop: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        mic_node: devices.active,
        service_url: service_url.to_string(),
        token_storage: backend,
        checks,
    }
}
