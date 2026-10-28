//! Foreground-window snapshot (FR-005, ADR-012). Reads only the HWND, the
//! owning process id, its exe file name and integrity level. Window titles
//! and contents are never read.

use crate::focus_policy::{normalize_exe, FocusSnapshot, FocusSource};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// Closes a Win32 handle on drop.
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle was returned by OpenProcess/OpenProcessToken and is owned here.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Mandatory integrity RID of a process token (e.g. 0x2000 medium, 0x3000 high).
fn integrity_of_token(token: HANDLE) -> Option<u32> {
    let mut needed = 0u32;
    // SAFETY: size query with a null buffer is the documented pattern.
    unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed == 0 || needed > 1024 {
        return None;
    }
    // u64 storage keeps the TOKEN_MANDATORY_LABEL pointer field aligned.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: buffer is at least `needed` bytes and suitably aligned.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            buf.as_mut_ptr() as *mut c_void,
            needed,
            &mut needed,
        )
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: on success the buffer holds a TOKEN_MANDATORY_LABEL whose SID points into it.
    unsafe {
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        if sid.is_null() {
            return None;
        }
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, u32::from(count) - 1))
    }
}

fn own_integrity() -> Option<u32> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle; token is written on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Handle(token);
    integrity_of_token(token.0)
}

/// Capture the current foreground window. Returns `None` when there is none.
pub fn snapshot() -> Option<FocusSnapshot> {
    // SAFETY: plain Win32 queries without pointer arguments beyond locals.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: hwnd came from GetForegroundWindow; pid is a local out-param.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == 0 {
        return None;
    }
    // SAFETY: no arguments.
    let own_pid = unsafe { GetCurrentProcessId() };
    let mut exe = String::new();
    let mut elevated = false;
    // SAFETY: limited-information access is allowed for most processes; failure is handled.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        // Access denied for the limited query almost always means a protected or
        // higher-integrity process; treat as elevated (copy only).
        elevated = true;
    } else {
        let process = Handle(process);
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: buf/len describe a valid writable UTF-16 buffer.
        if unsafe {
            QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        } != 0
        {
            exe = normalize_exe(&String::from_utf16_lossy(&buf[..len as usize]));
        }
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: process handle is valid; token written on success.
        if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } != 0 {
            let token = Handle(token);
            if let (Some(theirs), Some(ours)) = (integrity_of_token(token.0), own_integrity()) {
                elevated = theirs > ours;
            }
        } else if pid != own_pid {
            // Medium-integrity callers can open tokens of same-integrity processes;
            // refusal indicates a higher integrity level.
            elevated = true;
        }
    }
    Some(FocusSnapshot {
        source: FocusSource::Win32,
        app_id: String::new(),
        window: hwnd as usize as u64,
        pid,
        exe,
        elevated,
        own_process: pid == own_pid,
    })
}
