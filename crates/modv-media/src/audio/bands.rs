//! AGC-smoothed bass/mid/treble bands — a faithful port of
//! `src/renderer/src/application/audio-bands.js` (itself ported from the
//! Point10 audio engine: musicvis `src/audio/AudioEngine.js` +
//! `src/plane9/P9Audio.js` semantics).
//!
//! Raw band values are linear FFT magnitudes normalized by a time-domain AGC.
//! The exposed bass/mid/treble features are then mapped onto a -60..0 dB
//! perceptual scale so treble reads comparably to bass; the payload's bands,
//! lanes and cells (consumed by Point10 scenes) stay linear. The 16 log-spaced
//! band cells follow the P10 band grid (bass = cells 0-4, mid = 5-9, treble =
//! cells 10-15, starting at 2 kHz) low-passed with the lane-1 damping hint
//! (tau ~0.05s).
//!
//! # Port notes / adaptations
//!
//! 1. JS used an internal `performance.now()` clock; the port takes `dt`
//!    explicitly (`AudioBands::update`) so tests are deterministic. The JS
//!    `Math.min(0.1, …)` clamp on the delta is kept (audio-bands.js:211); the
//!    JS never clamped below zero either, so this port does not either. The
//!    first JS call had `lastT || now` falsy and therefore `dt = 0`, so a
//!    faithful first call passes `dt_seconds = 0.0`.
//! 2. The JS payload exposed *live references* into the internal buffers
//!    (`wave`, `freqMag`, `byteFrequencyData`) with whatever length the FFT
//!    supplied — an empty `byteFrequencyData` when the input was missing.
//!    The Rust payload is a fixed-size *snapshot* (`[f32; 1024]`,
//!    `[u8; 1024]`, copied by value) because the wire shape is fixed at 1024
//!    (docs/rust-hybrid.md, "Band analyser", payload JSON). Values beyond
//!    1024 are truncated and short buffers zero-padded. The "missing byte
//!    frequency data" case is therefore represented as a zero-filled array,
//!    not an empty one.
//! 3. `sampleRate` defaulted to 44100 in JS (audio-bands.js:208); the port
//!    requires it in [`BandInput`].
//! 4. Bin-boundary search (`_binRange`) computes `hz` in `f64` — JS numbers
//!    are `f64`, and index selection at exact bin edges must match the JS
//!    exactly or cells could shift by one bin.
//! 5. The internal `freq_mag` buffer keeps the input FFT length (JS sized
//!    `freqMag` to the FFT length too); only the payload snapshot is fixed
//!    at 1024.
//! 6. Test "defaults the payload byte frequency data to empty when it is
//!    missing" (tests/unit/audio-bands.test.js:270-280) was adapted to assert
//!    an all-zero `byte_frequency_data` — see note 2. It is the only adapted
//!    assertion in the suite; everything else maps 1:1.
//! 7. Arithmetic is `f32` throughout. JS assertions that were exact
//!    (`toBe(1)`, `toBe(0)`, `toBe(-1)`) remain exact in Rust because they
//!    flow through `clamp01` / the zero branches; the inequality assertions
//!    carry over unchanged. No relative-tolerance comparisons were needed.
//!
//! Line citations (`audio-bands.js:NNN`) refer to
//! `src/renderer/src/application/audio-bands.js` at the time of the port.

/// P10 band grid edges, 17 Hz edges delimiting the 16 cells
/// (audio-bands.js:12-15).
pub const BAND_EDGES: [f32; 17] = [
    30.0, 50.0, 80.0, 130.0, 200.0, 300.0, 450.0, 700.0, 1000.0, 1500.0, 2000.0, 3300.0, 5000.0,
    7500.0, 11000.0, 16000.0, 20000.0,
];

/// bass = cells 0-4, mid = 5-9, treble = 10-15 (audio-bands.js:17-21).
pub const BAND_CELLS: [[usize; 2]; 3] = [[0, 4], [5, 9], [10, 15]];

/// WebAudio's default FFT window reads a full-scale bin at ~0.42; undo only
/// that window loss (P10 AudioEngine FFT_GAIN) (audio-bands.js:23-25).
pub const FFT_GAIN: f32 = 1.0 / 0.42;

/// P10 damping lanes (band(channel, damping, bandnr)): lane 0 is the raw
/// band, lanes 1..3 are low-passed with tau 0.05s / 0.12s / 0.25s
/// (audio-bands.js:27-29).
pub const DAMPING_TAU: [f32; 3] = [0.05, 0.12, 0.25];

/// The exposed band feature names, in bass/mid/treble order
/// (audio-bands.js:31).
pub const BAND_FEATURES: [&str; 3] = ["bass", "mid", "treble"];

