//! KDE Plasma Wayland focus source (ADR-012): the Scribiva KWin script
//! (`extensions/kwin/scribiva-focus`) pushes every window activation to us.
//!
//! KWin scripts cannot own a D-Bus name, so the direction is reversed from
//! GNOME: the app owns `app.scribiva.Desktop` on the session bus and serves
//! `app.scribiva.Focus1` at `/app/scribiva/Focus`; the script calls
//! `Activated(s resource_class, s desktop_file, s window_uuid, s pid)`.
//! The pid travels as text because KWin's `callDBus` turns JS numbers into
//! int32/double, never into a uint32.
//!
//! Trust and freshness, so a stale or spoofed focus never allows a paste:
//! - calls are accepted only from the current owner of `org.kde.KWin`;
//! - a stored focus is used only while that same KWin instance still owns
//!   the name and reports the script as loaded;
//! - nothing received since startup = unknown (copy only).
//!
//! Only app ids, WM class, an opaque window UUID and the pid are handled;
//! window titles are never sent or read.

use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub const BUS_NAME: &str = "app.scribiva.Desktop";
pub const OBJECT_PATH: &str = "/app/scribiva/Focus";
pub const INTERFACE: &str = "app.scribiva.Focus1";
/// KPackage id of the script (`kwinrc [Plugins] scribiva-focusEnabled`).
pub const SCRIPT_ID: &str = "scribiva-focus";
pub const KWIN_BUS_NAME: &str = "org.kde.KWin";
pub const SCRIPTING_PATH: &str = "/Scripting";
pub const SCRIPTING_INTERFACE: &str = "org.kde.kwin.Scripting";

/// Freshness checks run right before a paste; KWin answers in milliseconds.
const CHECK_TIMEOUT: Duration = Duration::from_millis(800);
const MAX_FIELD: usize = 200;

// ---------------------------------------------------------------- session

/// Pure session check: `XDG_CURRENT_DESKTOP` lists KDE and the session is
/// Wayland (`XDG_SESSION_TYPE=wayland`, or unset with `WAYLAND_DISPLAY`).
pub fn kde_wayland_from(
    desktop: Option<&str>,
    session: Option<&str>,
    wayland_display: bool,
) -> bool {
    let kde = desktop.is_some_and(|d| d.split(':').any(|p| p.trim().eq_ignore_ascii_case("KDE")));
    let wayland = match session.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.eq_ignore_ascii_case("wayland"),
        None => wayland_display,
    };
    kde && wayland
}

pub fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|d| d.split(':').any(|p| p.trim().eq_ignore_ascii_case("KDE")))
}

pub fn is_kde_wayland() -> bool {
    kde_wayland_from(
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
    )
}

// ---------------------------------------------------------------- data

/// One `Activated` call, validated. Empty `window_uuid` = no usable window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Activation {
    pub resource_class: String,
    pub desktop_file: String,
    /// Normalized KWin `internalId` (lower-case, no braces), or empty.
    pub window_uuid: String,
    pub pid: u32,
}

fn clip(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_FIELD)
        .collect()
}

/// `{8-4-4-4-12}` hex UUID → lower-case without braces. Anything else is
/// rejected: if the script ever sent a constant placeholder, every window
/// would look the same and the focus check would be meaningless.
pub fn normalize_uuid(raw: &str) -> Option<String> {
    let s = raw.trim();
    let s = s
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .unwrap_or(s);
    let groups: Vec<&str> = s.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    let ok = groups.len() == lens.len()
        && groups
            .iter()
            .zip(lens)
            .all(|(g, n)| g.len() == n && g.chars().all(|c| c.is_ascii_hexdigit()));
    let lower = s.to_ascii_lowercase();
    (ok && lower != "00000000-0000-0000-0000-000000000000").then_some(lower)
}

impl Activation {
    pub fn parse(resource_class: &str, desktop_file: &str, window_uuid: &str, pid: &str) -> Self {
        match normalize_uuid(window_uuid) {
            Some(uuid) => Activation {
                resource_class: clip(resource_class),
                desktop_file: clip(desktop_file),
                window_uuid: uuid,
                pid: pid.trim().parse().unwrap_or(0),
            },
            // No window, panel/desktop surface or malformed id: unknown.
            None => Activation::default(),
        }
    }
}

/// Stable 64-bit FNV-1a, so a window keeps the same id for the whole session
/// and across builds (`FocusSnapshot::window` is a u64).
pub fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Window id for a normalized UUID; 0 (no window) only for an empty UUID.
pub fn window_id(uuid: &str) -> u64 {
    if uuid.is_empty() {
        0
    } else {
        fnv1a64(uuid).max(1)
    }
}

