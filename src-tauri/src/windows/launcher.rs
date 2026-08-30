//! Windows implementation of the ZCode launcher process operations.
//!
//! - `find_pids`: one `CreateToolhelp32Snapshot` per call (on-demand only —
//!   the launcher never busy-polls).
//! - `focus_window`: `EnumWindows` → match owner pid → restore + foreground
//!   via the `AttachThreadInput` dance (foreground rights).
//! - `spawn`: `CreateProcessW` (GUI exe, no console).
//! - `exe_version`: version resource via `GetFileVersionInfoW`.

#![cfg(windows)]

use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, CreateProcessW, GetCurrentThreadId, GetProcessId,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, SW_RESTORE, SW_SHOW,
    WS_EX_TOOLWINDOW,
};

use crate::providers::zlauncher::{process_match, ProcessOps};

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct WinProcOps;

impl ProcessOps for WinProcOps {
    fn find_pids(&self, exe: &Path) -> Vec<u32> {
        snapshot_pids()
            .into_iter()
            .filter(|(_, name)| process_match(exe, name))
            .map(|(pid, _)| pid)
            .collect()
    }

    fn focus_window(&self, pids: &[u32]) -> bool {
        focus_windows(pids)
    }

    fn spawn(&self, exe: &Path) -> Result<u32, String> {
        let path = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
        let exe_wide = to_wide(&path.to_string_lossy());
        let mut cmdline = to_wide(&format!("\"{}\"", path.to_string_lossy()));
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let result = unsafe {
            CreateProcessW(
                PCWSTR(exe_wide.as_ptr()),
                PWSTR(cmdline.as_mut_ptr()),
                None,
                None,
                false,
                CREATE_UNICODE_ENVIRONMENT,
                None,
                None,
                &si,
                &mut pi,
            )
        };
        match result {
            Ok(()) => {
                let pid = unsafe { GetProcessId(pi.hProcess) };
                unsafe {
                    let _ = CloseHandle(pi.hProcess);
                    let _ = CloseHandle(pi.hThread);
                }
                Ok(pid)
            }
            Err(e) => Err(format!("CreateProcessW 失败: {e}")),
        }
    }

    fn exe_version(&self, exe: &Path) -> Option<String> {
        exe_version_resource(exe)
    }
}

fn snapshot_pids() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return out,
        };
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            out.push((entry.th32ProcessID, name));
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

struct FocusCtx {
    pids: Vec<u32>,
    found: bool,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FocusCtx);
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if ctx.pids.contains(&pid) && IsWindowVisible(hwnd).as_bool() {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 == 0 {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            // Foreground permission: borrow the current foreground thread's
            // input queue for the duration of the SetForegroundWindow call.
            let fg = GetForegroundWindow();
            let mut fg_pid: u32 = 0;
            let fg_thread = GetWindowThreadProcessId(fg, Some(&mut fg_pid));
            let this_thread = GetCurrentThreadId();
            if fg_thread != 0 && fg_thread != this_thread {
                let _ = AttachThreadInput(this_thread, fg_thread, true);
                let _ = SetForegroundWindow(hwnd);
                let _ = AttachThreadInput(this_thread, fg_thread, false);
            } else {
                let _ = SetForegroundWindow(hwnd);
            }
            ctx.found = true;
            return BOOL(0); // stop enumeration
        }
    }
    BOOL(1)
}

fn focus_windows(pids: &[u32]) -> bool {
    if pids.is_empty() {
        return false;
    }
    let mut ctx = FocusCtx { pids: pids.to_vec(), found: false };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut FocusCtx as isize));
    }
    ctx.found
}

fn exe_version_resource(exe: &Path) -> Option<String> {
    let wide = to_wide(&exe.to_string_lossy());
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(PCWSTR(wide.as_ptr()), 0, size, buf.as_mut_ptr().cast()).is_err() {
            return None;
        }
        let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        // 040904b0 = US English + Unicode codepage; the standard fallback
            // every versioned exe ships. Try neutral 00000000 afterwards.
        for sub in ["\\StringFileInfo\\040904b0\\ProductVersion", "\\StringFileInfo\\00000000\\ProductVersion"] {
            let subblock = to_wide(sub);
            if VerQueryValueW(buf.as_ptr().cast(), PCWSTR(subblock.as_ptr()), &mut ptr, &mut len)
                .as_bool()
                && len > 0
            {
                let ws = std::slice::from_raw_parts(ptr.cast::<u16>(), len as usize);
                let s = String::from_utf16_lossy(ws);
                let s = s.trim_end_matches('\0').trim();
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
        None
    }
}