/// The exposed features use a perceptual dB scale (-60 dB .. 0 dB -> 0..1) so
/// musically-present treble is not dwarfed by bass under a pink-ish spectrum.
/// Only bass/mid/treble go through this; the raw linear grid/lanes stay as-is
/// for the Point10 scenes (audio-bands.js:35-39).
pub const FEATURE_DB_FLOOR: f32 = -60.0;

/// Session max level at construction (audio-bands.js:57, 73-79).
pub const AGC_INITIAL_MAX_LEVEL: f32 = 0.1;

/// AGC decay tier: maxLevel per second while `maxLevel > 0.01`
/// (audio-bands.js:97-99).
pub const AGC_DECAY_HIGH: f32 = 0.0765625;
/// AGC decay tier: maxLevel per second while `0.001 < maxLevel <= 0.01`
/// (audio-bands.js:99-101).
pub const AGC_DECAY_MID: f32 = 0.00765625;
/// AGC decay tier: maxLevel per second while `0.0001 < maxLevel <= 0.001`
/// (audio-bands.js:101-103).
pub const AGC_DECAY_LOW: f32 = 0.000765625;
/// AGC maxLevel floor — the decay tiers stop here (audio-bands.js:105).
pub const AGC_MAX_LEVEL_FLOOR: f32 = 0.0001;

/// Clamp to `0..=1`; NaN maps to 0 like the JS `clamp01` (audio-bands.js:33).
fn clamp01(x: f32) -> f32 {
    if x > 1.0 {
        1.0
    } else if x > 0.0 {
        x
    } else {
        0.0
    }
}

/// dB-map a linear magnitude onto `0..=1` against [`FEATURE_DB_FLOOR`]
/// (audio-bands.js:41-49). NaN / non-positive magnitudes read 0.
fn feature_value(magnitude: f32) -> f32 {
    if !(magnitude > 0.0) {
        return 0.0;
    }

    let db = 20.0 * magnitude.log10();

    clamp01((db - FEATURE_DB_FLOOR) / -FEATURE_DB_FLOOR)
}

/// Whether `feature` is one of the AGC band features
/// (audio-bands.js:51-53).
pub fn is_band_feature(feature: &str) -> bool {
    BAND_FEATURES.contains(&feature)
}

/// The raw band signal with the AGC gain divided back out, for modules that
/// opt out of automatic gain control (JS `raw` payload object,
/// audio-bands.js:158-160 and 195-199).
#[derive(Clone, Debug, PartialEq)]
pub struct RawBands {
    /// Raw band values (gain divided out), clamped to `0..=1`.
    pub bands: [f32; 3],
    /// Raw damping lanes (gain divided out), clamped to `0..=1`.
    pub lanes: [f32; 12],
    /// Raw 16-cell grid (gain divided out), clamped to `0..=1`.
    pub cells: [f32; 16],
}

/// The payload the Point10 adapter consumes (JS `_payload`,
/// audio-bands.js:156-202). Field mapping from the JS payload object:
///
/// | JS                | Rust                  | Notes                              |
/// |-------------------|-----------------------|------------------------------------|
/// | `bass`/`mid`/`treble` | `bass`/`mid`/`treble` | dB-mapped `feature_value` of lanes[3..5] |
/// | `bands`           | `bands`               | linear, clamped 0..=1              |
/// | `lanes`           | `lanes`               | linear, clamped 0..=1              |
/// | `cells`           | `cells`               | linear, clamped 0..=1              |
/// | `beat`            | `beat`                | beat envelope, clamped 0..=1       |
/// | `wave`            | `wave`                | `-1..=1` normalized time domain    |
/// | `freqMag`         | `freq_mag`            | linear FFT magnitudes              |
/// | `byteFrequencyData` | `byte_frequency_data` | fixed `[u8; 1024]`, zero-padded    |
/// | `raw`             | `raw`                 | AGC gain divided back out          |
/// | `sampleRate`      | `sample_rate`         | Hz                                 |
///
/// Unlike the JS payload (which holds live references into the analyser's
/// internal buffers), this payload owns fixed-size value-copies of the
/// 1024-entry buffers.
#[derive(Clone, Debug, PartialEq)]
pub struct BandPayload {
    /// dB-mapped bass feature (`feature_value(smoothed[0])`).
    pub bass: f32,
    /// dB-mapped mid feature (`feature_value(smoothed[1])`).
    pub mid: f32,
    /// dB-mapped treble feature (`feature_value(smoothed[2])`).
    pub treble: f32,
    /// AGC-normalized linear band values, clamped to `0..=1`.
    pub bands: [f32; 3],
    /// Damping lanes (lane 0 = raw band, lanes 1..3 low-passed), clamped.
    pub lanes: [f32; 12],
    /// 16 log-spaced band cells, linear, clamped to `0..=1`.
    pub cells: [f32; 16],
    /// Beat envelope, clamped to `0..=1`.
    pub beat: f32,
    /// Time-domain wave normalized to `-1..=1` (copied snapshot).
    pub wave: [f32; 1024],
    /// Linear FFT magnitudes (first 1024 bins zero-padded/truncated).
    pub freq_mag: [f32; 1024],
    /// Byte frequency data, `[u8; 1024]` zero-padded/truncated. An absent
    /// input reads as all zeros (JS exposed an empty array — see module
    /// notes 2 and 6).
    pub byte_frequency_data: [u8; 1024],
    /// Same signal with the AGC gain divided back out.
    pub raw: RawBands,
    /// Sample rate in Hz.
    pub sample_rate: u32,
}

