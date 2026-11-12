//! Point10 audio bridge and the `p10Audio` reader API.
//!
//! Ports, in one module because they are one contract:
//!
//! * `src/renderer/src/application/point10/audioBridge.js` —
//!   [`AudioBridge`]: applies the per-module responsiveness levers (gain,
//!   smoothing, per-band gain, AGC on/off) to modV's audio payload once per
//!   frame, so every scene reads the same lever-adjusted values.
//! * `src/renderer/src/application/point10/p10Audio.js` — [`P10Audio`]: the
//!   `P9Audio`-compatible read API the vendored scenes call
//!   (`band` / `beat` / `waveform` / `spectrum`).
//! * `src/renderer/src/application/point10/props.js` —
//!   [`RESPONSIVENESS_PROPS`] + [`ResponsivenessLevers`] (`readLevers`).
//! * `src/renderer/src/application/audio-bands.js` — the payload shape
//!   ([`BandPayload`]) and the 16-cell band edges ([`BAND_EDGES`]).
//!
//! # Why the payload type is local
//!
//! `BandPayload` lives in `modv-media` (WS-9 owns the analyser), and
//! `modv-renderers` does not depend on `modv-media`. The engine adapter fills
//! [`BandPayload`] from the analyser and hands it to the renderer, so the
//! renderer crate owns the *consumer-facing* shape and no dependency edge is
//! needed. Both sides read the same field list as `audio-bands.js:update`'s
//! return value.
//!
//! # `P9Audio` (plane9) aliases
//!
//! `p10Audio.js` is a thin reader; the *engine* object the older plane9
//! helpers read (`freqArray`, `freqMagL`, `freqMagR`, `dataArray`, `ctx`) is
//! the bridge itself in the JS (`audioBridge.js:16-39`). That mapping is
//! preserved literally: [`AudioBridge::freq_mag`] is the mono FFT array all
//! three aliases point at, [`AudioBridge::data_array`] is the byte waveform
//! `P9Audio` converts back to `-1..1`, and [`AudioBridge::ctx`] is the sample
//! rate holder. No separate `P9Audio` struct is needed until a scene in a
//! later batch actually calls those legacy fields.

#![forbid(unsafe_code)]

use crate::point10::SceneProp;

/// 16 log-spaced band-cell edges in Hz.
///
/// `audio-bands.js: const BAND_EDGES = [30, 50, 80, ..., 20000]` — 17 edges for
/// 16 cells. Duplicated here because the renderer crate must not depend on the
/// audio crate; the engine asserts both copies agree in its own tests.
pub const BAND_EDGES: [f32; 17] = [
    30.0, 50.0, 80.0, 130.0, 200.0, 300.0, 450.0, 700.0, 1000.0, 1500.0, 2000.0, 3300.0, 5000.0,
    7500.0, 11000.0, 16000.0, 20000.0,
];

/// Which of the three bands a 16-cell grid cell belongs to.
///
/// `audioBridge.js: const BAND_OF_CELL = (cell) => (cell <= 4 ? 0 : cell <= 9 ? 1 : 2)`.
pub fn band_of_cell(cell: usize) -> usize {
    if cell <= 4 {
        0
    } else if cell <= 9 {
        1
    } else {
        2
    }
}

/// Number of waveform / FFT samples the scenes read.
///
/// `p10Audio.js: this.freq = new Float32Array(1024)` and
/// `audioBridge.js: this.wave = new Float32Array(1024)`.
pub const AUDIO_SAMPLES: usize = 1024;

/// Number of damping lanes per band. Lane 0 is the raw band; lanes 1..=3 are
/// low-passed at 0.05 s / 0.12 s / 0.25 s (`audio-bands.js: DAMPING_TAU`).
pub const LANES_PER_BAND: usize = 4;

/// The audio payload the bridge consumes — `audio-bands.js:update`'s return
/// value (the `features.p10` object the JS adapter forwards).
///
/// * `lanes` — 12 values, `[damping * 3 + band]`.
/// * `cells` — the 16 log-spaced grid cells.
/// * `beat` — the per-frame kick envelope.
/// * `wave` — 1024 time-domain samples, `-1..1`.
/// * `freq_mag` — 1024 linear FFT magnitudes.
/// * `raw` — the same `lanes`/`cells` **without** AGC, used when the
///   `useAgc` lever is off (`audioBridge.js:130`).
/// * `byte_frequency_data` — the `Uint8Array` FFT the scenes read as
///   `audioData` (`audioBridge.js:194-216`). Variable length: it is the FFT
///   size, so 1024 for the current analyser but the bridge must not assume it.
#[derive(Clone, Debug)]
pub struct BandPayload {
    /// Analyser sample rate in Hz. `audioBridge.js:118` (`payload.sampleRate || 44100`).
    pub sample_rate: f32,
    /// 12 damping lanes (AGC-normalised, linear).
    pub lanes: [f32; 12],
    /// 16 grid cells (AGC-normalised, linear).
    pub cells: [f32; 16],
    /// Kick envelope for this frame.
    pub beat: f32,
    /// 1024 time-domain samples.
    pub wave: [f32; AUDIO_SAMPLES],
    /// 1024 linear FFT magnitudes.
    pub freq_mag: [f32; AUDIO_SAMPLES],
    /// The AGC-free twin of `lanes`/`cells`.
    pub raw: RawBands,
    /// The byte FFT handed to scenes as `audioData`.
    pub byte_frequency_data: Vec<u8>,
}

