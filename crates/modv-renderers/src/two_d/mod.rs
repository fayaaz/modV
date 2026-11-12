//! 2D renderer and canvas facade (M3 / WS-5).
//!
//! Port of `src/renderer/src/application/renderers/2d/` — the renderer
//! (`index.js`, 210 LOC) and the 15 hand-written 2D modules. See
//! `docs/rust-hybrid.md`, "WS-5 2D renderer and canvas facade".
//!
//! # Why a hand-written canvas facade
//!
//! The JS modules are `CanvasRenderingContext2D` programs. The measured
//! corpus uses exactly 22 operations (plan §"Canvas2D API surface actually
//! used"), with no gradients, patterns, beziers or `getImageData`. That is
//! small enough to implement on wgpu directly instead of pulling in
//! `skia-safe` (a second GPU backend and a large binary) or `vello` (a CPU
//! rasteriser that would reintroduce per-module readback).
//!
//! # Architecture
//!
//! | Piece | File | Responsibility |
//! |-------|------|----------------|
//! | [`canvas::Canvas`] | `canvas.rs` | RGBA8 canvas texture, command recording, GPU submission |
//! | [`path`] | `path.rs` | affine transforms, path flattening, fill tessellation, stroke expansion |
//! | [`text`] | `text.rs` | cosmic-text shaping + swash glyph rasterisation into an R8 mask |
//! | [`modules`] | `modules/` | the 15 ported modules, [`modules::all_modules`] registry |
//!
//! The module code looks synchronous, like the JS does: every draw call
//! appends to a host-side command list (fine-grained triangles, plus the
//! texture to sample and the clip shape). [`canvas::Canvas::submit`] then
//! uploads one vertex buffer and one clip-uniform buffer and encodes the
//! draws into as few render passes as possible. This keeps the ported module
//! code a direct transcription of the JS and removes GPU synchronisation
//! from the per-draw path.
//!
//! # Rasterisation strategy (documented choices)
//!
//! * **Paths live in canvas space.** Canvas 2D transforms a point by the CTM
//!   when the point is *added* (not when `fill`/`stroke` runs), so a path
//!   built inside `save()`/`translate()`/`restore()` survives the `restore`.
//!   This is load-bearing: `Polygon.js:137-150` builds its path inside
//!   `save()`/`translate()`/`rotate()` and calls `stroke()` *after*
//!   `restore()`. Every point is therefore transformed on the CPU at
//!   record time.
//! * **Arcs flatten to a polyline** with 32 segments per full turn (scaled by
//!   the sweep). At the radii the corpus uses (circles from a few pixels to a
//!   few hundred) 32 segments are visually indistinguishable from a smooth
//!   arc, and it keeps the path representation uniform for fill and stroke.
//!   A fixed count (rather than an adaptive tolerance) also keeps vertex
//!   counts deterministic for golden tests.
//! * **Fill** tessellates on the CPU: a circle subpath becomes a triangle fan
//!   from the centre; every other subpath is ear-clipped (concave polygons
//!   included). The corpus never relies on non-zero winding *across* subpaths
//!   (no holes), so subpaths are tessellated independently — documented
//!   limitation.
//! * **Stroke** expands the polyline into per-segment quads plus a small disc
//!   at interior joins (round joins). Caps are butt (canvas default). This
//!   approximates canvas' miter default; with the corpus' widths (1–20 px)
//!   and shapes (single segments, waveform polylines, circles) the
//!   difference is not observable.
//! * **Anti-aliasing** comes from 4× MSAA: all geometry is drawn into a
//!   reusable 4-sample target that is resolved into the canvas texture. A
//!   reusable snapshot texture lets modules sample the canvas itself
//!   (`Smear`, `Pixelate`, `GridStretch`, `Buffer`) without sampling a
//!   texture that is bound as the resolve target.
//! * **Colour space** is premultiplied RGBA8, matching WS-2's compositor
//!   contract (`modv-render/src/blend.rs`: "All internal textures are
//!   premultiplied RGBA"). Blending is `One / OneMinusSrcAlpha`; the canvas
//!   texture is un-premultiplied at the present boundary like every other
//!   engine texture. `fillRect([1.0, 0.0, 0.0, 0.5])` therefore reads back
//!   as `(128, 0, 0, 128)`, not `(255, 0, 0, 128)`.
//! * **Clip** accepts the path's circle subpath (implemented analytically in
//!   the fragment shader) or, for every other path, its axis-aligned
//!   bounding box (the plan's stated minimum). `X-Drips.js:81-88` clips to a
//!   circle, so the analytic circle path keeps its rounded notches. Canvas2D
//!   `clip()` intersects with the active region, so the rect and circle are
//!   accumulated (tighter rect, tightest circle) rather than replaced.
//! * **`clearRect`** erases with a transparent-black `Replace` draw, so draws
//!   recorded earlier in the same frame survive outside the rect and the
//!   active clip still applies.
//! * **Text** is shaped by cosmic-text and rasterised by swash into a
//!   straight-alpha R8 mask (one texture per `fill_text` call, owned by the
//!   command list until `submit`). `stroke_text` draws the same mask as a
//!   ring of stamps behind the fill, approximating canvas' stroked outline.
//!
//! # Text availability
//!
//! cosmic-text's `FontSystem` loads system fonts through fontdb. When the
//! database is empty (some CI containers), [`canvas::Canvas::fill_text`]
//! draws nothing and sets [`canvas::Canvas::text_unavailable`] so tests can
//! skip instead of failing.
//!
//! # Audio
//!
//! The engine (M2e) owns Meyda/audio analysis. This crate does not depend on
//! `modv-media`, so [`AudioFrame`] is a decoupled mirror of the analyser
//! payload: the AGC bands, the Meyda feature bag (`features.zcr`,
//! `features.rms`, `features.buffer` — `docs/rust-hybrid.md` WS-5 "Module
//! contract"), the Point10 damping lanes/cells, wave, FFT magnitudes and
//! sample rate (WS-9).

