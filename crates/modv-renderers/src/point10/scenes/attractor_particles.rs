//! AttractorParticles — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/AttractorParticles.js`.
//!
//! The JS scene is **WebGPU/TSL**: three.js' `webgpu_tsl_compute_attractors_particles`
//! example (MIT) with 2^18 particles orbiting three gravity+spin attractors.
//! The music drives it instead of the example's `TransformControls`: bass
//! pushes `maxSpeed` up and the spinning strength hard, snare/hi-hat percussion
//! nudges the attractor mass exponent (6..8), overall energy drives
//! `particleGlobalMassExponent` (4..6), and the colourA/colourB ramp hue-rotates
//! on bass drops exactly like Ball of Fire. Three Ball-of-Fire lava balls sit on
//! the attractors with their own drop-pulse hues and the bloom.
//!
//! # Ported exactly
//!
//! * the force model (`AttractorParticles.js:475-519`): `gravityStrength =
//!   attractorMass * particleMass * 6.67e-11 / distance²`, `direction =
//!   normalize(attractor - position)`, the spin term
//!   `cross(axis * gravityStrength * spinningStrength, toAttractor)`,
//!   `velocity += force * 1/60`, the `maxSpeed` clamp, `* (1 - 0.1)` damping,
//!   `position += velocity * 1/60` and the `mod(…, 8) - 4` box loop;
//! * the initial state (`:445-464`): positions in a `5 x 0.2 x 5` box, a
//!   `sphericalToVec3(phi, theta) * 0.05` velocity and the per-particle mass
//!   multiplier `remap(hash, 0.25, 1)`;
//! * the particle colour (`:524-531`): `mix(hue(colorA, hueA), hue(colorB,
//!   hueB), smoothstep(0, 0.5, speed / maxSpeed))`;
//! * the sprite (`:533-539`): `scale = particleMassMultiplier * 0.008 *
//!   particleSize`, additive blending, `depthWrite: false`, and the round
//!   `1 - smoothstep(0.55, 1, |uv * 2 - 1|)` alpha;
//! * the lava balls (`:352-410`): `SphereGeometry(0.01, 64, 40)`, the moon
//!   albedo, the `AmbientLight(0.5)` + `DirectionalLight(1.5)` rig, the
//!   three-octave `mx_noise` lava ramp, `saturation(…, 1.1)`, `fireHue`'s
//!   luminance-equalised rotation, `lavaT.pow(2.5)`, `* 3.5 * (1.5 + bass * 5.3)`,
//!   and the per-ball drop-pulse hues starting at red/yellow/indigo;
//! * the camera orbit (`:586-596`): `azimuth += delta * 0.12`,
//!   `polar = π/2 + sin(time * 0.05) * 0.2`, radius around the attractor
//!   centroid, `lookAt(centroid)`; `delta = min(dt || 1/60, 1/30)`;
//! * the bloom (`:558-567`, `:578-580`): threshold 0.5, radius 1.0,
//!   `min(0.12 + bass * 3.6, 4) * brightness`;
//! * `audioUniforms` (`:95-110`) and the smoothing rates in `update`
//!   (`:162-237`): bass `dt*8` (5 bins), snare `dt*12` (2%..13% bins),
//!   hi-hat `dt*10` (top 15% bins), energy `dt*10` (all bins), and the two
//!   `dropColorPulse` ramps + three ball ramps.
//!
//! # Replaced TSL machinery — the compute passes
//!
//! `modv-renderers` has no compute path, so the two TSL compute passes are
//! replaced with the wgpu-idiomatic equivalent that runs on the same GPU state:
//!
//! | TSL | Rust |
//! |-----|------|
//! | `init().compute(count)` (position/velocity `instancedArray`) | a CPU seeding pass uploaded into two `Rgba32Float` state textures (`position.xyz` + mass multiplier in `w`, `velocity.xyz`) |
//! | `update().compute(count)` (ping-pong storage buffers) | a full-screen **fragment** pass over the state textures, MRT writing the next position/velocity textures |
//! | `positionBuffer.toAttribute()` / `velocityBuffer.toAttribute()` | `textureLoad` in the vertex shader (instance index → texel), same data |
//! | `InstancedMesh(PlaneGeometry(1,1), SpriteNodeMaterial)` | one unit-quad instance per particle, camera-facing in the vertex shader |
//!
//! That is the same motion for the same state: the simulation is one
//! fixed `1/60` step per rendered frame (the JS's `delta = float(1 / 60) *
//! timeScale`), and the fragment pass evaluates exactly the TSL arithmetic.
//! The seeding hash becomes [`seeded_rand`] (the JS uses `Math.random()`, so
//! the distribution — not the seed — is what the JS defines).
//!
//! # Deviations
//!
//! * **The lava balls are a 32x16 sphere, not 64x40.** Three of them at radius
//!   0.01 are ~2 px at 1080p; the segment count is invisible and the shading is
//!   the same.
//! * **`mx_noise_float` is `p9_noise`.** The Plane9 3D simplex noise stands in
//!   for MaterialX's value noise in the lava ramp (same role: smooth 3D noise
//!   in `0..1`); the three-octave blend, `pow(2.5)` and fire ramp are kept.
//! * **The final alpha is `clamp(max(rgb), 0, 1)`.** The JS leaves the output
//!   node's alpha to the additive chain; with `threshold: 0.5` an unlit pixel
//!   receives no bloom, so the two agree on the background, while the explicit
//!   rule guarantees the module's layer alpha survives (the same rule Smoke
//!   Room writes explicitly).
//! * `THREE.Clock`/`_dt`'s first-frame quirk (`this._dt || 1/60`) is kept: a
//!   zero delta advances 1/60 s, not 0.
//! * The camera `cameraDistance` prop is applied as `_camRadius` exactly like
//!   the JS binder (`sceneProps.js:349-360`).

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge};
use crate::point10::postfx::{Composer, ToneMapping, UnrealBloomPass};
use crate::point10::scenes::cube_mesh::{
    Mat4, Mesh, MeshGpu, Vertex, create_depth, depth_state, look_at, mat4_mul, normalise,
    perspective,
};
use crate::point10::shared::assets::{ColourSpace, Wrap, load_texture};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::particle_alpha::PARTICLE_SPRITE_INNER;
use crate::point10::shared::phrase_color::{
    DropColorState, ROYGBIV_HUES, drop_color_pulse, ease_phrase_hue, srgb_hex_to_linear,
};
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    UnitQuad, frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`AttractorParticles.js:54-84`)
// ---------------------------------------------------------------------------

/// `count = Math.pow(2, 18)`.
pub const PARTICLE_COUNT: u32 = 1 << 18;
/// `Math.sqrt(PARTICLE_COUNT)` — the state texture is square.
pub const STATE_SIZE: u32 = 512;

/// `BASS_BINS` — the first five bins are the bass band.
pub const BASS_BINS: usize = 5;

/// `MAX_SPEED_BASE`.
pub const MAX_SPEED_BASE: f32 = 8.0;
/// `MAX_SPEED_PER_BASS`.
pub const MAX_SPEED_PER_BASS: f32 = 6.0;
/// `MASS_EXP_MIN`.
pub const MASS_EXP_MIN: f32 = 6.0;
/// `MASS_EXP_RANGE`.
pub const MASS_EXP_RANGE: f32 = 2.0;
/// `PERCUSSION_K`.
pub const PERCUSSION_K: f32 = 1.5;
/// `MASS_EXP_PER_BASS`.
pub const MASS_EXP_PER_BASS: f32 = 0.5;
/// `SPIN_BASE`.
pub const SPIN_BASE: f32 = 2.75;
/// `SPIN_PER_BASS`.
pub const SPIN_PER_BASS: f32 = 10.0;
/// `SPIN_PER_ENERGY`.
pub const SPIN_PER_ENERGY: f32 = 2.0;
/// `PARTICLE_MASS_EXP_MIN`.
pub const PARTICLE_MASS_EXP_MIN: f32 = 4.0;
/// `PARTICLE_MASS_EXP_RANGE`.
pub const PARTICLE_MASS_EXP_RANGE: f32 = 2.0;

/// `BALL_RADIUS`.
pub const BALL_RADIUS: f32 = 0.01;
/// `BLOOM_BASE`.
pub const BLOOM_BASE: f32 = 0.12;
/// `BLOOM_PER_BASS`.
pub const BLOOM_PER_BASS: f32 = 3.6;
/// The JS clamps the bloom at 4.0.
pub const BLOOM_MAX: f32 = 4.0;
/// `FIRE_START` (0xffe68c).
pub const FIRE_START: u32 = 0x00ff_e68c;
/// `FIRE_MID` (0xff7305).
pub const FIRE_MID: u32 = 0x00ff_7305;
/// `FIRE_END` (0xff0000).
pub const FIRE_END: u32 = 0x00ff_0000;

