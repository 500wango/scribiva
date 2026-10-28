//! Linux delivery (FR-005, FR-017, ADR-012), port of `engine.deliver`.
//!
//! Order: auto-insert off → copy only; unknown focus → copy only (unless the
//! user enabled `insert_without_focus_check`); own window → copy only; GNOME
//! extension v2 → `InsertText` inside the compositor; otherwise re-check the
//! focus and use the clipboard + one paste chord. Suspected noise never gets
//! here: the state machine keeps it as a recovery-only result. One dispatch,
//! never retried, never followed by Enter.

use super::clipboard::{self, PasteError};
use super::focus::{is_own_window, is_terminal, same_target};
use super::shell::{self, InsertError, InsertOptions};
use crate::core::InsertOutcome;
use crate::focus_policy::{FocusSnapshot, FocusSource};
use crate::storage::Settings;
use std::time::Duration;

/// Wait before restoring the previous clipboard (target apps read it asynchronously).
const RESTORE_DELAY: Duration = Duration::from_millis(800);
/// Let the clipboard owner settle before sending the chord.
const PASTE_SETTLE: Duration = Duration::from_millis(120);

/// Side effects used by delivery; faked in tests.
pub trait Io {
    fn copy(&self, text: &str) -> Result<(), String>;
    fn read_plain(&self) -> Option<String>;
    fn focus(&self) -> Option<FocusSnapshot>;
    fn extension_version(&self) -> u32;
    fn insert_text(
        &self,
        text: &str,
        window: u64,
        opts: &InsertOptions<'_>,
    ) -> Result<String, InsertError>;
    fn paste(&self, terminal: bool) -> Result<(), PasteError>;
    fn sleep(&self, d: Duration);
}

pub struct System;

