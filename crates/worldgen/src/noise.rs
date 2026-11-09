//! Deterministic hashing and gradient noise.
//!
//! Coordinates are f64 and lattice points are hashed as integers, so the noise is
//! identical and precise at any distance from the origin.

#[inline]
pub fn hash2(seed: u32, x: i32, y: i32) -> u32 {
    let mut h = seed
        .wrapping_add((x as u32).wrapping_mul(0x8da6_b343))
        .wrapping_add((y as u32).wrapping_mul(0xd816_3841));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^ (h >> 15)
}

#[inline]
pub fn hash3(seed: u32, x: i32, y: i32, z: i32) -> u32 {
    hash2(seed ^ (z as u32).wrapping_mul(0xcb1a_b31f), x, y)
}

/// Uniform in [0, 1).
#[inline]
pub fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn grad2(h: u32, x: f32, y: f32) -> f32 {
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const G: [(f32, f32); 8] = [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (D, D),
        (-D, D),
        (D, -D),
        (-D, -D),
    ];
    let (gx, gy) = G[(h >> 13) as usize & 7];
    gx * x + gy * y
}

/// 2D gradient noise, roughly in [-1, 1].
pub fn perlin2(seed: u32, x: f64, y: f64) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i64 as i32, fy as i64 as i32);
    let (tx, ty) = ((x - fx) as f32, (y - fy) as f32);
    let (ix1, iy1) = (ix.wrapping_add(1), iy.wrapping_add(1));
    let a = grad2(hash2(seed, ix, iy), tx, ty);
    let b = grad2(hash2(seed, ix1, iy), tx - 1.0, ty);
    let c = grad2(hash2(seed, ix, iy1), tx, ty - 1.0);
    let d = grad2(hash2(seed, ix1, iy1), tx - 1.0, ty - 1.0);
    let (u, v) = (fade(tx), fade(ty));
    lerp(lerp(a, b, u), lerp(c, d, u), v) * 1.41
}

#[inline]
fn grad3(h: u32, x: f32, y: f32, z: f32) -> f32 {
    // The 12 edge directions of a cube.
    match (h >> 13) % 12 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}

/// 3D gradient noise, roughly in [-1, 1].
pub fn perlin3(seed: u32, x: f64, y: f64, z: f64) -> f32 {
    let (fx, fy, fz) = (x.floor(), y.floor(), z.floor());
    let (ix, iy, iz) = (fx as i64 as i32, fy as i64 as i32, fz as i64 as i32);
    let (tx, ty, tz) = ((x - fx) as f32, (y - fy) as f32, (z - fz) as f32);
    let (u, v, w) = (fade(tx), fade(ty), fade(tz));
    let g = |dx: i32, dy: i32, dz: i32| {
        grad3(
            hash3(seed, ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz)),
            tx - dx as f32,
            ty - dy as f32,
            tz - dz as f32,
        )
    };
    let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w) * 0.97
}

/// Fractal sum of `octaves` layers; output roughly in [-1, 1].
pub fn fbm2(seed: u32, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    let (mut fx, mut fy) = (x, y);
    for o in 0..octaves {
        sum += perlin2(seed.wrapping_add(o.wrapping_mul(0x9e37_79b9)), fx, fy) * amp;
        norm += amp;
        amp *= gain;
        fx *= lacunarity;
        fy *= lacunarity;
    }
    sum / norm
}

/// Ridged fractal: sharp crests, output roughly in [0, 1].
pub fn ridged2(seed: u32, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 1.0, 0.0);
    let (mut fx, mut fy) = (x, y);
    for o in 0..octaves {
        let n = 1.0 - perlin2(seed.wrapping_add(o.wrapping_mul(0x9e37_79b9)), fx, fy).abs();
        sum += n * n * amp;
        norm += amp;
        amp *= gain;
        fx *= lacunarity;
        fy *= lacunarity;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_bounded_and_continuous() {
        let mut prev = perlin2(7, 0.0, 0.3);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 1..20_000 {
            let x = i as f64 * 0.01;
            let n = perlin2(7, x, 0.3 + x * 0.37);
            assert_eq!(n, perlin2(7, x, 0.3 + x * 0.37));
            assert!((n - prev).abs() < 0.08, "jump at {x}");
            prev = n;
            lo = lo.min(n);
            hi = hi.max(n);
        }
        assert!(lo >= -1.05 && hi <= 1.05, "range {lo}..{hi}");
        assert!(lo < -0.6 && hi > 0.6, "range too narrow: {lo}..{hi}");
    }

    #[test]
    fn noise_keeps_precision_far_from_origin() {
        // A step of 0.01 must still produce a small smooth change ten million units out.
        let base = 10_000_000.0;
        let a = perlin2(1, base + 0.50, base + 0.25);
        let b = perlin2(1, base + 0.51, base + 0.25);
        assert!(a != b && (a - b).abs() < 0.05);
        let c = perlin3(1, base + 0.50, 3.2, -base + 0.25);
        let d = perlin3(1, base + 0.51, 3.2, -base + 0.25);
        assert!(c != d && (c - d).abs() < 0.05);
    }
}
