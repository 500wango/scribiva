//! First-run desktop integration for GNOME, port of `src/scribiva/setup.py`.
//!
//! - A GNOME custom shortcut that runs `scribiva --toggle` (GNOME 46 has no
//!   global-shortcut API for apps). Other custom shortcuts are preserved; a
//!   binding already used by one of them is refused, never overwritten.
//! - The Scribiva GNOME Shell extension state, and enabling it (ADR-012). On
//!   Wayland a newly enabled extension loads only after the next login.
//! - KDE Plasma: the Scribiva KWin focus script, enabled in kwinrc and loaded
//!   into the running KWin over D-Bus (no logout).
//!
//! All gsettings / gnome-extensions / kwriteconfig6 / qdbus calls go through `Runner`, so the logic is
//! unit-tested with a fake and never touches the real session in tests.

use super::proc;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
pub const BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
pub const EXTENSION_UUID: &str = "scribiva@scribiva.app";
pub const DEFAULT_BINDING: &str = "<Primary><Alt>v";
const SHELL_SCHEMA: &str = "org.gnome.shell";
const PACKAGED_EXE: &str = "/usr/bin/scribiva";

pub fn our_path() -> String {
    format!("{BASE}scribiva/")
}

fn legacy_paths() -> [String; 2] {
    [
        format!("{BASE}scribiva-alpha/"),
        format!("{BASE}scribiva-gate1/"),
    ]
}

fn item(path: &str) -> String {
    format!("{SCHEMA}.custom-keybinding:{path}")
}

/// Runs desktop configuration tools; faked in tests.
pub trait Runner {
    fn available(&self, program: &str) -> bool;
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
}

pub struct System;

impl Runner for System {
    fn available(&self, program: &str) -> bool {
        proc::which(program).is_some()
    }
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        proc::run_ok(program, args, Duration::from_secs(10))
    }
}

// ---------------------------------------------------------------- GVariant text

/// Parse a GVariant text string literal (`'a'` or `"a"`), or return the input
/// unchanged when it is not quoted.
pub fn unquote(value: &str) -> String {
    let v = value.trim();
    let mut chars = v.chars();
    let Some(q @ ('\'' | '"')) = chars.next() else {
        return v.to_string();
    };
    let mut out = String::new();
    let mut escaped = false;
    for c in chars {
        if escaped {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == q {
            return out;
        } else {
            out.push(c);
        }
    }
    v.to_string()
}

/// Parse `['a', 'b']` or `@as []` into strings.
pub fn parse_string_list(raw: &str) -> Result<Vec<String>, String> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("@as").map(str::trim).unwrap_or(raw);
    let inner = raw
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .ok_or_else(|| {
            format!(
                "unexpected gsettings value: {}",
                raw.chars().take(80).collect::<String>()
            )
        })?;
    let mut items = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let Some(q) = chars.next() else {
            break;
        };
        if q != '\'' && q != '"' {
            return Err("unexpected gsettings list".into());
        }
        let mut s = String::new();
        let mut closed = false;
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(n) = chars.next() {
                    s.push(n);
                }
            } else if c == q {
                closed = true;
                break;
            } else {
                s.push(c);
            }
        }
        if !closed {
            return Err("unterminated gsettings string".into());
        }
        items.push(s);
    }
    Ok(items)
}

/// GVariant text for a string (always quoted, so gsettings never reinterprets it).
pub fn quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

