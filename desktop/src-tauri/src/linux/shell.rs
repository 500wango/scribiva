//! Client for the Scribiva GNOME Shell extension on the session bus
//! (`extensions/gnome-shell/scribiva@scribiva.app`, ADR-012):
//! bus `app.scribiva.Shell`, path `/app/scribiva/Shell`, iface
//! `app.scribiva.Shell1`. Blocking zbus calls; callers run on worker threads.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

pub const BUS_NAME: &str = "app.scribiva.Shell";
pub const OBJECT_PATH: &str = "/app/scribiva/Shell";
pub const INTERFACE: &str = "app.scribiva.Shell1";
/// Secret Service (gnome-keyring, KeePassXC...), checked by diagnostics.
pub const SECRETS_BUS_NAME: &str = "org.freedesktop.secrets";

/// Extension-side wait for InsertText: settle + paste + clipboard restore.
const INSERT_TIMEOUT: Duration = Duration::from_secs(8);
const QUICK_TIMEOUT: Duration = Duration::from_secs(2);

static SHARED: Mutex<Option<Connection>> = Mutex::new(None);

fn connect(timeout: Duration) -> Result<Connection, String> {
    zbus::blocking::connection::Builder::session()
        .map_err(|e| e.to_string())?
        .method_timeout(timeout)
        .build()
        .map_err(|e| e.to_string())
}

/// Shared connection for quick calls; reconnects after a failure.
fn shared() -> Result<Connection, String> {
    let mut g = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = g.as_ref() {
        return Ok(c.clone());
    }
    let c = connect(QUICK_TIMEOUT)?;
    *g = Some(c.clone());
    Ok(c)
}

fn reset_shared() {
    *SHARED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Raw `GetFocus` reply: (app_id, wm_class, window_id, pid).
pub type FocusReply = (String, String, u64, u32);

pub fn get_focus() -> Option<FocusReply> {
    let conn = shared().ok()?;
    match conn.call_method(
        Some(BUS_NAME),
        OBJECT_PATH,
        Some(INTERFACE),
        "GetFocus",
        &(),
    ) {
        Ok(msg) => msg.body().deserialize::<FocusReply>().ok(),
        Err(e) => {
            if !is_absent(&e) {
                reset_shared();
            }
            None
        }
    }
}

/// Interface version of the running extension, 0 when unavailable.
pub fn version() -> u32 {
    let Ok(conn) = shared() else {
        return 0;
    };
    let reply = conn.call_method(
        Some(BUS_NAME),
        OBJECT_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(INTERFACE, "Version"),
    );
    let Ok(msg) = reply else {
        return 0;
    };
    let Ok(value) = msg.body().deserialize::<OwnedValue>() else {
        return 0;
    };
    parse_version(&value)
}

fn parse_version(value: &Value<'_>) -> u32 {
    match value {
        Value::Str(s) => s.as_str().trim().parse().unwrap_or(0),
        Value::U32(n) => *n,
        Value::Value(inner) => parse_version(inner),
        _ => 0,
    }
}

pub const LOADER_PATH: &str = "/app/scribiva/Loader";
pub const LOADER_INTERFACE: &str = "app.scribiva.Loader1";

/// Loader state: which implementation is running and which is installed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LoaderState {
    pub loader: bool,
    pub running: String,
    pub available: String,
}

fn loader_property(conn: &Connection, name: &str) -> Option<String> {
    let msg = conn
        .call_method(
            Some(BUS_NAME),
            LOADER_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(LOADER_INTERFACE, name),
        )
        .ok()?;
    let value = msg.body().deserialize::<OwnedValue>().ok()?;
    string_value(&value)
}

fn string_value(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.as_str().trim().to_string()),
        Value::Value(inner) => string_value(inner),
        _ => None,
    }
}

/// `None` when no extension is on the bus. `loader: false` means an extension
/// from before the hot-reload loader is running (one logout upgrades it).
pub fn loader_state() -> Option<LoaderState> {
    let conn = shared().ok()?;
    match loader_property(&conn, "LoaderVersion") {
        Some(_) => Some(LoaderState {
            loader: true,
            running: loader_property(&conn, "ImplVersion").unwrap_or_default(),
            available: loader_property(&conn, "AvailableImplVersion").unwrap_or_default(),
        }),
        None if version() > 0 => Some(LoaderState {
            loader: false,
            running: String::new(),
            available: String::new(),
        }),
        None => None,
    }
}

/// Ask the running loader to import the installed implementation, without a
/// logout. Returns the implementation version now running.
pub fn reload_implementation() -> Result<String, String> {
    let conn = connect(Duration::from_secs(10))?;
    let msg = conn
        .call_method(
            Some(BUS_NAME),
            LOADER_PATH,
            Some(LOADER_INTERFACE),
            "Reload",
            &(),
        )
        .map_err(|e| e.to_string())?;
    reset_shared();
    msg.body()
        .deserialize::<String>()
        .map_err(|e| e.to_string())
}

