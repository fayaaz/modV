//! Shader renderer — port of `src/renderer/src/application/renderers/shader/`.
//! See docs/rust-hybrid.md, "WS-3 Shader renderer".
//!
//! # What this module owns
//!
//! * The **uniform contract** — [`uniforms`]: the `ShaderUniforms` block
//!   (160 bytes, binding 0), the per-module prop buffer (binding 7), the
//!   fixed texture/sampler bindings 1–6 and 8–9, and the shared
//!   [`uniforms::SHARED_UNIFORM_WGSL`] preamble every module WGSL is built
//!   from.
//! * The **default shader flavours** — [`default_shader`]: the
//!   full-screen-triangle vertex shader and the gradient fallback, plus
//!   the mainImage wrapper template.
//! * The **module registry** — [`modules`]: the 37 built-in modules,
//!   ported from GLSL to WGSL. Sources are cited per module.
//! * **Pipeline creation & rendering** — [`pipeline`]: one fixed bind
//!   group layout, per-slot pipelines (live vs gallery), and the
//!   per-frame upload/encode/draw path.
//!
//! # Two flavours, as in the JS
//!
//! | Flavour | GLSL rule | WGSL port |
//! |---------|-----------|-----------|
//! | direct | writes `gl_FragColor` | module declares `fs_main` returning `@location(0) vec4<f32>` |
//! | mainImage shim | no `gl_FragColor` → `fWrap` injection | module declares `mainImage(fragCoord) -> vec4<f32>`; [`default_shader::DEFAULT_MAINIMAGE_WRAP`] calls it and forces `image.a = 1.0` |
//!
//! The JS rule lives in a comment at `index.js:140-144`
//! (`frag.search("gl_FragColor") < 0`); the port hard-codes the decision
//! per module instead of re-parsing the source at registration time.
//!
//! # Context split, translated
//!
//! The legacy renderer keeps **two** WebGL2 contexts — `shader-buffer`
//! (live) and `shader-buffer-gallery` (preview). wgpu has one device, so
//! isolation is expressed as **separate pipelines + separate prop buffers
//! per slot** ([`pipeline::ShaderRenderer::setup_module`] with
//! `is_gallery`), and the caller renders into separate target textures.
//! Live and gallery get distinct slots and distinct uniform/prop buffers.
//!
//! # Per-frame upload
//!
//! The legacy whole-canvas re-upload (`index.js:253-257`) disappears: the
//! group texture is already GPU-resident, so the per-frame work is two
//! `queue.write_buffer`s (160-byte shared block + prop slots) and one bind
//! group rebuild. No GPU↔CPU copies.
//!
//! # Not in this crate
//!
//! JSON → [`uniforms::PropSlots`] coercion (prop clamping, expression
//! application, tween resolution) belongs to the engine boundary
//! (modv-engine / WS-1, WS-2). This crate receives pre-encoded float
//! lanes in the module's declared prop order; the mapping from store
//! state to [`uniforms::ShaderUniforms`] (wall-clock `iDate`, dpr, meyda
//! bands, frame counter) is the engine's job too — see
//! [`pipeline::ShaderRenderer::render`]'s parameter list, which is the
//! exact information the worker must supply per frame.
//!
//! # Byte-drift ported faithfully (not fixed)
//!
//! `iTimeDelta` is not a delta; `u_time` is milliseconds while everything
//! else is seconds; `u_kick` travels as `u32`; `u_fftResolution` is a
//! scalar f32 (the doc table's `vec2` is wrong; the JS wins);
//! `iChannelTime` is declared but never uploaded. All are documented at
//! the declaration sites with the JS anchors.
//!
//! # Deliberate deviations from the GLSL
//!
//! * `DEFAULT_ES3_VERTEX` (`v300`) is not ported — WGSL has one vertex
//!   stage; [`default_shader::DEFAULT_VERTEX`] serves both flavours.
//! * `modv_uv` does **no** y-flip: GL's clip-space `vUv = (1-x, y)` folds
//!   to the same top-left-origin `fragCoord / resolution` wgpu produces.
//!   Golden-image comparison may revisit this (WS-15).
//! * `fragCoord` is top-left origin in wgpu; pixel-grid effects (Crt,
//!   Crosshatching, Bayer dithers) mirror rows relative to GL.
//! * `glsl_mod_*` exist because GLSL `mod` is floor-based; WGSL `%`
//!   truncates. See `uniforms::GLSL_MOD_F_WGSL`.
//!
//! # Test strategy
//!
//! The `#[cfg(test)]` tests in this module set run under plain `cargo test`
//! with naga as a dev-dependency: every module's final WGSL source is
//! **parsed and validated** (including naga's uniform-control-flow pass,
//! the exact check that would fail `create_shader_module` for the two
//! documented `textureSampleLevel` sites). Byte layout is pinned by
//! `size_of::<ShaderUniforms>() == 160` and by a naga-computed struct
//! span check.

#![forbid(unsafe_code)]

pub mod default_shader;
pub mod modules;
pub mod pipeline;
pub mod uniforms;

pub use modules::{ShaderModule, all_modules};
pub use pipeline::{ShaderAudioUniforms, ShaderError, ShaderRenderer};
