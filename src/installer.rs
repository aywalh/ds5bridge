//! Per-user install without admin rights: copy the exe to %LOCALAPPDATA%\Programs\DS5 Bridge,
//! add Start menu / desktop shortcuts, an "Apps & features" entry and optional autostart.

use crate::sys::*;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

pub const APP: &str = "DS5 Bridge";
pub const EXE: &str = "DS5Bridge.exe";
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\DS5Bridge";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

pub const FOLDER_LOCAL: Guid = Guid { d1: 0xF1B3_2785, d2: 0x6FBA, d3: 0x4FCF, d4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91] };
pub const FOLDER_ROAMING: Guid = Guid { d1: 0x3EB6_85DB, d2: 0x65F9, d3: 0x4CF6, d4: [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D] };
const FOLDER_PROGRAMS: Guid = Guid { d1: 0xA77F_5D77, d2: 0x2E2B, d3: 0x44C3, d4: [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51] };
const FOLDER_DESKTOP: Guid = Guid { d1: 0xB4BF_CC3A, d2: 0xDB2C, d3: 0x424C, d4: [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41] };

pub struct Options {
    pub start_menu: bool,
    pub desktop: bool,
    pub autostart: bool,
}

/// A Windows known folder (handles redirected Desktop/OneDrive correctly).
pub fn known_folder(id: Guid) -> PathBuf {
    let mut p: *mut u16 = std::ptr::null_mut();
    unsafe {
        if SHGetKnownFolderPath(&id, 0, 0, &mut p) == 0 && !p.is_null() {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            CoTaskMemFree(p as *const c_void);
            return PathBuf::from(s);
        }
    }
    std::env::temp_dir()
}

pub fn install_dir() -> PathBuf {
    known_folder(FOLDER_LOCAL).join("Programs").join(APP)
}

pub fn installed_exe() -> PathBuf {
    install_dir().join(EXE)
}

fn start_menu_link() -> PathBuf {
    known_folder(FOLDER_PROGRAMS).join(format!("{APP}.lnk"))
}

fn desktop_link() -> PathBuf {
    known_folder(FOLDER_DESKTOP).join(format!("{APP}.lnk"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).map(|p| p.to_string_lossy().to_lowercase());
    matches!((canon(a), canon(b)), (Ok(x), Ok(y)) if x == y)
}

/// Is this process the installed copy?
pub fn running_installed() -> bool {
    std::env::current_exe().map(|e| same_file(&e, &installed_exe())).unwrap_or(false)
}

pub fn is_installed() -> bool {
    installed_exe().exists()
}

// ------------------------------------------------------------- registry ---

fn reg_set(sub: &str, name: &str, value: &str) -> bool {
    let mut key = 0isize;
    unsafe {
        // KEY_WRITE
        if RegCreateKeyExW(HKCU, wide(sub).as_ptr(), 0, std::ptr::null(), 0, 0x2_0006, std::ptr::null(), &mut key, std::ptr::null_mut()) != 0 {
            return false;
        }
        let data = wide(value);
        let ok = RegSetValueExW(key, wide(name).as_ptr(), 0, 1, data.as_ptr() as *const u8, (data.len() * 2) as u32) == 0; // REG_SZ
        RegCloseKey(key);
        ok
    }
}

fn reg_set_dword(sub: &str, name: &str, value: u32) {
    let mut key = 0isize;
    unsafe {
        if RegCreateKeyExW(HKCU, wide(sub).as_ptr(), 0, std::ptr::null(), 0, 0x2_0006, std::ptr::null(), &mut key, std::ptr::null_mut()) == 0 {
            RegSetValueExW(key, wide(name).as_ptr(), 0, 4, &value as *const u32 as *const u8, 4); // REG_DWORD
            RegCloseKey(key);
        }
    }
}

fn reg_get(sub: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKCU, wide(sub).as_ptr(), wide(name).as_ptr(), 0x2, std::ptr::null_mut(), buf.as_mut_ptr() as *mut c_void, &mut len) }; // RRF_RT_REG_SZ
    (r == 0).then(|| from_wide(&buf))
}

pub fn autostart_enabled() -> bool {
    reg_get(RUN_KEY, APP).is_some()
}

pub fn set_autostart(on: bool) {
    if on {
        reg_set(RUN_KEY, APP, &format!("\"{}\" --background", installed_exe().display()));
    } else {
        unsafe { RegDeleteKeyValueW(HKCU, wide(RUN_KEY).as_ptr(), wide(APP).as_ptr()) };
    }
}

/// Keep the "Apps & features" version in step after an update.
pub fn set_registered_version(v: &str) {
    if reg_get(UNINSTALL_KEY, "DisplayName").is_some() {
        reg_set(UNINSTALL_KEY, "DisplayVersion", v);
    }
}

// ------------------------------------------------------------ shortcuts ---

