//! Shared helpers for the Point10 scene port (`point10/shared/`).
//!
//! Ported so far (the batch-1/2 dependency sets, see docs/rust-hybrid.md
//! "Port order rationale"):
//!
//! | JS source | Rust module | Used by |
//! |-----------|-------------|---------|
//! | `shared/particleAlpha.js`, `shared/particleSprite.js` | [`particle_alpha`] | ParticleField, SpiralParticles |
//! | `shared/random.js` | [`random`] | ParticleField, SpiralParticles (deterministic stand-ins for `Math.random`), every `PERMRAND` in batch 2 |
//! | `plane9/P9Glsl.js` (the helpers batch 2 calls) | [`p9_glsl`] | LightTunnel, Wormhole, ItIsComingForYou |
//!
//! `plane9/P9Audio.js` is deliberately **not** here: it is subsumed by
//! [`crate::point10::audio_bridge::P10Audio`] (see the module docs there).
//! `P9Glsl.js`'s GLSL preprocessor is replaced with [`p9_glsl`]'s WGSL subset
//! plus per-scene WGSL.
//!
//! Not ported yet — the later scene batches pull them in:
//!
//! * `shared/textures.js` / `shared/assets.js` — asset loading. The Rust
//!   renderer has no `fetch`/`fs` split: the engine hands textures to scenes
//!   as [`wgpu::Texture`] handles, so the loader belongs on the engine side.
//!   SquareTunnel (the only batch-2 scene with a texture) builds a procedural
//!   stand-in for `checkerboard_diffuse.png` instead; see
//!   [`crate::point10::scenes::square_tunnel`].
//! * `shared/phraseColor.js` (drop colour pulse / equalised hue) — needed by
//!   WaterRipple, Mycelium, AttractorParticles (scenes 10, 17, 21).
//! * `shared/waterWave.js` — needed by WaterRipple and AroundPlus (10, 18).

pub mod p9_glsl;
pub mod particle_alpha;
pub mod random;
