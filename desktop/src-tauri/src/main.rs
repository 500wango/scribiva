//! Scribiva desktop app: one native Rust core for Windows and Linux.
//!
//! - `native`: the orchestrator (state machine driver, upload, login).
//! - `core`, `api`, `audio_dsp`, `focus_policy`, `storage`: platform
//!   independent logic, unit-tested on every OS.
//! - `platform`: the OS adapters behind a small boundary (`audio`, `focus`,
//!   `clipboard`, `deliver`, `hotkey`, `indicator`, `setting_changed`),
//!   implemented in `windows/` (WASAPI, Win32, RegisterHotKey) and `linux/`
//!   (pw-record, GNOME extension / X11, wl-clipboard + eitype, GNOME shortcut).
//! - `tray`, `commands`: shared Tauri surface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
// Resampling/downmix serve cpal (Windows); pw-record already delivers 16 kHz mono.
#[cfg_attr(not(windows), allow(dead_code))]
mod audio_dsp;
mod commands;
mod core;
mod focus_policy;
#[cfg(target_os = "linux")]
mod linux;
mod native;
mod storage;
mod tray;
#[cfg(windows)]
mod windows;

/// OS adapters used by `native` and `commands`.
mod platform {
    #[cfg(target_os = "linux")]
    pub use crate::linux::{
        audio, clipboard, deliver, focus, hotkey, indicator, on_second_instance, setting_changed,
        setup,
    };
    #[cfg(windows)]
    pub use crate::windows::{
        audio, clipboard, deliver, focus, hotkey, indicator, on_second_instance, setting_changed,
        setup,
    };
}

#[cfg(any(windows, target_os = "linux"))]
fn main() {
    let builder = tauri::Builder::default()
        // Must be first so a second launch is redirected before anything else
        // starts. On Linux this is how the GNOME shortcut (`scribiva --toggle`)
        // reaches the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            platform::on_second_instance(app, &argv);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    #[cfg(windows)]
    let builder = builder.plugin(windows::hotkey::plugin());
    builder
        .invoke_handler(tauri::generate_handler![
            commands::platform,
            commands::status,
            commands::settings,
            commands::set_setting,
            commands::devices,
            commands::mic_test,
            commands::last,
            commands::action,
            commands::login,
            commands::cancel_login,
            commands::logout,
            commands::account,
            commands::open_billing,
            commands::dictionary,
            commands::set_dictionary,
            commands::delete_data,
            commands::setup_status,
            commands::setup,
            commands::diagnostics,
        ])
        .setup(|app| platform::setup(app))
        .run(tauri::generate_context!())
        .expect("Failed to run Scribiva");
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("Scribiva desktop supports Windows and Linux only");
}
