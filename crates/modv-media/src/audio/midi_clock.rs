//! MIDI clock (type 248, 24 PPQN) → BPM derivation.
//!
//! A faithful port of the clock branch of
//! `src/renderer/src/application/setup-midi.js:38-80`:
//!
//! ```js
//! if (type === 248 && listenForClock) {
//!   const currentTime = audioContext.currentTime * 1000;
//!   clockHistory.push(currentTime);
//!   if (clockHistory.length > 1) {
//!     const then = clockHistory[0];
//!     const now = clockHistory[1];
//!     const difference = now - then;
//!     clockHistory.splice(0, 1);
//!     diffHistory.push(difference);
//!     if (diffHistory.length > 1) { ... > 50 ms jump resets both ... }
//!     if (diffHistory.length > 94) diffHistory.splice(0, 1);
//!     if (currentTime > nextBpmUpdate) {
//!       nextBpmUpdate = currentTime + 500;
//!       const averageDiff = diffHistory.reduce((a, b) => a + b) / diffHistory.length;
//!       const bpm = Math.round((1000 / averageDiff / 24) * 60);
//!       store.dispatch("beats/setBpm", { bpm, source: "midi" });
//!     }
//!   }
//! }
//! ```
//!
//! So: the interval between consecutive clocks is kept in a 95-sample moving
//! average, a jump of more than +50 ms versus the previous interval clears the
//! history (tempo-change resync), and a BPM estimate — computed from the mean
//! interval, 24 clocks per quarter note — is emitted at most every 500 ms.
//!
//! The JS reads `audioContext.currentTime` in milliseconds; this port takes
//! **seconds** from any monotonic clock, which is what the engine has
//! (`std::time::Instant` elapsed seconds, or the audio graph's play position).
//!
//! Deviations:
//!
//! * The JS `return`s from the MIDI message handler on a too-large jump, which
//!   also clears the *pending* clock timestamp. [`MidiClockTracker::push`]
//!   returns `None` for that tick, so the next tick starts a fresh interval,
//!   exactly as the JS does.
//! * Intervals are stored in an explicit 95-entry ring ([`INTERVAL_HISTORY_LIMIT`])
//!   instead of a JS array that is `splice(0, 1)`-ed. The JS `clockHistory`
//!   never holds more than two timestamps, so a single `Option`-like pending
//!   timestamp is enough; the port keeps a small deque to mirror the code.
//! * A non-finite or non-positive average yields `None` instead of dispatching
//!   `Infinity` to the store. With a monotonic clock and strictly increasing
//!   ticks this cannot occur.
//! * `u32` is the delivered BPM type (the Vuex `beats/setBpm` payload is an
//!   integer BPM; `Math.round` is applied in the JS). `round` is half-away-from
//!   zero in Rust and half-up in JS, identical for positive BPM.

use std::collections::VecDeque;

/// MIDI clocks per quarter note (24 PPQN, the MIDI specification). Used by the
/// JS conversion `(1000 / averageDiff / 24) * 60` (setup-midi.js:72).
pub const MIDI_CLOCKS_PER_QUARTER_NOTE: u32 = 24;

/// Maximum number of inter-clock intervals in the moving average
/// (`diffHistory.length > 94` after pushing, setup-midi.js:64-66).
pub const INTERVAL_HISTORY_LIMIT: usize = 95;

/// Minimum time between emitted BPM values, in milliseconds
/// (`nextBpmUpdate = currentTime + 500`, setup-midi.js:70).
pub const BPM_UPDATE_INTERVAL_MS: f64 = 500.0;

/// A positive jump larger than this versus the previous interval clears the
/// history (`checkDifference > 50`, setup-midi.js:51-63). Negative jumps (a
/// faster tempo) do not reset: the JS only tests `difference - lastDifference`.
pub const MAX_INTERVAL_JUMP_MS: f64 = 50.0;

/// Converts a mean inter-clock interval in milliseconds to BPM
/// (setup-midi.js:72): `(1000 / interval_ms / 24) * 60`.
pub fn bpm_from_interval_ms(interval_ms: f64) -> f64 {
    (1000.0 / interval_ms / MIDI_CLOCKS_PER_QUARTER_NOTE as f64) * 60.0
}

