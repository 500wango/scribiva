//! Focused-window identity for the Linux focus guard (ADR-012), port of
//! `src/scribiva/focus.py`. Sources:
//! - X11 session (`XDG_SESSION_TYPE=x11`): `_NET_ACTIVE_WINDOW` via xprop.
//! - Otherwise (GNOME Wayland): the Scribiva GNOME Shell extension.
//!
//! No source = `None` (unknown): the caller copies instead of pasting.
//! Window titles are never read.

use super::{proc, shell};
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
    source: FocusSource,
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
        source,
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
    snapshot_of(FocusSource::GnomeExtension, app_id, wm_class, window, pid)
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
    snapshot_of(FocusSource::X11, class, instance, window, pid)
}

/// Identity of the focused window, or None when it cannot be determined.
pub fn snapshot() -> Option<FocusSnapshot> {
    if is_x11() {
        x11_snapshot()
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