/// `SEED_A`.
pub const SEED_A: u32 = 1;
/// `SEED_B`.
pub const SEED_B: u32 = 2;
/// `SEED_BALLS`.
pub const SEED_BALLS: [u32; 3] = [3, 4, 5];
/// `BALL_START_HUES` — red, yellow, indigo.
pub const BALL_START_HUES: [f32; 3] = [ROYGBIV_HUES[0], ROYGBIV_HUES[2], ROYGBIV_HUES[5]];

/// `gravityConstant = 6.67e-11`.
pub const GRAVITY_CONSTANT: f32 = 6.67e-11;
/// `velocityDamping = uniform(0.1)`.
pub const VELOCITY_DAMPING: f32 = 0.1;
/// `scale = uniform(0.008)`.
pub const PARTICLE_SCALE: f32 = 0.008;
/// `boundHalfExtent = uniform(8)`.
pub const BOUND_HALF_EXTENT: f32 = 8.0;

/// The three attractor positions (`uniformArray([...])`).
pub const ATTRACTORS: [[f32; 3]; 3] = [[-1.0, 0.0, 0.0], [1.0, 0.0, -0.5], [0.0, 0.5, 1.0]];
/// The three rotation axes (`uniformArray([...])`, the third normalised).
pub const ATTRACTOR_AXES: [[f32; 3]; 3] = [
    [0.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.894_427_2, 0.0, -0.447_213_6],
];

/// `CAMERA_FOV` (`PerspectiveCamera(25, …)`).
pub const CAMERA_FOV: f32 = 25.0;
/// `CAMERA_NEAR`.
pub const CAMERA_NEAR: f32 = 0.1;
/// `CAMERA_FAR`.
pub const CAMERA_FAR: f32 = 100.0;
/// The camera's initial position (`:296`).
pub const CAMERA_START: [f32; 3] = [3.0, 5.0, 8.0];

/// `colorA` (`uniform(color("#5900ff"))`).
pub const COLOUR_A_HEX: u32 = 0x0059_00ff;
/// `colorB` (`uniform(color("#ffa575"))`).
pub const COLOUR_B_HEX: u32 = 0x00ff_a575;

/// `SCENE_BACKGROUND = null` — the particle field is an overlay.
pub const CLEAR_ALPHA: f64 = 0.0;

/// The attractor centroid `(0, 1/6, 1/6)`.
pub fn camera_centre() -> [f32; 3] {
    let count = ATTRACTORS.len() as f32;
    [
        ATTRACTORS.iter().map(|p| p[0]).sum::<f32>() / count,
        ATTRACTORS.iter().map(|p| p[1]).sum::<f32>() / count,
        ATTRACTORS.iter().map(|p| p[2]).sum::<f32>() / count,
    ]
}

/// `_makeRand(seed)` — the per-pulse seeded stream (`seededRand(seed, n++)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeedStream {
    seed: u32,
    counter: u32,
}

impl SeedStream {
    /// A fresh stream.
    pub const fn new(seed: u32) -> Self {
        Self { seed, counter: 0 }
    }

    /// The next `[0,1)` sample; advances the counter like the JS closure.
    pub fn next_sample(&mut self) -> f32 {
        let sample = seeded_rand(self.seed as i32, self.counter as i32);
        self.counter = self.counter.wrapping_add(1);
        sample
    }

    /// The counter value the JS would have reached.
    pub fn counter(&self) -> u32 {
        self.counter
    }

    /// Advance the stream without consuming a sample (the JS increments `n`
    /// only when `pickRoygbivHue` actually draws).
    pub fn advance(&mut self) {
        self.counter = self.counter.wrapping_add(1);
    }
}

/// `audioUniforms(bass, snare, hi, energy)` (`AttractorParticles.js:95-110`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioUniforms {
    /// `maxSpeed`.
    pub max_speed: f32,
    /// `attractorMass`.
    pub attractor_mass: f32,
    /// `spinningStrength`.
    pub spinning_strength: f32,
    /// `particleGlobalMass`.
    pub particle_global_mass: f32,
}

/// Pure audio → uniform mapping.
pub fn audio_uniforms(bass: f32, snare: f32, hi: f32, energy: f32) -> AudioUniforms {
    let expo_frac = ((snare + hi) * PERCUSSION_K + bass * MASS_EXP_PER_BASS).min(1.0);

    AudioUniforms {
        max_speed: MAX_SPEED_BASE + bass * MAX_SPEED_PER_BASS,
        attractor_mass: 10.0_f32.powf(MASS_EXP_MIN + expo_frac * MASS_EXP_RANGE),
        spinning_strength: SPIN_BASE + bass * SPIN_PER_BASS + energy * SPIN_PER_ENERGY,
        particle_global_mass: 10.0_f32
            .powf(PARTICLE_MASS_EXP_MIN + energy * PARTICLE_MASS_EXP_RANGE),
    }
}

/// The four band means the JS computes from `audio.freqMag`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandMeans {
    /// Mean of the first five bins.
    pub bass: f32,
    /// Mean of the 2%..13% bins (the snare crack).
    pub snare: f32,
    /// Mean of the top 15% bins (hi-hats).
    pub hi: f32,
    /// Mean of every bin.
    pub energy: f32,
}

/// Pure band extraction (`AttractorParticles.js:169-193`).
pub fn band_means(freq_mag: &[f32; AUDIO_SAMPLES]) -> BandMeans {
    let n = AUDIO_SAMPLES;
    let mut bass = 0.0;
    for value in freq_mag.iter().take(BASS_BINS) {
        bass += value;
    }
    bass /= BASS_BINS as f32;

    let snare_lo = (n as f32 * 0.02).floor() as usize;
    let snare_hi = (n as f32 * 0.13).floor() as usize;
    let mut snare = 0.0;
    for value in freq_mag.iter().take(snare_hi).skip(snare_lo) {
        snare += value;
    }
    snare /= (snare_hi.max(snare_lo + 1) - snare_lo) as f32;

    let hi_start = n - (n as f32 * 0.15).floor() as usize;
    let mut hi = 0.0;
    for value in freq_mag.iter().skip(hi_start) {
        hi += value;
    }
    hi /= (n - hi_start) as f32;

    let mut energy = 0.0;
    for value in freq_mag.iter() {
        energy += value;
    }
    energy /= n as f32;

    BandMeans {
        bass,
        snare,
        hi,
        energy,
    }
}

/// The simulation constants of one step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimParams {
    /// `attractorMass`.
    pub attractor_mass: f32,
    /// `particleGlobalMass`.
    pub particle_global_mass: f32,
    /// `spinningStrength`.
    pub spinning_strength: f32,
    /// `maxSpeed`.
    pub max_speed: f32,
    /// `gravityConstant`.
    pub gravity: f32,
    /// `velocityDamping`.
    pub damping: f32,
    /// `boundHalfExtent`.
    pub bound_half_extent: f32,
}

impl SimParams {
    /// The silence defaults the JS uniforms start at (`:418-426`).
    pub fn silence() -> Self {
        Self {
            attractor_mass: 1e7,
            particle_global_mass: 1e4,
            spinning_strength: SPIN_BASE,
            max_speed: MAX_SPEED_BASE,
            gravity: GRAVITY_CONSTANT,
            damping: VELOCITY_DAMPING,
            bound_half_extent: BOUND_HALF_EXTENT,
        }
    }
}

/// One particle's seeded initial state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleSeed {
    /// World position.
    pub position: [f32; 3],
    /// World velocity.
    pub velocity: [f32; 3],
    /// `hash(...).remap(0.25, 1)`.
    pub mass_multiplier: f32,
}

/// `hash(a)` stand-in: `seeded_rand(index, salt)` (see the module docs).
fn init_hash(index: u32, salt: u32) -> f32 {
    seeded_rand(index as i32, salt as i32)
}

