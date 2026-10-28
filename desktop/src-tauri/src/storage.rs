//! Settings model and validation (platform independent), plus persistence:
//! settings JSON in the app config dir and the `scr_` token in the OS secret
//! store (Windows Credential Manager, Linux Secret Service). On Linux without
//! a Secret Service the token falls back to a 0600 file in the config dir and
//! diagnostics report it. The token is never returned to the webview.

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
    /// Linux: paste even when the focused window cannot be identified.
    pub insert_without_focus_check: bool,
    /// Start in the tray at login (`scribiva --hidden`), applied only once
    /// onboarding is done. Linux: XDG autostart entry; Windows: HKCU Run value.
    pub start_at_login: bool,
    /// Small always-on-top, non-focusable recording HUD (docs/06 §3).
    pub show_hud: bool,
    /// Check for signed updates at startup and every 24 h (FR-012).
    pub auto_update_check: bool,
    /// Report only the insertion outcome enum for each job (north-star
    /// metric, docs/11); never text or the target app.
    pub share_insert_outcome: bool,
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
            insert_without_focus_check: false,
            start_at_login: true,
            show_hud: true,
            auto_update_check: true,
            share_insert_outcome: true,
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

/// Keys that exist on the other platform only: hidden from the UI and
/// refused by `apply` (Linux sets its shortcut in GNOME, see `linux::setup`).
#[cfg(windows)]
pub const OTHER_PLATFORM_KEYS: &[&str] = &["insert_without_focus_check"];
#[cfg(not(windows))]
pub const OTHER_PLATFORM_KEYS: &[&str] = &["hotkey"];

/// Settings as shown to the webview: platform-specific keys removed.
pub fn settings_view(settings: &Settings) -> serde_json::Value {
    let mut v = serde_json::to_value(settings).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        for k in OTHER_PLATFORM_KEYS {
            obj.remove(*k);
        }
    }
    v
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
        if OTHER_PLATFORM_KEYS.contains(&key) {
            return Err("Unsupported setting".into());
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
            "insert_without_focus_check" => self.insert_without_focus_check = parse_bool(value)?,
            "start_at_login" => self.start_at_login = parse_bool(value)?,
            "show_hud" => self.show_hud = parse_bool(value)?,
            "auto_update_check" => self.auto_update_check = parse_bool(value)?,
            "share_insert_outcome" => self.share_insert_outcome = parse_bool(value)?,
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

pub use persist::*;

mod persist {
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
            .map_err(|e| format!("Credential store unavailable: {e}"))
    }

    fn keyring_load() -> Option<String> {
        entry()
            .ok()?
            .get_password()
            .ok()
            .filter(|t| crate::api::is_valid_token(t))
    }

    #[cfg(windows)]
    pub fn load_token(_config_dir: &Path) -> Option<String> {
        keyring_load()
    }

    #[cfg(windows)]
    pub fn store_token(_config_dir: &Path, token: &str) -> Result<(), String> {
        if !crate::api::is_valid_token(token) {
            return Err("Invalid token".into());
        }
        entry()?
            .set_password(token)
            .map_err(|e| format!("Credential Manager: {e}"))
    }

    #[cfg(windows)]
    pub fn delete_token(_config_dir: &Path) {
        if let Ok(e) = entry() {
            let _ = e.delete_credential();
        }
    }

    #[cfg(windows)]
    pub fn token_backend(_config_dir: &Path) -> &'static str {
        "credential-manager"
    }

    // ------------------------------------------------------------ Linux

    /// Fallback token file, used only when no Secret Service is reachable.
    #[cfg(not(windows))]
    pub fn token_file(config_dir: &Path) -> PathBuf {
        config_dir.join("service-token")
    }

    #[cfg(not(windows))]
    pub fn load_token(config_dir: &Path) -> Option<String> {
        keyring_load().or_else(|| {
            std::fs::read_to_string(token_file(config_dir))
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| crate::api::is_valid_token(t))
        })
    }

    #[cfg(not(windows))]
    pub fn store_token(config_dir: &Path, token: &str) -> Result<(), String> {
        if !crate::api::is_valid_token(token) {
            return Err("Invalid token".into());
        }
        match entry().and_then(|e| e.set_password(token).map_err(|e| e.to_string())) {
            Ok(()) => {
                // A token in the secret store supersedes an older fallback file.
                let _ = std::fs::remove_file(token_file(config_dir));
                Ok(())
            }
            Err(_) => write_private(&token_file(config_dir), token),
        }
    }

    #[cfg(not(windows))]
    pub fn delete_token(config_dir: &Path) {
        if let Ok(e) = entry() {
            let _ = e.delete_credential();
        }
        let _ = std::fs::remove_file(token_file(config_dir));
    }

    /// `secret-service`, or `file` when the 0600 fallback file is in use.
    #[cfg(not(windows))]
    pub fn token_backend(config_dir: &Path) -> &'static str {
        if token_file(config_dir).is_file() {
            "file"
        } else if keyring::Entry::store_status().is_ok() {
            "secret-service"
        } else {
            "unavailable"
        }
    }

    /// Owner-only file written atomically (created 0600, then renamed).
    #[cfg(not(windows))]
    pub fn write_private(path: &Path, content: &str) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| format!("Config dir: {e}"))?;
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("Token file: {e}"))?;
        f.write_all(content.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| format!("Token file: {e}"))?;
        drop(f);
        std::fs::rename(&tmp, path).map_err(|e| format!("Token file: {e}"))
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
        assert!(s.show_hud && s.auto_update_check && s.share_insert_outcome);
        assert!(s.start_at_login);
    }

    #[test]
    fn new_toggles_apply_and_default_for_old_files() {
        let mut s = Settings::default();
        s.apply("show_hud", "false").unwrap();
        s.apply("auto_update_check", "0").unwrap();
        s.apply("share_insert_outcome", "false").unwrap();
        s.apply("start_at_login", "false").unwrap();
        assert!(!s.show_hud && !s.auto_update_check && !s.share_insert_outcome);
        assert!(!s.start_at_login);
        assert!(s.apply("show_hud", "maybe").is_err());
        // A settings file from an older version gets the defaults.
        let old: Settings =
            serde_json::from_str(r#"{"language":"ja","auto_insert":false}"#).unwrap();
        assert!(old.show_hud && old.auto_update_check && old.share_insert_outcome);
        assert_eq!(old.language, "ja");
        assert!(!old.auto_insert);
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
        for k in OTHER_PLATFORM_KEYS {
            assert!(s.apply(k, "true").is_err(), "{k}");
        }
    }

    #[test]
    fn view_hides_other_platform_keys() {
        let v = settings_view(&Settings::default());
        for k in OTHER_PLATFORM_KEYS {
            assert!(v.get(*k).is_none(), "{k}");
        }
        assert!(v.get("mode").is_some());
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_only_settings() {
        let mut s = Settings::default();
        assert!(s.start_at_login && !s.insert_without_focus_check);
        s.apply("insert_without_focus_check", "true").unwrap();
        s.apply("start_at_login", "false").unwrap();
        assert!(s.insert_without_focus_check && !s.start_at_login);
        assert!(s.apply("hotkey", "Ctrl+Alt+B").is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn private_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("scribiva-test-{}", std::process::id()));
        let path = dir.join("sub").join("service-token");
        write_private(&path, "scr_x").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "scr_x");
        let _ = std::fs::remove_dir_all(&dir);
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