/// AGC-free band values (`audio-bands.js` payload `raw` field).
#[derive(Clone, Debug, Default)]
pub struct RawBands {
    /// 12 damping lanes before AGC.
    pub lanes: [f32; 12],
    /// 16 grid cells before AGC.
    pub cells: [f32; 16],
}

impl Default for BandPayload {
    fn default() -> Self {
        Self::silent(44_100.0)
    }
}

impl BandPayload {
    /// A silent payload at `sample_rate` (used before the first analyser frame).
    pub fn silent(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            lanes: [0.0; 12],
            cells: [0.0; 16],
            beat: 0.0,
            wave: [0.0; AUDIO_SAMPLES],
            freq_mag: [0.0; AUDIO_SAMPLES],
            raw: RawBands::default(),
            byte_frequency_data: Vec::new(),
        }
    }

    /// Build a payload from the analyser's parts.
    ///
    /// This is the `from`-style constructor the engine adapter calls: every
    /// argument maps 1:1 onto the `audio-bands.js` payload field of the same
    /// name. `raw` defaults to the *same* values as `lanes`/`cells` when the
    /// analyser does not separate them (silence is the only case where the
    /// distinction is visible, and `useAgc` is a per-module lever).
    pub fn from_parts(
        sample_rate: f32,
        lanes: [f32; 12],
        cells: [f32; 16],
        beat: f32,
        wave: [f32; AUDIO_SAMPLES],
        freq_mag: [f32; AUDIO_SAMPLES],
        byte_frequency_data: Vec<u8>,
    ) -> Self {
        Self {
            sample_rate,
            lanes,
            cells,
            beat,
            wave,
            freq_mag,
            raw: RawBands { lanes, cells },
            byte_frequency_data,
        }
    }

    /// Override the AGC-free twin (the analyser's `raw` field).
    pub fn with_raw(mut self, raw: RawBands) -> Self {
        self.raw = raw;
        self
    }
}

// ---------------------------------------------------------------------------
// Responsiveness levers (`props.js`)
// ---------------------------------------------------------------------------

/// Per-module responsiveness levers.
///
/// `props.js: readLevers(props)` clamps `gain` to `[0,4]`, `smoothing` to
/// `[0,1]`, the three per-band gains to `[0,4]`, and treats anything but an
/// explicit `false` as `useAgc = true`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResponsivenessLevers {
    /// Master gain (`props.gain`, default 1).
    pub gain: f32,
    /// Extra one-pole smoothing on the lanes/grid/beat (`props.smoothing`, 0..1).
    pub smoothing: f32,
    /// Bass gain (`props.bassGain`, default 1).
    pub bass_gain: f32,
    /// Mid gain (`props.midGain`, default 1).
    pub mid_gain: f32,
    /// Treble gain (`props.trebleGain`, default 1).
    pub treble_gain: f32,
    /// Read the AGC payload (`true`) or its raw twin (`false`).
    pub use_agc: bool,
}

impl Default for ResponsivenessLevers {
    fn default() -> Self {
        Self {
            gain: 1.0,
            smoothing: 0.0,
            bass_gain: 1.0,
            mid_gain: 1.0,
            treble_gain: 1.0,
            use_agc: true,
        }
    }
}