/// One particle's initial state, deterministic per index.
pub fn particle_seed(index: u32) -> ParticleSeed {
    let base = [
        (init_hash(index, 1) - 0.5) * 5.0,
        (init_hash(index, 2) - 0.5) * 0.2,
        (init_hash(index, 3) - 0.5) * 5.0,
    ];
    // `sphericalToVec3(phi, theta) * 0.05`: phi = hash * 2π, theta = hash * π.
    let phi = init_hash(index, 4) * std::f32::consts::TAU;
    let theta = init_hash(index, 5) * std::f32::consts::PI;
    let sin_phi = phi.sin();
    let velocity = [
        sin_phi * theta.sin() * 0.05,
        phi.cos() * 0.05,
        sin_phi * theta.cos() * 0.05,
    ];

    ParticleSeed {
        position: base,
        velocity,
        mass_multiplier: 0.25 + init_hash(index, 6) * 0.75,
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// GLSL/TSL `mod` for vec3 (`x - y * floor(x / y)`).
fn glsl_mod3(x: [f32; 3], y: [f32; 3]) -> [f32; 3] {
    [
        x[0] - y[0] * (x[0] / y[0]).floor(),
        x[1] - y[1] * (x[1] / y[1]).floor(),
        x[2] - y[2] * (x[2] / y[2]).floor(),
    ]
}

/// One simulation step — the CPU twin of the fragment pass, used by the tests.
pub fn step_particle(
    position: [f32; 3],
    velocity: [f32; 3],
    mass_multiplier: f32,
    params: &SimParams,
    delta: f32,
) -> ([f32; 3], [f32; 3]) {
    let particle_mass = mass_multiplier * params.particle_global_mass;
    let mut force = [0.0_f32; 3];

    for (attractor, axis) in ATTRACTORS.iter().zip(ATTRACTOR_AXES.iter()) {
        let to_attractor = [
            attractor[0] - position[0],
            attractor[1] - position[1],
            attractor[2] - position[2],
        ];
        let distance = length(to_attractor);
        let inverse = 1.0 / distance;
        let direction = [
            to_attractor[0] * inverse,
            to_attractor[1] * inverse,
            to_attractor[2] * inverse,
        ];
        let gravity_strength =
            params.attractor_mass * particle_mass * params.gravity / (distance * distance);
        for channel in 0..3 {
            force[channel] += direction[channel] * gravity_strength;
        }
        let spinning_force = [
            axis[0] * gravity_strength * params.spinning_strength,
            axis[1] * gravity_strength * params.spinning_strength,
            axis[2] * gravity_strength * params.spinning_strength,
        ];
        let spinning_velocity = cross(spinning_force, to_attractor);
        for channel in 0..3 {
            force[channel] += spinning_velocity[channel];
        }
    }

    let mut next_velocity = [
        velocity[0] + force[0] * delta,
        velocity[1] + force[1] * delta,
        velocity[2] + force[2] * delta,
    ];
    let speed = length(next_velocity);
    if speed > params.max_speed {
        let scale = params.max_speed / speed;
        for channel in 0..3 {
            next_velocity[channel] *= scale;
        }
    }
    let keep = 1.0 - params.damping;
    for channel in 0..3 {
        next_velocity[channel] *= keep;
    }

    let mut next_position = [
        position[0] + next_velocity[0] * delta,
        position[1] + next_velocity[1] * delta,
        position[2] + next_velocity[2] * delta,
    ];
    let half = params.bound_half_extent * 0.5;
    let wrapped = glsl_mod3(
        [
            next_position[0] + half,
            next_position[1] + half,
            next_position[2] + half,
        ],
        [params.bound_half_extent; 3],
    );
    next_position = [wrapped[0] - half, wrapped[1] - half, wrapped[2] - half];

    (next_position, next_velocity)
}

/// The camera orbit state (`_camAzimuth` / `_camPolar` / `_camRadius`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitState {
    /// Azimuth, radians.
    pub azimuth: f32,
    /// Polar angle, radians.
    pub polar: f32,
    /// Orbit radius.
    pub radius: f32,
}

impl OrbitState {
    /// Seed the orbit from the JS' starting camera (`:312-319`).
    pub fn from_start(radius: f32) -> Self {
        let centre = camera_centre();
        let dx = CAMERA_START[0] - centre[0];
        let dy = CAMERA_START[1] - centre[1];
        let dz = CAMERA_START[2] - centre[2];
        let distance = dot([dx, dy, dz], [dx, dy, dz]).sqrt();
        Self {
            azimuth: dz.atan2(dx),
            polar: (dy / distance).clamp(-1.0, 1.0).acos(),
            radius,
        }
    }

    /// `_animate`'s orbit advance and the resulting camera position.
    pub fn advance(&mut self, time: f32, delta: f32) -> [f32; 3] {
        self.azimuth += delta * 0.12;
        self.polar = std::f32::consts::FRAC_PI_2 + (time * 0.05).sin() * 0.2;
        let centre = camera_centre();
        let r = self.radius;
        [
            centre[0] + r * self.polar.sin() * self.azimuth.cos(),
            centre[1] + r * self.polar.cos(),
            centre[2] + r * self.polar.sin() * self.azimuth.sin(),
        ]
    }
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `ATTRACTOR_PARTICLES_PROPS` (`sceneProps.js:66-71`).
const ATTRACTOR_PROPS: &[SceneProp] = &[
    SceneProp::float("cameraDistance", "Camera Distance", 9.9, 4.0, 30.0, 0.1),
    SceneProp::float("particleSize", "Particle Size", 1.0, 0.1, 4.0, 0.01),
    SceneProp::float("brightness", "Particle Brightness", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("maxSpeed", "Max Speed", 1.0, 0.25, 3.0, 0.01),
];

/// The simulation pass uniform block (128 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SimUniforms {
    attractor_mass: f32,
    particle_global_mass: f32,
    spinning_strength: f32,
    max_speed: f32,
    attractors: [[f32; 4]; 3],
    axes: [[f32; 4]; 3],
    gravity: f32,
    damping: f32,
    delta: f32,
    bound_half_extent: f32,
}

/// The particle billboard uniform block (144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ParticleUniforms {
    view_proj: Mat4,
    camera_right: [f32; 4],
    camera_up: [f32; 4],
    resolution: [f32; 2],
    scale: f32,
    max_speed: f32,
    colour_a: [f32; 4],
    colour_b: [f32; 4],
}

/// The lava-ball uniform block (144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BallUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    fire_start: [f32; 4],
    fire_mid: [f32; 4],
    fire_end: [f32; 4],
    /// x time, y bass glow.
    params: [f32; 4],
}

/// The final composite uniform block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FinalUniforms {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// `BallInstance` — position + the ball's drop-pulse hue.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BallInstance {
    position: [f32; 3],
    hue: f32,
}

impl BallInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 12,
                    shader_location: 5,
                },
            ],
        }
    }
}

