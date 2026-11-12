//! Meyda feature extractors.
//!
//! Ports the feature set the renderer asks Meyda for
//! (`src/renderer/src/application/worker/store/modules/meyda.js:13-31`) plus the
//! FFT preparation Meyda performs before handing a buffer to those extractors.
//! Source of truth is Meyda 5.6.3 as vendored in the toolchain image, primarily
//! `node_modules/meyda/src/main.ts` (`extract`, `prepareSignalWithSpectrum`),
//! `node_modules/meyda/src/utilities.ts` and
//! `node_modules/meyda/src/extractors/*.ts`.
//!
//! # Signal preparation (matches Meyda exactly)
//!
//! * modV constructs its analyser with `bufferSize: AUDIO_BUFFER_SIZE` (512) and
//!   `windowingFunction: "rect"` (`src/renderer/src/application/setup-media.js:360-368`),
//!   so the default here is [`WindowFunction::Rect`] — the raw samples. Meyda's
//!   own default is `hanning`; [`MeydaExtractor::with_window`] selects it (and
//!   the other Meyda windows) when a caller wants the library default.
//! * The window is applied *before* the FFT (`prepareSignalWithSpectrum`,
//!   main.ts:411-414), and every time-domain feature (`rms`, `energy`, `zcr`) is
//!   computed on that windowed signal (`MeydaFeatures::buffer` is the same
//!   windowed samples — Meyda's `buffer` extractor is `args.signal`,
//!   `featureExtractors.ts:27-29`).
//! * Meyda's `fftjs` real transform keeps all `bufferSize` bins; the magnitude
//!   spectrum (`ampSpectrum`) is only the first `bufferSize / 2` bins
//!   (main.ts:415-422). This port keeps the same split: [`MeydaFeatures::complex_spectrum`]
//!   has `fft_size` bins (mirrored conjugate half, exactly what a full complex
//!   FFT of real input returns) and the extractors use the first `fft_size / 2`.
//! * Inputs shorter than `fft_size` are zero-padded and longer inputs are
//!   truncated, so a caller can always pass the analyser's current chunk.
//!
//! # Deliberate deviations from Meyda (all documented per feature)
//!
//! 1. **Frequency units.** Meyda's `spectralCentroid` is `mu(1, ampSpectrum)`
//!    (`extractors/spectralCentroid.ts`), i.e. a *bin index*, not a frequency —
//!    the same `mu` helper is used by spread/skewness/kurtosis. The plan
//!    (docs/rust-hybrid.md, WS-9 table) asks for the magnitude-weighted mean
//!    *frequency*, so [`MeydaFeatures::spectral_centroid`] and
//!    [`MeydaFeatures::spectral_spread`] are in Hz (`bin_index * sample_rate / fft_size`).
//!    The bin-domain values are exposed alongside as
//!    [`MeydaFeatures::spectral_centroid_bin`] / [`MeydaFeatures::spectral_spread_bin`]
//!    for exact Meyda parity. Skewness and kurtosis are scale invariant, so they
//!    are identical in either unit.
//! 2. **Rolloff threshold.** WS-9's table says "85 % of energy", but Meyda's
//!    `spectralRolloff` uses `0.99 * total` (`extractors/spectralRolloff.ts:16-22`).
//!    The implementation follows the code (99 %), because the UI expects Meyda's
//!    numbers. See `SPECTRAL_ROLLOFF_THRESHOLD`.
//! 3. **Flatness epsilon.** Meyda's `spectralFlatness` has no epsilon: it sums
//!    `log(amp)` (so any zero bin makes the geometric mean zero) and divides by
//!    the arithmetic mean (`extractors/spectralFlatness.ts:8-20`). This port
//!    reproduces that, including the `0 / 0 = NaN` result for digital silence.
//! 4. **Slope denominator.** Meyda's `spectralSlope` divides by
//!    `ampSum * (powFreqSum - freqSum^2)` instead of the textbook
//!    `n * powFreqSum - freqSum^2` (`extractors/spectralSlope.ts:27-30`), which
//!    flips the sign for ordinary spectra. Reproduced as-is.
//! 5. **Kurtosis.** Meyda's numerator uses `6 * mu1 * mu2` where the fourth
//!    standardised moment calls for `6 * mu1^2 * mu2`
//!    (`extractors/spectralKurtosis.ts:15-19`). Reproduced as-is; it makes the
//!    value large for narrow-band input.
//! 6. **Perceptual sharpness.** Meyda adds a constant `0.066 * exp(0.171 * (i + 1))`
//!    for every Bark band `i >= 15` instead of a weighted term
//!    (`extractors/perceptualSharpness.ts:12-21`), so sharpness is not monotonic
//!    above roughly 4 kHz. Reproduced as-is.
//! 7. Bark bands follow `createBarkScale` (`utilities.ts:44-55`) — the
//!    Traunmüller/Zwicker form `13*atan(f/1315.8) + 3.5*atan((f/7518)^2)` — and
//!    Meyda's `loudness` extractor (`extractors/loudness.ts`) with 24 bands.
//!    The band-limit walk is clamped so it can never index past the end of the
//!    limit table (the JS `Int32Array` silently drops out-of-range writes).
//!
//! The extractor owns its FFT plan, window and scratch buffers; `extract` only
//! allocates the returned [`MeydaFeatures`] (plus one scratch magnitude
//! vector per call).

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Number of Bark bands Meyda's `loudness` extractor uses by default
/// (`main.ts:249` `numberOfBarkBands: 24`).
pub const NUMBER_OF_BARK_BANDS: usize = 24;