/// MIDI-clock → BPM tracker. Feed every type-248 clock tick to [`Self::push`].
#[derive(Debug, Clone)]
pub struct MidiClockTracker {
    /// Pending clock timestamps in milliseconds (the JS keeps at most two).
    clock_history: VecDeque<f64>,
    /// Inter-clock intervals in milliseconds, oldest first, ≤ 95 entries.
    intervals_ms: VecDeque<f64>,
    /// `nextBpmUpdate` in milliseconds.
    next_bpm_update: f64,
    /// How many times the >50 ms outlier reset fired.
    resets: u64,
}

impl MidiClockTracker {
    /// An empty tracker (`clockHistory` / `diffHistory` module state,
    /// setup-midi.js:3-5).
    pub fn new() -> Self {
        Self {
            clock_history: VecDeque::with_capacity(2),
            intervals_ms: VecDeque::with_capacity(INTERVAL_HISTORY_LIMIT),
            next_bpm_update: 0.0,
            resets: 0,
        }
    }

    /// Records one MIDI clock tick at `now_seconds` (monotonic, seconds).
    ///
    /// Returns `Some(bpm)` when this tick crosses the 500 ms emit throttle and
    /// at least one interval exists, `None` otherwise (first tick, a reset
    /// tick, or a throttled tick).
    pub fn push(&mut self, now_seconds: f64) -> Option<u32> {
        let current_time = now_seconds * 1000.0; // setup-midi.js:40

        self.clock_history.push_back(current_time);

        // setup-midi.js:43 — a single clock sample has no interval yet.
        if self.clock_history.len() <= 1 {
            return None;
        }

        let then = self.clock_history[0];
        let now = self.clock_history[1];
        let difference = now - then;
        self.clock_history.pop_front();

        self.intervals_ms.push_back(difference);

        // setup-midi.js:50-63 — tempo-change compensation. Only a *longer*
        // interval triggers the reset; the JS takes the difference against the
        // interval before the one just pushed.
        if self.intervals_ms.len() > 1 {
            let last_difference = self.intervals_ms[self.intervals_ms.len() - 2];
            let check_difference = difference - last_difference;

            if check_difference > MAX_INTERVAL_JUMP_MS {
                self.intervals_ms.clear();
                self.clock_history.clear();
                self.resets += 1;
                return None;
            }
        }

        // setup-midi.js:64-66 — cap the moving average at 95 samples.
        while self.intervals_ms.len() > INTERVAL_HISTORY_LIMIT {
            self.intervals_ms.pop_front();
        }

        // setup-midi.js:68-78 — emit at most once per 500 ms.
        if current_time > self.next_bpm_update {
            self.next_bpm_update = current_time + BPM_UPDATE_INTERVAL_MS;

            let average_interval =
                self.intervals_ms.iter().sum::<f64>() / self.intervals_ms.len() as f64;
            let bpm = bpm_from_interval_ms(average_interval);

            if !bpm.is_finite() || bpm <= 0.0 {
                return None;
            }

            return Some(bpm.round() as u32);
        }

        None
    }

    /// Clears the pending clock, interval history and emit throttle
    /// (equivalent to a device disconnect in `handleDevices`, which rebuilds
    /// the handler state).
    pub fn reset(&mut self) {
        self.clock_history.clear();
        self.intervals_ms.clear();
        self.next_bpm_update = 0.0;
    }

    /// The current inter-clock intervals in milliseconds, oldest first.
    pub fn intervals_ms(&self) -> &VecDeque<f64> {
        &self.intervals_ms
    }

    /// The mean of the current interval history, in milliseconds.
    pub fn average_interval_ms(&self) -> Option<f64> {
        if self.intervals_ms.is_empty() {
            return None;
        }
        Some(self.intervals_ms.iter().sum::<f64>() / self.intervals_ms.len() as f64)
    }

    /// The next time (milliseconds) at which an estimate may be emitted.
    pub fn next_bpm_update_ms(&self) -> f64 {
        self.next_bpm_update
    }

    /// How many outlier resets have fired.
    pub fn resets(&self) -> u64 {
        self.resets
    }
}

