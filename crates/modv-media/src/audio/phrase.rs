//! Dependency-free port of musicvis' phrase detector, ported from
//! `src/renderer/src/application/worker/phrase-detector.js` (96 LOC).
//!
//! A phrase boundary ("drop") is the bass band re-charging out of its recent
//! trough: the climb must be strong (peak rise past `rise_frac` of the level,
//! floored by the AGC-scaled floor), reach the crest region (`peak_frac` of the
//! session running max) and roll over (bass < prev), with `rearm_sec` spacing the
//! flips. modV's audio-bands AGC normalizes the session peak to ~1.0, which is
//! the same scale assumption musicvis' constants were tuned against
//! (phrase-detector.js:1-10).
//!
//! Time is passed explicitly as `dt` seconds — the JS already takes `dt` as an
//! argument and never reads a clock, so no behavioural change beyond Rust
//! idioms:
//!
//! * The JS `undefined` sentinels on `state.low` / `state.phraseMax` /
//!   `state.prev` (phrase-detector.js:22-24) become `Option<f64>`
//!   (`None` = "no sample yet").
//! * The JS `opts = DROP` default parameter (phrase-detector.js:37) becomes
//!   [`update_phrase`], which delegates to [`update_phrase_with_options`] with
//!   [`DROP`].
//! * The JS spread-copy of the state (`{ ...state, ... }`, phrase-detector.js:77)
//!   becomes an owned new [`PhraseState`]; the input state is borrowed and left
//!   untouched.
//! * The JS `state.peakRise || 0` guard (phrase-detector.js:47) is folded into
//!   the non-optional `peak_rise: 0.0` field; hand-constructed JS states with a
//!   missing `peakRise` would have used 0 implicitly.

// phrase-detector.js:12-18 — DROP
/// Options for the drop detector. All values are AGC normalized (~1.0 session
// max). See the module docs for the semantics of each field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DropOptions {
    /// Required rise as a fraction of `max(bass, prev_low)`; the rise must also
    /// clear [`DropOptions::floor`] (phrase-detector.js:13).
    pub rise_frac: f64,
    /// Flip may only happen when `bass` is at or above this fraction of the
    /// session running max (phrase-detector.js:14).
    pub peak_frac: f64,
    /// Absolute floor for the required rise, so a near-silent wobble cannot
    /// flip (phrase-detector.js:15).
    pub floor: f64,
    /// Time constant, in seconds, for the trailing trough estimate to release
    /// towards the current bass level (phrase-detector.js:16).
    pub release_sec: f64,
    /// Minimum seconds since the last flip before another flip is allowed
    /// (phrase-detector.js:17).
    pub rearm_sec: f64,
}

/// The DROP constants verbatim from phrase-detector.js:12-18.
pub const DROP: DropOptions = DropOptions {
    rise_frac: 0.4,
    peak_frac: 0.75,
    floor: 0.05,
    release_sec: 2.0,
    rearm_sec: 10.0,
};

impl Default for DropOptions {
    fn default() -> Self {
        DROP
    }
}

/// Phrase detector state. `None` on `low` / `phrase_max` / `prev` models the JS
/// `undefined` ("no sample yet"); `t` starts at 0 and `peak_rise` at 0
/// (phrase-detector.js:20-26).
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseState {
    /// Seconds since the last flip; resets to 0 on a flip (phrase-detector.js:21).
    pub t: f64,
    /// Trailing trough estimate; `None` until the first sample
    /// (phrase-detector.js:22).
    pub low: Option<f64>,
    /// Session running max, never reset; `None` until the first sample
    /// (phrase-detector.js:23).
    pub phrase_max: Option<f64>,
    /// Previous bass level; `None` until the first sample (phrase-detector.js:24).
    pub prev: Option<f64>,
    /// Strongest rise of the current ascent; 0 between ascents
    /// (phrase-detector.js:25).
    pub peak_rise: f64,
}

/// Fresh phrase state (phrase-detector.js:20-26 — `createPhraseState`).
pub fn create_phrase_state() -> PhraseState {
    PhraseState {
        t: 0.0,
        low: None,
        phrase_max: None,
        prev: None,
        peak_rise: 0.0,
    }
}

