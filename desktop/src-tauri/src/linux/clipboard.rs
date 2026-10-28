//! Clipboard and paste chord for the Linux fallback path (no GNOME
//! extension v2): `wl-copy`/`wl-paste` + eitype on Wayland, `xclip` +
//! `xdotool` on X11. Text goes through stdin only. The paste chord is
//! dispatched once and never retried.

use super::focus::is_x11;
use super::proc;
use std::path::PathBuf;
use std::time::Duration;

const TEXT_MIME: &str = "text/plain;charset=utf-8";
/// Previous clipboard larger than this is not restored.
const MAX_RESTORE: usize = 2_000_000;
/// Bundled by the .deb (Apache-2.0, license in /usr/share/doc/scribiva/).
pub const BUNDLED_EITYPE: &str = "/usr/lib/scribiva/bin/eitype";

pub fn write_text(text: &str) -> Result<(), String> {
    let t = Duration::from_secs(5);
    // No capture: both tools fork a background owner that inherits stdout.
    let out = if is_x11() {
        proc::run(
            "xclip",
            &["-selection", "clipboard", "-in"],
            Some(text.as_bytes()),
            t,
            false,
        )
    } else {
        proc::run(
            "wl-copy",
            &["--type", TEXT_MIME],
            Some(text.as_bytes()),
            t,
            false,
        )
    };
    match out {
        Ok(o) if o.success => Ok(()),
        Ok(_) => Err("CLIPBOARD_WRITE_FAILED".into()),
        Err(e) => Err(format!("CLIPBOARD_WRITE_FAILED: {e}")),
    }
}

/// X11 meta targets that accompany any selection.
const X11_META: &[&str] = &[
    "TARGETS",
    "TIMESTAMP",
    "MULTIPLE",
    "SAVE_TARGETS",
    "DELETE",
    "INCR",
];
const X11_TEXT: &[&str] = &["UTF8_STRING", "STRING", "TEXT", "COMPOUND_TEXT"];

/// True when every offered type is a text type, so restoring only the plain
/// text loses nothing (images, files or app-private formats are never "restored").
pub fn is_plain_text_types(types: &str) -> bool {
    let mut any_text = false;
    for t in types.lines().map(str::trim).filter(|t| !t.is_empty()) {
        if X11_META.contains(&t) {
            continue;
        }
        let base = t.split(';').next().unwrap_or("");
        if X11_TEXT.contains(&t) || base == "text/plain" {
            any_text = true;
            continue;
        }
        // text/html, text/uri-list, images, app formats: richer than what we restore.
        return false;
    }
    any_text
}

/// Plain-text clipboard contents, or None when it is empty, not plain text,
/// too large or unreadable.
pub fn read_plain_text() -> Option<String> {
    let t = Duration::from_secs(2);
    let (program, list, read): (&str, &[&str], &[&str]) = if is_x11() {
        (
            "xclip",
            &["-o", "-selection", "clipboard", "-t", "TARGETS"],
            &["-o", "-selection", "clipboard", "-t", "UTF8_STRING"],
        )
    } else {
        (
            "wl-paste",
            &["--list-types"],
            &["--no-newline", "--type", TEXT_MIME],
        )
    };
    let types = proc::run_ok(program, list, t).ok()?;
    if !is_plain_text_types(&types) {
        return None;
    }
    let out = proc::run(program, read, None, t, true).ok()?;
    let bytes = out.success.then_some(out.stdout)?;
    if bytes.len() > MAX_RESTORE {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// eitype: bundled copy, then the developer install, then PATH.
pub fn eitype_path() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(BUNDLED_EITYPE)];
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".cargo/bin/eitype"));
    }
    candidates
        .into_iter()
        .find(|p| proc::is_executable(p))
        .or_else(|| proc::which("eitype"))
}

/// argv for one paste chord (Ctrl+V, or Ctrl+Shift+V for terminals).
pub fn paste_args(x11: bool, terminal: bool) -> Vec<&'static str> {
    match (x11, terminal) {
        (true, false) => vec!["key", "--clearmodifiers", "ctrl+v"],
        (true, true) => vec!["key", "--clearmodifiers", "ctrl+shift+v"],
        (false, false) => vec!["-M", "ctrl", "v"],
        (false, true) => vec!["-M", "ctrl", "-M", "shift", "v"],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteError {
    /// No tool to send keys with (eitype/xdotool missing): nothing was sent.
    Unavailable,
    /// The tool exited with an error before sending: nothing was sent.
    Failed,
    /// Timed out: keys may have been sent. Never retry.
    Uncertain,
}

/// Send one paste chord. The caller must never retry, whatever the result.
pub fn send_paste(terminal: bool) -> Result<(), PasteError> {
    let x11 = is_x11();
    let program = if x11 {
        proc::which("xdotool").ok_or(PasteError::Unavailable)?
    } else {
        eitype_path().ok_or(PasteError::Unavailable)?
    };
    let program = program.to_string_lossy().into_owned();
    // eitype may wait for the RemoteDesktop portal consent on first use.
    match proc::run(
        &program,
        &paste_args(x11, terminal),
        None,
        Duration::from_secs(12),
        false,
    ) {
        Ok(out) if out.success => Ok(()),
        Ok(_) => Err(PasteError::Failed),
        Err(e) if e.contains("timed out") => Err(PasteError::Uncertain),
        Err(_) => Err(PasteError::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_detection() {
        assert!(is_plain_text_types(
            "text/plain;charset=utf-8\ntext/plain\nUTF8_STRING\nSTRING\nTEXT\n"
        ));
        assert!(is_plain_text_types("TARGETS\nTIMESTAMP\nUTF8_STRING\n"));
        assert!(!is_plain_text_types("image/png\n"));
        assert!(!is_plain_text_types("text/plain\nimage/png\n"));
        assert!(!is_plain_text_types("text/html\ntext/plain\n"));
        assert!(!is_plain_text_types(
            "text/uri-list\nx-special/gnome-copied-files\n"
        ));
        assert!(!is_plain_text_types(""));
        assert!(!is_plain_text_types("TARGETS\n"));
    }

    #[test]
    fn paste_chords() {
        assert_eq!(paste_args(false, false), ["-M", "ctrl", "v"]);
        assert_eq!(paste_args(false, true), ["-M", "ctrl", "-M", "shift", "v"]);
        assert_eq!(
            paste_args(true, false),
            ["key", "--clearmodifiers", "ctrl+v"]
        );
        assert_eq!(
            paste_args(true, true),
            ["key", "--clearmodifiers", "ctrl+shift+v"]
        );
    }
}
