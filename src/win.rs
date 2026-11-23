//! Hardware access without crates: the DualSense through Windows HID, and a virtual Xbox 360
//! controller through the open-source ViGEmBus driver (protocol from Nefarius' ViGEmClient, MIT:
//! BusShared.h / Common.h).

use crate::decode::{stick, Pad};
use crate::sys::*;
use std::ffi::c_void;

const GENERIC_RW: u32 = 0x8000_0000 | 0x4000_0000;
const SHARE_RW: u32 = 0x1 | 0x2;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const ERROR_IO_PENDING: u32 = 997;

const fn ctl(function: u32) -> u32 {
    // CTL_CODE(FILE_DEVICE_BUS_EXTENDER, function, METHOD_BUFFERED, FILE_WRITE_DATA)
    (0x2A << 16) | (2 << 14) | (function << 2)
}
const IOCTL_PLUGIN: u32 = ctl(0x801);
const IOCTL_UNPLUG: u32 = ctl(0x802);
const IOCTL_CHECK_VERSION: u32 = ctl(0x803);
const IOCTL_WAIT_READY: u32 = ctl(0x804);
const IOCTL_XUSB_SUBMIT: u32 = ctl(0xA02);
const VIGEM_GUID: Guid = Guid { d1: 0x96E4_2B22, d2: 0xF5E9, d3: 0x42F8, d4: [0xB0, 0x43, 0xED, 0x0F, 0x93, 0x2F, 0x01, 0x4F] };

const SONY: u16 = 0x054C;
const DUALSENSE: [u16; 2] = [0x0CE6, 0x0DF2]; // DualSense, DualSense Edge

/// Error codes shown (translated) in the UI.
pub const ERR_NO_VIGEM: u32 = 1;
pub const ERR_NO_SLOT: u32 = 2;
pub const ERR_SEND: u32 = 3;

#[repr(C)]
struct PluginTarget {
    size: u32,
    serial: u32,
    target_type: i32,
    vid: u16,
    pid: u16,
}

#[repr(C)]
struct SerialReq {
    size: u32,
    serial: u32,
} // CHECK_VERSION, UNPLUG, WAIT_DEVICE_READY

#[repr(C)]
struct XusbSubmit {
    size: u32,
    serial: u32,
    buttons: u16,
    lt: u8,
    rt: u8,
    lx: i16,
    ly: i16,
    rx: i16,
    ry: i16,
}

