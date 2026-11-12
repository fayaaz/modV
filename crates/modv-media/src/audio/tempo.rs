//! Dependency-free tempo tracker, ported from
//! `src/renderer/src/application/tempo-detector.js` (475 LOC).
//!
//! Differences from the JS source, all intentional:
//!
//! * **Time is injected everywhere.** The JS class never reads a clock itself —
//!   callers pass a monotonic timestamp in seconds (`process(timeSeconds, ...)`,
//!   e.g. `setup-beat-detektor.js:112` feeds it `performance.now() / 1000`). This
//!   port keeps that explicit-time signature 1:1, so tests drive the detector
//!   deterministically without wall-clock sleeps.
//! * The JS `options` object spread (`{ ...DEFAULT_OPTIONS, ...options }`,
//!   tempo-detector.js:45) becomes the typed [`TempoDetectorOptions`] struct with
//!   [`Default`]. Partial options are expressed with struct-update syntax
//!   (`TempoDetectorOptions { min_bpm: 70.0, ..Default::default() }`).
//! * The JS no-arg default on `addOnset(timeSeconds, onsetStrength = 1)`
//!   (tempo-detector.js:118) cannot be expressed in Rust; `add_onset` takes the
//!   strength explicitly. Callers that relied on the default must pass `1.0`.
//! * `bpm`/`confidence` stay public `f64` fields with the JS sentinel
//!   semantics (`bpm == 0.0` means "no estimate yet") because call sites read
//!   them directly (`setup-beat-detektor.js:120-128`). There is no `Option`
//!   wrapper: behaviour is identical, including the silence case
//!   (tempo-detector.js:84, :126).
//! * JS `Math.round` rounds half towards +∞; Rust `f64::round` rounds half away
//!   from zero. They agree for the non-negative values involved here
//!   (envelope frame counts, `smoothed_bpm * 10.0` where `smoothed_bpm >= 1`).
//!
//! Onset strengths are accumulated into a uniformly sampled envelope, then
//! scored with a harmonic autocorrelation/comb filter across the supported BPM
//! range. A midpoint-energy check corrects octave errors (for example, 174 BPM
//! material whose every other beat is slightly stronger). Estimates are
//! median-smoothed and confidence is smoothed over time so the reported tempo
//! does not jump between analyses.

pub const MIN_BPM: f64 = 70.0; // tempo-detector.js:12
pub const MAX_BPM: f64 = 200.0; // tempo-detector.js:13

/// Lowest analysis-envelope rate [`TempoDetector::new`] accepts.
///
/// Defence-in-depth deviation from the JS (which stores `analysisRate`
/// verbatim): `frame_duration = 1 / rate`, so a zero/negative rate produced
/// `inf`/`NaN` frame times and an unusable envelope.
pub const MIN_ANALYSIS_RATE: f64 = 1.0;
/// Highest analysis-envelope rate [`TempoDetector::new`] accepts.
///
/// Caps `history_frames` (`history_seconds * analysis_rate`), which is
/// allocated as a `Vec` on every analysis.
pub const MAX_ANALYSIS_RATE: f64 = 1_000.0;
/// Most envelope history (seconds) a detector keeps.
pub const MAX_HISTORY_SECONDS: f64 = 60.0;
/// Largest `history_frames` a detector can reach
/// (`MAX_HISTORY_SECONDS * MAX_ANALYSIS_RATE`), i.e. the biggest envelope
/// [`TempoDetector`] will allocate.
pub const MAX_HISTORY_FRAMES: usize = 60_000;

