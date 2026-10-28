//! One-time import from the Python client (`~/.config/scribiva/`). Old files
//! are read, never modified or deleted. Values are applied through
//! `Settings::apply`, so anything invalid is skipped instead of imported.

use crate::storage::Settings;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Marker in the new config dir: the import ran once (even if it found nothing).
const MARKER: &str = "migrated-from-python";

pub fn legacy_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/scribiva"))
}

fn as_setting(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => n.as_u64().map(|n| n.to_string()),
        _ => None,
    }
}

/// Apply the Python `settings.json` and `mic-node` to `settings`. Returns the
/// list of imported keys. `service_url` is validated but set directly, since
/// it is not a UI-settable key.
pub fn import_settings(legacy: &Path, settings: &mut Settings) -> Vec<&'static str> {
    let mut imported = Vec::new();
    let json = std::fs::read_to_string(legacy.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let obj = json.as_ref().and_then(Value::as_object);
    // Python key → Rust key.
    const KEYS: &[(&str, &str)] = &[
        ("language", "language"),
        ("mode", "mode"),
        ("auto_insert", "auto_insert"),
        ("restore_text_clipboard", "restore_text_clipboard"),
        ("max_record_seconds", "max_record_seconds"),
        ("output_script", "output_script"),
        ("dictation_commands", "dictation_commands"),
        ("insert_without_focus_check", "insert_without_focus_check"),
        ("mic_node", "mic_device"),
    ];
    if let Some(obj) = obj {
        for (py, rs) in KEYS {
            let Some(value) = obj.get(*py).and_then(as_setting) else {
                continue;
            };
            if *py == "mic_node" && value.trim().is_empty() {
                continue;
            }
            if settings.apply(rs, &value).is_ok() {
                imported.push(*rs);
            }
        }
        if let Some(url) = obj.get("service_url").and_then(Value::as_str) {
            if let Ok(url) = crate::api::validate_base_url(url) {
                settings.service_url = url;
                imported.push("service_url");
            }
        }
    }
    // Legacy single-line mic file, used when settings.json has no mic_node.
    if !imported.contains(&"mic_device") {
        if let Ok(node) = std::fs::read_to_string(legacy.join("mic-node")) {
            let node = node.trim();
            if !node.is_empty()
                && super::audio::valid_node_name(node)
                && settings.apply("mic_device", node).is_ok()
            {
                imported.push("mic_device");
            }
        }
    }
    imported
}

/// The Python `service-token`, if it holds a valid token.
pub fn legacy_token(legacy: &Path) -> Option<String> {
    std::fs::read_to_string(legacy.join("service-token"))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| crate::api::is_valid_token(t))
}

pub struct Migration {
    pub settings: Option<Settings>,
    pub token: Option<String>,
}

/// Plan the first-run import. Returns None when it already ran or when the
/// new config dir already has settings (not a first run).
pub fn plan(config_dir: &Path, legacy: &Path, current: &Settings) -> Option<Migration> {
    if config_dir.join(MARKER).exists() || crate::storage::settings_path(config_dir).exists() {
        return None;
    }
    if !legacy.is_dir() {
        return None;
    }
    let mut next = current.clone();
    let imported = import_settings(legacy, &mut next);
    Some(Migration {
        settings: (!imported.is_empty()).then_some(next),
        token: legacy_token(legacy),
    })
}

pub fn mark_done(config_dir: &Path) {
    let _ = std::fs::create_dir_all(config_dir);
    let _ = std::fs::write(config_dir.join(MARKER), b"1\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scribiva-mig-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn imports_python_settings_fixture() {
        let legacy = tmp("legacy");
        std::fs::write(
            legacy.join("settings.json"),
            r#"{"language": "ja", "mode": "polished", "auto_insert": false,
                "restore_text_clipboard": false, "max_record_seconds": 300,
                "mic_node": "alsa_input.usb-Blue_Yeti-00.analog-stereo",
                "output_script": "hant", "dictation_commands": false,
                "insert_without_focus_check": true, "service_url": "https://api.scribiva.app/",
                "proxy_url": "http://user:secret@proxy:3128", "asr_model": "whisper-large-v3",
                "grammar_model": "x", "delete_audio_on_success": true}"#,
        )
        .unwrap();
        std::fs::write(
            legacy.join("mic-node"),
            "ignored-because-settings-has-one\n",
        )
        .unwrap();
        let token = format!("scr_{}", "a".repeat(32));
        std::fs::write(legacy.join("service-token"), format!("{token}\n")).unwrap();

        let config = tmp("config");
        let m = plan(&config, &legacy, &Settings::default()).unwrap();
        let s = m.settings.unwrap();
        assert_eq!((s.language.as_str(), s.mode.as_str()), ("ja", "polished"));
        assert!(!s.auto_insert && !s.restore_text_clipboard && !s.dictation_commands);
        assert!(s.insert_without_focus_check);
        assert_eq!(s.max_record_seconds, 300);
        assert_eq!(s.mic_device, "alsa_input.usb-Blue_Yeti-00.analog-stereo");
        assert_eq!(s.output_script, "hant");
        assert_eq!(s.service_url, "https://api.scribiva.app");
        assert_eq!(m.token.as_deref(), Some(token.as_str()));
        // Old files untouched.
        assert!(legacy.join("service-token").exists() && legacy.join("settings.json").exists());

        mark_done(&config);
        assert!(plan(&config, &legacy, &Settings::default()).is_none());
        let _ = std::fs::remove_dir_all(&legacy);
        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn invalid_values_are_skipped_and_mic_file_used() {
        let legacy = tmp("legacy2");
        std::fs::write(
            legacy.join("settings.json"),
            r#"{"language": "klingon", "mode": "smart", "max_record_seconds": 99999,
                "service_url": "http://evil.example", "mic_node": ""}"#,
        )
        .unwrap();
        std::fs::write(legacy.join("mic-node"), "alsa_input.pci-0000_00_1f.3\n").unwrap();
        std::fs::write(legacy.join("service-token"), "not-a-token").unwrap();
        let mut s = Settings::default();
        let imported = import_settings(&legacy, &mut s);
        assert_eq!(imported, ["mic_device"]);
        assert_eq!(s.language, "auto");
        assert_eq!(s.mode, "standard");
        assert!(s.service_url.is_empty());
        assert_eq!(s.mic_device, "alsa_input.pci-0000_00_1f.3");
        assert!(legacy_token(&legacy).is_none());
        let _ = std::fs::remove_dir_all(&legacy);
    }

    #[test]
    fn no_legacy_dir_or_existing_config_means_no_migration() {
        let config = tmp("config3");
        let missing = config.join("nope");
        assert!(plan(&config, &missing, &Settings::default()).is_none());
        let legacy = tmp("legacy3");
        crate::storage::save_settings(&config, &Settings::default()).unwrap();
        assert!(plan(&config, &legacy, &Settings::default()).is_none());
        let _ = std::fs::remove_dir_all(&config);
        let _ = std::fs::remove_dir_all(&legacy);
    }
}
