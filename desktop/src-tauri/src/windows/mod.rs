//! Native Windows core (no Python). Compiled only for `cfg(windows)`.

pub mod app;
pub mod audio;
pub mod clipboard;
pub mod focus;
pub mod hotkey;
pub mod inject;
pub mod tray;

use app::Native;
use std::sync::Arc;
use tauri::{Manager, WindowEvent};

/// Tauri setup for Windows: rustls provider, state, tray, hotkey,
/// hide-to-tray on close.
pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // reqwest is built with `rustls-no-provider`; install ring once per process.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config_dir = app.path().app_config_dir()?;
    let native = Native::new(app.handle().clone(), config_dir);
    app.manage(native.clone());
    tray::setup(app.handle(), &native)?;
    hotkey::register_initial(&native);

    if let Some(window) = app.get_webview_window("main") {
        // Show the window only for first-run onboarding; afterwards the app
        // starts in the tray and never steals focus on its own.
        if !native.settings().onboarding_done {
            let _ = window.show();
        }
        let w = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = w.hide();
            }
        });
    }
    native.publish();
    if native.view().signed_in {
        native.spawn_refresh_account();
    }
    Ok(())
}

/// A second launch (e.g. from the Start menu) opens settings instead.
pub fn on_second_instance(app: &tauri::AppHandle) {
    tray::show_main(app);
}

pub fn native(app: &tauri::AppHandle) -> Arc<Native> {
    app.state::<Arc<Native>>().inner().clone()
}
