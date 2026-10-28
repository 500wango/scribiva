//! Platform-independent focus-guard policy (FR-005, ADR-012).
//!
//! The platform adapter captures a `FocusSnapshot` at stop-recording and
//! again right before injection; this module decides what is allowed on
//! Windows (`decide`); the Linux rules live in `linux::focus`. Window titles
//! are never read or stored, only window ids and app identifiers.

use serde::Serialize;

/// Where a snapshot came from. Snapshots from different sources are never
/// the same target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
// Each OS constructs only its own variants.
#[allow(dead_code)]
pub enum FocusSource {
    Win32,
    /// Scribiva GNOME Shell extension (`app.scribiva.Shell1.GetFocus`).
    GnomeExtension,
    /// `_NET_ACTIVE_WINDOW` via xprop.
    X11,
    /// Scribiva KWin script (`app.scribiva.Focus1.Activated`).
    KWin,
}

/// What the focus adapter could observe about the foreground window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FocusSnapshot {
    pub source: FocusSource,
    /// Opaque window id (HWND, Mutter window id or X11 window). 0 = none.
    pub window: u64,
    /// Owning process id (0 when unknown, e.g. some Wayland clients).
    pub pid: u32,
    /// Windows: executable file name only (e.g. `notepad.exe`), never a full
    /// path or title. Linux: the WM class.
    pub exe: String,
    /// Linux: GNOME app id (`org.gnome.Terminal.desktop`) or X11 WM_CLASS
    /// class name. Empty on Windows.
    pub app_id: String,
    /// The target runs at a higher integrity level than we do (UIPI blocks input).
    pub elevated: bool,
    /// The window belongs to this app (our own settings window).
    pub own_process: bool,
}

/// Outcome of comparing the stop-time snapshot with the pre-injection one.
/// Windows rules; pure logic, so it is compiled and tested on every OS.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusDecision {
    /// Same window, normal target: paste allowed.
    Inject,
    /// The window or process changed between stop and injection.
    Changed,
    /// No trustworthy foreground information.
    Unknown,
    /// Terminal-class target: copy only, never send a paste shortcut.
    Terminal,
    /// Higher-integrity (elevated) target: UIPI would drop our input.
    Elevated,
    /// The target is Scribiva itself.
    OwnWindow,
    /// Desktop shell surfaces (taskbar, Start, search): there is no text field
    /// to paste into, typically because recording was stopped from the tray.
    Shell,
}

#[cfg_attr(not(windows), allow(unused_imports))]
pub use win::*;

#[cfg_attr(not(windows), allow(dead_code))]
mod win {
    use super::{FocusDecision, FocusSnapshot};

    /// Exe names treated as terminals. Pasting into a shell can execute commands,
    /// so these are always copy-only. Compared case-insensitively.
    const TERMINAL_EXES: &[&str] = &[
        "windowsterminal.exe",
        "wt.exe",
        "openconsole.exe",
        "conhost.exe",
        "cmd.exe",
        "powershell.exe",
        "powershell_ise.exe",
        "pwsh.exe",
        "wezterm-gui.exe",
        "wezterm.exe",
        "alacritty.exe",
        "mintty.exe",
        "kitty.exe",
        "putty.exe",
        "kitty_portable.exe",
        "mobaxterm.exe",
        "conemu.exe",
        "conemu64.exe",
        "cmder.exe",
        "hyper.exe",
        "tabby.exe",
        "wsl.exe",
        "bash.exe",
        "ubuntu.exe",
    ];

    /// Shell processes that own the taskbar, Start menu and search flyouts.
    const SHELL_EXES: &[&str] = &[
        "explorer.exe",
        "shellexperiencehost.exe",
        "startmenuexperiencehost.exe",
        "searchhost.exe",
        "searchapp.exe",
        "lockapp.exe",
    ];

    /// Strip any directory component and lower-case the exe name.
    pub fn normalize_exe(path_or_name: &str) -> String {
        path_or_name
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    }

    pub fn is_terminal_exe(exe: &str) -> bool {
        let name = normalize_exe(exe);
        TERMINAL_EXES.contains(&name.as_str())
    }

    pub fn is_shell_exe(exe: &str) -> bool {
        let name = normalize_exe(exe);
        SHELL_EXES.contains(&name.as_str())
    }

