//! Settings model and validation (platform independent), plus the Windows-only
//! persistence: settings JSON in the app config dir and the `scr_` token in
//! Windows Credential Manager. The token is never written to a plain file and
//! never returned to the webview.

use serde::{Deserialize, Serialize};

/// Speech languages: auto + the 20 launch languages (PRD §5).
pub const LANGUAGES: &[&str] = &[
    "auto", "en", "zh", "ja", "ko", "es", "fr", "de", "pt", "it", "ru", "ar", "hi", "tr", "nl",
    "pl", "id", "vi", "th", "sv", "uk",
];
pub const MODES: &[&str] = &["raw", "standard", "polished"];
pub const UI_LANGUAGES: &[&str] = &["en", "zh"];
pub const OUTPUT_SCRIPTS: &[&str] = &["", "hans", "hant"];
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+V";
pub const MIN_RECORD_SECONDS: u32 = 5;
pub const MAX_RECORD_SECONDS: u32 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub service_url: String,
    pub language: String,
    pub mode: String,
    pub output_script: String,
    pub dictation_commands: bool,
    /// Empty = system default input device; otherwise a cpal device id.
    pub mic_device: String,
    pub max_record_seconds: u32,
    /// Paste automatically when the focus check passes; otherwise copy only.
    pub auto_insert: bool,
    pub restore_text_clipboard: bool,
    pub hotkey: String,
    /// Windows toast for states that need attention.
    pub notifications: bool,
    /// UI / tray language: `en` or `zh` (Simplified Chinese).
    pub ui_language: String,
    pub onboarding_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            service_url: String::new(),
            language: "auto".into(),
            mode: "standard".into(),
            output_script: String::new(),
            dictation_commands: true,
            mic_device: String::new(),
            max_record_seconds: 180,
            auto_insert: true,
            restore_text_clipboard: true,
            hotkey: DEFAULT_HOTKEY.into(),
            notifications: true,
            ui_language: "en".into(),
            onboarding_done: false,
        }
    }
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err("Expected true or false".into()),
    }
}

impl Settings {
    /// Repair a file that was edited by hand or written by an older version.
    pub fn sanitized(mut self) -> Self {
        let d = Settings::default();
        if !LANGUAGES.contains(&self.language.as_str()) {
            self.language = d.language;
        }
        if !MODES.contains(&self.mode.as_str()) {
            self.mode = d.mode;
        }
        if !OUTPUT_SCRIPTS.contains(&self.output_script.as_str()) {
            self.output_script = d.output_script;
        }
        if !(MIN_RECORD_SECONDS..=MAX_RECORD_SECONDS).contains(&self.max_record_seconds) {
            self.max_record_seconds = d.max_record_seconds;
        }
        match normalize_hotkey(&self.hotkey) {
            Ok(h) => self.hotkey = h,
            Err(_) => self.hotkey = d.hotkey,
        }
        if !self.service_url.is_empty() && crate::api::validate_base_url(&self.service_url).is_err()
        {
            self.service_url.clear();
        }
        if !UI_LANGUAGES.contains(&self.ui_language.as_str()) {
            self.ui_language = d.ui_language;
        }
        if self.mic_device.len() > 512 {
            self.mic_device.clear();
        }
        self
    }