#[derive(Debug, Clone)]
struct Latest {
    /// Unique bus name of the KWin connection that sent it.
    sender: String,
    focus: Activation,
}

#[derive(Debug, Default)]
struct State {
    latest: Option<Latest>,
    received: bool,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Accept a call only from the connection that currently owns `org.kde.KWin`.
pub fn accept_sender(sender: &str, kwin_owner: Option<&str>) -> bool {
    !sender.is_empty() && kwin_owner == Some(sender)
}

/// The stored focus, if it came from the KWin instance that owns the name now
/// and the script is still loaded there.
fn fresh(
    latest: Option<&Latest>,
    kwin_owner: Option<&str>,
    loaded: Option<bool>,
) -> Option<Activation> {
    let latest = latest?;
    (kwin_owner == Some(latest.sender.as_str()) && loaded == Some(true))
        .then(|| latest.focus.clone())
}

// ---------------------------------------------------------------- D-Bus

struct FocusIface {
    state: Arc<Mutex<State>>,
    trusted: String,
}

async fn name_owner_async(conn: &zbus::Connection, name: &str) -> Option<String> {
    let msg = conn
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &(name,),
        )
        .await
        .ok()?;
    msg.body().deserialize::<String>().ok()
}

#[zbus::interface(name = "app.scribiva.Focus1")]
impl FocusIface {
    /// Called by the KWin script on load and on every activation.
    async fn activated(
        &self,
        resource_class: String,
        desktop_file: String,
        window_uuid: String,
        pid: String,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let owner = name_owner_async(conn, &self.trusted).await;
        if !accept_sender(&sender, owner.as_deref()) {
            return Err(zbus::fdo::Error::AccessDenied(format!(
                "only {} may report focus",
                self.trusted
            )));
        }
        let focus = Activation::parse(&resource_class, &desktop_file, &window_uuid, &pid);
        let mut g = lock(&self.state);
        g.latest = Some(Latest { sender, focus });
        g.received = true;
        Ok(())
    }

    #[zbus(property)]
    fn version(&self) -> &str {
        "1"
    }
}

/// The exported focus object plus the connection used for freshness checks.
pub struct Server {
    conn: zbus::blocking::Connection,
    state: Arc<Mutex<State>>,
    trusted: String,
}

impl Server {
    /// Own `app.scribiva.Desktop` and serve the focus object. Fails with
    /// `NameTaken` when another Scribiva process already owns the name.
    pub fn start(
        builder: zbus::blocking::connection::Builder<'static>,
        trusted: &str,
    ) -> Result<Server, String> {
        let state = Arc::new(Mutex::new(State::default()));
        let iface = FocusIface {
            state: state.clone(),
            trusted: trusted.to_string(),
        };
        // zbus requests names with AllowReplacement | ReplaceExisting by
        // default: a second process would silently take over the name.
        let conn = builder
            .method_timeout(CHECK_TIMEOUT)
            .allow_name_replacements(false)
            .replace_existing_names(false)
            .name(BUS_NAME)
            .and_then(|b| b.serve_at(OBJECT_PATH, iface))
            .and_then(|b| b.build())
            .map_err(|e| e.to_string())?;
        Ok(Server {
            conn,
            state,
            trusted: trusted.to_string(),
        })
    }

    pub fn received(&self) -> bool {
        lock(&self.state).received
    }

    fn name_owner(&self, name: &str) -> Option<String> {
        self.conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(name,),
            )
            .ok()?
            .body()
            .deserialize::<String>()
            .ok()
    }

    fn scripting(&self, method: &str, body: &(&str,)) -> Option<zbus::Message> {
        self.conn
            .call_method(
                Some(self.trusted.as_str()),
                SCRIPTING_PATH,
                Some(SCRIPTING_INTERFACE),
                method,
                body,
            )
            .ok()
    }

    /// `org.kde.kwin.Scripting.isScriptLoaded("scribiva-focus")`; None when
    /// KWin cannot be asked.
    pub fn script_loaded(&self) -> Option<bool> {
        self.scripting("isScriptLoaded", &(SCRIPT_ID,))?
            .body()
            .deserialize::<bool>()
            .ok()
    }

    /// Latest focus, only if still trustworthy (see module docs).
    pub fn current(&self) -> Option<Activation> {
        let latest = lock(&self.state).latest.clone()?;
        let owner = self.name_owner(&self.trusted)?;
        if owner != latest.sender {
            // KWin restarted (or is gone): the old focus means nothing now.
            return None;
        }
        fresh(Some(&latest), Some(&owner), self.script_loaded())
    }

    /// The script reports the focused window when it loads. If it was loaded
    /// before we owned the name, reload it so the current focus is known
    /// without waiting for the next window switch. Only touches our script.
    pub fn resync(&self) {
        if self.received() || self.script_loaded() != Some(true) {
            return;
        }
        let _ = self.scripting("unloadScript", &(SCRIPT_ID,));
        let _ = self.conn.call_method(
            Some(self.trusted.as_str()),
            SCRIPTING_PATH,
            Some(SCRIPTING_INTERFACE),
            "start",
            &(),
        );
    }
}

