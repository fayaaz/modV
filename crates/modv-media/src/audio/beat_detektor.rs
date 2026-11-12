//! A Rust port of the vendored `BeatDetektor.js` (CubicFX BeatDetektor,
//! Copyright (c) 2009 Charles J. Cliffe, MIT).
//!
//! Source of truth: `lib/BeatDetektor.js` (665 lines) in this repository, used
//! by `src/renderer/src/application/setup-beat-detektor.js`, which constructs
//! `new BeatDetektor(85, 169)` (line 104) and calls
//! `beatDetektor.process(timer, features.complexSpectrum.real)` (line 128)
//! followed by `BassKick.process(beatDetektor)` / `isKick()` (lines 129-132).
//!
//! # What is ported
//!
//! * `BeatDetektor(config)` with the default kernel (`config_default`,
//!   BeatDetektor.js:209-222), `reset()` (:157-206) and `process()` (:229-591).
//! * `BeatDetektor.modules.vis.BassKick` (:598-611) as [`BassKick`].
//! * The whole detection pipeline: 128 quantised FFT ranges, two trailing
//!   moving averages per range (1/12 s), rising-edge trigger detection with the
//!   0.915 factor, gap quality via the reward-tolerance funnel, the BPM contest
//!   and its normalisation to the 60.0 finish line, and the quarter/half/beat
//!   counters.
//!
//! # Not ported (unused by modV)
//!
//! * `BeatDetektor.config_48_95` / `config_90_179` / `config_150_280` and the
//!   `alt_config` argument (`setup-beat-detektor.js` always uses the default
//!   kernel). [`BeatDetektorConfig`] therefore only carries the port's own
//!   switch, [`BeatDetektorConfig::ignore_silence`].
//! * `BeatDetektor.modules.vis.VU` (:615-663) — no modV code uses it.
//!
//! # Units (important)
//!
//! BeatDetektor's `current_bpm`, `winning_bpm`, `bpm_predict`, `ma_bpm_range`
//! and `maa_bpm_range` are **seconds per beat**, not beats per minute: the
//! contest index is `round((60 / current_bpm) * 10)` (:550) and the winner is
//! converted with `60.0 / (winner / 10.0)` (:568). The port keeps the JS field
//! names and units for parity and exposes [`BeatDetektor::bpm`] /
//! [`BeatDetektor::winning_bpm_value`] for convenience.
//!
//! # Deliberate deviations
//!
//! 1. **[`BeatDetektorConfig::ignore_silence`]** defaults to `true` (the JS has
//!    no such behaviour). In JS an all-zero spectrum makes the trigger test
//!    `0 * 0.915 >= 0` true for every range, and the 0.001 detection-quality
//!    floor then makes every range contribute to the draft, so pure digital
//!    silence manufactures an ~85.3 BPM estimate and a kick on every frame. The
//!    port treats an exact all-zero frame as "no trigger" for this frame and
//!    skips the draft/prediction stage, so silence produces no beats. The
//!    transport timing (`bpm_timer`, contest decay, counters) still advances, so
//!    an already-established BPM keeps ticking through a gap. Set the flag to
//!    `false` for bit-for-bit JS behaviour (asserted by
//!    `silence_matches_the_js_quality_floor_quirk_when_enabled`).
//! 2. `process` takes the same "real part of the complex spectrum" array as
//!    `setup-beat-detektor.js:128`. [`BeatDetektor::update`] is the engine-facing
//!    form that takes the full `[real, imag]` spectrum (plus the band triple for
//!    call-site symmetry; `setup-beat-detektor.js` never passes bands to
//!    BeatDetektor, so it is ignored).
//! 3. The JS splits the spectrum with a floating `range_step`
//!    (`fft_data.length / 128`) and indexes `fft_data` with fractional indices
//!    for non-divisible lengths (yielding `undefined` → `NaN`). The port splits
//!    on integer boundaries and ignores a trailing partial range; every modV
//!    buffer length is a power of two ≥ 512, so the two are identical in
//!    practice. Inputs shorter than 128 bins are ignored (the JS would produce
//!    `NaN` for the whole frame).
//! 4. `bpm_contest` / `bpm_contest_lo` are sparse integer-keyed JS arrays; the
//!    port uses [`BTreeMap`]s. JS iterates integer-like keys in ascending order,
//!    as does `BTreeMap`, so winner tie-breaks match.
//! 5. The JS `if (typeof(draft[draft_int]=='undefined')) draft[draft_int] = 0;`
//!    (:434) compares a string to `'undefined'`, so the condition is always
//!    truthy and every contribution *overwrites* the draft bucket instead of
//!    accumulating. Ported as-is (the `draft.insert` below), because it changes
//!    which bucket wins.
//! 6. `parseInt` truncation toward zero is `f64::trunc() as i32`; `Math.round`
//!    (half up) is `f64::round()` (half away from zero) — identical for the
//!    non-negative values involved.
//! 7. The unused `bpm_total`, `is_erratic`, `current_bpm_lo` bookkeeping is kept
//!    only where it is observable; otherwise it is dropped.

use std::collections::BTreeMap;

