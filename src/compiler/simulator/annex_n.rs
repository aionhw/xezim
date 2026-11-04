//! IEEE 1800-2017 Annex N: the random-number generators behind `$random`
//! and the `$dist_*` functions (§20.15.1, §20.15.2). The standard gives the
//! C source; every simulator that follows it produces the same sequence for
//! the same seed (`$random` with no seed starts 303379748, -1064739199, ...),
//! so testbench logs compare across tools.
//!
//! The C code runs on 32-bit `long`: the seed wraps at 32 bits and the
//! `uniform` mantissa trick reinterprets those 32 bits as an IEEE float.

/// Annex N `uniform`: advances `seed` once and returns a real in
/// `[start, end)` (or `[0, 2^31-1)` when `start >= end`).
fn uniform(seed: &mut i32, start: i32, end: i32) -> f64 {
    let d = 0.000_000_119_209_289_550_781_25_f64;
    if *seed == 0 {
        *seed = 259_341_593;
    }
    let (a, b) = if start >= end {
        (0.0, 2_147_483_647.0)
    } else {
        (start as f64, end as f64)
    };
    *seed = seed.wrapping_mul(69069).wrapping_add(1);
    let bits = ((*seed as u32) >> 9) | 0x3f80_0000;
    let mut c = f32::from_bits(bits) as f64;
    c += c * d;
    (b - a) * (c - 1.0) + a
}

fn normal(seed: &mut i32, mean: i32, deviation: i32) -> f64 {
    let mut s = 1.0f64;
    let mut v1 = 0.0f64;
    while s >= 1.0 || s == 0.0 {
        v1 = uniform(seed, -1, 1);
        let v2 = uniform(seed, -1, 1);
        s = v1 * v1 + v2 * v2;
    }
    let s = v1 * ((-2.0 * s.ln()) / s).sqrt();
    s * deviation as f64 + mean as f64
}

fn exponential(seed: &mut i32, mean: i32) -> f64 {
    let n = uniform(seed, 0, 1);
    if n != 0.0 { -n.ln() * mean as f64 } else { n }
}

fn poisson(seed: &mut i32, mean: i32) -> i32 {
    let mut n = 0i32;
    let p = (-(mean as f64)).exp();
    let mut q = uniform(seed, 0, 1);
    while p < q {
        n = n.wrapping_add(1);
        q *= uniform(seed, 0, 1);
    }
    n
}

fn chi_square(seed: &mut i32, deg_of_free: i32) -> f64 {
    let mut x = if deg_of_free % 2 != 0 {
        let x = normal(seed, 0, 1);
        x * x
    } else {
        0.0
    };
    let mut k = 2;
    while k <= deg_of_free {
        x += 2.0 * exponential(seed, 1);
        k += 2;
    }
    x
}

fn t(seed: &mut i32, deg_of_free: i32) -> f64 {
    let chi2 = chi_square(seed, deg_of_free);
    let root = (chi2 / deg_of_free as f64).sqrt();
    normal(seed, 0, 1) / root
}

fn erlangian(seed: &mut i32, k: i32, mean: i32) -> f64 {
    let mut x = 1.0f64;
    for _ in 1..=k {
        x *= uniform(seed, 0, 1);
    }
    -(mean as f64) * x.ln() / k as f64
}

/// C `(long) r` (truncation) of an Annex N real, wrapped to 32 bits.
fn trunc(r: f64) -> i64 {
    r as i64
}

/// `rtl_dist_*` rounding: half away from zero.
fn round_away(r: f64) -> i32 {
    if r >= 0.0 {
        trunc(r + 0.5) as i32
    } else {
        -(trunc(-r + 0.5) as i32)
    }
}

pub(super) fn dist_uniform(seed: &mut i32, start: i32, end: i32) -> i32 {
    if start >= end {
        return start;
    }
    if end != i32::MAX {
        let end = end + 1;
        let r = uniform(seed, start, end);
        let mut i = if r >= 0.0 { trunc(r) } else { trunc(r - 1.0) };
        if i < start as i64 {
            i = start as i64;
        }
        if i >= end as i64 {
            i = end as i64 - 1;
        }
        i as i32
    } else if start != i32::MIN {
        let start = start - 1;
        let r = uniform(seed, start, end) + 1.0;
        let mut i = if r >= 0.0 { trunc(r) } else { trunc(r - 1.0) };
        if i <= start as i64 {
            i = start as i64 + 1;
        }
        if i > end as i64 {
            i = end as i64;
        }
        i as i32
    } else {
        let r = (uniform(seed, start, end) + 2_147_483_648.0) / 4_294_967_295.0;
        let r = r * 4_294_967_296.0 - 2_147_483_648.0;
        let i = if r >= 0.0 { trunc(r) } else { trunc(r - 1.0) };
        i as i32
    }
}

/// `$random`: `rtl_dist_uniform(seed, LONG_MIN, LONG_MAX)`.
pub(super) fn random(seed: &mut i32) -> i32 {
    dist_uniform(seed, i32::MIN, i32::MAX)
}

pub(super) fn dist_normal(seed: &mut i32, mean: i32, sd: i32) -> i32 {
    round_away(normal(seed, mean, sd))
}

pub(super) fn dist_exponential(seed: &mut i32, mean: i32) -> i32 {
    if mean > 0 {
        round_away(exponential(seed, mean))
    } else {
        0
    }
}

pub(super) fn dist_poisson(seed: &mut i32, mean: i32) -> i32 {
    if mean > 0 { poisson(seed, mean) } else { 0 }
}

pub(super) fn dist_chi_square(seed: &mut i32, df: i32) -> i32 {
    if df > 0 {
        round_away(chi_square(seed, df))
    } else {
        0
    }
}

pub(super) fn dist_t(seed: &mut i32, df: i32) -> i32 {
    if df > 0 { round_away(t(seed, df)) } else { 0 }
}

pub(super) fn dist_erlang(seed: &mut i32, k: i32, mean: i32) -> i32 {
    if k > 0 {
        round_away(erlangian(seed, k, mean))
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_matches_the_reference_sequence() {
        let mut s = 0;
        assert_eq!(random(&mut s), 303_379_748);
        assert_eq!(s, -1_844_104_698);
        assert_eq!(random(&mut s), -1_064_739_199);
        assert_eq!(s, 1_082_744_015);
        assert_eq!(random(&mut s), -2_071_669_239);
        let mut s = 42;
        assert_eq!(random(&mut s), -2_144_582_656);
        assert_eq!(s, 2_900_899);
        assert_eq!(random(&mut s), 646_214_477);
        let mut s = -7;
        assert_eq!(random(&mut s), 2_147_000_831);
        assert_eq!(s, -483_482);
    }
}
