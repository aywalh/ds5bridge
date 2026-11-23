//! The app icon, drawn in code (no image files): a white controller on a red rounded square.

/// Coverage (0..1) of the controller silhouette at a point in a 0..1 square.
fn controller(x: f32, y: f32) -> bool {
    let circle = |cx: f32, cy: f32, r: f32| (x - cx).powi(2) + (y - cy).powi(2) <= r * r;
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        let qx = (x0 + r - x).max(x - (x1 - r)).max(0.0);
        let qy = (y0 + r - y).max(y - (y1 - r)).max(0.0);
        x >= x0 && x <= x1 && y >= y0 && y <= y1 && qx * qx + qy * qy <= r * r
    };
    let body = rrect(0.13, 0.31, 0.87, 0.58, 0.12) || circle(0.26, 0.64, 0.14) || circle(0.74, 0.64, 0.14);
    // d-pad "+" on the left and four face buttons on the right, cut out so the red shows through
    let dpad = (x - 0.30).abs() < 0.075 && (y - 0.445).abs() < 0.022 || (x - 0.30).abs() < 0.022 && (y - 0.445).abs() < 0.075;
    let face = [(0.70, 0.385), (0.70, 0.505), (0.64, 0.445), (0.76, 0.445)].iter().any(|&(cx, cy)| circle(cx, cy, 0.03));
    body && !dpad && !face
}

/// RGBA pixels, `n` x `n`, 4x supersampled.
pub fn rgba(n: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    let s = 4;
    for py in 0..n {
        for px in 0..n {
            let (mut bg, mut fg) = (0u32, 0u32);
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
                        if controller(x, y) {
                            fg += 1;
                        }
                    }
                }
            }
            let total = (s * s) as f32;
            let a = bg as f32 / total;
            let w = if bg > 0 { fg as f32 / bg as f32 } else { 0.0 };
            let mix = |red: f32, white: f32| (red + (white - red) * w).round() as u8;
            out.extend_from_slice(&[mix(228.0, 255.0), mix(30.0, 255.0), mix(42.0, 255.0), (a * 255.0).round() as u8]);
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
        // centre is white (controller body), corner is transparent
        let p = rgba(64);
        let at = |x: usize, y: usize| &p[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(at(32, 26), &[255, 255, 255, 255]);
        assert_eq!(at(0, 0)[3], 0);
    }
}