    /// Apply one `set_setting(key, value)` from the UI. `service_url` is not
    /// settable here; it changes only through login.
    pub fn apply(&mut self, key: &str, value: &str) -> Result<(), String> {
        if value.len() > 512 {
            return Err("Value too long".into());
        }
        match key {
            "language" if LANGUAGES.contains(&value) => self.language = value.into(),
            "mode" if MODES.contains(&value) => self.mode = value.into(),
            "output_script" if OUTPUT_SCRIPTS.contains(&value) => self.output_script = value.into(),
            "dictation_commands" => self.dictation_commands = parse_bool(value)?,
            "auto_insert" => self.auto_insert = parse_bool(value)?,
            "restore_text_clipboard" => self.restore_text_clipboard = parse_bool(value)?,
            "notifications" => self.notifications = parse_bool(value)?,
            "onboarding_done" => self.onboarding_done = parse_bool(value)?,
            "ui_language" if UI_LANGUAGES.contains(&value) => self.ui_language = value.into(),
            "mic_device" => {
                if value.chars().any(char::is_control) {
                    return Err("Invalid device id".into());
                }
                self.mic_device = value.trim().into();
            }
            "max_record_seconds" => {
                let v: u32 = value
                    .trim()
                    .parse()
                    .map_err(|_| "Expected a number".to_string())?;
                if !(MIN_RECORD_SECONDS..=MAX_RECORD_SECONDS).contains(&v) {
                    return Err(format!(
                        "Must be between {MIN_RECORD_SECONDS} and {MAX_RECORD_SECONDS} seconds"
                    ));
                }
                self.max_record_seconds = v;
            }
            "hotkey" => self.hotkey = normalize_hotkey(value)?,
            "language" | "mode" | "output_script" | "ui_language" => {
                return Err("Unsupported value".into())
            }
            _ => return Err("Unsupported setting".into()),
        }
        Ok(())
    }
}

/// Validate a user hotkey and return it in canonical `Ctrl+Alt+Shift+Super+Key`
/// order. At least one of Ctrl/Alt/Super is required (except F13–F24) so the
/// binding does not swallow normal typing, and paste chords are refused
/// because Scribiva itself sends Ctrl+V.
pub fn normalize_hotkey(input: &str) -> Result<String, String> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut sup = false;
    let mut key: Option<String> = None;
    let tokens: Vec<&str> = input.split('+').map(str::trim).collect();
    if tokens.iter().any(|t| t.is_empty()) {
        return Err("Invalid shortcut".into());
    }
    for token in tokens {
        if key.is_some() {
            return Err("The main key must come last".into());
        }
        match token.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => ctrl = true,
            "ALT" | "OPTION" => alt = true,
            "SHIFT" => shift = true,
            "SUPER" | "WIN" | "META" | "CMD" => sup = true,
            other => key = Some(normalize_key(other)?),
        }
    }
    let key = key.ok_or_else(|| "A main key is required".to_string())?;
    let high_f = key.starts_with('F')
        && key[1..]
            .parse::<u8>()
            .map(|n| (13..=24).contains(&n))
            .unwrap_or(false);
    if !(ctrl || alt || sup || high_f) {
        return Err("Use at least one of Ctrl, Alt or Win".into());
    }
    if ctrl && !alt && !sup && (key == "V" || key == "Insert") {
        return Err("Paste shortcuts cannot be used".into());
    }
    if shift && !ctrl && !alt && !sup && key == "Insert" {
        return Err("Paste shortcuts cannot be used".into());
    }
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    if sup {
        parts.push("Super".to_string());
    }
    parts.push(key);
    Ok(parts.join("+"))
}

