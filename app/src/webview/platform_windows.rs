//! Windows-specific utilities for native child-window overlay positioning.
//!
//! Uses raw Win32 FFI to find the Makepad HWND, query DPI, and reposition
//! the CEF child window via `SetWindowPos`.

const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_NOZORDER: u32 = 0x0004;
const SW_SHOW: i32 = 5;
const SW_HIDE: i32 = 0;

type WNDENUMPROC = unsafe extern "system" fn(isize, isize) -> i32;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(lpEnumFunc: WNDENUMPROC, lParam: isize) -> i32;
    fn GetWindowThreadProcessId(hWnd: isize, lpdwProcessId: *mut u32) -> u32;
    fn SetWindowPos(
        hWnd: isize,
        hWndInsertAfter: isize,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        uFlags: u32,
    ) -> i32;
    fn ShowWindow(hWnd: isize, nCmdShow: i32) -> i32;
    fn IsWindowVisible(hWnd: isize) -> i32;
    fn GetDpiForWindow(hwnd: isize) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}

struct FindWindowData {
    target_pid: u32,
    found_hwnd: isize,
}

unsafe extern "system" fn enum_windows_cb(hwnd: isize, lparam: isize) -> i32 {
    let data = unsafe { &mut *(lparam as *mut FindWindowData) };
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == data.target_pid && unsafe { IsWindowVisible(hwnd) } != 0 {
        data.found_hwnd = hwnd;
        return 0; // stop enumeration
    }
    1 // continue
}

/// Find the main Makepad window HWND by matching the current process ID.
///
/// Returns the first visible top-level window belonging to this process.
pub fn find_makepad_hwnd() -> Option<isize> {
    let mut data = FindWindowData {
        target_pid: unsafe { GetCurrentProcessId() },
        found_hwnd: 0,
    };
    unsafe { EnumWindows(enum_windows_cb, &mut data as *mut _ as isize) };
    if data.found_hwnd != 0 {
        Some(data.found_hwnd)
    } else {
        None
    }
}

/// Reposition a child window within its parent's client area.
pub fn reposition_child(child_hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        SetWindowPos(
            child_hwnd,
            0, // ignored with SWP_NOZORDER
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

/// Show or hide a child window.
pub fn show_child(child_hwnd: isize, visible: bool) {
    unsafe {
        ShowWindow(child_hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

/// Get the DPI scaling factor for the given window.
///
/// Returns 1.0 for standard 96 DPI, 1.5 for 144 DPI, 2.0 for 192 DPI, etc.
pub fn get_dpi_factor(hwnd: isize) -> f64 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
}
