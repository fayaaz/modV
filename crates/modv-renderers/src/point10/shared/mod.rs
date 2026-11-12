//! Shared helpers for the Point10 scene port (`point10/shared/`).
//!
//! Ported so far (the batch-1 dependency set, see docs/rust-hybrid.md
//! "Port order rationale"):
//!
//! | JS source | Rust module | Used by |
//! |-----------|-------------|---------|
//! | `shared/particleAlpha.js`, `shared/particleSprite.js` | [`particle_alpha`] | ParticleField, SpiralParticles |
//! | `shared/random.js` | [`random`] | ParticleField, SpiralParticles (deterministic stand-ins for `Math.random`) |
//!
//! Not ported yet — the later scene batches pull them in:
//!
//! * `shared/textures.js` / `shared/assets.js` — asset loading. The Rust
//!   renderer has no `fetch`/`fs` split: the engine hands textures to scenes
//!   as [`wgpu::Texture`] handles, so the loader belongs on the engine side.
//! * `shared/phraseColor.js` (drop colour pulse / equalised hue) — needed by
//!   WaterRipple, Mycelium, AttractorParticles (scenes 10, 17, 21).
//! * `shared/waterWave.js` — needed by WaterRipple and AroundPlus (10, 18).
//!
//! `plane9/P9Glsl.js` + `plane9/P9Audio.js` are deliberately **not** here:
//! `P9Audio` is subsumed by [`crate::point10::audio_bridge::P10Audio`] (see
//! the module docs there), and `P9Glsl` is a GLSL preprocessor that the port
//! replaces with per-scene WGSL.

pub mod particle_alpha;
pub mod random;
