//! The live DualSense preview: a dark controller drawn entirely in code (no images from anyone
//! else) on a quiet blue halo. Pressed inputs turn white, sticks move, triggers fill.
//! Grid: 420 x 280. The outline is a mirrored chain of curves, filled with a triangulated mesh.

use crate::{decode::*, ipc::Status};
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, TextureHandle};
use std::sync::OnceLock;

const WING: Color32 = Color32::from_rgb(30, 31, 36);
const PLATE: Color32 = Color32::from_rgb(15, 16, 19);
const WELL: Color32 = Color32::from_rgb(9, 10, 12);
const EDGE: Color32 = Color32::from_rgb(72, 76, 88);
const SYMBOL: Color32 = Color32::from_rgb(150, 155, 166);
const LIT: Color32 = Color32::from_rgb(244, 247, 255);
const INK: Color32 = Color32::from_rgb(32, 35, 42);
const BLUE: Color32 = Color32::from_rgb(66, 105, 230);

type Pt = (f32, f32);

/// Points along a cubic curve from `a` (excluded) to `d`.
fn cubic(out: &mut Vec<Pt>, a: Pt, b: Pt, c: Pt, d: Pt) {
    for i in 1..=12 {
        let t = i as f32 / 12.0;
        let u = 1.0 - t;
        let f = |a: f32, b: f32, c: f32, d: f32| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
        out.push((f(a.0, b.0, c.0, d.0), f(a.1, b.1, c.1, d.1)));
    }
}

/// Left half from the top centre down to the bottom centre as curves [control, control, end];
/// returns the whole outline with the right half mirrored.
fn mirrored(start: Pt, segs: &[[Pt; 3]]) -> Vec<Pt> {
    let mut l = vec![start];
    for s in segs {
        let cur = *l.last().unwrap();
        cubic(&mut l, cur, s[0], s[1], s[2]);
    }
    let n = l.len();
    let mut full = l.clone();
    full.extend(l[1..n - 1].iter().rev().map(|&(x, y)| (420.0 - x, y)));
    full
}

/// Ear clipping: triangle indices for a simple (non-self-intersecting) polygon.
fn triangulate(pts: &[Pt]) -> Vec<u32> {
    let cross = |a: Pt, b: Pt, c: Pt| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let area: f32 = (0..pts.len()).map(|i| cross((0.0, 0.0), pts[i], pts[(i + 1) % pts.len()])).sum();
    let s = area.signum();
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    let mut tris = vec![];
    while idx.len() > 3 {
        let n = idx.len();
        let tri = |i: usize| (idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]);
        let ear = (0..n).find(|&i| {
            let (a, b, c) = tri(i);
            let (a, b, c) = (pts[a], pts[b], pts[c]);
            cross(a, b, c) * s > 1e-6
                && !idx.iter().any(|&j| {
                    let q = pts[j];
                    q != a && q != b && q != c && cross(a, b, q) * s >= 0.0 && cross(b, c, q) * s >= 0.0 && cross(c, a, q) * s >= 0.0
                })
        });
        match ear {
            Some(i) => {
                let (a, b, c) = tri(i);
                tris.extend([a as u32, b as u32, c as u32]);
                idx.remove(i);
            }
            // only straight (collinear) points left to drop
            None => match (0..n).find(|&i| cross(pts[tri(i).0], pts[tri(i).1], pts[tri(i).2]).abs() <= 1e-6) {
                Some(i) => {
                    idx.remove(i);
                }
                None => break,
            },
        }
    }
    if idx.len() == 3 {
        tris.extend(idx.iter().map(|&v| v as u32));
    }
    tris
}

struct Shell {
    body: Vec<Pt>,
    body_t: Vec<u32>,
    plate: Vec<Pt>, // the darker centre, like the real two-tone shell
    plate_t: Vec<u32>,
}