/// Meyda's `spectralRolloff` energy threshold (`extractors/spectralRolloff.ts:16`).
pub const SPECTRAL_ROLLOFF_THRESHOLD: f64 = 0.99;

/// The window functions Meyda ships (`node_modules/meyda/src/windowing.ts`) that
/// this port needs. `Rect` is modV's choice for the live analyser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFunction {
    /// No window (`windowingFunction: "rect"`; `utilities.applyWindow` returns
    /// the signal untouched for `"rect"`, utilities.ts:25-37).
    Rect,
    /// Meyda's `hanning` window, `0.5 - 0.5*cos(2*pi*i/(size-1))`
    /// (`windowing.ts:33-41`).
    Hanning,
}

impl WindowFunction {
    /// Builds the window samples for `size`.
    fn coefficients(self, size: usize) -> Vec<f32> {
        match self {
            Self::Rect => vec![1.0; size],
            Self::Hanning => (0..size)
                .map(|i| {
                    let angle = 2.0 * std::f64::consts::PI * i as f64 / (size as f64 - 1.0);
                    (0.5 - 0.5 * angle.cos()) as f32
                })
                .collect(),
        }
    }
}

/// The feature bag for one analysis window. Field names mirror the Vuex feature
/// names (`store/modules/meyda.js:13-31`); the two `*_bin` companions are the
/// exact Meyda bin-domain values described in the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct MeydaFeatures {
    /// The time-domain window after windowing (Meyda's `buffer` extractor,
    /// `featureExtractors.ts:27-29`). Always `fft_size` samples.
    pub buffer: Vec<f32>,
    /// Full FFT bins as `[real, imaginary]`; length `fft_size` (fftjs returns
    /// the full complex spectrum, main.ts:415).
    pub complex_spectrum: Vec<[f32; 2]>,
    /// `sqrt(mean(x^2))` over [`Self::buffer`] (`extractors/rms.ts`).
    pub rms: f32,
    /// Number of sign changes between adjacent samples (`extractors/zcr.ts`).
    /// Meyda returns a count, not a rate.
    pub zcr: f32,
    /// `sum(x^2)` over [`Self::buffer`] (`extractors/energy.ts`).
    pub energy: f32,
    /// Magnitude-weighted mean frequency in Hz (WS-9 table); see
    /// [`Self::spectral_centroid_bin`] for Meyda's raw value.
    pub spectral_centroid: f32,
    /// Meyda's `mu(1, ampSpectrum)` — the centroid as a bin index.
    pub spectral_centroid_bin: f32,
    /// Geometric mean / arithmetic mean of the magnitude spectrum
    /// (`extractors/spectralFlatness.ts`).
    pub spectral_flatness: f32,
    /// Meyda's linear regression of magnitude against frequency
    /// (`extractors/spectralSlope.ts`).
    pub spectral_slope: f32,
    /// Frequency below which `SPECTRAL_ROLLOFF_THRESHOLD` of the magnitude sum
    /// lies, in Hz (`extractors/spectralRolloff.ts`).
    pub spectral_rolloff: f32,
    /// Weighted standard deviation around the centroid, in Hz.
    pub spectral_spread: f32,
    /// `sqrt(mu(2) - mu(1)^2)` — the spread as a bin count.
    pub spectral_spread_bin: f32,
    /// Weighted third standardised moment (`extractors/spectralSkewness.ts`).
    pub spectral_skewness: f32,
    /// Weighted fourth standardised moment (`extractors/spectralKurtosis.ts`,
    /// including its `6*mu1*mu2` quirk).
    pub spectral_kurtosis: f32,
    /// `((total - max) / total)^2` over the Bark-band loudness
    /// (`extractors/perceptualSpread.ts`).
    pub perceptual_spread: f32,
    /// `0.11 / total` weighted sum of the Bark-band loudness
    /// (`extractors/perceptualSharpness.ts`).
    pub perceptual_sharpness: f32,
}

