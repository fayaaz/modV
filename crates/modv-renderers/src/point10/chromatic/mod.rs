//! Shared Chromatic Line / Chromatic Wave simulation.
//!
//! Extracted from the original ChromaticLine port so ChromaticWave (which
//! subclasses ChromaticLine in the JS) can reuse the ring buffer, the texel
//! windows and the band read instead of duplicating them.
//!
//! # The JS design, in one paragraph
//!
//! `ChromaticLine.js` is a full-screen fragment shader whose line is evaluated
//! three times with the x input radially offset per channel, fed by a ring
//! buffer of ~5.9 s of audio ([`HORIZON`] samples). Each texel of a 256x1
//! waveform texture is the mean `|v|` of its own ~23 ms window ([`SPX`] = 1024
//! samples). Red reads the bass band, green the mids, blue the highs, each
//! through `P9Audio`'s damping-2 lane. The scene's `texelByte` mapping differs
//! between the two variants — the line biases the texel up from centre so it
//! sits at the middle in silence, the wave writes the raw energy because its
//! shader multiplies a sine by `0.75 + 0.5 * energy`.
//!
//! [`TexelMapping`] carries that difference; everything else (ring, windows,
//! damping, band→uniform curves) is shared.
//!
//! Line-number references below (e.g. "ChromaticLine.js:171") are anchors, not
//! contracts — they drift with the source file.

#![forbid(unsafe_code)]

pub mod line;
pub mod wave;

/// Samples per texel (~23 ms at 44.1 kHz). `ChromaticLine.js: this.SPX = 1024`.
pub(crate) const SPX: usize = 1024;
/// Texel count of the wave texture. `ChromaticLine.js: this.WAVE_N = 256`.
pub(crate) const WAVE_N: usize = 256;
/// Displayed horizon in samples (SPX * WAVE_N, ~5.9 s). `ChromaticLine.js: this.D`.
pub(crate) const HORIZON: usize = SPX * WAVE_N;
/// Ring length: horizon + one frame's append burst headroom.
/// `ChromaticLine.js: this.R = this.D + 4096` (line 75).
pub(crate) const RING_LEN: usize = HORIZON + 4096;
/// Byte size of the 256x1 RGBA wave texture payload.
pub(crate) const WAVE_BYTES: usize = WAVE_N * 4;
/// Sample count of `P9Audio.wave`. `p10Audio.js: this.wave = new Float32Array(1024)`.
/// `k = min(round(rate * dt), wave.length)` in ChromaticLine.js:168.
pub(crate) const SAMPLE_CAP: usize = 1024;
/// Sample rate of [`SyntheticSine`].
pub(crate) const SAMPLE_RATE: u32 = 48_000;
/// Fixture sine amplitude. Note the ring stores the raw signed sample, but
/// texel bytes are built from mean |v|, so the sign does not leak through.
const SINE_AMPLITUDE: f32 = 0.9;
/// Tau of the damped band read that approximates P9Audio damping lane 2
/// (`band(-1, 2, n)`, p10Audio.js:71-80). The fixture uses a fixed lane-2 tau
/// of 0.25 s; the live path reads the bridge's lane-2 (`lanes[6 + band]`),
/// which the audio bridge already low-passes at 0.12 s.
const BAND_TAU_SECONDS: f32 = 0.25;

/// Per-variant texel byte mapping.
///
/// `ChromaticLine.js: texelValue`/`texelByte` (lines 129-134) and
/// `ChromaticWave.js: texelByte` (lines 59-61).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TexelMapping {
    /// Line: `128 + ((0.5 + 0.5 * e) * 127) | 0` — biased up from centre.
    Line,
    /// Wave: `(e * 255) | 0` — plain energy, the shader's `mul` uses it raw.
    Energy,
}

impl TexelMapping {
    /// Byte value for a window's mean `|v|` (`e` in `0..1`).
    pub(crate) fn texel_byte(self, e: f32) -> u8 {
        match self {
            TexelMapping::Line => {
                // `const t = 0.5 + 0.5 * e; return 128 + ((t * 127) | 0);`
                // `|0` is truncation toward zero: the range is [0.5, 1.0] ->
                // t*127 in [63.5, 127] -> truncated [63, 127] -> byte in
                // [191, 255]. Silence therefore yields 191, NOT 128 — the 0.5
                // bias exists so unsigned textures never clip negative values,
                // and it is harmless because `amp` is 0 in silence. Do NOT
                // "fix" this: the visual baseline is `amp`, not the texel.
                let t = 0.5 + 0.5 * e;
                (128.0 + (t * 127.0).trunc()) as u8
            }
            TexelMapping::Energy => {
                // `return (e * 255) | 0;`
                (e * 255.0).trunc() as u8
            }
        }
    }
}

/// Fixture audio source for ChromaticLine: a sine wave at `freq_hz`.
///
/// This stands in for the live analyser path (`AudioBands` → `AudioBridge` →
/// `P10Audio.wave`) that the scene consumes in production. Samples are
/// generated on demand from an absolute sample counter, 48 000 Hz, mono
/// (duplicated to both channels in the real graph; the scene reads a single
/// mono waveform), amplitude 0.9.
pub struct SyntheticSine {
    freq_hz: f32,
    sample_index: u64,
}