/// How many ranges the FFT is quantised into (BeatDetektor.js:210).
pub const BD_DETECTION_RANGES: usize = 128;
/// Rate in `1.0 / BD_DETECTION_RATE` seconds (:211).
pub const BD_DETECTION_RATE: f64 = 12.0;
/// Trigger ratio between the two trailing averages (:212).
pub const BD_DETECTION_FACTOR: f64 = 0.915;
/// Range and contest decay (:213).
pub const BD_QUALITY_DECAY: f64 = 0.6;
/// Use the top x % of contest results (:214).
pub const BD_QUALITY_TOLERANCE: f64 = 0.96;
/// Award weight (:215).
pub const BD_QUALITY_REWARD: f64 = 10.0;
/// Award step / roaming speed (:216).
pub const BD_QUALITY_STEP: f64 = 0.1;
/// At least this many ranges must agree to process a result (:217).
pub const BD_MINIMUM_CONTRIBUTIONS: usize = 6;
/// Contest values are normalised to this finish line (:218).
pub const BD_FINISH_LINE: f64 = 60.0;
/// The "funnel" that pulls ranges into alignment (:220).
pub const BD_REWARD_TOLERANCES: [f64; 9] = [0.001, 0.005, 0.01, 0.02, 0.04, 0.08, 0.10, 0.15, 0.30];
/// Reward multipliers matching [`BD_REWARD_TOLERANCES`] (:221).
pub const BD_REWARD_MULTIPLIERS: [f64; 9] = [
    20.0,
    10.0,
    8.0,
    1.0,
    1.0 / 2.0,
    1.0 / 4.0,
    1.0 / 8.0,
    1.0 / 16.0,
    1.0 / 32.0,
];
/// `new BeatDetektor(85, 169)` default minimum (BeatDetektor.js:117-118,
/// setup-beat-detektor.js:104).
pub const DEFAULT_BPM_MINIMUM: f64 = 85.0;
/// Default maximum (BeatDetektor.js:118).
pub const DEFAULT_BPM_MAXIMUM: f64 = 169.0;

/// Port-specific switches (the JS default kernel is fixed, see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeatDetektorConfig {
    /// Treat an all-zero spectrum as digital silence: no rising edges and no
    /// draft/prediction on that frame. Defaults to `true`; see deviation 1.
    pub ignore_silence: bool,
}

impl Default for BeatDetektorConfig {
    fn default() -> Self {
        Self {
            ignore_silence: true,
        }
    }
}

/// What one [`BeatDetektor::update`] call produced, in modV-friendly units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatDetektorUpdate {
    /// `BassKick.isKick()` for this frame (BeatDetektor.js:603-606).
    pub kick: bool,
    /// `beat_counter` after the call (:547).
    pub beat_counter: u64,
    /// `half_counter` after the call (:546).
    pub half_counter: u64,
    /// `quarter_counter` after the call (:545).
    pub quarter_counter: u64,
    /// `60 / current_bpm`; `0.0` before the first prediction.
    pub bpm: f64,
    /// `60 / winning_bpm` (the contest leader); `0.0` before the first winner.
    pub winning_bpm: f64,
}

/// The vendored BeatDetektor detector.
#[derive(Debug, Clone)]
pub struct BeatDetektor {
    config: BeatDetektorConfig,

    /// Requested BPM window (`BPM_MIN` / `BPM_MAX`, BeatDetektor.js:122-123).
    pub bpm_min: f64,
    pub bpm_max: f64,

    // Per-range state (:130-147).
    /// Current average (this sample) for range n.
    pub a_freq_range: Vec<f64>,
    /// Moving average of the frequency range.
    pub ma_freq_range: Vec<f64>,
    /// Moving average of the moving average.
    pub maa_freq_range: Vec<f64>,
    /// Timestamp of the last detection for range n.
    pub last_detection: Vec<f64>,
    /// Moving average of gap lengths (seconds per beat).
    pub ma_bpm_range: Vec<f64>,
    /// Moving average of moving average of gap lengths.
    pub maa_bpm_range: Vec<f64>,
    /// Range n quality attribute.
    pub detection_quality: Vec<f64>,
    /// Current trigger state for range n.
    pub detection: Vec<bool>,

    // Quality bookkeeping (:175-182).
    pub ma_quality_avg: f64,
    pub ma_quality_total: f64,
    pub quality_total: f64,
    pub quality_avg: f64,

    // Contest state (:178-200).
    /// Sparse `bpm*10`-indexed contest (seconds-per-beat key times ten).
    pub bpm_contest: BTreeMap<i32, f64>,
    /// Sparse coarse contest (`bpm` integer key).
    pub bpm_contest_lo: BTreeMap<i32, f64>,

    /// Current estimate in **seconds per beat**; `0.0` means "no estimate".
    pub current_bpm: f64,
    /// Coarse current estimate in seconds per beat.
    pub current_bpm_lo: f64,
    /// Predicted seconds per beat from the draft stage.
    pub bpm_predict: f64,

    /// Contest leader in seconds per beat.
    pub winning_bpm: f64,
    /// Contest leader's value.
    pub win_val: f64,
    /// Coarse contest leader in seconds per beat.
    pub winning_bpm_lo: f64,
    /// Coarse contest leader's value.
    pub win_val_lo: f64,
    /// Contest leader's integer index.
    pub win_bpm_int: i32,
    /// Coarse contest leader's integer index.
    pub win_bpm_int_lo: i32,

