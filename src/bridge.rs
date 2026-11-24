//! The bridge thread: DualSense in, virtual Xbox 360 controller out. Runs in the background
//! process and reports through the shared block.

use crate::decode::{self, decode, Pad};
use crate::ipc::{self, Shared};
use crate::win::{DualSense, VirtualX360};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

fn stop(sh: &Shared) -> bool {
    sh.quit.load(Relaxed) != 0 || sh.paused.load(Relaxed) != 0
}

pub fn run(sh: &Shared) {
    while sh.quit.load(Relaxed) == 0 {
        if sh.paused.load(Relaxed) != 0 {
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        let pad = match VirtualX360::new() {
            Ok(p) => p,
            Err(code) => {
                sh.error.store(code, Relaxed);
                std::thread::sleep(Duration::from_secs(2)); // e.g. ViGEmBus missing: retry quietly
                continue;
            }
        };
        sh.error.store(0, Relaxed);
        sh.slot.store(pad.serial, Relaxed);
        sh.running.store(1, Relaxed);
        serve(sh, &pad);
        sh.running.store(0, Relaxed);
        sh.slot.store(0, Relaxed);
        // `pad` dropped here: virtual controller unplugged
    }
}

fn serve(sh: &Shared, pad: &VirtualX360) {
    while !stop(sh) {
        let Some(mut ds) = DualSense::find() else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        let bt = ds.bluetooth;
        sh.controller.store(if bt { 2 } else { 1 }, Relaxed);
        sh.exclusive.store(ds.exclusive as u32, Relaxed);
        ipc::signal();
        let (mut n, mut fresh, mut t) = (0u32, 0u32, Instant::now());
        let (mut last_at, mut last_ts, mut last_pad) = (None::<Instant>, None::<u32>, None::<Pad>);
        let (mut gap_sum, mut gap_sq, mut gaps) = (0f32, 0f32, 0u32);
        // optional rate limit: hold changes back and send the newest one when its turn comes
        let (mut pending, mut last_sent) = (None::<Pad>, Instant::now());
        while !stop(sh) {
            let limit = sh.rate_limit.load(Relaxed);
            let gap = if limit > 0 { Duration::from_micros(1_000_000 / limit as u64) } else { Duration::ZERO };
            let wait = if pending.is_some() { gap.saturating_sub(last_sent.elapsed()).as_millis().max(1) as u32 } else { 250 };
            match ds.read(wait) {
                Err(()) => break, // controller disconnected
                Ok(None) => {}
                Ok(Some(r)) => {
                    let now = Instant::now();
                    if let Some(prev) = last_at {
                        let g = (now - prev).as_secs_f32() * 1000.0;
                        (gap_sum, gap_sq, gaps) = (gap_sum + g, gap_sq + g * g, gaps + 1);
                    }
                    last_at = Some(now);
                    match decode::sensor_time(r, bt) {
                        Some(ts) if Some(ts) == last_ts => {} // polled again, nothing new
                        ts => {
                            fresh += 1;
                            last_ts = ts;
                        }
                    }
                    let dz = sh.deadzone.load(Relaxed) as f32 / 1000.0;
                    if let Some(p) = decode(r, bt, dz) {
                        n += 1;
                        // only talk to the driver when something changed: keeps CPU near zero
                        pending = if last_pad != Some(p) { Some(p) } else { None };
                    }
                }
            }
            if let Some(p) = pending {
                if last_sent.elapsed() >= gap {
                    if pad.send(&p).is_err() {
                        sh.error.store(crate::win::ERR_SEND, Relaxed);
                    }
                    sh.set_pad(&p);
                    ipc::signal();
                    (last_pad, pending, last_sent) = (Some(p), None, Instant::now());
                }
            }
            if t.elapsed() >= Duration::from_secs(1) {
                let mean = gap_sum / gaps.max(1) as f32;
                let var = (gap_sq / gaps.max(1) as f32 - mean * mean).max(0.0);
                sh.rate.store(n, Relaxed);
                sh.fresh.store(fresh, Relaxed);
                Shared::set_f32(&sh.interval, mean);
                Shared::set_f32(&sh.jitter, var.sqrt());
                (n, fresh, t) = (0, 0, Instant::now());
                (gap_sum, gap_sq, gaps) = (0.0, 0.0, 0);
            }
        }
        let _ = pad.send(&Pad::default()); // release everything, no stuck stick
        sh.set_pad(&Pad::default());
        sh.controller.store(0, Relaxed);
        ipc::signal();
        for a in [&sh.rate, &sh.fresh, &sh.interval, &sh.jitter] {
            a.store(0, Relaxed);
        }
    }
}
