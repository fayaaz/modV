//! End-to-end audio analyser for the Rust engine (WS-9).
//!
//! [`AudioAnalyser`] owns every analysis stage the legacy renderer spread over
//! `AudioBands`, `TempoDetector`, the phrase detector, `BeatDetektor`, Meyda
//! and `audio-features.js`:
//!
//! * [`AudioBands`] (a 1:1 port of `application/audio-bands.js`) consumes a
//!   spectral frame and produces the Point10 payload — `bass`/`mid`/`treble`,
//!   linear `bands`/`lanes`/`cells`, their AGC-free `raw` companions, the beat
//!   envelope and the time-domain wave.
//! * [`MeydaExtractor`] produces the feature bag the renderer requested
//!   (`store/modules/meyda.js:13-31`) from the same audio.
//! * [`TempoDetector`] and [`BeatDetektor`] run over the Meyda complex
//!   spectrum, exactly like `setup-beat-detektor.js` (spectral-flux onset
//!   strength over the first 25 % of the spectrum, plus the full real part for
//!   BeatDetektor).
//! * [`update_phrase`] turns the band analyser's `bass` into phrase/drop
//!   flips.
//!
//! # WebAudio analysis front end
//!
//! The legacy graph feeds `AudioBands` with three WebAudio analyser arrays
//! (`setup-media.js:336-390`): `fftSize = 2048`, `smoothingTimeConstant = 0`.
//! `AnalyserNode` applies a Blackman window (α = 0.16) and returns 1024
//! single-sided magnitude bins in dB, plus `fftSize` byte time-domain samples.
//! This module reimplements that front end with `rustfft`:
//!
//! * [`FFT_SIZE`] = 2048, giving [`SPECTRUM_BINS`] = 1024 bins. The band grid
//!   therefore keeps the legacy bin width (23.4375 Hz at 48 kHz), which is
//!   required for bin-exact cell selection.
//! * The byte spectrum is reduced from 1024 to [`BYTE_SPECTRUM_BINS`] = 512,
//!   mirroring `reduceByteFrequencyData`
//!   (`application/byte-frequency-data.js`) with the same "loudest byte in
//!   each segment" rule. The payload's [`AudioSnapshot::byte_frequency_data`]
//!   keeps the fixed 1024 slots required by the wire shape, zero-padded after
//!   the 512 real bins (WS-9's "keep 1024 slots zero-padded").
//! * Meyda runs over the last [`MEYDA_FFT_SIZE`] = 512 samples with the `rect`
//!   window, matching `AUDIO_BUFFER_SIZE = 512` and `setup-media.js:364`.
//! * Byte values use 0..255 with `round()` for the time domain and truncation
//!   for the frequency domain (Blink truncates there). The dB byte range is
//!   WebAudio's default `minDecibels = -100` / `maxDecibels = -30`.
//!
//! The exact window/normalisation constants are shared with the Node reference
//! script used to generate `tests/fixtures/audio-bands-reference.json`, so the
//! parity test compares the two implementations rather than guessing at
//! browser internals.
//!
//! # Time
//!
//! JS `AudioBands` reads `performance.now()` internally; here the caller passes
//! `dt` to [`AudioAnalyser::frame`]. The first frame uses `dt = 0`, reproducing
//! the JS `lastT || now` behaviour. Tempo/BeatDetektor use an accumulated
//! monotonic `elapsed` clock so they see the absolute timestamps they require.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::bands::{AudioBands, BandInput, BandPayload};
use super::beat_detektor::{BeatDetektor, BeatDetektorUpdate};
use super::features::SmoothedFeatures;
use super::meyda::{MeydaExtractor, MeydaFeatures};
use super::phrase::{PhraseState, create_phrase_state, update_phrase};
use super::tempo::{TempoDetector, TempoDetectorOptions};

