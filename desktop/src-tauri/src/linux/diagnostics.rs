//! Environment checks for the UI (replaces `scribiva doctor`). Reports tool
//! presence and states only: no window titles, no transcript text, no token.

use super::{audio, autostart, clipboard, focus, kwin, proc, setup, shell};
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
    let kde = kwin::is_kde();
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
            None if kde => check(
                "eitype",
                "warn",
                "eitype not found; on KDE text is only copied without it",
            ),
            None => check(
                "eitype",
                "warn",
                "eitype not found; used only when the GNOME extension is unavailable",
            ),
        });
    }
    if kde {
        checks.extend(kwin_checks(&setup::kwin_status(&setup::System), x11));
    } else {
        checks.push(gnome_extension_check(x11));
    }
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

fn gnome_extension_check(x11: bool) -> Check {
    let ext = setup::extension_status(&setup::System);
    let state = if ext.active {
        "ok"
    } else if x11 {
        "warn"
    } else {
        "fail"
    };
    check(
        "gnome-extension",
        state,
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
    )
}

/// KDE Plasma: the KWin focus script (installed / enabled / reporting).
/// Without it on Wayland the focus is unknown and text is only copied.
pub fn kwin_checks(k: &setup::KwinStatus, x11: bool) -> Vec<Check> {
    let missing = if x11 { "warn" } else { "fail" };
    vec![
        if k.kwin_script_installed {
            check("kwin-script", "ok", "installed")
        } else {
            check(
                "kwin-script",
                missing,
                "Scribiva KWin script not installed; reinstall the scribiva package",
            )
        },
        if k.kwin_script_enabled {
            check("kwin-script-enabled", "ok", "enabled in KWin")
        } else {
            check(
                "kwin-script-enabled",
                missing,
                "not enabled; enable \"Scribiva focus\" in System Settings > KWin Scripts",
            )
        },
        if k.kwin_focus_active {
            check("kwin-focus", "ok", "receiving focus updates from KWin")
        } else if x11 {
            check(
                "kwin-focus",
                "warn",
                "no focus updates (X11 uses xprop instead)",
            )
        } else {
            check(
                "kwin-focus",
                "fail",
                "no focus updates from KWin yet; dictated text is only copied",
            )
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kwin_check_states() {
        let all = setup::KwinStatus {
            kde: true,
            kwin_script_installed: true,
            kwin_script_enabled: true,
            kwin_focus_active: true,
        };
        let c = kwin_checks(&all, false);
        assert_eq!(
            c.iter().map(|c| (c.id, c.status)).collect::<Vec<_>>(),
            [
                ("kwin-script", "ok"),
                ("kwin-script-enabled", "ok"),
                ("kwin-focus", "ok")
            ]
        );
        let none = setup::KwinStatus {
            kde: true,
            ..Default::default()
        };
        assert!(kwin_checks(&none, false).iter().all(|c| c.status == "fail"));
        assert!(kwin_checks(&none, true).iter().all(|c| c.status == "warn"));
    }
}
