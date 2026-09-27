//! Deterministic noise for irregular, natural-looking shapes: hashes, smooth
//! 1D and 2D gradient noise, and fractal sums of it (fbm, ridged). Everything
//! is a pure function of its inputs, so shapes built from it stay put from
//! frame to frame without storing anything.

/// Pseudo-random number in [0, 1) for an integer cell and a seed.
pub fn hash(i: i64, seed: u64) -> f64 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn hash2(x: i64, y: i64, seed: u64) -> f64 {
    hash(x.wrapping_mul(0x8DA6_B343) ^ y.wrapping_mul(0xD816_3841), seed)
}

/// Quintic fade: smooth in value, slope and curvature at the lattice.
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 1D gradient noise, about −1..1, zero at integer x.
pub fn noise1(x: f64, seed: u64) -> f64 {
    let i = x.floor();
    let t = x - i;
    let slope = |k: f64| hash(k as i64, seed) * 2.0 - 1.0;
    let a = slope(i) * t;
    let b = slope(i + 1.0) * (t - 1.0);
    (a + (b - a) * fade(t)) * 2.0
}

/// 2D gradient (Perlin) noise, about −1..1.
pub fn noise2(x: f64, y: f64, seed: u64) -> f64 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (x - xi, y - yi);
    let grad = |dx: f64, dy: f64| {
        let a = hash2((xi + dx) as i64, (yi + dy) as i64, seed) * std::f64::consts::TAU;
        a.cos() * (tx - dx) + a.sin() * (ty - dy)
    };
    let (u, v) = (fade(tx), fade(ty));
    let a = grad(0.0, 0.0) + (grad(1.0, 0.0) - grad(0.0, 0.0)) * u;
    let b = grad(0.0, 1.0) + (grad(1.0, 1.0) - grad(0.0, 1.0)) * u;
    (a + (b - a) * v) * 1.41
}

/// Fractal 1D noise: `octaves` layers, each twice as fine and half as strong.
/// About −1..1.
pub fn fbm1(x: f64, octaves: u32, seed: u64) -> f64 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += noise1(x * freq, seed.wrapping_add(o as u64 * 101)) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

/// Fractal 2D noise, about −1..1.
pub fn fbm2(x: f64, y: f64, octaves: u32, seed: u64) -> f64 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += noise2(x * freq, y * freq, seed.wrapping_add(o as u64 * 101)) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

/// Ridged fractal 1D noise, 0..1: sharp crests like eroded rock ridges.
pub fn ridge1(x: f64, octaves: u32, seed: u64) -> f64 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        let n = 1.0 - noise1(x * freq, seed.wrapping_add(o as u64 * 131)).abs().min(1.0);
        sum += n * n * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.1;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_bounded_and_deterministic() {
        for i in 0..2000 {
            let x = i as f64 * 0.137 - 50.0;
            let y = i as f64 * 0.071 + 3.0;
            for v in [noise1(x, 3), noise2(x, y, 5), fbm1(x, 5, 7), fbm2(x, y, 4, 9)] {
                assert!((-1.5..=1.5).contains(&v), "{v}");
            }
            assert!((0.0..=1.0).contains(&ridge1(x, 4, 1)));
            assert_eq!(noise2(x, y, 5), noise2(x, y, 5));
        }
    }
}