    /// Decide whether the prepared text may be pasted into the current foreground
    /// window. `at_stop` is the snapshot taken when recording stopped.
    pub fn decide(at_stop: Option<&FocusSnapshot>, now: Option<&FocusSnapshot>) -> FocusDecision {
        let (Some(stop), Some(now)) = (at_stop, now) else {
            return FocusDecision::Unknown;
        };
        if stop.window == 0 || now.window == 0 || stop.pid == 0 || now.pid == 0 {
            return FocusDecision::Unknown;
        }
        if stop.window != now.window
            || stop.pid != now.pid
            || normalize_exe(&stop.exe) != normalize_exe(&now.exe)
        {
            return FocusDecision::Changed;
        }
        if now.own_process {
            return FocusDecision::OwnWindow;
        }
        if is_terminal_exe(&now.exe) {
            return FocusDecision::Terminal;
        }
        // Elevated processes may refuse even the exe-name query, so this is
        // checked before the "no exe name" case to give the right explanation.
        if now.elevated || stop.elevated {
            return FocusDecision::Elevated;
        }
        if now.exe.is_empty() {
            return FocusDecision::Unknown;
        }
        if is_shell_exe(&now.exe) {
            return FocusDecision::Shell;
        }
        FocusDecision::Inject
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(window: u64, pid: u32, exe: &str) -> FocusSnapshot {
        FocusSnapshot {
            source: FocusSource::Win32,
            app_id: String::new(),
            window,
            pid,
            exe: exe.into(),
            elevated: false,
            own_process: false,
        }
    }

    #[test]
    fn same_window_allows_injection() {
        let a = snap(0x1234, 10, "Obsidian.exe");
        assert_eq!(decide(Some(&a), Some(&a.clone())), FocusDecision::Inject);
    }

    #[test]
    fn exe_comparison_is_case_insensitive() {
        let a = snap(1, 10, "Code.exe");
        let b = snap(1, 10, "code.EXE");
        assert_eq!(decide(Some(&a), Some(&b)), FocusDecision::Inject);
    }

    #[test]
    fn different_window_or_process_is_changed() {
        let a = snap(1, 10, "notepad.exe");
        assert_eq!(
            decide(Some(&a), Some(&snap(2, 10, "notepad.exe"))),
            FocusDecision::Changed
        );
        assert_eq!(
            decide(Some(&a), Some(&snap(1, 11, "notepad.exe"))),
            FocusDecision::Changed
        );
        // HWND reuse by a different exe must not pass.
        assert_eq!(
            decide(Some(&a), Some(&snap(1, 10, "chrome.exe"))),
            FocusDecision::Changed
        );
    }

    #[test]
    fn missing_information_is_unknown() {
        let a = snap(1, 10, "notepad.exe");
        assert_eq!(decide(None, Some(&a)), FocusDecision::Unknown);
        assert_eq!(decide(Some(&a), None), FocusDecision::Unknown);
        assert_eq!(
            decide(Some(&snap(0, 0, "")), Some(&snap(0, 0, ""))),
            FocusDecision::Unknown
        );
        assert_eq!(
            decide(Some(&snap(1, 10, "")), Some(&snap(1, 10, ""))),
            FocusDecision::Unknown
        );
    }

    #[test]
    fn terminals_are_copy_only() {
        for exe in [
            "WindowsTerminal.exe",
            "cmd.exe",
            "powershell.exe",
            "pwsh.exe",
            "conhost.exe",
            "wezterm-gui.exe",
            "alacritty.exe",
            "mintty.exe",
            r"C:\Program Files\Git\usr\bin\mintty.exe",
        ] {
            assert!(is_terminal_exe(exe), "{exe}");
            let s = snap(5, 1, exe);
            assert_eq!(decide(Some(&s), Some(&s)), FocusDecision::Terminal, "{exe}");
        }
        assert!(!is_terminal_exe("Code.exe"));
        assert!(!is_terminal_exe("notepad.exe"));
        assert!(!is_terminal_exe("cmd.exe.bak"));
    }

    #[test]
    fn elevated_targets_are_copy_only() {
        let mut s = snap(5, 1, "regedit.exe");
        s.elevated = true;
        assert_eq!(decide(Some(&s), Some(&s)), FocusDecision::Elevated);
        // Exe name unreadable because the target is elevated.
        let mut hidden = snap(5, 1, "");
        hidden.elevated = true;
        assert_eq!(
            decide(Some(&hidden), Some(&hidden)),
            FocusDecision::Elevated
        );
    }

    #[test]
    fn own_window_is_never_pasted_into() {
        let mut s = snap(5, 1, "scribiva-desktop.exe");
        s.own_process = true;
        assert_eq!(decide(Some(&s), Some(&s)), FocusDecision::OwnWindow);
    }

    #[test]
    fn shell_surfaces_are_copy_only() {
        let s = snap(9, 4, "explorer.exe");
        assert_eq!(decide(Some(&s), Some(&s)), FocusDecision::Shell);
        assert!(is_shell_exe("SearchHost.exe"));
    }

    #[test]
    fn normalize_strips_paths() {
        assert_eq!(normalize_exe(r"C:\Windows\System32\CMD.EXE"), "cmd.exe");
        assert_eq!(normalize_exe("/usr/bin/foo"), "foo");
        assert_eq!(normalize_exe(""), "");
    }
}