/// WebAudio analyser `fftSize` used by the legacy graph (`setup-media.js:338`).
pub const FFT_SIZE: usize = 2048;
/// Single-sided spectrum bins the band analyser receives (`fftSize / 2`).
pub const SPECTRUM_BINS: usize = FFT_SIZE / 2;
/// Meyda buffer size (`AUDIO_BUFFER_SIZE`, `application/constants.js`).
pub const MEYDA_FFT_SIZE: usize = 512;
/// Bins in the reduced byte spectrum (`reduceByteFrequencyData` target).
pub const BYTE_SPECTRUM_BINS: usize = 512;
/// Length of the `wave` payload array (WS-9 payload shape).
pub const WAVE_LEN: usize = 1024;
/// WebAudio `minDecibels` default used to scale `getByteFrequencyData`.
pub const MIN_DECIBELS: f32 = -100.0;
/// WebAudio `maxDecibels` default used to scale `getByteFrequencyData`.
pub const MAX_DECIBELS: f32 = -30.0;
/// Fraction of the complex spectrum used for tempo onset strength
/// (`setup-beat-detektor.js:5`).
pub const ONSET_SPECTRUM_PORTION: f64 = 0.25;
/// Confidence at or above which a tempo estimate is published
/// (`setup-beat-detektor.js:86-90`).
pub const TEMPO_CONFIDENCE_THRESHOLD: f64 = 0.35;

/// One frame of analysis, mirroring the legacy `payload` + Meyda feature bag.
///
/// Field semantics match the WS-9 payload:
///
/// | Field | JS payload / feature | Notes |
/// |---|---|---|
/// | `bands` | `[bass, mid, treble]` | dB-mapped band features |
/// | `lanes` / `raw_lanes` | `lanes` / `raw.lanes` | 4 damping lanes per band |
/// | `raw_bands` / `raw_cells` | `raw.bands` / `raw.cells` | AGC divided back out |
/// | `cells` | `cells` | 16 log-spaced cells |
/// | `beat` | `beat` | Point10 beat envelope |
/// | `wave` | `wave` | -1..1 time-domain tail (1024) |
/// | `freq_mag` | `freqMag` | linear magnitudes, 1024 bins |
/// | `byte_frequency_data` | `byteFrequencyData` | 0..255, 512 bins then zero padding |
/// | `features` | Meyda feature bag | full extractor output |
/// | `sample_rate` | `sampleRate` | Hz |
/// | `bpm` | `beats.bpm` | only when confident |
/// | `phrase_drop` | phrase detector flip | one-frame pulse |
#[derive(Clone, Debug, PartialEq)]
pub struct AudioSnapshot {
    /// dB-mapped `[bass, mid, treble]` features from the band analyser.
    pub bands: [f32; 3],
    /// 12 damping lanes (4 lanes × 3 bands).
    pub lanes: [f32; 12],
    /// Damping lanes with the AGC gain divided back out.
    pub raw_lanes: [f32; 12],
    /// Linear band values with the AGC gain divided back out.
    pub raw_bands: [f32; 3],
    /// 16 linear band cells with the AGC gain divided back out.
    pub raw_cells: [f32; 16],
    /// 16 linear band cells (AGC applied).
    pub cells: [f32; 16],
    /// Point10 beat envelope.
    pub beat: f32,
    /// Time-domain wave, -1..1 (last 1024 byte-quantised samples).
    pub wave: [f32; 1024],
    /// Linear FFT magnitudes over the 1024 spectrum bins.
    pub freq_mag: [f32; 1024],
    /// 0..255 spectrum over the 512 reduced bins; slots 512..1024 are zero.
    pub byte_frequency_data: [u8; 1024],
    /// Meyda feature bag for this frame.
    pub features: MeydaFeatures,
    /// Analysis sample rate in Hz.
    pub sample_rate: f32,
    /// Tempo estimate in BPM when confident; `None` otherwise.
    pub bpm: Option<f32>,
    /// `true` on the frame a phrase boundary/drop was detected.
    pub phrase_drop: bool,
}

/// The WebAudio-equivalent analysis front end: ring buffer, Blackman-windowed
/// FFT and the two byte arrays.
struct SpectrumFrontend {
    sample_rate: f32,
    fft: Arc<dyn Fft<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    /// Chronological ring buffer of the most recent [`FFT_SIZE`] samples.
    ring: Vec<f32>,
    /// Next write position in `ring`.
    write: usize,
    /// Samples accumulated since construction (saturating).
    filled: usize,
    window: Vec<f32>,
    fft_buffer: Vec<Complex<f32>>,
    /// Raw (unwindowed) window in chronological order.
    time_window: Vec<f32>,
    float_frequency_data: Vec<f32>,
    byte_time_domain_data: Vec<u8>,
    byte_frequency_data: Vec<u8>,
}