/// A reusable Meyda-compatible analyser for a fixed `fft_size` / sample rate.
pub struct MeydaExtractor {
    fft_size: usize,
    sample_rate: f32,
    window: WindowFunction,
    window_values: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    time_buffer: Vec<f32>,
    windowed: Vec<f32>,
    /// Complex FFT buffer. fftjs (which Meyda uses) is a full complex FFT with
    /// a zero imaginary part, so the port does the same; `windowed` stays
    /// untouched for [`MeydaFeatures::buffer`] and the time-domain features.
    fft_buffer: Vec<Complex<f32>>,
    /// Meyda's `createBarkScale(bufferSize, sampleRate, bufferSize)`; entry `i`
    /// is the Bark value of bin `i` (utilities.ts:44-55).
    bark_scale: Vec<f32>,
}

impl MeydaExtractor {
    /// Creates an extractor using modV's window (`rect`, see
    /// `setup-media.js:365`).
    ///
    /// # Panics
    ///
    /// Panics when `fft_size` is smaller than 4 or odd: Meyda requires a
    /// power-of-two buffer (`main.ts:296-298`), and the magnitude spectrum is
    /// only the lower half (`fft_size / 2` bins), so 4 is the smallest size that
    /// keeps it non-empty and every Bark band index in range.
    pub fn new(fft_size: usize, sample_rate: f32) -> Self {
        Self::with_window(fft_size, sample_rate, WindowFunction::Rect)
    }

    /// Creates an extractor with an explicit window function.
    ///
    /// # Panics
    ///
    /// As [`Self::new`].
    pub fn with_window(fft_size: usize, sample_rate: f32, window: WindowFunction) -> Self {
        assert!(
            fft_size >= 4 && fft_size % 2 == 0,
            "MeydaExtractor requires an even fft_size >= 4, got {fft_size}"
        );

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);
        let fft_scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        let bark_scale = (0..fft_size)
            .map(|i| {
                let f = i as f64 * sample_rate as f64 / fft_size as f64;
                13.0 * (f / 1315.8).atan() + 3.5 * ((f / 7518.0).powi(2)).atan()
            })
            .map(|bark| bark as f32)
            .collect();

