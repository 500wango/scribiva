//! Focused-window identity for the Linux focus guard (ADR-012), port of
//! `src/scribiva/focus.py`. Sources:
//! - X11 session (`XDG_SESSION_TYPE=x11`): `_NET_ACTIVE_WINDOW` via xprop.
//! - KDE Plasma Wayland: the Scribiva KWin script, which pushes activations
//!   to `linux::kwin` (no InsertText there; delivery re-checks this snapshot
//!   right before the clipboard + paste chord).
//! - Otherwise (GNOME Wayland): the Scribiva GNOME Shell extension.
//!
//! No source = `None` (unknown): the caller copies instead of pasting.
//! Window titles are never read.

use super::{kwin, proc, shell};
use crate::focus_policy::{FocusSnapshot, FocusSource};
use std::time::Duration;

/// Matched case-insensitively against app id and WM class. Terminals get
/// single-line text and Ctrl+Shift+V, never Enter (FR-005).
const TERMINAL_PATTERNS: &[&str] = &[
    "terminal",
    "konsole",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "tilix",
    "terminator",
    "xterm",
    "urxvt",
    "rxvt",
    "guake",
    "yakuake",
    "ptyxis",
    "blackbox",
    "contour",
    "st-256color",
    "termite",
    "sakura",
    "qterminal",
    "lxterminal",
    "mate-terminal",
    "xfce4-terminal",
    "ghostty",
    "warp",
];

/// Linux focus sources. `crate::focus_policy::FocusSource` has no KWin
/// variant yet (that file is shared with Windows), so the mapping to it lives
/// here, in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Gnome,
    KWin,
    X11,
}

impl Source {
    pub fn policy(self) -> FocusSource {
        match self {
            Source::Gnome => FocusSource::GnomeExtension,
            Source::X11 => FocusSource::X11,
            Source::KWin => FocusSource::KWin,
        }
    }
}

pub fn is_x11() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s.eq_ignore_ascii_case("x11"))
}

pub fn is_terminal(target: &FocusSnapshot) -> bool {
    let ids = format!("{} {}", target.app_id, target.exe).to_lowercase();
    TERMINAL_PATTERNS.iter().any(|p| ids.contains(p))
}

/// Scribiva's own window: clicking its record button focuses it, and the
/// result must not be typed back into Scribiva itself.
pub fn is_own_window(target: &FocusSnapshot) -> bool {
    let ids = format!("{} {}", target.app_id, target.exe).to_lowercase();
    target.own_process || ids.contains("scribiva")
}

/// Same source, same window, same app: the target did not change.
pub fn same_target(before: &FocusSnapshot, after: Option<&FocusSnapshot>) -> bool {
    after.is_some_and(|a| {
        a.source == before.source
            && a.window == before.window
            && a.window != 0
            && a.app_id == before.app_id
            && a.exe == before.exe
    })
}

fn snapshot_of(
    source: Source,
    app_id: String,
    wm_class: String,
    window: u64,
    pid: u32,
) -> Option<FocusSnapshot> {
    if window == 0 {
        return None;
    }
    let clip = |s: String| s.chars().take(200).collect::<String>();
    Some(FocusSnapshot {
        source: source.policy(),
        window,
        pid,
        own_process: pid != 0 && pid == std::process::id(),
        exe: clip(wm_class),
        app_id: clip(app_id),
        elevated: false,
    })
}

pub fn from_gnome(reply: shell::FocusReply) -> Option<FocusSnapshot> {
    let (app_id, wm_class, window, pid) = reply;
    snapshot_of(Source::Gnome, app_id, wm_class, window, pid)
}

/// KWin activation → snapshot: app id is the desktop file name (as GNOME's
/// app id), falling back to the resource class; the WM class slot holds the
/// resource class; the window id is a stable hash of KWin's window UUID.
pub fn from_kwin(a: kwin::Activation) -> Option<FocusSnapshot> {
    let window = kwin::window_id(&a.window_uuid);
    let app_id = if a.desktop_file.is_empty() {
        a.resource_class.clone()
    } else {
        a.desktop_file
    };
    snapshot_of(Source::KWin, app_id, a.resource_class, window, a.pid)
}