impl Default for MidiClockTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inter-clock interval for a given BPM (24 PPQN), in milliseconds.
    fn interval_for_bpm(bpm: f64) -> f64 {
        1000.0 / (bpm / 60.0 * MIDI_CLOCKS_PER_QUARTER_NOTE as f64)
    }

    /// Feeds a fixed-BPM clock trace for `seconds`, returning the emitted BPM
    /// values with their timestamps in milliseconds.
    fn run_clock(bpm: f64, seconds: f64) -> (MidiClockTracker, Vec<(f64, u32)>) {
        let mut tracker = MidiClockTracker::new();
        let interval = interval_for_bpm(bpm);
        let mut emissions = Vec::new();
        let mut time_ms = 0.0;

        while time_ms <= seconds * 1000.0 {
            if let Some(bpm) = tracker.push(time_ms / 1000.0) {
                emissions.push((time_ms, bpm));
            }
            time_ms += interval;
        }

        (tracker, emissions)
    }

    #[test]
    fn constants_match_the_js_clock_branch() {
        assert_eq!(MIDI_CLOCKS_PER_QUARTER_NOTE, 24);
        assert_eq!(INTERVAL_HISTORY_LIMIT, 95);
        assert_eq!(BPM_UPDATE_INTERVAL_MS, 500.0);
        assert_eq!(MAX_INTERVAL_JUMP_MS, 50.0);
    }

    #[test]
    fn bpm_from_interval_matches_the_js_formula() {
        // 20.833... ms per clock is 120 BPM.
        assert!((bpm_from_interval_ms(1000.0 / 48.0) - 120.0).abs() < 1e-9);
        assert!((bpm_from_interval_ms(20.0) - 125.0).abs() < 1e-9);
        assert!((bpm_from_interval_ms(1000.0 / 24.0) - 60.0).abs() < 1e-9);
    }

    #[test]
    fn the_first_clock_only_starts_the_history() {
        let mut tracker = MidiClockTracker::new();

        assert_eq!(tracker.push(1.0), None);
        assert_eq!(tracker.intervals_ms().len(), 0);
        assert_eq!(tracker.average_interval_ms(), None);

        // The second clock produces the first interval and emits immediately.
        assert_eq!(tracker.push(1.0 + 1000.0 / 48.0 / 1000.0), Some(120));
        assert_eq!(tracker.intervals_ms().len(), 1);
    }

    #[test]
    fn fixed_120_bpm_clock_reports_120() {
        let (tracker, emissions) = run_clock(120.0, 2.0);

        assert_eq!(emissions.first().map(|(_, bpm)| *bpm), Some(120));
        assert!(emissions.iter().all(|(_, bpm)| *bpm == 120));
        assert!((tracker.average_interval_ms().unwrap() - 1000.0 / 48.0).abs() < 1e-9);
        assert_eq!(tracker.resets(), 0);
    }

    #[test]
    fn fixed_128_bpm_clock_reports_128() {
        let (_, emissions) = run_clock(128.0, 2.0);

        assert!(!emissions.is_empty());
        assert!(emissions.iter().all(|(_, bpm)| *bpm == 128));
    }

    #[test]
    fn fixed_90_and_174_bpm_clock_reports_those_values() {
        let (_, slow) = run_clock(90.0, 3.0);
        let (_, fast) = run_clock(174.0, 3.0);

        assert!(slow.iter().all(|(_, bpm)| *bpm == 90));
        assert!(fast.iter().all(|(_, bpm)| *bpm == 174));
    }

    #[test]
    fn emission_is_throttled_to_once_per_500_ms() {
        let (_, emissions) = run_clock(120.0, 3.0);

        assert!(emissions.len() >= 3, "emissions {emissions:?}");
        assert!(emissions.len() <= 7, "emissions {emissions:?}");
        for pair in emissions.windows(2) {
            let gap = pair[1].0 - pair[0].0;
            assert!(
                gap >= BPM_UPDATE_INTERVAL_MS - 1e-6,
                "emitted after only {gap} ms"
            );
        }
        // The emit throttle is wall time, not tick count: one clock tick
        // (1/24 of a beat) is far below 500 ms at these BPMs.
        assert!(interval_for_bpm(120.0) / 24.0 < BPM_UPDATE_INTERVAL_MS);
    }

    #[test]
    fn a_100_ms_gap_resets_the_moving_average() {
        let mut tracker = MidiClockTracker::new();
        let interval = interval_for_bpm(120.0) / 1000.0; // seconds
        let mut time = 1.0;

        // Seed ten regular 120 BPM intervals.
        for _ in 0..10 {
            tracker.push(time);
            time += interval;
        }
        assert_eq!(tracker.intervals_ms().len(), 9);
        assert!(tracker.average_interval_ms().unwrap() < 25.0);

        // A 100 ms gap: 100 - 20.83 = 79.2 ms jump > 50 ms.
        time += 0.100 - interval;
        assert_eq!(tracker.push(time), None, "a reset tick must not emit");
        assert_eq!(tracker.resets(), 1);
        assert_eq!(tracker.intervals_ms().len(), 0);
        assert_eq!(tracker.average_interval_ms(), None);

        // After a reset the next clock has no interval partner. Wait past the
        // 500 ms emit throttle so the next estimate is not suppressed.
        time += 0.600;
        assert_eq!(tracker.push(time), None);
        assert_eq!(tracker.intervals_ms().len(), 0);

        // The following clock starts a new history from a single interval.
        time += interval;
        assert_eq!(tracker.push(time), Some(120));
        assert_eq!(tracker.intervals_ms().len(), 1);
    }

    #[test]
    fn a_faster_tempo_does_not_reset_the_average() {
        let mut tracker = MidiClockTracker::new();
        let mut time = 1.0;

        // Long intervals, then a much shorter one: the jump is negative, and the
        // JS only resets on `checkDifference > 50`.
        for _ in 0..5 {
            tracker.push(time);
            time += 0.100;
        }
        let before = tracker.intervals_ms().len();
        tracker.push(time);
        time += 0.020;
        tracker.push(time);

        assert_eq!(tracker.resets(), 0);
        assert_eq!(tracker.intervals_ms().len(), before + 2);
        assert!(tracker.intervals_ms().back().unwrap() < &21.0);
    }

    #[test]
    fn interval_history_is_capped_at_95_samples() {
        let mut tracker = MidiClockTracker::new();
        let interval = interval_for_bpm(120.0) / 1000.0;
        let mut time = 1.0;

        for i in 0..400 {
            // Add a small deterministic wobble (max 0.03 ms) so the mean is not
            // trivially the same interval, then verify it equals the last 95
            // samples.
            let jitter_ms = ((i % 7) as f64 - 3.0) * 0.01;
            tracker.push(time + jitter_ms / 1000.0);
            if i > 0 {
                assert!(tracker.intervals_ms().len() <= INTERVAL_HISTORY_LIMIT);
            }
            time += interval;
        }

        assert_eq!(tracker.intervals_ms().len(), INTERVAL_HISTORY_LIMIT);
        let manual = tracker.intervals_ms().iter().sum::<f64>() / INTERVAL_HISTORY_LIMIT as f64;
        assert!((tracker.average_interval_ms().unwrap() - manual).abs() < 1e-12);
        assert_eq!(tracker.resets(), 0);
    }

    #[test]
    fn reset_clears_the_state_and_the_throttle() {
        let (mut tracker, _) = run_clock(120.0, 2.0);
        assert!(!tracker.intervals_ms().is_empty());
        assert!(tracker.next_bpm_update_ms() > 0.0);

        tracker.reset();

        assert!(tracker.intervals_ms().is_empty());
        assert_eq!(tracker.average_interval_ms(), None);
        assert_eq!(tracker.next_bpm_update_ms(), 0.0);
        // The next pair of clock ticks emits again immediately.
        assert_eq!(tracker.push(10.0), None);
        assert_eq!(tracker.push(10.0 + 1000.0 / 48.0 / 1000.0), Some(120));
    }

    #[test]
    fn a_jittery_clock_averages_to_the_midpoint_tempo() {
        // Alternating 19 ms / 21 ms intervals average 20 ms, i.e. 125 BPM. Until
        // the history holds an even number of samples the mean sits slightly
        // below 20 ms (the history always starts with a 19 ms interval), so
        // every estimate stays a few BPM above the midpoint.
        let mut tracker = MidiClockTracker::new();
        let intervals = [19.0, 21.0];
        let mut time = 1.0;
        let mut emissions = Vec::new();

        tracker.push(time);
        for i in 0..40 {
            time += intervals[i % 2] / 1000.0;
            if let Some(bpm) = tracker.push(time) {
                emissions.push(bpm);
            }
        }

        assert!((tracker.average_interval_ms().unwrap() - 20.0).abs() < 1e-9);
        assert!(!emissions.is_empty());
        assert!(
            emissions.iter().all(|bpm| (125..=132).contains(bpm)),
            "emissions {emissions:?}"
        );
        assert_eq!(tracker.resets(), 0);
    }
}
