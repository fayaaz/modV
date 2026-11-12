//! Phrase colour logic — port of
//! `src/renderer/src/application/point10/shared/phraseColor.js`.
//!
//! Three pieces, all CPU-side (the scenes write the results into uniforms):
//!
//! * [`hue_rotate`] — the CPU twin of TSL's `hue()`: a Rodrigues rotation of
//!   the RGB vector about the white axis.
//! * [`equalized_hue`] — `hueRotate` plus the luminance equalisation that makes
//!   every ROYGBIV state read as bright as the green one.
//! * [`drop_color_pulse`] — the drop detector that flips the hue on musical
//!   phrase boundaries (`DropColorState` replaces the JS `{ hue, t, low,
//!   phraseMax, prev, peakRise }` object).
//!
//! Used by WaterRipple (equalised palette), Mycelium (hue rotation of its four
//! base colours + fog) and AroundPlus (equalised ring colour).
//!
//! # Randomness
//!
//! `pickRoygbivHue` calls `Math.random()` in the JS. Renders must be
//! reproducible (golden frames, and a scene has no ambient RNG), so
//! [`pick_roygbiv_hue`] takes the `[0,1)` sample as an argument and the scenes
//! feed it [`crate::point10::shared::random::seeded_rand`] with a frame
//! counter. The distribution is the JS one; a given run is deterministic.
//!
//! # Colour space
//!
//! `THREE.ColorManagement` is enabled in the worker, so `new THREE.Color(hex)`
//! converts sRGB → linear-sRGB before the value reaches a shader, while
//! `new THREE.Color(r, g, b)` with numbers is already in the working (linear)
//! space. [`srgb_hex_to_linear`] is that conversion for the hex-based scenes
//! (WaterRipple, AroundPlus); the JSON read in the JS sources is in the
//! numbers-are-linear form already (Mycelium).

#![forbid(unsafe_code)]

/// The ROYGBIV hue rotations (`phraseColor.js: ROYGBIV_HUES`), radians.
pub const ROYGBIV_HUES: [f32; 7] = [
    0.0,
    -std::f32::consts::PI / 12.0,
    -std::f32::consts::PI / 4.0,
    -(2.0 * std::f32::consts::PI) / 3.0,
    (2.0 * std::f32::consts::PI) / 3.0,
    (3.0 * std::f32::consts::PI) / 4.0,
    (7.0 * std::f32::consts::PI) / 12.0,
];

/// The white rotation axis (`_hueAxis = new THREE.Vector3(0.57735, …)`).
pub const HUE_AXIS: [f32; 3] = [0.577_35, 0.577_35, 0.577_35];

/// Drop-detector thresholds (`phraseColor.js: DROP`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropOptions {
    /// Minimum climb as a fraction of the current level.
    pub rise_frac: f32,
    /// How close to the phrase ceiling the crest must be.
    pub peak_frac: f32,
    /// Absolute climb floor (the plane9 AGC normalises the session to ~1).
    pub floor: f32,
    /// Rest-level release time constant, seconds.
    pub release_sec: f32,
    /// Minimum spacing between hue flips, seconds.
    pub rearm_sec: f32,
}

/// `DROP` exactly as the JS declares it.
pub const DROP: DropOptions = DropOptions {
    rise_frac: 0.4,
    peak_frac: 0.75,
    floor: 0.05,
    release_sec: 2.0,
    rearm_sec: 10.0,
};

/// `hueRotate(color, angle)` — Rodrigues rotation about [`HUE_AXIS`].
///
/// Each component is clamped at 0 like `target.setRGB(Math.max(0, x), …)`
/// (three clamps a negative channel when the rotated vector leaves the RGB
/// cube). Returns linear working-space RGB.
pub fn hue_rotate(color: [f32; 3], angle: f32) -> [f32; 3] {
    let cos = angle.cos();
    let sin = angle.sin();
    let dot = HUE_AXIS[0] * color[0] + HUE_AXIS[1] * color[1] + HUE_AXIS[2] * color[2];

    let x = color[0] * cos
        + (HUE_AXIS[1] * color[2] - HUE_AXIS[2] * color[1]) * sin
        + HUE_AXIS[0] * dot * (1.0 - cos);
    let y = color[1] * cos
        + (HUE_AXIS[2] * color[0] - HUE_AXIS[0] * color[2]) * sin
        + HUE_AXIS[1] * dot * (1.0 - cos);
    let z = color[2] * cos
        + (HUE_AXIS[0] * color[1] - HUE_AXIS[1] * color[0]) * sin
        + HUE_AXIS[2] * dot * (1.0 - cos);

    // `Math.max(0, …)` — `f32::max` would swallow a NaN the way JS does not,
    // but a NaN channel can only come from a NaN input, which callers never
    // pass (the palette is static).
    [x.max(0.0), y.max(0.0), z.max(0.0)]
}