    /// Average absolute gap between the draft's ranges (:469).
    pub bpm_offset: f64,
    /// Last absolute timestamp (`last_timer`, :199).
    pub last_timer: f64,
    /// Delta since the previous call (`last_update`, :200).
    pub last_update: f64,
    /// Time accumulated towards the next counter tick (:202).
    pub bpm_timer: f64,

    /// Beat counter (`beat_counter`, :547).
    pub beat_counter: u64,
    /// Half-note counter (:546).
    pub half_counter: u64,
    /// Quarter-note counter, incremented once per 16th-note interval (:545).
    pub quarter_counter: u64,
}

impl BeatDetektor {
    /// Creates a detector with the default kernel, mirroring
    /// `new BeatDetektor(bpm_minimum, bpm_maximum)` (BeatDetektor.js:115-155).
    pub fn new(base_bpm: f64, max_bpm: f64) -> Self {
        Self::with_config(base_bpm, max_bpm, BeatDetektorConfig::default())
    }

    /// Creates a detector with explicit port switches.
    pub fn with_config(bpm_minimum: f64, bpm_maximum: f64, config: BeatDetektorConfig) -> Self {
        let mut detector = Self {
            config,
            bpm_min: bpm_minimum,
            bpm_max: bpm_maximum,
            a_freq_range: vec![0.0; BD_DETECTION_RANGES],
            ma_freq_range: vec![0.0; BD_DETECTION_RANGES],
            maa_freq_range: vec![0.0; BD_DETECTION_RANGES],
            last_detection: vec![0.0; BD_DETECTION_RANGES],
            ma_bpm_range: vec![0.0; BD_DETECTION_RANGES],
            maa_bpm_range: vec![0.0; BD_DETECTION_RANGES],
            detection_quality: vec![0.0; BD_DETECTION_RANGES],
            detection: vec![false; BD_DETECTION_RANGES],
            ma_quality_avg: 0.0,
            ma_quality_total: 0.0,
            quality_total: 0.0,
            quality_avg: 0.0,
            bpm_contest: BTreeMap::new(),
            bpm_contest_lo: BTreeMap::new(),
            current_bpm: 0.0,
            current_bpm_lo: 0.0,
            bpm_predict: 0.0,
            winning_bpm: 0.0,
            win_val: 0.0,
            winning_bpm_lo: 0.0,
            win_val_lo: 0.0,
            win_bpm_int: 0,
            win_bpm_int_lo: 0,
            bpm_offset: 0.0,
            last_timer: 0.0,
            last_update: 0.0,
            bpm_timer: 0.0,
            beat_counter: 0,
            half_counter: 0,
            quarter_counter: 0,
        };
        detector.reset();
        detector
    }

    /// Resets all detection, quality and contest state
    /// (BeatDetektor.js:157-206).
    pub fn reset(&mut self) {
        for i in 0..BD_DETECTION_RANGES {
            self.a_freq_range[i] = 0.0;
            self.ma_freq_range[i] = 0.0;
            self.maa_freq_range[i] = 0.0;
            self.last_detection[i] = 0.0;

            // Linear sweep from the slowest to the fastest allowed beat
            // (:168-169).
            self.ma_bpm_range[i] = 60.0 / self.bpm_min
                + ((60.0 / self.bpm_max - 60.0 / self.bpm_min)
                    * (i as f64 / BD_DETECTION_RANGES as f64));
            self.maa_bpm_range[i] = self.ma_bpm_range[i];

            self.detection_quality[i] = 0.0;
            self.detection[i] = false;
        }

        self.ma_quality_avg = 0.0;
        self.ma_quality_total = 0.0;
        self.bpm_contest.clear();
        self.bpm_contest_lo.clear();

        self.quality_total = 0.0;
        self.quality_avg = 0.0;

        self.current_bpm = 0.0;
        self.current_bpm_lo = 0.0;

        self.winning_bpm = 0.0;
        self.win_val = 0.0;
        self.winning_bpm_lo = 0.0;
        self.win_val_lo = 0.0;

        self.win_bpm_int = 0;
        self.win_bpm_int_lo = 0;

        self.bpm_predict = 0.0;

        self.bpm_offset = 0.0;
        self.last_timer = 0.0;
        self.last_update = 0.0;

        self.bpm_timer = 0.0;
        self.beat_counter = 0;
        self.half_counter = 0;
        self.quarter_counter = 0;
    }

    /// Current estimate in beats per minute; `0.0` before the first prediction.
    pub fn bpm(&self) -> f64 {
        if self.current_bpm == 0.0 {
            0.0
        } else {
            60.0 / self.current_bpm
        }
    }

    /// Contest leader in beats per minute; `0.0` before the first winner.
    pub fn winning_bpm_value(&self) -> f64 {
        if self.winning_bpm == 0.0 {
            0.0
        } else {
            60.0 / self.winning_bpm
        }
    }

    /// `process(timer_seconds, fft_data)` (BeatDetektor.js:229): `fft_data` is
    /// the real part of the complex spectrum, exactly what
    /// `setup-beat-detektor.js:128` passes.
    ///
    /// The first call only seeds the timer and does nothing else (:231), as
    /// does a backwards jump or a gap longer than one second (:232, :239).
    pub fn process(&mut self, timer_seconds: f64, fft_data: &[f32]) {
        let silent = fft_data.iter().all(|&value| value == 0.0);
        self.process_inner(timer_seconds, fft_data.len(), silent, |i| {
            fft_data[i] as f64
        });
    }

