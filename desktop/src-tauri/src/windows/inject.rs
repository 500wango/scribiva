//! Paste dispatch: one Ctrl+V via SendInput. Never retried automatically and
//! never followed by Enter. The result is at best `dispatched_unverified`.

use std::thread::sleep;
use std::time::{Duration, Instant};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    VK_V,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteError {
    /// The user is still holding modifiers; sending Ctrl+V now could become
    /// another chord (e.g. Ctrl+Alt+V = our own hotkey). Nothing was sent.
    ModifiersHeld,
    /// SendInput inserted no events (UIPI or desktop switch). Nothing was sent.
    Blocked,
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: simple query; the high bit reports the current key state.
    (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8000) != 0
}

fn modifiers_held() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .into_iter()
        .any(key_down)
}

/// Wait (bounded) until the user released all modifier keys.
fn wait_for_modifiers(timeout: Duration) -> bool {
    let start = Instant::now();
    while modifiers_held() {
        if start.elapsed() >= timeout {
            return false;
        }
        sleep(Duration::from_millis(15));
    }
    true
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    // SAFETY: MapVirtualKeyW is a pure lookup.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Send a single Ctrl+V to the foreground window. On `Ok`, the keystrokes may
/// have been delivered; the caller must report `dispatched_unverified` and
/// must not send again.
pub fn send_paste() -> Result<(), PasteError> {
    if !wait_for_modifiers(Duration::from_millis(1500)) {
        return Err(PasteError::ModifiersHeld);
    }
    let inputs = [
        key(VK_CONTROL, false),
        key(VK_V, false),
        key(VK_V, true),
        key(VK_CONTROL, true),
    ];
    // SAFETY: `inputs` is a valid array of INPUT with the correct element size.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent == 0 {
        return Err(PasteError::Blocked);
    }
    if (sent as usize) < inputs.len() {
        // Partially inserted: make sure Ctrl is not left logically pressed.
        // Some events may have been delivered, so this still counts as dispatched.
        let release = [key(VK_V, true), key(VK_CONTROL, true)];
        // SAFETY: as above.
        unsafe {
            SendInput(
                release.len() as u32,
                release.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
    }
    Ok(())
}
