//! Linux bridge: whitelisted calls into the installed Python CLI
//! (`~/.local/bin/scribiva`). Rust never accepts arbitrary command paths or
//! shell strings; arguments are passed as an argv array.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter};

fn binary() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not defined".to_string())?;
    let path = PathBuf::from(home).join(".local/bin/scribiva");
    if !path.is_file() {
        return Err("Install the Linux core first: bash scripts/install-ubuntu.sh".to_string());
    }
    Ok(path)
}

fn invoke(args: &[&str]) -> Result<String, String> {
    let output = Command::new(binary()?)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Scribiva CLI unavailable: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("scribiva {} failed", args.first().unwrap_or(&""))
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn invoke_json(args: &[&str]) -> Result<Value, String> {
    serde_json::from_str(&invoke(args)?).map_err(|e| e.to_string())
}

/// Settings the UI may change through `scribiva set`. Keys the installed CLI
/// does not know yet are rejected by the CLI itself; the UI hides them when
/// they are absent from `scribiva settings`.
pub const SETTABLE: &[&str] = &[
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
    "output_script",
    "dictation_commands",
    "insert_without_focus_check",
];

pub fn status() -> Result<Value, String> {
    Ok(serde_json::json!({
        "state": invoke_json(&["status"])?,
        "doctor": invoke_json(&["doctor"])?,
    }))
}

pub fn settings() -> Result<Value, String> {
    let mut v = invoke_json(&["settings"])?;
    // Defensive: never forward proxy credentials even if a CLI build prints them.
    if let Some(obj) = v.as_object_mut() {
        obj.remove("proxy_url");
    }
    Ok(v)
}

pub fn devices() -> Result<Value, String> {
    invoke_json(&["devices", "--json"])
}

pub fn last() -> Result<Value, String> {
    invoke_json(&["last", "--json"])
}

pub fn service_status() -> Result<Value, String> {
    invoke_json(&["service", "status"])
}

pub fn dictionary() -> Result<Value, String> {
    invoke_json(&["dictionary", "--json"])
}

pub fn set_dictionary(terms: &[String]) -> Result<Value, String> {
    if terms.len() > 200 || terms.iter().any(|t| t.chars().count() > 64) {
        return Err("Dictionary too large".into());
    }
    let json = serde_json::to_string(terms).map_err(|e| e.to_string())?;
    invoke(&["dictionary", "set-json", &json])?;
    dictionary()
}

pub fn set_setting(key: &str, value: &str) -> Result<(), String> {
    if !SETTABLE.contains(&key) || value.len() > 512 || value.contains('\0') {
        return Err("Unsupported setting".to_string());
    }
    invoke(&["set", key, value]).map(|_| ())
}

pub fn action(name: &str) -> Result<String, String> {
    match name {
        "toggle" | "cancel" | "retry" | "discard" | "copy-last" | "clear-last" => invoke(&[name]),
        _ => Err("Unsupported action".to_string()),
    }
}

/// `service billing|portal` prints a Stripe URL on the first stdout line.
pub fn billing_url(portal: bool, plan: &str) -> Result<String, String> {
    let out = if portal {
        invoke(&["service", "portal"])?
    } else {
        if plan != "monthly" && plan != "annual" {
            return Err("Unsupported plan".into());
        }
        invoke(&["service", "billing", "--plan", plan])?
    };
    Ok(out.lines().next().unwrap_or_default().trim().to_string())
}

pub fn logout() -> Result<String, String> {
    invoke(&["service", "logout"])
}

pub fn delete_data() -> Result<String, String> {
    invoke(&["service", "delete", "--confirm"])
}

/// `scribiva service login --url URL` runs the device flow itself (opens the
/// browser and polls). Its stdout lines (user code, URL) are streamed to the
/// webview as `login-line` events; the token never passes through here.
pub fn login(app: AppHandle, url: &str) -> Result<(), String> {
    let url = crate::api::validate_base_url(url)?;
    let mut child = Command::new(binary()?)
        .args(["service", "login", "--url", &url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Scribiva CLI unavailable: {e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().map_while(Result::ok).take(50) {
                let _ = app.emit("login-line", line.chars().take(300).collect::<String>());
            }
        }
        let err = stderr
            .map(|e| {
                BufReader::new(e)
                    .lines()
                    .map_while(Result::ok)
                    .take(20)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        let _ = app.emit(
            "login-done",
            serde_json::json!({ "ok": ok, "error": if ok { None } else { Some(err) } }),
        );
    });
    Ok(())
}
