//! Windows platform adapters for the shared orchestrator (`crate::native`):
//! WASAPI capture (cpal), Win32 focus snapshot, clipboard + single Ctrl+V,
//! RegisterHotKey. Compiled only for `cfg(windows)`.

pub mod audio;
pub mod clipboard;
pub mod deliver;
pub mod focus;
pub mod hotkey;
pub mod inject;

use crate::core::Phase;
use crate::native::{self, Native};
use crate::storage::Settings;
use std::path::Path;
use tauri::Manager;

/// The tray icon is the indicator on Windows.
pub fn indicator(_phase: Phase) {}

pub fn setting_changed(app: &tauri::AppHandle, _config_dir: &Path, key: &str, settings: &Settings) {
    if key == "start_at_login" || key == "onboarding_done" {
        sync_autostart(app, settings);
    }
}

/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` entry running
/// `scribiva.exe --hidden`, via tauri-plugin-autostart. Like Linux, only
/// written once onboarding is done.
pub fn sync_autostart(app: &tauri::AppHandle, settings: &Settings) {
    let Some(al) = app.try_state::<tauri_plugin_autostart::AutoLaunchManager>() else {
        return;
    };
    let want = settings.onboarding_done && settings.start_at_login;
    match (want, al.is_enabled().unwrap_or(false)) {
        (true, false) => {
            let _ = al.enable();
        }
        (false, true) => {
            let _ = al.disable();
        }
        _ => {}
    }
}

/// Tauri setup for Windows: rustls provider, state, tray, hotkey,
/// hide-to-tray on close.
pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // reqwest is built with `rustls-no-provider`; install ring once per process.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config_dir = app.path().app_config_dir()?;
    let native = Native::new(app.handle().clone(), config_dir);
    native.load_token();
    app.manage(native.clone());
    crate::tray::setup(app.handle(), &native)?;
    hotkey::register_initial(&native);
    sync_autostart(app.handle(), &native.settings());
    crate::hud::create(app.handle());
    crate::updater::start(&native);

    if let Some(window) = app.get_webview_window("main") {
        // Show the window only for first-run onboarding; afterwards the app
        // starts in the tray and never steals focus on its own.
        if !native.settings().onboarding_done {
            let _ = window.show();
        }
        native::hide_on_close(&window);
    }
    native.publish();
    if native.view().signed_in {
        native.spawn_refresh_account();
    }
    Ok(())
}

/// A second launch (e.g. from the Start menu) opens settings; `--toggle` /
/// `--cancel` act on the running instance instead.
pub fn on_second_instance(app: &tauri::AppHandle, argv: &[String]) {
    native::run_launch_action(app, native::launch_action(argv));
}