/// Options for [`TempoDetector`]. Defaults match `DEFAULT_OPTIONS`
/// (tempo-detector.js:15-26).
#[derive(Debug, Clone, PartialEq)]
pub struct TempoDetectorOptions {
    /// Lower supported BPM (tempo-detector.js:16, default [`MIN_BPM`]). Clamped
    /// to `[1, 400]` by [`TempoDetector::new`].
    pub min_bpm: f64,
    /// Upper supported BPM (tempo-detector.js:17, default [`MAX_BPM`]). Clamped
    /// to `[min_bpm + 1, 400]` by [`TempoDetector::new`].
    pub max_bpm: f64,
    /// Seconds of history kept in the analysis envelope (tempo-detector.js:18).
    pub history_seconds: f64,
    /// Analysis envelope sample rate in Hz; frames per second
    /// (tempo-detector.js:19).
    pub analysis_rate: f64,
    /// Minimum seconds between analyses (tempo-detector.js:20).
    pub analysis_interval: f64,
    /// Minimum seconds of history required before an estimate is produced
    /// (tempo-detector.js:21).
    pub min_history_seconds: f64,
    /// Median-smoothing window in number of estimates. Fractional values behave
    /// as in JS (`estimates.len() > window` with a float comparison,
    /// tempo-detector.js:458); clamped to at least 1 (tempo-detector.js:53).
    pub smoothing_window: f64,
    /// Exponential smoothing factor for confidence in `[0, 1]`
    /// (tempo-detector.js:23). Clamped by [`TempoDetector::new`].
    pub confidence_smoothing: f64,
    /// Ratio of midpoint energy to beat energy above which the estimate is
    /// doubled (octave correction, tempo-detector.js:24).
    pub octave_midpoint_ratio: f64,
    /// Confidence at or above which callers may trust the BPM estimate
    /// (tempo-detector.js:25). Read by call sites, e.g. setup-beat-detektor.js:122.
    pub confidence_threshold: f64,
}

impl Default for TempoDetectorOptions {
    // tempo-detector.js:15-26 — DEFAULT_OPTIONS verbatim.
    fn default() -> Self {
        Self {
            min_bpm: MIN_BPM,
            max_bpm: MAX_BPM,
            history_seconds: 10.0,
            analysis_rate: 100.0,
            analysis_interval: 0.2,
            min_history_seconds: 3.0,
            smoothing_window: 5.0,
            confidence_smoothing: 0.4,
            octave_midpoint_ratio: 0.35,
            confidence_threshold: 0.25,
        }
    }
}

// tempo-detector.js:28-30 — clamp
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.min(max).max(min)
}

/// `clamp`, but a `NaN` value falls back to `fallback` instead of being pushed
/// to one end of the range (`f64::min`/`max` ignore `NaN`, so `clamp(NaN, …)`
/// silently returns `max`).
fn clamp_or(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_nan() {
        fallback
    } else {
        clamp(value, min, max)
    }
}

// tempo-detector.js:32-41 — median (NaN handling differs: the JS comparator
// `(a, b) => a - b` would order NaN inconsistently; estimates are always
// finite here, and `partial_cmp` falls back to Equal as a safety net.)
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let middle = sorted.len() / 2;

    if sorted.len() % 2 == 1 {
        return sorted[middle];
    }

    (sorted[middle - 1] + sorted[middle]) / 2.0
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    time: f64,
    value: f64,
}

/// Onset-onset interval BPM tracker. See the module docs for the differences
/// from the JS implementation; the analysis math is ported 1:1.
pub struct TempoDetector {
    /// Lower bound of the supported BPM range (tempo-detector.js:47).
    pub min_bpm: f64,
    /// Upper bound of the supported BPM range (tempo-detector.js:48).
    pub max_bpm: f64,
    /// Seconds of history kept in the analysis envelope.
    pub history_seconds: f64,
    /// Envelope sample rate in Hz.
    pub analysis_rate: f64,
    /// Minimum seconds between analyses.
    pub analysis_interval: f64,
    /// Minimum seconds of history required before an estimate is produced.
    pub min_history_seconds: f64,
    /// Median-smoothing window in number of estimates.
    pub smoothing_window: f64,
    /// Exponential smoothing factor for confidence in `[0, 1]`.
    pub confidence_smoothing: f64,
    /// Ratio of midpoint energy to beat energy above which the estimate is
    /// doubled (octave correction).
    pub octave_midpoint_ratio: f64,
    /// Confidence at or above which callers may trust the BPM estimate.
    pub confidence_threshold: f64,

    frame_duration: f64,
    history_frames: usize,
    samples: Vec<Sample>,
    estimates: Vec<f64>,
    last_analysis_time: f64,

    /// Current median-smoothed estimate in BPM; `0.0` means no estimate yet
    /// (tempo-detector.js:68).
    pub bpm: f64,
    /// Exponentially smoothed confidence in `[0, 1]`; `0.0` when silent
    /// (tempo-detector.js:69).
    pub confidence: f64,
}