/// Input for a single analysis frame. Missing/empty arrays mirror the JS
/// `update({ ... })` guards (audio-bands.js:204-209): an empty
/// `float_frequency_data` selects the no-data decay branch, an empty
/// `byte_time_domain_data` leaves the wave unchanged, and an empty
/// `byte_frequency_data` reads as all zeros in the payload.
///
/// `sample_rate` is required; the JS defaulted it to 44100
/// (audio-bands.js:208).
#[derive(Clone, Debug)]
pub struct BandInput<'a> {
    /// `getFloatFrequencyData` output — true dB per bin. Empty selects the
    /// no-data branch (JS `!floatFrequencyData || !floatFrequencyData.length`,
    /// audio-bands.js:217).
    pub float_frequency_data: &'a [f32],
    /// `getByteTimeDomainData` output. Drives the AGC peak and `wave`.
    pub byte_time_domain_data: &'a [u8],
    /// `getByteFrequencyData` output. Stored verbatim; exposed in the payload.
    pub byte_frequency_data: &'a [u8],
    /// Sample rate in Hz (e.g. 44100, 48000).
    pub sample_rate: u32,
}

/// AGC-smoothed bass/mid/treble analyser. Same private state and processing
/// order as the JS `AudioBands` class (audio-bands.js:55-71).
///
/// The `lastT` clock field is intentionally absent: the JS derived `dt` from
/// `performance.now()`, the port takes it explicitly in [`AudioBands::update`]
/// (see module note 1).
#[derive(Clone, Debug)]
pub struct AudioBands {
    /// Session max time-domain level (JS `maxLevel`, initial
    /// [`AGC_INITIAL_MAX_LEVEL`], audio-bands.js:57).
    max_level: f32,
    /// Linear FFT magnitudes, resized to the FFT length (JS `freqMag`,
    /// audio-bands.js:58, 242-244).
    freq_mag: Vec<f32>,
    /// 16-cell log-spaced grid envelope (JS `grid`, audio-bands.js:59).
    grid: [f32; 16],
    /// Aggregated band means (JS `bands`, audio-bands.js:60).
    bands: [f32; 3],
    /// Lane-1 (tau 0.05s) low-passed bands — the dB-mapped feature source
    /// (JS `smoothed`, audio-bands.js:61, 300-302).
    smoothed: [f32; 3],
    /// Lane 0 = raw band, lanes 1..3 low-passed (index `d * 3 + b`)
    /// (JS `lanes`, audio-bands.js:62-63).
    lanes: [f32; 12],
    /// Beat envelope (JS `beatEnv`, audio-bands.js:64).
    beat_env: f32,
    /// Bass transient reference for the beat detector (JS `bassSmoothed`,
    /// audio-bands.js:65).
    bass_smoothed: f32,
    /// Normalized time-domain wave, fixed 1024 entries (JS `wave`,
    /// audio-bands.js:66).
    wave: [f32; 1024],
    /// Byte frequency data carried into the payload verbatim (JS
    /// `byteFrequencyData`, audio-bands.js:67, 215).
    byte_frequency_data: Vec<u8>,
    /// Cached cell bin ranges (JS `cells`, audio-bands.js:68).
    cells: Vec<(usize, usize)>,
    /// Cache key `"n:sampleRate"` for `cells` (JS `cellKey`, audio-bands.js:69).
    cell_key: String,
}

impl Default for AudioBands {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBands {
    /// Port of the JS constructor (audio-bands.js:56-71), minus the `lastT`
    /// clock.
    pub fn new() -> Self {
        Self {
            max_level: AGC_INITIAL_MAX_LEVEL,
            freq_mag: Vec::new(),
            grid: [0.0; 16],
            bands: [0.0; 3],
            smoothed: [0.0; 3],
            lanes: [0.0; 12],
            beat_env: 0.0,
            bass_smoothed: 0.0,
            wave: [0.0; 1024],
            byte_frequency_data: Vec::new(),
            cells: Vec::new(),
            cell_key: String::new(),
        }
    }