impl Io for System {
    fn copy(&self, text: &str) -> Result<(), String> {
        clipboard::write_text(text)
    }
    fn read_plain(&self) -> Option<String> {
        clipboard::read_plain_text()
    }
    fn focus(&self) -> Option<FocusSnapshot> {
        super::focus::snapshot()
    }
    fn extension_version(&self) -> u32 {
        shell::version()
    }
    fn insert_text(
        &self,
        text: &str,
        window: u64,
        opts: &InsertOptions<'_>,
    ) -> Result<String, InsertError> {
        shell::insert_text(text, window, opts)
    }
    fn paste(&self, terminal: bool) -> Result<(), PasteError> {
        clipboard::send_paste(terminal)
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

pub type Delivery = (InsertOutcome, Option<String>);

pub fn deliver(
    text: &str,
    at_stop: Option<&FocusSnapshot>,
    settings: &Settings,
    is_current: &dyn Fn() -> bool,
) -> Delivery {
    deliver_with(&System, text, at_stop, settings, is_current)
}

/// What the extension's `InsertText` outcome means for the user, and
/// whether the text still has to be put on the clipboard.
pub fn map_insert_outcome(outcome: &str) -> (InsertOutcome, Option<&'static str>, bool) {
    match outcome {
        "committed" => (InsertOutcome::Inserted, None, false),
        "pasted" => (InsertOutcome::DispatchedUnverified, None, false),
        "focus_changed" | "no_focus" => (InsertOutcome::FocusChanged, Some("focus_changed"), true),
        "no_text_input" => (InsertOutcome::ClipboardOnly, Some("no_text_input"), true),
        "busy" => (InsertOutcome::ClipboardOnly, Some("insert_busy"), true),
        _ => (InsertOutcome::ClipboardOnly, Some("insert_failed"), true),
    }
}

/// Dictated text is data: drop control characters except newline and tab,
/// never produce a carriage return, and for terminals collapse line breaks
/// so a paste cannot run a command (mirrors the extension's `sanitize`).
pub fn sanitize(text: &str, single_line: bool) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let kept: String = text
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    if !single_line {
        return kept;
    }
    let mut out = String::with_capacity(kept.len());
    let mut in_break = false;
    for c in kept.chars() {
        if c == '\n' || c == '\t' {
            if !in_break {
                out.push(' ');
            }
            in_break = true;
        } else {
            out.push(c);
            in_break = false;
        }
    }
    out
}

fn copy_only(io: &dyn Io, text: &str, outcome: InsertOutcome, reason: &str) -> Delivery {
    match io.copy(text) {
        Ok(()) => (outcome, Some(reason.to_string())),
        Err(e) => (InsertOutcome::ClipboardOnly, Some(e)),
    }
}

pub fn deliver_with(
    io: &dyn Io,
    text: &str,
    at_stop: Option<&FocusSnapshot>,
    settings: &Settings,
    is_current: &dyn Fn() -> bool,
) -> Delivery {
    if !settings.auto_insert {
        return copy_only(io, text, InsertOutcome::ClipboardOnly, "auto_insert_off");
    }
    let Some(target) = at_stop else {
        if settings.insert_without_focus_check {
            if !is_current() {
                return (InsertOutcome::ClipboardOnly, Some("cancelled".into()));
            }
            return paste_via_clipboard(
                io,
                &sanitize(text, false),
                false,
                settings.restore_text_clipboard,
            );
        }
        return copy_only(io, text, InsertOutcome::ClipboardOnly, "focus_unknown");
    };
    if is_own_window(target) {
        return copy_only(io, text, InsertOutcome::ClipboardOnly, "own_window");
    }
    let terminal = is_terminal(target);
    // Last-moment cancellation check before anything reaches the target.
    if !is_current() {
        return (InsertOutcome::ClipboardOnly, Some("cancelled".into()));
    }
    if target.source == FocusSource::GnomeExtension && io.extension_version() >= 2 {
        let opts = InsertOptions {
            single_line: terminal,
            paste_shortcut: if terminal { "ctrl+shift+v" } else { "ctrl+v" },
            restore_clipboard: settings.restore_text_clipboard,
        };
        match io.insert_text(text, target.window, &opts) {
            Ok(outcome) => {
                let (result, reason, copy) = map_insert_outcome(&outcome);
                if copy {
                    return copy_only(io, text, result, reason.unwrap_or("insert_failed"));
                }
                return (result, reason.map(str::to_string));
            }
            // The extension may have typed already: report it, never retry.
            Err(InsertError::Uncertain) => {
                return (
                    InsertOutcome::DispatchedUnverified,
                    Some("insert_uncertain".into()),
                );
            }
            // Not on the bus: nothing was sent, the client-side path is safe.
            Err(InsertError::Absent) => {}
        }
    }
    if !same_target(target, io.focus().as_ref()) {
        return copy_only(io, text, InsertOutcome::FocusChanged, "focus_changed");
    }
    paste_via_clipboard(
        io,
        &sanitize(text, terminal),
        terminal,
        settings.restore_text_clipboard,
    )
}

/// Clipboard + one paste chord, then the guarded restore: only when the
/// previous clipboard was plain text and still holds our text afterwards.
fn paste_via_clipboard(io: &dyn Io, text: &str, terminal: bool, restore: bool) -> Delivery {
    let previous = if restore { io.read_plain() } else { None };
    if let Err(e) = io.copy(text) {
        return (InsertOutcome::ClipboardOnly, Some(e));
    }
    io.sleep(PASTE_SETTLE);
    match io.paste(terminal) {
        Ok(()) => {
            if let Some(prev) = previous.filter(|p| p != text) {
                io.sleep(RESTORE_DELAY);
                if io.read_plain().as_deref() == Some(text) {
                    let _ = io.copy(&prev);
                }
            }
            (InsertOutcome::DispatchedUnverified, None)
        }
        Err(PasteError::Uncertain) => (
            InsertOutcome::DispatchedUnverified,
            Some("paste_uncertain".into()),
        ),
        Err(PasteError::Unavailable) => (
            InsertOutcome::ClipboardOnly,
            Some("paste_unavailable".into()),
        ),
        Err(PasteError::Failed) => (InsertOutcome::ClipboardOnly, Some("paste_failed".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::focus::test_snap as snap;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        clipboard: RefCell<Option<String>>,
        /// Clipboard the user (or another app) sets right after our paste.
        changed_after_paste: Option<String>,
        focus: Option<FocusSnapshot>,
        version: u32,
        insert: Option<Result<String, InsertError>>,
        paste_result: Option<Result<(), PasteError>>,
        log: RefCell<Vec<String>>,
    }

    impl Io for Fake {
        fn copy(&self, text: &str) -> Result<(), String> {
            self.log.borrow_mut().push(format!("copy:{text}"));
            *self.clipboard.borrow_mut() = Some(text.to_string());
            Ok(())
        }
        fn read_plain(&self) -> Option<String> {
            self.clipboard.borrow().clone()
        }
        fn focus(&self) -> Option<FocusSnapshot> {
            self.log.borrow_mut().push("focus".into());
            self.focus.clone()
        }
        fn extension_version(&self) -> u32 {
            self.version
        }
        fn insert_text(
            &self,
            text: &str,
            window: u64,
            o: &InsertOptions<'_>,
        ) -> Result<String, InsertError> {
            self.log.borrow_mut().push(format!(
                "insert:{text}:{window}:{}:{}:{}",
                o.single_line, o.paste_shortcut, o.restore_clipboard
            ));
            self.insert.clone().unwrap_or(Err(InsertError::Absent))
        }
        fn paste(&self, terminal: bool) -> Result<(), PasteError> {
            self.log.borrow_mut().push(format!("paste:{terminal}"));
            if let Some(c) = &self.changed_after_paste {
                *self.clipboard.borrow_mut() = Some(c.clone());
            }
            self.paste_result.unwrap_or(Ok(()))
        }
        fn sleep(&self, _d: Duration) {}
    }

    impl Fake {
        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    fn settings() -> Settings {
        Settings::default()
    }
    fn yes() -> bool {
        true
    }

    fn gnome(app: &str) -> FocusSnapshot {
        snap(FocusSource::GnomeExtension, app, "cls", 7)
    }

    #[test]
    fn insert_outcome_mapping() {
        assert_eq!(
            map_insert_outcome("committed"),
            (InsertOutcome::Inserted, None, false)
        );
        assert_eq!(
            map_insert_outcome("pasted"),
            (InsertOutcome::DispatchedUnverified, None, false)
        );
        for o in ["focus_changed", "no_focus"] {
            assert_eq!(
                map_insert_outcome(o),
                (InsertOutcome::FocusChanged, Some("focus_changed"), true)
            );
        }
        for o in [
            "no_text_input",
            "busy",
            "invalid",
            "failed",
            "something-new",
        ] {
            let (outcome, reason, copy) = map_insert_outcome(o);
            assert_eq!(outcome, InsertOutcome::ClipboardOnly, "{o}");
            assert!(reason.is_some() && copy, "{o}");
        }
        assert_eq!(
            serde_json::to_string(&InsertOutcome::Inserted).unwrap(),
            "\"inserted\""
        );
    }

    #[test]
    fn auto_insert_off_copies_only() {
        let io = Fake::default();
        let mut s = settings();
        s.auto_insert = false;
        let t = gnome("a.desktop");
        let r = deliver_with(&io, "hi", Some(&t), &s, &yes);
        assert_eq!(
            r,
            (InsertOutcome::ClipboardOnly, Some("auto_insert_off".into()))
        );
        assert_eq!(io.log(), ["copy:hi"]);
    }

    #[test]
    fn unknown_focus_copies_unless_opted_in() {
        let io = Fake::default();
        let r = deliver_with(&io, "hi", None, &settings(), &yes);
        assert_eq!(
            r,
            (InsertOutcome::ClipboardOnly, Some("focus_unknown".into()))
        );
        assert_eq!(io.log(), ["copy:hi"]);

        let io = Fake::default();
        let mut s = settings();
        s.insert_without_focus_check = true;
        let r = deliver_with(&io, "hi", None, &s, &yes);
        assert_eq!(r, (InsertOutcome::DispatchedUnverified, None));
        assert_eq!(io.log(), ["copy:hi", "paste:false"]);
    }

    #[test]
    fn own_window_copies_only() {
        let io = Fake {
            version: 2,
            ..Default::default()
        };
        let t = gnome("scribiva.desktop");
        let r = deliver_with(&io, "hi", Some(&t), &settings(), &yes);
        assert_eq!(r, (InsertOutcome::ClipboardOnly, Some("own_window".into())));
        assert_eq!(io.log(), ["copy:hi"]);
    }

    #[test]
    fn extension_commit_and_paste() {
        let t = gnome("org.gnome.TextEditor.desktop");
        let io = Fake {
            version: 2,
            insert: Some(Ok("committed".into())),
            ..Default::default()
        };
        let r = deliver_with(&io, "a\nb", Some(&t), &settings(), &yes);
        assert_eq!(r, (InsertOutcome::Inserted, None));
        assert_eq!(io.log(), ["insert:a\nb:7:false:ctrl+v:true"]);

        let io = Fake {
            version: 2,
            insert: Some(Ok("pasted".into())),
            ..Default::default()
        };
        let r = deliver_with(&io, "x", Some(&t), &settings(), &yes);
        assert_eq!(r, (InsertOutcome::DispatchedUnverified, None));
        // No client-side paste, no clipboard write: one dispatch only.
        assert_eq!(io.log().len(), 1);
    }

    #[test]
    fn extension_terminal_is_single_line_with_shift() {
        let t = gnome("org.gnome.Terminal.desktop");
        let io = Fake {
            version: 2,
            insert: Some(Ok("committed".into())),
            ..Default::default()
        };
        let mut s = settings();
        s.restore_text_clipboard = false;
        deliver_with(&io, "ls", Some(&t), &s, &yes);
        assert_eq!(io.log(), ["insert:ls:7:true:ctrl+shift+v:false"]);
    }

    #[test]
    fn extension_refusals_copy_text() {
        let t = gnome("a.desktop");
        for (outcome, expect) in [
            ("focus_changed", InsertOutcome::FocusChanged),
            ("no_focus", InsertOutcome::FocusChanged),
            ("busy", InsertOutcome::ClipboardOnly),
            ("failed", InsertOutcome::ClipboardOnly),
        ] {
            let io = Fake {
                version: 2,
                insert: Some(Ok(outcome.into())),
                ..Default::default()
            };
            let r = deliver_with(&io, "hi", Some(&t), &settings(), &yes);
            assert_eq!(r.0, expect, "{outcome}");
            assert_eq!(io.log().last().unwrap(), "copy:hi", "{outcome}");
            assert!(
                !io.log().iter().any(|l| l.starts_with("paste")),
                "{outcome}"
            );
        }
    }

    #[test]
    fn extension_uncertain_is_never_retried() {
        let t = gnome("a.desktop");
        let io = Fake {
            version: 2,
            insert: Some(Err(InsertError::Uncertain)),
            focus: Some(t.clone()),
            ..Default::default()
        };
        let r = deliver_with(&io, "hi", Some(&t), &settings(), &yes);
        assert_eq!(r.0, InsertOutcome::DispatchedUnverified);
        assert_eq!(io.log().len(), 1);
    }

    #[test]
    fn old_extension_falls_back_with_focus_recheck() {
        let t = gnome("a.desktop");
        // Version 1: no InsertText; same focus → clipboard + Ctrl+V.
        let io = Fake {
            version: 1,
            focus: Some(t.clone()),
            ..Default::default()
        };
        let r = deliver_with(&io, "hi", Some(&t), &settings(), &yes);
        assert_eq!(r, (InsertOutcome::DispatchedUnverified, None));
        assert_eq!(io.log(), ["focus", "copy:hi", "paste:false"]);

        // Focus moved → copy only.
        let io = Fake {
            version: 1,
            focus: Some(gnome("b.desktop")),
            ..Default::default()
        };
        let r = deliver_with(&io, "hi", Some(&t), &settings(), &yes);
        assert_eq!(
            r,
            (InsertOutcome::FocusChanged, Some("focus_changed".into()))
        );
        assert_eq!(io.log(), ["focus", "copy:hi"]);
    }

    #[test]
    fn x11_terminal_fallback_is_single_line_with_shift() {
        let t = snap(
            FocusSource::X11,
            "Gnome-terminal",
            "gnome-terminal-server",
            9,
        );
        let io = Fake {
            version: 2, // ignored: focus did not come from the extension
            focus: Some(t.clone()),
            ..Default::default()
        };
        let r = deliver_with(&io, "echo a\r\nrm x\n", Some(&t), &settings(), &yes);
        assert_eq!(r.0, InsertOutcome::DispatchedUnverified);
        assert_eq!(io.log(), ["focus", "copy:echo a rm x ", "paste:true"]);
    }

    #[test]
    fn clipboard_restore_is_guarded() {
        let t = snap(FocusSource::X11, "firefox", "Navigator", 3);
        // Previous plain text restored when the clipboard still holds our text.
        let io = Fake {
            clipboard: RefCell::new(Some("old".into())),
            focus: Some(t.clone()),
            ..Default::default()
        };
        deliver_with(&io, "new", Some(&t), &settings(), &yes);
        assert_eq!(io.log(), ["focus", "copy:new", "paste:false", "copy:old"]);
        // The user copied something else meanwhile: never overwrite it.
        let io = Fake {
            clipboard: RefCell::new(Some("old".into())),
            changed_after_paste: Some("user".into()),
            focus: Some(t.clone()),
            ..Default::default()
        };
        deliver_with(&io, "new", Some(&t), &settings(), &yes);
        assert_eq!(io.clipboard.borrow().as_deref(), Some("user"));
        // Non-text previous clipboard (read_plain = None): nothing restored.
        let io = Fake {
            focus: Some(t.clone()),
            ..Default::default()
        };
        deliver_with(&io, "new", Some(&t), &settings(), &yes);
        assert_eq!(io.log(), ["focus", "copy:new", "paste:false"]);
    }

    #[test]
    fn paste_errors() {
        let t = snap(FocusSource::X11, "firefox", "Navigator", 3);
        for (err, outcome, reason) in [
            (
                PasteError::Unavailable,
                InsertOutcome::ClipboardOnly,
                "paste_unavailable",
            ),
            (
                PasteError::Failed,
                InsertOutcome::ClipboardOnly,
                "paste_failed",
            ),
            (
                PasteError::Uncertain,
                InsertOutcome::DispatchedUnverified,
                "paste_uncertain",
            ),
        ] {
            let io = Fake {
                focus: Some(t.clone()),
                paste_result: Some(Err(err)),
                ..Default::default()
            };
            let r = deliver_with(&io, "x", Some(&t), &settings(), &yes);
            assert_eq!(r, (outcome, Some(reason.to_string())));
            assert_eq!(
                io.log().iter().filter(|l| l.starts_with("paste")).count(),
                1
            );
        }
    }

    #[test]
    fn cancelled_job_sends_nothing() {
        let t = gnome("a.desktop");
        let io = Fake {
            version: 2,
            insert: Some(Ok("committed".into())),
            ..Default::default()
        };
        let r = deliver_with(&io, "x", Some(&t), &settings(), &|| false);
        assert_eq!(r, (InsertOutcome::ClipboardOnly, Some("cancelled".into())));
        assert!(io.log().is_empty());
    }

    #[test]
    fn sanitize_rules() {
        assert_eq!(sanitize("a\r\nb\u{7}c", false), "a\nbc");
        assert_eq!(sanitize("a\n\n\tb\n", true), "a b ");
        assert_eq!(sanitize("中文\u{1b}[31m", false), "中文[31m");
    }

    #[test]
    fn suspected_noise_never_reaches_delivery() {
        let mut core = crate::core::Core::new();
        core.start_recording("j".into(), 0).unwrap();
        core.stop_recording("j", 5_000);
        let action = core.result_arrived("j", "Thank you.".into(), String::new(), vec![], true);
        assert_eq!(action, crate::core::ResultAction::KeepOnly);
    }
}