/// `_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00007` → `0x3a00007`.
pub fn parse_active_window(out: &str) -> Option<String> {
    let (_, rest) = out.split_once("window id #")?;
    let id = rest.split_whitespace().next()?.trim_end_matches(',');
    let hex = id.strip_prefix("0x")?;
    let value = u64::from_str_radix(hex, 16).ok()?;
    (value != 0).then(|| format!("0x{value:x}"))
}

/// `WM_CLASS(STRING) = "instance", "Class"` and `_NET_WM_PID(CARDINAL) = 1234`
/// → (class as app id, instance as wm class, pid).
pub fn parse_window_props(out: &str) -> (String, String, u32) {
    let mut instance = String::new();
    let mut class = String::new();
    let mut pid = 0;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("WM_CLASS(STRING) =") {
            let parts: Vec<String> = rest
                .split(',')
                .map(|p| p.trim().trim_matches('"').to_string())
                .collect();
            instance = parts.first().cloned().unwrap_or_default();
            class = parts.last().cloned().unwrap_or_default();
        } else if let Some(rest) = line.strip_prefix("_NET_WM_PID(CARDINAL) =") {
            pid = rest.trim().parse().unwrap_or(0);
        }
    }
    (class, instance, pid)
}

fn x11_snapshot() -> Option<FocusSnapshot> {
    std::env::var_os("DISPLAY")?;
    let t = Duration::from_millis(1500);
    let root = proc::run_ok("xprop", &["-root", "_NET_ACTIVE_WINDOW"], t).ok()?;
    let id = parse_active_window(&root)?;
    let props =
        proc::run_ok("xprop", &["-id", &id, "WM_CLASS", "_NET_WM_PID"], t).unwrap_or_default();
    let (class, instance, pid) = parse_window_props(&props);
    let window = u64::from_str_radix(id.trim_start_matches("0x"), 16).ok()?;
    snapshot_of(Source::X11, class, instance, window, pid)
}

/// Identity of the focused window, or None when it cannot be determined.
pub fn snapshot() -> Option<FocusSnapshot> {
    if is_x11() {
        x11_snapshot()
    } else if kwin::is_kde_wayland() {
        kwin::current().and_then(from_kwin)
    } else {
        shell::get_focus().and_then(from_gnome)
    }
}

/// Snapshot fixture shared by the Linux tests.
#[cfg(test)]
pub fn test_snap(source: FocusSource, app_id: &str, wm_class: &str, window: u64) -> FocusSnapshot {
    FocusSnapshot {
        source,
        window,
        pid: 4242,
        exe: wm_class.into(),
        app_id: app_id.into(),
        elevated: false,
        own_process: false,
    }
}

#[cfg(test)]
mod tests {
    use super::test_snap as snap;
    use super::*;

    #[test]
    fn xprop_parsing() {
        assert_eq!(
            parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x3a00007").as_deref(),
            Some("0x3a00007")
        );
        assert_eq!(
            parse_active_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0"),
            None
        );
        assert_eq!(parse_active_window("_NET_ACTIVE_WINDOW:  not found."), None);
        assert_eq!(parse_active_window("window id # 0xZZ"), None);
        let (class, instance, pid) =
            parse_window_props("WM_CLASS(STRING) = \"gnome-terminal-server\", \"Gnome-terminal\"\n_NET_WM_PID(CARDINAL) = 3141\n");
        assert_eq!(
            (class.as_str(), instance.as_str(), pid),
            ("Gnome-terminal", "gnome-terminal-server", 3141)
        );
        let (class, _, pid) =
            parse_window_props("WM_CLASS:  not found.\n_NET_WM_PID:  not found.\n");
        assert_eq!((class.as_str(), pid), ("", 0));
    }

    #[test]
    fn gnome_reply_mapping() {
        assert!(from_gnome((String::new(), String::new(), 0, 0)).is_none());
        let s = from_gnome((
            "org.gnome.TextEditor.desktop".into(),
            "org.gnome.TextEditor".into(),
            77,
            9,
        ))
        .unwrap();
        assert_eq!(s.source, FocusSource::GnomeExtension);
        assert_eq!((s.window, s.pid), (77, 9));
        assert!(!is_terminal(&s) && !is_own_window(&s));
        let own = from_gnome((String::new(), "x".into(), 5, std::process::id())).unwrap();
        assert!(own.own_process && is_own_window(&own));
    }