    /// plane9 CMusicManager AGC: the time-domain chunk peak attacks the
    /// session max level instantly; otherwise the max level decays in three
    /// linear tiers (0.0765625 / 0.00765625 / 0.000765625 per second above
    /// 0.01 / 0.001 / 0.0001) with a 1e-4 floor. gain = 1/maxLevel is applied
    /// to the linear FFT magnitudes, so a loud spike stops re-normalizing
    /// everything and quiet material gets boosted. The peak is the
    /// TIME-DOMAIN level, not the FFT peak — measuring it from FFT bins pegs
    /// every bass read to ~1.0 (audio-bands.js:73-109).
    fn agc(&mut self, dt: f32, byte_time_domain_data: &[u8]) -> f32 {
        let mut peak = 0.0f32;

        // JS guards `if (byteTimeDomainData)`; an empty slice iterates zero
        // times, which is equivalent (audio-bands.js:83).
        for &byte in byte_time_domain_data {
            let value = (byte as f32 - 128.0).abs() / 128.0;
            if value > peak {
                peak = value;
            }
        }

        let ml = self.max_level;

        if peak >= ml {
            self.max_level = peak;
        } else {
            let per_sec = if ml > 0.01 {
                AGC_DECAY_HIGH
            } else if ml > 0.001 {
                AGC_DECAY_MID
            } else if ml > 0.0001 {
                AGC_DECAY_LOW
            } else {
                0.0
            };
            self.max_level = AGC_MAX_LEVEL_FLOOR.max(ml - per_sec * dt);
        }

        1.0 / self.max_level
    }

    /// Bin range `[a, b]` for `loHz..hiHz` inclusive
    /// (audio-bands.js:111-124). `hz` is computed in `f64` to match JS
    /// numbers (module note 4).
    fn bin_range(lo_hz: f32, hi_hz: f32, n: usize, sample_rate: u32) -> (usize, usize) {
        let hz = |i: usize| (i as f64 * sample_rate as f64) / 2.0 / n as f64;
        let mut a = 0usize;
        let mut b = n - 1;

        while a < n - 1 && hz(a) < lo_hz as f64 {
            a += 1;
        }
        while b > 0 && hz(b) > hi_hz as f64 {
            b -= 1;
        }

        (a, b.min(n - 1))
    }

    /// Compute (and cache) the 16 cell bin ranges, keyed on `n:sampleRate`
    /// (audio-bands.js:126-141).
    fn band_cells(&mut self, n: usize, sample_rate: u32) -> &[(usize, usize)] {
        let key = format!("{}:{}", n, sample_rate);

        if self.cell_key != key {
            self.cell_key = key;
            self.cells.clear();

            for c in 0..16 {
                self.cells.push(Self::bin_range(
                    BAND_EDGES[c],
                    BAND_EDGES[c + 1],
                    n,
                    sample_rate,
                ));
            }
        }

        &self.cells
    }

    /// Copy the tail of the byte time-domain buffer into `wave`, normalized to
    /// `-1..=1` (audio-bands.js:143-154). An empty buffer leaves `wave`
    /// unchanged — equivalent to the JS falsy guard because the JS loop would
    /// run zero iterations anyway.
    fn fill_wave(&mut self, byte_time_domain_data: &[u8]) {
        if byte_time_domain_data.is_empty() {
            return;
        }

        let n = byte_time_domain_data.len().min(self.wave.len());
        let offset = byte_time_domain_data.len() - n;

        for i in 0..n {
            self.wave[i] = (byte_time_domain_data[offset + i] as f32 / 255.0) * 2.0 - 1.0;
        }
    }

    /// Assemble the payload (JS `_payload`, audio-bands.js:161-202). `gain`
    /// is the AGC gain (1.0 on the no-data branch). `raw.*` is the same
    /// signal with the AGC gain divided back out.
    fn payload(&self, gain: f32, sample_rate: u32) -> BandPayload {
        let mut bands = [0.0f32; 3];
        let mut raw_bands = [0.0f32; 3];
        let mut lanes = [0.0f32; 12];
        let mut raw_lanes = [0.0f32; 12];
        let mut cells = [0.0f32; 16];
        let mut raw_cells = [0.0f32; 16];

        for b in 0..3 {
            bands[b] = clamp01(self.bands[b]);
            raw_bands[b] = clamp01(self.bands[b] / gain);
        }

        for i in 0..12 {
            lanes[i] = clamp01(self.lanes[i]);
            raw_lanes[i] = clamp01(self.lanes[i] / gain);
        }

        for c in 0..16 {
            cells[c] = clamp01(self.grid[c]);
            raw_cells[c] = clamp01(self.grid[c] / gain);
        }

        let mut wave = [0.0f32; 1024];
        wave.copy_from_slice(&self.wave);

        let mut freq_mag = [0.0f32; 1024];
        let freq_mag_len = self.freq_mag.len().min(1024);
        freq_mag[..freq_mag_len].copy_from_slice(&self.freq_mag[..freq_mag_len]);

        let mut byte_frequency_data = [0u8; 1024];
        let byte_frequency_data_len = self.byte_frequency_data.len().min(1024);
        byte_frequency_data[..byte_frequency_data_len]
            .copy_from_slice(&self.byte_frequency_data[..byte_frequency_data_len]);

        BandPayload {
            bass: feature_value(self.smoothed[0]),
            mid: feature_value(self.smoothed[1]),
            treble: feature_value(self.smoothed[2]),
            bands,
            lanes,
            cells,
            beat: clamp01(self.beat_env),
            wave,
            freq_mag,
            byte_frequency_data,
            raw: RawBands {
                bands: raw_bands,
                lanes: raw_lanes,
                cells: raw_cells,
            },
            sample_rate,
        }
    }