        Self {
            fft_size,
            sample_rate,
            window,
            window_values: window.coefficients(fft_size),
            fft,
            fft_scratch,
            time_buffer: vec![0.0; fft_size],
            windowed: vec![0.0; fft_size],
            fft_buffer: vec![Complex::new(0.0, 0.0); fft_size],
            bark_scale,
        }
    }

    /// The configured FFT/window size.
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// The configured sample rate.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// The configured window.
    pub fn window(&self) -> WindowFunction {
        self.window
    }

    /// Analyzes one buffer and returns the whole feature bag.
    ///
    /// Mirrors `Meyda.extract` (`main.ts:295-390`) and
    /// `prepareSignalWithSpectrum` (main.ts:393-423): window, FFT, magnitude
    /// spectrum over the lower half, then every requested extractor.
    pub fn extract(&mut self, time_domain: &[f32]) -> MeydaFeatures {
        let fft_size = self.fft_size;
        let copy_len = time_domain.len().min(fft_size);

        self.time_buffer[..copy_len].copy_from_slice(&time_domain[..copy_len]);
        self.time_buffer[copy_len..].fill(0.0);

        for i in 0..fft_size {
            self.windowed[i] = self.time_buffer[i] * self.window_values[i];
            self.fft_buffer[i] = Complex::new(self.windowed[i], 0.0);
        }

        // Full complex FFT, like fftjs. (rustfft 6 has no `RealFftPlanner`; that
        // lives in the separate `realfft` crate, which this port does not take
        // on as an extra dependency.)
        self.fft
            .process_with_scratch(&mut self.fft_buffer, &mut self.fft_scratch);

        let half = fft_size / 2;
        let mut amp_spectrum = vec![0.0f64; half];
        for (i, amp) in amp_spectrum.iter_mut().enumerate() {
            let bin = self.fft_buffer[i];
            *amp = ((bin.re as f64).powi(2) + (bin.im as f64).powi(2)).sqrt();
        }

        let complex_spectrum: Vec<[f32; 2]> =
            self.fft_buffer.iter().map(|bin| [bin.re, bin.im]).collect();

        let signal = &self.windowed;
        let n = signal.len() as f64;

        let energy: f64 = signal.iter().map(|&x| (x as f64) * (x as f64)).sum();
        let rms = (energy / n).sqrt();
        let zcr = signal
            .windows(2)
            .filter(|pair| (pair[0] >= 0.0) != (pair[1] >= 0.0))
            .count() as f64;

        let centroid_bin = mu(1, &amp_spectrum);
        let spread_bin = (mu(2, &amp_spectrum) - centroid_bin.powi(2)).sqrt();
        let bin_width = self.sample_rate as f64 / fft_size as f64;

        let loudness = self.loudness(&amp_spectrum);

        MeydaFeatures {
            buffer: self.windowed.clone(),
            complex_spectrum,
            rms: rms as f32,
            zcr: zcr as f32,
            energy: energy as f32,
            spectral_centroid: (centroid_bin * bin_width) as f32,
            spectral_centroid_bin: centroid_bin as f32,
            spectral_flatness: spectral_flatness(&amp_spectrum) as f32,
            spectral_slope: self.spectral_slope(&amp_spectrum) as f32,
            spectral_rolloff: self.spectral_rolloff(&amp_spectrum) as f32,
            spectral_spread: (spread_bin * bin_width) as f32,
            spectral_spread_bin: spread_bin as f32,
            spectral_skewness: spectral_skewness(&amp_spectrum) as f32,
            spectral_kurtosis: spectral_kurtosis(&amp_spectrum) as f32,
            perceptual_spread: loudness.perceptual_spread as f32,
            perceptual_sharpness: loudness.perceptual_sharpness as f32,
        }
    }

    /// `extractors/spectralRolloff.ts`. Meyda's `nyqBin` is
    /// `sampleRate / (2 * (bins - 1))` because the magnitude spectrum excludes
    /// the Nyquist bin; the loop walks down from the top until the remaining sum
    /// is at or below the threshold. Digital silence leaves the index at the top
    /// bin, so it reports just under Nyquist — a Meyda quirk kept as-is.
    fn spectral_rolloff(&self, amp_spectrum: &[f64]) -> f64 {
        let nyquist_bin = self.sample_rate as f64 / (2.0 * (amp_spectrum.len() as f64 - 1.0));
        let energy: f64 = amp_spectrum.iter().sum();
        let threshold = SPECTRAL_ROLLOFF_THRESHOLD * energy;
        let mut remaining = energy;
        let mut n = amp_spectrum.len() as isize - 1;

        while remaining > threshold && n >= 0 {
            remaining -= amp_spectrum[n as usize];
            n -= 1;
        }

        (n + 1) as f64 * nyquist_bin
    }

    /// `extractors/spectralSlope.ts`, including its non-standard denominator.
    fn spectral_slope(&self, amp_spectrum: &[f64]) -> f64 {
        let mut amp_sum = 0.0;
        let mut freq_sum = 0.0;
        let mut pow_freq_sum = 0.0;
        let mut amp_freq_sum = 0.0;

        for (i, amp) in amp_spectrum.iter().enumerate() {
            let frequency = i as f64 * self.sample_rate as f64 / self.fft_size as f64;
            amp_sum += amp;
            pow_freq_sum += frequency * frequency;
            freq_sum += frequency;
            amp_freq_sum += frequency * amp;
        }

        let denominator = amp_sum * (pow_freq_sum - freq_sum.powi(2));
        (amp_spectrum.len() as f64 * amp_freq_sum - freq_sum * amp_sum) / denominator
    }

    /// `extractors/loudness.ts` and the two perceptual extractors built on it.
    fn loudness(&self, amp_spectrum: &[f64]) -> Loudness {
        let mut specific = vec![0.0f64; NUMBER_OF_BARK_BANDS];
        let mut limits = vec![0usize; NUMBER_OF_BARK_BANDS + 1];
        let last_bark = self.bark_scale[amp_spectrum.len() - 1] as f64;

        limits[0] = 0;
        let mut current_band_end = last_bark / NUMBER_OF_BARK_BANDS as f64;
        let mut current_band = 1;
        for (i, &bark) in self.bark_scale[..amp_spectrum.len()].iter().enumerate() {
            while bark as f64 > current_band_end {
                if current_band >= NUMBER_OF_BARK_BANDS {
                    // The JS writes past the end of the Int32Array here, which is
                    // silently ignored; clamping is the explicit equivalent.
                    current_band_end = f64::INFINITY;
                    break;
                }
                limits[current_band] = i;
                current_band += 1;
                current_band_end = current_band as f64 * last_bark / NUMBER_OF_BARK_BANDS as f64;
            }
        }
        limits[NUMBER_OF_BARK_BANDS] = amp_spectrum.len() - 1;

        for (i, band) in specific.iter_mut().enumerate() {
            let sum: f64 = amp_spectrum[limits[i]..limits[i + 1]].iter().sum();
            *band = sum.powf(0.23);
        }
        let total: f64 = specific.iter().sum();

        let max = specific.iter().copied().fold(0.0f64, f64::max);
        let perceptual_spread = ((total - max) / total).powi(2); // perceptualSpread.ts:12-16

        let mut sharpness = 0.0;
        for i in 0..specific.len() {
            if i < 15 {
                sharpness += (i as f64 + 1.0) * specific[i + 1]; // perceptualSharpness.ts:13-17
            } else {
                sharpness += 0.066 * (0.171 * (i as f64 + 1.0)).exp(); // :15
            }
        }
        let perceptual_sharpness = sharpness * 0.11 / total; // :19

        Loudness {
            perceptual_spread,
            perceptual_sharpness,
        }
    }
}