/// `equalizedHue(base, angle)` — `hueRotate` scaled so its Rec. 709 luminance
/// matches the green ROYGBIV state's.
pub fn equalized_hue(base: [f32; 3], angle: f32) -> [f32; 3] {
    let target = hue_rotate(base, angle);
    let green = hue_rotate(base, ROYGBIV_HUES[4]);

    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let lr = luma(target);
    let lg = luma(green);
    // `const k = lg / Math.max(1e-4, lr);`
    let k = lg / lr.max(1e-4);

    [
        (target[0] * k).max(0.0),
        (target[1] * k).max(0.0),
        (target[2] * k).max(0.0),
    ]
}

/// `pickRoygbivHue(avoid, rand)` — a ROYGBIV rotation at least 0.2 rad away
/// from `avoid`, so every flip is visible.
///
/// `rand` is the `[0,1)` sample the JS takes from `Math.random`; values
/// outside `[0,1)` (including NaN) read as 0, and a sample of exactly 1.0
/// selects the last choice instead of `undefined`.
pub fn pick_roygbiv_hue(avoid: f32, rand: f32) -> f32 {
    let mut pool: Vec<f32> = ROYGBIV_HUES
        .iter()
        .copied()
        .filter(|hue| (hue - avoid).abs() > 0.2)
        .collect();
    if pool.is_empty() {
        pool.extend_from_slice(&ROYGBIV_HUES);
    }

    let sample = if rand.is_finite() {
        rand.clamp(0.0, 0.999_999_94)
    } else {
        0.0
    };
    let index = (sample * pool.len() as f32).floor() as usize;
    pool[index.min(pool.len() - 1)]
}

/// The phrase state `dropColorPulse` threads through frames.
///
/// `low`, `phrase_max`, `prev` and `peak_rise` are `None` on the first call,
/// matching the JS' `state.x === undefined` / `??` checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropColorState {
    /// The displayed (target) hue rotation, radians.
    pub hue: f32,
    /// Seconds since the last flip.
    pub t: f32,
    /// The released rest level of the bass band.
    pub low: Option<f32>,
    /// The session's running bass maximum.
    pub phrase_max: Option<f32>,
    /// The previous frame's bass.
    pub prev: Option<f32>,
    /// The strongest single-frame climb of the current ascent.
    pub peak_rise: Option<f32>,
}

impl Default for DropColorState {
    fn default() -> Self {
        Self::new()
    }
}

impl DropColorState {
    /// `this._phrase = { hue: 0, t: 0 }` (WaterRipple/Mycelium/AroundPlus).
    pub fn new() -> Self {
        Self {
            hue: 0.0,
            t: 0.0,
            low: None,
            phrase_max: None,
            prev: None,
            peak_rise: None,
        }
    }
}