impl TempoDetector {
    /// Creates a detector from `config`, clamping the values exactly as the JS
    /// constructor does (tempo-detector.js:44-70).
    ///
    /// Two values the JS stores verbatim are clamped here (R-19 deviation):
    /// `analysis_rate` to `[MIN_ANALYSIS_RATE, MAX_ANALYSIS_RATE]` and
    /// `history_seconds` to `[0, MAX_HISTORY_SECONDS]`. `frame_duration` is
    /// `1 / analysis_rate` and `history_frames` is
    /// `history_seconds * analysis_rate`, so a degenerate rate produced
    /// infinite frame times and an absurd history an unbounded envelope
    /// allocation. Non-finite (`NaN`) values fall back to the default.
    pub fn new(config: TempoDetectorOptions) -> Self {
        let min_bpm = clamp(config.min_bpm, 1.0, 400.0); // tempo-detector.js:47
        let max_bpm = clamp(config.max_bpm, min_bpm + 1.0, 400.0); // tempo-detector.js:48

        let defaults = TempoDetectorOptions::default();
        let analysis_rate = clamp_or(
            config.analysis_rate,
            MIN_ANALYSIS_RATE,
            MAX_ANALYSIS_RATE,
            defaults.analysis_rate,
        );
        let history_seconds = clamp_or(
            config.history_seconds,
            0.0,
            MAX_HISTORY_SECONDS,
            defaults.history_seconds,
        );

        let frame_duration = 1.0 / analysis_rate; // tempo-detector.js:58
        // tempo-detector.js:59-62 — Math.max(2, Math.round(historySeconds * analysisRate))
        let history_frames = ((history_seconds * analysis_rate).round() as i64).max(2) as usize;
        debug_assert!(history_frames <= MAX_HISTORY_FRAMES);

        Self {
            min_bpm,
            max_bpm,
            history_seconds,
            analysis_rate,
            analysis_interval: config.analysis_interval,
            min_history_seconds: config.min_history_seconds,
            smoothing_window: config.smoothing_window.max(1.0), // tempo-detector.js:53
            confidence_smoothing: clamp(config.confidence_smoothing, 0.0, 1.0), // tempo-detector.js:54
            octave_midpoint_ratio: config.octave_midpoint_ratio,
            confidence_threshold: config.confidence_threshold,
            frame_duration,
            history_frames,
            samples: Vec::new(),
            estimates: Vec::new(),
            last_analysis_time: f64::NEG_INFINITY, // tempo-detector.js:66 (-Infinity)
            bpm: 0.0,                              // tempo-detector.js:68
            confidence: 0.0,                       // tempo-detector.js:69
        }
    }

    /// Feeds one sample of the onset-strength envelope (tempo-detector.js:73-113).
    ///
    /// `time_seconds` is a monotonic timestamp in seconds; `onset_strength` is a
    /// non-negative onset strength. Non-finite time is ignored; non-finite or
    /// negative strength is coerced to `0` (tempo-detector.js:79-85). A sample
    /// out of order resets the detector (tempo-detector.js:87-91). Analysis is
    /// throttled to at most one run per `analysis_interval` seconds of
    /// timestamp (tempo-detector.js:109-112).
    pub fn process(&mut self, time_seconds: f64, onset_strength: f64) {
        // tempo-detector.js:79-81
        if !time_seconds.is_finite() {
            return;
        }

        // tempo-detector.js:83-85
        let strength = if onset_strength.is_finite() {
            onset_strength.max(0.0)
        } else {
            0.0
        };

        // tempo-detector.js:87-91
        if let Some(last_sample) = self.samples.last()
            && time_seconds < last_sample.time
        {
            self.reset();
        }

        // tempo-detector.js:93
        self.samples.push(Sample {
            time: time_seconds,
            value: strength,
        });

        // tempo-detector.js:95-107
        let cutoff = time_seconds - self.history_seconds - 1.0;
        let remove_count = self
            .samples
            .iter()
            .take_while(|sample| sample.time < cutoff)
            .count();

        if remove_count > 0 {
            self.samples.drain(..remove_count);
        }

        // tempo-detector.js:109-112
        if time_seconds - self.last_analysis_time >= self.analysis_interval {
            self.last_analysis_time = time_seconds;
            self.analyze();
        }
    }

    /// Alias kept for callers that think in discrete onsets
    /// (tempo-detector.js:115-120). The JS defaults `onsetStrength` to `1`;
    /// Rust has no default arguments, so pass `1.0` explicitly.
    pub fn add_onset(&mut self, time_seconds: f64, onset_strength: f64) {
        self.process(time_seconds, onset_strength); // tempo-detector.js:119
    }