/// Outlines are built and triangulated once.
fn shell() -> &'static Shell {
    static S: OnceLock<Shell> = OnceLock::new();
    S.get_or_init(|| {
        let body = mirrored(
            (210.0, 56.0),
            &[
                [(190.0, 56.0), (170.0, 56.0), (152.0, 56.0)],
                [(128.0, 56.0), (116.0, 46.0), (92.0, 46.0)],
                [(64.0, 46.0), (50.0, 58.0), (44.0, 80.0)],
                [(34.0, 118.0), (22.0, 170.0), (20.0, 206.0)],
                [(18.0, 248.0), (40.0, 270.0), (68.0, 270.0)],
                [(96.0, 270.0), (108.0, 250.0), (118.0, 228.0)],
                [(132.0, 200.0), (160.0, 196.0), (210.0, 196.0)],
            ],
        );
        let plate = mirrored(
            (210.0, 56.0),
            &[
                [(190.0, 56.0), (166.0, 56.0), (148.0, 56.0)],
                [(140.0, 120.0), (114.0, 160.0), (118.0, 228.0)],
                [(132.0, 200.0), (160.0, 196.0), (210.0, 196.0)],
            ],
        );
        Shell { body_t: triangulate(&body), plate_t: triangulate(&plate), body, plate }
    })
}

/// A soft blue halo behind the controller, made once as a small texture.
fn glow(p: &Painter, area: Rect) {
    static T: OnceLock<TextureHandle> = OnceLock::new();
    let t = T.get_or_init(|| {
        let mut rgba = Vec::with_capacity(256 * 256 * 4);
        for y in 0..256 {
            for x in 0..256 {
                let d = ((x as f32 - 127.5) / 127.5).powi(2) + ((y as f32 - 127.5) / 127.5).powi(2);
                let alpha = ((-3.4 * d).exp() * (1.0 - d).max(0.0) * 145.0).round() as u8;
                rgba.extend_from_slice(&[66, 105, 230, alpha]);
            }
        }
        p.ctx().load_texture("dualsense-halo", egui::ColorImage::from_rgba_unmultiplied([256, 256], &rgba), egui::TextureOptions::LINEAR)
    });
    let r = Rect::from_center_size(area.center() + vec2(0.0, area.height() * 0.12), vec2(area.width() * 1.15, area.height() * 1.2));
    p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

/// Everything a part needs to draw itself.
struct Pen<'a> {
    p: &'a Painter,
    o: Pos2,
    k: f32,
    pad: Pad,
    st: Stroke,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.o + vec2(x, y) * self.k
    }
    fn on(&self, bit: u16) -> bool {
        self.pad.buttons & bit != 0
    }
    fn pick(&self, on: bool) -> (Color32, Stroke) {
        if on { (LIT, Stroke::new(self.st.width, LIT)) } else { (WELL, self.st) }
    }
    fn fill(&self, pts: &[Pt], tris: &[u32], color: Color32) {
        let mut m = egui::Mesh::default();
        for &(x, y) in pts {
            m.colored_vertex(self.at(x, y), color);
        }
        m.indices = tris.to_vec();
        self.p.add(Shape::mesh(m));
    }
    fn trace(&self, pts: &[Pt], stroke: Stroke) {
        self.p.add(Shape::closed_line(pts.iter().map(|&(x, y)| self.at(x, y)).collect(), stroke));
    }
    fn trigger(&self, x0: f32, v: u8, name: &str) {
        let t = Rect::from_min_max(self.at(x0, 0.0), self.at(x0 + 56.0, 30.0));
        let cr = CornerRadius { nw: (14.0 * self.k) as u8, ne: (14.0 * self.k) as u8, sw: (3.0 * self.k) as u8, se: (3.0 * self.k) as u8 };
        self.p.rect_filled(t, cr, WING);
        if v > 0 {
            let h = t.height() * v as f32 / 255.0;
            self.p.rect_filled(Rect::from_min_max(pos2(t.left(), t.bottom() - h), t.max), if h > 14.0 * self.k { cr } else { CornerRadius::ZERO }, LIT);
        }
        self.p.rect_stroke(t, cr, self.st, StrokeKind::Inside);
        self.p.text(t.center(), Align2::CENTER_CENTER, name, FontId::proportional(9.0 * self.k), if v > 150 { INK } else { SYMBOL });
    }
    fn bumper(&self, x0: f32, bit: u16, name: &str) {
        let b = Rect::from_min_max(self.at(x0, 32.0), self.at(x0 + 72.0, 41.0));
        let on = self.on(bit);
        self.p.rect_filled(b, 4.5 * self.k, if on { LIT } else { WING });
        self.p.rect_stroke(b, 4.5 * self.k, self.st, StrokeKind::Inside);
        self.p.text(b.center(), Align2::CENTER_CENTER, name, FontId::proportional(7.0 * self.k), if on { INK } else { SYMBOL });
    }
    fn round(&self, c: Pt, r: f32, on: bool) {
        let (fill, stroke) = self.pick(on);
        self.p.circle_filled(self.at(c.0, c.1), r * self.k, fill);
        self.p.circle_stroke(self.at(c.0, c.1), r * self.k, stroke);
    }
    /// Convex key relative to a centre.
    fn key(&self, c: Pt, pts: &[Pt], on: bool) {
        let (fill, stroke) = self.pick(on);
        self.p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| self.at(c.0 + x, c.1 + y)).collect(), fill, stroke));
    }
    /// Well, a cap that follows the stick, a thin line from the centre while it is pushed.
    fn stick(&self, c: Pt, x: f32, y: f32, bit: u16) {
        let k = self.k;
        let cc = self.at(c.0, c.1);
        self.p.circle_filled(cc, 23.0 * k, WELL);
        self.p.circle_stroke(cc, 23.0 * k, self.st);
        let d = cc + vec2(x.clamp(-1.0, 1.0), -y.clamp(-1.0, 1.0)) * 9.0 * k;
        let pressed = self.on(bit);
        if x != 0.0 || y != 0.0 {
            self.p.line_segment([cc, d], Stroke::new(1.4 * k, LIT.gamma_multiply(0.7)));
        }
        self.p.circle_filled(d, 15.0 * k, if pressed { LIT } else { WING });
        self.p.circle_stroke(d, 15.0 * k, if pressed { Stroke::new(self.st.width, LIT) } else { self.st });
        self.p.circle_stroke(d, 9.5 * k, Stroke::new(self.st.width * 0.8, if pressed { INK.gamma_multiply(0.5) } else { EDGE.gamma_multiply(0.6) }));
    }
}