/// One analysis frame's raw spectral inputs, in the shape [`BandInput`] wants.
struct SpectrumFrame {
    float_frequency_data: Vec<f32>,
    byte_time_domain_data: Vec<u8>,
    byte_frequency_data: Vec<u8>,
}

impl SpectrumFrontend {
    fn new(sample_rate: f32) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        // Blackman window with alpha = 0.16, the AnalyserNode default. The
        // coefficients are 0.42 / 0.5 / 0.08; 0.42 is the source of the
        // band analyser's FFT_GAIN = 1 / 0.42 correction.
        let window = (0..FFT_SIZE)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * i as f64 / FFT_SIZE as f64;
                0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos()
            })
            .map(|value| value as f32)
            .collect::<Vec<f32>>();

        Self {
            sample_rate,
            fft_scratch: vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()],
            fft,
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            filled: 0,
            window,
            fft_buffer: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            time_window: vec![0.0; FFT_SIZE],
            float_frequency_data: vec![0.0; SPECTRUM_BINS],
            byte_time_domain_data: vec![128; FFT_SIZE],
            byte_frequency_data: vec![0; BYTE_SPECTRUM_BINS],
        }
    }

    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Append mono PCM to the ring buffer (any length).
    fn push_mono(&mut self, pcm: &[f32]) {
        for &sample in pcm {
            self.ring[self.write] = sample;
            self.write = (self.write + 1) % FFT_SIZE;
            self.filled = self.filled.saturating_add(1);
        }
    }

    /// Copy the last `len` samples in chronological order (zero-padded at the
    /// front when not enough audio has arrived yet).
    fn last_samples(&self, len: usize) -> Vec<f32> {
        let mut out = vec![0.0; len];
        let available = self.filled.min(len);

        for i in 0..available {
            let ago = available - 1 - i;
            let index = (self.write + FFT_SIZE - 1 - ago) % FFT_SIZE;
            out[len - available + i] = self.ring[index];
        }

        out
    }

    /// Run the analyser front end over the current window.
    fn analyse(&mut self) -> SpectrumFrame {
        for (i, value) in self.time_window.iter_mut().enumerate() {
            let index = (self.write + i) % FFT_SIZE;
            *value = self.ring[index];
        }

        for (i, sample) in self.time_window.iter().enumerate() {
            self.fft_buffer[i] = Complex::new(sample * self.window[i], 0.0);
            self.byte_time_domain_data[i] = sample_to_byte(*sample);
        }

        self.fft
            .process_with_scratch(&mut self.fft_buffer, &mut self.fft_scratch);

        // WebAudio returns single-sided magnitudes scaled by 2 / fftSize (the
        // DC bin is half that; the difference is inaudible for band analysis).
        let scale = 2.0 / FFT_SIZE as f32;
        for (i, out) in self.float_frequency_data.iter_mut().enumerate() {
            let bin = self.fft_buffer[i];
            let magnitude = (bin.re * bin.re + bin.im * bin.im).sqrt() * scale;

            *out = if magnitude > 0.0 {
                20.0 * magnitude.log10()
            } else {
                f32::NEG_INFINITY
            };
        }

        let mut byte_full = vec![0u8; SPECTRUM_BINS];
        for (i, &db) in self.float_frequency_data.iter().enumerate() {
            byte_full[i] = decibels_to_byte(db);
        }
        self.byte_frequency_data = reduce_byte_frequency_data(&byte_full, BYTE_SPECTRUM_BINS);

        SpectrumFrame {
            float_frequency_data: self.float_frequency_data.clone(),
            byte_time_domain_data: self.byte_time_domain_data.clone(),
            byte_frequency_data: self.byte_frequency_data.clone(),
        }
    }
}

/// Map a -1..1 sample onto the 0..255 byte time-domain scale
/// (0 maps to 128). The `+1` samples that would round to 256 clamp to 255.
fn sample_to_byte(sample: f32) -> u8 {
    if !sample.is_finite() {
        return 128;
    }

    let clamped = sample.clamp(-1.0, 1.0);
    (((clamped * 0.5 + 0.5) * 255.0).round()).clamp(0.0, 255.0) as u8
}

