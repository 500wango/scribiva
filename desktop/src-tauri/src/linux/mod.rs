//! Linux platform adapters for the shared orchestrator (`crate::native`):
//! PipeWire capture, focus from the GNOME extension or X11, delivery through
//! the extension or clipboard + one paste chord, GNOME shortcut setup,
//! autostart and Python-client migration. Compiled only on Linux.

pub mod audio;
pub mod autostart;
pub mod clipboard;
pub mod deliver;
pub mod diagnostics;
pub mod focus;
pub mod migrate;
pub mod proc;
pub mod setup;
pub mod shell;

use crate::core::Phase;
use crate::native::{self, LaunchAction, Native};
use crate::storage::{self, Settings};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use tauri::Manager;

/// GNOME has no global-shortcut API for apps: the custom shortcut runs
/// `scribiva --toggle` and single-instance forwards it. Nothing to register.
pub mod hotkey {
    use crate::core::Phase;
    use tauri::AppHandle;

    /// Global Esc-to-cancel is not possible on GNOME; cancel via the tray,
    /// the window or `scribiva --cancel`.
    pub fn on_phase(_app: &AppHandle, _prev: Option<Phase>, _now: Phase) {}
}

fn indicator_phase(phase: Phase) -> &'static str {
    match phase {
        Phase::Recording => "recording",
        Phase::Processing => "processing",
        Phase::Error | Phase::Quota => "error",
        _ => "idle",
    }
}

static INDICATOR: Mutex<Option<Sender<&'static str>>> = Mutex::new(None);

/// Best-effort top-bar indicator through the GNOME extension. One worker
/// thread keeps the calls ordered and never blocks the caller.
pub fn indicator(phase: Phase) {
    let state = indicator_phase(phase);
    let mut g = INDICATOR.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tx) = g.as_ref() {
        if tx.send(state).is_ok() {
            return;
        }
    }
    let (tx, rx) = mpsc::channel::<&'static str>();
    let _ = tx.send(state);
    let spawned = std::thread::Builder::new()
        .name("scribiva-indicator".into())
        .spawn(move || {
            while let Ok(mut s) = rx.recv() {
                // Only the latest state matters.
                while let Ok(next) = rx.try_recv() {
                    s = next;
                }
                // Fails fast (ServiceUnknown) when the extension is not loaded.
                shell::set_state(s);
            }
        });
    if spawned.is_ok() {
        *g = Some(tx);
    }
}

pub fn setting_changed(_config_dir: &Path, key: &str, settings: &Settings) {
    if key == "start_at_login" || key == "onboarding_done" {
        autostart::sync(settings);
    }
}

/// Re-read the GNOME shortcut and show it in the UI and tray.
pub fn refresh_hotkey_label(native: &Native) {
    let status = setup::hotkey_status(&setup::System, &setup::toggle_command());
    native.set_hotkey_label(Some(if status.installed || status.broken {
        status.label.unwrap_or_default()
    } else {
        String::new()
    }));
    native.publish();
}

/// Tauri setup for Linux: migration, state, tray, autostart, shortcut repair,
/// hide-to-tray on close, and the first launch's `--toggle/--cancel/--hidden`.
pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // reqwest is built with `rustls-no-provider`; install ring once per process.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config_dir = app.path().app_config_dir()?;
    let legacy_token = migrate_settings(&config_dir);
    let native = Native::new(app.handle().clone(), config_dir.clone());
    // The GNOME shortcut is read in the background; never show the Windows default.
    native.set_hotkey_label(Some(String::new()));
    app.manage(native.clone());
    crate::tray::setup(app.handle(), &native)?;
    autostart::sync(&native.settings());

    let action = native::launch_action(&std::env::args().collect::<Vec<_>>());
    if let Some(window) = app.get_webview_window("main") {
        // A plain launch opens the window; the shortcut (`--toggle`) and
        // autostart (`--hidden`) never show it, so focus stays in the target.
        if action == LaunchAction::Show {
            let _ = window.show();
        }
        native::hide_on_close(&window);
    }
    native.publish();

    // The Secret Service may block (unlock prompt): load the token off the
    // main thread, then run the launch action that may need it.
    let n = native.clone();
    let handle = app.handle().clone();
    std::thread::spawn(move || {
        if let Some(token) = legacy_token {
            if storage::load_token(&config_dir).is_none() {
                let _ = storage::store_token(&config_dir, &token);
            }
        }
        let signed_in = n.load_token();
        n.publish();
        if action != LaunchAction::Show {
            native::run_launch_action(&handle, action);
        }
        if signed_in {
            n.spawn_refresh_account();
        }
    });

    // Point a Scribiva shortcut from an older install at this binary; no-op
    // when the user never set one up.
    let n = native.clone();
    std::thread::spawn(move || {
        let _ = setup::repair_hotkey(&setup::System, &setup::toggle_command());
        refresh_hotkey_label(&n);
    });
    Ok(())
}

/// First Linux run: import Python client settings. Returns its token, if any,
/// for the background thread to put in the secret store.
fn migrate_settings(config_dir: &Path) -> Option<String> {
    let legacy = migrate::legacy_dir()?;
    let current = storage::load_settings(config_dir);
    let plan = migrate::plan(config_dir, &legacy, &current)?;
    if let Some(s) = plan.settings.as_ref() {
        let _ = storage::save_settings(config_dir, s);
    }
    migrate::mark_done(config_dir);
    plan.token
}

/// Second launches (GNOME shortcut, menu entry) arrive here via D-Bus.
pub fn on_second_instance(app: &tauri::AppHandle, argv: &[String]) {
    native::run_launch_action(app, native::launch_action(argv));
}