    #[test]
    fn kwin_activation_mapping() {
        let uuid = "{3F2504E0-4F89-11D3-9A0C-0305E82C3301}";
        assert!(from_kwin(kwin::Activation::parse("", "", "", "0")).is_none());
        assert!(from_kwin(kwin::Activation::parse(
            "kate",
            "org.kde.kate",
            "bogus",
            "9"
        ))
        .is_none());
        let s = from_kwin(kwin::Activation::parse("kate", "org.kde.kate", uuid, "9")).unwrap();
        assert_eq!(s.source, Source::KWin.policy());
        assert_ne!(s.source, FocusSource::GnomeExtension);
        assert_ne!(s.source, FocusSource::X11);
        assert_eq!(
            (s.app_id.as_str(), s.exe.as_str(), s.pid),
            ("org.kde.kate", "kate", 9)
        );
        assert_eq!(
            s.window,
            kwin::window_id("3f2504e0-4f89-11d3-9a0c-0305e82c3301")
        );
        assert!(!is_terminal(&s) && !is_own_window(&s));
        // Same window reported again (braces or case differ): same target.
        let again = from_kwin(kwin::Activation::parse(
            "kate",
            "org.kde.kate",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "9",
        ));
        assert!(same_target(&s, again.as_ref()));
        let other = from_kwin(kwin::Activation::parse(
            "kate",
            "org.kde.kate",
            "{3F2504E0-4F89-11D3-9A0C-0305E82C3302}",
            "9",
        ));
        assert!(!same_target(&s, other.as_ref()));
        // No desktop file: the resource class is the app id.
        let k = from_kwin(kwin::Activation::parse("konsole", "", uuid, "0")).unwrap();
        assert_eq!(k.app_id, "konsole");
        assert!(is_terminal(&k));
        let y = from_kwin(kwin::Activation::parse(
            "yakuake",
            "org.kde.yakuake",
            uuid,
            "1",
        ))
        .unwrap();
        assert!(is_terminal(&y));
        let own = from_kwin(kwin::Activation::parse("scribiva", "scribiva", uuid, "1")).unwrap();
        assert!(is_own_window(&own));
        let own_pid = from_kwin(kwin::Activation::parse(
            "x",
            "",
            uuid,
            &std::process::id().to_string(),
        ))
        .unwrap();
        assert!(own_pid.own_process && is_own_window(&own_pid));
    }

    #[test]
    fn terminal_and_own_window_detection() {
        for (app, class) in [
            ("org.gnome.Terminal.desktop", "gnome-terminal-server"),
            ("org.gnome.Ptyxis.desktop", "org.gnome.Ptyxis"),
            ("", "kitty"),
            ("com.mitchellh.ghostty.desktop", ""),
            ("", "XTerm"),
        ] {
            assert!(
                is_terminal(&snap(FocusSource::GnomeExtension, app, class, 1)),
                "{app} {class}"
            );
        }
        assert!(!is_terminal(&snap(
            FocusSource::X11,
            "firefox",
            "Navigator",
            1
        )));
        assert!(is_own_window(&snap(
            FocusSource::GnomeExtension,
            "scribiva.desktop",
            "",
            1
        )));
        assert!(is_own_window(&snap(FocusSource::X11, "", "Scribiva", 1)));
    }

    #[test]
    fn same_target_rules() {
        let a = snap(FocusSource::GnomeExtension, "a.desktop", "a", 1);
        assert!(same_target(&a, Some(&a.clone())));
        assert!(!same_target(&a, None));
        assert!(!same_target(
            &a,
            Some(&snap(FocusSource::GnomeExtension, "a.desktop", "a", 2))
        ));
        assert!(!same_target(
            &a,
            Some(&snap(FocusSource::GnomeExtension, "b.desktop", "a", 1))
        ));
        assert!(!same_target(
            &a,
            Some(&snap(FocusSource::X11, "a.desktop", "a", 1))
        ));
    }
}