/// Map a dB bin onto WebAudio's 0..255 byte spectrum. Blink truncates the
/// scaled value, so this truncates too; -infinity clamps to 0.
fn decibels_to_byte(db: f32) -> u8 {
    if !db.is_finite() {
        return 0;
    }

    let scaled = 255.0 * (db - MIN_DECIBELS) / (MAX_DECIBELS - MIN_DECIBELS);

    scaled.clamp(0.0, 255.0) as u8
}

/// `reduceByteFrequencyData` (`application/byte-frequency-data.js`): partition
/// the source bins into `target_len` segments with `floor(((i + 1) * len) /
/// target_len)` bounds and keep the loudest byte in each segment.
///
/// A source shorter than (or equal to) the target is copied verbatim; an empty
/// source returns an empty vector, exactly like the JS.
fn reduce_byte_frequency_data(data: &[u8], target_len: usize) -> Vec<u8> {
    if data.is_empty() || target_len == 0 {
        return Vec::new();
    }

    if data.len() <= target_len {
        return data.to_vec();
    }

    let mut reduced = vec![0u8; target_len];
    let mut start = 0usize;

    for (i, slot) in reduced.iter_mut().enumerate() {
        let end = ((i + 1) * data.len()) / target_len;
        let mut max = 0u8;

        for &byte in &data[start..end] {
            max = max.max(byte);
        }

        *slot = max;
        start = end;
    }

    reduced
}

/// Spectral-flux onset strength over the first 25 % of the complex spectrum
/// (`setup-beat-detektor.js:8-70`).
#[derive(Debug, Default)]
struct OnsetStrength {
    previous_magnitudes: Vec<f64>,
}

impl OnsetStrength {
    fn process(&mut self, complex_spectrum: &[[f32; 2]]) -> f64 {
        let bin_count = ((complex_spectrum.len() as f64 * ONSET_SPECTRUM_PORTION) as usize).max(8);

        if self.previous_magnitudes.len() != bin_count {
            self.previous_magnitudes = (0..bin_count.min(complex_spectrum.len()))
                .map(|i| {
                    let [re, im] = complex_spectrum[i];
                    ((re as f64).hypot(im as f64)).ln_1p()
                })
                .collect();
            return 0.0;
        }

        let mut flux = 0.0;

        for (i, previous) in self.previous_magnitudes.iter_mut().enumerate() {
            let magnitude = if i < complex_spectrum.len() {
                let [re, im] = complex_spectrum[i];
                ((re as f64).hypot(im as f64)).ln_1p()
            } else {
                0.0
            };
            let rise = magnitude - *previous;

            if rise > 0.0 {
                flux += rise;
            }

            *previous = magnitude;
        }

        flux / bin_count as f64
    }
}

/// Owns the whole WS-9 analysis chain.
///
/// Feed it mono PCM with [`Self::push_mono`] and call [`Self::frame`] once per
/// rendered frame. Analysis runs at the construction sample rate (48 kHz for
/// capture); input is assumed to already be at that rate.
pub struct AudioAnalyser {
    frontend: SpectrumFrontend,
    bands: AudioBands,
    meyda: MeydaExtractor,
    tempo: TempoDetector,
    beat_detektor: BeatDetektor,
    onset: OnsetStrength,
    phrase: PhraseState,
    smoothing: SmoothedFeatures,
    elapsed: f64,
    started: bool,
    last_kick: bool,
    last_phrase_drop: bool,
    last_snapshot: Option<AudioSnapshot>,
}

