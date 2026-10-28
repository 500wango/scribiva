//! Clipboard access via Win32 (CF_UNICODETEXT only). Restoring the previous
//! clipboard happens only when it was plain text and nobody changed the
//! clipboard after our own write (checked with GetClipboardSequenceNumber).

use std::thread::sleep;
use std::time::Duration;
use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::System::Ole::{CF_LOCALE, CF_OEMTEXT, CF_TEXT, CF_UNICODETEXT};

/// Formats Windows synthesises from CF_UNICODETEXT; a clipboard holding only
/// these is "plain text" and can be restored losslessly.
const TEXT_FORMATS: [u32; 4] = [
    CF_UNICODETEXT as u32,
    CF_TEXT as u32,
    CF_OEMTEXT as u32,
    CF_LOCALE as u32,
];

/// Holds the clipboard open; closes it on drop.
struct Open;

impl Open {
    fn acquire() -> Result<Self, String> {
        // Another app may hold the clipboard briefly; retry a few times.
        for _ in 0..10 {
            // SAFETY: null owner window is allowed.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Open);
            }
            sleep(Duration::from_millis(20));
        }
        Err("CLIPBOARD_BUSY".into())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: we opened the clipboard in `acquire`.
        unsafe { CloseClipboard() };
    }
}

pub fn sequence() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetClipboardSequenceNumber() }
}

/// Plain-text contents of the clipboard, or `None` if it holds anything
/// besides text formats (images, files, rich text...) or is empty.
pub fn read_plain_text() -> Option<String> {
    let _open = Open::acquire().ok()?;
    let mut format = 0u32;
    let mut has_text = false;
    loop {
        // SAFETY: clipboard is open; EnumClipboardFormats walks formats.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if !TEXT_FORMATS.contains(&format) {
            return None;
        }
        has_text |= format == CF_UNICODETEXT as u32;
    }
    // SAFETY: clipboard open.
    if !has_text || unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } == 0 {
        return None;
    }
    // SAFETY: clipboard open; handle owned by the clipboard and only read here.
    unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle.is_null() {
            return None;
        }
        let ptr = GlobalLock(handle as _) as *const u16;
        if ptr.is_null() {
            return None;
        }
        let max = GlobalSize(handle as _) / 2;
        let slice = std::slice::from_raw_parts(ptr, max);
        let len = slice.iter().position(|&c| c == 0).unwrap_or(max);
        let text = String::from_utf16_lossy(&slice[..len]);
        GlobalUnlock(handle as _);
        Some(text)
    }
}

/// Replace the clipboard with `text` as CF_UNICODETEXT. Returns the sequence
/// number right after the write.
pub fn write_text(text: &str) -> Result<u32, String> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes = wide.len() * 2;
    let _open = Open::acquire()?;
    // SAFETY: clipboard open; memory is allocated, filled and handed to the
    // clipboard, which takes ownership on success. Freed on failure.
    unsafe {
        if EmptyClipboard() == 0 {
            return Err("CLIPBOARD_WRITE_FAILED".into());
        }
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if mem.is_null() {
            return Err("CLIPBOARD_WRITE_FAILED".into());
        }
        let dst = GlobalLock(mem) as *mut u16;
        if dst.is_null() {
            GlobalFree(mem);
            return Err("CLIPBOARD_WRITE_FAILED".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
        GlobalUnlock(mem);
        if SetClipboardData(CF_UNICODETEXT as u32, mem as HANDLE).is_null() {
            GlobalFree(mem);
            return Err("CLIPBOARD_WRITE_FAILED".into());
        }
    }
    drop(_open);
    Ok(sequence())
}

/// Restore `previous` after `delay` if the clipboard still holds exactly what
/// we wrote (sequence unchanged). Returns true when restored.
pub fn restore_if_unchanged(previous: &str, our_sequence: u32, delay: Duration) -> bool {
    sleep(delay);
    if sequence() != our_sequence {
        // Someone (user or another app) changed it: never overwrite.
        return false;
    }
    write_text(previous).is_ok()
}