/// A texture plus its view.
struct TextureTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl TextureTarget {
    fn new(
        device: &wgpu::Device,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

/// GPU resources built in `init`.
struct AttractorGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    // state ping-pong
    positions: [TextureTarget; 2],
    velocities: [TextureTarget; 2],
    sim_uniforms: wgpu::Buffer,
    sim_pipeline: wgpu::RenderPipeline,
    sim_bind: [wgpu::BindGroup; 2],
    particle_pipeline: wgpu::RenderPipeline,
    particle_uniforms: wgpu::Buffer,
    particle_bind: [wgpu::BindGroup; 2],
    ball_pipeline: wgpu::RenderPipeline,
    ball_uniforms: wgpu::Buffer,
    ball_bind_group: wgpu::BindGroup,
    ball_instances: wgpu::Buffer,
    _ball_texture: wgpu::Texture,
    mesh: MeshGpu,
    quad: UnitQuad,
    scene_target: TextureTarget,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    bloom: UnrealBloomPass,
    final_pipeline: wgpu::RenderPipeline,
    final_uniforms: wgpu::Buffer,
    final_bind_group: wgpu::BindGroup,
    final_layout: wgpu::BindGroupLayout,
    final_sampler: wgpu::Sampler,
    composer: Composer,
    read: usize,
}

/// AttractorParticles scene.
pub struct AttractorParticles {
    /// Smoothed bass band energy.
    bass: f32,
    /// Smoothed snare band energy.
    snare: f32,
    /// Smoothed hi-hat band energy.
    hi: f32,
    /// Smoothed overall spectrum energy.
    energy: f32,
    /// Two ROYGBIV drop-pulses (one per colour-ramp end).
    pulse_a: DropColorState,
    /// Second ramp end.
    pulse_b: DropColorState,
    /// The three lava balls' pulses.
    pulse_balls: [DropColorState; 3],
    /// Seeded streams behind each pulse.
    seeds: [SeedStream; 5],
    /// `uHueA`.
    hue_a: f32,
    /// `uHueB`.
    hue_b: f32,
    /// `uBallHues`.
    ball_hues: [f32; 3],
    /// Orbit state (`_camAzimuth`/`_camPolar`/`_camRadius`).
    orbit: OrbitState,
    /// `_uTime`.
    time: f32,
    /// `audioUniforms` as last applied.
    uniforms: AudioUniforms,
    /// `_particleSize` prop.
    particle_size: f32,
    /// `_bloomScale` prop.
    bloom_scale: f32,
    /// `_speedScale` prop.
    speed_scale: f32,
    /// `_camRadius` prop.
    camera_distance: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<AttractorGpu>,
}

impl Default for AttractorParticles {
    fn default() -> Self {
        Self::new()
    }
}

impl AttractorParticles {
    /// Create the scene (`AttractorParticles.js:112-153` defaults).
    pub fn new() -> Self {
        Self {
            bass: 0.0,
            snare: 0.0,
            hi: 0.0,
            energy: 0.0,
            pulse_a: DropColorState::new(),
            pulse_b: DropColorState::new(),
            pulse_balls: [DropColorState::new(); 3],
            seeds: [
                SeedStream::new(SEED_A),
                SeedStream::new(SEED_B),
                SeedStream::new(SEED_BALLS[0]),
                SeedStream::new(SEED_BALLS[1]),
                SeedStream::new(SEED_BALLS[2]),
            ],
            hue_a: 0.0,
            hue_b: 0.0,
            ball_hues: BALL_START_HUES,
            orbit: OrbitState::from_start(9.9),
            time: 0.0,
            // The JS uniform initialisers (`:418-426`), used until the first
            // audio payload arrives: `attractorMass = 1e7`, not the
            // `audioUniforms(0,0,0,0)` value of 1e6.
            uniforms: AudioUniforms {
                max_speed: MAX_SPEED_BASE,
                attractor_mass: 1e7,
                spinning_strength: SPIN_BASE,
                particle_global_mass: 1e4,
            },
            particle_size: 1.0,
            bloom_scale: 1.0,
            speed_scale: 1.0,
            camera_distance: 9.9,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:349-360`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(ATTRACTOR_PROPS, props) {
            match key {
                "cameraDistance" => {
                    self.camera_distance = value;
                    self.orbit.radius = value;
                }
                "particleSize" => self.particle_size = value,
                "brightness" => self.bloom_scale = value,
                "maxSpeed" => self.speed_scale = value,
                _ => {}
            }
        }
    }

    /// Smoothed bass.
    pub fn bass(&self) -> f32 {
        self.bass
    }

    /// The eased `colorA` hue rotation.
    pub fn hue_a(&self) -> f32 {
        self.hue_a
    }

    /// The eased `colorB` hue rotation.
    pub fn hue_b(&self) -> f32 {
        self.hue_b
    }

    /// The three lava-ball hue rotations.
    pub fn ball_hues(&self) -> [f32; 3] {
        self.ball_hues
    }

    /// `audioUniforms` as last applied.
    pub fn uniforms(&self) -> AudioUniforms {
        self.uniforms
    }

    /// `_uTime`.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// The bloom strength the JS writes each frame (`:578-580`).
    pub fn bloom_strength(&self) -> f32 {
        ((BLOOM_BASE + self.bass * BLOOM_PER_BASS).min(BLOOM_MAX)) * self.bloom_scale
    }

    fn camera_position(&self) -> [f32; 3] {
        let centre = camera_centre();
        let r = self.orbit.radius;
        [
            centre[0] + r * self.orbit.polar.sin() * self.orbit.azimuth.cos(),
            centre[1] + r * self.orbit.polar.cos(),
            centre[2] + r * self.orbit.polar.sin() * self.orbit.azimuth.sin(),
        ]
    }

    /// View-projection plus the camera-facing billboard basis.
    fn camera(&self) -> (Mat4, [f32; 3], [f32; 3], [f32; 3]) {
        let position = self.camera_position();
        let centre = camera_centre();
        let up = [0.0, 1.0, 0.0];
        // `camera.lookAt(centre)` with three's default up.
        let back = normalise([
            position[0] - centre[0],
            position[1] - centre[1],
            position[2] - centre[2],
        ]);
        let right = normalise(cross(up, back));
        let camera_up = cross(back, right);
        let view = look_at(position, centre, up);
        let view_proj = mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &view,
        );
        (view_proj, right, camera_up, position)
    }

    fn sim_params(&self) -> SimParams {
        SimParams {
            attractor_mass: self.uniforms.attractor_mass,
            particle_global_mass: self.uniforms.particle_global_mass,
            spinning_strength: self.uniforms.spinning_strength,
            max_speed: self.uniforms.max_speed * self.speed_scale,
            gravity: GRAVITY_CONSTANT,
            damping: VELOCITY_DAMPING,
            bound_half_extent: BOUND_HALF_EXTENT,
        }
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (view_proj, right, up, position) = self.camera();
        let params = self.sim_params();

        gpu.queue.write_buffer(
            &gpu.sim_uniforms,
            0,
            bytemuck::bytes_of(&SimUniforms {
                attractor_mass: params.attractor_mass,
                particle_global_mass: params.particle_global_mass,
                spinning_strength: params.spinning_strength,
                max_speed: params.max_speed,
                attractors: ATTRACTORS.map(|p| [p[0], p[1], p[2], 0.0]),
                axes: ATTRACTOR_AXES.map(|p| [p[0], p[1], p[2], 0.0]),
                gravity: params.gravity,
                damping: params.damping,
                delta: 1.0 / 60.0,
                bound_half_extent: params.bound_half_extent,
            }),
        );
        let colour_a = crate::point10::shared::phrase_color::hue_rotate(
            srgb_hex_to_linear(COLOUR_A_HEX),
            self.hue_a,
        );
        let colour_b = crate::point10::shared::phrase_color::hue_rotate(
            srgb_hex_to_linear(COLOUR_B_HEX),
            self.hue_b,
        );
        gpu.queue.write_buffer(
            &gpu.particle_uniforms,
            0,
            bytemuck::bytes_of(&ParticleUniforms {
                view_proj,
                camera_right: [right[0], right[1], right[2], 0.0],
                camera_up: [up[0], up[1], up[2], 0.0],
                resolution: [self.size.0 as f32, self.size.1 as f32],
                scale: PARTICLE_SCALE * self.particle_size,
                max_speed: params.max_speed,
                colour_a: [colour_a[0], colour_a[1], colour_a[2], 1.0],
                colour_b: [colour_b[0], colour_b[1], colour_b[2], 1.0],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.ball_uniforms,
            0,
            bytemuck::bytes_of(&BallUniforms {
                view_proj,
                camera_pos: [position[0], position[1], position[2], 1.0],
                fire_start: linear_rgba(FIRE_START),
                fire_mid: linear_rgba(FIRE_MID),
                fire_end: linear_rgba(FIRE_END),
                params: [self.time, self.bass, 0.0, 0.0],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.final_uniforms,
            0,
            bytemuck::bytes_of(&FinalUniforms {
                resolution: [self.size.0 as f32, self.size.1 as f32],
                _pad: [0.0; 2],
            }),
        );
        let hue_instances: Vec<BallInstance> = (0..3)
            .map(|index| BallInstance {
                position: ATTRACTORS[index],
                hue: self.ball_hues[index],
            })
            .collect();
        gpu.queue
            .write_buffer(&gpu.ball_instances, 0, bytemuck::cast_slice(&hue_instances));
    }

    /// Seed the state textures with [`particle_seed`].
    fn seed_state(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let mut positions = vec![0.0_f32; (STATE_SIZE * STATE_SIZE * 4) as usize];
        let mut velocities = vec![0.0_f32; (STATE_SIZE * STATE_SIZE * 4) as usize];

        for index in 0..PARTICLE_COUNT {
            let seed = particle_seed(index);
            let at = index as usize * 4;
            positions[at] = seed.position[0];
            positions[at + 1] = seed.position[1];
            positions[at + 2] = seed.position[2];
            positions[at + 3] = seed.mass_multiplier;
            velocities[at] = seed.velocity[0];
            velocities[at + 1] = seed.velocity[1];
            velocities[at + 2] = seed.velocity[2];
        }

        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(STATE_SIZE * 16),
            rows_per_image: Some(STATE_SIZE),
        };
        for (texture, data) in [
            (&gpu.positions[0], &positions),
            (&gpu.velocities[0], &velocities),
        ] {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(data),
                layout,
                wgpu::Extent3d {
                    width: STATE_SIZE,
                    height: STATE_SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Recreate the size-dependent targets and re-bind the final pass.
    fn rebuild_targets(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.scene_target = TextureTarget::new(
            &gpu.device,
            "point10-attractor-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) = create_depth(
            &gpu.device,
            "point10-attractor-depth",
            self.size.0,
            self.size.1,
        );
        gpu._depth = depth;
        gpu.depth_view = depth_view;
        gpu.bloom.resize(self.size.0, self.size.1);
        gpu.bloom.set_input(&gpu.scene_target.view);
        gpu.composer.resize(self.size.0, self.size.1);
        gpu.final_bind_group = create_final_bind_group(&gpu.device, gpu);
        self.upload_uniforms();
    }
}

/// `new THREE.Color(hex)` → linear RGBA.
fn linear_rgba(hex: u32) -> [f32; 4] {
    let colour = srgb_hex_to_linear(hex);
    [colour[0], colour[1], colour[2], 1.0]
}

impl Point10Scene for AttractorParticles {
    fn name(&self) -> &'static str {
        "Point10 Attractor Particles"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);
        self.camera_aspect = ctx.aspect();

        let loader = ctx.asset_loader();
        let moon = load_texture(
            &device,
            &queue,
            &loader,
            "moon_04_diff_1k.jpg",
            ColourSpace::Srgb,
            Wrap::Clamp,
        );

        // --- state textures ------------------------------------------------
        let state_usage = wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST;
        let positions = [
            TextureTarget::new(
                &device,
                "point10-attractor-positions-a",
                STATE_SIZE,
                STATE_SIZE,
                wgpu::TextureFormat::Rgba32Float,
                state_usage,
            ),
            TextureTarget::new(
                &device,
                "point10-attractor-positions-b",
                STATE_SIZE,
                STATE_SIZE,
                wgpu::TextureFormat::Rgba32Float,
                state_usage,
            ),
        ];
        let velocities = [
            TextureTarget::new(
                &device,
                "point10-attractor-velocities-a",
                STATE_SIZE,
                STATE_SIZE,
                wgpu::TextureFormat::Rgba32Float,
                state_usage,
            ),
            TextureTarget::new(
                &device,
                "point10-attractor-velocities-b",
                STATE_SIZE,
                STATE_SIZE,
                wgpu::TextureFormat::Rgba32Float,
                state_usage,
            ),
        ];

        // --- simulation pass (the compute replacement) ---------------------
        let sim_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-attractor-sim-shader"),
            source: wgpu::ShaderSource::Wgsl(attractor_sim_wgsl().into()),
        });
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-attractor-sim-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                raw_texture_entry(1),
                raw_texture_entry(2),
            ],
        });
        let sim_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-attractor-sim-uniforms"),
            size: std::mem::size_of::<SimUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let make_sim_bind = |read: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("point10-attractor-sim-bind-group"),
                layout: &sim_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: sim_uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&positions[read].view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&velocities[read].view),
                    },
                ],
            })
        };
        let sim_bind = [make_sim_bind(0), make_sim_bind(1)];
        let sim_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-attractor-sim-pipeline-layout"),
            bind_group_layouts: &[Some(&sim_layout)],
            immediate_size: 0,
        });
        let sim_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-attractor-sim-pipeline"),
            layout: Some(&sim_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &sim_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sim_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                // MRT: position and velocity state textures.
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            multiview_mask: None,
            cache: None,
        });

        // --- particle billboards ------------------------------------------
        let particle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-attractor-particle-shader"),
            source: wgpu::ShaderSource::Wgsl(attractor_particle_wgsl().into()),
        });
        let particle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-attractor-particle-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                raw_texture_entry(1),
                raw_texture_entry(2),
            ],
        });
        let particle_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-attractor-particle-uniforms"),
            size: std::mem::size_of::<ParticleUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let make_particle_bind = |read: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("point10-attractor-particle-bind-group"),
                layout: &particle_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: particle_uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&positions[read].view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&velocities[read].view),
                    },
                ],
            })
        };
        let particle_bind = [make_particle_bind(0), make_particle_bind(1)];
        let particle_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-attractor-particle-pipeline-layout"),
                bind_group_layouts: &[Some(&particle_layout)],
                immediate_size: 0,
            });
        let quad = UnitQuad::new(&device);
        let particle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-attractor-particle-pipeline"),
            layout: Some(&particle_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &particle_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(UnitQuad::vertex_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::point10::scenes::cube_mesh::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &particle_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    // `THREE.AdditiveBlending`: srcAlpha * src + 1 * dst.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // --- lava balls ----------------------------------------------------
        let ball_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-attractor-ball-shader"),
            source: wgpu::ShaderSource::Wgsl(attractor_ball_wgsl().into()),
        });
        let ball_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-attractor-ball-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let ball_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-attractor-ball-uniforms"),
            size: std::mem::size_of::<BallUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ball_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-attractor-ball-bind-group"),
            layout: &ball_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ball_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&moon.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&moon.sampler),
                },
            ],
        });
        let ball_instances = {
            use wgpu::util::DeviceExt;
            let instances: Vec<BallInstance> = (0..3)
                .map(|index| BallInstance {
                    position: ATTRACTORS[index],
                    hue: BALL_START_HUES[index],
                })
                .collect();
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-attractor-ball-instances"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };
        let ball_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-attractor-ball-pipeline-layout"),
            bind_group_layouts: &[Some(&ball_layout)],
            immediate_size: 0,
        });
        let ball_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-attractor-ball-pipeline"),
            layout: Some(&ball_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &ball_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout()), Some(BallInstance::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &ball_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // --- scene target, bloom and final composite -----------------------
        let scene_target = TextureTarget::new(
            &device,
            "point10-attractor-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) =
            create_depth(&device, "point10-attractor-depth", self.size.0, self.size.1);

        let mut bloom = UnrealBloomPass::new(&device, &queue, self.size.0, self.size.1);
        bloom.set_strength(BLOOM_BASE);
        bloom.set_radius(1.0);
        bloom.set_threshold(0.5);
        bloom.set_input(&scene_target.view);

        let final_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-attractor-final-shader"),
            source: wgpu::ShaderSource::Wgsl(attractor_final_wgsl().into()),
        });
        let final_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-attractor-final-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let final_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-attractor-final-uniforms"),
            size: std::mem::size_of::<FinalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let final_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-attractor-final-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let final_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-attractor-final-pipeline",
            &final_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-attractor-final-pipeline-layout"),
                bind_group_layouts: &[Some(&final_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );
        let final_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-attractor-final-bind-group"),
            layout: &final_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: final_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scene_target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        bloom
                            .composite_view()
                            .expect("the bloom pass always has a composite target"),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&final_sampler),
                },
            ],
        });

        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        let mesh = MeshGpu::new(
            &device,
            "point10-attractor-ball-mesh",
            &build_sphere(BALL_RADIUS, 32, 16),
        );

        self.gpu = Some(AttractorGpu {
            device,
            queue,
            positions,
            velocities,
            sim_uniforms,
            sim_pipeline,
            sim_bind,
            particle_pipeline,
            particle_uniforms,
            particle_bind,
            ball_pipeline,
            ball_uniforms,
            ball_bind_group,
            ball_instances,
            _ball_texture: moon.texture,
            mesh,
            quad,
            scene_target,
            _depth: depth,
            depth_view,
            bloom,
            final_pipeline,
            final_uniforms,
            final_bind_group,
            final_layout,
            final_sampler,
            composer,
            read: 0,
        });
        self.seed_state();
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        self.rebuild_targets();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);

        if audio.is_available() {
            let bands = band_means(&audio.freq_mag);
            let smooth = |current: f32, target: f32, rate: f32| {
                current + (target - current) * (dt * rate).min(1.0)
            };
            self.bass = smooth(self.bass, bands.bass, 8.0);
            self.snare = smooth(self.snare, bands.snare, 12.0);
            self.hi = smooth(self.hi, bands.hi, 10.0);
            self.energy = smooth(self.energy, bands.energy, 10.0);

            self.uniforms = audio_uniforms(self.bass, self.snare, self.hi, self.energy);

            // The JS closure only draws a sample when the detector fires.
            let sample_a = seeded_rand(SEED_A as i32, self.seeds[0].counter() as i32);
            if drop_color_pulse(&mut self.pulse_a, self.bass, dt, sample_a) {
                self.seeds[0].advance();
            }
            let sample_b = seeded_rand(SEED_B as i32, self.seeds[1].counter() as i32);
            if drop_color_pulse(&mut self.pulse_b, self.bass, dt, sample_b) {
                self.seeds[1].advance();
            }
            self.hue_a = ease_phrase_hue(self.hue_a, self.pulse_a.hue, dt, 1.2);
            self.hue_b = ease_phrase_hue(self.hue_b, self.pulse_b.hue, dt, 1.2);

            for ball in 0..3 {
                let sample = seeded_rand(
                    SEED_BALLS[ball] as i32,
                    self.seeds[2 + ball].counter() as i32,
                );
                if drop_color_pulse(&mut self.pulse_balls[ball], self.bass, dt, sample) {
                    self.seeds[2 + ball].advance();
                }
                self.ball_hues[ball] =
                    ease_phrase_hue(self.ball_hues[ball], self.pulse_balls[ball].hue, dt, 1.2);
            }
        }

        // `_animate`: `delta = Math.min(this._dt || 1/60, 1/30)`.
        let delta = if dt > 0.0 {
            dt.min(1.0 / 30.0)
        } else {
            1.0 / 60.0
        };
        self.time += delta;
        self.orbit.advance(self.time, delta);
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let bloom_strength = self.bloom_strength();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let read = gpu.read;
        let write = 1 - read;

        // `renderer.compute(updateCompute)` — one fixed 1/60 step.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-attractor-sim"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &gpu.positions[write].view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &gpu.velocities[write].view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.sim_pipeline);
            pass.set_bind_group(0, &gpu.sim_bind[read], &[]);
            pass.draw(0..3, 0..1);
        }

        // `scenePass`: lava balls (opaque, depth) then the additive particles.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-attractor-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.scene_target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.ball_pipeline);
            pass.set_bind_group(0, &gpu.ball_bind_group, &[]);
            pass.set_vertex_buffer(1, gpu.ball_instances.slice(..));
            gpu.mesh.draw_instanced(&mut pass, 3);

            pass.set_pipeline(&gpu.particle_pipeline);
            pass.set_bind_group(0, &gpu.particle_bind[write], &[]);
            gpu.quad.draw_instanced(&mut pass, PARTICLE_COUNT);
        }

        // The bright pass is bound to the scene target in `init` and after
        // every resize (`rebuild_targets`).
        gpu.bloom.set_strength(bloom_strength);
        gpu.bloom.encode(encoder);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-attractor-final"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gpu.composer.scene_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.final_pipeline);
            pass.set_bind_group(0, &gpu.final_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.composer.render(encoder, target);
        gpu.read = write;
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(ATTRACTOR_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10AttractorParticles.js` sets `webgpu: true` and no category.
        SceneMeta::for_scene("Point10 Attractor Particles", "3d-scenes")
    }
}

/// Rebuild the final-composite bind group after its targets change.
fn create_final_bind_group(device: &wgpu::Device, gpu: &AttractorGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-attractor-final-bind-group"),
        layout: &gpu.final_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: gpu.final_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&gpu.scene_target.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    gpu.bloom
                        .composite_view()
                        .expect("the bloom pass always has a composite target"),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&gpu.final_sampler),
            },
        ],
    })
}