impl ResponsivenessLevers {
    /// `props.js: readLevers` — clamps into the declared ranges, and replaces
    /// non-finite values (JS `isFinite`) with the defaults.
    pub fn new(
        gain: f32,
        smoothing: f32,
        bass_gain: f32,
        mid_gain: f32,
        treble_gain: f32,
        use_agc: bool,
    ) -> Self {
        let clamp = |value: f32, fallback: f32, min: f32, max: f32| {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            }
        };
        Self {
            gain: clamp(gain, 1.0, 0.0, 4.0),
            smoothing: clamp(smoothing, 0.0, 0.0, 1.0),
            bass_gain: clamp(bass_gain, 1.0, 0.0, 4.0),
            mid_gain: clamp(mid_gain, 1.0, 0.0, 4.0),
            treble_gain: clamp(treble_gain, 1.0, 0.0, 4.0),
            use_agc,
        }
    }

    /// Resolve the levers from a prop getter (the engine's resolved props).
    ///
    /// `get` receives the prop key (`gain`, `smoothing`, `bassGain`, …) and
    /// returns the resolved numeric value, `None` when the prop is absent. The
    /// boolean `useAgc` is read as `!= false`, exactly like the JS.
    pub fn from_getter(mut get: impl FnMut(&str) -> Option<f32>) -> Self {
        let mut number = |key: &str, default: f32| get(key).unwrap_or(default);
        let gain = number("gain", 1.0);
        let smoothing = number("smoothing", 0.0);
        let bass_gain = number("bassGain", 1.0);
        let mid_gain = number("midGain", 1.0);
        let treble_gain = number("trebleGain", 1.0);
        // JS: `useAgc: props.useAgc !== false` — only an explicit 0/false
        // turns AGC off.
        let use_agc = get("useAgc").map(|value| value != 0.0).unwrap_or(true);
        Self::new(gain, smoothing, bass_gain, mid_gain, treble_gain, use_agc)
    }

    /// The three band gains the bridge multiplies by (`audioBridge.js:124-128`).
    pub fn band_gain(&self) -> [f32; 3] {
        [
            self.gain * self.bass_gain,
            self.gain * self.mid_gain,
            self.gain * self.treble_gain,
        ]
    }
}

