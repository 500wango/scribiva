//! Developer Tauri shell for installed Linux Python core.
//! Cross-platform UI only. Native Windows capture/input adapter is NOT shipped.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde_json::Value;
use std::{path::PathBuf, process::Command};

fn linux_binary() -> Result<PathBuf, String> {
    if !cfg!(target_os = "linux") {
        return Err("Windows native adapter is planned, not implemented".to_string());
    }
    let home = std::env::var("HOME").map_err(|_| "HOME not defined".to_string())?;
    let path = PathBuf::from(home).join(".local/bin/scribiva");
    if !path.is_file() {
        return Err("Install Linux core first: bash scripts/install-ubuntu.sh".to_string());
    }
    Ok(path)
}

fn invoke_cli(args: &[&str]) -> Result<String, String> {
    let output = Command::new(linux_binary()?)
        .args(args)
        .output()
        .map_err(|e| format!("Scribiva CLI unavailable: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tauri::command]
fn get_status() -> Result<Value, String> {
    let status = invoke_cli(&["status"])?;
    let doctor = invoke_cli(&["doctor"])?;
    Ok(serde_json::json!({
        "state": serde_json::from_str::<Value>(&status).map_err(|e| e.to_string())?,
        "doctor": serde_json::from_str::<Value>(&doctor).map_err(|e| e.to_string())?
    }))
}

#[tauri::command]
fn get_settings() -> Result<Value, String> {
    let result = invoke_cli(&["settings"])?;
    serde_json::from_str(&result).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_devices() -> Result<Value, String> {
    let result = invoke_cli(&["devices", "--json"])?;
    serde_json::from_str(&result).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_last() -> Result<Value, String> {
    let result = invoke_cli(&["last", "--json"])?;
    serde_json::from_str(&result).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_last() -> Result<(), String> {
    invoke_cli(&["clear-last"])?;
    Ok(())
}

#[tauri::command]
fn set_setting(key: String, value: String) -> Result<(), String> {
    const ALLOWED: [&str; 10] = [
        "language",
        "mode",
        "asr_model",
        "auto_insert",
        "restore_text_clipboard",
        "max_record_seconds",
        "mic_node",
        "delete_audio_on_success",
        "asr_prompt",
        "service_url",
    ];
    if !ALLOWED.contains(&key.as_str()) || value.len() > 512 {
        return Err("Unsupported setting".to_string());
    }
    invoke_cli(&["set", &key, &value])?;
    Ok(())
}

#[tauri::command]
fn action(name: String) -> Result<String, String> {
    match name.as_str() {
        "toggle" | "cancel" | "retry" | "discard" | "copy-last" => invoke_cli(&[name.as_str()]),
        _ => Err("Unsupported action".to_string()),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_settings,
            get_devices,
            get_last,
            clear_last,
            set_setting,
            action
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run Scribiva");
}
