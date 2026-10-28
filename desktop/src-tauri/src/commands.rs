//! Tauri commands exposed to the webview. Each OS gets its own whitelist;
//! nothing here ever returns the service token.

#[cfg(any(target_os = "linux", windows))]
use serde_json::Value;

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

#[cfg(target_os = "linux")]
fn open_trusted_billing(app: &tauri::AppHandle, url: String) -> Result<(), String> {
    if !crate::api::is_trusted_billing_url(&url) {
        return Err("UNTRUSTED_BILLING_URL: refusing to open a non-Stripe page".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ Linux

#[cfg(target_os = "linux")]
pub mod linux_cmds {
    use super::*;
    use crate::linux;

    #[tauri::command]
    pub fn get_status() -> Result<Value, String> {
        linux::status()
    }
    #[tauri::command]
    pub fn get_settings() -> Result<Value, String> {
        linux::settings()
    }
    #[tauri::command]
    pub fn get_devices() -> Result<Value, String> {
        linux::devices()
    }
    #[tauri::command]
    pub fn get_last() -> Result<Value, String> {
        linux::last()
    }
    #[tauri::command]
    pub fn clear_last() -> Result<(), String> {
        linux::action("clear-last").map(|_| ())
    }
    #[tauri::command]
    pub fn set_setting(key: String, value: String) -> Result<(), String> {
        linux::set_setting(&key, &value)
    }
    #[tauri::command]
    pub fn action(name: String) -> Result<String, String> {
        linux::action(&name)
    }
    #[tauri::command]
    pub fn linux_service_status() -> Result<Value, String> {
        linux::service_status()
    }
    #[tauri::command]
    pub fn linux_dictionary() -> Result<Value, String> {
        linux::dictionary()
    }
    #[tauri::command]
    pub fn linux_set_dictionary(terms: Vec<String>) -> Result<Value, String> {
        linux::set_dictionary(&terms)
    }
    #[tauri::command]
    pub fn linux_login(app: tauri::AppHandle, url: String) -> Result<(), String> {
        linux::login(app, &url)
    }
    #[tauri::command]
    pub fn linux_logout() -> Result<String, String> {
        linux::logout()
    }
    #[tauri::command]
    pub fn linux_open_billing(
        app: tauri::AppHandle,
        portal: bool,
        plan: String,
    ) -> Result<(), String> {
        let url = linux::billing_url(portal, &plan)?;
        open_trusted_billing(&app, url)
    }
    #[tauri::command]
    pub fn linux_delete_data(confirm: String) -> Result<String, String> {
        require_delete_confirmation(&confirm)?;
        linux::delete_data()
    }
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
pub mod win_cmds {
    use super::*;
    use crate::windows::{app::LastView, app::StatusView, audio, native};
    use tauri::AppHandle;

    #[tauri::command]
    pub fn win_status(app: AppHandle) -> StatusView {
        native(&app).view()
    }
    #[tauri::command]
    pub fn win_settings(app: AppHandle) -> crate::storage::Settings {
        native(&app).settings()
    }
    #[tauri::command]
    pub fn win_set_setting(
        app: AppHandle,
        key: String,
        value: String,
    ) -> Result<crate::storage::Settings, String> {
        native(&app).set_setting(&key, &value)
    }
    #[tauri::command]
    pub async fn win_devices() -> Result<Vec<audio::InputDevice>, String> {
        tauri::async_runtime::spawn_blocking(audio::list_devices)
            .await
            .map_err(|e| e.to_string())?
    }
    /// Local microphone level test; audio is discarded immediately.
    #[tauri::command]
    pub async fn win_mic_test(app: AppHandle) -> Result<Value, String> {
        let n = native(&app);
        if n.view().phase == crate::core::Phase::Recording {
            return Err("Recording in progress".into());
        }
        let device = n.settings().mic_device;
        let (peak, rms) =
            tauri::async_runtime::spawn_blocking(move || audio::mic_test(&device, 1500))
                .await
                .map_err(|e| e.to_string())??;
        Ok(serde_json::json!({ "peak": peak, "rms": rms }))
    }
    #[tauri::command]
    pub fn win_last(app: AppHandle) -> LastView {
        native(&app).last()
    }
    #[tauri::command]
    pub fn win_action(app: AppHandle, name: String) -> Result<(), String> {
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
    pub async fn win_login(app: AppHandle, url: String) -> Result<Value, String> {
        let view = native(&app).login(url).await?;
        serde_json::to_value(view).map_err(|e| e.to_string())
    }
    #[tauri::command]
    pub fn win_cancel_login(app: AppHandle) {
        native(&app).cancel_login();
    }
    #[tauri::command]
    pub fn win_logout(app: AppHandle) {
        native(&app).logout();
    }
    #[tauri::command]
    pub async fn win_account(
        app: AppHandle,
        refresh: bool,
    ) -> Result<Option<crate::api::Me>, String> {
        let n = native(&app);
        if !refresh {
            return Ok(n.cached_account());
        }
        n.refresh_account().await.map(Some)
    }
    #[tauri::command]
    pub async fn win_open_billing(
        app: AppHandle,
        portal: bool,
        plan: String,
    ) -> Result<(), String> {
        native(&app).open_billing(portal, plan).await
    }
    #[tauri::command]
    pub async fn win_dictionary(app: AppHandle) -> Result<crate::api::Dictionary, String> {
        native(&app).dictionary().await
    }
    #[tauri::command]
    pub async fn win_set_dictionary(
        app: AppHandle,
        terms: Vec<String>,
    ) -> Result<crate::api::Dictionary, String> {
        native(&app).set_dictionary(terms).await
    }
    #[tauri::command]
    pub async fn win_delete_data(app: AppHandle, confirm: String) -> Result<(), String> {
        require_delete_confirmation(&confirm)?;
        native(&app).delete_data().await
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