/// A `THREE.SphereGeometry(radius, widthSegments, heightSegments)` twin:
/// lat-long sphere with outward normals and 0..1 uvs.
pub fn build_sphere(radius: f32, width_segments: u32, height_segments: u32) -> Mesh {
    let width_segments = width_segments.max(2);
    let height_segments = height_segments.max(2);
    let mut mesh = Mesh::default();
    let cols = width_segments + 1;
    let rows = height_segments + 1;

    for iy in 0..rows {
        let v = iy as f32 / height_segments as f32;
        let phi = v * std::f32::consts::PI;
        let sin_phi = phi.sin();
        let cos_phi = phi.cos();
        for ix in 0..cols {
            let u = ix as f32 / width_segments as f32;
            let theta = u * std::f32::consts::TAU;
            let normal = [sin_phi * theta.cos(), cos_phi, sin_phi * theta.sin()];
            mesh.vertices.push(Vertex {
                position: [normal[0] * radius, normal[1] * radius, normal[2] * radius],
                normal,
                uv: [u, 1.0 - v],
            });
        }
    }

    for iy in 0..height_segments {
        for ix in 0..width_segments {
            let a = iy * cols + ix;
            let b = a + cols;
            let c = b + 1;
            let d = a + 1;
            mesh.indices.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }

    mesh
}

/// A uniform binding entry.
fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A `textureLoad`-only texture entry (`Rgba32Float` is not filterable).
fn raw_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A sampled-texture binding entry.
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A filtering-sampler binding entry.
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// The simulation fragment pass — the TSL `update().compute(count)` twin.
fn attractor_sim_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{ATTRACTOR_SIM_WGSL}")
}