/// All device paths of a device interface (null-terminated UTF-16).
fn device_paths(guid: &Guid) -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(guid, std::ptr::null(), 0, 0x2 | 0x10); // PRESENT | DEVICEINTERFACE
        if set == INVALID {
            return out;
        }
        for i in 0.. {
            let mut d = SpDeviceInterfaceData { cb_size: std::mem::size_of::<SpDeviceInterfaceData>() as u32, guid: Guid::default(), flags: 0, reserved: 0 };
            if SetupDiEnumDeviceInterfaces(set, std::ptr::null(), guid, i, &mut d) == 0 {
                break;
            }
            let mut need = 0u32;
            SetupDiGetDeviceInterfaceDetailW(set, &d, std::ptr::null_mut(), 0, &mut need, std::ptr::null_mut());
            let mut buf = vec![0u32; (need as usize + 3) / 4 + 1];
            buf[0] = if cfg!(target_pointer_width = "64") { 8 } else { 6 }; // cbSize
            if SetupDiGetDeviceInterfaceDetailW(set, &d, buf.as_mut_ptr() as *mut u8, need, std::ptr::null_mut(), std::ptr::null_mut()) != 0 {
                let p = (buf.as_ptr() as *const u8).add(4) as *const u16;
                let len = (0..).take_while(|&k| *p.add(k) != 0).count();
                let mut path = std::slice::from_raw_parts(p, len).to_vec();
                path.push(0);
                out.push(path);
            }
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

fn ioctl<T>(h: Handle, code: u32, s: &T) -> bool {
    let mut n = 0u32;
    unsafe { DeviceIoControl(h, code, s as *const T as *const c_void, std::mem::size_of::<T>() as u32, std::ptr::null_mut(), 0, &mut n, std::ptr::null_mut()) != 0 }
}

/// Is the ViGEmBus driver installed and running?
pub fn vigem_present() -> bool {
    !device_paths(&VIGEM_GUID).is_empty()
}

// ------------------------------------------------------------------ ViGEm ---

/// Minimal ViGEmBus client: one virtual Xbox 360 controller. Dropping it unplugs it.
pub struct VirtualX360 {
    h: Handle,
    pub serial: u32,
}

impl VirtualX360 {
    pub fn new() -> Result<Self, u32> {
        let h = device_paths(&VIGEM_GUID)
            .into_iter()
            .find_map(|path| unsafe {
                let h = CreateFileW(path.as_ptr(), GENERIC_RW, SHARE_RW, std::ptr::null(), OPEN_EXISTING, 0, 0);
                if h == INVALID {
                    return None;
                }
                if ioctl(h, IOCTL_CHECK_VERSION, &SerialReq { size: 8, serial: 1 }) {
                    Some(h) // protocol version 1
                } else {
                    CloseHandle(h);
                    None
                }
            })
            .ok_or(ERR_NO_VIGEM)?;
        let mut pad = VirtualX360 { h, serial: 0 };
        for serial in 1..=16 {
            // first free number, like ViGEmClient; type 0 = Xbox360Wired, VID/PID 0 = driver default
            let p = PluginTarget { size: 16, serial, target_type: 0, vid: 0, pid: 0 };
            if ioctl(h, IOCTL_PLUGIN, &p) {
                pad.serial = serial;
                ioctl(h, IOCTL_WAIT_READY, &SerialReq { size: 8, serial }); // unknown to old drivers: fine
                // Send something non-zero, then zero: otherwise Windows keeps stale stick values
                // in the slot until the first real change.
                std::thread::sleep(std::time::Duration::from_millis(300));
                pad.send_raw(0, 0, 0, 1, 0, 0, 0).map_err(|_| ERR_SEND)?;
                pad.send(&Pad::default()).map_err(|_| ERR_SEND)?;
                return Ok(pad);
            }
        }
        Err(ERR_NO_SLOT)
    }

    fn send_raw(&self, buttons: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16) -> Result<(), ()> {
        let r = XusbSubmit { size: 20, serial: self.serial, buttons, lt, rt, lx, ly, rx, ry };
        ioctl(self.h, IOCTL_XUSB_SUBMIT, &r).then_some(()).ok_or(())
    }

    pub fn send(&self, p: &Pad) -> Result<(), ()> {
        self.send_raw(p.buttons, p.lt, p.rt, stick(p.lx), stick(p.ly), stick(p.rx), stick(p.ry))
    }
}

impl Drop for VirtualX360 {
    fn drop(&mut self) {
        if self.serial != 0 {
            ioctl(self.h, IOCTL_UNPLUG, &SerialReq { size: 8, serial: self.serial }); // no ghost pad
        }
        unsafe { CloseHandle(self.h) };
    }
}

// -------------------------------------------------------------- DualSense ---

pub struct DualSense {
    h: Handle,
    ov: Box<Overlapped>, // fixed address while a read is in flight
    buf: Vec<u8>,
    pub bluetooth: bool,
    pub exclusive: bool,
}

impl DualSense {
    pub fn find() -> Option<Self> {
        let mut g = Guid::default();
        unsafe { HidD_GetHidGuid(&mut g) };
        for path in device_paths(&g) {
            let Some(caps) = (unsafe { gamepad_caps(&path) }) else { continue };
            let bluetooth = caps.input_len > 64; // USB = 64 bytes, Bluetooth = 78 bytes
            if bluetooth && !bt_settled(&path) {
                continue; // Windows is still setting it up (pairing): don't interfere
            }
            // USB: try exclusive first (games then don't see the DualSense twice), else shared.
            // Bluetooth: shared only, so Windows' own Bluetooth HID stack stays undisturbed.
            let shares: &[u32] = if bluetooth { &[SHARE_RW] } else { &[0, SHARE_RW] };
            for &share in shares {
                let h = unsafe { CreateFileW(path.as_ptr(), GENERIC_RW, share, std::ptr::null(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, 0) };
                if h == INVALID {
                    continue;
                }
                if bluetooth {
                    // reading feature report 0x05 (calibration) switches Bluetooth to full report 0x31
                    let mut f = vec![0u8; (caps.feature_len as usize).max(41)];
                    f[0] = 0x05;
                    unsafe { HidD_GetFeature(h, f.as_mut_ptr() as *mut c_void, f.len() as u32) };
                }
                let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
                return Some(DualSense {
                    h,
                    ov: Box::new(Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event }),
                    buf: vec![0; caps.input_len as usize],
                    bluetooth,
                    exclusive: share == 0,
                });
            }
        }
        None
    }

    /// Read one report. Ok(None) = timeout, Err = controller gone.
    pub fn read(&mut self, timeout_ms: u32) -> Result<Option<&[u8]>, ()> {
        let mut got = 0u32;
        unsafe {
            ResetEvent(self.ov.event);
            if ReadFile(self.h, self.buf.as_mut_ptr(), self.buf.len() as u32, std::ptr::null_mut(), &mut *self.ov) == 0
                && GetLastError() != ERROR_IO_PENDING
            {
                return Err(());
            }
            if WaitForSingleObject(self.ov.event, timeout_ms) != 0 {
                CancelIo(self.h);
                GetOverlappedResult(self.h, &mut *self.ov, &mut got, 1); // wait until really cancelled
                return Ok(None);
            }
            if GetOverlappedResult(self.h, &mut *self.ov, &mut got, 0) == 0 {
                return Err(());
            }
        }
        Ok(Some(&self.buf[..got as usize]))
    }
}

impl Drop for DualSense {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.h);
            CloseHandle(self.ov.event);
        }
    }
}

