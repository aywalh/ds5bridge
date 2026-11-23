//! The background process: bridge thread + tray icon, plain Win32 (no UI toolkit loaded), so it
//! idles at a few MB of RAM and near-zero CPU. The window opens as a separate process on demand
//! and closing it does not stop the bridge.

use crate::i18n::{tr_native, trf_native};
use crate::ipc::{self, Shared};
use crate::sys::*;
use std::sync::atomic::Ordering::Relaxed;

const CLASS: &str = "DS5BridgeTray";
const WM_TRAY: u32 = 0x8000 + 1; // WM_APP + 1
const WM_SHOW_UI: u32 = 0x8000 + 2;
const ID_OPEN: usize = 1;
const ID_QUIT: usize = 2;

static mut TASKBAR_CREATED: u32 = 0;
static mut ICON: isize = 0;

fn shared() -> &'static Shared {
    static S: std::sync::OnceLock<&'static Shared> = std::sync::OnceLock::new();
    S.get_or_init(ipc::open)
}

/// Start the UI process (or bring an open window to the front).
pub fn open_ui() {
    if mutex_exists(ipc::UI_MUTEX) {
        unsafe {
            let h = FindWindowW(std::ptr::null(), wide(crate::installer::APP).as_ptr());
            if h != 0 {
                ShowWindow(h, 9); // SW_RESTORE
                SetForegroundWindow(h);
            }
        }
        return;
    }
    if let Ok(me) = std::env::current_exe() {
        let _ = std::process::Command::new(me).arg("--ui").spawn();
    }
}

/// Plain start (Start menu): make sure the background runs, then show the window through it.
pub fn start_and_show() {
    unsafe {
        let h = FindWindowW(wide(CLASS).as_ptr(), std::ptr::null());
        if h != 0 {
            PostMessageW(h, WM_SHOW_UI, 0, 0);
            return;
        }
    }
    if let Ok(me) = std::env::current_exe() {
        let _ = std::process::Command::new(me).args(["--background", "--show"]).spawn();
    }
}

fn tooltip() -> String {
    let s = shared().status();
    let state = if s.paused {
        tr_native("Paused").to_string()
    } else {
        match s.controller {
            1 => trf_native("Connected via {}", &[&"USB"]),
            2 => trf_native("Connected via {}", &[&"Bluetooth"]),
            _ => tr_native("Waiting for DualSense").to_string(),
        }
    };
    format!("DS5 Bridge – {state}")
}

fn icon_data(hwnd: Hwnd) -> NotifyIconDataW {
    let mut d: NotifyIconDataW = unsafe { std::mem::zeroed() };
    d.cb_size = std::mem::size_of::<NotifyIconDataW>() as u32;
    d.hwnd = hwnd;
    d.id = 1;
    d.flags = 0x1 | 0x2 | 0x4; // NIF_MESSAGE | NIF_ICON | NIF_TIP
    d.callback_message = WM_TRAY;
    d.icon = unsafe { ICON };
    fill_wide(&mut d.tip, &tooltip());
    d
}

fn show_menu(hwnd: Hwnd) {
    unsafe {
        let m = CreatePopupMenu();
        AppendMenuW(m, 0, ID_OPEN, wide(tr_native("Open")).as_ptr());
        AppendMenuW(m, 0x800, 0, std::ptr::null()); // separator
        AppendMenuW(m, 0, ID_QUIT, wide(tr_native("Quit")).as_ptr());
        let mut p = Point::default();
        GetCursorPos(&mut p);
        SetForegroundWindow(hwnd); // otherwise the menu does not close when clicking elsewhere
        let cmd = TrackPopupMenu(m, 0x100 | 0x20, p.x, p.y, 0, hwnd, std::ptr::null()); // TPM_RETURNCMD | TPM_RIGHTBUTTON
        DestroyMenu(m);
        match cmd as usize {
            ID_OPEN => open_ui(),
            ID_QUIT => {
                shared().quit.store(1, Relaxed);
                DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, w: usize, l: isize) -> isize {
    unsafe {
        match msg {
            WM_TRAY => match (l & 0xFFFF) as u32 {
                0x0202 => open_ui(),                   // WM_LBUTTONUP
                0x0205 | 0x007B => show_menu(hwnd),    // WM_RBUTTONUP | WM_CONTEXTMENU
                _ => {}
            },
            WM_SHOW_UI => open_ui(),
            0x0113 => {
                // WM_TIMER: refresh tooltip/language, notice a quit request from the UI
                let sh = shared();
                if sh.quit.load(Relaxed) != 0 {
                    DestroyWindow(hwnd);
                } else {
                    crate::i18n::set(sh.lang.load(Relaxed) as usize);
                    Shell_NotifyIconW(1, &icon_data(hwnd)); // NIM_MODIFY
                }
            }
            0x0002 => {
                // WM_DESTROY
                Shell_NotifyIconW(2, &icon_data(hwnd)); // NIM_DELETE
                PostQuitMessage(0);
            }
            m if m == TASKBAR_CREATED && m != 0 => {
                Shell_NotifyIconW(0, &icon_data(hwnd)); // Explorer restarted: add the icon again
            }
            _ => return DefWindowProcW(hwnd, msg, w, l),
        }
    }
    0
}

pub fn run_background(show: bool) {
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, wide(ipc::BG_MUTEX).as_ptr()) };
    if unsafe { GetLastError() } == 183 {
        // ERROR_ALREADY_EXISTS: another background instance runs; just show its window
        if show {
            start_and_show();
        }
        return;
    }
    crate::updater::cleanup();
    let sh = shared();
    let s = crate::settings::load();
    sh.quit.store(0, Relaxed);
    sh.paused.store(0, Relaxed);
    sh.deadzone.store(s.deadzone, Relaxed);
    sh.lang.store(crate::i18n::cur() as u32, Relaxed);

    let bridge = std::thread::spawn(|| crate::bridge::run(shared()));

    unsafe {
        TASKBAR_CREATED = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let n = GetSystemMetrics(49).clamp(16, 64) as u32; // SM_CXSMICON, DPI aware enough
        let bits = crate::icon::dib(n);
        ICON = CreateIconFromResourceEx(bits.as_ptr(), bits.len() as u32, 1, 0x0003_0000, n as i32, n as i32, 0);
        let inst = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS);
        let wc = WndClassExW {
            cb_size: std::mem::size_of::<WndClassExW>() as u32,
            style: 0,
            wnd_proc,
            cls_extra: 0,
            wnd_extra: 0,
            instance: inst,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
            icon_sm: 0,
        };
        RegisterClassExW(&wc);
        // a normal hidden top-level window (not message-only), so it receives TaskbarCreated
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide(CLASS).as_ptr(), 0, 0, 0, 0, 0, 0, 0, inst, std::ptr::null());
        Shell_NotifyIconW(0, &icon_data(hwnd)); // NIM_ADD
        SetTimer(hwnd, 1, 1000, 0);
        if show {
            open_ui();
        }
        let mut msg: Msg = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    sh.quit.store(1, Relaxed);
    let _ = bridge.join(); // unplugs the virtual controller
    sh.running.store(0, Relaxed);
    sh.controller.store(0, Relaxed);
    unsafe { CloseHandle(mutex) };
}
