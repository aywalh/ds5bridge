//! Reads PlayStation controllers and optionally publishes a virtual Xbox 360 controller. Runs in the background
//! process and reports through the shared block.

use crate::decode::{self, decode, Pad, SonyModel};
use crate::ipc::{self, Shared};
use crate::win::{SonyController, VirtualX360};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

pub fn run(sh: &Shared) {
    while sh.quit.load(Relaxed) == 0 {
        let enabled = sh.paused.load(Relaxed) == 0;
        let Some(mut controller) = SonyController::find(enabled) else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        serve(sh, &mut controller, enabled);
    }
}

fn serve(sh: &Shared, ds: &mut SonyController, enabled: bool) {
    let bt = ds.bluetooth;
    let model = ds.model;
    let model_offset = if model == SonyModel::DualShock4 { 2 } else { 0 };
    sh.controller
        .store(model_offset + if bt { 2 } else { 1 }, Relaxed);
    sh.exclusive.store(ds.exclusive as u32, Relaxed);
    ipc::signal();
    let mut pad = None::<VirtualX360>;
    let mut retry_at = Instant::now();
    let (mut n, mut fresh, mut t) = (0u32, 0u32, Instant::now());
    let (mut last_at, mut last_ts, mut preview) = (None::<Instant>, None::<u32>, None::<Pad>);
    let (mut gap_sum, mut gap_sq, mut gaps) = (0f32, 0f32, 0u32);
    // optional rate limit: hold changes back and send the newest one when its turn comes
    let (mut pending, mut sent, mut last_sent) = (None::<Pad>, None::<Pad>, Instant::now());
    while sh.quit.load(Relaxed) == 0 && (sh.paused.load(Relaxed) == 0) == enabled {
        if enabled && pad.is_none() && Instant::now() >= retry_at {
            match VirtualX360::new() {
                Ok(output) => {
                    sh.error.store(0, Relaxed);
                    sh.slot.store(output.serial, Relaxed);
                    sh.running.store(1, Relaxed);
                    pending = preview;
                    pad = Some(output);
                }
                Err(code) => sh.error.store(code, Relaxed),
            }
            retry_at = Instant::now() + Duration::from_secs(2);
            ipc::signal();
        }
        let limit = sh.rate_limit.load(Relaxed);
        let gap = if limit > 0 {
            Duration::from_micros(1_000_000 / limit as u64)
        } else {
            Duration::ZERO
        };
        let wait = if pending.is_some() {
            gap.saturating_sub(last_sent.elapsed()).as_millis().max(1) as u32
        } else {
            250
        };
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
                match (model == SonyModel::DualSense)
                    .then(|| decode::sensor_time(r, bt))
                    .flatten()
                {
                    Some(ts) if Some(ts) == last_ts => {} // polled again, nothing new
                    ts => {
                        fresh += 1;
                        last_ts = ts;
                    }
                }
                let dz = sh.deadzone.load(Relaxed) as f32 / 1000.0;
                if let Some(p) = decode(r, model, bt, dz) {
                    n += 1;
                    if preview != Some(p) {
                        sh.set_pad(&p);
                        ipc::signal();
                        preview = Some(p);
                    }
                    pending = (pad.is_some() && sent != Some(p)).then_some(p);
                }
            }
        }
        if let (Some(output), Some(p)) = (&pad, pending) {
            if last_sent.elapsed() >= gap {
                let error = if output.send(&p).is_ok() {
                    sent = Some(p);
                    0
                } else {
                    crate::win::ERR_SEND
                };
                sh.error.store(error, Relaxed);
                pending = None;
                last_sent = Instant::now();
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
    if let Some(output) = pad {
        let _ = output.send(&Pad::default());
    }
    sh.running.store(0, Relaxed);
    sh.slot.store(0, Relaxed);
    sh.error.store(0, Relaxed);
    sh.set_pad(&Pad::default());
    sh.controller.store(0, Relaxed);
    ipc::signal();
    for a in [&sh.rate, &sh.fresh, &sh.interval, &sh.jitter] {
        a.store(0, Relaxed);
    }
}
