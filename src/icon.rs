//! The app icon, drawn in code (no image files): the DS5 Bridge mark, an ivory arch with two
//! champagne dots, on a graphite rounded square. Same shape as `gui::mark`.

/// What the mark covers at a point in a 0..1 square: 0 nothing, 1 the arch, 2 a dot.
fn mark(x: f32, y: f32) -> u8 {
    // the mark lives on a 40 x 40 grid, centred with a margin
    let (mx, my) = ((x - 0.14) / 0.72 * 40.0, (y - 0.14) / 0.72 * 40.0);
    let d = |cx: f32, cy: f32| ((mx - cx).powi(2) + (my - cy).powi(2)).sqrt();
    let h = 2.9; // half the line width
    let arch = (my <= 20.0 && (d(20.0, 20.0) - 14.0).abs() <= h)
        || ((mx - 6.0).abs() <= h || (mx - 34.0).abs() <= h) && (20.0..=34.0).contains(&my)
        || d(6.0, 34.0) <= h
        || d(34.0, 34.0) <= h;
    if arch {
        1
    } else if d(15.0, 29.0) <= 2.7 || d(25.0, 29.0) <= 2.7 {
        2
    } else {
        0
    }
}

/// RGBA pixels, `n` x `n`, 4x supersampled.
pub fn rgba(n: u32) -> Vec<u8> {
    const BG: [f32; 3] = [30.0, 30.0, 34.0];
    const INK: [f32; 3] = [232.0, 228.0, 220.0];
    const DOT: [f32; 3] = [201.0, 179.0, 126.0];
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    let s = 4;
    for py in 0..n {
        for px in 0..n {
            let (mut bg, mut ink, mut dot) = (0u32, 0u32, 0u32);
            for sy in 0..s {
                for sx in 0..s {
                    let x = (px as f32 + (sx as f32 + 0.5) / s as f32) / n as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / s as f32) / n as f32;
                    // rounded square background
                    let r = 0.22;
                    let qx = (r - x).max(x - (1.0 - r)).max(0.0);
                    let qy = (r - y).max(y - (1.0 - r)).max(0.0);
                    if qx * qx + qy * qy <= r * r {
                        bg += 1;
                        match mark(x, y) {
                            1 => ink += 1,
                            2 => dot += 1,
                            _ => {}
                        }
                    }
                }
            }
            let a = bg as f32 / (s * s) as f32;
            let (wi, wd) = if bg > 0 { (ink as f32 / bg as f32, dot as f32 / bg as f32) } else { (0.0, 0.0) };
            let c = |i: usize| (BG[i] * (1.0 - wi - wd) + INK[i] * wi + DOT[i] * wd).round() as u8;
            out.extend_from_slice(&[c(0), c(1), c(2), (a * 255.0).round() as u8]);
        }
    }
    out
}

/// Icon image as Windows stores it inside .ico files and resources: a 32-bit DIB
/// (BITMAPINFOHEADER, bottom-up BGRA, then an all-zero AND mask).
pub fn dib(n: u32) -> Vec<u8> {
    let px = rgba(n);
    let mask_row = n.div_ceil(32) * 4;
    let mut v = Vec::new();
    for x in [40u32, n, n * 2] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&1u16.to_le_bytes()); // planes
    v.extend_from_slice(&32u16.to_le_bytes()); // bits
    v.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    v.extend_from_slice(&(n * n * 4 + mask_row * n).to_le_bytes());
    v.extend_from_slice(&[0u8; 16]);
    for y in (0..n).rev() {
        for x in 0..n {
            let i = ((y * n + x) * 4) as usize;
            v.extend_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
        }
    }
    v.resize(v.len() + (mask_row * n) as usize, 0);
    v
}

/// A complete .ico file with the common sizes.
pub fn ico_file() -> Vec<u8> {
    let sizes = [16u32, 24, 32, 48, 64, 256];
    let images: Vec<Vec<u8>> = sizes.iter().map(|&n| dib(n)).collect();
    let mut v = Vec::new();
    v.extend_from_slice(&[0, 0, 1, 0]);
    v.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (n, img) in sizes.iter().zip(&images) {
        let b = if *n >= 256 { 0 } else { *n as u8 };
        v.extend_from_slice(&[b, b, 0, 0]);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&32u16.to_le_bytes());
        v.extend_from_slice(&(img.len() as u32).to_le_bytes());
        v.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        v.extend_from_slice(&img);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_sizes() {
        assert_eq!(rgba(16).len(), 16 * 16 * 4);
        assert_eq!(dib(32).len(), 40 + 32 * 32 * 4 + 4 * 32);
        let ico = ico_file();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 6, 0]);
        // top of the arch is ivory, corner is transparent
        let p = rgba(64);
        let at = |x: usize, y: usize| &p[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(at(32, 15), &[232, 228, 220, 255]);
        assert_eq!(at(0, 0)[3], 0);
    }
}
