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
//! # Production vs fixture
//!
//! Both scenes now drive the simulation from the live [`AudioBridge`]: the
//! per-frame append pulls the genuinely new samples off `bridge.wave`'s tail
//! and the bands come from `P9Audio.band(-1, 2, n)` ([`P10Audio`]) — exactly
//! what `ChromaticLine.js:update` does (lines 159-217). [`SyntheticSine`] is a
//! *test/headless* helper that fills a [`BandPayload`] with a deterministic
//! tone; it is never constructed on the production path.
//!
//! Line-number references below (e.g. "ChromaticLine.js:171") are anchors, not
//! contracts — they drift with the source file.
//!
//! [`AudioBridge`]: crate::point10::audio_bridge::AudioBridge
//! [`P10Audio`]: crate::point10::audio_bridge::P10Audio

#![forbid(unsafe_code)]

pub mod line;
pub mod wave;

use crate::point10::SceneProp;
use crate::point10::audio_bridge::{AUDIO_SAMPLES, BandPayload};

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
/// Sample rate of [`SyntheticSine`].
pub(crate) const SAMPLE_RATE: u32 = 48_000;
/// Fixture sine amplitude. Note the ring stores the raw signed sample, but
/// texel bytes are built from mean |v|, so the sign does not leak through.
const SINE_AMPLITUDE: f32 = 0.9;