/// Result of [`update_phrase`] (phrase-detector.js:35 — `{ state, flipped }`).
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseUpdate {
    /// The advanced state. On a flip, `t` resets to 0 and `peak_rise` carries the
    /// current rise (phrase-detector.js:70-73); otherwise the spread copy of the
    /// input with the recomputed fields (phrase-detector.js:77).
    pub state: PhraseState,
    /// `true` when a phrase boundary was detected on this step.
    pub flipped: bool,
}

/// Advances the phrase state by `dt` seconds of bass energy using the default
/// [`DROP`] options (phrase-detector.js:37 — `opts = DROP`).
pub fn update_phrase(state: &PhraseState, bass: f64, dt: f64) -> PhraseUpdate {
    update_phrase_with_options(state, bass, dt, &DROP)
}

/// Advances the phrase state by `dt` seconds of bass energy
/// (phrase-detector.js:29-80 — `updatePhrase`).
///
/// `state` is the previous phrase state; `bass` is the current bass band level
/// (0-1, AGC normalized); `dt` is the elapsed time in seconds; `opts` carries
/// the drop detector tuning.
pub fn update_phrase_with_options(
    state: &PhraseState,
    bass: f64,
    dt: f64,
    opts: &DropOptions,
) -> PhraseUpdate {
    // phrase-detector.js:38-39
    let prev = state.prev.unwrap_or(bass);
    let prev_low = state.low.unwrap_or(bass);

    // Rise is measured against the previous rest level so the release term
    // cannot eat the climb on the same step.
    // phrase-detector.js:43
    let rise = bass - prev_low;

    // Strongest rise of the current ascent only; resets when the bass sinks
    // back to its trough.
    // phrase-detector.js:47
    let peak_rise = if rise <= 0.0 {
        0.0
    } else {
        state.peak_rise.max(rise)
    };

    // phrase-detector.js:49-52
    let low = bass.min(prev_low + (bass - prev_low) * (dt / opts.release_sec).min(1.0));

    // Session running max (never reset): near-crest is judged against the
    // track's own ceiling.
    // phrase-detector.js:56-59
    let phrase_max = state.phrase_max.unwrap_or(bass).max(bass);

    // phrase-detector.js:61
    let t = state.t + dt;

    // phrase-detector.js:63-67
    let flipped = bass < prev
        && peak_rise >= (opts.rise_frac * bass.max(prev_low)).max(opts.floor)
        && bass >= opts.peak_frac * phrase_max
        && t >= opts.rearm_sec;

    // phrase-detector.js:69-74
    if flipped {
        return PhraseUpdate {
            state: PhraseState {
                t: 0.0,
                low: Some(bass),
                phrase_max: Some(phrase_max),
                prev: Some(bass),
                peak_rise: rise,
            },
            flipped: true,
        };
    }

    // phrase-detector.js:76-79
    PhraseUpdate {
        state: PhraseState {
            t,
            low: Some(low),
            phrase_max: Some(phrase_max),
            prev: Some(bass),
            peak_rise,
        },
        flipped: false,
    }
}

