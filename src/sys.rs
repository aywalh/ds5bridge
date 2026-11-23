//! Raw Win32 declarations. No crates: everything links against DLLs that ship with Windows.
#![allow(non_snake_case, clippy::too_many_arguments)]

use std::ffi::c_void;

pub type Handle = isize;
pub type Hwnd = isize;
pub const INVALID: Handle = -1;
pub const HKCU: isize = 0x8000_0001u32 as i32 as isize;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

#[repr(C)]
pub struct SpDeviceInterfaceData {
    pub cb_size: u32,
    pub guid: Guid,
    pub flags: u32,
    pub reserved: usize,
}

#[repr(C)]
pub struct Overlapped {
    pub internal: usize,
    pub internal_high: usize,
    pub offset: u32,
    pub offset_high: u32,
    pub event: Handle,
}

#[repr(C)]
#[derive(Default)]
pub struct HiddAttributes {
    pub size: u32,
    pub vid: u16,
    pub pid: u16,
    pub version: u16,
}

#[repr(C)]
pub struct HidpCaps {
    pub usage: u16,
    pub usage_page: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
    pub rest: [u16; 27],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct Msg {
    pub hwnd: Hwnd,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub pt: Point,
    pub private: u32,
}

pub type WndProc = extern "system" fn(Hwnd, u32, usize, isize) -> isize;

#[repr(C)]
pub struct WndClassExW {
    pub cb_size: u32,
    pub style: u32,
    pub wnd_proc: WndProc,
    pub cls_extra: i32,
    pub wnd_extra: i32,
    pub instance: isize,
    pub icon: isize,
    pub cursor: isize,
    pub background: isize,
    pub menu_name: *const u16,
    pub class_name: *const u16,
    pub icon_sm: isize,
}

#[repr(C)]
pub struct NotifyIconDataW {
    pub cb_size: u32,
    pub hwnd: Hwnd,
    pub id: u32,
    pub flags: u32,
    pub callback_message: u32,
    pub icon: isize,
    pub tip: [u16; 128],
    pub state: u32,
    pub state_mask: u32,
    pub info: [u16; 256],
    pub version: u32,
    pub info_title: [u16; 64],
    pub info_flags: u32,
    pub guid: Guid,
    pub balloon_icon: isize,
}

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: Handle) -> Handle;
    pub fn CloseHandle(h: Handle) -> i32;
    pub fn DeviceIoControl(h: Handle, code: u32, inb: *const c_void, insz: u32, outb: *mut c_void, outsz: u32, ret: *mut u32, ov: *mut Overlapped) -> i32;
    pub fn ReadFile(h: Handle, buf: *mut u8, len: u32, read: *mut u32, ov: *mut Overlapped) -> i32;
    pub fn GetOverlappedResult(h: Handle, ov: *mut Overlapped, n: *mut u32, wait: i32) -> i32;
    pub fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    pub fn CreateEventW(sa: *const c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
    pub fn ResetEvent(h: Handle) -> i32;
    pub fn CancelIo(h: Handle) -> i32;
    pub fn GetLastError() -> u32;
    pub fn CreateMutexW(sa: *const c_void, owner: i32, name: *const u16) -> Handle;
    pub fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> Handle;
    pub fn CreateFileMappingW(file: Handle, sa: *const c_void, protect: u32, size_hi: u32, size_lo: u32, name: *const u16) -> Handle;
    pub fn MapViewOfFile(h: Handle, access: u32, off_hi: u32, off_lo: u32, bytes: usize) -> *mut c_void;
    pub fn GetModuleHandleW(name: *const u16) -> isize;
    pub fn GetUserDefaultGeoName(buf: *mut u16, n: i32) -> i32;
    pub fn GetUserDefaultUILanguage() -> u16;
    pub fn GetUserDefaultLocaleName(buf: *mut u16, n: i32) -> i32;
}

#[link(name = "setupapi")]
extern "system" {
    pub fn SetupDiGetClassDevsW(guid: *const Guid, enumerator: *const u16, hwnd: isize, flags: u32) -> Handle;
    pub fn SetupDiEnumDeviceInterfaces(set: Handle, info: *const c_void, guid: *const Guid, idx: u32, out: *mut SpDeviceInterfaceData) -> i32;
    pub fn SetupDiGetDeviceInterfaceDetailW(set: Handle, d: *const SpDeviceInterfaceData, detail: *mut u8, size: u32, req: *mut u32, info: *mut c_void) -> i32;
    pub fn SetupDiDestroyDeviceInfoList(set: Handle) -> i32;
}

#[link(name = "hid")]
extern "system" {
    pub fn HidD_GetHidGuid(g: *mut Guid);
    pub fn HidD_GetAttributes(h: Handle, a: *mut HiddAttributes) -> u8;
    pub fn HidD_GetPreparsedData(h: Handle, pp: *mut isize) -> u8;
    pub fn HidD_FreePreparsedData(pp: isize) -> u8;
    pub fn HidP_GetCaps(pp: isize, caps: *mut HidpCaps) -> i32;
    pub fn HidD_GetFeature(h: Handle, buf: *mut c_void, len: u32) -> u8;
}

