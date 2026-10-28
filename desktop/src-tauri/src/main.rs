//! Scribiva desktop shell.
//!
//! - Linux: whitelisted calls into the installed Python CLI (`linux.rs`).
//! - Windows: native Rust core (`windows/`): hotkey, WASAPI capture, focus
//!   guard, clipboard + single Ctrl+V, API client, Credential Manager.
//!
//! Platform-independent logic (state machine, DSP, focus policy, API parsing,
//! settings validation) lives in the top-level modules and is unit-tested on
//! every OS. On Linux those modules are only partly used, hence the
//! `dead_code` allowance there.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg_attr(not(windows), allow(dead_code))]
mod api;
#[cfg_attr(not(windows), allow(dead_code))]
mod audio_dsp;
mod commands;
#[cfg_attr(not(windows), allow(dead_code))]
mod core;
#[cfg_attr(not(windows), allow(dead_code))]
mod focus_policy;
#[cfg(target_os = "linux")]
mod linux;
#[cfg_attr(not(windows), allow(dead_code))]
mod storage;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
fn main() {
    use commands::linux_cmds::*;
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::platform,
            get_status,
            get_settings,
            get_devices,
            get_last,
            clear_last,
            set_setting,
            action,
            linux_service_status,
            linux_dictionary,
            linux_set_dictionary,
            linux_login,
            linux_logout,
            linux_open_billing,
            linux_delete_data,
            linux_setup_status,
            linux_setup,
        ])
        .setup(|app| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Failed to run Scribiva");
}

#[cfg(windows)]
fn main() {
    use commands::win_cmds::*;
    tauri::Builder::default()
        // Must be first so a second launch is redirected before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            windows::on_second_instance(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(windows::hotkey::plugin())
        .invoke_handler(tauri::generate_handler![
            commands::platform,
            win_status,
            win_settings,
            win_set_setting,
            win_devices,
            win_mic_test,
            win_last,
            win_action,
            win_login,
            win_cancel_login,
            win_logout,
            win_account,
            win_open_billing,
            win_dictionary,
            win_set_dictionary,
            win_delete_data,
        ])
        .setup(|app| windows::setup(app))
        .run(tauri::generate_context!())
        .expect("Failed to run Scribiva");
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("Scribiva desktop supports Windows and Linux only");
}
