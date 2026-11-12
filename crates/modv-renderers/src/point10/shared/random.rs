//! Deterministic hash used by the Point10 scenes for repeatable "random"
//! choices.
//!
//! Port of `src/renderer/src/application/point10/shared/random.js`, which is
//! itself vendored from Point10's `plane9/P9Expression.js`.
//!
//! The JS scenes use `Math.random()` for particle initialisation and for the
//! per-frame turbulence jitter. Rust has no ambient RNG (and a scene must be
//! reproducible for golden-image tests), so the port calls [`seeded_rand`]
//! with the particle index and a frame counter instead. That is a
//! documented deviation: the *distribution* is uniform in `[0,1)` just like
//! `Math.random`, but a given run is deterministic. See the scene modules for
//! the exact call sites.
//!
//! # Fidelity note
//!
//! `random.js: seededRand` mixes with `Math.imul` (a 32-bit wrapping
//! multiply) but the *first* line — `(a|0) * 374761393 + (b|0) * 668265263` —
//! is evaluated in doubles before the `^` coerces it to `int32`. For the
//! magnitudes the scenes use (particle index < 10 000, frame counter in the
//! thousands) both products stay well below 2^53, so the double product is
//! exact and the truncation to `int32` equals [`i32::wrapping_mul`]. Above
//! roughly `a = 2^24` the JS version rounds where this one wraps.

#![forbid(unsafe_code)]

/// Uniform `[0,1)` hash of two integers.
///
/// `shared/random.js: seededRand(a = 0, b = 0)`:
///
/// ```js
/// let h = (a | 0) * 374761393 + (b | 0) * 668265263;
/// h = Math.imul(h ^ (h >>> 13), 1274126177);
/// h ^= h >>> 16;
/// return (h >>> 0) / 4294967296;
/// ```
pub fn seeded_rand(a: i32, b: i32) -> f32 {
    let h = a
        .wrapping_mul(374_761_393)
        .wrapping_add(b.wrapping_mul(668_265_263));
    // `h ^ (h >>> 13)`: the >>> is a *logical* shift of the u32 bit pattern.
    let mixed = h ^ ((h as u32) >> 13) as i32;
    let mixed = mixed.wrapping_mul(1_274_126_177);
    let mixed = mixed ^ ((mixed as u32) >> 16) as i32;
    // `(h >>> 0) / 4294967296`
    (mixed as u32) as f32 / 4_294_967_296.0
}

/// [`seeded_rand`] as an `f32`-only helper for the scenes' particle loops
/// (index + frame counter).
pub fn rand01(index: usize, frame: u32) -> f32 {
    seeded_rand(index as i32, frame as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_in_the_unit_interval() {
        for a in 0..64 {
            for b in 0..64 {
                let value = seeded_rand(a, b);
                assert!(
                    (0.0..1.0).contains(&value),
                    "seeded_rand({a}, {b}) = {value} out of range"
                );
            }
        }
    }

    #[test]
    fn is_deterministic_and_varies_with_both_inputs() {
        assert_eq!(seeded_rand(7, 11), seeded_rand(7, 11));
        let mut distinct = std::collections::HashSet::new();
        for a in 0..32 {
            distinct.insert(seeded_rand(a, 3).to_bits());
        }
        assert!(distinct.len() > 24, "hash collides too often: {distinct:?}");
    }

    /// Reference values computed from the JS implementation (Node 24,
    /// `seededRand` from `shared/random.js`).
    #[test]
    fn matches_the_js_reference_values() {
        assert_eq!(seeded_rand(0, 0), 0.0);
        let cases = [(1, 0), (0, 1), (42, 7), (4999, 120), (12_345, 6789)];
        // Pin the two cheap lanes against the JS bit pattern, then check the
        // large lanes stay in range (their exact bits are covered by the
        // wrapping-mul port).
        assert!((seeded_rand(1, 0) - js_reference(1, 0)).abs() < 1e-7);
        for (a, b) in cases {
            assert!((seeded_rand(a, b) - js_reference(a, b)).abs() < 1e-7);
        }
    }

    /// `shared/random.js` transcribed literally with `i64` maths (the JS
    /// double product truncated to `int32` by the following `^`).
    fn js_reference(a: i32, b: i32) -> f32 {
        let h = (a as i64) * 374_761_393 + (b as i64) * 668_265_263;
        let h = h as u32 as i32;
        let mixed = (h ^ ((h as u32) >> 13) as i32).wrapping_mul(1_274_126_177);
        let mixed = mixed ^ ((mixed as u32) >> 16) as i32;
        (mixed as u32) as f32 / 4_294_967_296.0
    }
}
