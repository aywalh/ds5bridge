//! PlayStation controller report -> Xbox 360 state. Pure logic, no Windows.

// Xbox-360-Tastenbits (ViGEm Common.h, XUSB_BUTTON)
pub const UP: u16 = 0x0001;
pub const DOWN: u16 = 0x0002;
pub const LEFT: u16 = 0x0004;
pub const RIGHT: u16 = 0x0008;
pub const START: u16 = 0x0010;
pub const BACK: u16 = 0x0020;
pub const LS: u16 = 0x0040;
pub const RS: u16 = 0x0080;
pub const LB: u16 = 0x0100;
pub const RB: u16 = 0x0200;
pub const GUIDE: u16 = 0x0400;
pub const A: u16 = 0x1000;
pub const B: u16 = 0x2000;
pub const X: u16 = 0x4000;
pub const Y: u16 = 0x8000;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Pad {
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: f32, // -1..1, oben/rechts positiv
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SonyModel { DualShock4, DualSense }

impl SonyModel {
    pub fn from_usb_ids(vendor: u16, product: u16) -> Option<Self> {
        match (vendor, product) {
            (0x054c, 0x05c4 | 0x09cc) => Some(Self::DualShock4),
            (0x054c, 0x0ce6 | 0x0df2) => Some(Self::DualSense),
            _ => None,
        }
    }
}

struct Layout { lx: usize, ly: usize, rx: usize, ry: usize, l2: usize, r2: usize, b0: usize, b1: usize, b2: usize }

/// Vollreport (USB 0x01, Bluetooth 0x31 mit 1 Byte Versatz)
const FULL: Layout = Layout { lx: 1, ly: 2, rx: 3, ry: 4, l2: 5, r2: 6, b0: 8, b1: 9, b2: 10 };
/// Kurzreport, den der DualSense per Bluetooth schickt, bevor er umgeschaltet ist
const SHORT: Layout = Layout { lx: 1, ly: 2, rx: 3, ry: 4, b0: 5, b1: 6, b2: 7, l2: 8, r2: 9 };

/// 0..255 -> -1..1 mit Totzone
pub fn axis(v: u8, deadzone: f32) -> f32 {
    let x = (v as f32 - 128.0) / if v < 128 { 128.0 } else { 127.0 };
    if x.abs() < deadzone { 0.0 } else { x }
}

pub fn stick(v: f32) -> i16 {
    (v * 32767.0).round() as i16
}

/// None = kein Eingabe-Report (oder zu kurz)
pub fn decode(r: &[u8], model: SonyModel, bluetooth: bool, deadzone: f32) -> Option<Pad> {
    let (l, o) = match (model, bluetooth, *r.first()?) {
        (SonyModel::DualShock4, true, 0x11) => (&SHORT, 2),
        (SonyModel::DualShock4, _, 0x01) => (&SHORT, 0),
        (SonyModel::DualSense, true, 0x31) => (&FULL, 1),
        (SonyModel::DualSense, true, 0x01) => (&SHORT, 0),
        (SonyModel::DualSense, false, 0x01) => (&FULL, 0),
        _ => return None,
    };
    let b = |i: usize| r.get(i + o).copied();
    let (b0, b1, b2) = (b(l.b0)?, b(l.b1)?, b(l.b2)?);
    let mut buttons = match b0 & 0x0F {
        0 => UP, 1 => UP | RIGHT, 2 => RIGHT, 3 => DOWN | RIGHT,
        4 => DOWN, 5 => DOWN | LEFT, 6 => LEFT, 7 => UP | LEFT, _ => 0,
    };
    for (byte, bit, xb) in [
        (b0, 0x10, X), (b0, 0x20, A), (b0, 0x40, B), (b0, 0x80, Y),
        (b1, 0x01, LB), (b1, 0x02, RB), (b1, 0x10, BACK), (b1, 0x20, START),
        (b1, 0x40, LS), (b1, 0x80, RS), (b2, 0x01, GUIDE),
    ] {
        if byte & bit != 0 {
            buttons |= xb;
        }
    }
    Some(Pad {
        buttons,
        lt: b(l.l2)?,
        rt: b(l.r2)?,
        lx: axis(b(l.lx)?, deadzone),
        ly: -axis(b(l.ly)?, deadzone), // DualSense oben = 0, Xbox oben = positiv
        rx: axis(b(l.rx)?, deadzone),
        ry: -axis(b(l.ry)?, deadzone),
    })
}

/// Interner Sensor-Zeitstempel des Controllers (Einheit 1/3 µs), nur im Vollreport.
/// Gleicher Wert wie beim letzten Report = keine neuen Daten, nur erneut abgefragt.
pub fn sensor_time(r: &[u8], bluetooth: bool) -> Option<u32> {
    let o = match *r.first()? {
        0x31 => 1,
        0x01 if !bluetooth => 0,
        _ => return None,
    };
    Some(u32::from_le_bytes(r.get(28 + o..32 + o)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_timestamp() {
        let mut r = usb();
        r[28..32].copy_from_slice(&12000u32.to_le_bytes());
        assert_eq!(sensor_time(&r, false), Some(12000));
        let mut bt = vec![0x31, 0x00];
        bt.extend_from_slice(&r[1..]);
        assert_eq!(sensor_time(&bt, true), Some(12000));
        assert_eq!(sensor_time(&r, true), None); // BT-Kurzreport hat keinen
    }

    fn usb() -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0x01;
        r[1] = 128; r[3] = 128; r[4] = 128; // Sticks mittig
        r[2] = 0;                           // LY ganz oben
        r[5] = 255;                         // L2 voll
        r[8] = 0x20;                        // Kreuz, D-Pad Nord
        r[9] = 0x01;                        // L1
        r[10] = 0x01;                       // PS
        r
    }

    #[test]
    fn usb_full_report() {
        let p = decode(&usb(), SonyModel::DualSense, false, 0.08).unwrap();
        assert_eq!(p.buttons, A | UP | LB | GUIDE);
        assert_eq!((p.lx, p.ly, p.rx, p.ry, p.lt, p.rt), (0.0, 1.0, 0.0, 0.0, 255, 0));
        assert_eq!(stick(p.ly), 32767);
    }

    #[test]
    fn bluetooth_full_report_same_as_usb() {
        let u = usb();
        let mut bt = vec![0x31, 0x00];
        bt.extend_from_slice(&u[1..]);
        bt.resize(78, 0);
        assert_eq!(decode(&bt, SonyModel::DualSense, true, 0.08), decode(&u, SonyModel::DualSense, false, 0.08));
    }

    #[test]
    fn bluetooth_short_report() {
        let mut s = vec![0u8; 78];
        s[..9].copy_from_slice(&[0x01, 128, 0, 128, 128, 0x20, 0, 0, 255]);
        let p = decode(&s, SonyModel::DualSense, true, 0.08).unwrap();
        assert_eq!(p.buttons, A | UP);
        assert_eq!(p.lt, 255);
    }

    #[test]
    fn edges() {
        assert_eq!(decode(&[0x05; 64], SonyModel::DualSense, false, 0.08), None);
        assert_eq!(decode(&[0x01, 1, 2], SonyModel::DualSense, false, 0.08), None);
        assert_eq!(axis(255, 0.08), 1.0);
        assert_eq!(axis(0, 0.08), -1.0);
        assert_eq!(axis(0x81, 0.08), 0.0); // leichter Ruhe-Versatz -> Totzone
        let mut r = usb();
        r[8] = 0x08; // D-Pad neutral, keine Gesichtstaste
        assert_eq!(decode(&r, SonyModel::DualSense, false, 0.08).unwrap().buttons, LB | GUIDE);
    }
}

#[cfg(test)]
mod dualshock_tests {
    use super::*;

    #[test]
    fn dualshock_usb_and_bluetooth_have_the_same_controls() {
        let usb = [1, 255, 128, 128, 0, 0x21, 0x43, 1, 65, 170];
        let mut bt = [0u8; 78];
        bt[0] = 0x11;
        bt[3..12].copy_from_slice(&usb[1..]);
        let p = decode(&usb, SonyModel::DualShock4, false, 0.0).unwrap();
        assert_eq!(decode(&bt, SonyModel::DualShock4, true, 0.0), Some(p));
        assert_eq!(p.buttons, A | UP | RIGHT | LB | RB | LS | GUIDE);
        assert_eq!((p.lt, p.rt, p.lx, p.ry), (65, 170, 1.0, 1.0));
        assert!(decode(&bt[..10], SonyModel::DualShock4, true, 0.0).is_none());
    }
}