impl SyntheticSine {
    /// Create a 48 000 Hz sine fixture at `freq_hz`. The absolute sample
    /// counter starts at 0, so the generated sequence is deterministic.
    pub fn new(freq_hz: f32) -> Self {
        Self {
            freq_hz,
            sample_index: 0,
        }
    }

    /// Sample rate of the fixture in Hz (always 48 000).
    pub fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    /// One raw signed sample of the fixture at the absolute sample index.
    fn next_sample(&mut self) -> f32 {
        let t = self.sample_index as f32 / SAMPLE_RATE as f32;
        self.sample_index += 1;
        SINE_AMPLITUDE * (2.0 * std::f32::consts::PI * self.freq_hz * t).sin()
    }

    /// The three band targets the fixture reports through the P9Audio
    /// damping-2 lane, mirroring the bass/mid/treble split the audio bridge
    /// computes for a tone: sub-150 Hz lands in bass, up to 2 kHz in mid,
    /// above that in treble. The envelope is a steady 0.5 (a sine's RMS
    /// times ~1.4, capped) so the band read is deterministic.
    fn band_targets(&self) -> [f32; 3] {
        const A: f32 = 0.5;
        if self.freq_hz <= 150.0 {
            [A, 0.0, 0.0]
        } else if self.freq_hz <= 2000.0 {
            [0.0, A, 0.0]
        } else {
            [0.0, 0.0, A]
        }
    }
}

/// CPU-side simulation state shared by the line and the wave: the ring buffer,
/// the texel windows and the damped band read. GPU-free so unit tests can drive
/// it directly without a device.
pub(crate) struct ChromaticLineSim {
    /// Ring of the last `RING_LEN` samples. `ChromaticLine.js: this.ring` (line 76).
    ring: Vec<f32>,
    /// Write position into the ring. `ChromaticLine.js: this.head = 0` (line 77).
    head: usize,
    /// Damped band read (`P9Audio._smoo` lanes 6..=8, i.e. damping lane 2 for
    /// bands 0..=2). `p10Audio.js` lane layout: `[damping * 3 + band]`.
    smoo: [f32; 3],
    /// `ChromaticLine.js: this.amp = 1` — scale on the fixture band curve.
    amp_scale: f32,
    /// `ChromaticLine.js: this.glowScale = 1` (line 81).
    glow_scale: f32,
    /// `ChromaticLine.js: this.distortion = 0.05` (line 79).
    distortion: f32,
}

/// Output of one simulation advance: the rebuilt wave texture payload and the
/// band-driven uniforms, plus the raw damped bands (the wave variant maps them
/// differently). Kept separate from the GPU write so tests can inspect the
/// values without a device.
pub(crate) struct AdvanceOutput {
    pub(crate) bytes: [u8; WAVE_BYTES],
    pub(crate) band_uniforms: BandUniforms,
}

/// Band-driven uniform values of the line, before `resolution` is merged in.
/// `ChromaticLine.js: _audioUniforms` (lines 137-151).
pub(crate) struct BandUniforms {
    pub(crate) amp: [f32; 3],
    pub(crate) glow: [f32; 3],
    pub(crate) x_scale: [f32; 3],
    pub(crate) distortion: f32,
}

impl ChromaticLineSim {
    /// `ChromaticLine.js: _init()` ring setup (lines 71-83).
    pub(crate) fn new() -> Self {
        Self {
            ring: vec![0.0; RING_LEN],
            head: 0,
            smoo: [0.0; 3],
            amp_scale: 1.0,
            glow_scale: 1.0,
            distortion: 0.05,
        }
    }

    /// Append `samples` to the ring, wrapping at `RING_LEN`.
    ///
    /// `ChromaticLine.js update()` (lines 171-175):
    /// `ring[(this.head + m) % this.R] = w[w.length - k + m]; this.head = (this.head + k) % this.R;`
    /// The fixture generates exactly the new samples, so the samples are
    /// appended directly instead of re-indexed from the analyser tail.
    pub(crate) fn append(&mut self, samples: &[f32]) {
        for (m, &v) in samples.iter().enumerate() {
            self.ring[(self.head + m) % RING_LEN] = v;
        }
        self.head = (self.head + samples.len()) % RING_LEN;
    }

    /// Append the newest `k` samples of an analyser waveform tail.
    ///
    /// Live path of `ChromaticLine.js:167-176`:
    /// `const k = Math.min(Math.round(this.rate * dt), this.p9.wave.length);`
    /// then `ring[(head + m) % R] = wave[wave.length - k + m]`. The wave is
    /// 1024 samples (~23 ms), so a slower-than-24 fps gap drops samples rather
    /// than re-reading them.
    pub(crate) fn append_wave_tail(&mut self, wave: &[f32]) {
        if wave.is_empty() {
            return;
        }
        let k = wave.len();
        for m in 0..k {
            self.ring[(self.head + m) % RING_LEN] = wave[wave.len() - k + m];
        }
        self.head = (self.head + k) % RING_LEN;
    }

