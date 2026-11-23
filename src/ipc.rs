//! State shared between the background process (bridge + tray) and the UI process, through a
//! small named shared-memory block. Only atomics, so both sides read and write without locks.
//! Nothing here leaves the PC.

use crate::decode::Pad;
use crate::sys::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

pub const BG_MUTEX: &str = "Local\\DS5Bridge.Background";
pub const UI_MUTEX: &str = "Local\\DS5Bridge.Window";
const MAPPING: &str = "Local\\DS5Bridge.State.v1";

#[repr(C)]
pub struct Shared {
    // bridge -> UI
    pub running: AtomicU32,    // virtual controller plugged in
    pub slot: AtomicU32,       // ViGEm serial (Xbox slot)
    pub controller: AtomicU32, // 0 none, 1 USB, 2 Bluetooth
    pub exclusive: AtomicU32,
    pub buttons: AtomicU32,
    pub triggers: AtomicU32, // lt | rt << 8
    pub sticks: [AtomicU32; 4], // f32 bits
    pub rate: AtomicU32,     // reports per second
    pub fresh: AtomicU32,    // reports with a new controller timestamp per second
    pub interval: AtomicU32, // f32 bits, ms
    pub jitter: AtomicU32,   // f32 bits, ms
    pub error: AtomicU32,    // win::ERR_*
    // UI -> bridge
    pub deadzone: AtomicU32, // per mille
    pub paused: AtomicU32,
    pub quit: AtomicU32,
    pub lang: AtomicU32,
}

/// Snapshot for drawing.
#[derive(Clone, Default, PartialEq)]
pub struct Status {
    pub running: bool,
    pub slot: u32,
    pub controller: u32,
    pub exclusive: bool,
    pub pad: Pad,
    pub rate: u32,
    pub fresh: u32,
    pub interval_ms: f32,
    pub jitter_ms: f32,
    pub error: u32,
    pub paused: bool,
}

impl Shared {
    pub fn status(&self) -> Status {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Relaxed));
        let t = self.triggers.load(Relaxed);
        Status {
            running: self.running.load(Relaxed) != 0,
            slot: self.slot.load(Relaxed),
            controller: self.controller.load(Relaxed),
            exclusive: self.exclusive.load(Relaxed) != 0,
            pad: Pad {
                buttons: self.buttons.load(Relaxed) as u16,
                lt: t as u8,
                rt: (t >> 8) as u8,
                lx: f(&self.sticks[0]),
                ly: f(&self.sticks[1]),
                rx: f(&self.sticks[2]),
                ry: f(&self.sticks[3]),
            },
            rate: self.rate.load(Relaxed),
            fresh: self.fresh.load(Relaxed),
            interval_ms: f(&self.interval),
            jitter_ms: f(&self.jitter),
            error: self.error.load(Relaxed),
            paused: self.paused.load(Relaxed) != 0,
        }
    }

    pub fn set_pad(&self, p: &Pad) {
        self.buttons.store(p.buttons as u32, Relaxed);
        self.triggers.store(p.lt as u32 | (p.rt as u32) << 8, Relaxed);
        for (a, v) in self.sticks.iter().zip([p.lx, p.ly, p.rx, p.ry]) {
            a.store(v.to_bits(), Relaxed);
        }
    }

    pub fn set_f32(a: &AtomicU32, v: f32) {
        a.store(v.to_bits(), Relaxed);
    }
}

/// Open (or create, zero-filled) the shared block. Lives as long as the process.
pub fn open() -> &'static Shared {
    let size = std::mem::size_of::<Shared>() as u32;
    unsafe {
        // INVALID_HANDLE_VALUE = backed by the page file; PAGE_READWRITE
        let h = CreateFileMappingW(INVALID, std::ptr::null(), 0x04, 0, size, wide(MAPPING).as_ptr());
        let p = if h == 0 { std::ptr::null_mut() } else { MapViewOfFile(h, 0x000F_001F, 0, 0, size as usize) };
        if p.is_null() {
            // should never happen; fall back to private memory so the app still works alone
            return Box::leak(Box::new(std::mem::zeroed::<Shared>()));
        }
        &*(p as *const Shared)
    }
}