#[link(name = "user32")]
extern "system" {
    pub fn RegisterClassExW(wc: *const WndClassExW) -> u16;
    pub fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: isize, inst: isize, param: *const c_void) -> Hwnd;
    pub fn DefWindowProcW(h: Hwnd, msg: u32, w: usize, l: isize) -> isize;
    pub fn DestroyWindow(h: Hwnd) -> i32;
    pub fn PostQuitMessage(code: i32);
    pub fn GetMessageW(msg: *mut Msg, h: Hwnd, min: u32, max: u32) -> i32;
    pub fn TranslateMessage(msg: *const Msg) -> i32;
    pub fn DispatchMessageW(msg: *const Msg) -> isize;
    pub fn PostMessageW(h: Hwnd, msg: u32, w: usize, l: isize) -> i32;
    pub fn FindWindowW(class: *const u16, title: *const u16) -> Hwnd;
    pub fn SetTimer(h: Hwnd, id: usize, ms: u32, f: isize) -> usize;
    pub fn CreatePopupMenu() -> isize;
    pub fn AppendMenuW(menu: isize, flags: u32, id: usize, text: *const u16) -> i32;
    pub fn TrackPopupMenu(menu: isize, flags: u32, x: i32, y: i32, r: i32, h: Hwnd, rect: *const c_void) -> i32;
    pub fn DestroyMenu(menu: isize) -> i32;
    pub fn GetCursorPos(p: *mut Point) -> i32;
    pub fn SetForegroundWindow(h: Hwnd) -> i32;
    pub fn ShowWindow(h: Hwnd, cmd: i32) -> i32;
    pub fn RegisterWindowMessageW(name: *const u16) -> u32;
    pub fn CreateIconFromResourceEx(bits: *const u8, size: u32, icon: i32, ver: u32, cx: i32, cy: i32, flags: u32) -> isize;
    pub fn GetSystemMetrics(i: i32) -> i32;
}

#[link(name = "shell32")]
extern "system" {
    pub fn Shell_NotifyIconW(msg: u32, data: *const NotifyIconDataW) -> i32;
    pub fn ShellExecuteW(h: Hwnd, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
    pub fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: isize, out: *mut *mut u16) -> i32;
}

#[link(name = "ole32")]
extern "system" {
    pub fn CoInitializeEx(reserved: *const c_void, coinit: u32) -> i32;
    pub fn CoCreateInstance(clsid: *const Guid, outer: *const c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
    pub fn CoTaskMemFree(p: *const c_void);
}

#[link(name = "advapi32")]
extern "system" {
    pub fn RegCreateKeyExW(key: isize, sub: *const u16, res: u32, class: *const u16, opts: u32, sam: u32, sa: *const c_void, out: *mut isize, disp: *mut u32) -> i32;
    pub fn RegSetValueExW(key: isize, name: *const u16, res: u32, typ: u32, data: *const u8, len: u32) -> i32;
    pub fn RegDeleteKeyValueW(key: isize, sub: *const u16, name: *const u16) -> i32;
    pub fn RegDeleteTreeW(key: isize, sub: *const u16) -> i32;
    pub fn RegCloseKey(key: isize) -> i32;
    pub fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, typ: *mut u32, data: *mut c_void, len: *mut u32) -> i32;
}

#[link(name = "bcrypt")]
extern "system" {
    pub fn BCryptOpenAlgorithmProvider(h: *mut isize, alg: *const u16, imp: *const u16, flags: u32) -> i32;
    pub fn BCryptHash(h: isize, secret: *const u8, secret_len: u32, input: *const u8, input_len: u32, out: *mut u8, out_len: u32) -> i32;
    pub fn BCryptCloseAlgorithmProvider(h: isize, flags: u32) -> i32;
}

/// Null-terminated UTF-16 for the W functions.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// UTF-16 buffer up to the first null, into a String.
pub fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Copy text into a fixed UTF-16 field (tray tooltip etc.), cut to fit.
pub fn fill_wide<const N: usize>(dst: &mut [u16; N], s: &str) {
    let src: Vec<u16> = s.encode_utf16().take(N - 1).collect();
    dst[..src.len()].copy_from_slice(&src);
    dst[src.len()] = 0;
}

/// Is a named mutex (single-instance marker) currently held by some process?
pub fn mutex_exists(name: &str) -> bool {
    let h = unsafe { OpenMutexW(0x0010_0000, 0, wide(name).as_ptr()) }; // SYNCHRONIZE
    if h == 0 {
        return false;
    }
    unsafe { CloseHandle(h) };
    true
}

/// Open a URL or folder with the user's default handler.
pub fn open_url(url: &str) {
    unsafe { ShellExecuteW(0, wide("open").as_ptr(), wide(url).as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn struct_layouts() {
        if cfg!(target_pointer_width = "64") {
            assert_eq!(size_of::<NotifyIconDataW>(), 976);
            assert_eq!(size_of::<WndClassExW>(), 80);
            assert_eq!(size_of::<SpDeviceInterfaceData>(), 32);
        }
        assert_eq!(size_of::<HidpCaps>(), 64);
        let mut f = [0u16; 4];
        fill_wide(&mut f, "abcdef");
        assert_eq!(from_wide(&f), "abc");
    }
}