/// The instanced billboard pass (`SpriteNodeMaterial` + the two attribute
/// buffers).
fn attractor_particle_wgsl() -> String {
    ATTRACTOR_PARTICLE_WGSL.replace("$SPRITE_INNER", &format!("{PARTICLE_SPRITE_INNER}"))
}

/// The lava-ball material.
fn attractor_ball_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{ATTRACTOR_BALL_WGSL}")
}

/// The bloom + alpha-max composite (`renderPipeline.outputNode`).
fn attractor_final_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{ATTRACTOR_FINAL_WGSL}")
}

const ATTRACTOR_SIM_WGSL: &str = r#"
struct SimUniforms {
  attractor_mass: f32,
  particle_global_mass: f32,
  spinning_strength: f32,
  max_speed: f32,
  attractors: array<vec4<f32>, 3>,
  axes: array<vec4<f32>, 3>,
  gravity: f32,
  damping: f32,
  delta: f32,
  bound_half_extent: f32,
}
@group(0) @binding(0) var<uniform> u: SimUniforms;
@group(0) @binding(1) var positions: texture_2d<f32>;
@group(0) @binding(2) var velocities: texture_2d<f32>;

struct SimOut {
  @location(0) position: vec4<f32>,
  @location(1) velocity: vec4<f32>,
}

fn glsl_mod3(x: vec3<f32>, y: vec3<f32>) -> vec3<f32> {
  return x - y * floor(x / y);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> SimOut {
  let coord = vec2<i32>(i32(position.x), i32(position.y));
  let state = textureLoad(positions, coord, 0);
  let velocity = textureLoad(velocities, coord, 0).xyz;
  let particle_mass = state.w * u.particle_global_mass;

  var force = vec3<f32>(0.0);
  for (var i = 0u; i < 3u; i = i + 1u) {
    let to_attractor = u.attractors[i].xyz - state.xyz;
    let distance = length(to_attractor);
    let direction = to_attractor / distance;

    let gravity_strength = u.attractor_mass * particle_mass * u.gravity / (distance * distance);
    force = force + direction * gravity_strength;

    let spinning_force = u.axes[i].xyz * gravity_strength * u.spinning_strength;
    force = force + cross(spinning_force, to_attractor);
  }

  var next_velocity = velocity + force * u.delta;
  let speed = length(next_velocity);
  if (speed > u.max_speed) {
    next_velocity = normalize(next_velocity) * u.max_speed;
  }
  next_velocity = next_velocity * (1.0 - u.damping);

  var next_position = state.xyz + next_velocity * u.delta;
  let half_extent = u.bound_half_extent * 0.5;
  next_position = glsl_mod3(
    next_position + vec3<f32>(half_extent),
    vec3<f32>(u.bound_half_extent),
  ) - vec3<f32>(half_extent);

  var out: SimOut;
  out.position = vec4<f32>(next_position, state.w);
  out.velocity = vec4<f32>(next_velocity, 0.0);
  return out;
}
"#;

const ATTRACTOR_PARTICLE_WGSL: &str = r#"
struct ParticleUniforms {
  view_proj: mat4x4<f32>,
  camera_right: vec4<f32>,
  camera_up: vec4<f32>,
  resolution: vec2<f32>,
  scale: f32,
  max_speed: f32,
  colour_a: vec4<f32>,
  colour_b: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: ParticleUniforms;
@group(0) @binding(1) var positions: texture_2d<f32>;
@group(0) @binding(2) var velocities: texture_2d<f32>;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) colour: vec3<f32>,
  @location(1) uv: vec2<f32>,
}

@vertex
fn vs_main(
  @builtin(instance_index) instance: u32,
  @location(2) corner: vec2<f32>,
  @location(3) uv: vec2<f32>,
) -> VsOut {
  // `positionBuffer.toAttribute()` / `velocityBuffer.toAttribute()`.
  let coord = vec2<i32>(
    i32(instance % 512u),
    i32(instance / 512u),
  );
  let state = textureLoad(positions, coord, 0);
  let velocity = textureLoad(velocities, coord, 0).xyz;

  // `scaleNode = particleMassMultiplier * scale`
  let size = state.w * u.scale;
  let world = state.xyz + u.camera_right.xyz * corner.x * size
    + u.camera_up.xyz * corner.y * size;

  let speed = length(velocity);
  let colour_mix = smoothstep(0.0, 0.5, speed / u.max_speed);

  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(world, 1.0);
  out.colour = mix(u.colour_a.rgb, u.colour_b.rgb, colour_mix);
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let radius = length(in.uv * 2.0 - 1.0);
  let alpha = 1.0 - smoothstep($SPRITE_INNER, 1.0, radius);
  return vec4<f32>(in.colour, alpha);
}
"#;