/// Open a Bluetooth DualSense only after it has been present for 3 s in a row.
/// Right after pairing Windows is still setting it up; touching it then can break the pairing.
fn bt_settled(path: &[u16]) -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static SEEN: Mutex<Option<(Vec<u16>, Instant)>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap();
    match &*seen {
        Some((p, since)) if p == path => since.elapsed() >= Duration::from_secs(3),
        _ => {
            *seen = Some((path.to_vec(), Instant::now()));
            false
        }
    }
}

/// HID caps if the path is a DualSense gamepad collection.
unsafe fn gamepad_caps(path: &[u16]) -> Option<HidpCaps> {
    let q = CreateFileW(path.as_ptr(), 0, SHARE_RW, std::ptr::null(), OPEN_EXISTING, 0, 0); // query only
    if q == INVALID {
        return None;
    }
    let mut a = HiddAttributes { size: std::mem::size_of::<HiddAttributes>() as u32, ..Default::default() };
    let mut caps: HidpCaps = std::mem::zeroed();
    let mut ok = HidD_GetAttributes(q, &mut a) != 0 && a.vid == SONY && DUALSENSE.contains(&a.pid);
    if ok {
        let mut pp = 0isize;
        ok = HidD_GetPreparsedData(q, &mut pp) != 0;
        if ok {
            ok = HidP_GetCaps(pp, &mut caps) == 0x0011_0000 && caps.usage_page == 1 && caps.usage == 5; // gamepad
            HidD_FreePreparsedData(pp);
        }
    }
    CloseHandle(q);
    ok.then_some(caps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn layouts_match_vigem_headers() {
        assert_eq!(size_of::<PluginTarget>(), 16);
        assert_eq!(size_of::<XusbSubmit>(), 20);
        assert_eq!(IOCTL_PLUGIN, 0x2AA004);
        assert_eq!(IOCTL_XUSB_SUBMIT, 0x2AA808);
    }
}