/// The two perceptual values derived from the Bark-band loudness.
struct Loudness {
    perceptual_spread: f64,
    perceptual_sharpness: f64,
}

/// `mu(i, amplitudeSpectrum)` from `extractors/extractorUtilities.ts:1-10`:
/// `sum(k^i * |amp[k]|) / sum(amp[k])`. Returns `NaN` for an all-zero spectrum
/// (JS `0 / 0`), which callers see in every centroid-derived feature.
fn mu(i: i32, amp_spectrum: &[f64]) -> f64 {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (k, &amp) in amp_spectrum.iter().enumerate() {
        numerator += (k as f64).powi(i) * amp.abs();
        denominator += amp;
    }
    numerator / denominator
}

/// `extractors/spectralFlatness.ts`: geometric mean / arithmetic mean, with no
/// epsilon. A single zero bin zeroes the geometric mean; an all-zero spectrum
/// yields `0 / 0 = NaN`, exactly as in JS.
fn spectral_flatness(amp_spectrum: &[f64]) -> f64 {
    let mut log_sum = 0.0;
    let mut has_zero = false;
    let mut sum = 0.0;

    for &amp in amp_spectrum {
        sum += amp;
        if amp <= 0.0 {
            has_zero = true;
        } else {
            log_sum += amp.ln();
        }
    }

    let geometric = if has_zero {
        0.0
    } else {
        (log_sum / amp_spectrum.len() as f64).exp()
    };

    geometric * amp_spectrum.len() as f64 / sum
}

/// `extractors/spectralSkewness.ts:13-17`.
fn spectral_skewness(amp_spectrum: &[f64]) -> f64 {
    let mu1 = mu(1, amp_spectrum);
    let mu2 = mu(2, amp_spectrum);
    let mu3 = mu(3, amp_spectrum);
    let numerator = 2.0 * mu1.powi(3) - 3.0 * mu1 * mu2 + mu3;
    let denominator = (mu2 - mu1.powi(2)).sqrt().powi(3);
    numerator / denominator
}

/// `extractors/spectralKurtosis.ts:13-19` — note the `6 * mu1 * mu2` term (the
/// JS source, not the textbook `6 * mu1^2 * mu2`).
fn spectral_kurtosis(amp_spectrum: &[f64]) -> f64 {
    let mu1 = mu(1, amp_spectrum);
    let mu2 = mu(2, amp_spectrum);
    let mu3 = mu(3, amp_spectrum);
    let mu4 = mu(4, amp_spectrum);
    let numerator = -3.0 * mu1.powi(4) + 6.0 * mu1 * mu2 - 4.0 * mu1 * mu3 + mu4;
    let denominator = (mu2 - mu1.powi(2)).sqrt().powi(4);
    numerator / denominator
}

#[cfg(test)]
mod tests {

    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;
    const FFT_SIZE: usize = 1024;