const ATTRACTOR_BALL_WGSL: &str = r#"
struct BallUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  fire_start: vec4<f32>,
  fire_mid: vec4<f32>,
  fire_end: vec4<f32>,
  // x time, y bass glow
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: BallUniforms;
@group(0) @binding(1) var moon: texture_2d<f32>;
@group(0) @binding(2) var moon_sampler: sampler;

struct BallInstance {
  @location(4) position: vec3<f32>,
  @location(5) hue: f32,
}

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) w_pos: vec3<f32>,
  @location(2) local: vec3<f32>,
  @location(3) uv: vec2<f32>,
  @location(4) hue: f32,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
  instance: BallInstance,
) -> VsOut {
  let world = instance.position + position;
  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(world, 1.0);
  out.normal = normal;
  out.w_pos = world;
  out.local = position;
  out.uv = uv;
  out.hue = instance.hue;
  return out;
}

fn luminance(rgb: vec3<f32>) -> f32 {
  return dot(vec3<f32>(0.2126, 0.7152, 0.0722), rgb);
}

// TSL `hue()`: a Rodrigues rotation about the white axis.
fn hue_rotate(rgb: vec3<f32>, angle: f32) -> vec3<f32> {
  let axis = vec3<f32>(0.57735);
  let c = cos(angle);
  let s = sin(angle);
  let d = dot(axis, rgb);
  return rgb * c + cross(axis, rgb) * s + axis * d * (1.0 - c);
}

// `fireHue(c, adjustment)`: equalise the rotated luminance to the green state.
fn fire_hue(rgb: vec3<f32>, adjustment: f32) -> vec3<f32> {
  let rotated = hue_rotate(rgb, adjustment);
  let green = hue_rotate(rgb, 2.0943951); // ROYGBIV_HUES[4]
  return rotated * (luminance(green) / max(luminance(rotated), 0.001));
}

fn saturation(rgb: vec3<f32>, adjustment: f32) -> vec3<f32> {
  return mix(vec3<f32>(luminance(rgb)), rgb, adjustment);
}

fn fire_ramp(t: f32) -> vec3<f32> {
  var colour = mix(u.fire_end.rgb * 0.0, u.fire_end.rgb, smoothstep(0.05, 0.35, t));
  colour = mix(colour, u.fire_mid.rgb, smoothstep(0.35, 0.65, t));
  colour = mix(colour, u.fire_start.rgb, smoothstep(0.65, 1.0, t));
  return colour;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // three uploads textures with flipY = true; the loader uploads row 0 = top.
  let albedo = textureSample(moon, moon_sampler, vec2<f32>(in.uv.x, 1.0 - in.uv.y)).rgb;
  let normal = normalize(in.normal);
  let light_dir = normalize(vec3<f32>(4.0, 2.0, 0.0) - in.w_pos);
  // AmbientLight(0.5) + DirectionalLight(1.5) at (4, 2, 0).
  let lit = albedo * (0.5 + 1.5 * max(dot(normal, light_dir), 0.0));

  let p = in.local * 0.5;
  let flow = vec3<f32>(0.0, -u.params.x, 0.0);
  let n1 = p9_noise(p + flow) * 0.5 + 0.5;
  let n2 = p9_noise(p * 2.0 - flow * 1.5 + vec3<f32>(n1 * 0.4)) * 0.5 + 0.5;
  let n3 = p9_noise(p * 4.0 + flow * 2.5) * 0.5 + 0.5;
  let noise_value = n1 * 0.5 + n2 * 0.35 + n3 * 0.15;
  let lava_t = clamp(pow(noise_value, 2.5), 0.0, 1.0);
  let fire = fire_ramp(lava_t + 0.1);
  let coloured = fire_hue(saturation(fire, 1.1), in.hue);
  let emissive = coloured * 3.5 * (1.5 + u.params.y * 5.3);

  return vec4<f32>(lit + emissive, 1.0);
}
"#;