    /// Engine-facing form: takes the full complex spectrum
    /// (`MeydaFeatures::complex_spectrum`) and the bass/mid/treble triple.
    ///
    /// Only the real part is used, matching
    /// `beatDetektor.process(timer, features.complexSpectrum.real)`
    /// (setup-beat-detektor.js:127-128). `bands` is accepted for call-site
    /// symmetry with the other analysers; `setup-beat-detektor.js` never hands
    /// bands to BeatDetektor, so it is ignored here as well.
    pub fn update(
        &mut self,
        time_seconds: f64,
        complex_spectrum: &[[f32; 2]],
        bands: &[f32; 3],
    ) -> BeatDetektorUpdate {
        let _ = bands;
        let silent = complex_spectrum.iter().all(|bin| bin[0] == 0.0);
        self.process_inner(time_seconds, complex_spectrum.len(), silent, |i| {
            complex_spectrum[i][0] as f64
        });

        let mut kick = BassKick::new();
        kick.process(self);

        BeatDetektorUpdate {
            kick: kick.is_kick(),
            beat_counter: self.beat_counter,
            half_counter: self.half_counter,
            quarter_counter: self.quarter_counter,
            bpm: self.bpm(),
            winning_bpm: self.winning_bpm_value(),
        }
    }

    /// The shared body of [`Self::process`] / [`Self::update`]. `bin` returns
    /// the (signed) real component of bin `i`.
    fn process_inner(
        &mut self,
        timer_seconds: f64,
        bin_count: usize,
        silent: bool,
        bin: impl Fn(usize) -> f64,
    ) {
        // :231 — ignore a zero start time.
        if self.last_timer == 0.0 {
            self.last_timer = timer_seconds;
            return;
        }
        // :232 — a backwards clock means the audio graph was rebuilt.
        if self.last_timer > timer_seconds {
            self.reset();
            return;
        }

        let timestamp = timer_seconds;
        self.last_update = timer_seconds - self.last_timer;
        self.last_timer = timer_seconds;

        // :239 — gaps longer than a second reset the detector.
        if self.last_update > 1.0 {
            self.reset();
            return;
        }

        let bpm_floor = 60.0 / self.bpm_max;
        let bpm_ceil = 60.0 / self.bpm_min;
        // Deviation 1: digital silence never triggers a detection and takes no
        // part in the draft stage, unless the JS-parity switch is off.
        let ignore_silence = silent && self.config.ignore_silence;

        // Even when the frame is treated as silence the two averages and the
        // counters must advance (deviation 1), so the transport keeps ticking.
        let range_step = bin_count / BD_DETECTION_RANGES;
        if range_step == 0 {
            return;
        }

        for range in 0..BD_DETECTION_RANGES {
            let start = range * range_step;
            let end = start + range_step;
            if end > bin_count {
                break;
            }

            let mut average = 0.0;
            for i in start..end {
                average += bin(i).abs();
            }
            self.a_freq_range[range] = average / range_step as f64;

            // Two averages chase the instantaneous one at 1/BD_DETECTION_RATE
            // (:268-270).
            self.ma_freq_range[range] -= (self.ma_freq_range[range] - self.a_freq_range[range])
                * self.last_update
                * BD_DETECTION_RATE;
            self.maa_freq_range[range] -= (self.maa_freq_range[range] - self.ma_freq_range[range])
                * self.last_update
                * BD_DETECTION_RATE;

            // Rising edge when the leading average clears the trailing one by
            // the detection factor (:273). An all-zero frame satisfies
            // `0 >= 0`, which is why the JS reports detections in digital
            // silence; the port suppresses that (deviation 1).
            let detected = !ignore_silence
                && (self.ma_freq_range[range] * BD_DETECTION_FACTOR >= self.maa_freq_range[range]);

            // Clamp the gap averages to the input BPM range (:278-281).
            self.ma_bpm_range[range] = self.ma_bpm_range[range].clamp(bpm_floor, bpm_ceil);
            self.maa_bpm_range[range] = self.maa_bpm_range[range].clamp(bpm_floor, bpm_ceil);

            let mut rewarded = false;

            if !self.detection[range] && detected {
                let mut trigger_gap = timestamp - self.last_detection[range];

                if trigger_gap < bpm_ceil && trigger_gap > bpm_floor {
                    // The reward funnel (:297-304).
                    for (tolerance, multiplier) in BD_REWARD_TOLERANCES
                        .iter()
                        .zip(BD_REWARD_MULTIPLIERS.iter())
                    {
                        if (self.ma_bpm_range[range] - trigger_gap).abs()
                            < self.ma_bpm_range[range] * tolerance
                        {
                            self.detection_quality[range] += BD_QUALITY_REWARD * multiplier;
                            rewarded = true;
                        }
                    }

                    if rewarded {
                        self.last_detection[range] = timestamp;
                    }
                } else if trigger_gap >= bpm_ceil {
                    // Too slow: test a half beat (:311-334).
                    trigger_gap /= 2.0;

                    if trigger_gap < bpm_ceil && trigger_gap > bpm_floor {
                        for (tolerance, multiplier) in BD_REWARD_TOLERANCES
                            .iter()
                            .zip(BD_REWARD_MULTIPLIERS.iter())
                        {
                            if (self.ma_bpm_range[range] - trigger_gap).abs()
                                < self.ma_bpm_range[range] * tolerance
                            {
                                self.detection_quality[range] += BD_QUALITY_REWARD * multiplier;
                                rewarded = true;
                            }
                        }
                    }

                    if !rewarded {
                        trigger_gap *= 2.0;
                    }
                    self.last_detection[range] = timestamp;
                }

                if rewarded {
                    let mut quality_step =
                        (self.detection_quality[range] / self.quality_avg) * BD_QUALITY_STEP;
                    if quality_step > 1.0 {
                        quality_step = 1.0;
                    }

                    self.ma_bpm_range[range] -=
                        (self.ma_bpm_range[range] - trigger_gap) * quality_step;
                    self.maa_bpm_range[range] -=
                        (self.maa_bpm_range[range] - self.ma_bpm_range[range]) * quality_step;
                } else if trigger_gap >= bpm_floor && trigger_gap <= bpm_ceil {
                    if self.detection_quality[range] < self.quality_avg * BD_QUALITY_TOLERANCE
                        && self.current_bpm != 0.0
                    {
                        self.ma_bpm_range[range] -=
                            (self.ma_bpm_range[range] - trigger_gap) * BD_QUALITY_STEP;
                        self.maa_bpm_range[range] -= (self.maa_bpm_range[range]
                            - self.ma_bpm_range[range])
                            * BD_QUALITY_STEP;
                    }
                    self.detection_quality[range] -= BD_QUALITY_STEP;
                } else if trigger_gap >= bpm_ceil {
                    if (self.detection_quality[range] < self.quality_avg * BD_QUALITY_TOLERANCE)
                        && self.current_bpm != 0.0
                    {
                        self.ma_bpm_range[range] -=
                            (self.ma_bpm_range[range] - self.current_bpm) * 0.5;
                        self.maa_bpm_range[range] -=
                            (self.maa_bpm_range[range] - self.ma_bpm_range[range]) * 0.5;
                    }
                    self.detection_quality[range] -= BD_QUALITY_STEP;
                }
            }

            if (!rewarded && timestamp - self.last_detection[range] > bpm_ceil)
                || (detected
                    && (self.ma_bpm_range[range] - self.current_bpm).abs() > self.bpm_offset)
            {
                self.detection_quality[range] -= self.detection_quality[range]
                    * BD_QUALITY_STEP
                    * BD_QUALITY_DECAY
                    * self.last_update;
            }

            // Quality bottomed out, set to the floor (:372).
            if self.detection_quality[range] < 0.001 {
                self.detection_quality[range] = 0.001;
            }

            self.detection[range] = detected;
        }

        // Total contribution weight (:380).
        self.quality_total = self.detection_quality.iter().sum();
        self.quality_avg = self.quality_total / BD_DETECTION_RANGES as f64;

        if self.quality_total != 0.0 {
            // :401-406 — three chasing averages; the last line decays the
            // leading one.
            self.ma_quality_avg +=
                (self.quality_avg - self.ma_quality_avg) * self.last_update * BD_DETECTION_RATE
                    / 2.0;
            self.ma_quality_total +=
                (self.quality_total - self.ma_quality_total) * self.last_update * BD_DETECTION_RATE
                    / 2.0;
            self.ma_quality_avg -= 0.98 * self.ma_quality_avg * self.last_update * 3.0;
        } else {
            self.quality_avg = 0.001;
        }

        if self.ma_quality_total <= 0.0 {
            self.ma_quality_total = 0.001;
        }
        if self.ma_quality_avg <= 0.0 {
            self.ma_quality_avg = 0.001;
        }

        let mut avg_bpm_offset = 0.0;
        let mut offset_test_bpm = self.current_bpm;
        let mut draft: BTreeMap<i32, f64> = BTreeMap::new();
        let mut bpm_contributions = 0usize;

        // Deviation 1: pure digital silence takes no part in the draft stage.
        if self.quality_avg != 0.0 && !ignore_silence {
            for range in 0..BD_DETECTION_RANGES {
                if self.detection_quality[range] * BD_QUALITY_TOLERANCE >= self.ma_quality_avg
                    && self.ma_bpm_range[range] < bpm_ceil
                    && self.ma_bpm_range[range] > bpm_floor
                {
                    // :429-432 — the draft index is a tenth-of-a-second-per-beat
                    // bucket.
                    let mut draft_float = ((60.0 / self.maa_bpm_range[range]) * 1000.0).round();
                    let current_ticks = if self.current_bpm != 0.0 {
                        (60.0 / self.current_bpm) * 1000.0
                    } else {
                        f64::INFINITY
                    };
                    draft_float = if (draft_float.ceil() - current_ticks).abs()
                        < (draft_float.floor() - current_ticks).abs()
                    {
                        (draft_float / 10.0).ceil()
                    } else {
                        (draft_float / 10.0).floor()
                    };
                    let draft_int = (draft_float / 10.0).trunc() as i32;

                    // :434 — the JS `typeof(draft[draft_int]=='undefined')` is
                    // always truthy, so every contribution resets the bucket
                    // (deviation 5).
                    draft.insert(draft_int, 0.0);
                    *draft.get_mut(&draft_int).unwrap() +=
                        self.detection_quality[range] / self.quality_avg;
                    bpm_contributions += 1;

                    if offset_test_bpm == 0.0 {
                        offset_test_bpm = self.maa_bpm_range[range];
                    } else {
                        avg_bpm_offset += (offset_test_bpm - self.maa_bpm_range[range]).abs();
                    }
                }
            }
        }

        let has_prediction = bpm_contributions >= BD_MINIMUM_CONTRIBUTIONS;

        if has_prediction {
            let mut draft_winner = 0;
            let mut win_val = 0.0;
            for (&bucket, &value) in draft.iter() {
                if value > win_val {
                    win_val = value;
                    draft_winner = bucket;
                }
            }

            // :466 — 60 / (bucket / 10) is seconds per beat.
            self.bpm_predict = 60.0 / (draft_winner as f64 / 10.0);
            self.bpm_offset = avg_bpm_offset / bpm_contributions as f64;

            if self.current_bpm == 0.0 {
                self.current_bpm = self.bpm_predict;
            }
        }

        if self.current_bpm != 0.0 && self.bpm_predict != 0.0 {
            self.current_bpm -= (self.current_bpm - self.bpm_predict) * self.last_update;
        }

        // The BPM contest (:480-528).
        let mut contest_max = 0.0f64;
        for (&key, &value) in self.bpm_contest.iter() {
            if contest_max < value {
                contest_max = value;
            }
            if value > BD_FINISH_LINE / 2.0 {
                let draft_int_lo = (key as f64 / 10.0).round() as i32;
                *self.bpm_contest_lo.entry(draft_int_lo).or_insert(0.0) +=
                    (value / 6.0) * self.last_update;
            }
        }

        if contest_max > BD_FINISH_LINE {
            for value in self.bpm_contest.values_mut() {
                *value = (*value / contest_max) * BD_FINISH_LINE;
            }
        }

        contest_max = self.bpm_contest_lo.values().copied().fold(0.0f64, f64::max);
        if contest_max > BD_FINISH_LINE {
            for value in self.bpm_contest_lo.values_mut() {
                *value = (*value / contest_max) * BD_FINISH_LINE;
            }
        }

        for value in self.bpm_contest.values_mut() {
            *value -= *value * (self.last_update / BD_DETECTION_RATE);
        }
        for value in self.bpm_contest_lo.values_mut() {
            *value -= *value * (self.last_update / BD_DETECTION_RATE);
        }

        self.bpm_timer += self.last_update;

        // Attempt to display the beat at the beat interval (:536-589).
        if self.bpm_timer > self.winning_bpm / 4.0 && self.current_bpm != 0.0 {
            self.win_val = 0.0;
            self.win_val_lo = 0.0;

            // :541 guards the wrap with `if (this.winning_bpm)`; without that
            // the subtraction would never advance on the first pass.
            if self.winning_bpm != 0.0 {
                while self.bpm_timer > self.winning_bpm / 4.0 {
                    self.bpm_timer -= self.winning_bpm / 4.0;
                }
            }

            self.quarter_counter += 1;
            self.half_counter = self.quarter_counter / 2;
            self.beat_counter = self.quarter_counter / 4;

            let idx = ((60.0 / self.current_bpm) * 10.0).round() as i32;
            *self.bpm_contest.entry(idx).or_insert(0.0) += BD_QUALITY_REWARD;

            let mut winner = 0;
            for (&key, &value) in self.bpm_contest.iter() {
                if self.win_val < value {
                    winner = key;
                    self.win_val = value;
                }
            }
            if winner != 0 {
                self.win_bpm_int = winner;
                self.winning_bpm = 60.0 / (winner as f64 / 10.0);
            }

            let mut winner_lo = 0;
            for (&key, &value) in self.bpm_contest_lo.iter() {
                if self.win_val_lo < value {
                    winner_lo = key;
                    self.win_val_lo = value;
                }
            }
            if winner_lo != 0 {
                self.win_bpm_int_lo = winner_lo;
                self.winning_bpm_lo = 60.0 / winner_lo as f64;
            }
        }
    }
}