fn normalize_key(upper: &str) -> Result<String, String> {
    if upper.len() == 1 {
        let c = upper.chars().next().unwrap_or(' ');
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Ok(c.to_string());
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=24).contains(&n) {
            return Ok(format!("F{n}"));
        }
    }
    match upper {
        "SPACE" => Ok("Space".into()),
        "INSERT" => Ok("Insert".into()),
        "PAUSE" => Ok("Pause".into()),
        "BACKQUOTE" | "`" => Ok("Backquote".into()),
        _ => Err(format!("Unsupported key: {upper}")),
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::Settings;
    use std::path::{Path, PathBuf};

    const KEYRING_SERVICE: &str = "app.scribiva.desktop";
    const KEYRING_USER: &str = "service-token";

    pub fn settings_path(config_dir: &Path) -> PathBuf {
        config_dir.join("settings.json")
    }

    pub fn load_settings(config_dir: &Path) -> Settings {
        std::fs::read_to_string(settings_path(config_dir))
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .unwrap_or_default()
            .sanitized()
    }

    /// Atomic write: temp file + rename. Settings contain no secrets.
    pub fn save_settings(config_dir: &Path, settings: &Settings) -> Result<(), String> {
        std::fs::create_dir_all(config_dir).map_err(|e| format!("Settings dir: {e}"))?;
        let path = settings_path(config_dir);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| format!("Settings write: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("Settings write: {e}"))
    }

    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| format!("Credential Manager unavailable: {e}"))
    }

    pub fn load_token() -> Option<String> {
        entry()
            .ok()?
            .get_password()
            .ok()
            .filter(|t| crate::api::is_valid_token(t))
    }

    pub fn store_token(token: &str) -> Result<(), String> {
        if !crate::api::is_valid_token(token) {
            return Err("Invalid token".into());
        }
        entry()?
            .set_password(token)
            .map_err(|e| format!("Credential Manager: {e}"))
    }

    pub fn delete_token() {
        if let Ok(e) = entry() {
            let _ = e.delete_credential();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let s = Settings::default();
        assert_eq!(s.hotkey, "Ctrl+Alt+V");
        assert_eq!(s.max_record_seconds, 180);
        assert_eq!(s.mode, "standard");
        assert!(s.dictation_commands);
        assert_eq!(LANGUAGES.len(), 21);
    }

    #[test]
    fn apply_validates_values() {
        let mut s = Settings::default();
        s.apply("language", "ja").unwrap();
        s.apply("mode", "polished").unwrap();
        s.apply("output_script", "hant").unwrap();
        s.apply("output_script", "").unwrap();
        s.apply("dictation_commands", "false").unwrap();
        s.apply("max_record_seconds", "60").unwrap();
        assert_eq!((s.language.as_str(), s.mode.as_str()), ("ja", "polished"));
        assert!(!s.dictation_commands);
        assert!(s.apply("language", "xx").is_err());
        assert!(s.apply("mode", "smart").is_err());
        assert!(s.apply("output_script", "trad").is_err());
        assert!(s.apply("max_record_seconds", "4").is_err());
        assert!(s.apply("max_record_seconds", "601").is_err());
        assert!(s.apply("auto_insert", "yes").is_err());
        assert!(s.apply("service_url", "https://x").is_err());
        assert!(s.apply("unknown", "1").is_err());
        assert!(s.apply("mic_device", "a\nb").is_err());
    }

    #[test]
    fn sanitize_repairs_bad_file() {
        let s: Settings = serde_json::from_str(
            r#"{"language":"klingon","mode":"x","max_record_seconds":99999,"hotkey":"V","service_url":"http://evil.example"}"#,
        )
        .unwrap();
        let s = s.sanitized();
        assert_eq!(s.language, "auto");
        assert_eq!(s.mode, "standard");
        assert_eq!(s.max_record_seconds, 180);
        assert_eq!(s.hotkey, DEFAULT_HOTKEY);
        assert!(s.service_url.is_empty());
    }

    #[test]
    fn hotkey_normalization() {
        assert_eq!(normalize_hotkey("ctrl+alt+v").unwrap(), "Ctrl+Alt+V");
        assert_eq!(
            normalize_hotkey("Alt + Ctrl + Space").unwrap(),
            "Ctrl+Alt+Space"
        );
        assert_eq!(normalize_hotkey("Win+Shift+d").unwrap(), "Shift+Super+D");
        assert_eq!(normalize_hotkey("F13").unwrap(), "F13");
        assert_eq!(normalize_hotkey("Ctrl+F9").unwrap(), "Ctrl+F9");
        assert!(normalize_hotkey("V").is_err());
        assert!(normalize_hotkey("Shift+A").is_err());
        assert!(normalize_hotkey("Ctrl+V").is_err());
        assert!(normalize_hotkey("Shift+Insert").is_err());
        assert!(normalize_hotkey("Ctrl+").is_err());
        assert!(normalize_hotkey("Ctrl+A+B").is_err());
        assert!(normalize_hotkey("Ctrl+Alt").is_err());
        assert!(normalize_hotkey("Ctrl+Alt+Ä").is_err());
        assert!(normalize_hotkey("F25").is_err());
    }
}