/// What the app should do about the extension after start or an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionAction {
    /// Nothing on the bus (not installed, disabled, or never loaded).
    None,
    /// Running implementation differs from the installed one: hot reload.
    Reload,
    /// Up to date.
    Current,
    /// Pre-loader extension running: only a logout can replace it.
    NeedsLogout,
}

pub fn extension_action(state: Option<&LoaderState>) -> ExtensionAction {
    match state {
        None => ExtensionAction::None,
        Some(s) if !s.loader => ExtensionAction::NeedsLogout,
        Some(s) if !s.available.is_empty() && s.running != s.available => ExtensionAction::Reload,
        Some(_) => ExtensionAction::Current,
    }
}

/// Bring the running extension up to the installed implementation when that
/// is possible without a logout. Returns the resulting action for diagnostics.
pub fn sync_extension() -> ExtensionAction {
    let state = loader_state();
    let action = extension_action(state.as_ref());
    if action == ExtensionAction::Reload {
        if let Err(e) = reload_implementation() {
            eprintln!("Scribiva: extension reload failed: {e}");
        }
    }
    action
}

/// Best-effort top-bar indicator: `idle|recording|processing|error`.
pub fn set_state(phase: &str) {
    if let Ok(conn) = shared() {
        let _ = conn.call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "SetState",
            &(phase,),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertError {
    /// The extension is not on the bus (not loaded, older shell): nothing
    /// was sent, a client-side fallback is safe.
    Absent,
    /// The call was sent but no usable reply came back (timeout, crash):
    /// text may already have been typed, so the caller must not retry.
    Uncertain,
}

pub struct InsertOptions<'a> {
    pub single_line: bool,
    pub paste_shortcut: &'a str,
    pub restore_clipboard: bool,
}

/// `InsertText(text, expected_window_id, options) -> outcome`.
pub fn insert_text(
    text: &str,
    window_id: u64,
    opts: &InsertOptions<'_>,
) -> Result<String, InsertError> {
    // Dedicated connection: this call may legitimately take seconds.
    let conn = connect(INSERT_TIMEOUT).map_err(|_| InsertError::Absent)?;
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("single_line", Value::from(opts.single_line));
    options.insert("paste_shortcut", Value::from(opts.paste_shortcut));
    options.insert("restore_clipboard", Value::from(opts.restore_clipboard));
    options.insert("paste_fallback", Value::from(true));
    match conn.call_method(
        Some(BUS_NAME),
        OBJECT_PATH,
        Some(INTERFACE),
        "InsertText",
        &(text, window_id, options),
    ) {
        Ok(msg) => msg
            .body()
            .deserialize::<String>()
            .map_err(|_| InsertError::Uncertain),
        Err(e) if is_absent(&e) => Err(InsertError::Absent),
        Err(_) => Err(InsertError::Uncertain),
    }
}

/// Errors that prove the call never reached an implementation.
fn is_absent(e: &zbus::Error) -> bool {
    match e {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
        ),
        _ => false,
    }
}

/// Whether `name` currently has an owner on the session bus.
pub fn name_has_owner(name: &str) -> bool {
    let Ok(conn) = shared() else {
        return false;
    };
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(name,),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

/// Whether `name` can be started by D-Bus activation.
pub fn name_activatable(name: &str) -> bool {
    let Ok(conn) = shared() else {
        return false;
    };
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "ListActivatableNames",
        &(),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<Vec<String>>().ok())
    .is_some_and(|names| names.iter().any(|n| n == name))
}

pub fn extension_active() -> bool {
    name_has_owner(BUS_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_actions() {
        use super::{extension_action, ExtensionAction, LoaderState};
        let st = |loader: bool, running: &str, available: &str| LoaderState {
            loader,
            running: running.into(),
            available: available.into(),
        };
        assert_eq!(extension_action(None), ExtensionAction::None);
        assert_eq!(
            extension_action(Some(&st(false, "", ""))),
            ExtensionAction::NeedsLogout
        );
        assert_eq!(
            extension_action(Some(&st(true, "aaa", "bbb"))),
            ExtensionAction::Reload
        );
        assert_eq!(
            extension_action(Some(&st(true, "bbb", "bbb"))),
            ExtensionAction::Current
        );
        // Source checkout: no installed version file, dev implementation runs.
        assert_eq!(
            extension_action(Some(&st(true, "dev-1", ""))),
            ExtensionAction::Current
        );
    }

    #[test]
    fn version_values() {
        assert_eq!(parse_version(&Value::from("2")), 2);
        assert_eq!(parse_version(&Value::from(" 1 ")), 1);
        assert_eq!(parse_version(&Value::from("x")), 0);
        assert_eq!(parse_version(&Value::Value(Box::new(Value::from("3")))), 3);
        assert_eq!(parse_version(&Value::from(true)), 0);
    }
}
