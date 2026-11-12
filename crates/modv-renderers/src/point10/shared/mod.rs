//! Shared helpers for the Point10 scene port (`point10/shared/`).
//!
//! Ported so far (the batch-1/2 dependency sets, see docs/rust-hybrid.md
//! "Port order rationale"):
//!
//! | JS source | Rust module | Used by |
//! |-----------|-------------|---------|
//! | `shared/particleAlpha.js`, `shared/particleSprite.js` | [`particle_alpha`] | ParticleField, SpiralParticles |
//! | `shared/random.js` | [`random`] | ParticleField, SpiralParticles (deterministic stand-ins for `Math.random`), every `PERMRAND` in batch 2, `pickRoygbivHue` in batch 3 |
//! | `plane9/P9Glsl.js` (the helpers batch 2 calls) | [`p9_glsl`] | LightTunnel, Wormhole, ItIsComingForYou, AroundPlus |
//! | `plane9/P9Glsl.js` (the lighting/colour half) | [`p9_shading`] | Boxy, ColorCube |
//! | `shared/phraseColor.js` | [`phrase_color`] | WaterRipple, Mycelium, AroundPlus |
//! | `shared/waterWave.js` | [`water_wave`] | WaterRipple, AroundPlus |
//! | `shared/textures.js` (the loader half) | [`assets`] | GlassLens, AroundPlus, Boxy, UltraHDR |
//!
//! `plane9/P9Audio.js` is deliberately **not** here: it is subsumed by
//! [`crate::point10::audio_bridge::P10Audio`] (see the module docs there).
//! `P9Glsl.js`'s GLSL preprocessor is replaced with [`p9_glsl`]'s WGSL subset
//! plus per-scene WGSL.
//!
//! `shared/assets.js` is not ported: no vendored scene imports it (UltraHDR
//! uses its own `UltraHDRLoader`).

pub mod assets;
pub mod p9_glsl;
pub mod p9_shading;
pub mod particle_alpha;
pub mod phrase_color;
pub mod random;
pub mod water_wave;