impl Default for BeatDetektor {
    fn default() -> Self {
        Self::new(DEFAULT_BPM_MINIMUM, DEFAULT_BPM_MAXIMUM)
    }
}

/// `BeatDetektor.modules.vis.BassKick` (BeatDetektor.js:598-611): flags a kick
/// when both of the two lowest ranges trigger, or when the lowest range's
/// leading average is 1.4× its trailing average.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BassKick {
    is_kick: bool,
}

impl BassKick {
    /// `new BassKick()` (:598-601).
    pub fn new() -> Self {
        Self { is_kick: false }
    }

    /// `BassKick.process(det)` (:603-606).
    pub fn process(&mut self, det: &BeatDetektor) {
        self.is_kick = (det.detection[0] && det.detection[1])
            || (det.ma_freq_range[0] / det.maa_freq_range[0]) > 1.4;
    }

    /// `BassKick.isKick()` (:608-610).
    pub fn is_kick(&self) -> bool {
        self.is_kick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FFT_SIZE: usize = 512;
    const FRAME_RATE: f64 = 60.0;

    /// A synthetic "bass kick" spectrum: the lowest ten bins are lit, the rest
    /// are zero.
    fn kick_spectrum() -> Vec<f32> {
        (0..FFT_SIZE)
            .map(|i| if i < 10 { 1.0 } else { 0.0 })
            .collect()
    }

    /// A silent spectrum.
    fn silence() -> Vec<f32> {
        vec![0.0; FFT_SIZE]
    }

    /// Runs a synthetic click train at `bpm` for `seconds` at 60 fps, returning
    /// the detector and the number of frames the BassKick flagged.
    ///
    /// The first call seeds the timer with a non-zero timestamp (the JS uses
    /// `performance.now() / 1000`, never 0), matching
    /// `setup-beat-detektor.js:125`.
    fn run_click_train(bpm: f64, seconds: f64) -> (BeatDetektor, u32) {
        let mut detector = BeatDetektor::new(DEFAULT_BPM_MINIMUM, DEFAULT_BPM_MAXIMUM);
        let dt = 1.0 / FRAME_RATE;
        let beat_period = 60.0 / bpm;
        let frames = (seconds * FRAME_RATE) as usize;
        let mut kicks = 0;
        let mut bass_kick = BassKick::new();

        for frame in 0..frames {
            let time = frame as f64 * dt;
            let on_beat = time % beat_period < dt * 0.75;
            let spectrum = if on_beat { kick_spectrum() } else { silence() };
            detector.process(time + 0.016, &spectrum);
            bass_kick.process(&detector);
            if bass_kick.is_kick() {
                kicks += 1;
            }
        }

        (detector, kicks)
    }

    #[test]
    fn new_matches_the_js_defaults_and_initial_state() {
        let detector = BeatDetektor::new(85.0, 169.0);

        assert_eq!(detector.bpm_min, 85.0);
        assert_eq!(detector.bpm_max, 169.0);
        assert_eq!(detector.beat_counter, 0);
        assert_eq!(detector.half_counter, 0);
        assert_eq!(detector.quarter_counter, 0);
        assert_eq!(detector.current_bpm, 0.0);
        assert_eq!(detector.bpm(), 0.0);
        assert_eq!(detector.winning_bpm_value(), 0.0);
        assert!(detector.bpm_contest.is_empty());

        // The initial gap sweep spans exactly the requested BPM window and
        // starts at the slowest end (60 / 85 seconds per beat).
        let seconds_per_beat = 60.0 / 85.0;
        assert!((detector.ma_bpm_range[0] - seconds_per_beat).abs() < 1e-12);
        assert!((detector.maa_bpm_range[0] - detector.ma_bpm_range[0]).abs() < 1e-12);
        assert!(detector.ma_bpm_range[BD_DETECTION_RANGES - 1] < detector.ma_bpm_range[0]);
        assert!(detector.ma_bpm_range[BD_DETECTION_RANGES - 1] > 60.0 / 169.0);
        assert_eq!(detector.a_freq_range.len(), BD_DETECTION_RANGES);
        assert_eq!(detector.detection.len(), BD_DETECTION_RANGES);
    }

    #[test]
    fn default_is_beat_detektor_85_169() {
        let detector = BeatDetektor::default();
        assert_eq!(detector.bpm_min, DEFAULT_BPM_MINIMUM);
        assert_eq!(detector.bpm_max, DEFAULT_BPM_MAXIMUM);
    }

    #[test]
    fn first_call_only_seeds_the_timer() {
        let mut detector = BeatDetektor::new(85.0, 169.0);

        detector.process(123.0, &kick_spectrum());

        assert_eq!(detector.last_timer, 123.0);
        assert_eq!(detector.last_update, 0.0);
        assert_eq!(detector.beat_counter, 0);
        assert_eq!(detector.current_bpm, 0.0);
        assert!(detector.detection.iter().all(|&d| !d));
    }

    #[test]
    fn a_backwards_timer_resets_the_detector() {
        let mut detector = BeatDetektor::new(85.0, 169.0);
        let (mut started, _) = run_click_train(120.0, 3.0);
        assert!(started.bpm() > 0.0);

        started.process(0.5, &kick_spectrum());

        assert_eq!(started.last_timer, 0.0);
        assert_eq!(started.beat_counter, 0);
        assert_eq!(started.current_bpm, 0.0);
        assert_eq!(detector.beat_counter, 0);

        detector.process(10.0, &kick_spectrum());
        detector.process(9.0, &kick_spectrum());
        assert_eq!(detector.last_timer, 0.0);
    }

    #[test]
    fn a_gap_longer_than_one_second_resets_the_detector() {
        let mut detector = BeatDetektor::new(85.0, 169.0);
        detector.process(1.0, &kick_spectrum());
        detector.process(1.5, &kick_spectrum());
        detector.process(3.0, &kick_spectrum());

        assert_eq!(detector.last_timer, 0.0);
        assert_eq!(detector.current_bpm, 0.0);
        assert_eq!(detector.beat_counter, 0);
    }

    #[test]
    fn silence_produces_no_beats_or_kicks() {
        let mut detector = BeatDetektor::new(85.0, 169.0);
        let mut bass_kick = BassKick::new();
        let mut kicks = 0;

        for frame in 0..600 {
            detector.process(frame as f64 / FRAME_RATE + 0.016, &silence());
            bass_kick.process(&detector);
            if bass_kick.is_kick() {
                kicks += 1;
            }
        }

        assert_eq!(detector.beat_counter, 0);
        assert_eq!(detector.quarter_counter, 0);
        assert_eq!(detector.current_bpm, 0.0);
        assert_eq!(detector.bpm(), 0.0);
        assert_eq!(kicks, 0);
        // The quality floor is still applied, exactly as in the JS.
        assert!(detector.quality_total > 0.0);
    }

    #[test]
    fn silence_matches_the_js_quality_floor_quirk_when_enabled() {
        // With the port switch off, BeatDetektor.js's `0 * 0.915 >= 0` trigger
        // and its 0.001 quality floor manufacture an estimate from silence.
        let config = BeatDetektorConfig {
            ignore_silence: false,
        };
        let mut detector =
            BeatDetektor::with_config(DEFAULT_BPM_MINIMUM, DEFAULT_BPM_MAXIMUM, config);

        for frame in 0..600 {
            detector.process(frame as f64 / FRAME_RATE + 0.016, &silence());
        }

        assert!(detector.current_bpm > 0.0);
        assert!(detector.beat_counter > 0);
        // 60 / 0.001-tolerance draft lands on the slowest range: ~85.3 BPM.
        assert!(
            (detector.bpm() - 85.3).abs() < 1.0,
            "bpm {}",
            detector.bpm()
        );
    }

    #[test]
    fn detects_a_120_bpm_click_train() {
        let (detector, kicks) = run_click_train(120.0, 20.0);

        // 120 BPM over 20 s is 40 beats; the counter increments once per
        // processed frame at most, so allow a one-sided margin.
        assert!(
            (30..=50).contains(&detector.beat_counter),
            "beats {}",
            detector.beat_counter
        );
        assert!(
            (detector.bpm() - 120.0).abs() < 4.0,
            "bpm {}",
            detector.bpm()
        );
        assert!(
            (detector.winning_bpm_value() - 120.0).abs() < 4.0,
            "winning bpm {}",
            detector.winning_bpm_value()
        );
        assert!(kicks > 0);
        assert!(kicks < 600, "kicks {kicks}");
    }

    #[test]
    fn detects_a_128_bpm_click_train() {
        let (detector, _) = run_click_train(128.0, 20.0);

        assert!(
            (32..=53).contains(&detector.beat_counter),
            "beats {}",
            detector.beat_counter
        );
        assert!(
            (detector.bpm() - 128.0).abs() < 5.0,
            "bpm {}",
            detector.bpm()
        );
    }

    #[test]
    fn detects_a_100_bpm_click_train() {
        let (detector, _) = run_click_train(100.0, 20.0);

        assert!(
            (24..=42).contains(&detector.beat_counter),
            "beats {}",
            detector.beat_counter
        );
        assert!(
            (detector.bpm() - 100.0).abs() < 4.0,
            "bpm {}",
            detector.bpm()
        );
    }

    #[test]
    fn update_matches_process_on_the_same_spectrum() {
        let mut via_process = BeatDetektor::new(85.0, 169.0);
        let mut via_update = BeatDetektor::new(85.0, 169.0);
        let dt = 1.0 / FRAME_RATE;
        let mut last_update = None;
        let mut saw_kick = false;

        for frame in 0..1_200 {
            let time = frame as f64 * dt + 0.016;
            let on_beat = (frame as f64 * dt) % 0.5 < dt * 0.75;
            let real = if on_beat { kick_spectrum() } else { silence() };
            let complex: Vec<[f32; 2]> = real.iter().map(|&value| [value, 0.0]).collect();

            via_process.process(time, &real);
            let update = via_update.update(time, &complex, &[0.0, 0.0, 0.0]);

            assert_eq!(via_process.quarter_counter, via_update.quarter_counter);
            assert_eq!(
                via_process.beat_counter, update.beat_counter,
                "frame {frame}"
            );
            saw_kick |= update.kick;
            last_update = Some(update);
        }

        let update = last_update.expect("frames were processed");
        assert!(saw_kick);
        assert!((update.bpm - via_process.bpm()).abs() < 1e-12);
        assert!((update.bpm - 120.0).abs() < 4.0);
        assert_eq!(update.half_counter, update.quarter_counter / 2);
        assert_eq!(update.beat_counter, update.quarter_counter / 4);
    }

    #[test]
    fn bass_kick_flags_the_bass_ranges() {
        let mut detector = BeatDetektor::new(85.0, 169.0);
        let mut bass_kick = BassKick::new();
        assert!(!bass_kick.is_kick());

        detector.process(1.0, &kick_spectrum());
        detector.maa_freq_range[0] = 1.0;
        detector.ma_freq_range[0] = 1.0;
        detector.detection[0] = false;
        detector.detection[1] = false;
        bass_kick.process(&detector);
        assert!(!bass_kick.is_kick());

        detector.ma_freq_range[0] = 1.5;
        bass_kick.process(&detector);
        assert!(bass_kick.is_kick());

        detector.ma_freq_range[0] = 1.0;
        detector.detection[0] = true;
        detector.detection[1] = true;
        bass_kick.process(&detector);
        assert!(bass_kick.is_kick());
    }

    #[test]
    fn zero_length_and_short_spectra_do_not_panic() {
        let mut detector = BeatDetektor::new(85.0, 169.0);
        detector.process(1.0, &[]);
        detector.process(1.016, &[]);
        detector.process(1.032, &[1.0; 8]);

        // Shorter than the 128 ranges: the frame carries no usable spectrum.
        assert_eq!(detector.beat_counter, 0);

        let mut empty = BeatDetektor::new(85.0, 169.0);
        let update = empty.update(2.0, &[], &[0.0, 0.0, 0.0]);
        assert!(!update.kick);
        assert_eq!(update.bpm, 0.0);
    }
}