pub fn format_string_list(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// ---------------------------------------------------------------- bindings

/// GNOME accelerator such as `<Primary><Alt>v`: at least one modifier, one
/// alphanumeric key, and never a paste chord (it would fire on every paste).
pub fn valid_gnome_binding(b: &str) -> bool {
    let Some((mods, key)) = split_binding(b) else {
        return false;
    };
    let key_ok = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let ctrl = mods.iter().any(|m| *m == "<Primary>" || *m == "<Control>");
    let only_ctrl_or_shift = mods
        .iter()
        .all(|m| matches!(*m, "<Primary>" | "<Control>" | "<Shift>"));
    let paste = ctrl && only_ctrl_or_shift && key.eq_ignore_ascii_case("v");
    !mods.is_empty() && key_ok && !paste
}

const MODS: &[&str] = &[
    "<Primary>",
    "<Control>",
    "<Ctrl>",
    "<Alt>",
    "<Shift>",
    "<Super>",
];

fn split_binding(b: &str) -> Option<(Vec<&'static str>, &str)> {
    if b.len() > 64 || !b.is_ascii() {
        return None;
    }
    let mut rest = b;
    let mut mods = Vec::new();
    while let Some(m) = MODS
        .iter()
        .find(|m| rest.len() >= m.len() && rest[..m.len()].eq_ignore_ascii_case(m))
    {
        mods.push(if *m == "<Ctrl>" { "<Control>" } else { *m });
        rest = &rest[m.len()..];
    }
    Some((mods, rest))
}

/// Canonical form for comparing accelerators (`<Control>` = `<Primary>`,
/// modifier order and key case ignored).
pub fn canonical_binding(b: &str) -> String {
    let Some((mods, key)) = split_binding(b.trim()) else {
        return b.trim().to_ascii_lowercase();
    };
    let mut names: Vec<&str> = mods
        .iter()
        .map(|m| if *m == "<Control>" { "<Primary>" } else { m })
        .collect();
    names.sort_unstable();
    names.dedup();
    format!("{}{}", names.concat(), key.to_ascii_lowercase())
}

/// `<Primary><Alt>v` → `Ctrl+Alt+V` for the UI and tray.
pub fn pretty_binding(b: &str) -> String {
    let Some((mods, key)) = split_binding(b) else {
        return b.to_string();
    };
    let mut parts: Vec<String> = Vec::new();
    for (m, label) in [
        ("<Primary>", "Ctrl"),
        ("<Alt>", "Alt"),
        ("<Shift>", "Shift"),
        ("<Super>", "Super"),
    ] {
        if mods
            .iter()
            .any(|x| *x == m || (m == "<Primary>" && *x == "<Control>"))
        {
            parts.push(label.into());
        }
    }
    let key = if key.len() == 1 {
        key.to_ascii_uppercase()
    } else {
        let mut c = key.chars();
        c.next()
            .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
            .unwrap_or_default()
    };
    parts.push(key);
    parts.join("+")
}

// ---------------------------------------------------------------- command

/// Quote an argument for GNOME's command parsing (g_shell_parse_argv).
fn shell_word(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Executable the shortcut and autostart entry run: the packaged binary if
/// installed, else this executable (developer builds).
pub fn app_exe() -> PathBuf {
    let packaged = Path::new(PACKAGED_EXE);
    if proc::is_executable(packaged) {
        return packaged.to_path_buf();
    }
    std::env::current_exe().unwrap_or_else(|_| packaged.to_path_buf())
}

pub fn toggle_command_for(exe: &Path) -> String {
    format!("{} --toggle", shell_word(&exe.to_string_lossy()))
}

pub fn toggle_command() -> String {
    toggle_command_for(&app_exe())
}

// ---------------------------------------------------------------- hotkey

#[derive(Debug, Clone, PartialEq, Eq)]
struct Shortcut {
    path: String,
    command: String,
    binding: String,
}

fn get(r: &dyn Runner, path: &str, key: &str) -> Result<String, String> {
    r.run("gsettings", &["get", &item(path), key])
        .map(|v| unquote(&v))
}

fn set(r: &dyn Runner, path: &str, key: &str, value: &str) -> Result<(), String> {
    r.run("gsettings", &["set", &item(path), key, &quote(value)])
        .map(|_| ())
}

fn paths(r: &dyn Runner) -> Result<Vec<String>, String> {
    parse_string_list(&r.run("gsettings", &["get", SCHEMA, "custom-keybindings"])?)
}

/// Shortcuts created by any Scribiva version (incl. Gate/alpha scripts and
/// the Python CLI's `scribiva-cli toggle`).
fn is_scribiva(path: &str, command: &str) -> bool {
    path == our_path()
        || legacy_paths().iter().any(|p| p == path)
        || command.to_lowercase().contains("scribiva")
}

fn ours(r: &dyn Runner, all: &[String]) -> Result<Vec<Shortcut>, String> {
    let mut found = Vec::new();
    for path in all {
        let command = get(r, path, "command")?;
        if is_scribiva(path, &command) {
            found.push(Shortcut {
                path: path.clone(),
                binding: get(r, path, "binding")?,
                command,
            });
        }
    }
    Ok(found)
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct HotkeyStatus {
    pub available: bool,
    pub installed: bool,
    /// A Scribiva shortcut exists but runs an old or missing command.
    pub broken: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

pub fn hotkey_status(r: &dyn Runner, desired_command: &str) -> HotkeyStatus {
    if !r.available("gsettings") {
        return HotkeyStatus::default();
    }
    let Ok(found) = paths(r).and_then(|p| ours(r, &p)) else {
        return HotkeyStatus::default();
    };
    let Some(main) = found
        .iter()
        .find(|s| s.path == our_path())
        .or(found.first())
    else {
        return HotkeyStatus {
            available: true,
            ..Default::default()
        };
    };
    let working = main.path == our_path() && found.len() == 1 && main.command == desired_command;
    HotkeyStatus {
        available: true,
        installed: working,
        broken: !working,
        label: Some(pretty_binding(&main.binding)),
        binding: Some(main.binding.clone()),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BindResult {
    pub binding: String,
    pub command: String,
    pub replaced: Vec<String>,
}

/// Create or update the Scribiva shortcut; never takes another one's binding.
pub fn bind_hotkey(r: &dyn Runner, binding: &str, command: &str) -> Result<BindResult, String> {
    if !r.available("gsettings") {
        return Err(format!(
            "gsettings not found; add a shortcut that runs `{command}` in your desktop settings"
        ));
    }
    let all = paths(r)?;
    let found = ours(r, &all)?;
    let our_paths: Vec<&str> = found.iter().map(|s| s.path.as_str()).collect();
    let wanted = canonical_binding(binding);
    for path in all.iter().filter(|p| !our_paths.contains(&p.as_str())) {
        if canonical_binding(&get(r, path, "binding")?) == wanted {
            let name = get(r, path, "name").unwrap_or_default();
            let name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
            return Err(format!(
                "HOTKEY_CONFLICT: {} is already used by the custom shortcut \"{name}\"; choose another binding",
                pretty_binding(binding)
            ));
        }
    }
    let ours_path = our_path();
    set(r, &ours_path, "name", "Scribiva")?;
    set(r, &ours_path, "command", command)?;
    set(r, &ours_path, "binding", binding)?;
    let stale: Vec<String> = found
        .iter()
        .map(|s| s.path.clone())
        .filter(|p| *p != ours_path)
        .collect();
    let mut updated: Vec<String> = all.iter().filter(|p| !stale.contains(p)).cloned().collect();
    if !updated.contains(&ours_path) {
        updated.push(ours_path);
    }
    if updated != all {
        r.run(
            "gsettings",
            &[
                "set",
                SCHEMA,
                "custom-keybindings",
                &format_string_list(&updated),
            ],
        )?;
    }
    // Older Scribiva entries are cleared so one shortcut runs the current command.
    for old in &stale {
        for key in ["name", "command", "binding"] {
            let _ = r.run("gsettings", &["reset", &item(old), key]);
        }
    }
    let mut replaced = stale;
    replaced.sort();
    Ok(BindResult {
        binding: binding.to_string(),
        command: command.to_string(),
        replaced,
    })
}

/// Point an existing but broken Scribiva shortcut (Python CLI, deleted
/// script, moved binary) at the current command, keeping the user's key
/// combination. Does nothing when none exists (the user never opted in).
pub fn repair_hotkey(r: &dyn Runner, command: &str) -> Option<Result<BindResult, String>> {
    let status = hotkey_status(r, command);
    if !status.broken {
        return None;
    }
    let binding = status
        .binding
        .filter(|b| !b.is_empty() && b.len() <= 64 && !b.chars().any(char::is_control))
        .unwrap_or_else(|| DEFAULT_BINDING.to_string());
    Some(bind_hotkey(r, &binding, command))
}

// ---------------------------------------------------------------- extension

pub fn extension_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/share/gnome-shell/extensions")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/gnome-shell/extensions"));
    }
    dirs
}

pub fn extension_installed() -> bool {
    extension_dirs()
        .iter()
        .any(|d| d.join(EXTENSION_UUID).join("metadata.json").is_file())
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ExtensionStatus {
    pub installed: bool,
    /// Persisted choice in `org.gnome.shell`.
    pub enabled: bool,
    /// Loaded in this session (bus name owned).
    pub active: bool,
    /// Interface version (`Version` property), 0 when not active.
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_relogin: Option<bool>,
    /// The running extension predates the hot-reload loader, so it cannot be
    /// updated in place; the installed version takes over at the next login.
    #[serde(default)]
    pub update_at_next_login: bool,
}

fn string_list(r: &dyn Runner, schema: &str, key: &str) -> Result<Vec<String>, String> {
    parse_string_list(&r.run("gsettings", &["get", schema, key])?)
}

pub fn extension_enabled(r: &dyn Runner) -> bool {
    if !r.available("gsettings") {
        return false;
    }
    let enabled = string_list(r, SHELL_SCHEMA, "enabled-extensions").unwrap_or_default();
    let disabled = string_list(r, SHELL_SCHEMA, "disabled-extensions").unwrap_or_default();
    enabled.iter().any(|e| e == EXTENSION_UUID) && !disabled.iter().any(|e| e == EXTENSION_UUID)
}

/// Enable the extension now (if the shell already knows it) and persist the
/// choice so it loads at the next login. `installed`/`active` are probes.
pub fn enable_extension(
    r: &dyn Runner,
    installed: bool,
    active: &dyn Fn() -> (bool, u32),
) -> Result<ExtensionStatus, String> {
    if !installed {
        return Err(
            "The Scribiva GNOME extension is not installed; reinstall the scribiva package".into(),
        );
    }
    if r.available("gnome-extensions") {
        // On Wayland the shell may not have scanned a new extension yet; the
        // command then fails and the persisted list below takes over.
        let _ = r.run("gnome-extensions", &["enable", EXTENSION_UUID]);
    }
    let persist = || -> Result<(), String> {
        let mut enabled = string_list(r, SHELL_SCHEMA, "enabled-extensions")?;
        if !enabled.iter().any(|e| e == EXTENSION_UUID) {
            enabled.push(EXTENSION_UUID.to_string());
            r.run(
                "gsettings",
                &[
                    "set",
                    SHELL_SCHEMA,
                    "enabled-extensions",
                    &format_string_list(&enabled),
                ],
            )?;
        }
        let disabled = string_list(r, SHELL_SCHEMA, "disabled-extensions")?;
        if disabled.iter().any(|e| e == EXTENSION_UUID) {
            let rest: Vec<String> = disabled
                .into_iter()
                .filter(|e| e != EXTENSION_UUID)
                .collect();
            r.run(
                "gsettings",
                &[
                    "set",
                    SHELL_SCHEMA,
                    "disabled-extensions",
                    &format_string_list(&rest),
                ],
            )?;
        }
        Ok(())
    };
    persist().map_err(|e| {
        format!(
            "Could not enable the extension: {}",
            e.chars().take(200).collect::<String>()
        )
    })?;
    let enabled = extension_enabled(r);
    let (active, version) = active();
    Ok(ExtensionStatus {
        installed,
        enabled,
        active,
        version,
        needs_relogin: Some(enabled && !active),
        update_at_next_login: false,
    })
}

// ---------------------------------------------------------------- KWin script

/// `kwinrc [Plugins]` key KWin reads for the script (`<Id>Enabled`).
pub fn kwin_enabled_key() -> String {
    format!("{}Enabled", super::kwin::SCRIPT_ID)
}

pub fn kwin_script_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/share/kwin/scripts")];
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(d) = data_home {
        dirs.push(d.join("kwin/scripts"));
    }
    dirs
}

/// A KPackage with our id and its main script, in any of `dirs`.
pub fn kwin_script_installed_in(dirs: &[PathBuf]) -> bool {
    dirs.iter().any(|d| {
        let root = d.join(super::kwin::SCRIPT_ID);
        root.join("metadata.json").is_file() && root.join("contents/code/main.js").is_file()
    })
}

pub fn kwin_script_installed() -> bool {
    kwin_script_installed_in(&kwin_script_dirs())
}

/// KConfig boolean as KWin reads it (`QVariant(value).toBool()`).
fn kconfig_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "on" | "yes"
    )
}