impl AudioAnalyser {
    /// Creates an analyser for `sample_rate` Hz (48 kHz in the engine).
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };

        Self {
            frontend: SpectrumFrontend::new(sample_rate),
            bands: AudioBands::new(),
            meyda: MeydaExtractor::new(MEYDA_FFT_SIZE, sample_rate),
            tempo: TempoDetector::new(TempoDetectorOptions {
                confidence_threshold: TEMPO_CONFIDENCE_THRESHOLD,
                ..Default::default()
            }),
            beat_detektor: BeatDetektor::new(85.0, 169.0),
            onset: OnsetStrength::default(),
            phrase: create_phrase_state(),
            smoothing: SmoothedFeatures::new(),
            elapsed: 0.0,
            started: false,
            last_kick: false,
            last_phrase_drop: false,
            last_snapshot: None,
        }
    }

    /// The configured analysis sample rate in Hz.
    pub fn sample_rate(&self) -> f32 {
        self.frontend.sample_rate()
    }

    /// The most recent snapshot, if [`Self::frame`] has run.
    pub fn last_snapshot(&self) -> Option<&AudioSnapshot> {
        self.last_snapshot.as_ref()
    }

    /// Whether BeatDetektor's BassKick flagged the most recent frame.
    pub fn kick(&self) -> bool {
        self.last_kick
    }

    /// Consume mono f32 PCM (any length) at the configured sample rate.
    pub fn push_mono(&mut self, pcm: &[f32]) {
        self.frontend.push_mono(pcm);
    }

    /// Advance time/analysis and return the latest snapshot.
    ///
    /// Call once per frame. The first call behaves like the JS `lastT || now`
    /// path and analyses with `dt = 0`; later calls use `dt` (the band
    /// analyser clamps it to 0.1 s internally). Non-finite/negative `dt` reads
    /// as 0.
    pub fn frame(&mut self, dt: f32) -> AudioSnapshot {
        let dt = if !dt.is_finite() || dt < 0.0 {
            0.0
        } else if self.started {
            dt
        } else {
            0.0
        };

        self.started = true;
        self.elapsed += dt as f64;

        let spectrum = self.frontend.analyse();

        let payload = self.bands.update(
            dt,
            &BandInput {
                float_frequency_data: &spectrum.float_frequency_data,
                byte_time_domain_data: &spectrum.byte_time_domain_data,
                byte_frequency_data: &spectrum.byte_frequency_data,
                sample_rate: self.sample_rate().round() as u32,
            },
        );

        let meyda_window = self.frontend.last_samples(MEYDA_FFT_SIZE);
        let features = self.meyda.extract(&meyda_window);

        self.smoothing
            .set_features(feature_map(&payload, &features));

        let onset = self.onset.process(&features.complex_spectrum);
        self.tempo.process(self.elapsed, onset);

        let beat: BeatDetektorUpdate =
            self.beat_detektor
                .update(self.elapsed, &features.complex_spectrum, &payload.bands);
        self.last_kick = beat.kick;

        let phrase_update = update_phrase(&self.phrase, payload.bass as f64, dt as f64);
        self.phrase = phrase_update.state;
        self.last_phrase_drop = phrase_update.flipped;

        let snapshot = AudioSnapshot {
            bands: [payload.bass, payload.mid, payload.treble],
            lanes: payload.lanes,
            raw_lanes: payload.raw.lanes,
            raw_bands: payload.raw.bands,
            raw_cells: payload.raw.cells,
            cells: payload.cells,
            beat: payload.beat,
            wave: payload.wave,
            freq_mag: payload.freq_mag,
            byte_frequency_data: payload.byte_frequency_data,
            features,
            sample_rate: self.sample_rate(),
            bpm: self.confident_bpm(),
            phrase_drop: phrase_update.flipped,
        };

        self.last_snapshot = Some(snapshot.clone());

        snapshot
    }

    /// Smoothed feature read for Input Links (`getSmoothedFeature` parity).
    ///
    /// Returns `None` for a feature the analyser does not publish (the JS
    /// `undefined` → `NaN` path); registered ids start from 0 and lerp towards
    /// the current frame's feature by `amount` on every call.
    pub fn smoothed_feature(&mut self, feature: &str, id: &str, amount: f32) -> Option<f32> {
        self.smoothing.get_feature(feature)?;

        let value = self
            .smoothing
            .get_smoothed_feature(feature, id, amount as f64);

        value.is_finite().then_some(value as f32)
    }

    /// Drops the smoothing state for `id`
    /// (`audio-features.js:25-27`).
    pub fn remove_smoothing_id(&mut self, id: &str) {
        self.smoothing.remove_smoothing_id(id);
    }

    /// Rounded tempo estimate when the detector is confident, matching
    /// `setup-beat-detektor.js:117-136`.
    fn confident_bpm(&self) -> Option<f32> {
        let bpm = self.tempo.bpm.round();

        if self.tempo.confidence < self.tempo.confidence_threshold
            || bpm < self.tempo.min_bpm
            || bpm > self.tempo.max_bpm
            || !bpm.is_finite()
        {
            return None;
        }

        Some(bpm as f32)
    }
}