static SERVER: OnceLock<Server> = OnceLock::new();

/// Startup wiring: on KDE Plasma Wayland, export the focus object. Blocking;
/// call from a worker thread. Logs and skips when the name is taken.
pub fn start() {
    if !is_kde_wayland() || SERVER.get().is_some() {
        return;
    }
    let builder = match zbus::blocking::connection::Builder::session() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Scribiva: KWin focus: no session bus: {e}");
            return;
        }
    };
    match Server::start(builder, KWIN_BUS_NAME) {
        Ok(server) => SERVER.get_or_init(|| server).resync(),
        Err(e) => {
            eprintln!("Scribiva: KWin focus: cannot own {BUS_NAME} for {INTERFACE}, skipping: {e}")
        }
    }
}

/// Whether any `Activated` call arrived since startup.
pub fn received() -> bool {
    SERVER.get().is_some_and(Server::received)
}

/// Latest trustworthy KWin focus; None = unknown.
pub fn current() -> Option<Activation> {
    SERVER.get()?.current()
}

/// Reload the script if it is loaded but has not reported yet.
pub fn resync() {
    if let Some(s) = SERVER.get() {
        s.resync();
    }
}

/// After enabling the script: wait briefly for its first report.
pub fn wait_received(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if received() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "{3f2504e0-4f89-11d3-9a0c-0305e82c3301}";

    #[test]
    fn session_detection() {
        assert!(kde_wayland_from(Some("KDE"), Some("wayland"), false));
        assert!(kde_wayland_from(Some("ubuntu:KDE"), None, true));
        assert!(!kde_wayland_from(Some("KDE"), Some("x11"), true));
        assert!(!kde_wayland_from(Some("KDE"), None, false));
        assert!(!kde_wayland_from(
            Some("ubuntu:GNOME"),
            Some("wayland"),
            true
        ));
        assert!(!kde_wayland_from(Some("KDEX"), Some("wayland"), true));
        assert!(!kde_wayland_from(None, Some("wayland"), true));
    }

    #[test]
    fn uuid_and_window_ids() {
        assert_eq!(
            normalize_uuid(" {3F2504E0-4F89-11D3-9A0C-0305E82C3301} ").as_deref(),
            Some("3f2504e0-4f89-11d3-9a0c-0305e82c3301")
        );
        assert_eq!(
            normalize_uuid("3f2504e0-4f89-11d3-9a0c-0305e82c3301"),
            normalize_uuid(UUID)
        );
        for bad in [
            "",
            "{}",
            "undefined",
            "42",
            "{00000000-0000-0000-0000-000000000000}",
            "3f2504e0-4f89-11d3-9a0c-0305e82c330",
            "3f2504e0-4f89-11d3-9a0c-0305e82c33zz",
        ] {
            assert!(normalize_uuid(bad).is_none(), "{bad}");
        }
        // FNV-1a reference values: stable across builds and platforms.
        assert_eq!(fnv1a64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(window_id(""), 0);
        let id = window_id("3f2504e0-4f89-11d3-9a0c-0305e82c3301");
        assert_ne!(id, 0);
        assert_ne!(id, window_id("3f2504e0-4f89-11d3-9a0c-0305e82c3302"));
    }

    #[test]
    fn activation_parsing() {
        let a = Activation::parse("kate", "org.kde.kate", UUID, "4242");
        assert_eq!(
            (a.resource_class.as_str(), a.desktop_file.as_str(), a.pid),
            ("kate", "org.kde.kate", 4242)
        );
        assert_eq!(a.window_uuid, "3f2504e0-4f89-11d3-9a0c-0305e82c3301");
        assert_eq!(
            Activation::parse("kate", "x", "", "1"),
            Activation::default()
        );
        assert_eq!(Activation::parse("a", "b", UUID, "-5").pid, 0);
        assert_eq!(Activation::parse("a", "b", UUID, "x").pid, 0);
        let long = "a\u{7}".repeat(500);
        let c = Activation::parse(&long, &long, UUID, "1");
        assert_eq!(c.resource_class.chars().count(), MAX_FIELD);
        assert!(!c.resource_class.chars().any(char::is_control));
    }

    #[test]
    fn trust_and_freshness() {
        assert!(accept_sender(":1.5", Some(":1.5")));
        assert!(!accept_sender(":1.6", Some(":1.5")));
        assert!(!accept_sender(":1.5", None));
        assert!(!accept_sender("", Some("")));
        let latest = Latest {
            sender: ":1.5".into(),
            focus: Activation::parse("kate", "org.kde.kate", UUID, "1"),
        };
        assert!(fresh(Some(&latest), Some(":1.5"), Some(true)).is_some());
        // KWin restarted: new owner, the old focus is void.
        assert!(fresh(Some(&latest), Some(":1.9"), Some(true)).is_none());
        // Script unloaded (disabled) or KWin not answering: unknown.
        assert!(fresh(Some(&latest), Some(":1.5"), Some(false)).is_none());
        assert!(fresh(Some(&latest), Some(":1.5"), None).is_none());
        assert!(fresh(None, Some(":1.5"), Some(true)).is_none());
    }

    /// End to end on a private bus (never the user's session): a fake KWin
    /// owns its name and serves `isScriptLoaded`, then calls `Activated`.
    mod private_bus {
        use super::super::*;
        use std::io::{BufRead, BufReader};
        use std::process::{Child, Command, Stdio};

        struct Daemon(Child);
        impl Drop for Daemon {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        fn daemon() -> Option<(Daemon, String)> {
            let mut child = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let mut line = String::new();
            BufReader::new(child.stdout.take()?)
                .read_line(&mut line)
                .ok()?;
            let addr = line.trim().to_string();
            (!addr.is_empty()).then(|| (Daemon(child), addr))
        }

        struct FakeScripting {
            loaded: Arc<Mutex<bool>>,
        }

        #[zbus::interface(name = "org.kde.kwin.Scripting")]
        impl FakeScripting {
            // KWin's real member name (Qt keeps the camelCase).
            #[zbus(name = "isScriptLoaded")]
            fn is_script_loaded(&self, name: &str) -> bool {
                name == SCRIPT_ID && *self.loaded.lock().unwrap()
            }
        }

        fn bus(addr: &str) -> zbus::blocking::connection::Builder<'static> {
            zbus::blocking::connection::Builder::address(addr.parse::<zbus::Address>().unwrap())
                .unwrap()
        }

        fn activated(
            conn: &zbus::blocking::Connection,
            args: (&str, &str, &str, &str),
        ) -> zbus::Result<zbus::Message> {
            conn.call_method(
                Some(BUS_NAME),
                OBJECT_PATH,
                Some(INTERFACE),
                "Activated",
                &args,
            )
        }

        #[test]
        fn kwin_reports_focus_over_dbus() {
            let Some((_daemon, addr)) = daemon() else {
                eprintln!("dbus-daemon not available; skipping");
                return;
            };
            const FAKE_KWIN: &str = "org.kde.KWinTest";
            let server = Server::start(bus(&addr), FAKE_KWIN).unwrap();
            assert!(!server.received());
            assert!(server.current().is_none());

            // A second process cannot take the name.
            assert!(Server::start(bus(&addr), FAKE_KWIN).is_err());

            let loaded = Arc::new(Mutex::new(true));
            let kwin = bus(&addr)
                .name(FAKE_KWIN)
                .unwrap()
                .serve_at(
                    SCRIPTING_PATH,
                    FakeScripting {
                        loaded: loaded.clone(),
                    },
                )
                .unwrap()
                .build()
                .unwrap();
            let args = (
                "kate",
                "org.kde.kate",
                "{3F2504E0-4F89-11D3-9A0C-0305E82C3301}",
                "77",
            );

            // Anyone else on the bus is refused and changes nothing.
            let intruder = bus(&addr).build().unwrap();
            assert!(activated(&intruder, args).is_err());
            assert!(!server.received());

            activated(&kwin, args).unwrap();
            assert!(server.received());
            let a = server.current().unwrap();
            assert_eq!((a.desktop_file.as_str(), a.pid), ("org.kde.kate", 77));
            assert_eq!(a.window_uuid, "3f2504e0-4f89-11d3-9a0c-0305e82c3301");

            // Script unloaded in KWin: unknown again.
            *loaded.lock().unwrap() = false;
            assert!(server.current().is_none());
            *loaded.lock().unwrap() = true;

            // No active window: stored, but no usable target.
            activated(&kwin, ("", "", "", "")).unwrap();
            assert_eq!(server.current(), Some(Activation::default()));

            // KWin gone: the stored focus is void.
            activated(&kwin, args).unwrap();
            drop(kwin);
            std::thread::sleep(Duration::from_millis(100));
            assert!(server.current().is_none());
        }
    }
}
