//! Global hotkey via tauri-plugin-global-shortcut (RegisterHotKey). Toggle
//! mode only (FR-001 P0). Esc is registered only while recording so the user
//! can cancel without focusing our window; it is released right after.

use crate::core::Phase;
use crate::native::Native;
use std::str::FromStr;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const CANCEL_KEY: &str = "Escape";

fn parse(binding: &str) -> Result<Shortcut, String> {
    Shortcut::from_str(binding).map_err(|e| format!("HOTKEY_INVALID: {e}"))
}

fn conflict(binding: &str, e: impl std::fmt::Display) -> String {
    format!("HOTKEY_CONFLICT: {binding} is already used by another application or Windows ({e})")
}

/// The plugin-level handler for every shortcut we register.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let Some(native) = app.try_state::<Arc<Native>>() else {
                return;
            };
            let native = native.inner().clone();
            let cancel = parse(CANCEL_KEY).is_ok_and(|c| c.id() == shortcut.id());
            // Hand off: handlers run on the event loop thread and must not block.
            std::thread::spawn(move || {
                if cancel {
                    native.cancel();
                } else {
                    native.toggle();
                }
            });
        })
        .build()
}

/// Register the configured toggle hotkey at startup.
pub fn register_initial(native: &Arc<Native>) {
    let binding = native.settings().hotkey;
    let result = parse(&binding).and_then(|s| {
        native
            .app
            .global_shortcut()
            .register(s)
            .map_err(|e| conflict(&binding, e))
    });
    native.set_hotkey_error(result.err());
}

/// Swap bindings. On failure the old binding is restored and an error returned.
pub fn rebind(native: &Arc<Native>, old: &str, new: &str) -> Result<(), String> {
    let gs = native.app.global_shortcut();
    let new_shortcut = parse(new)?;
    if let Ok(old_shortcut) = parse(old) {
        if gs.is_registered(old_shortcut) {
            let _ = gs.unregister(old_shortcut);
        }
    }
    match gs.register(new_shortcut) {
        Ok(()) => {
            native.set_hotkey_error(None);
            Ok(())
        }
        Err(e) => {
            let err = conflict(new, e);
            let restored = parse(old).and_then(|s| gs.register(s).map_err(|e| conflict(old, e)));
            native.set_hotkey_error(restored.err());
            Err(err)
        }
    }
}

/// Esc is registered only while recording.
pub fn on_phase(app: &AppHandle, prev: Option<Phase>, now: Phase) {
    if now == Phase::Recording {
        register_cancel(app);
    } else if prev == Some(Phase::Recording) {
        unregister_cancel(app);
    }
}

/// Register Esc for "cancel recording". Failure is harmless (another app owns
/// Esc globally); the tray and UI still offer cancel.
pub fn register_cancel(app: &AppHandle) {
    if let Ok(s) = parse(CANCEL_KEY) {
        let gs = app.global_shortcut();
        if !gs.is_registered(s) {
            let _ = gs.register(s);
        }
    }
}

pub fn unregister_cancel(app: &AppHandle) {
    if let Ok(s) = parse(CANCEL_KEY) {
        let gs = app.global_shortcut();
        if gs.is_registered(s) {
            let _ = gs.unregister(s);
        }
    }
}