/// The responsiveness levers as engine prop schemas.
///
/// `props.js: RESPONSIVENESS_PROPS` — every imported Point10 scene gets these
/// on top of its own schema (`adapter.js:868-871`).
pub const RESPONSIVENESS_PROPS: &[SceneProp] = &[
    SceneProp::float("gain", "Gain", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("smoothing", "Smoothing", 0.0, 0.0, 1.0, 0.01),
    SceneProp::float("bassGain", "Bass Gain", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("midGain", "Mid Gain", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("trebleGain", "Treble Gain", 1.0, 0.0, 4.0, 0.01),
    SceneProp::boolean("useAgc", "Auto Gain (AGC)", true),
];

// ---------------------------------------------------------------------------
// AudioBridge (`audioBridge.js`)
// ---------------------------------------------------------------------------

/// Analyser sample-rate holder exposed to the vendored plane9 helpers as
/// `engine.ctx.sampleRate` (`audioBridge.js:36`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioContext {
    /// Sample rate in Hz.
    pub sample_rate: f32,
}

impl Default for AudioContext {
    fn default() -> Self {
        Self {
            sample_rate: 44_100.0,
        }
    }
}

/// Lever-adjusted band signal, once per frame.
///
/// Every field mirrors `Point10AudioBridge` field for field so a scene can be
/// diffed against the JS: `has_audio` ↔ `hasAudio`, `bands` ↔ `bands`,
/// `lanes` ↔ `lanes`, `grid` ↔ `grid`, `beat_env` ↔ `beatEnv`, `wave` ↔
/// `wave`, `freq_mag` ↔ `freqMag`, `spectrum` ↔ `spectrum`, `data_array` ↔
/// `dataArray`, `ctx` ↔ `ctx`.
///
/// `analyser` in the JS is `hasAudio ? this : null`; the Rust equivalent is
/// [`AudioBridge::is_available`], which is what [`P10Audio::has_audio`] checks.
pub struct AudioBridge {
    /// True once a payload has arrived and stays true until a `None` payload
    /// clears it (`audioBridge.js:112-116`).
    pub has_audio: bool,
    /// The three band values — lane 0 of each band (`audioBridge.js:146-148`).
    pub bands: [f32; 3],
    /// 12 damping lanes (`[damping * 3 + band]`).
    pub lanes: [f32; 12],
    /// 16 grid cells.
    pub grid: [f32; 16],
    /// Kick envelope.
    pub beat_env: f32,
    /// 1024 time-domain samples, lever-clamped to `-1..1`.
    pub wave: [f32; AUDIO_SAMPLES],
    /// Byte form of `wave`, `((v + 1) / 2) * 255` — `dataArray`.
    pub data_array: [u8; AUDIO_SAMPLES],
    /// 1024 linear FFT magnitudes scaled by `min(4, gain)`.
    pub freq_mag: [f32; AUDIO_SAMPLES],
    /// Analyser sample rate in Hz.
    pub sample_rate: f32,
    /// `engine.ctx` alias for the plane9 helpers.
    pub ctx: AudioContext,
    /// Byte FFT, band-gain scaled — the `audioData` the scenes receive.
    pub spectrum: Vec<u8>,
    /// Cache key `n:sampleRate` for the bin→band table (`audioBridge.js:49-51`).
    bin_band_key: Option<(usize, u32)>,
    /// Cached bin→band table.
    bin_band: Vec<u8>,
}

impl Default for AudioBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBridge {
    /// Empty bridge — no audio, defaults from `audioBridge.js:17-39`.
    pub fn new() -> Self {
        Self {
            has_audio: false,
            bands: [0.0; 3],
            lanes: [0.0; 12],
            grid: [0.0; 16],
            beat_env: 0.0,
            wave: [0.0; AUDIO_SAMPLES],
            data_array: [0; AUDIO_SAMPLES],
            freq_mag: [0.0; AUDIO_SAMPLES],
            sample_rate: 44_100.0,
            ctx: AudioContext::default(),
            spectrum: Vec::new(),
            bin_band_key: None,
            bin_band: Vec::new(),
        }
    }

    /// The JS shim's `analyser` getter: `hasAudio ? this : null`.
    pub fn is_available(&self) -> bool {
        self.has_audio
    }

    /// Advance the bridge one frame.
    ///
    /// `audioBridge.js:update(dt, payload, levers)` (lines 111-190), line for
    /// line:
    ///
    /// 1. `payload == None` clears the audio gate and returns without
    ///    touching the signal (the scenes then read their `nomusic` values).
    /// 2. `src` is the AGC payload or its `raw` twin.
    /// 3. Per-band gain folds `gain` and the band gain.
    /// 4. `tau = 0.02 + smoothing * 0.6`, `k = smoothing > 0 ? 1 - exp(-dt/tau) : 1`
    ///    — a one-pole low-pass applied to the lanes, the grid and the beat.
    /// 5. `wave` is scaled by `min(1, gain)` and clamped to `-1..1`; the byte
    ///    twin is rebuilt from it.
    /// 6. `freq_mag` is scaled by `min(4, gain)`.
    /// 7. The byte spectrum is scaled by the band gain of each bin's band.
    pub fn update(
        &mut self,
        dt: f32,
        payload: Option<&BandPayload>,
        levers: &ResponsivenessLevers,
    ) {
        let Some(payload) = payload else {
            // `if (!payload) { this.hasAudio = false; return; }`
            self.has_audio = false;
            return;
        };

        self.has_audio = true;
        self.sample_rate = if payload.sample_rate > 0.0 {
            payload.sample_rate
        } else {
            44_100.0
        };
        self.ctx.sample_rate = self.sample_rate;

        let src_lanes = if levers.use_agc {
            &payload.lanes
        } else {
            &payload.raw.lanes
        };
        let src_cells = if levers.use_agc {
            &payload.cells
        } else {
            &payload.raw.cells
        };
        let band_gain = levers.band_gain();

        // `const tau = 0.02 + levers.smoothing * 0.6;`
        // `const k = levers.smoothing > 0 ? 1 - Math.exp(-dt / tau) : 1;`
        let tau = 0.02 + levers.smoothing * 0.6;
        // Guard `dt` so a NaN/negative frame cannot poison the filter state.
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let k = if levers.smoothing > 0.0 {
            1.0 - (-dt / tau).exp()
        } else {
            1.0
        };

        for damping in 0..LANES_PER_BAND {
            for band in 0..3 {
                let index = damping * 3 + band;
                let target = clamp01(src_lanes[index] * band_gain[band]);
                self.lanes[index] += (target - self.lanes[index]) * k;
            }
        }

        // `for (b...) this.bands[b] = this.lanes[b];`
        for band in 0..3 {
            self.bands[band] = self.lanes[band];
        }

        for cell in 0..16 {
            let target = clamp01(src_cells[cell] * band_gain[band_of_cell(cell)]);
            self.grid[cell] += (target - self.grid[cell]) * k;
        }

        // Note: the beat is scaled by the *bass* band gain only, and it reads
        // the AGC payload even when `useAgc` is off (`audioBridge.js:155`).
        let beat = clamp01(payload.beat * band_gain[0]);
        self.beat_env += (beat - self.beat_env) * k;

        let wave_gain = levers.gain.min(1.0);
        for (index, &sample) in payload.wave.iter().enumerate() {
            let scaled = sample * wave_gain;
            let clamped = scaled.clamp(-1.0, 1.0);
            self.wave[index] = clamped;
            // `Math.round(((this.wave[i] + 1) / 2) * 255)`
            self.data_array[index] = (((clamped + 1.0) / 2.0) * 255.0).round() as u8;
        }

        let mag_gain = levers.gain.min(4.0);
        for (index, &magnitude) in payload.freq_mag.iter().enumerate() {
            self.freq_mag[index] = magnitude * mag_gain;
        }

        self.update_spectrum(payload, band_gain);
    }

    /// Rebuild the band-gain-scaled byte spectrum.
    ///
    /// `audioBridge.js:194-216` — the byte FFT array scaled by the band gain
    /// of each bin's band, clamped to 0..255. Empty input ⇒ empty spectrum,
    /// which is what the adapter turns into a `null` `audioData`.
    fn update_spectrum(&mut self, payload: &BandPayload, band_gain: [f32; 3]) {
        let data = &payload.byte_frequency_data;
        if data.is_empty() {
            self.spectrum = Vec::new();
            return;
        }

        if self.spectrum.len() != data.len() {
            self.spectrum = vec![0; data.len()];
        }

        let sample_rate = if payload.sample_rate > 0.0 {
            payload.sample_rate
        } else {
            self.sample_rate
        };
        self.bin_bands(data.len(), sample_rate);

        for (index, &value) in data.iter().enumerate() {
            let band = self.bin_band[index] as usize;
            let scaled = value as f32 * band_gain[band];
            // `v > 255 ? 255 : v < 0 ? 0 : v`, then Uint8Array truncation.
            self.spectrum[index] = if scaled > 255.0 {
                255
            } else if scaled < 0.0 {
                0
            } else {
                scaled as u8
            };
        }
    }

    /// Bin→band table for `n` bins at `sample_rate`, cached on
    /// `(n, sampleRate)` like `audioBridge.js:_binBands` (lines 49-77).
    ///
    /// `hz = (i * sampleRate) / 2 / n`; cell = the first `c` with
    /// `hz < BAND_EDGES[c + 1]`, defaulting to the last cell, then folded to a
    /// band with [`band_of_cell`].
    fn bin_bands(&mut self, n: usize, sample_rate: f32) {
        let key = (n, sample_rate.to_bits());
        if self.bin_band_key == Some(key) {
            return;
        }
        self.bin_band_key = Some(key);
        self.bin_band = vec![0_u8; n];

        for index in 0..n {
            let hz = (index as f32 * sample_rate) / 2.0 / n as f32;
            let mut cell = 15;
            for candidate in 0..16 {
                if hz < BAND_EDGES[candidate + 1] {
                    cell = candidate;
                    break;
                }
            }
            self.bin_band[index] = band_of_cell(cell) as u8;
        }
    }

    /// Waveform sample `pos` (0..1) — the tail the chromatic scenes scroll.
    pub fn waveform_at(&self, pos: f32) -> f32 {
        let index = ((num01(pos) * (AUDIO_SAMPLES - 1) as f32) as usize).min(AUDIO_SAMPLES - 1);
        self.wave[index]
    }
}

/// `audioBridge.js: const clamp = (x) => (x > 1 ? 1 : x > 0 ? x : 0)`.
///
/// Both comparisons are false for `NaN`, so the JS clamps it to 0; the port
/// keeps that (do not reach for [`f32::clamp`], which propagates `NaN`).
pub fn clamp01(x: f32) -> f32 {
    if x > 1.0 {
        1.0
    } else if x > 0.0 {
        x
    } else {
        0.0
    }
}

/// `p10Audio.js: const num01 = (v) => (typeof v === "number" && isFinite(v) ? v : 0)`
/// — non-finite values read as 0.
fn num01(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

// ---------------------------------------------------------------------------
// P10Audio (`p10Audio.js`)
// ---------------------------------------------------------------------------

/// P9Audio-compatible reader over an [`AudioBridge`].
///
/// The vendored scenes construct `new P9Audio(this.audio)` and call
/// `update(dt)` / `band(channel, damping, bandnr)` / `beat()` / `waveform()` /
/// `spectrum()`. In musicvis those reads walk the audio engine's raw FFT data;
/// in modV the band math already ran (AudioBands + [`AudioBridge`]), so this is
/// a reader rather than a second implementation.
///
/// Unlike the JS (`p10Audio.js:9-27`, which keeps an engine reference), the
/// Rust reader is a snapshot: scenes own a `P10Audio` and hand the bridge to
/// [`P10Audio::update`] each frame. The field set and the decay behaviour when
/// audio is lost are identical.
#[derive(Clone, Debug)]
pub struct P10Audio {
    /// Copies of the bridge's 1024 FFT magnitudes.
    freq: [f32; AUDIO_SAMPLES],
    /// Copies of the bridge's 1024 waveform samples.
    wave: [f32; AUDIO_SAMPLES],
    /// Copies of the bridge's 16 grid cells.
    grid: [f32; 16],
    /// Copies of the bridge's three bands.
    bands: [f32; 3],
    /// Bass band (`this.bass = engine.bands[0]`).
    bass: f32,
    /// Beat envelope.
    beat_env: f32,
    /// `[damping * 3 + band]` — lane 0 is the raw band, 1..3 are low-passed.
    /// In the JS these are the *bridge's* lanes; here they are the snapshot.
    smoo: [f32; 12],
    /// Whether the bridge had audio at the last [`P10Audio::update`].
    has_audio: bool,
}

impl Default for P10Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl P10Audio {
    /// Empty reader (`p10Audio.js:9-27`).
    pub fn new() -> Self {
        Self {
            freq: [0.0; AUDIO_SAMPLES],
            wave: [0.0; AUDIO_SAMPLES],
            grid: [0.0; 16],
            bands: [0.0; 3],
            bass: 0.0,
            beat_env: 0.0,
            smoo: [0.0; 12],
            has_audio: false,
        }
    }

    /// Whether the bridge currently has audio (`get hasAudio()`, line 29-31).
    pub fn has_audio(&self) -> bool {
        self.has_audio
    }

    /// Copy the bridge state, or decay it when audio is gone.
    ///
    /// `p10Audio.js:update(dt)` (lines 33-67):
    ///
    /// * no audio: `k = exp(-dt * 8)`, applied to `_smoo`, `bands`, `beatEnv`
    ///   and `bass`;
    /// * audio: the 12 lanes, 3 bands, 16 cells, 1024 wave samples, 1024 FFT
    ///   magnitudes, `beatEnv` and `bass` are copied straight from the bridge.
    ///
    /// The wave/FFT copy is kept even though it duplicates the bridge, because
    /// scenes may read the reader while writing the bridge in the same frame
    /// (and the JS copies, so past frames are stable).
    pub fn update(&mut self, dt: f32, engine: &AudioBridge) {
        self.has_audio = engine.is_available();
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };

        if !self.has_audio {
            let k = (-dt * 8.0).exp();
            for value in &mut self.smoo {
                *value *= k;
            }
            for value in &mut self.bands {
                *value *= k;
            }
            self.beat_env *= k;
            self.bass *= k;
            return;
        }

        self.smoo.copy_from_slice(&engine.lanes);
        self.bands.copy_from_slice(&engine.bands);
        self.grid.copy_from_slice(&engine.grid);
        self.wave.copy_from_slice(&engine.wave);
        self.freq.copy_from_slice(&engine.freq_mag);
        self.beat_env = engine.beat_env;
        self.bass = engine.bands[0];
    }

    /// Damped band read.
    ///
    /// `p10Audio.js:band(channel, damping, bandnr, nomusic)` (lines 69-82):
    /// `bandnr` picks the band (rounded, clamped to 0..2), `damping` picks the
    /// lane (rounded, clamped to 0..3); lane 0 is the raw band. `channel` is
    /// accepted for signature compatibility and ignored (modV's payload is
    /// mono, exactly as in the JS). Without audio the read is `nomusic`.
    pub fn band(&self, _channel: i32, damping: i32, bandnr: i32, nomusic: f32) -> f32 {
        if !self.has_audio {
            return num01(nomusic);
        }

        let index = (bandnr as f32).round().clamp(0.0, 2.0) as usize;
        let damping = (damping as f32).round().clamp(0.0, 3.0) as usize;

        if damping > 0 {
            self.smoo[damping * 3 + index]
        } else {
            self.bands[index]
        }
    }

    /// Kick envelope, capped at 1 (`p10Audio.js:beat(fallback)`, lines 84-90).
    pub fn beat(&self, fallback: f32) -> f32 {
        if !self.has_audio {
            return num01(fallback);
        }

        self.beat_env.min(1.0)
    }

    /// Waveform sample at `pos` (0..1) (`p10Audio.js:waveform`, lines 92-101).
    pub fn waveform(&self, _channel: i32, pos: f32, nomusic: f32) -> f32 {
        if !self.has_audio {
            return num01(nomusic);
        }

        let index =
            ((num01(pos) * (AUDIO_SAMPLES - 1) as f32).floor() as usize).min(AUDIO_SAMPLES - 1);
        self.wave[index]
    }

    /// Spectrum magnitude at `pos` (0..1), interpolated.
    ///
    /// `p10Audio.js:spectrum` (lines 103-119): plane9 reads the spectrum
    /// linearly — `bin = pos * 399` interpolated on its decimated FFT. modV's
    /// 2048-FFT analyser maps that to `pos * 199.5 + 1`, skipping the inflated
    /// DC bin. The port keeps those exact numbers and clamps the upper sample
    /// to the last bin.
    pub fn spectrum(&self, _channel: i32, pos: f32, nomusic: f32) -> f32 {
        if !self.has_audio {
            return num01(nomusic);
        }

        let p = num01(pos).clamp(0.0, 1.0);
        let j = p * 199.5 + 1.0;
        let a = j.floor() as usize;
        let b = (a + 1).min(AUDIO_SAMPLES - 1);
        let frac = j - a as f32;
        let va = self.freq[a.min(AUDIO_SAMPLES - 1)];
        let vb = self.freq[b];
        va + (vb - va) * frac
    }

    /// The 1024 FFT magnitudes the plane9 helpers read as `freqArray`/`freq`.
    pub fn freq(&self) -> &[f32; AUDIO_SAMPLES] {
        &self.freq
    }

    /// The 1024 waveform samples the chromatic scenes scroll.
    pub fn wave(&self) -> &[f32; AUDIO_SAMPLES] {
        &self.wave
    }

    /// The three bands (lane 0).
    pub fn bands(&self) -> &[f32; 3] {
        &self.bands
    }

    /// The 16 grid cells.
    pub fn grid(&self) -> &[f32; 16] {
        &self.grid
    }

    /// The beat envelope (`this.beatEnv`).
    pub fn beat_env(&self) -> f32 {
        self.beat_env
    }

    /// The bass band (`this.bass`).
    pub fn bass(&self) -> f32 {
        self.bass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_levers() -> ResponsivenessLevers {
        ResponsivenessLevers::default()
    }

    /// `tests/unit/point10-audio-bridge.test.js: makePayload()`.
    fn make_payload() -> BandPayload {
        let mut lanes = [0.0; 12];
        let mut cells = [0.0; 16];
        lanes[0] = 0.5;
        cells[0] = 0.4;

        BandPayload {
            sample_rate: 44_100.0,
            lanes,
            cells,
            beat: 0.5,
            wave: [0.5; AUDIO_SAMPLES],
            freq_mag: [0.5; AUDIO_SAMPLES],
            raw: RawBands {
                lanes: [0.25; 12],
                cells: [0.25; 16],
            },
            byte_frequency_data: Vec::new(),
        }
    }

    #[test]
    fn exposes_no_analyser_until_audio_arrives() {
        let mut bridge = AudioBridge::new();
        assert!(!bridge.is_available());

        bridge.update(0.1, Some(&make_payload()), &default_levers());
        assert!(bridge.is_available());
    }

    #[test]
    fn clears_the_audio_gate_when_the_payload_disappears() {
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&make_payload()), &default_levers());
        bridge.update(0.1, None, &default_levers());
        assert!(!bridge.has_audio);
        assert!(!bridge.is_available());
    }

    #[test]
    fn copies_bands_cells_beat_and_wave_through_unity_levers() {
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&make_payload()), &default_levers());

        assert_eq!(bridge.bands[0], 0.5);
        assert!((bridge.grid[0] - 0.4).abs() < 1e-6);
        assert!((bridge.beat_env - 0.5).abs() < 1e-6);
        assert_eq!(bridge.wave[0], 0.5);
        assert_eq!(bridge.freq_mag[0], 0.5);
        assert_eq!(bridge.sample_rate, 44_100.0);
        // `dataArray[0] = round(((0.5 + 1) / 2) * 255) = 191`
        assert_eq!(bridge.data_array[0], 191);
        assert_eq!(bridge.ctx.sample_rate, 44_100.0);
    }

    #[test]
    fn applies_gain_and_per_band_gain_with_clamping() {
        let mut bridge = AudioBridge::new();
        let levers = ResponsivenessLevers::new(2.0, 0.0, 1.5, 1.0, 1.0, true);
        bridge.update(0.1, Some(&make_payload()), &levers);

        assert_eq!(bridge.lanes[0], 1.0);
        assert_eq!(bridge.grid[0], 1.0);
        assert_eq!(bridge.beat_env, 1.0);
        assert_eq!(bridge.freq_mag[0], 1.0);
    }

    #[test]
    fn uses_the_raw_signal_when_agc_is_disabled() {
        let mut bridge = AudioBridge::new();
        let levers = ResponsivenessLevers {
            use_agc: false,
            ..ResponsivenessLevers::default()
        };
        bridge.update(0.1, Some(&make_payload()), &levers);

        assert!((bridge.lanes[0] - 0.25).abs() < 1e-6);
        assert!((bridge.grid[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn smooths_towards_the_target_when_the_smoothing_lever_is_set() {
        let mut bridge = AudioBridge::new();
        let levers = ResponsivenessLevers::new(1.0, 1.0, 1.0, 1.0, 1.0, true);
        bridge.update(0.1, Some(&make_payload()), &levers);

        assert!(bridge.lanes[0] > 0.0 && bridge.lanes[0] < 0.5);
        assert!(bridge.grid[0] > 0.0 && bridge.grid[0] < 0.4);
    }

    #[test]
    fn builds_a_real_spectrum_and_scales_it_by_band_gain() {
        let mut bridge = AudioBridge::new();
        let mut data = vec![0_u8; 1024];
        data[2] = 100;
        data[14] = 50;
        data[1023] = 200;

        let mut payload = make_payload();
        payload.byte_frequency_data = data.clone();

        bridge.update(0.1, Some(&payload), &default_levers());
        assert_eq!(bridge.spectrum.len(), 1024);
        // Unity levers: the spectrum is the byte FFT unchanged.
        assert_eq!(bridge.spectrum, data);

        // Bin 2 is ~43 Hz (bass cell), bin 14 is ~301 Hz (mid cell).
        let scaled = ResponsivenessLevers::new(1.0, 0.0, 2.0, 3.0, 1.0, true);
        bridge.update(0.1, Some(&payload), &scaled);
        assert_eq!(bridge.spectrum[2], 200);
        assert_eq!(bridge.spectrum[14], 150);
    }

    #[test]
    fn clamps_the_byte_spectrum_to_0_255() {
        let mut bridge = AudioBridge::new();
        let mut payload = make_payload();
        payload.byte_frequency_data = vec![0; 1024];
        payload.byte_frequency_data[2] = 200;

        bridge.update(
            0.1,
            Some(&payload),
            &ResponsivenessLevers::new(4.0, 0.0, 1.0, 1.0, 1.0, true),
        );
        assert_eq!(bridge.spectrum[2], 255);
    }

    #[test]
    fn tracks_sample_rate_changes_on_the_p9audio_ctx_alias() {
        let mut bridge = AudioBridge::new();
        let mut payload = make_payload();
        payload.sample_rate = 48_000.0;
        bridge.update(0.1, Some(&payload), &default_levers());

        assert_eq!(bridge.sample_rate, 48_000.0);
        assert_eq!(bridge.ctx.sample_rate, 48_000.0);
    }

    #[test]
    fn bin_bands_follow_the_band_edges() {
        // 1024 bins at 44100 Hz: bin i covers i * 21.53 Hz.
        let mut bridge = AudioBridge::new();
        bridge.bin_bands(1024, 44_100.0);
        // 0 Hz -> cell 0 (bass), ~1 kHz -> cell 8 (mid), ~10 kHz -> cell 14.
        assert_eq!(bridge.bin_band[0], 0);
        assert_eq!(bridge.bin_band[46], 1); // ~990 Hz
        assert_eq!(bridge.bin_band[464], 2); // ~9.99 kHz
    }

    #[test]
    fn p10audio_reads_the_bridge_and_gates_on_audio() {
        let mut bridge = AudioBridge::new();
        let mut p9 = P10Audio::new();
        p9.update(0.1, &bridge);
        // No audio: `band`/`beat`/`waveform`/`spectrum` return `nomusic`.
        assert_eq!(p9.band(-1, 2, 0, 0.7), 0.7);
        assert_eq!(p9.beat(0.3), 0.3);
        assert_eq!(p9.waveform(-1, 0.5, 0.9), 0.9);
        assert_eq!(p9.spectrum(-1, 0.5, 0.2), 0.2);

        bridge.update(0.1, Some(&make_payload()), &default_levers());
        p9.update(0.1, &bridge);

        assert!(p9.has_audio());
        assert_eq!(p9.freq()[0], 0.5);
        assert!((p9.wave()[0] - 0.5).abs() < 1e-6);
        assert!(p9.bands()[0] > 0.0);
        assert_eq!(p9.band(-1, 0, 0, 0.0), p9.bands()[0]);
        // Damping lane 1 of band 0 was never set (lanes[3] = 0).
        assert_eq!(p9.band(-1, 1, 0, 0.0), 0.0);
        assert_eq!(p9.beat(0.0), 0.5);
    }

    #[test]
    fn p10audio_decays_when_audio_disappears() {
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&make_payload()), &default_levers());
        let mut p9 = P10Audio::new();
        p9.update(0.1, &bridge);
        let before = p9.bands()[0];

        bridge.update(0.1, None, &default_levers());
        p9.update(0.1, &bridge);
        assert!(!p9.has_audio());
        assert!(p9.bands()[0] < before);
        // k = exp(-0.1 * 8) = 0.4493…
        assert!((p9.bands()[0] - before * (-0.8_f32).exp()).abs() < 1e-6);
    }

    #[test]
    fn spectrum_interpolates_across_bins() {
        let mut bridge = AudioBridge::new();
        let mut payload = make_payload();
        payload.freq_mag = [0.0; AUDIO_SAMPLES];
        payload.freq_mag[1] = 1.0;
        payload.freq_mag[2] = 3.0;
        bridge.update(0.1, Some(&payload), &default_levers());

        let mut p9 = P10Audio::new();
        p9.update(0.1, &bridge);

        // pos = 0 -> j = 1 exactly -> bin 1.
        assert_eq!(p9.spectrum(-1, 0.0, 0.0), 1.0);
        // pos = 1/199.5 -> j = 2 exactly -> bin 2.
        assert!((p9.spectrum(-1, 1.0 / 199.5, 0.0) - 3.0).abs() < 1e-4);
        // Halfway between bins 1 and 2.
        assert!((p9.spectrum(-1, 0.5 / 199.5, 0.0) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn levers_from_getter_matches_read_levers() {
        let levers = ResponsivenessLevers::from_getter(|key| match key {
            "gain" => Some(9.0),
            "bassGain" => Some(-1.0),
            "useAgc" => Some(0.0),
            _ => None,
        });

        assert_eq!(levers.gain, 4.0);
        assert_eq!(levers.bass_gain, 0.0);
        assert!(!levers.use_agc);
        assert_eq!(levers.smoothing, 0.0);
        assert_eq!(levers.mid_gain, 1.0);
    }
}