const ATTRACTOR_FINAL_WGSL: &str = r#"
struct FinalUniforms {
  resolution: vec2<f32>,
  _pad: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: FinalUniforms;
@group(0) @binding(1) var scene_colour: texture_2d<f32>;
@group(0) @binding(2) var bloom_colour: texture_2d<f32>;
@group(0) @binding(3) var source_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let scene = textureSample(scene_colour, source_sampler, uv);
  let bloom = textureSample(bloom_colour, source_sampler, uv);
  let rgb = scene.rgb + bloom.rgb;
  return vec4<f32>(rgb, clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-attractor-particles.test.js.

    fn approx(actual: f32, expected: f32) {
        assert!(
            (actual / expected - 1.0).abs() < 1e-4,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn audio_uniforms_use_the_example_silence_defaults() {
        let uniforms = audio_uniforms(0.0, 0.0, 0.0, 0.0);
        assert_eq!(uniforms.max_speed, 8.0);
        approx(uniforms.attractor_mass, 1e6);
        assert_eq!(uniforms.spinning_strength, 2.75);
        approx(uniforms.particle_global_mass, 1e4);
    }

    #[test]
    fn audio_uniforms_drive_speed_and_spin_with_bass() {
        let uniforms = audio_uniforms(1.0, 0.0, 0.0, 0.0);
        assert_eq!(uniforms.max_speed, 14.0);
        assert_eq!(uniforms.spinning_strength, 12.75);
        approx(uniforms.attractor_mass, 1e7);
    }

    #[test]
    fn percussion_and_energy_raise_the_mass_exponents_and_spin() {
        let uniforms = audio_uniforms(1.0, 1.0, 1.0, 1.0);
        approx(uniforms.attractor_mass, 1e8);
        approx(uniforms.particle_global_mass, 1e6);
        assert_eq!(uniforms.spinning_strength, 14.75);
    }

    #[test]
    fn percussion_exponent_assist_clamps_at_the_top_of_its_range() {
        let loud = audio_uniforms(0.0, 1.0, 1.0, 0.0);
        approx(loud.attractor_mass, 1e8);
        approx(loud.particle_global_mass, 1e4);
    }

    #[test]
    fn audio_uniforms_scale_smoothly_between_the_documented_ranges() {
        let uniforms = audio_uniforms(0.5, 0.25, 0.0, 0.5);
        assert!((uniforms.max_speed - 11.0).abs() < 1e-5);
        assert!((uniforms.spinning_strength - (2.75 + 5.0 + 1.0)).abs() < 1e-5);
        assert!((uniforms.attractor_mass - 10.0_f32.powf(6.0 + (0.375 + 0.25) * 2.0)).abs() < 1e-1);
        assert!((uniforms.particle_global_mass - 1e5).abs() < 1e-1);
    }

    #[test]
    fn band_means_split_bass_snare_hi_and_energy_like_the_js() {
        let mut freq = [0.0_f32; AUDIO_SAMPLES];
        for value in freq.iter_mut().take(5) {
            *value = 0.4;
        }
        for value in freq.iter_mut().take(133).skip(20) {
            *value = 0.2;
        }
        for value in freq.iter_mut().skip(AUDIO_SAMPLES - 153) {
            *value = 0.1;
        }

        let bands = band_means(&freq);
        assert!((bands.bass - 0.4).abs() < 1e-6);
        assert!((bands.snare - 0.2).abs() < 1e-6);
        assert!((bands.hi - 0.1).abs() < 1e-6);
        let expected_energy = (0.4 * 5.0 + 0.2 * 113.0 + 0.1 * 153.0) / AUDIO_SAMPLES as f32;
        assert!((bands.energy - expected_energy).abs() < 1e-6);
    }

    #[test]
    fn particle_seeds_are_deterministic_and_inside_the_documented_bounds() {
        for index in [0_u32, 1, 17, 1000, PARTICLE_COUNT - 1] {
            let seed = particle_seed(index);
            assert_eq!(seed, particle_seed(index));
            assert!(seed.position[0].abs() <= 2.5);
            assert!(seed.position[1].abs() <= 0.1);
            assert!(seed.position[2].abs() <= 2.5);
            let speed = length(seed.velocity);
            assert!((speed - 0.05).abs() < 1e-4, "speed {speed}");
            assert!((0.25..=1.0).contains(&seed.mass_multiplier));
        }
    }

    #[test]
    fn step_clamps_the_speed_and_damps_the_velocity() {
        let params = SimParams {
            max_speed: 0.01,
            ..SimParams::silence()
        };
        let (_, velocity) = step_particle([0.0; 3], [5.0, 0.0, 0.0], 1.0, &params, 1.0 / 60.0);
        // The force is negligible over one step, so the clamp dominates and the
        // damping scales the clamped speed by (1 - 0.1).
        assert!(
            length(velocity) <= 0.01 * 0.9 + 1e-6,
            "velocity {velocity:?}"
        );
    }

    #[test]
    fn step_wraps_positions_into_the_box() {
        let params = SimParams {
            bound_half_extent: 8.0,
            ..SimParams::silence()
        };
        let (position, _) = step_particle([4.0, 0.0, 0.0], [30.0, 0.0, 0.0], 1.0, &params, 1.0);
        assert!(
            position[0] >= -4.0 && position[0] <= 4.0,
            "position {position:?}"
        );
    }

    #[test]
    fn step_pulls_a_particle_toward_the_attractors() {
        let params = SimParams::silence();
        // Far down +z, every attractor is behind the particle, so the pull is
        // along -z (attractor 3 sits at z = 1).
        let start = [0.0, 0.0, 4.0];
        let (position, velocity) = step_particle(start, [0.0; 3], 1.0, &params, 1.0 / 60.0);
        assert!(length(velocity) > 0.0);
        assert!(position[2] < start[2], "position {position:?}");
    }

    #[test]
    fn attractor_axes_are_unit_vectors() {
        for axis in ATTRACTOR_AXES {
            approx(length(axis), 1.0);
        }
        // The third axis is `normalize(1, 0, -0.5)`.
        assert!((ATTRACTOR_AXES[2][0] - 0.894_427_2).abs() < 1e-6);
        assert!((ATTRACTOR_AXES[2][2] + 0.447_213_6).abs() < 1e-6);
    }

    #[test]
    fn particle_count_is_two_to_the_eighteen_on_a_square_state() {
        assert_eq!(PARTICLE_COUNT, 262_144);
        assert_eq!(STATE_SIZE * STATE_SIZE, PARTICLE_COUNT);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(MAX_SPEED_BASE, 8.0);
        assert_eq!(MAX_SPEED_PER_BASS, 6.0);
        assert_eq!((MASS_EXP_MIN, MASS_EXP_RANGE), (6.0, 2.0));
        assert_eq!(PERCUSSION_K, 1.5);
        assert_eq!(MASS_EXP_PER_BASS, 0.5);
        assert_eq!(SPIN_BASE, 2.75);
        assert_eq!((SPIN_PER_BASS, SPIN_PER_ENERGY), (10.0, 2.0));
        assert_eq!((PARTICLE_MASS_EXP_MIN, PARTICLE_MASS_EXP_RANGE), (4.0, 2.0));
        assert_eq!(BALL_RADIUS, 0.01);
        assert_eq!((BLOOM_BASE, BLOOM_PER_BASS), (0.12, 3.6));
        assert_eq!(
            BALL_START_HUES,
            [
                0.0,
                -std::f32::consts::PI / 4.0,
                3.0 * std::f32::consts::PI / 4.0
            ]
        );
        assert_eq!(CLEAR_ALPHA, 0.0);
    }

    #[test]
    fn camera_orbit_starts_at_the_js_pose_and_drifts() {
        let mut orbit = OrbitState::from_start(9.9);
        // Seeded from (3, 5, 8) around (0, 1/6, 1/6).
        assert!((orbit.azimuth - (7.833_334_f32).atan2(3.0)).abs() < 1e-5);

        let before = orbit.advance(0.0, 0.0);
        assert!(
            (length([
                before[0] - camera_centre()[0],
                before[1] - camera_centre()[1],
                before[2] - camera_centre()[2],
            ]) - 9.9)
                .abs()
                < 1e-4
        );
        assert!(before[1] >= camera_centre()[1] - 0.4);

        let after = orbit.advance(1.0, 1.0 / 60.0);
        assert!(after[0] != before[0]);
    }

    #[test]
    fn bloom_strength_follows_the_bass_and_the_brightness_prop() {
        let mut scene = AttractorParticles::new();
        assert!((scene.bloom_strength() - BLOOM_BASE).abs() < 1e-6);

        scene.bass = 0.25;
        assert!((scene.bloom_strength() - (0.12 + 0.9)).abs() < 1e-5);

        scene.bass = 10.0;
        scene.apply_props(|key| (key == "brightness").then_some(0.5));
        assert!((scene.bloom_strength() - BLOOM_MAX * 0.5).abs() < 1e-5);
    }

    #[test]
    fn update_smooths_the_bands_and_keeps_silence_at_the_defaults() {
        let mut scene = AttractorParticles::new();
        // No audio: the JS returns before touching the uniforms.
        scene.update(1.0 / 60.0, &AudioBridge::new());
        // The JS uniform initialisers, untouched without audio.
        approx(scene.uniforms().attractor_mass, 1e7);
        assert_eq!(scene.uniforms().max_speed, 8.0);
        // `this._dt || 1/60` — the first frame still advances the clock.
        assert!((scene.time() - 1.0 / 60.0).abs() < 1e-6);

        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag = [0.5; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());
        scene.update(0.1, &bridge);
        assert!((scene.bass() - 0.5 * 0.8).abs() < 1e-5);
        assert!(scene.uniforms().max_speed > MAX_SPEED_BASE);
    }

    #[test]
    fn seeds_advance_only_when_a_drop_fires() {
        let mut stream = SeedStream::new(SEED_A);
        let first = stream.next_sample();
        assert!((first - seeded_rand(SEED_A as i32, 0)).abs() < 1e-7);
        assert_eq!(stream.counter(), 1);
        let second = stream.next_sample();
        assert!((second - seeded_rand(SEED_A as i32, 1)).abs() < 1e-7);
        assert_eq!(stream.counter(), 2);
    }

    #[test]
    fn hue_easing_takes_the_short_arc() {
        let eased = ease_phrase_hue(0.0, ROYGBIV_HUES[6], 0.1, 1.2);
        assert!(eased > 0.0 && eased < ROYGBIV_HUES[6]);
        assert_eq!(ease_phrase_hue(0.0, 1.0, 0.0, 1.2), 0.0);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<SimUniforms>(), 128);
        assert_eq!(std::mem::size_of::<ParticleUniforms>(), 144);
        assert_eq!(std::mem::size_of::<BallUniforms>(), 144);
        assert_eq!(std::mem::size_of::<FinalUniforms>(), 16);
        assert_eq!(std::mem::size_of::<BallInstance>(), 16);
    }

    #[test]
    fn sphere_mesh_is_outward_and_round() {
        let mesh = build_sphere(0.01, 8, 6);
        assert_eq!(mesh.vertices.len(), 9 * 7);
        assert_eq!(mesh.indices.len(), 8 * 6 * 6);
        for vertex in &mesh.vertices {
            let point = vertex.position;
            assert!((length(point) - 0.01).abs() < 1e-6);
            assert!((dot(normalise(point), vertex.normal) - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("sim", attractor_sim_wgsl()),
            ("particle", attractor_particle_wgsl()),
            ("ball", attractor_ball_wgsl()),
            ("final", attractor_final_wgsl()),
        ] {
            let module = match naga::front::wgsl::parse_str(&program) {
                Ok(module) => module,
                Err(error) => panic!("{label} WGSL failed to parse: {error}"),
            };
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{label} WGSL must validate: {error}"));
        }
    }

    #[test]
    fn shaders_keep_the_preset_maths() {
        let sim = attractor_sim_wgsl();
        for needle in [
            "u.attractor_mass * particle_mass * u.gravity / (distance * distance)",
            "cross(spinning_force, to_attractor)",
            "next_velocity = next_velocity * (1.0 - u.damping)",
            "if (speed > u.max_speed)",
            "glsl_mod3(",
        ] {
            assert!(sim.contains(needle), "sim lost {needle}");
        }

        let particle = attractor_particle_wgsl();
        for needle in [
            "let size = state.w * u.scale;",
            "smoothstep(0.0, 0.5, speed / u.max_speed)",
            "let alpha = 1.0 - smoothstep(",
        ] {
            assert!(particle.contains(needle), "particle lost {needle}");
        }

        let ball = attractor_ball_wgsl();
        for needle in [
            "fn fire_hue(",
            "fn fire_ramp(",
            "luminance(green) / max(luminance(rotated), 0.001)",
            "emissive = coloured * 3.5 * (1.5 + u.params.y * 5.3)",
            "pow(noise_value, 2.5)",
        ] {
            assert!(ball.contains(needle), "ball lost {needle}");
        }

        let final_shader = attractor_final_wgsl();
        assert!(final_shader.contains("let rgb = scene.rgb + bloom.rgb;"));
        assert!(final_shader.contains("clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0)"));
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = AttractorParticles::new();
        assert_eq!(scene.meta().name, "Point10 Attractor Particles");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 4..],
            &["cameraDistance", "particleSize", "brightness", "maxSpeed"]
        );
        assert_eq!(names[0], "gain");
    }
}
