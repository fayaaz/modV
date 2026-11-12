//! Point10 scenes that live in their own modules.
//!
//! | Module | Scene | JS source |
//! |--------|-------|-----------|
//! | [`chromatic_wave`] | Chromatic Wave | `scenes/ChromaticWave.js` |
//! | [`particle_field`] | Particle Field | `scenes/ParticleField.js` |
//! | [`spiral_particles`] | Spiral Particles | `scenes/SpiralParticles.js` |
//! | [`waveform`] | Waveform | `scenes/Waveform.js` |
//! | [`square_tunnel`] | Square Tunnel | `scenes/SquareTunnel.js` |
//! | [`light_tunnel`] | Light Tunnel | `scenes/LightTunnel.js` |
//! | [`wormhole`] | Wormhole | `scenes/Wormhole.js` |
//! | [`it_is_coming_for_you`] | It Is Coming For You | `scenes/ItIsComingForYou.js` |
//!
//! ChromaticLine lives with its shared simulation in
//! [`crate::point10::chromatic`] ([`crate::point10::chromatic::line`]) and is
//! registered from there by [`crate::point10::all_scenes`].
//!
//! [`particle_quad`] is not a scene: it is the instanced billboard pipeline
//! ParticleField and SpiralParticles share.

pub mod chromatic_wave;
pub mod it_is_coming_for_you;
pub mod light_tunnel;
pub mod particle_field;
pub mod particle_quad;
pub mod spiral_particles;
pub mod square_tunnel;
pub mod waveform;
pub mod wormhole;