/// Probability a phrase change switches scenes, ramping linearly from 0 at
/// `min_seconds` to 1 at `max_seconds`. Past `max_seconds` the next phrase
/// change always takes it (phrase-detector.js:82-96 — `switchChance`). When the
/// window is empty or inverted the chance is 1 (phrase-detector.js:88-90).
pub fn switch_chance(elapsed: f64, min_seconds: f64, max_seconds: f64) -> f64 {
    // phrase-detector.js:88-90
    if max_seconds <= min_seconds {
        return 1.0;
    }

    // phrase-detector.js:92-95
    ((elapsed - min_seconds) / (max_seconds - min_seconds)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    //! Behaviour port of `tests/unit/phrase-detector.test.js` (Vitest). The JS
    //! `toBeCloseTo(5.1)` assertion maps to a 1e-9 epsilon; JS `undefined` in
    //! state maps to `None`.
    use super::*;

    // phrase-detector.test.js:9-18 — "createPhraseState > starts with no previous samples and a zero timer"
    #[test]
    fn create_phrase_state_starts_with_no_previous_samples_and_a_zero_timer() {
        assert_eq!(
            create_phrase_state(),
            PhraseState {
                t: 0.0,
                low: None,
                phrase_max: None,
                prev: None,
                peak_rise: 0.0,
            }
        );
    }

    // phrase-detector.test.js:22-37 — "updatePhrase > flips when bass recharges out of a trough and rolls over"
    #[test]
    fn update_phrase_flips_when_bass_recharges_out_of_a_trough_and_rolls_over() {
        let state = PhraseState {
            t: DROP.rearm_sec,
            low: Some(0.05),
            phrase_max: Some(1.0),
            prev: Some(1.0),
            peak_rise: 0.0,
        };

        let result = update_phrase(&state, 0.8, 0.1);

        assert!(result.flipped);
        assert_eq!(result.state.t, 0.0);
        assert_eq!(result.state.phrase_max, Some(1.0));
        assert_eq!(result.state.prev, Some(0.8));
    }

    // phrase-detector.test.js:39-46 — "updatePhrase > does not flip before the rearm interval elapses"
    #[test]
    fn update_phrase_does_not_flip_before_the_rearm_interval_elapses() {
        let state = PhraseState {
            t: 5.0,
            low: Some(0.05),
            phrase_max: Some(1.0),
            prev: Some(1.0),
            peak_rise: 0.0,
        };

        let result = update_phrase(&state, 0.8, 0.1);

        assert!(!result.flipped);
        assert!((result.state.t - 5.1).abs() < 1e-9);
    }

    // phrase-detector.test.js:48-60 — "updatePhrase > does not flip below the crest threshold"
    #[test]
    fn update_phrase_does_not_flip_below_the_crest_threshold() {
        let state = PhraseState {
            t: DROP.rearm_sec,
            low: Some(0.05),
            phrase_max: Some(1.0),
            prev: Some(1.0),
            peak_rise: 0.0,
        };

        let result = update_phrase(&state, 0.5, 0.1);

        assert!(!result.flipped);
    }

    // phrase-detector.test.js:62-74 — "updatePhrase > requires a strong rise rather than a small wobble"
    #[test]
    fn update_phrase_requires_a_strong_rise_rather_than_a_small_wobble() {
        let state = PhraseState {
            t: DROP.rearm_sec,
            low: Some(0.5),
            phrase_max: Some(1.0),
            prev: Some(1.0),
            peak_rise: 0.0,
        };

        let result = update_phrase(&state, 0.8, 0.1);

        assert!(!result.flipped);
    }

    // phrase-detector.test.js:76-83 — "updatePhrase > tracks a running maximum that is never reset"
    #[test]
    fn update_phrase_tracks_a_running_maximum_that_is_never_reset() {
        let state = PhraseState {
            t: 0.0,
            low: Some(0.0),
            phrase_max: Some(0.9),
            prev: Some(0.9),
            peak_rise: 0.0,
        };

        let result = update_phrase(&state, 1.0, 0.1);

        assert!(!result.flipped);
        assert_eq!(result.state.phrase_max, Some(1.0));
    }

    // phrase-detector.test.js:87-95 — "switchChance > never changes before the minimum time"
    #[test]
    fn switch_chance_never_changes_before_the_minimum_time() {
        assert_eq!(switch_chance(0.0, 10.0, 60.0), 0.0);
        assert_eq!(switch_chance(10.0, 10.0, 60.0), 0.0);
    }

    // phrase-detector.test.js:92-94 — "switchChance > ramps linearly between the minimum and maximum time"
    #[test]
    fn switch_chance_ramps_linearly_between_the_minimum_and_maximum_time() {
        assert!((switch_chance(35.0, 10.0, 60.0) - 0.5).abs() < 1e-9);
    }

    // phrase-detector.test.js:96-99 — "switchChance > always changes once the maximum time has passed"
    #[test]
    fn switch_chance_always_changes_once_the_maximum_time_has_passed() {
        assert_eq!(switch_chance(60.0, 10.0, 60.0), 1.0);
        assert_eq!(switch_chance(120.0, 10.0, 60.0), 1.0);
    }

    // phrase-detector.test.js:101-104 — "switchChance > always changes when the window is empty or inverted"
    #[test]
    fn switch_chance_always_changes_when_the_window_is_empty_or_inverted() {
        assert_eq!(switch_chance(0.0, 10.0, 10.0), 1.0);
        assert_eq!(switch_chance(0.0, 60.0, 10.0), 1.0);
    }
}