/// Advance the drop detector one frame and return whether the hue flipped.
///
/// Literal port of `phraseColor.js: dropColorPulse`:
///
/// * `rise` is measured against the *previous* rest level, so the release term
///   cannot eat the climb on the same step;
/// * `peak_rise` resets to 0 once the bass sinks (`rise <= 0`) so an earlier
///   climb cannot leak into a later re-charge;
/// * `phrase_max` is the session maximum and never resets;
/// * a flip needs `bass < prev` (the crest rolled over), a strong climb, a
///   near-ceiling level and the rearm floor.
///
/// `rand` is the `[0,1)` sample used only when a flip fires
/// ([`pick_roygbiv_hue`]).
pub fn drop_color_pulse(state: &mut DropColorState, bass: f32, dt: f32, rand: f32) -> bool {
    let bass = if bass.is_finite() { bass } else { 0.0 };
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let opts = DROP;

    let prev = state.prev.unwrap_or(bass);
    let prev_low = state.low.unwrap_or(bass);
    let rise = bass - prev_low;
    let peak_rise = if rise <= 0.0 {
        0.0
    } else {
        state.peak_rise.unwrap_or(0.0).max(rise)
    };
    let low = bass.min(prev_low + (bass - prev_low) * (dt / opts.release_sec).min(1.0));
    let phrase_max = state.phrase_max.unwrap_or(bass).max(bass);
    let t = state.t + dt;

    let flip = bass < prev
        && peak_rise >= (opts.rise_frac * bass.max(prev_low)).max(opts.floor)
        && bass >= opts.peak_frac * phrase_max
        && t >= opts.rearm_sec;

    if flip {
        state.hue = pick_roygbiv_hue(state.hue, rand);
        state.t = 0.0;
        state.low = Some(bass);
        state.phrase_max = Some(phrase_max);
        state.prev = Some(bass);
        state.peak_rise = Some(rise);
        return true;
    }

    state.t = t;
    state.low = Some(low);
    state.phrase_max = Some(phrase_max);
    state.prev = Some(bass);
    state.peak_rise = Some(peak_rise);
    false
}

/// The eased displayed hue the scenes chase after a flip.
///
/// Common to every phrase-colour scene:
/// `delta = atan2(sin(target - displayed), cos(target - displayed)); displayed
/// += delta * min(1, dt * rate)` — the shortest arc around the hue circle.
pub fn ease_phrase_hue(displayed: f32, target: f32, dt: f32, rate: f32) -> f32 {
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let mut delta = target - displayed;
    delta = delta.sin().atan2(delta.cos());
    displayed + delta * (dt * rate).min(1.0)
}

/// three's `SRGBToLinear` transfer (`ColorManagement.js`).
pub fn srgb_to_linear(value: f32) -> f32 {
    if value < 0.04045 {
        value * 0.077_399_380_8
    } else {
        (value * 0.947_867_298_6 + 0.052_132_701_4).powf(2.4)
    }
}