/// The live controller, fitted into `area`. A disconnected controller shows nothing pressed.
pub fn paint(p: &Painter, area: Rect, status: &Status) {
    let p = &p.with_clip_rect(area.intersect(p.clip_rect()));
    let live = status.controller != 0;
    glow(p, area);
    let k = (area.width() / 430.0).min(area.height() / 290.0);
    let pen = Pen {
        p,
        o: area.center() - vec2(210.0, 135.0) * k,
        k,
        pad: if live { status.pad } else { Pad::default() },
        st: Stroke::new(k.max(1.0), EDGE),
    };
    let pad = pen.pad;

    pen.trigger(70.0, pad.lt, "L2");
    pen.trigger(294.0, pad.rt, "R2");
    pen.bumper(62.0, LB, "L1");
    pen.bumper(286.0, RB, "R1");

    let sh = shell();
    pen.fill(&sh.body, &sh.body_t, WING);
    pen.fill(&sh.plate, &sh.plate_t, PLATE);
    pen.trace(&sh.plate, Stroke::new(pen.st.width, EDGE.gamma_multiply(0.5)));
    pen.trace(&sh.body, pen.st);

    // touchpad: flat top, bottom edge curving down in the middle
    let mut tp = vec![(152.0, 60.0), (268.0, 60.0), (268.0, 106.0)];
    cubic(&mut tp, (268.0, 106.0), (250.0, 122.0), (226.0, 124.0), (210.0, 124.0));
    cubic(&mut tp, (210.0, 124.0), (194.0, 124.0), (170.0, 122.0), (152.0, 106.0));
    p.add(Shape::convex_polygon(tp.iter().map(|&(x, y)| pen.at(x, y)).collect(), Color32::from_rgb(20, 21, 25), pen.st));
    // blue light bars beside the touchpad
    for (a, b) in [((146.0, 64.0), (142.0, 110.0)), ((274.0, 64.0), (278.0, 110.0))] {
        let color = if live { BLUE } else { EDGE.gamma_multiply(0.6) };
        p.line_segment([pen.at(a.0, a.1), pen.at(b.0, b.1)], Stroke::new(2.4 * k, color));
        if live {
            p.line_segment([pen.at(a.0, a.1), pen.at(b.0, b.1)], Stroke::new(7.0 * k, BLUE.gamma_multiply(0.18)));
        }
    }
    // create / options
    for (x, bit) in [(134.0, BACK), (286.0, START)] {
        let (fill, stroke) = pen.pick(pen.on(bit));
        let r = Rect::from_center_size(pen.at(x, 70.0), vec2(5.0, 13.0) * k);
        p.rect_filled(r, 2.5 * k, fill);
        p.rect_stroke(r, 2.5 * k, stroke, StrokeKind::Inside);
    }
    // d-pad: four arrow keys pointing at the centre
    let arrow = [(-6.5, -21.0), (6.5, -21.0), (6.5, -9.0), (0.0, -3.5), (-6.5, -9.0)];
    for (rot, bit) in [(0, UP), (1, RIGHT), (2, DOWN), (3, LEFT)] {
        let pts: Vec<Pt> = arrow.iter().map(|&(x, y)| match rot { 0 => (x, y), 1 => (-y, x), 2 => (-x, -y), _ => (y, -x) }).collect();
        pen.key((96.0, 112.0), &pts, pen.on(bit));
    }
    // face buttons with thin symbols
    let fc = (324.0, 112.0);
    for (i, (dx, dy, bit)) in [(0.0, -1.0, Y), (1.0, 0.0, B), (0.0, 1.0, A), (-1.0, 0.0, X)].into_iter().enumerate() {
        let c = (fc.0 + dx * 21.0, fc.1 + dy * 21.0);
        let pressed = pen.on(bit);
        pen.round(c, 10.0, pressed);
        let (cc, m) = (pen.at(c.0, c.1), 4.4 * k);
        let st = Stroke::new((1.1 * k).max(0.9), if pressed { INK } else { SYMBOL });
        match i {
            0 => {
                p.add(Shape::closed_line(vec![cc + vec2(0.0, -m), cc + vec2(m * 0.95, m * 0.7), cc + vec2(-m * 0.95, m * 0.7)], st));
            }
            1 => {
                p.circle_stroke(cc, m, st);
            }
            2 => {
                p.line_segment([cc + vec2(-m, -m) * 0.8, cc + vec2(m, m) * 0.8], st);
                p.line_segment([cc + vec2(-m, m) * 0.8, cc + vec2(m, -m) * 0.8], st);
            }
            _ => {
                p.rect_stroke(Rect::from_center_size(cc, vec2(m, m) * 1.6), 1.0, st, StrokeKind::Middle);
            }
        }
    }
    // speaker grille, PS button, mute button
    for row in 0..3 {
        for col in 0..(7 - row % 2) {
            let x = 210.0 - (6 - row % 2) as f32 * 3.0 + col as f32 * 6.0;
            p.circle_filled(pen.at(x, 134.0 + row as f32 * 5.0), 1.0 * k, EDGE);
        }
    }
    pen.round((210.0, 162.0), 8.0, pen.on(GUIDE));
    p.rect_stroke(Rect::from_center_size(pen.at(210.0, 180.0), vec2(12.0, 4.0) * k), 2.0 * k, pen.st, StrokeKind::Inside);

    pen.stick((156.0, 164.0), pad.lx, pad.ly, LS);
    pen.stick((264.0, 164.0), pad.rx, pad.ry, RS);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The outlines must triangulate completely, or the controller would have holes.
    #[test]
    fn outlines_fill_completely() {
        let area = |pts: &[Pt]| (0..pts.len()).map(|i| { let (a, b) = (pts[i], pts[(i + 1) % pts.len()]); a.0 * b.1 - b.0 * a.1 }).sum::<f32>().abs() / 2.0;
        let tri_area = |pts: &[Pt], t: &[u32]| t.chunks(3).map(|c| area(&[pts[c[0] as usize], pts[c[1] as usize], pts[c[2] as usize]])).sum::<f32>();
        let sh = shell();
        assert!((area(&sh.body) - tri_area(&sh.body, &sh.body_t)).abs() < 1.0, "body");
        assert!((area(&sh.plate) - tri_area(&sh.plate, &sh.plate_t)).abs() < 1.0, "plate");
        // an L shape, the simplest non-convex case
        let l = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)];
        assert_eq!(triangulate(&l).len(), 12);
    }
}
