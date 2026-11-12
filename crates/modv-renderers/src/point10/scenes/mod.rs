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
//! | [`water_ripple`] | Water Ripple | `scenes/WaterRipple.js` |
//! | [`glass_lens`] | Glass Lens | `scenes/GlassLens.js` |
//! | [`mycelium`] | Mycelium | `scenes/Mycelium.js` |
//! | [`around_plus`] | Around Plus | `scenes/AroundPlus.js` |
//! | [`hollow_cube`] | Hollow Cube | `scenes/HollowCube.js` |
//! | [`attractor_particles`] | Attractor Particles | `scenes/AttractorParticles.js` |
//! | [`dvd_room`] | DVD Room | `scenes/DvdRoom.js` |
//! | [`ultra_hdr`] | Ultra HDR | `scenes/UltraHDR.js` |
//! | [`mirror_cube`] | Mirror Cube | `scenes/MirrorCube.js` |
//! | [`color_cube`] | Color Cube | `scenes/ColorCube.js` |
//! | [`boxy`] | Boxy | `scenes/Boxy.js` |
//! | [`sound_cube`] | Sound Cube | `scenes/SoundCube.js` |
//! | [`geometric_tunnel`] | Geometric Tunnel | `scenes/GeometricTunnel.js` |
//! | [`rolling_fire`] | Rolling Fire / Ball Of Fire | `scenes/RollingFire.js` |
//! | [`smoke_room`] | Smoke Room | `scenes/SmokeRoom.js` |
//! | [`latent_backrooms`] | Latent Backrooms | `scenes/LatentBackrooms.js` |
//!
//! ChromaticLine lives with its shared simulation in
//! [`crate::point10::chromatic`] ([`crate::point10::chromatic::line`]) and is
//! registered from there by [`crate::point10::all_scenes`].
//!
//! [`dvd_logo`] is not a scene either: it builds the DVD wordmark mesh from the
//! inlined SVG path for [`dvd_room`], which — like [`smoke_room`] and
//! [`rolling_fire`] — renders through [`crate::point10::volume`] rather than an
//! approximation. [`rolling_fire`] also holds `BallOfFire`, the frozen variant.
//!
//! [`particle_quad`] is not a scene: it is the instanced billboard pipeline
//! ParticleField and SpiralParticles share. [`cube_mesh`] is the batch-4
//! counterpart — `BoxGeometry`, the matrix helpers and the depth attachment the
//! five cube/lighting scenes share.

pub mod around_plus;
pub mod attractor_particles;
pub mod boxy;
pub mod chromatic_wave;
pub mod color_cube;
pub mod cube_mesh;
pub mod dvd_logo;
pub mod dvd_room;
pub mod fire_common;
pub mod geometric_tunnel;
pub mod glass_lens;
pub mod hollow_cube;
pub mod it_is_coming_for_you;
pub mod latent_backrooms;
pub mod light_tunnel;
pub mod mirror_cube;
pub mod mycelium;
pub mod particle_field;
pub mod particle_quad;
pub mod rolling_fire;
pub mod smoke_room;
pub mod sound_cube;
pub mod spiral_particles;
pub mod square_tunnel;
pub mod ultra_hdr;
pub mod water_ripple;
pub mod waveform;
pub mod wormhole;