    /// Clears history, estimates, the analysis throttle and the reported
    /// BPM/confidence (tempo-detector.js:122-128).
    pub fn reset(&mut self) {
        self.samples.clear(); // tempo-detector.js:123
        self.estimates.clear(); // tempo-detector.js:124
        self.last_analysis_time = f64::NEG_INFINITY; // tempo-detector.js:125
        self.bpm = 0.0; // tempo-detector.js:126
        self.confidence = 0.0; // tempo-detector.js:127
    }

    // tempo-detector.js:130-219 — _buildEnvelope
    fn build_envelope(&self) -> Vec<f64> {
        let frames = self.history_frames;
        let mut envelope = vec![0.0_f64; frames]; // tempo-detector.js:132

        // tempo-detector.js:134-136
        if self.samples.len() < 2 {
            return envelope;
        }

        let newest = self.samples[self.samples.len() - 1].time; // tempo-detector.js:138
        let start = newest - self.history_seconds; // tempo-detector.js:139
        let last_index = self.samples.len() - 1; // tempo-detector.js:140
        let mut sample_index = 0_usize; // tempo-detector.js:141

        for (i, envelope_slot) in envelope.iter_mut().enumerate().take(frames) {
            let time = start + i as f64 * self.frame_duration; // tempo-detector.js:144

            // tempo-detector.js:146-151
            while sample_index < last_index && self.samples[sample_index + 1].time <= time {
                sample_index += 1;
            }

            // tempo-detector.js:153-156
            if sample_index >= last_index {
                *envelope_slot = self.samples[last_index].value;
                continue;
            }

            let previous = self.samples[sample_index];
            let next = self.samples[sample_index + 1];
            let span = next.time - previous.time; // tempo-detector.js:160

            // tempo-detector.js:162-173
            if span > 0.08 {
                // Sparse event-style input: keep isolated onsets as short pulses
                // instead of interpolating a constant value between them.
                let to_previous = (time - previous.time).max(0.0);
                let to_next = (next.time - time).max(0.0);
                let distance = to_previous.min(to_next);
                let nearest_value = if to_previous <= to_next {
                    previous.value
                } else {
                    next.value
                };

                *envelope_slot = if distance < 0.05 {
                    nearest_value * (1.0 - distance / 0.05)
                } else {
                    0.0
                };
                continue;
            }

            // tempo-detector.js:175-179
            let fraction = if span > 0.0 {
                (time - previous.time) / span
            } else {
                0.0
            };
            let clamped_fraction = clamp(fraction, 0.0, 1.0);

            *envelope_slot = previous.value + (next.value - previous.value) * clamped_fraction;
        }

        // tempo-detector.js:182-192
        let mut peak = 0.0_f64;

        for &value in envelope.iter() {
            if value > peak {
                peak = value;
            }
        }

        if peak <= 0.0 {
            return envelope;
        }

        // tempo-detector.js:194-202
        let mut mean = 0.0_f64;

        for value in envelope.iter_mut() {
            *value /= peak;
            mean += *value;
        }

        mean /= frames as f64;
        peak = 0.0;

        // tempo-detector.js:204-210
        for value in envelope.iter_mut() {
            *value = (*value - mean).max(0.0);

            if *value > peak {
                peak = *value;
            }
        }

        // tempo-detector.js:212-216
        if peak > 0.0 {
            for value in envelope.iter_mut() {
                *value /= peak;
            }
        }

        envelope
    }

    // tempo-detector.js:221-260 — _autocorrelation
    fn autocorrelation(envelope: &[f64], lag: f64) -> f64 {
        let frames = envelope.len();

        // tempo-detector.js:224-226
        if lag <= 0.0 || lag >= (frames - 1) as f64 {
            return 0.0;
        }

        let base_lag = lag.floor() as usize; // tempo-detector.js:228
        let fraction = lag - base_lag as f64; // tempo-detector.js:229
        let last = frames as isize - base_lag as isize - 2; // tempo-detector.js:230

        // tempo-detector.js:232-238
        let mut mean = 0.0_f64;

        for &value in envelope.iter() {
            mean += value;
        }

        mean /= frames as f64;

        // tempo-detector.js:240-253
        let mut correlation = 0.0_f64;
        let mut energy_a = 0.0_f64;
        let mut energy_b = 0.0_f64;

        let mut i = 0_isize;

        while i < last {
            let index = i as usize;
            let a = envelope[index] - mean;
            let b = (envelope[index + base_lag] - mean) * (1.0 - fraction)
                + (envelope[index + base_lag + 1] - mean) * fraction;

            correlation += a * b;
            energy_a += a * a;
            energy_b += b * b;
            i += 1;
        }

        // tempo-detector.js:255-257
        if energy_a <= 0.0 || energy_b <= 0.0 {
            return 0.0;
        }

        // tempo-detector.js:259
        correlation / (energy_a * energy_b).sqrt()
    }