/// `new THREE.Color(0xRRGGBB)` → linear-sRGB working space.
pub fn srgb_hex_to_linear(hex: u32) -> [f32; 3] {
    let channel = |shift: u32| srgb_to_linear(((hex >> shift) & 0xff) as f32 / 255.0);
    [channel(16), channel(8), channel(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from a Node 24 run of `shared/phraseColor.js`
    /// (`hueRotate`/`equalizedHue` transcribed 1:1, see the batch-3 notes).
    const WATER_BASE_HEX: [u32; 3] = [0x28_7c_ff, 0x55_d7_ff, 0x8c_62_ff];

    fn close(actual: [f32; 3], expected: [f32; 3], tolerance: f32) {
        for channel in 0..3 {
            assert!(
                (actual[channel] - expected[channel]).abs() < tolerance,
                "channel {channel}: {:?} != {expected:?}",
                actual
            );
        }
    }

    #[test]
    fn srgb_hex_converts_like_three_color_management() {
        let base = WATER_BASE_HEX.map(srgb_hex_to_linear);
        close(base[0], [0.021_219_01, 0.201_556_25, 1.0], 1e-6);
        close(base[1], [0.090_841_71, 0.679_542_47, 1.0], 1e-6);
        close(base[2], [0.262_250_66, 0.122_138_77, 1.0], 1e-6);
    }

    #[test]
    fn hue_rotate_matches_the_js_reference() {
        let base = [
            [0.231, 0.51, 0.965],
            [0.659, 0.333, 0.969],
            [0.133, 0.827, 0.933],
            [0.957, 0.247, 0.369],
        ];

        // angle 0 is the identity
        close(hue_rotate(base[0], 0.0), base[0], 1e-6);

        close(
            hue_rotate(base[0], 0.5),
            [0.398_278_55, 0.314_013_24, 0.993_708_0],
            1e-5,
        );
        close(
            hue_rotate(base[3], 0.5),
            [0.937_803_15, 0.443_706_62, 0.191_490_05],
            1e-5,
        );
        close(
            hue_rotate(base[1], -1.2),
            [0.313_359_14, 0.704_285_18, 0.943_354_5],
            1e-5,
        );
        close(
            hue_rotate(base[2], 2.5),
            [1.066_594_4, 0.197_552_48, 0.628_849_9],
            1e-5,
        );
    }

    #[test]
    fn hue_rotate_clamps_negative_channels_at_zero() {
        // A rotation that drives red negative clamps it; the JS `Math.max(0,
        // x)` does the same for the blue base at -1.2 rad.
        let base = srgb_hex_to_linear(0x28_7c_ff);
        let rotated = equalized_hue(base, -1.2);
        assert_eq!(rotated[0], 0.0);
        assert!(rotated[2] > 0.0);
    }

    #[test]
    fn equalized_hue_matches_the_js_reference() {
        let base = WATER_BASE_HEX.map(srgb_hex_to_linear);

        let zero = equalized_hue(base[0], 0.0);
        close(zero, [0.023_281_07, 0.221_143_46, 1.097_179_9], 1e-5);

        let half = equalized_hue(base[1], 0.5);
        close(half, [0.182_935_96, 0.316_931_72, 0.845_851_9], 1e-5);

        let deep = equalized_hue(base[2], -1.2);
        close(deep, [0.0, 0.519_631_55, 0.517_161_25], 1e-5);
    }

    #[test]
    fn equalized_hue_keeps_the_green_state_luminance() {
        let base = srgb_hex_to_linear(0xff_e6_8c);
        let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let green = luma(hue_rotate(base, ROYGBIV_HUES[4]));

        for angle in ROYGBIV_HUES {
            let color = equalized_hue(base, angle);
            // A rotated colour whose luminance was clamped to zero cannot be
            // rescaled; the reference is dark in that case.
            if luma(hue_rotate(base, angle)) > 1e-4 {
                assert!(
                    (luma(color) - green).abs() < 1e-4,
                    "angle {angle}: {:?}",
                    luma(color)
                );
            }
        }
    }

    #[test]
    fn pick_roygbiv_avoids_the_current_hue() {
        for avoid in ROYGBIV_HUES {
            for sample in [0.0, 0.25, 0.5, 0.75, 0.999] {
                let hue = pick_roygbiv_hue(avoid, sample);
                assert!(
                    (hue - avoid).abs() > 0.2,
                    "picked {hue} too close to {avoid}"
                );
                assert!(ROYGBIV_HUES.contains(&hue));
            }
        }
    }

    #[test]
    fn pick_roygbiv_is_deterministic_and_total() {
        assert_eq!(pick_roygbiv_hue(0.0, 0.5), pick_roygbiv_hue(0.0, 0.5));
        // A sample of exactly 1.0 must not index past the pool.
        assert!(ROYGBIV_HUES.contains(&pick_roygbiv_hue(0.0, 1.0)));
        assert!(ROYGBIV_HUES.contains(&pick_roygbiv_hue(0.0, f32::NAN)));
        // Even when every hue is "too close" the fallback pool is used.
        assert!(ROYGBIV_HUES.contains(&pick_roygbiv_hue(f32::NAN, 0.0)));
    }

    /// The draw "crest" fixture: 10.33 s at 0.3, a sharp attack to 0.9, a
    /// shallow rollover to 0.87 (drop #1), 10 s at 0.2, a second attack and a
    /// second rollover (drop #2). Node 24 reference: flips at steps 621 and
    /// 1223, hues `-PI/12` and `2*PI/3`.
    #[test]
    fn drop_color_pulse_matches_the_js_crest_fixture() {
        let mut state = DropColorState::new();
        let mut flips: Vec<(usize, f32)> = Vec::new();
        let mut step = 0_usize;

        fn push(
            state: &mut DropColorState,
            step: &mut usize,
            flips: &mut Vec<(usize, f32)>,
            bass: f32,
        ) {
            let index = *step;
            let samples = [0.0_f32, 0.5, 0.25, 0.75];
            let fired = drop_color_pulse(
                state,
                bass,
                1.0 / 60.0,
                samples[flips.len() % samples.len()],
            );
            if fired {
                flips.push((index, state.hue));
            }
            *step += 1;
        }

        for _ in 0..620 {
            push(&mut state, &mut step, &mut flips, 0.3);
        }
        // After the rest phase the detector is armed (`t >= 10`) but the climb
        // state is still idle.
        assert!((state.t - 10.333_333).abs() < 1e-4);
        assert_eq!(state.peak_rise, Some(0.0));
        assert!(flips.is_empty());

        push(&mut state, &mut step, &mut flips, 0.9);
        // The attack records the climb but cannot flip (bass > prev).
        assert!((state.peak_rise.unwrap() - 0.6).abs() < 1e-5);
        assert!(flips.is_empty());

        push(&mut state, &mut step, &mut flips, 0.87);
        assert_eq!(flips.len(), 1, "the shallow crest must flip the hue");
        assert_eq!(flips[0].0, 621);
        assert!((flips[0].1 + std::f32::consts::PI / 12.0).abs() < 1e-6);
        // The flip resets the rearm clock and the rest level.
        assert_eq!(state.t, 0.0);
        assert_eq!(state.low, Some(0.87));
        assert!((state.peak_rise.unwrap() - 0.565).abs() < 1e-5);

        for _ in 0..600 {
            push(&mut state, &mut step, &mut flips, 0.2);
        }
        push(&mut state, &mut step, &mut flips, 0.9);
        push(&mut state, &mut step, &mut flips, 0.88);

        assert_eq!(flips.len(), 2, "the second crest must flip after the rearm");
        assert_eq!(flips[1].0, 1223);
        assert!((flips[1].1 - 2.0 * std::f32::consts::PI / 3.0).abs() < 1e-6);
        assert_eq!(state.t, 0.0);
    }

    #[test]
    fn plain_climb_never_flips() {
        // Only rising bass: no crest, so the detector must never flip even
        // after the rearm floor and a ceiling-reaching level.
        let mut state = DropColorState::new();
        for _ in 0..2000 {
            assert!(!drop_color_pulse(&mut state, 0.4, 1.0 / 60.0, 0.5));
        }
        for step in 0..600 {
            let bass = 0.4 + step as f32 * 0.001;
            assert!(!drop_color_pulse(&mut state, bass, 1.0 / 60.0, 0.5));
        }
    }

    #[test]
    fn non_finite_inputs_do_not_poison_the_state() {
        let mut state = DropColorState::new();
        drop_color_pulse(&mut state, f32::NAN, f32::NAN, 0.0);
        assert_eq!(state.t, 0.0);
        assert_eq!(state.prev, Some(0.0));
        assert_eq!(state.peak_rise, Some(0.0));
    }

    #[test]
    fn ease_phrase_hue_takes_the_short_arc() {
        let two_pi = 2.0 * std::f32::consts::PI;
        // WaterRipple.js:317-319. From +0.1 rad to (2π - 0.1): the raw delta
        // is 6.083 rad, and `atan2(sin, cos)` folds it to the -0.2 rad short
        // arc (through 0, not around the wrap). The first step therefore
        // moves *down* by 0.2 * min(1, 0.5 * 0.55).
        let next = ease_phrase_hue(0.1, two_pi - 0.1, 0.5, 0.55);
        let expected = 0.1 - 0.2 * 0.275;
        assert!(
            (next - expected).abs() < 1e-5,
            "expected the short arc {expected}, got {next}"
        );
        assert!(next < 0.1, "the short arc from 0.1 goes down, got {next}");

        // A full step converges.
        assert!((ease_phrase_hue(0.0, 1.0, 10.0, 0.55) - 1.0).abs() < 1e-6);
        // dt 0 leaves the hue alone.
        assert_eq!(ease_phrase_hue(0.0, 1.0, 0.0, 0.55), 0.0);
    }

    #[test]
    fn drop_options_match_the_js() {
        assert_eq!(DROP.rise_frac, 0.4);
        assert_eq!(DROP.peak_frac, 0.75);
        assert_eq!(DROP.floor, 0.05);
        assert_eq!(DROP.release_sec, 2.0);
        assert_eq!(DROP.rearm_sec, 10.0);
        assert_eq!(ROYGBIV_HUES.len(), 7);
    }
}