const CLSID_SHELL_LINK: Guid = Guid { d1: 0x0002_1401, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_SHELL_LINK_W: Guid = Guid { d1: 0x0002_14F9, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_PERSIST_FILE: Guid = Guid { d1: 0x0000_010B, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };

/// COM method `index` of the object's vtable.
unsafe fn vfn(obj: *mut c_void, index: usize) -> *const c_void {
    *(*(obj as *const *const *const c_void)).add(index)
}

/// Create a .lnk through IShellLinkW + IPersistFile (raw COM, no crates).
fn create_shortcut(link: &Path, target: &Path, icon: &Path, desc: &str) -> bool {
    type SetStr = unsafe extern "system" fn(*mut c_void, *const u16) -> i32;
    type SetIcon = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
    type Qi = unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32;
    type Save = unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    unsafe {
        CoInitializeEx(std::ptr::null(), 0x2); // apartment threaded; "already initialised" is fine
        let mut sl: *mut c_void = std::ptr::null_mut();
        if CoCreateInstance(&CLSID_SHELL_LINK, std::ptr::null(), 1, &IID_SHELL_LINK_W, &mut sl) != 0 {
            return false;
        }
        let call_str = |i: usize, s: &str| std::mem::transmute::<_, SetStr>(vfn(sl, i))(sl, wide(s).as_ptr());
        call_str(20, &target.to_string_lossy()); // SetPath
        call_str(9, &target.parent().unwrap_or(Path::new("")).to_string_lossy()); // SetWorkingDirectory
        call_str(7, desc); // SetDescription
        std::mem::transmute::<_, SetIcon>(vfn(sl, 17))(sl, wide(&icon.to_string_lossy()).as_ptr(), 0); // SetIconLocation
        let mut pf: *mut c_void = std::ptr::null_mut();
        let mut ok = std::mem::transmute::<_, Qi>(vfn(sl, 0))(sl, &IID_PERSIST_FILE, &mut pf) == 0;
        if ok {
            ok = std::mem::transmute::<_, Save>(vfn(pf, 6))(pf, wide(&link.to_string_lossy()).as_ptr(), 1) == 0;
            std::mem::transmute::<_, Release>(vfn(pf, 2))(pf);
        }
        std::mem::transmute::<_, Release>(vfn(sl, 2))(sl);
        ok
    }
}

// -------------------------------------------------------- install/remove ---

/// Ask a running background process to quit and wait (max ~5 s) until it is gone.
pub fn stop_background() {
    crate::ipc::open().quit.store(1, std::sync::atomic::Ordering::Relaxed);
    for _ in 0..50 {
        if !mutex_exists(crate::ipc::BG_MUTEX) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

pub fn install(o: &Options) -> Result<(), String> {
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = installed_exe();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if !same_file(&me, &exe) {
        stop_background(); // reinstall over a running copy
        std::fs::copy(&me, &exe).map_err(|e| e.to_string())?;
    }
    let icon = dir.join("ds5bridge.ico");
    std::fs::write(&icon, crate::icon::ico_file()).map_err(|e| e.to_string())?;

    for (want, link) in [(o.start_menu, start_menu_link()), (o.desktop, desktop_link())] {
        if want {
            create_shortcut(&link, &exe, &icon, "PS5 DualSense → Xbox controller");
        } else {
            let _ = std::fs::remove_file(link);
        }
    }
    set_autostart(o.autostart);

    let k = UNINSTALL_KEY;
    reg_set(k, "DisplayName", APP);
    reg_set(k, "DisplayVersion", env!("CARGO_PKG_VERSION"));
    reg_set(k, "Publisher", "aywalh");
    reg_set(k, "URLInfoAbout", crate::updater::REPO_URL);
    reg_set(k, "DisplayIcon", &icon.to_string_lossy());
    reg_set(k, "InstallLocation", &dir.to_string_lossy());
    reg_set(k, "UninstallString", &format!("\"{}\" --uninstall", exe.display()));
    reg_set_dword(k, "NoModify", 1);
    reg_set_dword(k, "NoRepair", 1);
    let kb = std::fs::metadata(&exe).map(|m| m.len() / 1024).unwrap_or(0) as u32;
    reg_set_dword(k, "EstimatedSize", kb);
    Ok(())
}

/// Remove everything the app created. The exe folder itself is deleted a moment after this
/// process exits (a running exe cannot delete itself).
pub fn uninstall() {
    stop_background();
    set_autostart(false);
    let _ = std::fs::remove_file(start_menu_link());
    let _ = std::fs::remove_file(desktop_link());
    unsafe { RegDeleteTreeW(HKCU, wide(UNINSTALL_KEY).as_ptr()) };
    let _ = std::fs::remove_dir_all(crate::settings::dir());
    let dir = install_dir();
    if dir.exists() {
        use std::os::windows::process::CommandExt;
        let cmd = format!("ping 127.0.0.1 -n 3 > nul & rmdir /s /q \"{}\"", dir.display());
        let _ = std::process::Command::new("cmd").args(["/C", &cmd]).creation_flags(0x0800_0000).spawn(); // CREATE_NO_WINDOW
    }
}