    // tempo-detector.js:262-280 — _combScore
    fn comb_score(&self, envelope: &[f64], bpm: f64) -> f64 {
        let lag = (60.0 / bpm) * self.analysis_rate; // tempo-detector.js:263
        let mut score = 0.0_f64;
        let mut weights = 0.0_f64;

        // tempo-detector.js:267-277
        for harmonic in 1..=4_i64 {
            let harmonic_lag = harmonic as f64 * lag;

            if harmonic_lag >= (envelope.len() - 2) as f64 {
                break;
            }

            let weight = 1.0 / (harmonic as f64).sqrt();
            score += weight * Self::autocorrelation(envelope, harmonic_lag);
            weights += weight;
        }

        // tempo-detector.js:279
        if weights > 0.0 { score / weights } else { 0.0 }
    }

    // tempo-detector.js:282-311 — _phaseStrength
    fn phase_strength(envelope: &[f64], period_frames: f64, offset_frames: f64) -> f64 {
        let frames = envelope.len();
        let beat_count_floor = ((frames as f64 - offset_frames) / period_frames).floor();

        // tempo-detector.js:286-288
        if beat_count_floor <= 0.0 {
            return 0.0;
        }

        let beat_count = beat_count_floor as i64;
        // Small timing jitter should not hide a beat. The radius scales with the
        // period, but stays narrow enough to see genuine half-time gaps.
        // tempo-detector.js:292
        let radius = ((period_frames * 0.06).round().max(1.0)) as isize; // tempo-detector.js:292
        let mut total = 0.0_f64;

        // tempo-detector.js:295-308
        for beat in 0..beat_count {
            let center = (offset_frames + beat as f64 * period_frames).round() as isize;
            let mut strongest = 0.0_f64;

            for offset in -radius..=radius {
                let index = center + offset;

                if index >= 0 && (index as usize) < frames {
                    let value = envelope[index as usize];

                    if value > strongest {
                        strongest = value;
                    }
                }
            }

            total += strongest;
        }

        total / beat_count as f64 // tempo-detector.js:310
    }

    // tempo-detector.js:313-329 — _bestPhase
    fn best_phase(envelope: &[f64], period_frames: f64) -> (f64, f64) {
        let mut best_offset = 0.0_f64;
        let mut best_strength = -1.0_f64;
        const STEPS: usize = 16; // tempo-detector.js:316-317

        for step in 0..STEPS {
            let offset = (step as f64 / STEPS as f64) * period_frames; // tempo-detector.js:319
            let strength = Self::phase_strength(envelope, period_frames, offset);

            if strength > best_strength {
                best_strength = strength;
                best_offset = offset;
            }
        }

        (best_offset, best_strength)
    }

    // tempo-detector.js:331-359 — _scoreCandidates
    fn score_candidates(&self, envelope: &[f64]) -> (f64, f64) {
        let mut best_bpm = self.min_bpm; // tempo-detector.js:332
        let mut best_score = f64::NEG_INFINITY; // tempo-detector.js:333
        const COARSE_STEP: f64 = 1.0; // tempo-detector.js:335

        // tempo-detector.js:337-344
        let mut bpm = self.min_bpm;

        while bpm <= self.max_bpm {
            let score = self.score_bpm(envelope, bpm);

            if score > best_score {
                best_score = score;
                best_bpm = bpm;
            }

            bpm += COARSE_STEP;
        }

        let refine_start = self.min_bpm.max(best_bpm - COARSE_STEP); // tempo-detector.js:346
        let refine_end = self.max_bpm.min(best_bpm + COARSE_STEP); // tempo-detector.js:347

        // tempo-detector.js:349-356
        let mut bpm = refine_start;

        while bpm <= refine_end {
            let score = self.score_bpm(envelope, bpm);

            if score > best_score {
                best_score = score;
                best_bpm = bpm;
            }

            bpm += 0.05;
        }

        (best_bpm, best_score) // tempo-detector.js:358
    }

