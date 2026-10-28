//! Tauri commands exposed to the webview. One whitelist for both platforms;
//! nothing here ever returns the service token. Platform-specific commands
//! (`setup_status`, `setup`) return an error where they do not apply.

use crate::native::{native, DeviceList, LastView, StatusView};
use crate::platform;
use serde_json::Value;
use tauri::AppHandle;

#[tauri::command]
pub fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}

fn require_delete_confirmation(confirm: &str) -> Result<(), String> {
    if confirm != "DELETE" {
        return Err("Type DELETE to confirm".into());
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn status(app: AppHandle) -> StatusView {
    native(&app).view()
}

#[tauri::command]
pub fn settings(app: AppHandle) -> Value {
    crate::storage::settings_view(&native(&app).settings())
}

#[tauri::command]
pub fn set_setting(app: AppHandle, key: String, value: String) -> Result<Value, String> {
    let s = native(&app).set_setting(&key, &value)?;
    Ok(crate::storage::settings_view(&s))
}

#[tauri::command]
pub async fn devices(app: AppHandle) -> Result<DeviceList, String> {
    let configured = native(&app).settings().mic_device;
    blocking(move || platform::audio::device_list(&configured)).await
}

/// Local microphone level test; audio is discarded immediately.
#[tauri::command]
pub async fn mic_test(app: AppHandle) -> Result<Value, String> {
    let n = native(&app);
    let (peak, rms) = blocking(move || n.mic_test(1500)).await??;
    Ok(serde_json::json!({ "peak": peak, "rms": rms }))
}

#[tauri::command]
pub fn last(app: AppHandle) -> LastView {
    native(&app).last()
}

#[tauri::command]
pub fn action(app: AppHandle, name: String) -> Result<(), String> {
    let n = native(&app);
    // Recording/stop work runs off the IPC thread so the UI stays responsive.
    match name.as_str() {
        "toggle" => {
            std::thread::spawn(move || n.toggle());
        }
        "cancel" => {
            std::thread::spawn(move || n.cancel());
        }
        "retry" => n.retry(),
        "discard" => n.discard(),
        "acknowledge" => n.acknowledge(),
        "clear-last" => n.clear_last(),
        "copy-last" => n.copy_last(false)?,
        "copy-raw" => n.copy_last(true)?,
        _ => return Err("Unsupported action".into()),
    }
    Ok(())
}

#[tauri::command]
pub async fn login(app: AppHandle, url: String) -> Result<Value, String> {
    let view = native(&app).login(url).await?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_login(app: AppHandle) {
    native(&app).cancel_login();
}

#[tauri::command]
pub fn logout(app: AppHandle) {
    native(&app).logout();
}

#[tauri::command]
pub async fn account(app: AppHandle, refresh: bool) -> Result<Option<crate::api::Me>, String> {
    let n = native(&app);
    if !refresh {
        return Ok(n.cached_account());
    }
    n.refresh_account().await.map(Some)
}

#[tauri::command]
pub async fn open_billing(app: AppHandle, portal: bool, plan: String) -> Result<(), String> {
    native(&app).open_billing(portal, plan).await
}

#[tauri::command]
pub async fn dictionary(app: AppHandle) -> Result<crate::api::Dictionary, String> {
    native(&app).dictionary().await
}

#[tauri::command]
pub async fn set_dictionary(
    app: AppHandle,
    terms: Vec<String>,
) -> Result<crate::api::Dictionary, String> {
    native(&app).set_dictionary(terms).await
}

#[tauri::command]
pub async fn delete_data(app: AppHandle, confirm: String) -> Result<(), String> {
    require_delete_confirmation(&confirm)?;
    native(&app).delete_data().await
}

/// Linux: GNOME shortcut and extension state.
#[tauri::command]
pub async fn setup_status() -> Result<Value, String> {
    #[cfg(target_os = "linux")]
    {
        blocking(|| serde_json::to_value(crate::linux::setup::status()).map_err(|e| e.to_string()))
            .await?
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("Unsupported on this platform".into())
    }
}

/// Linux: `action` = `hotkey` (bind the GNOME shortcut, optional `binding`
/// accelerator) or `extension` (enable the GNOME extension).
#[tauri::command]
pub async fn setup(
    app: AppHandle,
    action: String,
    binding: Option<String>,
) -> Result<Value, String> {
    #[cfg(target_os = "linux")]
    {
        let n = native(&app);
        blocking(move || {
            let r = crate::linux::setup::run_action(&action, binding.as_deref());
            if action == "hotkey" {
                crate::linux::refresh_hotkey_label(&n);
            }
            r
        })
        .await?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, action, binding);
        Err("Unsupported on this platform".into())
    }
}

/// Environment checks (Linux; Windows reports only the basics). No window
/// titles, no transcript text, no token.
#[tauri::command]
pub async fn diagnostics(app: AppHandle) -> Result<Value, String> {
    let n = native(&app);
    let s = n.settings();
    #[cfg(target_os = "linux")]
    {
        let dir = n.config_dir();
        blocking(move || {
            serde_json::to_value(crate::linux::diagnostics::collect(
                &dir,
                &s.mic_device,
                &s.service_url,
            ))
            .map_err(|e| e.to_string())
        })
        .await?
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(serde_json::json!({
            "session": "windows",
            "service_url": s.service_url,
            "token_storage": crate::storage::token_backend(&n.config_dir()),
            "checks": [],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_requires_exact_confirmation() {
        assert!(require_delete_confirmation("DELETE").is_ok());
        assert!(require_delete_confirmation("delete").is_err());
        assert!(require_delete_confirmation("").is_err());
    }
}