#![forbid(unsafe_code)]

pub mod canvas;
pub mod modules;
pub mod path;
pub mod text;

#[cfg(test)]
mod tests;

pub use canvas::{Canvas, ClipShape, TextAlign, TextBaseline};
pub use modules::{DrawContext, ModuleContext, TwoDModule, TwoDModuleMeta, TwoDProp, all_modules};

/// The full audio payload for modules and renderers.
///
/// This is the Rust mirror of the object `audio-bands.js::update` returns plus
/// the Meyda feature bag the renderer's audio pump forwards over the control
/// socket (`meyda/SET_BANDS`). It is decoupled from `modv-media` on purpose:
/// the renderer performs the analysis (`AudioBands` + Meyda) and the engine
/// only stores/forwards the result (WS-9).
///
/// Array lengths accept the renderer's wire shapes: the wave is 1024 samples
/// (the analyser's `fftSize`), `freq_mag` is 1024 linear FFT magnitudes, and
/// the byte spectrum/time-domain arrays are `AUDIO_BUFFER_SIZE` (512) values
/// padded to 1024. `byte_frequency_data` keeps the full 1024 slots so a
/// larger FFT can be forwarded without a type change.
#[derive(Clone, Debug)]
pub struct AudioFrame {
    /// AGC-normalised bass band, 0..1 (the dB-mapped `features.bass`).
    pub bass: f32,
    /// AGC-normalised mid band, 0..1.
    pub mid: f32,
    /// AGC-normalised treble band, 0..1.
    pub treble: f32,
    /// Beat envelope for the current frame, 0..1 (`features.p10.beat`).
    pub beat: f32,
    /// 1024 time-domain samples, -1..1 (the `features.buffer` analogue).
    pub wave: [f32; 1024],
    /// 1024 FFT bins, 0..255.
    pub byte_frequency_data: [u8; 1024],
    /// 12 damping lanes, `[damping * 3 + band]` (`audio-bands.js` payload).
    pub lanes: [f32; 12],
    /// The 16 log-spaced band cells (`audio-bands.js` payload).
    pub cells: [f32; 16],
    /// AGC-free twin of [`Self::lanes`] (`payload.raw.lanes`).
    pub raw_lanes: [f32; 12],
    /// AGC-free twin of [`Self::cells`] (`payload.raw.cells`).
    pub raw_cells: [f32; 16],
    /// 1024 linear FFT magnitudes (`payload.freqMag`).
    pub freq_mag: [f32; 1024],
    /// Analyser sample rate in Hz (fallback 44 100).
    pub sample_rate: f32,
    /// The Meyda feature bag subset the corpus reads.
    pub features: AudioFeatures,
}

