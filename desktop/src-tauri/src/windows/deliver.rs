//! Windows delivery: focus check + clipboard + single Ctrl+V (unchanged
//! behaviour, moved out of the orchestrator).

use super::{clipboard, focus, inject};
use crate::core::InsertOutcome;
use crate::focus_policy::{self, FocusDecision, FocusSnapshot};
use crate::storage::Settings;
use std::time::Duration;

/// Returns the outcome and an optional reason code for the UI.
pub fn deliver(
    text: &str,
    at_stop: Option<&FocusSnapshot>,
    settings: &Settings,
    is_current: &dyn Fn() -> bool,
) -> (InsertOutcome, Option<String>) {
    if !settings.auto_insert {
        return match clipboard::write_text(text) {
            Ok(_) => (InsertOutcome::ClipboardOnly, Some("auto_insert_off".into())),
            Err(e) => (InsertOutcome::ClipboardOnly, Some(e)),
        };
    }
    let now = focus::snapshot();
    let decision = focus_policy::decide(at_stop, now.as_ref());
    let copy_only = |reason: &str| -> (InsertOutcome, Option<String>) {
        let _ = clipboard::write_text(text);
        let outcome = if reason == "focus_changed" {
            InsertOutcome::FocusChanged
        } else {
            InsertOutcome::ClipboardOnly
        };
        (outcome, Some(reason.to_string()))
    };
    match decision {
        FocusDecision::Inject => {}
        FocusDecision::Changed => return copy_only("focus_changed"),
        FocusDecision::Unknown => return copy_only("focus_unknown"),
        FocusDecision::Terminal => return copy_only("terminal"),
        FocusDecision::Elevated => return copy_only("elevated"),
        FocusDecision::OwnWindow => return copy_only("own_window"),
        FocusDecision::Shell => return copy_only("shell"),
    }
    // Last-moment cancellation check right before touching the clipboard.
    if !is_current() {
        return (InsertOutcome::ClipboardOnly, Some("cancelled".into()));
    }
    let previous = if settings.restore_text_clipboard {
        clipboard::read_plain_text()
    } else {
        None
    };
    let seq = match clipboard::write_text(text) {
        Ok(seq) => seq,
        Err(e) => return (InsertOutcome::ClipboardOnly, Some(e)),
    };
    // Re-check focus once more right before sending the keys.
    if focus_policy::decide(at_stop, focus::snapshot().as_ref()) != FocusDecision::Inject {
        return (InsertOutcome::FocusChanged, Some("focus_changed".into()));
    }
    match inject::send_paste() {
        Ok(()) => {
            if let Some(prev) = previous {
                clipboard::restore_if_unchanged(&prev, seq, Duration::from_millis(800));
            }
            (InsertOutcome::DispatchedUnverified, None)
        }
        Err(inject::PasteError::ModifiersHeld) => {
            (InsertOutcome::ClipboardOnly, Some("modifiers_held".into()))
        }
        Err(inject::PasteError::Blocked) => {
            (InsertOutcome::ClipboardOnly, Some("input_blocked".into()))
        }
    }
}
