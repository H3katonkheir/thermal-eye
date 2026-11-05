//! The app mark, rendered in code so it needs no asset files.

const SIZE: usize = 64;
const SUPERSAMPLE: usize = 4;

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Colour and coverage at a point in [-1, 1]: a violet tile with an eye whose
/// iris runs from cool to hot.
fn shade(x: f32, y: f32) -> Option<[f32; 3]> {
    // Rounded square.
    let (qx, qy) = (x.abs() - 0.72, y.abs() - 0.72);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    if outside > 0.28 {
        return None;
    }
    let tile = mix([0.55, 0.36, 0.96], [0.23, 0.51, 0.96], (x + y + 2.0) / 4.0);

    // Almond: the overlap of two circles.
    let (radius, offset) = (0.98, 0.55);
    let in_eye = (x * x + (y - offset).powi(2)).sqrt() < radius
        && (x * x + (y + offset).powi(2)).sqrt() < radius;
    if !in_eye {
        return Some(tile);
    }

    let distance = (x * x + y * y).sqrt();
    if distance < 0.15 {
        Some([0.09, 0.09, 0.14])
    } else if distance < 0.38 {
        Some(mix([0.98, 0.75, 0.14], [0.94, 0.27, 0.27], (y + 0.38) / 0.76))
    } else {
        Some([0.96, 0.96, 0.99])
    }
}

/// Straight-alpha RGBA pixels, `SIZE` by `SIZE`.
pub fn pixels() -> (usize, Vec<u8>) {
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    for py in 0..SIZE {
        for px in 0..SIZE {
            let mut color = [0.0f32; 3];
            let mut coverage = 0.0f32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let u = (px as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32) / SIZE as f32;
                    let v = (py as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32) / SIZE as f32;
                    if let Some(c) = shade(u * 2.0 - 1.0, v * 2.0 - 1.0) {
                        (0..3).for_each(|i| color[i] += c[i]);
                        coverage += 1.0;
                    }
                }
            }
            for channel in color {
                let value = if coverage > 0.0 { channel / coverage } else { 0.0 };
                rgba.push((value * 255.0).round() as u8);
            }
            rgba.push((coverage / samples * 255.0).round() as u8);
        }
    }
    (SIZE, rgba)
}
