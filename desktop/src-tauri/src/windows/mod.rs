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

pub fn setting_changed(_config_dir: &Path, _key: &str, _settings: &Settings) {}

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
