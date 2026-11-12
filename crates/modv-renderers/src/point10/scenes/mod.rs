//! Batch-1 Point10 scenes that live in their own modules.
//!
//! | Module | Scene | JS source |
//! |--------|-------|-----------|
//! | [`chromatic_wave`] | Chromatic Wave | `scenes/ChromaticWave.js` |
//! | [`particle_field`] | Particle Field | `scenes/ParticleField.js` |
//! | [`spiral_particles`] | Spiral Particles | `scenes/SpiralParticles.js` |
//! | [`waveform`] | Waveform | `scenes/Waveform.js` |
//!
//! ChromaticLine lives with its shared simulation in
//! [`crate::point10::chromatic`] ([`crate::point10::chromatic::line`]) and is
//! registered from there by [`crate::point10::all_scenes`].

pub mod chromatic_wave;
pub mod particle_field;
pub mod particle_quad;
pub mod spiral_particles;
pub mod waveform;