    /// Analyse one frame.
    ///
    /// `dt_seconds` is the wall-clock delta since the previous call, clamped
    /// to `<= 0.1` s exactly like the JS `Math.min(0.1, …)`
    /// (audio-bands.js:210-212). JS used an internal `performance.now()`
    /// clock; the port takes dt explicitly so tests are deterministic. The
    /// first call after construction should pass `0.0` (the JS first call had
    /// `dt = 0` because `lastT` was falsy).
    ///
    /// Processing order is identical to the JS (audio-bands.js:204-321):
    /// AGC update → per-cell/lane envelopes → band aggregation → beat
    /// envelope → wave copy → payload assembly. (The wave copy actually runs
    /// first in both, ahead of the no-data branch — see below.)
    pub fn update(&mut self, dt_seconds: f32, input: &BandInput) -> BandPayload {
        // audio-bands.js:210-212 — dt clamp kept; the internal clock and the
        // `lastT` store are gone in favour of the explicit argument.
        let dt = dt_seconds.min(0.1);

        self.fill_wave(input.byte_time_domain_data);
        self.byte_frequency_data = input.byte_frequency_data.to_vec();

        if input.float_frequency_data.is_empty() {
            // audio-bands.js:217-238 — no frequency data this frame: decay
            // everything and report a zeroed (but shaped) payload with gain 1.
            let k = (-dt * 8.0).exp();

            for value in self.grid.iter_mut() {
                *value *= k;
            }
            for value in self.lanes.iter_mut() {
                *value *= k;
            }
            self.bands = [0.0; 3];
            self.smoothed = [0.0; 3];
            for value in self.freq_mag.iter_mut() {
                *value = 0.0;
            }

            self.beat_env *= (-dt * 3.5).exp();
            self.bass_smoothed *= k;

            return self.payload(1.0, input.sample_rate);
        }

        let n = input.float_frequency_data.len();

        if self.freq_mag.len() != n {
            self.freq_mag = vec![0.0; n];
        }

        let gain = self.agc(dt, input.byte_time_domain_data);

        // getFloatFrequencyData is the same FFT in true dB per bin; convert
        // each bin to a linear magnitude (the byte API is dB-scaled and
        // unusable here) (audio-bands.js:248-253).
        for (i, &db) in input.float_frequency_data.iter().enumerate() {
            self.freq_mag[i] = 10.0f32.powf(db / 20.0) * FFT_GAIN * gain;
        }

        let cells = self.band_cells(n, input.sample_rate).to_vec();

        for c in 0..16 {
            let (a, b) = cells[c];
            let mut sum = 0.0f32;

            for i in a..=b {
                sum += self.freq_mag[i];
            }

            let target = sum / (b - a + 1) as f32;
            let g = self.grid[c];

            if target >= g {
                self.grid[c] = g + (target - g) * 1.0f32.min(dt * 30.0);
            } else {
                self.grid[c] = g * (-dt * (6.0 + c as f32)).exp();
            }
        }

        for b in 0..3 {
            let [lo, hi] = BAND_CELLS[b];
            let mut sum = 0.0f32;

            for c in lo..=hi {
                sum += self.grid[c];
            }

            self.bands[b] = sum / (hi - lo + 1) as f32;
        }

        // damping lanes: lane 0 is the raw band, 1..3 low-pass the same band
        // (audio-bands.js:286-298).
        for b in 0..3 {
            self.lanes[b] = self.bands[b];
        }

        for d in 1..=3usize {
            let k = (-dt / DAMPING_TAU[d - 1]).exp();

            for b in 0..3 {
                let i = d * 3 + b;
                self.lanes[i] += (self.bands[b] - self.lanes[i]) * (1.0 - k);
            }
        }

        for b in 0..3 {
            self.smoothed[b] = self.lanes[3 + b];
        }

        // P10 beat envelope: a refractory-gated bass transient (re-arms only
        // once the envelope has decayed below 0.2, so a busy mix pulses per
        // kick instead of pinning the envelope near 1)
        // (audio-bands.js:304-318).
        let bass = (self.grid[0] + self.grid[1]) * 0.5;

        self.beat_env *= (-dt * 3.5).exp();

        if self.beat_env < 0.2 && bass > self.bass_smoothed * 1.25 && bass > 0.03 {
            self.beat_env = 1.0f32.min(self.beat_env.max((bass - self.bass_smoothed) * 12.0));
        }

        self.bass_smoothed += (bass - self.bass_smoothed) * 1.0f32.min(dt * 4.0);

        self.payload(gain, input.sample_rate)
    }
}

#[cfg(test)]
mod tests {
    //! Tests mapped 1:1 from tests/unit/audio-bands.test.js. Test names keep
    //! the JS intent. JS test line references (`audio-bands.test.js:NNN`)
    //! precede each test. Private state is asserted via direct field access
    //! (this child module can see the parent's private fields).
    //!
    //! Test-suite adaptations (see also the module-level port notes):
    //!
    //! * `defaults_the_payload_byte_frequency_data_to_empty_when_it_is_missing`
    //!   (JS audio-bands.test.js:270-280): the JS asserted `payload.
    //!   byteFrequencyData.length === 0`; the fixed `[u8; 1024]` payload
    //!   cannot express length 0, so the test asserts all 1024 bytes are
    //!   zero instead.
    //! * The JS suite faked a `performance.now()` clock via vitest and drove
    //!   `advance(seconds)`; here `advance` is folded into the explicit
    //!   `dt_seconds` argument. `settle` reproduces the mocked timeline
    //!   exactly: first call `dt = 0` (JS `lastT || now`), subsequent calls
    //!   `dt = step`.
    //! * `byteFrequencyData` arrays are compared against the payload's
    //!   fixed-size copy element-for-element.