/// `CHROMATIC_LINE_PROPS`, shared by the line and the wave
/// (`sceneProps.js:80-84`, bound for both at lines 369-378).
///
/// The wave carries `amplitude` even though its shader has no `amp` uniform:
/// the JS registry applies the same schema to both scenes and sets
/// `scene.ampScale`, which ChromaticWave's `_audioUniforms` simply never reads.
pub(crate) const CHROMATIC_PROPS: &[SceneProp] = &[
    SceneProp::float("distortion", "Chroma", 0.05, 0.0, 0.5, 0.001),
    SceneProp::float("amplitude", "Wave Amplitude", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("glow", "Glow", 1.0, 0.0, 3.0, 0.01),
];

/// Number of new samples to append to the ring this frame.
///
/// `ChromaticLine.js:168`: `Math.min(Math.round(this.rate * dt), this.p9.wave.length)`
/// with `this.rate = audio.ctx?.sampleRate || 44100`. A frame longer than the
/// 1024-sample (~23 ms) analyser window drops samples instead of re-reading
/// them, so the scroll stays real-time below ~24 fps.
pub(crate) fn sample_count(sample_rate: f32, dt: f32) -> usize {
    if !dt.is_finite() || !sample_rate.is_finite() || dt <= 0.0 || sample_rate <= 0.0 {
        return 0;
    }
    ((sample_rate * dt).round() as usize).min(AUDIO_SAMPLES)
}

/// Upload the 256x1 RGBA wave payload into a `DataTexture`-equivalent texture.
pub(crate) fn upload_wave_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    bytes: &[u8; WAVE_BYTES],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(WAVE_BYTES as u32),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: WAVE_N as u32,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

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

/// Test/headless audio fixture: a sine wave at `freq_hz`, delivered as the
/// analyser-sized [`BandPayload`] the live path consumes.
///
/// This stands in for the live analyser (`AudioBands` → [`BandPayload`] →
/// [`AudioBridge`]) without being on the production path: a caller that wants
/// audio without hardware asks [`SyntheticSine::fill_payload`] for a frame's
/// window and feeds the payload through a real bridge. Samples are generated
/// from an absolute sample counter at 48 000 Hz, mono (duplicated to both
/// channels in the real graph; the scenes read a single mono waveform),
/// amplitude 0.9.
///
/// [`BandPayload`]: crate::point10::audio_bridge::BandPayload
/// [`AudioBridge`]: crate::point10::audio_bridge::AudioBridge
pub struct SyntheticSine {
    freq_hz: f32,
    /// Absolute index of the newest generated sample.
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
    fn sample_at(&self, index: u64) -> f32 {
        let t = index as f32 / SAMPLE_RATE as f32;
        SINE_AMPLITUDE * (2.0 * std::f32::consts::PI * self.freq_hz * t).sin()
    }

    /// Fill `payload` with this frame's analyser window and advance the fixture
    /// clock by `round(rate * dt)` samples.
    ///
    /// The window is the 1024 samples ending at the new clock position, which
    /// is what the real analyser hands over (`p10Audio.js: this.wave =
    /// new Float32Array(1024)`), so a scene's `wave[wave.length - k ..]` tail
    /// append is exercised exactly as it is in production. The damping-2 lane
    /// (`[damping * 3 + band]`, i.e. lanes 6..=8) receives the fixture's band
    /// target, which is the lane `band(-1, 2, n)` reads.
    pub fn fill_payload(&mut self, payload: &mut BandPayload, dt_seconds: f32) {
        let advance = if dt_seconds.is_finite() && dt_seconds > 0.0 {
            ((SAMPLE_RATE as f32 * dt_seconds).round() as u64).min(AUDIO_SAMPLES as u64)
        } else {
            0
        };
        self.sample_index += advance;

        payload.sample_rate = SAMPLE_RATE as f32;
        let first = self.sample_index as i64 - AUDIO_SAMPLES as i64;
        for (index, sample) in payload.wave.iter_mut().enumerate() {
            let absolute = first + index as i64;
            // Samples before the fixture clock started read as silence, so the
            // first frames behave like an analyser that just opened.
            *sample = if absolute < 0 {
                0.0
            } else {
                self.sample_at(absolute as u64)
            };
        }

        let targets = self.band_targets();
        // Lane 0 is the raw band; lanes 6..=8 are damping lane 2.
        payload.lanes[..3].copy_from_slice(&targets);
        payload.lanes[6..9].copy_from_slice(&targets);
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
    /// `ChromaticLine.js: this.amp = 1` — scale on the band curve
    /// (`scene.ampScale`, bound from the `amplitude` prop).
    amp_scale: f32,
    /// `ChromaticLine.js: this.glowScale = 1` (line 81).
    glow_scale: f32,
    /// `ChromaticLine.js: this.distortion = 0.05` (line 79).
    distortion: f32,
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
            amp_scale: 1.0,
            glow_scale: 1.0,
            distortion: 0.05,
        }
    }

    /// `scene.ampScale = v.amplitude` (`sceneProps.js:369-373`).
    pub(crate) fn set_amp_scale(&mut self, amp_scale: f32) {
        self.amp_scale = amp_scale;
    }

    /// `scene.glowScale = v.glow` (`sceneProps.js:369-373`).
    pub(crate) fn set_glow_scale(&mut self, glow_scale: f32) {
        self.glow_scale = glow_scale;
    }

    /// `scene.distortion = v.distortion` (`sceneProps.js:369-373`).
    pub(crate) fn set_distortion(&mut self, distortion: f32) {
        self.distortion = distortion;
    }

    /// Append `samples` to the ring, wrapping at `RING_LEN`.
    ///
    /// Live path of `ChromaticLine.js update()` (lines 167-176):
    /// `const k = Math.min(Math.round(this.rate * dt), this.p9.wave.length);`
    /// then `ring[(this.head + m) % this.R] = wave[wave.length - k + m]` and
    /// `this.head = (this.head + k) % this.R`. The caller slices the tail
    /// (`&bridge.wave[AUDIO_SAMPLES - k ..]`) — that is the scene's job because
    /// only it knows `k`, and it is also how the fixture feeds samples in.
    pub(crate) fn append_wave_tail(&mut self, samples: &[f32]) {
        // Only the last RING_LEN samples can survive the wrap; a longer append
        // would overwrite the ring more than once and leave the same tail.
        let samples = if samples.len() > RING_LEN {
            &samples[samples.len() - RING_LEN..]
        } else {
            samples
        };
        if samples.is_empty() {
            return;
        }

        let first = (RING_LEN - self.head).min(samples.len());
        self.ring[self.head..self.head + first].copy_from_slice(&samples[..first]);
        self.ring[..samples.len() - first].copy_from_slice(&samples[first..]);
        self.head = (self.head + samples.len()) % RING_LEN;
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

    /// Map damped bands to the **line**'s band-driven uniforms.
    ///
    /// `ChromaticLine.js _audioUniforms` (lines 137-151):
    /// `amp[i] = min(b*10, 0.8) * ampScale`, `glow[i] = (0.05 + b*0.08) * glowScale`,
    /// `xScale[i] = 2.5 + b*0.8`, `distortion` never changes.
    ///
    /// `bands` is `P9Audio.band(-1, 2, n)` — the bridge's damping lane 2 — so
    /// the low-pass lives in the audio graph (`audio-bands.js: DAMPING_TAU`)
    /// and not in the scene.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::AudioBridge;

    #[test]
    fn sample_count_matches_the_js_cap() {
        // 60 fps at 44.1 kHz -> 735 samples; a 100 ms frame caps at the
        // 1024-sample analyser window.
        assert_eq!(sample_count(44_100.0, 1.0 / 60.0), 735);
        assert_eq!(sample_count(44_100.0, 0.1), 1024);
        assert_eq!(sample_count(44_100.0, 0.0), 0);
        assert_eq!(sample_count(44_100.0, f32::NAN), 0);
        assert_eq!(sample_count(0.0, 0.016), 0);
    }

    /// The fixture must deliver exactly the new samples each frame: with
    /// `k = round(rate * dt)` the bridge's wave tail is the continuation of the
    /// same 48 kHz sine, so a scene appending the tail sees a gap-free signal.
    #[test]
    fn synthetic_sine_fills_the_analyser_window_from_its_clock() {
        let mut sine = SyntheticSine::new(440.0);
        let mut payload = BandPayload::silent(44_100.0);
        sine.fill_payload(&mut payload, 1.0 / 60.0);

        assert_eq!(payload.sample_rate, SAMPLE_RATE as f32);
        // 800 new samples at 48 kHz; the first 224 read as silence.
        let new = 800;
        for index in 0..AUDIO_SAMPLES {
            let expected = if index < AUDIO_SAMPLES - new {
                0.0
            } else {
                sine.sample_at((index - (AUDIO_SAMPLES - new)) as u64)
            };
            assert!(
                (payload.wave[index] - expected).abs() < 1e-6,
                "sample {index}: {} != {expected}",
                payload.wave[index]
            );
        }
        // The damping-2 lane (band 1 for a 440 Hz tone) is the band read.
        assert_eq!(payload.lanes[0], 0.0);
        assert_eq!(payload.lanes[1], 0.5);
        assert_eq!(payload.lanes[7], 0.5);

        // Drive the payload through a real bridge: the fixture is on the same
        // path as live audio, so lane 2 reaches the scene.
        let mut bridge = AudioBridge::new();
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        assert!(bridge.is_available());
        assert_eq!(bridge.lanes[7], 0.5);
    }
}
