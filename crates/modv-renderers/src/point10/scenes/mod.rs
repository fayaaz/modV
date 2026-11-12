//! Batch-1 Point10 scenes that live in their own modules.
//!
//! | Module | Scene | JS source |
//! |--------|-------|-----------|
//! | [`chromatic_wave`] | Chromatic Wave | `scenes/ChromaticWave.js` |
//! | [`particle_field`] | Particle Field | `scenes/ParticleField.js` |
//! | [`spiral_particles`] | Spiral Particles | `scenes/SpiralParticles.js` |
//! | [`waveform`] | Waveform | `scenes/Waveform.js` |
//!
//! ChromaticLine keeps its historical module ([`crate::point10::chromatic_line`])
//! and its shared simulation ([`crate::point10::chromatic`]).

pub mod chromatic_wave;
pub mod particle_field;
pub mod particle_quad;
pub mod spiral_particles;
pub mod waveform;