/// `[Plugins] scribiva-focusEnabled=...` from kwinrc text (fallback when
/// kreadconfig6 is not installed). Missing key = disabled (the script is
/// not enabled by default).
pub fn kwinrc_enabled(text: &str) -> bool {
    let key = kwin_enabled_key();
    let mut in_plugins = false;
    let mut enabled = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_plugins = line == "[Plugins]";
        } else if in_plugins {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    enabled = kconfig_true(v);
                }
            }
        }
    }
    enabled
}

fn kwinrc_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("kwinrc"))
}

/// Persisted choice in kwinrc.
pub fn kwin_script_enabled(r: &dyn Runner) -> bool {
    let key = kwin_enabled_key();
    if r.available("kreadconfig6") {
        if let Ok(v) = r.run(
            "kreadconfig6",
            &["--file", "kwinrc", "--group", "Plugins", "--key", &key],
        ) {
            return kconfig_true(&v);
        }
    }
    kwinrc_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| kwinrc_enabled(&t))
}

/// Ask the running KWin to load newly enabled scripts, as the KWin Scripts
/// settings page does (`org.kde.kwin.Scripting.start`). No logout needed.
fn kwin_load_scripts(r: &dyn Runner) -> Result<(), String> {
    let kwin = super::kwin::KWIN_BUS_NAME;
    let path = super::kwin::SCRIPTING_PATH;
    let method = format!("{}.start", super::kwin::SCRIPTING_INTERFACE);
    for qdbus in ["qdbus6", "qdbus"] {
        if r.available(qdbus) {
            return r.run(qdbus, &[kwin, path, &method]).map(|_| ());
        }
    }
    if r.available("dbus-send") {
        let dest = format!("--dest={kwin}");
        return r
            .run(
                "dbus-send",
                &["--session", "--type=method_call", &dest, path, &method],
            )
            .map(|_| ());
    }
    Err("neither qdbus6 nor dbus-send found".into())
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct KwinStatus {
    /// KDE Plasma session.
    pub kde: bool,
    pub kwin_script_installed: bool,
    /// Persisted in kwinrc `[Plugins] scribiva-focusEnabled`.
    pub kwin_script_enabled: bool,
    /// An `Activated` call from KWin arrived since this app started.
    pub kwin_focus_active: bool,
}

/// Enable the script in kwinrc and load it into the running KWin.
/// `focus_active` waits briefly for the script's first report.
pub fn enable_kwin_script(
    r: &dyn Runner,
    kde: bool,
    installed: bool,
    focus_active: &dyn Fn() -> bool,
) -> Result<KwinStatus, String> {
    if !installed {
        return Err(
            "The Scribiva KWin script is not installed; reinstall the scribiva package".into(),
        );
    }
    if !r.available("kwriteconfig6") {
        return Err(format!(
            "kwriteconfig6 not found; enable \"Scribiva focus\" under System Settings > Window Management > KWin Scripts ({})",
            kwin_enabled_key()
        ));
    }
    r.run(
        "kwriteconfig6",
        &[
            "--file",
            "kwinrc",
            "--group",
            "Plugins",
            "--key",
            &kwin_enabled_key(),
            "--type",
            "bool",
            "true",
        ],
    )
    .map_err(|e| {
        format!(
            "Could not enable the KWin script: {}",
            e.chars().take(200).collect::<String>()
        )
    })?;
    // The choice is saved either way; a failed live load only means the
    // script starts with the next KWin start.
    if let Err(e) = kwin_load_scripts(r) {
        eprintln!("Scribiva: loading the KWin script failed: {e}");
    }
    Ok(KwinStatus {
        kde,
        kwin_script_installed: installed,
        kwin_script_enabled: kwin_script_enabled(r),
        kwin_focus_active: focus_active(),
    })
}

pub fn kwin_status(r: &dyn Runner) -> KwinStatus {
    let kde = super::kwin::is_kde();
    KwinStatus {
        kde,
        kwin_script_installed: kwin_script_installed(),
        kwin_script_enabled: kde && kwin_script_enabled(r),
        kwin_focus_active: super::kwin::received(),
    }
}

// ---------------------------------------------------------------- live probes

pub fn session_type() -> String {
    std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into())
}

pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("GNOME"))
}

fn active_probe() -> (bool, u32) {
    let active = super::shell::extension_active();
    (active, if active { super::shell::version() } else { 0 })
}

pub fn extension_status(r: &dyn Runner) -> ExtensionStatus {
    let (active, version) = active_probe();
    let loader = if active {
        super::shell::loader_state()
    } else {
        None
    };
    ExtensionStatus {
        installed: extension_installed(),
        enabled: extension_enabled(r),
        active,
        version,
        needs_relogin: None,
        update_at_next_login: super::shell::extension_action(loader.as_ref())
            == super::shell::ExtensionAction::NeedsLogout,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupStatus {
    pub session: String,
    pub gnome: bool,
    pub hotkey: HotkeyStatus,
    pub extension: ExtensionStatus,
    /// KDE Plasma: the Scribiva KWin focus script.
    pub kwin: KwinStatus,
    /// Command the shortcut runs (shown for manual setup on other desktops).
    pub command: String,
}

pub fn status() -> SetupStatus {
    let command = toggle_command();
    SetupStatus {
        session: session_type(),
        gnome: is_gnome(),
        hotkey: hotkey_status(&System, &command),
        extension: extension_status(&System),
        kwin: kwin_status(&System),
        command,
    }
}

/// `setup(action, binding)` from the UI.
pub fn run_action(action: &str, binding: Option<&str>) -> Result<serde_json::Value, String> {
    let mut out = serde_json::Map::new();
    match action {
        "hotkey" => {
            let binding = binding.unwrap_or(DEFAULT_BINDING);
            if !valid_gnome_binding(binding) {
                return Err("Unsupported shortcut".into());
            }
            let r = bind_hotkey(&System, binding, &toggle_command())?;
            out.insert(
                "hotkey".into(),
                serde_json::to_value(r).map_err(|e| e.to_string())?,
            );
        }
        "extension" => {
            let r = enable_extension(&System, extension_installed(), &active_probe)?;
            out.insert(
                "extension".into(),
                serde_json::to_value(r).map_err(|e| e.to_string())?,
            );
        }
        "kwin" => {
            let r = enable_kwin_script(
                &System,
                super::kwin::is_kde(),
                kwin_script_installed(),
                &|| {
                    super::kwin::resync();
                    super::kwin::wait_received(Duration::from_secs(2))
                },
            )?;
            out.insert(
                "kwin".into(),
                serde_json::to_value(r).map_err(|e| e.to_string())?,
            );
        }
        _ => return Err("Unsupported setup action".into()),
    }
    out.insert(
        "status".into(),
        serde_json::to_value(status()).map_err(|e| e.to_string())?,
    );
    Ok(serde_json::Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// In-memory gsettings: `schema[:path]` + key → GVariant text.
    #[derive(Default)]
    struct Fake {
        values: RefCell<BTreeMap<(String, String), String>>,
        calls: RefCell<Vec<String>>,
        no_gsettings: bool,
        /// Programs reported as not installed.
        missing: Vec<&'static str>,
    }

    impl Fake {
        fn with_shortcuts(list: &[(&str, &str, &str, &str)]) -> Self {
            let f = Fake::default();
            let paths: Vec<String> = list.iter().map(|(p, ..)| format!("{BASE}{p}/")).collect();
            f.put(SCHEMA, "custom-keybindings", &format_string_list(&paths));
            for (p, name, command, binding) in list {
                let it = item(&format!("{BASE}{p}/"));
                f.put(&it, "name", &quote(name));
                f.put(&it, "command", &quote(command));
                f.put(&it, "binding", &quote(binding));
            }
            f.put(SHELL_SCHEMA, "enabled-extensions", "@as []");
            f.put(
                SHELL_SCHEMA,
                "disabled-extensions",
                "['scribiva@scribiva.app']",
            );
            f
        }
        fn put(&self, schema: &str, key: &str, v: &str) {
            self.values
                .borrow_mut()
                .insert((schema.into(), key.into()), v.into());
        }
        fn value(&self, schema: &str, key: &str) -> Option<String> {
            self.values
                .borrow()
                .get(&(schema.to_string(), key.to_string()))
                .cloned()
        }
        fn shortcut(&self, path: &str, key: &str) -> Option<String> {
            self.value(&item(path), key).map(|v| unquote(&v))
        }
        fn list(&self) -> Vec<String> {
            parse_string_list(&self.value(SCHEMA, "custom-keybindings").unwrap()).unwrap()
        }
    }

    impl Runner for Fake {
        fn available(&self, program: &str) -> bool {
            !(self.missing.contains(&program) || self.no_gsettings && program == "gsettings")
        }
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            self.calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            // kwinrc stored as ("kwinrc:<group>", key).
            match (program, args) {
                ("kreadconfig6", ["--file", file, "--group", group, "--key", key]) => {
                    return Ok(self
                        .value(&format!("{file}:{group}"), key)
                        .unwrap_or_default());
                }
                (
                    "kwriteconfig6",
                    ["--file", file, "--group", group, "--key", key, "--type", "bool", value],
                ) => {
                    self.put(&format!("{file}:{group}"), key, value);
                    return Ok(String::new());
                }
                ("kwriteconfig6", _) => return Err("bad kwriteconfig6 call".into()),
                _ => {}
            }
            if program != "gsettings" {
                return Ok(String::new());
            }
            match args {
                ["get", schema, key] => Ok(self.value(schema, key).unwrap_or_else(|| "''".into())),
                ["set", schema, key, value] => {
                    self.put(schema, key, value);
                    Ok(String::new())
                }
                ["reset", schema, key] => {
                    self.values
                        .borrow_mut()
                        .remove(&(schema.to_string(), key.to_string()));
                    Ok(String::new())
                }
                _ => Err("bad call".into()),
            }
        }
    }

    const CMD: &str = "/usr/bin/scribiva --toggle";

    #[test]
    fn gvariant_text() {
        assert_eq!(parse_string_list("@as []").unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_string_list("['/a/', \"/b'c/\", '/d\\'e/']").unwrap(),
            ["/a/", "/b'c/", "/d'e/"]
        );
        assert!(parse_string_list("nope").is_err());
        assert!(parse_string_list("['open").is_err());
        assert_eq!(unquote("'it\\'s'"), "it's");
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(quote("a'b\\c"), "'a\\'b\\\\c'");
        let items = vec!["x'y".to_string(), "z".to_string()];
        assert_eq!(
            parse_string_list(&format_string_list(&items)).unwrap(),
            items
        );
    }

    #[test]
    fn gnome_bindings() {
        assert!(valid_gnome_binding("<Primary><Alt>v"));
        assert!(valid_gnome_binding("<Super>space"));
        assert!(valid_gnome_binding("<Control><Alt>F9"));
        assert!(!valid_gnome_binding("<Primary>v"));
        assert!(!valid_gnome_binding("<Control>V"));
        assert!(!valid_gnome_binding("<Primary><Shift>v"));
        assert!(!valid_gnome_binding("v"));
        assert!(!valid_gnome_binding("<Primary><Alt>v; rm -rf"));
        assert!(!valid_gnome_binding("<Primary><Alt>"));
        assert!(!valid_gnome_binding("<Primary><Alt>ä"));
        assert_eq!(
            canonical_binding("<Control><Alt>V"),
            canonical_binding("<Alt><Primary>v")
        );
        assert_ne!(
            canonical_binding("<Primary><Alt>v"),
            canonical_binding("<Primary><Alt>b")
        );
        assert_eq!(pretty_binding("<Primary><Alt>v"), "Ctrl+Alt+V");
        assert_eq!(pretty_binding("<Super>space"), "Super+Space");
    }

    #[test]
    fn toggle_command_quotes_paths() {
        assert_eq!(toggle_command_for(Path::new("/usr/bin/scribiva")), CMD);
        assert_eq!(
            toggle_command_for(Path::new("/home/a b/it's/scribiva")),
            "'/home/a b/it'\\''s/scribiva' --toggle"
        );
    }

    #[test]
    fn bind_fresh_keeps_other_shortcuts() {
        let f = Fake::with_shortcuts(&[("custom0", "Terminal", "gnome-terminal", "<Super>t")]);
        assert_eq!(
            hotkey_status(&f, CMD),
            HotkeyStatus {
                available: true,
                ..Default::default()
            }
        );
        let r = bind_hotkey(&f, DEFAULT_BINDING, CMD).unwrap();
        assert!(r.replaced.is_empty());
        assert_eq!(f.list(), [format!("{BASE}custom0/"), our_path()]);
        assert_eq!(f.shortcut(&our_path(), "command").as_deref(), Some(CMD));
        assert_eq!(
            f.shortcut(&our_path(), "binding").as_deref(),
            Some(DEFAULT_BINDING)
        );
        assert_eq!(f.shortcut(&our_path(), "name").as_deref(), Some("Scribiva"));
        assert_eq!(
            f.shortcut(&format!("{BASE}custom0/"), "command").as_deref(),
            Some("gnome-terminal")
        );
        let s = hotkey_status(&f, CMD);
        assert!(s.installed && !s.broken);
        assert_eq!(s.label.as_deref(), Some("Ctrl+Alt+V"));
    }

    #[test]
    fn conflict_with_other_shortcut_is_refused() {
        let f = Fake::with_shortcuts(&[(
            "custom0",
            "Clipboard \"tool\"",
            "copyq toggle",
            "<Control><Alt>V",
        )]);
        let err = bind_hotkey(&f, DEFAULT_BINDING, CMD).unwrap_err();
        assert!(err.starts_with("HOTKEY_CONFLICT"), "{err}");
        // Nothing was written.
        assert_eq!(f.list(), [format!("{BASE}custom0/")]);
        assert!(f.shortcut(&our_path(), "command").is_none());
        // Another binding works.
        assert!(bind_hotkey(&f, "<Super><Alt>d", CMD).is_ok());
    }

    #[test]
    fn rebinding_our_own_binding_is_not_a_conflict() {
        let f = Fake::with_shortcuts(&[("scribiva", "Scribiva", CMD, DEFAULT_BINDING)]);
        assert!(hotkey_status(&f, CMD).installed);
        assert!(bind_hotkey(&f, DEFAULT_BINDING, CMD).is_ok());
        assert_eq!(f.list(), [our_path()]);
    }

    #[test]
    fn repair_replaces_legacy_entries_and_keeps_binding() {
        let f = Fake::with_shortcuts(&[
            ("custom0", "Files", "nautilus", "<Super>e"),
            (
                "scribiva-gate1",
                "Scribiva Gate",
                "/home/u/gate/toggle.sh",
                "<Super><Alt>r",
            ),
            (
                "custom3",
                "Old Scribiva",
                "/usr/bin/scribiva-cli toggle",
                "<Super><Alt>r",
            ),
        ]);
        let s = hotkey_status(&f, CMD);
        assert!(s.broken && !s.installed);
        let r = repair_hotkey(&f, CMD).unwrap().unwrap();
        assert_eq!(r.binding, "<Super><Alt>r");
        assert_eq!(
            r.replaced,
            [format!("{BASE}custom3/"), format!("{BASE}scribiva-gate1/")]
        );
        assert_eq!(f.list(), [format!("{BASE}custom0/"), our_path()]);
        assert!(f.shortcut(&format!("{BASE}custom3/"), "command").is_none());
        assert_eq!(
            f.shortcut(&our_path(), "binding").as_deref(),
            Some("<Super><Alt>r")
        );
        assert!(hotkey_status(&f, CMD).installed);
        // Working now: repair is a no-op.
        assert!(repair_hotkey(&f, CMD).is_none());
    }

    #[test]
    fn repair_rewrites_stale_command_in_place() {
        let f = Fake::with_shortcuts(&[(
            "scribiva",
            "Scribiva",
            "/usr/bin/scribiva-cli toggle",
            "<Primary><Alt>b",
        )]);
        let r = repair_hotkey(&f, CMD).unwrap().unwrap();
        assert!(r.replaced.is_empty());
        assert_eq!(f.shortcut(&our_path(), "command").as_deref(), Some(CMD));
        assert_eq!(
            f.shortcut(&our_path(), "binding").as_deref(),
            Some("<Primary><Alt>b")
        );
    }

    #[test]
    fn repair_without_any_shortcut_does_nothing() {
        let f = Fake::with_shortcuts(&[("custom0", "Terminal", "gnome-terminal", "<Super>t")]);
        assert!(repair_hotkey(&f, CMD).is_none());
        assert!(f
            .calls
            .borrow()
            .iter()
            .all(|c| !c.starts_with("gsettings set")));
        let none = Fake {
            no_gsettings: true,
            ..Default::default()
        };
        assert!(repair_hotkey(&none, CMD).is_none());
        assert!(!hotkey_status(&none, CMD).available);
        assert!(bind_hotkey(&none, DEFAULT_BINDING, CMD).is_err());
    }

    #[test]
    fn enable_extension_persists_choice() {
        let f = Fake::with_shortcuts(&[]);
        assert!(!extension_enabled(&f));
        assert!(enable_extension(&f, false, &|| (false, 0)).is_err());
        let s = enable_extension(&f, true, &|| (false, 0)).unwrap();
        assert!(s.enabled && !s.active);
        assert_eq!(s.needs_relogin, Some(true));
        assert_eq!(
            parse_string_list(&f.value(SHELL_SCHEMA, "enabled-extensions").unwrap()).unwrap(),
            [EXTENSION_UUID]
        );
        assert!(
            parse_string_list(&f.value(SHELL_SCHEMA, "disabled-extensions").unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(f
            .calls
            .borrow()
            .iter()
            .any(|c| c == "gnome-extensions enable scribiva@scribiva.app"));
        let s = enable_extension(&f, true, &|| (true, 2)).unwrap();
        assert_eq!(
            (s.active, s.version, s.needs_relogin),
            (true, 2, Some(false))
        );
    }

    #[test]
    fn kwinrc_text_parsing() {
        assert!(kwinrc_enabled(
            "[Plugins]\nblurEnabled=true\nscribiva-focusEnabled=true\n"
        ));
        assert!(!kwinrc_enabled("[Plugins]\nscribiva-focusEnabled=false\n"));
        assert!(!kwinrc_enabled("[Plugins]\nblurEnabled=true\n"));
        // Same key in another group does not count.
        assert!(!kwinrc_enabled(
            "[Other]\nscribiva-focusEnabled=true\n[Plugins]\n"
        ));
        assert!(kwinrc_enabled("[Plugins]\n scribiva-focusEnabled = 1 \n"));
        assert!(!kwinrc_enabled(""));
    }

    #[test]
    fn kwin_script_install_detection() {
        let tmp = std::env::temp_dir().join(format!("scribiva-kwin-{}", std::process::id()));
        let root = tmp.join("scribiva-focus");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(root.join("contents/code")).unwrap();
        let dirs = vec![PathBuf::from("/nonexistent-scribiva"), tmp.clone()];
        assert!(!kwin_script_installed_in(&dirs));
        std::fs::write(root.join("metadata.json"), "{}").unwrap();
        assert!(!kwin_script_installed_in(&dirs));
        std::fs::write(root.join("contents/code/main.js"), "").unwrap();
        assert!(kwin_script_installed_in(&dirs));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn enable_kwin_script_writes_kwinrc_and_loads_live() {
        let f = Fake::default();
        assert!(!kwin_script_enabled(&f));
        f.calls.borrow_mut().clear();
        assert!(enable_kwin_script(&f, true, false, &|| false).is_err());
        assert!(f.calls.borrow().is_empty());
        let s = enable_kwin_script(&f, true, true, &|| true).unwrap();
        assert_eq!(
            s,
            KwinStatus {
                kde: true,
                kwin_script_installed: true,
                kwin_script_enabled: true,
                kwin_focus_active: true,
            }
        );
        assert_eq!(
            f.value("kwinrc:Plugins", "scribiva-focusEnabled")
                .as_deref(),
            Some("true")
        );
        let calls = f.calls.borrow();
        assert!(calls.iter().any(|c| c
            == "kwriteconfig6 --file kwinrc --group Plugins --key scribiva-focusEnabled --type bool true"));
        assert!(calls
            .iter()
            .any(|c| c == "qdbus6 org.kde.KWin /Scripting org.kde.kwin.Scripting.start"));
        // Never touches GNOME settings on KDE.
        assert!(calls.iter().all(|c| !c.starts_with("gsettings")));
    }

    #[test]
    fn enable_kwin_script_falls_back_to_dbus_send() {
        let f = Fake {
            missing: vec!["qdbus6", "qdbus"],
            ..Default::default()
        };
        let s = enable_kwin_script(&f, true, true, &|| false).unwrap();
        assert!(s.kwin_script_enabled && !s.kwin_focus_active);
        assert!(f.calls.borrow().iter().any(|c| c
            == "dbus-send --session --type=method_call --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.start"));
        // No loader tool at all: the choice is still saved.
        let none = Fake {
            missing: vec!["qdbus6", "qdbus", "dbus-send"],
            ..Default::default()
        };
        assert!(
            enable_kwin_script(&none, true, true, &|| false)
                .unwrap()
                .kwin_script_enabled
        );
        // Without kwriteconfig6: a clear manual instruction, nothing written.
        let no_write = Fake {
            missing: vec!["kwriteconfig6"],
            ..Default::default()
        };
        let err = enable_kwin_script(&no_write, true, true, &|| false).unwrap_err();
        assert!(err.contains("KWin Scripts"), "{err}");
        assert!(no_write.calls.borrow().is_empty());
    }
}
