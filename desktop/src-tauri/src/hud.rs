//! Recording HUD (docs/06 §3, FR-002): a small always-on-top, undecorated,
//! click-through webview at the bottom centre of the primary monitor, shown
//! only while recording / transcribing / inserting. It must never take
//! keyboard focus from the editor the user is dictating into.
//!
//! - Windows: created `focusable(false)` (tao sets `WS_EX_NOACTIVATE`) and
//!   `focused(false)`. Tauri's `show()` ends in `ShowWindow(SW_SHOW)`, which
//!   activates, so after creation the window is shown and hidden only with
//!   `ShowWindowAsync(SW_SHOWNOACTIVATE / SW_HIDE)` and kept topmost with
//!   `SetWindowPos(.., SWP_NOACTIVATE)`; `WS_EX_NOACTIVATE` is re-asserted
//!   before every show. It ignores the mouse, so a click can never activate
//!   it either (cancel stays on Esc, the tray and the main window).
//! - Linux X11: GTK `accept_focus(false)` (WM_HINTS input = False) and
//!   `show_all()` without `present()`: window managers do not focus it, and
//!   X11 lets us place it and keep it above.
//! - Linux Wayland (GNOME's default): disabled. A Wayland client cannot
//!   position its window or keep it above others, and GTK 3 has no way to
//!   tell the compositor "do not focus this new toplevel", so no-focus-steal
//!   cannot be guaranteed. The GNOME extension's top-bar indicator and the
//!   tray icon show the state instead (ADR-012).

use crate::core::Phase;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "hud";
const WIDTH: f64 = 320.0;
const HEIGHT: f64 = 58.0;
/// Gap above the taskbar / bottom edge of the work area (logical px).
const MARGIN: f64 = 28.0;

/// Phases during which the HUD is visible.
pub fn wanted(phase: Phase, enabled: bool) -> bool {
    enabled
        && supported()
        && matches!(
            phase,
            Phase::Recording | Phase::Processing | Phase::Injecting
        )
}

/// Whether this platform/session can show the HUD without stealing focus.
pub fn supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::focus::is_x11()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Bottom-centre position (physical px) inside a monitor work area.
pub fn bottom_center(
    work: (i32, i32, u32, u32),
    scale: f64,
    size_logical: (f64, f64),
) -> (i32, i32) {
    let (wx, wy, ww, wh) = work;
    let w = (size_logical.0 * scale).round() as i64;
    let h = (size_logical.1 * scale).round() as i64;
    let margin = (MARGIN * scale).round() as i64;
    let x = i64::from(wx) + (i64::from(ww) - w).max(0) / 2;
    let y = i64::from(wy) + (i64::from(wh) - h - margin).max(0);
    (x as i32, y as i32)
}

/// Create the hidden HUD window once, from Tauri setup (main thread).
pub fn create(app: &AppHandle) {
    if !supported() || app.get_webview_window(LABEL).is_some() {
        return;
    }
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Scribiva HUD")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .focusable(false)
        .shadow(false)
        .visible_on_all_workspaces(true);
    let Ok(window) = builder.build() else {
        return;
    };
    #[cfg(windows)]
    {
        // Purely an indicator: clicks pass through to the window below.
        let _ = window.set_ignore_cursor_events(true);
        if let Ok(hwnd) = window.hwnd() {
            win::remember(hwnd.0 as isize);
        }
    }
    // Linux: click-through is set after the first show, once GTK has
    // realized the window (tao needs its GdkWindow for the input shape).
    #[cfg(not(windows))]
    let _ = window;
}

fn place(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return;
    };
    let wa = monitor.work_area();
    let (x, y) = bottom_center(
        (wa.position.x, wa.position.y, wa.size.width, wa.size.height),
        monitor.scale_factor(),
        (WIDTH, HEIGHT),
    );
    // tao moves with SWP_NOACTIVATE (Windows) / gtk_window_move (X11).
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Show or hide for the current phase. Never focuses anything.
pub fn update(app: &AppHandle, phase: Phase, enabled: bool) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let show = wanted(phase, enabled);
    #[cfg(windows)]
    {
        if show && !win::is_shown() {
            place(app, &window);
        }
        win::set_shown(show);
    }
    #[cfg(not(windows))]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static CLICK_THROUGH: AtomicBool = AtomicBool::new(false);
        let visible = window.is_visible().unwrap_or(false);
        if show && !visible {
            place(app, &window);
            // gtk show_all() only; tao never calls present() here.
            let _ = window.show();
            if !CLICK_THROUGH.swap(true, Ordering::SeqCst) {
                let _ = window.set_ignore_cursor_events(true);
            }
        } else if !show && visible {
            let _ = window.hide();
        }
    }
}

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos,
        ShowWindowAsync, GWL_EXSTYLE, HWND_TOPMOST, LWA_ALPHA, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    };

    static HWND: AtomicIsize = AtomicIsize::new(0);
    static SHOWN: AtomicBool = AtomicBool::new(false);

    pub fn remember(hwnd: isize) {
        HWND.store(hwnd, Ordering::SeqCst);
    }

    pub fn is_shown() -> bool {
        SHOWN.load(Ordering::SeqCst)
    }

    /// Asynchronous, non-activating show/hide: safe from any thread and
    /// never waits for the UI thread. Re-issued on every status change
    /// (idempotent), because tao still believes the window is hidden and may
    /// hide it again when it re-applies its own style flags.
    pub fn set_shown(show: bool) {
        let raw = HWND.load(Ordering::SeqCst);
        let was = SHOWN.swap(show, Ordering::SeqCst);
        if raw == 0 || (!show && !was) {
            return;
        }
        let hwnd = raw as windows_sys::Win32::Foundation::HWND;
        // SAFETY: `hwnd` is the HUD's top-level window, created once and
        // never destroyed while the app runs; these calls only post messages
        // or change style bits on it.
        unsafe {
            if show {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let want = ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize;
                if want != ex {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
                }
                // Click-through adds WS_EX_LAYERED; a layered window without
                // attributes may not paint, so make it explicitly opaque.
                if want & WS_EX_LAYERED as isize != 0 {
                    SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                }
                ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE);
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                );
            } else {
                ShowWindowAsync(hwnd, SW_HIDE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_only_while_busy_and_enabled() {
        let on = supported();
        assert_eq!(wanted(Phase::Recording, true), on);
        assert_eq!(wanted(Phase::Processing, true), on);
        assert_eq!(wanted(Phase::Injecting, true), on);
        assert!(!wanted(Phase::Recording, false));
        for p in [
            Phase::Idle,
            Phase::Ready,
            Phase::Error,
            Phase::SuspectedNoise,
            Phase::Cancelled,
        ] {
            assert!(!wanted(p, true), "{p:?}");
        }
    }

    #[test]
    fn bottom_center_math() {
        // 1920x1040 work area (taskbar below), scale 1.
        let (x, y) = bottom_center((0, 0, 1920, 1040), 1.0, (320.0, 58.0));
        assert_eq!((x, y), (800, 1040 - 58 - 28));
        // Secondary origin and HiDPI.
        let (x, y) = bottom_center((-2560, 100, 2560, 1400), 2.0, (320.0, 58.0));
        assert_eq!(x, -2560 + (2560 - 640) / 2);
        assert_eq!(y, 100 + 1400 - 116 - 56);
        // Tiny work area never goes negative relative to its origin.
        assert_eq!(
            bottom_center((10, 10, 100, 50), 1.0, (320.0, 58.0)),
            (10, 10)
        );
    }
}