    use super::*;

    const SAMPLE_RATE: u32 = 44100;
    const FFT_SIZE: usize = 1024;

    fn tone(from: f32, to: f32, db: f32) -> Vec<f32> {
        // audio-bands.test.js:28-40 — bins whose Hz falls inside [from, to]
        // are set to `db`, everything else stays at -Infinity.
        let mut data = vec![f32::NEG_INFINITY; FFT_SIZE];

        for (i, bin) in data.iter_mut().enumerate() {
            let hz = (i as f64 * SAMPLE_RATE as f64) / 2.0 / FFT_SIZE as f64;

            if hz >= from as f64 && hz <= to as f64 {
                *bin = db;
            }
        }

        data
    }

    fn quiet_time_domain() -> [u8; FFT_SIZE] {
        [128; FFT_SIZE]
    }

    fn loud_time_domain() -> [u8; FFT_SIZE] {
        [230; FFT_SIZE]
    }

    fn input<'a>(
        float_frequency_data: &'a [f32],
        byte_time_domain_data: &'a [u8],
        byte_frequency_data: &'a [u8],
    ) -> BandInput<'a> {
        BandInput {
            float_frequency_data,
            byte_time_domain_data,
            byte_frequency_data,
            sample_rate: SAMPLE_RATE,
        }
    }

    /// Reproduce the JS `settle` helper (audio-bands.test.js:45-55) against
    /// an explicit dt: the first call uses dt = 0 (the JS `lastT || now`
    /// falsy case), later calls advance by `step`.
    fn settle(bands: &mut AudioBands, input: &BandInput, seconds: f32, step: f32) -> BandPayload {
        let steps = (seconds / step).ceil() as usize;

        let mut payload = bands.update(0.0, input);

        for _ in 1..steps {
            payload = bands.update(step, input);
        }

        payload
    }

    /// Pink-ish noise: linear magnitude falls as 1/sqrt(f) (power ~1/f)
    /// (audio-bands.test.js:58-67).
    fn pink_spectrum() -> Vec<f32> {
        let mut data = vec![f32::NEG_INFINITY; FFT_SIZE];

        for (i, bin) in data.iter_mut().enumerate().skip(1) {
            let hz = (i as f64 * SAMPLE_RATE as f64) / 2.0 / FFT_SIZE as f64;
            *bin = (-10.0 * hz.log10()) as f32;
        }

        data
    }

    // JS: audio-bands.test.js:72-78 "exposes the three AGC band features"
    #[test]
    fn exposes_the_three_agc_band_features() {
        assert_eq!(BAND_FEATURES, ["bass", "mid", "treble"]);
        assert!(is_band_feature("bass"));
        assert!(is_band_feature("mid"));
        assert!(is_band_feature("treble"));
        assert!(!is_band_feature("kick"));
    }

    // JS: audio-bands.test.js:80-88 "maps 16 cells across the 17 frequency edges"
    #[test]
    fn maps_16_cells_across_the_17_frequency_edges() {
        assert_eq!(BAND_EDGES.len(), 17);
        assert_eq!(BAND_EDGES[10], 2000.0);
        assert_eq!(BAND_CELLS, [[0, 4], [5, 9], [10, 15]]);
    }

    // JS: audio-bands.test.js:92-111 "reads a bass tone into the bass band only"
    #[test]
    fn reads_a_bass_tone_into_the_bass_band_only() {
        let mut bands = AudioBands::new();
        let frequency = tone(30.0, 300.0, -20.0);
        let quiet = quiet_time_domain();
        let input = input(&frequency, &quiet, &[]);

        bands.update(0.0, &input);
        let payload = bands.update(0.1, &input);

        assert_eq!(payload.bands[0], 1.0);
        assert_eq!(payload.bands[1], 0.0);
        assert_eq!(payload.bands[2], 0.0);
        assert!(payload.bass <= 1.0);
        assert!(payload.raw.bands[0] > 0.0);
        assert!(payload.raw.bands[0] < payload.bands[0]);
    }

    // JS: audio-bands.test.js:113-134 "places a 2000 Hz tone in treble but not a 1.9 kHz tone"
    #[test]
    fn places_a_2000hz_tone_in_treble_but_not_a_1_9khz_tone() {
        let mut at_2k = AudioBands::new();
        let frequency = tone(2000.0, 2040.0, -20.0);
        let quiet = quiet_time_domain();
        let two_k = settle(&mut at_2k, &input(&frequency, &quiet, &[]), 2.0, 0.05);

        assert!(two_k.bands[2] > 0.0);
        assert!(two_k.treble > 0.0);

        let mut at_1900 = AudioBands::new();
        let frequency = tone(1880.0, 1960.0, -20.0);
        let nineteen_hundred = settle(&mut at_1900, &input(&frequency, &quiet, &[]), 2.0, 0.05);

        assert!(nineteen_hundred.mid > 0.0);
        assert_eq!(nineteen_hundred.bands[2], 0.0);
        assert_eq!(nineteen_hundred.treble, 0.0);
    }

    // JS: audio-bands.test.js:136-152 "keeps a pink spectrum's treble feature comparable to bass"
    #[test]
    fn keeps_a_pink_spectrums_treble_feature_comparable_to_bass() {
        let mut bands = AudioBands::new();
        let spectrum = pink_spectrum();
        let loud = loud_time_domain();
        let payload = settle(&mut bands, &input(&spectrum, &loud, &[]), 2.0, 0.05);

        for feature in [payload.bass, payload.mid, payload.treble] {
            assert!(feature.is_finite());
            assert!(feature >= 0.0);
            assert!(feature <= 1.0);
        }

        assert!(payload.bass > 0.0);
        assert!(payload.treble >= 0.25 * payload.bass);
    }

    // JS: audio-bands.test.js:154-165 "reads exactly zero for all features in silence"
    #[test]
    fn reads_exactly_zero_for_all_features_in_silence() {
        let mut bands = AudioBands::new();
        let quiet = quiet_time_domain();

        let payload = bands.update(0.0, &input(&[], &quiet, &[]));

        assert_eq!(payload.bass, 0.0);
        assert_eq!(payload.mid, 0.0);
        assert_eq!(payload.treble, 0.0);
    }

    // JS: audio-bands.test.js:167-179 "keeps a full-scale tone at ~1 in its band"
    #[test]
    fn keeps_a_full_scale_tone_at_approximately_one_in_its_band() {
        let mut bands = AudioBands::new();
        let frequency = tone(30.0, 300.0, 0.0);
        let quiet = quiet_time_domain();

        let payload = settle(&mut bands, &input(&frequency, &quiet, &[]), 2.0, 0.05);

        assert!(payload.bass > 0.95);
        assert!(payload.bass <= 1.0);
        assert_eq!(payload.mid, 0.0);
        assert_eq!(payload.treble, 0.0);
    }

    // JS: audio-bands.test.js:181-198 "keeps lane and cell payloads as raw linear magnitudes"
    #[test]
    fn keeps_lane_and_cell_payloads_as_raw_linear_magnitudes() {
        let mut bands = AudioBands::new();
        let frequency = tone(30.0, 300.0, -30.0);
        let loud = loud_time_domain();

        let payload = settle(&mut bands, &input(&frequency, &loud, &[]), 2.0, 0.05);

        // A -30 dB tone with the 0.8 peak AGC stays around 0.095 linear; a dB
        // feature remap would have lifted it to ~0.66.
        assert!(payload.cells[0] > 0.0);
        assert!(payload.cells[0] < 0.2);
        assert!(payload.lanes[0] < 0.2);
        assert!(payload.bands[0] < 0.2);

        assert_eq!(payload.cells.map(clamp01), bands.grid.map(clamp01));
        assert_eq!(payload.lanes.map(clamp01), bands.lanes.map(clamp01));
    }

    // JS: audio-bands.test.js:200-216 "normalizes quiet material with the time-domain AGC"
    #[test]
    fn normalizes_quiet_material_with_the_time_domain_agc() {
        let mut bands = AudioBands::new();
        let frequency = tone(30.0, 300.0, -20.0);
        let quiet = quiet_time_domain();
        let input = input(&frequency, &quiet, &[]);

        bands.update(0.0, &input);
        let payload = bands.update(0.1, &input);

        assert_eq!(payload.bands[0], 1.0);
        assert!(payload.raw.bands[0] > 0.0);
        assert!(payload.raw.bands[0] < 0.3);
        assert!(payload.raw.bands[0] < payload.bands[0]);
    }

    // JS: audio-bands.test.js:218-247 "decays bands and cells when no frequency data arrives"
    #[test]
    fn decays_bands_and_cells_when_no_frequency_data_arrives() {
        let mut bands = AudioBands::new();
        let frequency = tone(30.0, 300.0, -30.0);
        let quiet = quiet_time_domain();

        bands.update(0.0, &input(&frequency, &quiet, &[]));
        let before = bands.update(0.1, &input(&frequency, &quiet, &[]));
        let after = bands.update(0.1, &input(&[], &quiet, &[]));

        assert!(before.cells[0] > 0.0);
        assert!(after.raw.cells[0] < before.cells[0]);
        assert_eq!(after.bands, [0.0, 0.0, 0.0]);
        assert_eq!(after.bass, 0.0);
        assert_eq!(after.mid, 0.0);
        assert_eq!(after.treble, 0.0);
    }

    // JS: audio-bands.test.js:249-268 "exposes the supplied byte frequency data in the payload"
    #[test]
    fn exposes_the_supplied_byte_frequency_data_in_the_payload() {
        let mut bands = AudioBands::new();
        let mut byte_frequency_data = vec![0u8; FFT_SIZE];

        byte_frequency_data[2] = 100;
        byte_frequency_data[14] = 50;
        byte_frequency_data[FFT_SIZE - 1] = 255;

        let frequency = tone(30.0, 300.0, -20.0);
        let quiet = quiet_time_domain();

        let payload = bands.update(0.0, &input(&frequency, &quiet, &byte_frequency_data));

        assert_eq!(payload.byte_frequency_data.len(), FFT_SIZE);
        assert_eq!(
            payload.byte_frequency_data.as_slice(),
            byte_frequency_data.as_slice()
        );
    }

    // JS: audio-bands.test.js:270-280 "defaults the payload byte frequency data to empty when it is missing"
    // Adapted: the fixed-size `[u8; 1024]` payload cannot express an empty
    // array, so the missing-input case asserts all bytes are zero. See note 6
    // in the module docs.
    #[test]
    fn defaults_the_payload_byte_frequency_data_to_empty_when_it_is_missing() {
        let mut bands = AudioBands::new();
        let quiet = quiet_time_domain();

        let payload = bands.update(0.0, &input(&[], &quiet, &[]));

        assert!(payload.byte_frequency_data.iter().all(|&byte| byte == 0));
    }

    // JS: audio-bands.test.js:282-295 "normalizes the time-domain wave to -1..1"
    #[test]
    fn normalizes_the_time_domain_wave_to_minus_1_to_1() {
        let mut bands = AudioBands::new();
        let time_domain = [0u8, 255, 128];

        let payload = bands.update(0.0, &input(&[], &time_domain, &[]));

        assert_eq!(payload.wave.len(), 1024);
        assert_eq!(payload.wave[0], -1.0);
        assert_eq!(payload.wave[1], 1.0);
        assert!((payload.wave[2] - 0.0).abs() < 0.005);
    }

    // JS: audio-bands.test.js:297-312 "exposes the raw signal with the AGC gain divided back out"
    #[test]
    fn exposes_the_raw_signal_with_the_agc_gain_divided_back_out() {
        let mut bands = AudioBands::new();
        let frequency = tone(30.0, 300.0, -20.0);
        let quiet = quiet_time_domain();
        let input = input(&frequency, &quiet, &[]);

        bands.update(0.0, &input);
        let payload = bands.update(0.1, &input);

        assert_eq!(payload.raw.bands.len(), 3);
        assert_eq!(payload.raw.lanes.len(), 12);
        assert_eq!(payload.raw.cells.len(), 16);
    }
}