    // tempo-detector.js:361-382 — _refineWithPhase (JS default `range = 1.5`)
    fn refine_with_phase(&self, envelope: &[f64], bpm: f64, range: f64) -> f64 {
        // Autocorrelation is robust but its peak can be a fraction of a BPM away
        // from the real tempo. Maximising beat alignment sharpens the estimate
        // without making the global search expensive.
        let start = self.min_bpm.max(bpm - range); // tempo-detector.js:365
        let end = self.max_bpm.min(bpm + range); // tempo-detector.js:366
        let mut best_bpm = bpm;
        let mut best_score = f64::NEG_INFINITY;

        // tempo-detector.js:370-379
        let mut candidate = start;

        while candidate <= end {
            let period_frames = (60.0 / candidate) * self.analysis_rate;
            let (_offset, strength) = Self::best_phase(envelope, period_frames);
            let score = strength + self.comb_score(envelope, candidate) * 0.01;

            if score > best_score {
                best_score = score;
                best_bpm = candidate;
            }

            candidate += 0.05;
        }

        best_bpm
    }

    // tempo-detector.js:384-391 — _scoreBpm
    fn score_bpm(&self, envelope: &[f64], bpm: f64) -> f64 {
        let score = self.comb_score(envelope, bpm);

        // Mild prior: avoid implausible extremes when the evidence is ambiguous.
        // tempo-detector.js:388
        let prior = (-0.5 * (bpm / 120.0).log2().powf(2.0)).exp();

        // tempo-detector.js:390
        score * (0.8 + 0.2 * prior)
    }

    // tempo-detector.js:393-416 — _correctOctave
    fn correct_octave(&self, envelope: &[f64], bpm: f64) -> f64 {
        // If the midpoint between the detected beats is also strong, the true beat
        // is twice as fast. Genuine half-time patterns leave those midpoints empty.
        let period_frames = (60.0 / bpm) * self.analysis_rate; // tempo-detector.js:396
        let (offset, _strength) = Self::best_phase(envelope, period_frames); // tempo-detector.js:397
        let beat_strength = Self::phase_strength(envelope, period_frames, offset); // tempo-detector.js:398
        let midpoint_strength =
            Self::phase_strength(envelope, period_frames, offset + period_frames / 2.0); // tempo-detector.js:399-403

        // tempo-detector.js:405-407
        if beat_strength <= 1e-6 {
            return bpm;
        }

        let midpoint_ratio = midpoint_strength / beat_strength; // tempo-detector.js:409

        // tempo-detector.js:411-413
        if midpoint_ratio > self.octave_midpoint_ratio && bpm * 2.0 <= self.max_bpm {
            return bpm * 2.0;
        }

        bpm // tempo-detector.js:415
    }

    // tempo-detector.js:418-468 — _analyze
    fn analyze(&mut self) {
        // tempo-detector.js:419-421
        if self.samples.len() < 4 {
            return;
        }

        let newest = self.samples[self.samples.len() - 1].time; // tempo-detector.js:423
        let oldest = self.samples[0].time; // tempo-detector.js:424

        // tempo-detector.js:426-428
        if newest - oldest < self.min_history_seconds {
            return;
        }

        let envelope = self.build_envelope(); // tempo-detector.js:430
        let mut activity = 0.0_f64;

        // tempo-detector.js:433-435
        for &value in envelope.iter() {
            activity += value;
        }

        // tempo-detector.js:437-445
        if activity <= 1e-6 {
            self.confidence += self.confidence_smoothing * (0.0 - self.confidence);

            if self.confidence < 0.01 {
                self.confidence = 0.0;
            }

            return;
        }

        let candidate = self.score_candidates(&envelope); // tempo-detector.js:447
        let refined_bpm = self.refine_with_phase(&envelope, candidate.0, 1.5); // tempo-detector.js:448
        let corrected_bpm = self.correct_octave(&envelope, refined_bpm); // tempo-detector.js:449
        let final_bpm = self.refine_with_phase(&envelope, corrected_bpm, 1.5); // tempo-detector.js:450
        let bpm = clamp(final_bpm, self.min_bpm, self.max_bpm); // tempo-detector.js:451
        let selected_score = self.comb_score(&envelope, bpm); // tempo-detector.js:452
        let activity_factor = (activity / (self.history_frames as f64 * 0.02)).min(1.0); // tempo-detector.js:453
        let raw_confidence = clamp(selected_score * activity_factor, 0.0, 1.0); // tempo-detector.js:454

        // tempo-detector.js:456-460
        self.estimates.push(bpm);

        if self.estimates.len() as f64 > self.smoothing_window {
            self.estimates.remove(0);
        }

        let smoothed_bpm = median(&self.estimates); // tempo-detector.js:462

        self.bpm = (smoothed_bpm * 10.0).round() / 10.0; // tempo-detector.js:464
        self.confidence += self.confidence_smoothing * (raw_confidence - self.confidence); // tempo-detector.js:465-466
        self.confidence = clamp(self.confidence, 0.0, 1.0); // tempo-detector.js:467
    }
}