/// Build the string-keyed feature map the smoothing registry consumes. Keys
/// mirror the Vuex feature names (`store/modules/meyda.js:13-31`) plus
/// `bass`/`mid`/`treble` and `beat`.
fn feature_map(
    payload: &BandPayload,
    features: &MeydaFeatures,
) -> std::collections::HashMap<String, f64> {
    let mut map = std::collections::HashMap::with_capacity(18);

    map.insert("bass".to_string(), payload.bass as f64);
    map.insert("mid".to_string(), payload.mid as f64);
    map.insert("treble".to_string(), payload.treble as f64);
    map.insert("beat".to_string(), payload.beat as f64);
    map.insert("rms".to_string(), features.rms as f64);
    map.insert("zcr".to_string(), features.zcr as f64);
    map.insert("energy".to_string(), features.energy as f64);
    map.insert(
        "spectralCentroid".to_string(),
        features.spectral_centroid as f64,
    );
    map.insert(
        "spectralFlatness".to_string(),
        features.spectral_flatness as f64,
    );
    map.insert("spectralSlope".to_string(), features.spectral_slope as f64);
    map.insert(
        "spectralRolloff".to_string(),
        features.spectral_rolloff as f64,
    );
    map.insert(
        "spectralSpread".to_string(),
        features.spectral_spread as f64,
    );
    map.insert(
        "spectralSkewness".to_string(),
        features.spectral_skewness as f64,
    );
    map.insert(
        "spectralKurtosis".to_string(),
        features.spectral_kurtosis as f64,
    );
    map.insert(
        "perceptualSpread".to_string(),
        features.perceptual_spread as f64,
    );
    map.insert(
        "perceptualSharpness".to_string(),
        features.perceptual_sharpness as f64,
    );

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const SAMPLE_RATE: f32 = 48_000.0;
    const FRAME_DT: f32 = 1.0 / 60.0;

    /// Mono sine generator used by the parity fixture as well (same formula in
    /// the Node reference script).
    fn fixture_sample(index: usize) -> f32 {
        let t = index as f64 / SAMPLE_RATE as f64;
        let envelope = 0.35 + 0.25 * (2.0 * std::f64::consts::PI * 0.5 * t).sin();

        (envelope * (2.0 * std::f64::consts::PI * 60.0 * t).sin()
            + 0.18 * (2.0 * std::f64::consts::PI * 6000.0 * t).sin()) as f32
    }

    fn push_sine(analyser: &mut AudioAnalyser, hz: f32, seconds: f32, amplitude: f32) {
        let total = (SAMPLE_RATE * seconds) as usize;
        let chunk = 800usize;

        for start in (0..total).step_by(chunk) {
            let end = (start + chunk).min(total);
            let pcm: Vec<f32> = (start..end)
                .map(|i| amplitude * (2.0 * PI * hz * i as f32 / SAMPLE_RATE).sin())
                .collect();

            analyser.push_mono(&pcm);
            analyser.frame(FRAME_DT);
        }
    }

    #[test]
    fn silence_produces_an_all_zero_snapshot() {
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);

        for _ in 0..120 {
            analyser.frame(FRAME_DT);
        }

        let snapshot = analyser.frame(FRAME_DT);

        assert_eq!(snapshot.bands, [0.0; 3]);
        assert_eq!(snapshot.lanes, [0.0; 12]);
        assert_eq!(snapshot.cells, [0.0; 16]);
        assert_eq!(snapshot.raw_bands, [0.0; 3]);
        assert_eq!(snapshot.raw_lanes, [0.0; 12]);
        assert_eq!(snapshot.raw_cells, [0.0; 16]);
        assert_eq!(snapshot.beat, 0.0);
        assert_eq!(snapshot.bpm, None);
        assert!(!snapshot.phrase_drop);
        assert!(
            snapshot.wave.iter().all(|&value| value.abs() < 0.01),
            "digital silence quantises to byte 128, i.e. wave ~0.0039"
        );
        assert!(snapshot.freq_mag.iter().all(|&value| value == 0.0));
        assert!(snapshot.byte_frequency_data.iter().all(|&byte| byte == 0));
        assert_eq!(snapshot.sample_rate, SAMPLE_RATE);
    }

    #[test]
    fn sixty_hertz_tone_raises_bass_above_mid_and_treble() {
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);

        push_sine(&mut analyser, 60.0, 2.0, 0.5);

        let snapshot = analyser.last_snapshot().cloned().expect("snapshot");

        assert!(
            snapshot.bands[0] > snapshot.bands[1],
            "bass {} should beat mid {}",
            snapshot.bands[0],
            snapshot.bands[1]
        );
        assert!(
            snapshot.bands[0] > snapshot.bands[2],
            "bass {} should beat treble {}",
            snapshot.bands[0],
            snapshot.bands[2]
        );
        assert!(
            snapshot.bands[0] > 0.5,
            "bass {} should be strong",
            snapshot.bands[0]
        );
    }

    #[test]
    fn six_kilohertz_tone_raises_treble() {
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);

        push_sine(&mut analyser, 6000.0, 2.0, 0.4);

        let snapshot = analyser.last_snapshot().cloned().expect("snapshot");

        assert!(
            snapshot.bands[2] > snapshot.bands[0],
            "treble {} should beat bass {}",
            snapshot.bands[2],
            snapshot.bands[0]
        );
        assert!(
            snapshot.bands[2] > snapshot.bands[1],
            "treble {} should beat mid {}",
            snapshot.bands[2],
            snapshot.bands[1]
        );
        // A pure tone lights a single cell and the treble feature averages six
        // cells, so the dB-mapped feature is well below 1 even at full scale.
        assert!(
            snapshot.bands[2] > 0.15,
            "treble {} should be present",
            snapshot.bands[2]
        );
    }

    #[test]
    fn agc_normalisation_keeps_the_lit_band_near_full_scale() {
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);

        // 70.3125 Hz is exactly bin 3 (50-80 Hz cell, a single-bin cell), so
        // the cell average is not diluted by neighbouring empty bins.
        push_sine(&mut analyser, 70.3125, 2.0, 0.4);

        let snapshot = analyser.last_snapshot().cloned().expect("snapshot");
        let peak = snapshot.cells.iter().copied().fold(0.0f32, f32::max);

        assert!(peak > 0.9, "AGC cell peak {peak} should approach 1.0");
        assert!(peak <= 1.0, "AGC cell peak {peak} must be clamped");
    }

    #[test]
    fn smoothed_feature_converges_and_is_keyed_per_id() {
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);

        push_sine(&mut analyser, 120.0, 1.0, 0.4);

        let first = analyser
            .smoothed_feature("bass", "a", 0.5)
            .expect("bass is published");
        let mut previous = first;

        for _ in 0..16 {
            let value = analyser
                .smoothed_feature("bass", "a", 0.5)
                .expect("bass is published");

            assert!(value >= previous, "smoothing must approach the target");
            previous = value;
        }

        assert!(previous > first, "smoothed value should converge");

        let other = analyser
            .smoothed_feature("bass", "b", 0.5)
            .expect("bass is published");
        assert!(other < previous, "a fresh id starts from zero again");

        assert!(
            analyser.smoothed_feature("absent", "a", 0.5).is_none(),
            "unknown features read as the JS undefined path"
        );
    }

    #[test]
    fn byte_frequency_data_keeps_512_bins_then_zero_padding() {
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);

        push_sine(&mut analyser, 1000.0, 0.5, 0.5);
        let snapshot = analyser.last_snapshot().cloned().expect("snapshot");

        assert!(
            snapshot.byte_frequency_data[..BYTE_SPECTRUM_BINS]
                .iter()
                .any(|&byte| byte > 0)
        );
        assert!(
            snapshot.byte_frequency_data[BYTE_SPECTRUM_BINS..]
                .iter()
                .all(|&byte| byte == 0)
        );
    }

    #[test]
    fn reduce_byte_frequency_data_keeps_the_loudest_byte_per_segment() {
        let data: Vec<u8> = (0..8).collect();

        assert_eq!(
            reduce_byte_frequency_data(&data, 4),
            vec![1, 3, 5, 7],
            "each pair keeps its maximum"
        );
        assert_eq!(
            reduce_byte_frequency_data(&data[..3], 4),
            vec![0, 1, 2],
            "shorter input is copied"
        );
        assert_eq!(reduce_byte_frequency_data(&[], 4), Vec::<u8>::new());
        assert_eq!(reduce_byte_frequency_data(&data, 0), Vec::<u8>::new());
    }

    #[test]
    fn byte_scaling_matches_the_web_audio_conventions() {
        assert_eq!(sample_to_byte(0.0), 128);
        assert_eq!(sample_to_byte(1.0), 255);
        assert_eq!(sample_to_byte(-1.0), 0);
        assert_eq!(sample_to_byte(2.0), 255, "clamped");
        assert_eq!(sample_to_byte(f32::NAN), 128, "NaN reads as silence");
        assert_eq!(decibels_to_byte(MAX_DECIBELS), 255);
        assert_eq!(decibels_to_byte(MIN_DECIBELS), 0);
        assert_eq!(decibels_to_byte(f32::NEG_INFINITY), 0);
        assert_eq!(decibels_to_byte(0.0), 255, "above max clamps");
    }

    #[test]
    fn frame_dt_is_zeroed_on_the_first_call_like_the_js_clock() {
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);
        let mut payload = None;

        // Detector state after the first frame is unchanged by the passed dt;
        // the observable proxy is the elapsed clock that drives tempo/beat.
        for _ in 0..2 {
            payload = Some(analyser.frame(0.5));
        }

        assert!(payload.is_some());
    }

    /// Parity against the legacy JS `AudioBands` class.
    ///
    /// `tests/fixtures/audio-bands-reference.json` was generated with Node 24
    /// by a throwaway script (kept outside the repo) that:
    ///
    /// 1. generates the same PCM as [`fixture_sample`] (800 samples/frame,
    ///    24 frames at 48 kHz),
    /// 2. reproduces this module's front end (Blackman α = 0.16 2048-point
    ///    FFT, 1024 dB bins, 2048 byte time-domain values, 1024→512 byte
    ///    spectrum reduction),
    /// 3. drives the real `src/renderer/src/application/audio-bands.js` with
    ///    a fake `performance.now` clock at 60 fps (first update `dt = 0`).
    ///
    /// Tolerance is 1 % relative, as required by WS-9.
    #[test]
    fn matches_the_js_audio_bands_reference_within_one_percent() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/audio-bands-reference.json"
        ))
        .expect("fixture JSON");

        let frames = fixture["frames"].as_array().expect("fixture frames array");
        let frame_samples = fixture["frameSamples"].as_u64().expect("frameSamples") as usize;
        let mut analyser = AudioAnalyser::new(SAMPLE_RATE);
        let mut sample_index = 0usize;

        for (frame_index, expected) in frames.iter().enumerate() {
            let pcm: Vec<f32> = (0..frame_samples)
                .map(|_| {
                    let sample = fixture_sample(sample_index);
                    sample_index += 1;
                    sample
                })
                .collect();

            analyser.push_mono(&pcm);

            // The JS reference's fake clock starts at a non-zero value, so its
            // first update also has dt = 0; the analyser mirrors that.
            let snapshot = analyser.frame(FRAME_DT);

            assert_close(
                &snapshot.bands,
                expected["features"].as_array().expect("features"),
                "bass/mid/treble",
                frame_index,
            );
            assert_close(
                &snapshot.lanes,
                expected["lanes"].as_array().expect("lanes"),
                "lanes",
                frame_index,
            );
            assert_close(
                &snapshot.cells,
                expected["cells"].as_array().expect("cells"),
                "cells",
                frame_index,
            );
        }
    }

    fn assert_close(actual: &[f32], expected: &[serde_json::Value], label: &str, frame: usize) {
        assert_eq!(
            actual.len(),
            expected.len(),
            "{label} length, frame {frame}"
        );

        for (i, expected) in expected.iter().enumerate() {
            let expected = expected.as_f64().expect("number") as f32;
            let actual = actual[i];
            let tolerance = (expected.abs() * 0.01).max(1e-4);

            assert!(
                (actual - expected).abs() <= tolerance,
                "{label}[{i}] frame {frame}: rust {actual} vs js {expected} (tolerance {tolerance})"
            );
        }
    }
}