impl Default for AudioFrame {
    fn default() -> Self {
        Self {
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            beat: 0.0,
            wave: [0.0; 1024],
            byte_frequency_data: [0; 1024],
            lanes: [0.0; 12],
            cells: [0.0; 16],
            raw_lanes: [0.0; 12],
            raw_cells: [0.0; 16],
            freq_mag: [0.0; 1024],
            sample_rate: 44_100.0,
            features: AudioFeatures::default(),
        }
    }
}

impl AudioFrame {
    /// A deterministic frame used by tests and by the gallery preview (so a
    /// module renders the same first frame everywhere).
    pub fn synthetic() -> Self {
        let mut wave = [0.0; 1024];
        for (index, sample) in wave.iter_mut().enumerate() {
            let t = index as f32 / 1024.0;
            *sample = (t * std::f32::consts::TAU * 8.0).sin() * 0.5;
        }
        let mut byte_frequency_data = [0u8; 1024];
        for (index, bin) in byte_frequency_data.iter_mut().enumerate() {
            let t = index as f32 / 1024.0;
            *bin = (((1.0 - t) * 255.0) * (0.5 + 0.5 * (t * 40.0).sin())) as u8;
        }
        Self {
            bass: 0.6,
            mid: 0.4,
            treble: 0.3,
            beat: 1.0,
            wave,
            byte_frequency_data,
            lanes: [0.6, 0.4, 0.3, 0.6, 0.4, 0.3, 0.6, 0.4, 0.3, 0.6, 0.4, 0.3],
            cells: [
                0.6, 0.6, 0.6, 0.6, 0.6, 0.4, 0.4, 0.4, 0.4, 0.4, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3,
            ],
            raw_lanes: [0.6, 0.4, 0.3, 0.6, 0.4, 0.3, 0.6, 0.4, 0.3, 0.6, 0.4, 0.3],
            raw_cells: [
                0.6, 0.6, 0.6, 0.6, 0.6, 0.4, 0.4, 0.4, 0.4, 0.4, 0.3, 0.3, 0.3, 0.3, 0.3, 0.3,
            ],
            freq_mag: [0.0; 1024],
            sample_rate: 44_100.0,
            features: AudioFeatures {
                rms: 0.5,
                zcr: 0.1,
                energy: 0.25,
                buffer: wave.to_vec(),
                ..AudioFeatures::default()
            },
        }
    }
}

/// Meyda feature bag subset used by the corpus (`features.zcr`, `.rms`,
/// `.energy`, `.buffer`) plus the spectral/perceptual scalars the engine
/// forwards for Input Links (WS-9).
#[derive(Clone, Debug)]
pub struct AudioFeatures {
    /// Root-mean-square amplitude, 0..1.
    pub rms: f32,
    /// Zero-crossing rate, 0..1.
    pub zcr: f32,
    /// Spectral energy, 0..1.
    pub energy: f32,
    /// Spectral centroid (Hz).
    pub spectral_centroid: f32,
    /// Spectral flatness, 0..1.
    pub spectral_flatness: f32,
    /// Spectral slope.
    pub spectral_slope: f32,
    /// Spectral rolloff (Hz).
    pub spectral_rolloff: f32,
    /// Spectral spread.
    pub spectral_spread: f32,
    /// Spectral skewness.
    pub spectral_skewness: f32,
    /// Spectral kurtosis.
    pub spectral_kurtosis: f32,
    /// Perceptual spread.
    pub perceptual_spread: f32,
    /// Perceptual sharpness.
    pub perceptual_sharpness: f32,
    /// 1024 time-domain samples (`features.buffer`; `Waveform.js:74` reads its
    /// length and values).
    pub buffer: Vec<f32>,
}

impl Default for AudioFeatures {
    fn default() -> Self {
        Self {
            rms: 0.0,
            zcr: 0.0,
            energy: 0.0,
            spectral_centroid: 0.0,
            spectral_flatness: 0.0,
            spectral_slope: 0.0,
            spectral_rolloff: 0.0,
            spectral_spread: 0.0,
            spectral_skewness: 0.0,
            spectral_kurtosis: 0.0,
            perceptual_spread: 0.0,
            perceptual_sharpness: 0.0,
            buffer: vec![0.0; 1024],
        }
    }
}