    /// Rebuild the 256x1 RGBA wave texture payload from the ring.
    ///
    /// `ChromaticLine.js update()` texel loop (lines 180-205): each texel is
    /// the mean `|v|` of its own 23 ms window starting at the texel's ring
    /// position (`m = head - D + i*SPX mod R`), then mapped to a byte by the
    /// variant's [`TexelMapping`].
    pub(crate) fn compute_bytes(&self, mapping: TexelMapping) -> [u8; WAVE_BYTES] {
        let span = RING_LEN as i64;
        let mut bytes = [0_u8; WAVE_BYTES];
        for i in 0..WAVE_N {
            // `let m = (this.head - D + i * S) % R; if (m < 0) m += R;` —
            // rem_euclid is the same modulo with the sign handled, and the
            // i64 intermediates cannot overflow for these magnitudes.
            let base = self.head as i64 - HORIZON as i64 + i as i64 * SPX as i64;
            let mut m = base.rem_euclid(span) as usize;
            let mut sum = 0.0_f32;
            for _ in 0..SPX {
                sum += self.ring[m].abs();
                m += 1;
                if m == RING_LEN {
                    m = 0;
                }
            }
            // `const q = sum / S;` — 0..1 mean |v| of this window.
            let e = sum / SPX as f32;
            let byte = mapping.texel_byte(e);
            let o = i * 4;
            bytes[o] = byte;
            bytes[o + 1] = byte;
            bytes[o + 2] = byte;
            bytes[o + 3] = 255;
        }
        bytes
    }

    /// Advance the one-pole damped band read toward `target`.
    ///
    /// `P9Audio.band(-1, 2, n)` (p10Audio.js:71-80) reads `_smoo[2*3+n]`;
    /// lane 2 is a low-pass lane. The audio bridge computes the lanes with a
    /// per-lane one-pole; this fixture uses a fixed lane-2 time constant of
    /// `BAND_TAU_SECONDS` (0.25 s). Returns the updated damped values.
    pub(crate) fn update_bands(&mut self, dt_seconds: f32, target: [f32; 3]) -> [f32; 3] {
        let k = 1.0 - (-dt_seconds / BAND_TAU_SECONDS).exp();
        for i in 0..3 {
            self.smoo[i] += (target[i] - self.smoo[i]) * k;
        }
        self.smoo
    }

    /// Map damped bands to the **line**'s band-driven uniforms.
    ///
    /// `ChromaticLine.js _audioUniforms` (lines 137-151):
    /// `amp[i] = min(b*10, 0.8) * ampScale`, `glow[i] = (0.05 + b*0.08) * glowScale`,
    /// `xScale[i] = 2.5 + b*0.8`, `distortion` never changes.
    pub(crate) fn band_uniforms(&self, bands: [f32; 3]) -> BandUniforms {
        let mut amp = [0.0_f32; 3];
        let mut glow = [0.0_f32; 3];
        let mut x_scale = [0.0_f32; 3];
        for i in 0..3 {
            amp[i] = (bands[i] * 10.0).min(0.8) * self.amp_scale;
            glow[i] = (0.05 + bands[i] * 0.08) * self.glow_scale;
            x_scale[i] = 2.5 + bands[i] * 0.8;
        }
        BandUniforms {
            amp,
            glow,
            x_scale,
            distortion: self.distortion,
        }
    }

    /// Advance the simulation by `dt_seconds`: pull the genuinely new samples
    /// from the fixture (if any), rebuild the texel bytes and return them
    /// with the band-driven uniforms.
    ///
    /// `ChromaticLine.js update()` (lines 159-217), audio/ring/texel parts:
    /// with `this.rate = audio.ctx?.sampleRate || 44100` — here the fixture's
    /// rate (48 000) is used when audio is present.
    pub(crate) fn advance(
        &mut self,
        dt_seconds: f32,
        audio: &mut Option<SyntheticSine>,
    ) -> AdvanceOutput {
        let mut bands = [0.0_f32; 3];
        if let Some(sine) = audio.as_mut() {
            let rate = sine.sample_rate() as f32;
            // Guard pathological dt (NaN/negative) so the `as usize` cast can
            // never panic; a zero/negative k appends nothing, matching the JS
            // `if (k > 0)` guard (ChromaticLine.js:169).
            let k = ((rate * dt_seconds).round().max(0.0) as usize).min(SAMPLE_CAP);
            let mut samples = Vec::with_capacity(k);
            for _ in 0..k {
                samples.push(sine.next_sample());
            }
            self.append(&samples);
            bands = self.update_bands(dt_seconds, sine.band_targets());
        }
        // No audio (p9 undefined): `band()` returns 0 for every channel
        // (p10Audio.js:71-80), so amp falls to 0 and glow to 0.05 — the idle
        // glow line. The damped lanes are left untouched, matching the JS.
        let bytes = self.compute_bytes(TexelMapping::Line);
        let band_uniforms = self.band_uniforms(bands);
        AdvanceOutput {
            bytes,
            band_uniforms,
        }
    }
}