impl Default for TempoDetector {
    /// Equivalent to `new TempoDetector()` in JS (tempo-detector.js:44).
    fn default() -> Self {
        Self::new(TempoDetectorOptions::default())
    }
}

/// Convenience constructor mirroring the JS `createTempoDetector` export
/// (tempo-detector.js:471-473). The JS accepts `undefined` to mean "defaults";
/// pass [`TempoDetectorOptions::default`] for the same behaviour.
pub fn create_tempo_detector(options: TempoDetectorOptions) -> TempoDetector {
    TempoDetector::new(options)
}

#[cfg(test)]
mod tests {
    //! Behaviour port of `tests/unit/tempo-detector.test.js` (Vitest). The JS
    //! helper names are kept where practical; `feed`/`make_onset_signal` are
    //! the fixtures from tempo-detector.test.js:11-41 and the LCG matches
    //! `createRandom` (tempo-detector.test.js:11-18) bit-for-bit.
    use super::*;

    const FPS: usize = 60; // tempo-detector.test.js:9

    // tempo-detector.test.js:11-18 — createRandom
    fn create_random(seed: u32) -> impl FnMut() -> f64 {
        let mut state = seed; // JS `seed >>> 0`

        move || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            state as f64 / 0x1_0000_0000_u64 as f64 // JS `state / 0x100000000`
        }
    }

    // tempo-detector.test.js:20-35 — makeOnsetSignal
    fn make_onset_signal(bpm: f64, seconds: f64, seed: u32) -> Vec<f64> {
        let mut random = create_random(seed);
        let period = 60.0 / bpm;
        let frames = (seconds * FPS as f64).round() as usize; // Math.round(seconds * FPS)
        let mut signal = vec![0.0_f64; frames];

        let mut beat = 0.0_f64;

        while beat < seconds {
            signal[(beat * FPS as f64).round() as usize] = 1.0;
            beat += period;
        }

        for i in 1..frames {
            signal[i] = signal[i].max(signal[i - 1] * 0.5 + random() * 0.001);
        }

        signal
    }

    // tempo-detector.test.js:37-41 — feed
    fn feed(detector: &mut TempoDetector, signal: &[f64]) {
        for (index, &value) in signal.iter().enumerate() {
            detector.process(index as f64 / FPS as f64, value);
        }
    }

    // R-19 regression: the public constructor used to store `analysis_rate` and
    // `history_seconds` verbatim, so `analysis_rate: 0.0` made
    // `frame_duration` infinite (`1/0`) and an infinite/absurd
    // `history_seconds` allocated an unbounded envelope (`vec![0.0; i64::MAX]`
    // → capacity overflow). Both are clamped now.
    #[test]
    fn degenerate_analysis_rate_and_history_are_clamped() {
        let detector = TempoDetector::new(TempoDetectorOptions {
            analysis_rate: 0.0,
            history_seconds: f64::INFINITY,
            ..Default::default()
        });

        assert!(
            detector.frame_duration.is_finite() && detector.frame_duration > 0.0,
            "frame_duration must be finite, got {}",
            detector.frame_duration
        );
        assert!(detector.analysis_rate >= MIN_ANALYSIS_RATE);
        assert!(detector.analysis_rate <= MAX_ANALYSIS_RATE);
        assert!(detector.history_seconds <= MAX_HISTORY_SECONDS);
        assert!(
            detector.history_frames <= MAX_HISTORY_FRAMES,
            "history_frames = {}",
            detector.history_frames
        );
    }

    #[test]
    fn absurd_analysis_rate_and_history_stay_bounded() {
        let mut detector = TempoDetector::new(TempoDetectorOptions {
            analysis_rate: 1e9,
            history_seconds: 1e9,
            ..Default::default()
        });

        assert_eq!(detector.analysis_rate, MAX_ANALYSIS_RATE);
        assert_eq!(detector.history_seconds, MAX_HISTORY_SECONDS);
        assert_eq!(detector.history_frames, MAX_HISTORY_FRAMES);

        // Feeding a signal must stay responsive and finite (no giant envelope,
        // no NaN frame times).
        for index in 0..480 {
            detector.process(index as f64 / 60.0, 1.0);
        }
        assert!(detector.bpm.is_finite());
        assert!(detector.confidence.is_finite());
    }

    #[test]
    fn non_finite_options_fall_back_to_the_defaults() {
        let detector = TempoDetector::new(TempoDetectorOptions {
            analysis_rate: f64::NAN,
            history_seconds: f64::NAN,
            ..Default::default()
        });
        let defaults = TempoDetectorOptions::default();

        assert_eq!(detector.analysis_rate, defaults.analysis_rate);
        assert_eq!(detector.history_seconds, defaults.history_seconds);
        assert!(detector.frame_duration.is_finite());
    }

    // tempo-detector.test.js:44-52 — "exposes a pure detector API with empty defaults"
    #[test]
    fn exposes_a_pure_detector_api_with_empty_defaults() {
        let detector = create_tempo_detector(TempoDetectorOptions::default());

        // `toBeInstanceOf(TempoDetector)` has no Rust equivalent; construction is
        // statically typed here.
        assert_eq!(detector.bpm, 0.0);
        assert_eq!(detector.confidence, 0.0);
        assert_eq!(detector.min_bpm, MIN_BPM);
        assert_eq!(detector.max_bpm, MAX_BPM);
    }

    // tempo-detector.test.js:54-63 — "detects a 120 BPM onset pattern within 2 BPM"
    #[test]
    fn detects_a_120_bpm_onset_pattern_within_2_bpm() {
        let mut detector = TempoDetector::new(TempoDetectorOptions::default());

        feed(&mut detector, &make_onset_signal(120.0, 12.0, 1));

        assert!(detector.bpm.is_finite());
        assert!(detector.confidence.is_finite());
        assert!(
            (detector.bpm - 120.0).abs() <= 2.0,
            "expected bpm within 2 of 120, got {}",
            detector.bpm
        );
        assert!(
            detector.confidence > 0.2,
            "expected confidence > 0.2, got {}",
            detector.confidence
        );
    }

    // tempo-detector.test.js:65-72 — "keeps a genuine half-time 87 BPM pattern at 87 BPM"
    #[test]
    fn keeps_a_genuine_half_time_87_bpm_pattern_at_87_bpm() {
        let mut detector = TempoDetector::new(TempoDetectorOptions::default());

        feed(&mut detector, &make_onset_signal(87.0, 12.0, 11));

        assert!(
            (detector.bpm - 87.0).abs() <= 2.0,
            "expected bpm within 2 of 87, got {}",
            detector.bpm
        );
        assert!(detector.bpm < 100.0);
    }

    // tempo-detector.test.js:74-81 — "stays inside the supported BPM range"
    #[test]
    fn stays_inside_the_supported_bpm_range() {
        let mut detector = TempoDetector::new(TempoDetectorOptions {
            min_bpm: 70.0,
            max_bpm: 200.0,
            ..Default::default()
        });

        feed(&mut detector, &make_onset_signal(174.0, 12.0, 12));

        assert!(detector.bpm >= 70.0);
        assert!(detector.bpm <= 200.0);
    }

    // tempo-detector.test.js:83-91 — "reports no tempo and low confidence for silence"
    #[test]
    fn reports_no_tempo_and_low_confidence_for_silence() {
        let mut detector = TempoDetector::new(TempoDetectorOptions::default());
        let silence = vec![0.0_f64; 12 * FPS];

        feed(&mut detector, &silence);

        assert_eq!(detector.bpm, 0.0);
        assert!(detector.confidence < 0.05);
    }

    // tempo-detector.test.js:93-101 — "resets to the initial state"
    #[test]
    fn resets_to_the_initial_state() {
        let mut detector = TempoDetector::new(TempoDetectorOptions::default());

        feed(&mut detector, &make_onset_signal(120.0, 6.0, 1));
        detector.reset();

        assert_eq!(detector.bpm, 0.0);
        assert_eq!(detector.confidence, 0.0);
    }

    // tempo-detector.test.js:103-111 — "ignores non-finite input"
    #[test]
    fn ignores_non_finite_input() {
        let mut detector = TempoDetector::new(TempoDetectorOptions::default());

        detector.process(f64::NAN, 1.0);
        detector.process(0.0, f64::NAN);

        assert_eq!(detector.bpm, 0.0);
        assert_eq!(detector.confidence, 0.0);
    }
}