    /// A sine tone of `frequency` Hz and `amplitude` peak.
    fn sine(frequency: f32, amplitude: f32) -> Vec<f32> {
        (0..FFT_SIZE)
            .map(|i| {
                amplitude * (2.0 * std::f32::consts::PI * frequency * i as f32 / SAMPLE_RATE).sin()
            })
            .collect()
    }

    /// Deterministic white noise in `[-1, 1)` (small LCG so the fixture does not
    /// need a `rand` dependency).
    fn white_noise(seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..FFT_SIZE)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state as f64 / (1u64 << 32) as f64) as f32 * 2.0 - 1.0
            })
            .collect()
    }

    fn hanning() -> MeydaExtractor {
        MeydaExtractor::with_window(FFT_SIZE, SAMPLE_RATE, WindowFunction::Hanning)
    }

    fn rect_extractor() -> MeydaExtractor {
        MeydaExtractor::new(FFT_SIZE, SAMPLE_RATE)
    }

    #[test]
    fn new_uses_modvs_rect_window_and_the_requested_geometry() {
        let extractor = rect_extractor();

        assert_eq!(extractor.fft_size(), FFT_SIZE);
        assert_eq!(extractor.sample_rate(), SAMPLE_RATE);
        assert_eq!(extractor.window(), WindowFunction::Rect);
        assert_eq!(extractor.bark_scale.len(), FFT_SIZE);
    }

    #[test]
    #[should_panic(expected = "MeydaExtractor requires an even fft_size")]
    fn new_rejects_an_odd_fft_size() {
        let _ = MeydaExtractor::new(1023, SAMPLE_RATE);
    }

    #[test]
    fn buffer_is_the_windowed_signal_and_pads_short_input() {
        let mut extractor = rect_extractor();
        let input = vec![0.5f32; 16];
        let features = extractor.extract(&input);

        assert_eq!(features.buffer.len(), FFT_SIZE);
        assert!(features.buffer[..16].iter().all(|&x| x == 0.5));
        assert!(features.buffer[16..].iter().all(|&x| x == 0.0));

        // With `rect`, the buffer is exactly the raw input.
        let tone = sine(440.0, 0.25);
        let features = extractor.extract(&tone);
        assert_eq!(features.buffer, tone);
    }

    #[test]
    fn complex_spectrum_has_fft_size_bins_and_mirrors_the_conjugate_half() {
        let mut extractor = rect_extractor();
        let features = extractor.extract(&sine(1000.0, 1.0));

        assert_eq!(features.complex_spectrum.len(), FFT_SIZE);
        for k in 1..FFT_SIZE / 2 {
            let lower = features.complex_spectrum[k];
            let upper = features.complex_spectrum[FFT_SIZE - k];
            // f32 roundoff in a 1024-point transform of a full-scale tone is
            // O(1e-3) of a ~512-magnitude bin; a 0.5 absolute tolerance still
            // proves the mirroring without depending on exact arithmetic.
            assert!((lower[0] - upper[0]).abs() < 0.5, "real bin {k}");
            assert!((lower[1] + upper[1]).abs() < 0.5, "imag bin {k}");
        }
    }

    #[test]
    fn rms_matches_amplitude_over_sqrt_two() {
        let mut extractor = rect_extractor();
        let features = extractor.extract(&sine(440.0, 1.0));

        // A full-scale sine's RMS is 1/sqrt(2); the 440 Hz tone is not bin
        // centred, but RMS is time-domain and unaffected by the FFT.
        assert!((features.rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-2);
        assert!((features.rms - 0.70710678).abs() < 1e-2);
    }

    #[test]
    fn rms_and_energy_scale_with_amplitude() {
        let mut extractor = rect_extractor();
        let half = extractor.extract(&sine(440.0, 0.5));
        let full = extractor.extract(&sine(440.0, 1.0));

        assert!((full.rms / half.rms - 2.0).abs() < 1e-4);
        assert!((full.energy / half.energy - 4.0).abs() < 1e-4);
    }

    #[test]
    fn energy_is_the_sum_of_squares_of_the_buffer() {
        let mut extractor = rect_extractor();
        let input = sine(440.0, 0.3);
        let features = extractor.extract(&input);
        let expected: f32 = input.iter().map(|x| x * x).sum();

        assert!((features.energy - expected).abs() < 1e-2 * expected);
        let from_buffer: f32 = features.buffer.iter().map(|x| x * x).sum();
        assert!((features.energy - from_buffer).abs() < 1e-3);
    }

    #[test]
    fn zcr_matches_the_tone_frequency() {
        let mut extractor = rect_extractor();
        let features = extractor.extract(&sine(440.0, 1.0));

        // Two crossings per cycle minus the boundary sample: 2*f/sr*(n-1).
        let expected = 2.0 * 440.0 / SAMPLE_RATE * (FFT_SIZE as f32 - 1.0);
        assert!(
            (features.zcr - expected).abs() <= 2.0,
            "zcr {} expected {expected}",
            features.zcr
        );

        let quiet = extractor.extract(&vec![0.0; FFT_SIZE]);
        assert_eq!(quiet.zcr, 0.0);
        assert_eq!(quiet.rms, 0.0);
        assert_eq!(quiet.energy, 0.0);
    }

    #[test]
    fn spectral_centroid_tracks_the_tone_frequency_and_its_bin_index() {
        let mut extractor = hanning();
        let features = extractor.extract(&sine(440.0, 1.0));

        // WS-9 asks for Hz; Meyda's own value is the bin index (9.39 here).
        assert!(
            (features.spectral_centroid - 440.0).abs() < 0.05 * 440.0,
            "centroid {} Hz",
            features.spectral_centroid
        );
        let bin_width = SAMPLE_RATE / FFT_SIZE as f32;
        assert!((features.spectral_centroid_bin - 440.0 / bin_width).abs() < 0.5);

        let high = extractor.extract(&sine(5_000.0, 1.0));
        assert!(high.spectral_centroid > features.spectral_centroid * 5.0);
    }

    #[test]
    fn spectral_rolloff_lands_near_the_tone() {
        let mut extractor = hanning();
        let bin_width = SAMPLE_RATE / FFT_SIZE as f32;

        // Meyda's rolloff uses 99 % of the magnitude sum, so a Hann main lobe
        // (2 bins either side) pushes it a couple of bins above the tone.
        let low = extractor.extract(&sine(440.0, 1.0));
        assert!(
            (low.spectral_rolloff - 440.0).abs() < 3.0 * bin_width,
            "rolloff {} Hz",
            low.spectral_rolloff
        );

        let high = extractor.extract(&sine(8_000.0, 1.0));
        assert!(
            (high.spectral_rolloff - 8_000.0).abs() < 3.0 * bin_width,
            "rolloff {} Hz",
            high.spectral_rolloff
        );
        assert!(high.spectral_rolloff > low.spectral_rolloff);
        assert!(high.spectral_rolloff <= SAMPLE_RATE / 2.0 * 1.01);
    }

    #[test]
    fn spectral_flatness_separates_a_tone_from_noise() {
        let mut extractor = hanning();
        let tone = extractor.extract(&sine(440.0, 1.0));
        let noise = extractor.extract(&white_noise(0x1234_5678));

        assert!(
            tone.spectral_flatness < 0.01,
            "tone {}",
            tone.spectral_flatness
        );
        assert!(
            noise.spectral_flatness > 0.6,
            "noise {}",
            noise.spectral_flatness
        );
        assert!(noise.spectral_flatness > 10.0 * tone.spectral_flatness);
    }

    #[test]
    fn spectral_flatness_reports_nan_for_silence_like_meyda() {
        let mut extractor = rect_extractor();
        let features = extractor.extract(&vec![0.0; FFT_SIZE]);

        // `0 / 0` in spectralFlatness.ts:19. For digital silence Meyda's
        // rolloff also walks to the top of the spectrum (0 > 0 is false).
        assert!(features.spectral_flatness.is_nan());
        assert!(features.spectral_centroid.is_nan());
        assert!((features.spectral_rolloff - SAMPLE_RATE / 2.0).abs() < 100.0);
        assert_eq!(features.zcr, 0.0);
    }

    #[test]
    fn spectral_slope_follows_meydas_denominator_sign() {
        // Meyda's denominator is `ampSum * (powFreqSum - freqSum^2)`, which is
        // negative for ordinary spectra, so a rising magnitude spectrum gets a
        // negative slope and a falling one a positive slope (the textbook
        // convention is the other way around). The private extractor is called
        // directly so the magnitude ramp is exact.
        let rising: Vec<f64> = (0..FFT_SIZE / 2).map(|i| i as f64 / 512.0).collect();
        let falling: Vec<f64> = (0..FFT_SIZE / 2)
            .map(|i| (512 - i) as f64 / 512.0)
            .collect();

        let extractor = rect_extractor();
        let rising = extractor.spectral_slope(&rising);
        let falling = extractor.spectral_slope(&falling);

        assert!(rising.is_finite() && falling.is_finite());
        assert!(rising < 0.0, "rising {rising}");
        assert!(falling > 0.0, "falling {falling}");
    }

    #[test]
    fn spectral_spread_is_wider_for_a_two_tone_spectrum() {
        let mut extractor = hanning();
        let single = extractor.extract(&sine(500.0, 1.0));

        let two_tone: Vec<f32> = sine(500.0, 0.5)
            .iter()
            .zip(sine(4_000.0, 0.5).iter())
            .map(|(a, b)| a + b)
            .collect();
        let two_tone = extractor.extract(&two_tone);

        assert!(two_tone.spectral_spread > 10.0 * single.spectral_spread);
        assert!(
            (two_tone.spectral_spread_bin
                - two_tone.spectral_spread / (SAMPLE_RATE / FFT_SIZE as f32))
                .abs()
                < 1e-2
        );
        assert!(single.spectral_spread > 0.0);
    }

    #[test]
    fn skewness_and_kurtosis_are_finite_and_noises_skewness_near_zero() {
        let mut extractor = hanning();
        let noise = extractor.extract(&white_noise(0x0BAD_F00D));

        assert!(noise.spectral_skewness.is_finite());
        assert!(noise.spectral_kurtosis.is_finite());
        assert!(
            noise.spectral_skewness.abs() < 0.5,
            "skew {}",
            noise.spectral_skewness
        );

        // A symmetric two-tone spectrum is close to symmetric about its centroid.
        let two_tone: Vec<f32> = sine(1_000.0, 0.5)
            .iter()
            .zip(sine(2_000.0, 0.5).iter())
            .map(|(a, b)| a + b)
            .collect();
        let two_tone = extractor.extract(&two_tone);
        assert!(two_tone.spectral_skewness.abs() < 2.0);
    }

    #[test]
    fn perceptual_sharpness_rises_with_pitch_and_spread_falls() {
        let mut extractor = hanning();
        let low = extractor.extract(&sine(200.0, 1.0));
        let mid = extractor.extract(&sine(1_000.0, 1.0));
        let high = extractor.extract(&sine(3_000.0, 1.0));
        let very_high = extractor.extract(&sine(5_000.0, 1.0));

        // Meyda's sharpness is monotonic up to the exp() branch at band 15
        // (~4 kHz); below that it tracks pitch. 5 kHz is only checked for
        // finiteness (the exp() branch breaks monotonicity above band 15).
        assert!(low.perceptual_sharpness < mid.perceptual_sharpness);
        assert!(mid.perceptual_sharpness < high.perceptual_sharpness);
        assert!(low.perceptual_spread >= mid.perceptual_spread);
        assert!(mid.perceptual_spread > high.perceptual_spread);
        assert!(very_high.perceptual_sharpness.is_finite());
        for value in [
            low.perceptual_spread,
            low.perceptual_sharpness,
            high.perceptual_spread,
            high.perceptual_sharpness,
        ] {
            assert!(value.is_finite() && value >= 0.0);
        }
    }

    #[test]
    fn reuses_the_extractor_across_buffers_without_stale_state() {
        let mut extractor = hanning();
        let tone = sine(440.0, 1.0);
        let first = extractor.extract(&tone);
        let _ = extractor.extract(&white_noise(7));
        let again = extractor.extract(&tone);

        assert_eq!(first, again);
    }

    #[test]
    fn windowing_changes_the_buffer_but_not_the_time_domain_geometry() {
        let mut rect = MeydaExtractor::new(FFT_SIZE, SAMPLE_RATE);
        let mut hann = hanning();
        let tone = sine(440.0, 1.0);

        let rect_features = rect.extract(&tone);
        let hann_features = hann.extract(&tone);

        assert_eq!(rect_features.buffer, tone);
        assert_ne!(hann_features.buffer, tone);
        assert!(hann_features.buffer[0].abs() < 1e-3);
        assert!(hann_features.buffer[FFT_SIZE / 2].abs() > 0.9);
        // RMS of a Hann-windowed full-scale sine is sqrt(mean(w^2) * 1/2) =
        // sqrt(3/8 * 1/2) = sqrt(3/16).
        assert!((hann_features.rms - (3.0f32 / 16.0).sqrt()).abs() < 1e-2);
    }
}
