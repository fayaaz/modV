//! RollingFire — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/RollingFire.js`.
//!
//! The JS scene is **WebGPU/TSL**, a port of three's `webgpu_volume_fire`
//! example: a 100x100x200 velocity/dye grid with seven compute passes per
//! 1/120 s step, rendered by an 18-step `VolumeNodeMaterial` raymarch and
//! bloomed. The teapot is a lava sphere that **rolls with the music**: bass
//! accelerates it, a bass-rise "beat" kicks it, hats and snares puff the fire
//! and the ROYGBIV hue flips on phrase crests.
//!
//! # Ported exactly
//!
//! * the ball drive (`:102-176`), the scene's only pure simulation:
//!   [`beat_kick`] (the `riseFrac`/`floor`/`maxRise` gates),
//!   [`roll_ball`] (the `5/7` solid-sphere factor, `ROLL_DAMP`, the
//!   `ROLL_MAX_SPEED` cap, the `BALL_ROVE_RADIUS = 40` rim with `BALL_BOUNCE`),
//!   `BALL_ACCEL_K = 60`, `BALL_BEAT_K = 3`, `BALL_DIR_SPIN = 0.35`,
//!   `BALL_DIR_SWAY = 2.0`;
//! * the audio mapping (`:228-291`): bass = mean of the first five bins with
//!   `dt * 8` smoothing, the hi-hat band (top 15%) with `dt * 10`, the hi-hat
//!   spectral centroid `sway = ((c/w - hiStart) / (len - hiStart) - 0.5) * 0.25`
//!   with `BALL_DIR_SWAY`, the snare band (2%..13%) with `dt * 12`,
//!   `uFireSize = 1 + hi * 2.5 + snare * 8`, the `dt * 0.15` camera-yaw
//!   smoothing and the `dropColorPulse` hue flip eased on the shortest arc at
//!   `dt * 1.2`;
//! * the per-frame integration (`:1403-1425`): the kick impulse into the
//!   velocity, `roll_ball`'s damping/cap/rim, the constant ball height
//!   `floorY + SPHERE_RADIUS`, the `delta * 0.25` sphere spin;
//! * the camera (`:1427-1453`): the world-offset follow
//!   `camFollow = (9, 3.5, 2.8)` rotated by the bass yaw, `lerp(k = delta *
//!   1.2)`, the two `ImprovedNoise` jitter terms, and `lookAt(ball - 1)` so the
//!   floor stays in frame;
//! * the fire light/emission calibration (`:389-405`, `:1000-1034`):
//!   `uEmitDensity 16`, `uEmitTemperature 14`, `uFireIntensity 26`,
//!   `fireRamp(temperature)` from `0xffe68c`/`0xff7305`/`0xff0000`, the
//!   `(temperature)^(6 - 5)` fire power, the `fireHue` luminance equalisation,
//!   `tempFactor = (emitTemp/33.36)^4`, `densityScale = emitDensity/44.08`,
//!   the 3-second fade-in and the bass self-glow `1.5 + bass * 5.3`;
//! * the bloom (`:1373-1378`): `min(0.12 + bass * 3.6, 4.0)`;
//! * the renderer grade (`:335-336`): ACES filmic, exposure 2;
//! * the composite (`:1236-1241`): `alpha = max(rgb)` on transparent black;
//! * the props (`sceneProps.js:46-58`, `bindSmoke`): `smokeDensity` 16,
//!   `fireIntensity` 26, `smokeLifespan` 1.5, `fireLifespan` 0.9,
//!   `simulationSpeed` 1.2, `sceneOffsetX`/`sceneOffsetY`.
//!
//! # Replaced TSL machinery — the fire volume
//!
//! This crate has no compute path or 3D storage textures, so the **motion,
//! colour and audio behaviour** of the simulation is kept and the density
//! field becomes seeded additive billboards (the same approach DVD Room and
//! Smoke Room document):
//!
//! | RollingFire (TSL) | RollingFire (Rust) |
//! |-------------------|--------------------|
//! | 2M-cell velocity/dye/pressure grids + seven compute passes | 48 CPU puffs emitted around the lava sphere |
//! | the radial buoyancy force (away from the sphere) + curl turbulence | each puff's spawn direction is radial, with a small upward bias and per-puff curl |
//! | `uCooling` / `uDissipation` (fire/smoke lifespan) | the puff age and colour decay |
//! | `uTurbulence`, `uTurbulenceDecay`, `uTurbFreq` | the per-puff curl and `simulationSpeed` |
//! | `uEmitDensity` / `uEmitTemperature` / `uFireIntensity` | puffs' density/brightness scalars |
//! | 18-step raymarch + volumetric shadow + gaussian blur | additive puffs + [`UnrealBloomPass`] |
//!
//! The puffs are seeded with [`seeded_rand`], so a run is deterministic; the
//! JS uses `Math.random` only through its curl noise, which is deterministic
//! per voxel anyway.
//!
//! # Other deviations
//!
//! * **The floor is a real textured plane, not a shadow-receiving PBR mesh.**
//!   The rubber diffuse map is loaded through [`load_texture`] (procedural
//!   fallback when the asset is missing) and shaded by a simplified overhead
//!   spot; the JS's `receiveShadow` depth map has no equivalent here.
//! * `ImprovedNoise` becomes [`value_noise`] (deterministic smooth value noise)
//!   for the flame height/sway/flicker/colour jitter.
//! * The key spot's shadow map, the volumetric shadow pass and the point
//!   light's projection noise are folded into the emissive/bloom calibration:
//!   the port has no shadow map.
//! * The lava sphere is a 32x16 UV sphere; its emissive noise is
//!   [`p9_noise`](crate::point10::shared::p9_glsl::P9_COMMON_WGSL) instead of
//!   `mx_noise_float` (the same substitution AttractorParticles documents).
//! * `Ball of Fire` (the frozen subclass) is not a separate Rust scene: the
//!   wrapper list for this batch is the four module names, and RollingFire
//!   keeps the free-roaming ball.
//!
//! # Transparency
//!
//! `_transparentOverlay = true` (`:189`), no scene background, and the output
//! node's alpha is `max(rgb)` (`:1239-1241`): the fire composites over the
//! layer below. The port keeps that rule in its final pass.
//!
//! [`UnrealBloomPass`]: crate::point10::postfx::UnrealBloomPass

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge};
use crate::point10::postfx::{Composer, ToneMapping, UnrealBloomPass};
use crate::point10::scenes::cube_mesh::{
    Mat4, Mesh, MeshGpu, Vertex, create_depth, depth_state, look_at, mat4_mul, normalise,
    perspective,
};
use crate::point10::scenes::fire_common::build_sphere;
use crate::point10::shared::assets::{ColourSpace, Wrap, load_texture};
use crate::point10::shared::phrase_color::{DropColorState, drop_color_pulse, ease_phrase_hue};
use crate::point10::shared::random::seeded_rand;
use crate::point10::volume::room::{SPHERE_RADIUS, band, fire_colour, treble_band};
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    UnitQuad, frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`RollingFire.js:63-138`)
// ---------------------------------------------------------------------------

/// `VOLUME_WORLD_SIZE_X`.
pub const VOLUME_WORLD_SIZE_X: f32 = 16.0;
/// `VOLUME_WORLD_SIZE_Y`.
pub const VOLUME_WORLD_SIZE_Y: f32 = 16.0;
/// `VOLUME_WORLD_SIZE_Z`.
pub const VOLUME_WORLD_SIZE_Z: f32 = 32.0;

/// `BASS_BINS`.
pub const BASS_BINS: usize = 5;
/// `BLOOM_BASE`.
pub const BLOOM_BASE: f32 = 0.12;
/// `BLOOM_PER_BASS` (the sphere's glow).
pub const BLOOM_PER_BASS: f32 = 3.6;
/// The JS clamps the rolling-fire bloom at 4.
pub const BLOOM_MAX: f32 = 4.0;

/// `FIRE_HAT_K`.
pub const FIRE_HAT_K: f32 = 2.5;
/// `FIRE_SNARE_K`.
pub const FIRE_SNARE_K: f32 = 8.0;

/// `BALL_ACCEL_K` — smoothed bass -> ball acceleration (m/s² per unit bass).
pub const BALL_ACCEL_K: f32 = 60.0;
/// `BALL_DIR_SPIN`.
pub const BALL_DIR_SPIN: f32 = 0.35;
/// `BALL_DIR_SWAY`.
pub const BALL_DIR_SWAY: f32 = 2.0;
/// `BALL_BEAT_K`.
pub const BALL_BEAT_K: f32 = 3.0;
/// `ROLL_FACTOR` — solid sphere rolling without slipping.
pub const ROLL_FACTOR: f32 = 5.0 / 7.0;
/// `ROLL_DAMP`.
pub const ROLL_DAMP: f32 = 0.3;
/// `ROLL_MAX_SPEED`.
pub const ROLL_MAX_SPEED: f32 = 30.0;
/// `BALL_ROVE_RADIUS`.
pub const BALL_ROVE_RADIUS: f32 = 40.0;
/// `BALL_BOUNCE`.
pub const BALL_BOUNCE: f32 = 0.6;
/// `FLOOR_AIM_DOWN`.
pub const FLOOR_AIM_DOWN: f32 = 1.0;

/// `BALL_BEAT` gates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatGates {
    /// `riseFrac`.
    pub rise_frac: f32,
    /// `floor`.
    pub floor: f32,
    /// `maxRise`.
    pub max_rise: f32,
}

/// `BALL_BEAT`.
pub const BALL_BEAT: BeatGates = BeatGates {
    rise_frac: 0.3,
    floor: 0.04,
    max_rise: 0.1,
};

/// `FLOOR_TILE` — world metres per texture tile.
pub const FLOOR_TILE: f32 = 10.0;
/// `FLOOR_PLANE`.
pub const FLOOR_PLANE: f32 = 400.0;
/// `floorMat.opacity`.
pub const FLOOR_OPACITY: f32 = 0.45;

/// `uEmitDensity` default.
pub const EMIT_DENSITY: f32 = 16.0;
/// `uEmitTemperature` default.
pub const EMIT_TEMPERATURE: f32 = 14.0;
/// `uFireIntensity` default.
pub const FIRE_INTENSITY: f32 = 26.0;

/// `camera.position.set(9, 3.5, 2.8)`.
pub const CAMERA_START: [f32; 3] = [9.0, 3.5, 2.8];
/// `PerspectiveCamera(60, aspect, 0.1, 100)`.
pub const CAMERA_FOV: f32 = 60.0;
/// Near plane.
pub const CAMERA_NEAR: f32 = 0.1;
/// Far plane.
pub const CAMERA_FAR: f32 = 100.0;

/// `floorGroup.position.y = -VY/2 + 0.4 + VY/2 + 0.4`.
pub const FLOOR_Y: f32 = 0.8;

/// The haze puff count standing in for the simulation grid.
pub const PUFF_COUNT: usize = 48;
/// The per-puff brightness scalar.
pub const PUFF_GAIN: f32 = 0.085;

/// `scene.background = null`.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `rubber002_diff_1k.jpg`.
pub const FLOOR_DIFFUSE_ASSET: &str = "rubber002_diff_1k.jpg";
/// `rubber002_rough_1k.jpg`.
pub const FLOOR_ROUGH_ASSET: &str = "rubber002_rough_1k.jpg";

// ---------------------------------------------------------------------------
// Pure maths (`RollingFire.js:102-176`)
// ---------------------------------------------------------------------------

/// The ball's state on the floor plane (`{x, z, vx, vz}`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ball {
    /// X position.
    pub x: f32,
    /// Z position.
    pub z: f32,
    /// X velocity.
    pub vx: f32,
    /// Z velocity.
    pub vz: f32,
}

impl Default for Ball {
    fn default() -> Self {
        Self::new()
    }
}

impl Ball {
    /// `{ x: 0, z: 0, vx: 0, vz: 0 }`.
    pub const fn new() -> Self {
        Self {
            x: 0.0,
            z: 0.0,
            vx: 0.0,
            vz: 0.0,
        }
    }
}

/// `beatKick(bass, prevBass, opts)` (`:141-145`).
///
/// A bass "beat" is 0..1 when the smoothed bass jumps sharply; a flat,
/// decaying or quiet bass reads 0.
pub fn beat_kick(bass: f32, prev_bass: f32, opts: BeatGates) -> f32 {
    let bass = if bass.is_finite() { bass } else { 0.0 };
    let prev_bass = if prev_bass.is_finite() {
        prev_bass
    } else {
        0.0
    };
    let rise = bass - prev_bass;
    if rise <= (opts.rise_frac * bass).max(opts.floor) {
        return 0.0;
    }
    (rise / opts.max_rise).min(1.0)
}

/// `rollBall(ball, grav, dt, opts)` (`:149-176`).
///
/// `grav` is `[gx, gy, gz]` in the floor's frame; the level plane ignores `gy`.
pub fn roll_ball(
    ball: Ball,
    grav: [f32; 3],
    dt: f32,
    damping: f32,
    bounce: f32,
    radius: f32,
    max_speed: f32,
) -> Ball {
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let damp = (1.0 - damping * dt).max(0.0);
    let mut vx = (ball.vx + grav[0] * ROLL_FACTOR * dt) * damp;
    let mut vz = (ball.vz + grav[2] * ROLL_FACTOR * dt) * damp;
    let speed = (vx * vx + vz * vz).sqrt();
    if speed > max_speed && speed > 0.0 {
        vx = (vx / speed) * max_speed;
        vz = (vz / speed) * max_speed;
    }
    let mut x = ball.x + vx * dt;
    let mut z = ball.z + vz * dt;
    let distance = (x * x + z * z).sqrt();
    if distance > radius && radius > 0.0 {
        x *= radius / distance;
        z *= radius / distance;
        let radial = (vx * x + vz * z) / radius;
        if radial > 0.0 {
            vx -= (1.0 + bounce) * radial * (x / radius);
            vz -= (1.0 + bounce) * radial * (z / radius);
        }
    }
    Ball { x, z, vx, vz }
}

/// `uFireSize = 1 + hi * FIRE_HAT_K + snare * FIRE_SNARE_K` (`:278-279`).
pub fn fire_size(hi: f32, snare: f32) -> f32 {
    1.0 + hi * FIRE_HAT_K + snare * FIRE_SNARE_K
}

/// The hi-hat band (top 15%, `:240-252`).
pub fn hi_band(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    treble_band(freq_mag)
}

/// The snare band (2%..13%, `:272-277`).
pub fn snare_band(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    let lo = (AUDIO_SAMPLES as f32 * 0.02).floor() as usize;
    let hi = (AUDIO_SAMPLES as f32 * 0.13).floor() as usize;
    band(freq_mag, lo, hi)
}

/// The hi-hat centroid steer (`:240-248`): `-0.125..0.125`.
pub fn hi_sway(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    let hi_start = AUDIO_SAMPLES - (AUDIO_SAMPLES as f32 * 0.15).floor() as usize;
    let mut weight = 0.0;
    let mut centroid = 0.0;
    for (index, value) in freq_mag.iter().enumerate().skip(hi_start) {
        weight += value;
        centroid += value * index as f32;
    }
    if weight <= 1e-6 {
        return 0.0;
    }
    ((centroid / weight - hi_start as f32) / (AUDIO_SAMPLES - hi_start) as f32 - 0.5) * 0.25
}

/// The lava sphere's emissive scale (`:1149-1160`), including the 3-second
/// fade-in and the bass self-glow.
pub fn sphere_glow(
    emit_temperature: f32,
    emit_density: f32,
    fire_intensity: f32,
    time: f32,
    bass: f32,
) -> f32 {
    let temp_factor = (emit_temperature / 33.36).max(0.0).powi(4);
    let density_scale = (emit_density / 44.08).max(0.0);
    let t = (time / 3.0).clamp(0.0, 1.0);
    let fade_in = t * t * (3.0 - 2.0 * t);
    temp_factor * density_scale * fire_intensity * fade_in * (1.5 + bass.max(0.0) * 5.3)
}

/// Deterministic smooth 1D value noise — the port's stand-in for
/// `ImprovedNoise.noise(x, y, z)` (`:1297-1311`).
pub fn value_noise(x: f32) -> f32 {
    let x = if x.is_finite() { x } else { 0.0 };
    let floor = x.floor();
    let fract = x - floor;
    let a = seeded_rand(floor as i32, 17) * 2.0 - 1.0;
    let b = seeded_rand(floor as i32 + 1, 17) * 2.0 - 1.0;
    let t = fract * fract * (3.0 - 2.0 * fract);
    a + (b - a) * t
}

/// A puff of the fire haze.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puff {
    /// World position.
    pub position: [f32; 3],
    /// World velocity.
    pub velocity: [f32; 3],
    /// Normalised age, 0..1.
    pub age: f32,
    /// World radius.
    pub size: f32,
    /// Brightness multiplier.
    pub alpha: f32,
    /// Temperature for the fire ramp.
    pub temperature: f32,
    /// The seed stream for the next respawn.
    pub seed: u32,
}

impl Puff {
    /// A dead puff (respawns on the next update).
    pub const fn dead() -> Self {
        Self {
            position: [0.0; 3],
            velocity: [0.0; 3],
            age: 1.0,
            size: 0.0,
            alpha: 0.0,
            temperature: 0.0,
            seed: 0,
        }
    }
}

/// Emit a puff on the sphere's surface around `emitter`.
pub fn spawn_puff(seed: u32, emitter: [f32; 3], radius: f32) -> Puff {
    let rand = |offset: u32| seeded_rand(seed as i32, offset as i32);
    let fraction = rand(1);
    let y = 1.0 - 2.0 * fraction;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    let theta = rand(2) * std::f32::consts::TAU;
    let direction = [theta.cos() * ring, y, theta.sin() * ring];
    let speed = 1.6 + rand(3) * 2.6;
    Puff {
        position: [
            emitter[0] + direction[0] * radius,
            emitter[1] + direction[1] * radius,
            emitter[2] + direction[2] * radius,
        ],
        velocity: [
            direction[0] * speed,
            direction[1] * speed + 0.6,
            direction[2] * speed,
        ],
        age: 0.0,
        size: 0.5 + rand(4) * 0.9,
        alpha: 0.6 + rand(5) * 0.4,
        temperature: 0.5 + rand(6) * 0.5,
        seed: seed.wrapping_add(1),
    }
}

/// Advance one puff (`uCooling` / `uDissipation` / `uTurbulence`).
pub fn advance_puff(
    puff: &mut Puff,
    emitter: [f32; 3],
    dt: f32,
    simulation_speed: f32,
    lifespan: f32,
    radius: f32,
) {
    let speed = if simulation_speed.is_finite() {
        simulation_speed.clamp(0.1, 4.0)
    } else {
        1.2
    };
    let life = if lifespan.is_finite() {
        lifespan.clamp(0.1, 20.0)
    } else {
        1.5
    };
    let step = if dt.is_finite() { dt.max(0.0) } else { 0.0 } * speed;
    if puff.age >= 1.0 {
        *puff = spawn_puff(puff.seed, emitter, radius);
        return;
    }
    puff.age = (puff.age + step / life).min(1.0);
    puff.position = [
        puff.position[0] + puff.velocity[0] * step,
        puff.position[1] + puff.velocity[1] * step,
        puff.position[2] + puff.velocity[2] * step,
    ];
    puff.velocity = [
        puff.velocity[0] * (1.0 - 0.45 * step),
        puff.velocity[1] * (1.0 - 0.25 * step) + 0.5 * step,
        puff.velocity[2] * (1.0 - 0.45 * step),
    ];
    puff.size += step * 2.2;
    puff.temperature *= 1.0 - 0.55 * step;
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `ROLLING_FIRE_PROPS` (`sceneProps.js:46-58`).
const ROLLING_FIRE_PROPS: &[SceneProp] = &[
    SceneProp::float("smokeDensity", "Smoke Density", 16.0, 0.0, 100.0, 0.1),
    SceneProp::float("fireIntensity", "Fire Intensity", 26.0, 0.0, 100.0, 0.1),
    SceneProp::float("smokeLifespan", "Smoke Lifespan", 1.5, 0.1, 20.0, 0.1),
    SceneProp::float("fireLifespan", "Fire Lifespan", 0.9, 0.1, 5.0, 0.1),
    SceneProp::float("simulationSpeed", "Simulation Speed", 1.2, 0.1, 4.0, 0.01),
    SceneProp::float("sceneOffsetX", "Offset X", 0.0, -10.0, 10.0, 0.1),
    SceneProp::float("sceneOffsetY", "Offset Y", 0.0, -10.0, 10.0, 0.1),
];

/// The geometry uniform block (128 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    /// rgb = fire colour, w = sphere glow scale.
    glow: [f32; 4],
    /// x = time, y = ball x, z = ball z, w = floor opacity.
    params: [f32; 4],
    /// x = floor tile repeat, y = ball y, z = flicker, w = unused.
    extra: [f32; 4],
}

/// The floor shader's uniform block (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FloorUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    params: [f32; 4],
}

/// The haze puff shader's uniform block (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct PuffUniforms {
    view_proj: Mat4,
    camera_right: [f32; 4],
    camera_up: [f32; 4],
}

/// One puff instance (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct PuffInstance {
    position: [f32; 3],
    size: f32,
    colour: [f32; 3],
    alpha: f32,
}

impl PuffInstance {
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
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 16,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 28,
                    shader_location: 7,
                },
            ],
        }
    }
}

/// The final composite uniform block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FinalUniforms {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// A texture target.
struct TextureTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// GPU resources built in `init`.
struct RollingFireGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    scene_pipeline: wgpu::RenderPipeline,
    scene_uniforms: wgpu::Buffer,
    scene_bind_group: wgpu::BindGroup,
    scene_mesh: MeshGpu,
    floor_pipeline: wgpu::RenderPipeline,
    floor_uniforms: wgpu::Buffer,
    floor_bind_group: wgpu::BindGroup,
    floor_mesh: MeshGpu,
    _floor_texture: wgpu::Texture,
    puff_pipeline: wgpu::RenderPipeline,
    puff_uniforms: wgpu::Buffer,
    puff_bind_group: wgpu::BindGroup,
    puff_instances: wgpu::Buffer,
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
}

/// RollingFire scene.
pub struct RollingFire {
    /// `this._bass`.
    bass: f32,
    /// `this._hi`.
    hi: f32,
    /// `this._snare`.
    snare: f32,
    /// `this._prevBass`.
    previous_bass: f32,
    /// `uFireSize`.
    fire_size: f32,
    /// `this._ball`.
    ball: Ball,
    /// `this._ballDir`.
    ball_direction: f32,
    /// `this._accelX`/`_accelZ`.
    accel: [f32; 3],
    /// `this._kickX`/`_kickZ`.
    kick: [f32; 3],
    /// `this._camYaw`.
    camera_yaw: f32,
    /// `this.camera.position`.
    camera: [f32; 3],
    /// `this._uFireHue`.
    fire_hue: f32,
    /// `this._pulse`.
    pulse: DropColorState,
    /// The seed stream behind the drop detector.
    seed: u32,
    /// `this.time` (`BaseSceneWebGPU.update` accumulates the frame delta).
    time: f32,
    /// `Math.min(this._dt || 1/60, 1/30)`.
    last_delta: f32,
    /// `this._simulationTime`.
    simulation_time: f32,
    /// The haze puffs.
    puffs: [Puff; PUFF_COUNT],
    /// Props.
    smoke_density: f32,
    fire_intensity: f32,
    smoke_lifespan: f32,
    fire_lifespan: f32,
    simulation_speed: f32,
    scene_offset: [f32; 2],
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<RollingFireGpu>,
}

impl Default for RollingFire {
    fn default() -> Self {
        Self::new()
    }
}

impl RollingFire {
    /// Create the scene (`RollingFire.js:178-221` defaults).
    pub fn new() -> Self {
        let mut puffs = [Puff::dead(); PUFF_COUNT];
        for (index, puff) in puffs.iter_mut().enumerate() {
            puff.seed = index as u32 * 13 + 5;
        }
        Self {
            bass: 0.0,
            hi: 0.0,
            snare: 0.0,
            previous_bass: 0.0,
            fire_size: 1.0,
            ball: Ball::new(),
            ball_direction: 0.0,
            accel: [0.0; 3],
            kick: [0.0; 3],
            camera_yaw: 0.0,
            camera: CAMERA_START,
            fire_hue: 0.0,
            pulse: DropColorState::new(),
            seed: 0,
            time: 0.0,
            last_delta: 1.0 / 60.0,
            simulation_time: 0.0,
            puffs,
            smoke_density: EMIT_DENSITY,
            fire_intensity: FIRE_INTENSITY,
            smoke_lifespan: 1.5,
            fire_lifespan: 0.9,
            simulation_speed: 1.2,
            scene_offset: [0.0; 2],
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:46-58`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(ROLLING_FIRE_PROPS, props) {
            match key {
                "smokeDensity" => self.smoke_density = value,
                "fireIntensity" => self.fire_intensity = value,
                "smokeLifespan" => self.smoke_lifespan = value,
                "fireLifespan" => self.fire_lifespan = value,
                "simulationSpeed" => self.simulation_speed = value,
                "sceneOffsetX" => self.scene_offset[0] = value,
                "sceneOffsetY" => self.scene_offset[1] = value,
                _ => {}
            }
        }
    }

    /// `this._ball`.
    pub fn ball(&self) -> Ball {
        self.ball
    }

    /// `this._bass`.
    pub fn bass(&self) -> f32 {
        self.bass
    }

    /// `uFireSize`.
    pub fn fire_size(&self) -> f32 {
        self.fire_size
    }

    /// `this._uFireHue`.
    pub fn fire_hue(&self) -> f32 {
        self.fire_hue
    }

    /// `this.camera.position`.
    pub fn camera_position(&self) -> [f32; 3] {
        self.camera
    }

    /// The haze puffs.
    pub fn puffs(&self) -> &[Puff; PUFF_COUNT] {
        &self.puffs
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// `min(BLOOM_BASE + bass * BLOOM_PER_BASS, 4)` (`:1373-1378`).
    pub fn bloom_strength(&self) -> f32 {
        (BLOOM_BASE + self.bass * BLOOM_PER_BASS).min(BLOOM_MAX)
    }

    /// `this.sphere.position` (`:1419-1423`).
    pub fn sphere_position(&self) -> [f32; 3] {
        [self.ball.x, FLOOR_Y + SPHERE_RADIUS, self.ball.z]
    }

    /// The lava sphere's emissive scale (`:1149-1160`).
    pub fn sphere_glow(&self) -> f32 {
        sphere_glow(
            EMIT_TEMPERATURE,
            self.smoke_density,
            self.fire_intensity,
            self.simulation_time,
            self.bass,
        )
    }

    /// The fire light colour the scene is lit by (`:1051-1171`).
    pub fn fire_light(&self) -> [f32; 4] {
        let colour = fire_colour(0.62, self.fire_hue);
        let intensity = (self.fire_intensity / 26.0).max(0.0) * 2.0;
        [
            colour[0] * intensity * 0.25,
            colour[1] * intensity * 0.25,
            colour[2] * intensity * 0.25,
            intensity * 0.25,
        ]
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &look_at(
                self.camera,
                [
                    self.ball.x + self.scene_offset[0],
                    self.sphere_position()[1] - FLOOR_AIM_DOWN + self.scene_offset[1],
                    self.ball.z,
                ],
                [0.0, 1.0, 0.0],
            ),
        )
    }

    fn billboard_basis(&self) -> ([f32; 3], [f32; 3]) {
        let target = [
            self.ball.x + self.scene_offset[0],
            self.sphere_position()[1] - FLOOR_AIM_DOWN + self.scene_offset[1],
            self.ball.z,
        ];
        let forward = normalise([
            target[0] - self.camera[0],
            target[1] - self.camera[1],
            target[2] - self.camera[2],
        ]);
        let right = normalise([-forward[2], 0.0, forward[0]]);
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        (right, up)
    }

    fn puff_instances(&self) -> Vec<PuffInstance> {
        self.puffs
            .iter()
            .map(|puff| {
                let fade = (1.0 - puff.age).clamp(0.0, 1.0);
                let density = (self.smoke_density / EMIT_DENSITY).max(0.0);
                let fire = (self.fire_intensity / FIRE_INTENSITY).max(0.0);
                let size = (self.fire_size / 1.0).max(0.0);
                let colour = fire_colour(puff.temperature * fade, self.fire_hue);
                let alpha = PUFF_GAIN * density * fire * size * puff.alpha * fade * fade;
                PuffInstance {
                    position: puff.position,
                    size: puff.size * (0.7 + fade * 0.9),
                    colour,
                    alpha,
                }
            })
            .collect()
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let view_proj = self.view_proj();
        let sphere = self.sphere_position();
        let colour = fire_colour(0.6, self.fire_hue);
        gpu.queue.write_buffer(
            &gpu.scene_uniforms,
            0,
            bytemuck::bytes_of(&SceneUniforms {
                view_proj,
                camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
                glow: [colour[0], colour[1], colour[2], self.sphere_glow()],
                params: [self.time, self.ball.x, self.ball.z, FLOOR_OPACITY],
                extra: [
                    FLOOR_PLANE / FLOOR_TILE,
                    sphere[1],
                    value_noise(0.8 * self.time) * 0.12
                        + value_noise(15.0 * self.time) * 0.06
                        + 0.82,
                    0.0,
                ],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.floor_uniforms,
            0,
            bytemuck::bytes_of(&FloorUniforms {
                view_proj,
                camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
                params: [
                    FLOOR_PLANE / FLOOR_TILE,
                    self.fire_light()[3],
                    self.time,
                    FLOOR_OPACITY,
                ],
            }),
        );
        let (right, up) = self.billboard_basis();
        gpu.queue.write_buffer(
            &gpu.puff_uniforms,
            0,
            bytemuck::bytes_of(&PuffUniforms {
                view_proj,
                camera_right: [right[0], right[1], right[2], 0.0],
                camera_up: [up[0], up[1], up[2], 0.0],
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
        let instances = self.puff_instances();
        gpu.queue
            .write_buffer(&gpu.puff_instances, 0, bytemuck::cast_slice(&instances));
    }

    fn rebuild_targets(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.scene_target = TextureTarget::new(
            &gpu.device,
            "point10-rolling-fire-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) = create_depth(
            &gpu.device,
            "point10-rolling-fire-depth",
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

impl Point10Scene for RollingFire {
    fn name(&self) -> &'static str {
        "Point10 Rolling Fire"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width.max(1), ctx.height.max(1));
        self.camera_aspect = ctx.aspect();

        // --- lava sphere ----------------------------------------------------
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-rolling-fire-sphere-shader"),
            source: wgpu::ShaderSource::Wgsl(sphere_wgsl().into()),
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-rolling-fire-sphere-layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let scene_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-rolling-fire-sphere-uniforms"),
            size: std::mem::size_of::<SceneUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-rolling-fire-sphere-bind-group"),
            layout: &scene_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_uniforms.as_entire_binding(),
            }],
        });
        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-rolling-fire-sphere-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-rolling-fire-sphere-pipeline-layout"),
                    bind_group_layouts: &[Some(&scene_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &scene_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &scene_shader,
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
        let scene_mesh = MeshGpu::new(
            &device,
            "point10-rolling-fire-sphere",
            &build_sphere(SPHERE_RADIUS, 32, 16),
        );

        // --- floor ----------------------------------------------------------
        let loader = ctx.asset_loader();
        let floor_texture = load_texture(
            &device,
            &queue,
            &loader,
            FLOOR_DIFFUSE_ASSET,
            ColourSpace::Srgb,
            Wrap::Repeat,
        );
        let floor_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-rolling-fire-floor-shader"),
            source: wgpu::ShaderSource::Wgsl(floor_wgsl().into()),
        });
        let floor_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-rolling-fire-floor-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let floor_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-rolling-fire-floor-uniforms"),
            size: std::mem::size_of::<FloorUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let floor_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-rolling-fire-floor-bind-group"),
            layout: &floor_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: floor_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&floor_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&floor_texture.sampler),
                },
            ],
        });
        let floor_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-rolling-fire-floor-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-rolling-fire-floor-pipeline-layout"),
                    bind_group_layouts: &[Some(&floor_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &floor_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &floor_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let floor_mesh = MeshGpu::new(
            &device,
            "point10-rolling-fire-floor",
            &build_floor(FLOOR_PLANE, FLOOR_Y),
        );

        // --- haze puffs -----------------------------------------------------
        let puff_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-rolling-fire-puff-shader"),
            source: wgpu::ShaderSource::Wgsl(puff_wgsl().into()),
        });
        let puff_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-rolling-fire-puff-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let puff_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-rolling-fire-puff-uniforms"),
            size: std::mem::size_of::<PuffUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let puff_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-rolling-fire-puff-bind-group"),
            layout: &puff_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: puff_uniforms.as_entire_binding(),
            }],
        });
        let quad = UnitQuad::new(&device);
        let puff_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-rolling-fire-puff-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-rolling-fire-puff-pipeline-layout"),
                    bind_group_layouts: &[Some(&puff_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &puff_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(UnitQuad::vertex_layout()),
                    Some(PuffInstance::layout()),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &puff_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
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
        let puff_instances = {
            use wgpu::util::DeviceExt;
            let empty = vec![
                PuffInstance {
                    position: [0.0; 3],
                    size: 0.0,
                    colour: [0.0; 3],
                    alpha: 0.0,
                };
                PUFF_COUNT
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-rolling-fire-puff-instances"),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };

        // --- targets, bloom, final composite --------------------------------
        let scene_target = TextureTarget::new(
            &device,
            "point10-rolling-fire-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) = create_depth(
            &device,
            "point10-rolling-fire-depth",
            self.size.0,
            self.size.1,
        );

        let mut bloom = UnrealBloomPass::new(&device, &queue, self.size.0, self.size.1);
        bloom.set_strength(BLOOM_BASE);
        bloom.set_radius(1.0);
        bloom.set_threshold(0.5);
        bloom.set_input(&scene_target.view);

        let final_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-rolling-fire-final-shader"),
            source: wgpu::ShaderSource::Wgsl(final_wgsl().into()),
        });
        let final_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-rolling-fire-final-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let final_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-rolling-fire-final-uniforms"),
            size: std::mem::size_of::<FinalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let final_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-rolling-fire-final-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let final_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-rolling-fire-final-pipeline",
            &final_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-rolling-fire-final-pipeline-layout"),
                bind_group_layouts: &[Some(&final_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );
        let final_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-rolling-fire-final-bind-group"),
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

        // `renderer.toneMapping = ACESFilmicToneMapping; exposure = 2`.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);
        composer.output_mut().set_exposure(2.0);

        self.gpu = Some(RollingFireGpu {
            device,
            queue,
            scene_pipeline,
            scene_uniforms,
            scene_bind_group,
            scene_mesh,
            floor_pipeline,
            floor_uniforms,
            floor_bind_group,
            floor_mesh,
            _floor_texture: floor_texture.texture,
            puff_pipeline,
            puff_uniforms,
            puff_bind_group,
            puff_instances,
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
        });

        let emitter = self.sphere_position();
        for puff in &mut self.puffs {
            *puff = spawn_puff(puff.seed, emitter, SPHERE_RADIUS);
        }
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        self.rebuild_targets();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let raw_dt = frame_dt(dt);
        self.time += raw_dt;
        // `const delta = Math.min(this._dt || 1 / 60, 1 / 30)` (`:1333`).
        let delta = if raw_dt > 0.0 {
            raw_dt.min(1.0 / 30.0)
        } else {
            1.0 / 60.0
        };
        self.last_delta = delta;
        self.simulation_time += delta * self.simulation_speed;

        // The JS returns before any motion until the renderer is ready and the
        // analyser has data (`:226-227`).
        if !audio.is_available() {
            self.upload_uniforms();
            return;
        }

        let freq_mag = &audio.freq_mag;
        let bass = band(freq_mag, 0, BASS_BINS);
        self.bass += (bass - self.bass) * (delta * 8.0).min(1.0);

        let sway = hi_sway(freq_mag);
        let hi = hi_band(freq_mag);
        self.hi += (hi - self.hi) * (delta * 10.0).min(1.0);
        self.ball_direction += (BALL_DIR_SPIN + sway * BALL_DIR_SWAY) * delta;

        let kick = beat_kick(self.bass, self.previous_bass, BALL_BEAT);
        self.previous_bass = self.bass;
        let magnitude = self.bass * BALL_ACCEL_K;
        self.accel = [
            self.ball_direction.cos() * magnitude,
            0.0,
            self.ball_direction.sin() * magnitude,
        ];
        self.kick = [
            self.ball_direction.cos() * kick * BALL_BEAT_K,
            0.0,
            self.ball_direction.sin() * kick * BALL_BEAT_K,
        ];

        // `this._camYaw += (this._bass * 0.15 - this._camYaw) * min(1, dt*0.15)`.
        self.camera_yaw += (self.bass * 0.15 - self.camera_yaw) * (delta * 0.15).min(1.0);

        // The snare band (`:272-279`).
        let snare = snare_band(freq_mag);
        self.snare += (snare - self.snare) * (delta * 12.0).min(1.0);
        self.fire_size = fire_size(self.hi, self.snare);

        // The drop detector (`:287-290`).
        let sample = seeded_rand(0, self.seed as i32);
        if drop_color_pulse(&mut self.pulse, self.bass, delta, sample) {
            self.seed = self.seed.wrapping_add(1);
        }
        self.fire_hue = ease_phrase_hue(self.fire_hue, self.pulse.hue, delta, 1.2);

        // The ball (`:1403-1414`).
        self.ball.vx += self.kick[0];
        self.ball.vz += self.kick[2];
        self.ball = roll_ball(
            self.ball,
            self.accel,
            delta,
            ROLL_DAMP,
            BALL_BOUNCE,
            BALL_ROVE_RADIUS,
            ROLL_MAX_SPEED,
        );
        self.kick = [0.0; 3];

        // The camera (`:1427-1453`).
        let (cos_yaw, sin_yaw) = self.camera_yaw.sin_cos();
        let offset = [self.scene_offset[0], self.scene_offset[1]];
        let sphere = self.sphere_position();
        let desired = [
            sphere[0] + CAMERA_START[0] * cos_yaw - CAMERA_START[2] * sin_yaw + offset[0],
            sphere[1] + CAMERA_START[1] + offset[1],
            sphere[2] + CAMERA_START[0] * sin_yaw + CAMERA_START[2] * cos_yaw,
        ];
        let lerp = (delta * 1.2).min(1.0);
        self.camera = [
            self.camera[0]
                + (desired[0] - self.camera[0]) * lerp
                + value_noise(self.time * 0.25) * 0.15,
            self.camera[1] + (desired[1] - self.camera[1]) * lerp,
            self.camera[2]
                + (desired[2] - self.camera[2]) * lerp
                + value_noise(self.time * 0.3 + 5.0) * 0.15,
        ];

        // The haze substitute for the fire volume.
        for puff in &mut self.puffs {
            advance_puff(
                puff,
                sphere,
                delta,
                self.simulation_speed,
                self.smoke_lifespan,
                SPHERE_RADIUS,
            );
        }
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let bloom_strength = self.bloom_strength();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        // Floor (alpha blended, depth write) then the opaque lava sphere.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-rolling-fire-scene"),
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
            pass.set_pipeline(&gpu.scene_pipeline);
            pass.set_bind_group(0, &gpu.scene_bind_group, &[]);
            gpu.scene_mesh.draw(&mut pass);

            pass.set_pipeline(&gpu.floor_pipeline);
            pass.set_bind_group(0, &gpu.floor_bind_group, &[]);
            gpu.floor_mesh.draw(&mut pass);
        }

        // Additive haze.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-rolling-fire-haze"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.scene_target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.puff_pipeline);
            pass.set_bind_group(0, &gpu.puff_bind_group, &[]);
            pass.set_vertex_buffer(1, gpu.puff_instances.slice(..));
            gpu.quad.draw_instanced(&mut pass, PUFF_COUNT as u32);
        }

        gpu.bloom.set_strength(bloom_strength);
        gpu.bloom.encode(encoder);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-rolling-fire-final"),
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
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(ROLLING_FIRE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10RollingFire.js` sets `webgpu: true` and no category.
        SceneMeta::for_scene("Point10 Rolling Fire", "3d-scenes")
    }
}

/// The 400 x 400 floor plane at `y`, two triangles facing up.
pub fn build_floor(size: f32, y: f32) -> Mesh {
    let half = size * 0.5;
    let mut mesh = Mesh::default();
    for position in [
        [-half, y, half],
        [half, y, half],
        [half, y, -half],
        [-half, y, -half],
    ] {
        mesh.vertices.push(Vertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    mesh
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

fn create_final_bind_group(device: &wgpu::Device, gpu: &RollingFireGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-rolling-fire-final-bind-group"),
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

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn sphere_wgsl() -> String {
    format!(
        "{}\n{SPHERE_WGSL}",
        crate::point10::shared::p9_glsl::P9_COMMON_WGSL
    )
}

fn floor_wgsl() -> String {
    FLOOR_WGSL.to_string()
}

fn puff_wgsl() -> String {
    PUFF_WGSL.to_string()
}

fn final_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{FINAL_WGSL}")
}

const SPHERE_WGSL: &str = r#"
struct SceneUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // rgb = fire colour, w = glow scale
  glow: vec4<f32>,
  // x time, y ball x, z ball z, w floor opacity
  params: vec4<f32>,
  // x floor tile repeat, y ball y, z flicker
  extra: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: SceneUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) local: vec3<f32>,
  @location(2) world: vec3<f32>,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
) -> VsOut {
  // The sphere sits at the ball's floor position; its world matrix is
  // `translation(x, floorY + radius, z) * rotationY(time * 0.25)`.
  let angle = u.params.x * 0.25;
  let c = cos(angle);
  let s = sin(angle);
  let rotated = vec3<f32>(position.x * c + position.z * s, position.y, -position.x * s + position.z * c);
  let world = rotated + vec3<f32>(u.params.y, u.extra.y, u.params.z);
  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(world, 1.0);
  out.normal = normalize(vec3<f32>(normal.x * c + normal.z * s, normal.y, -normal.x * s + normal.z * c));
  out.local = position;
  out.world = world;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // `MeshStandardMaterial({ color: 0x000000, roughness: 1 })` plus the
  // three-octave lava emissive (`:1000-1034`).
  let p = in.local * 0.5;
  let flow = vec3<f32>(0.0, -u.params.x, 0.0);
  let n1 = p9_noise(p + flow) * 0.5 + 0.5;
  let p2 = p * 2.0 - flow * 1.5;
  let n2 = p9_noise(p2 + vec3<f32>(n1 * 0.4)) * 0.5 + 0.5;
  let p3 = p * 4.0 + flow * 2.5;
  let n3 = p9_noise(p3) * 0.5 + 0.5;
  let noise = n1 * 0.5 + n2 * 0.35 + n3 * 0.15;
  let lava_t = clamp(pow(noise, 2.5), 0.0, 1.0);
  let emissive = u.glow.rgb * u.glow.w * (0.45 + lava_t * 1.1) * u.extra.z;
  // A little diffuse so the unlit side is not a silhouette; the JS sphere is
  // nearly black under the key light too.
  let normal = normalize(in.normal);
  let key = max(dot(normal, normalize(vec3<f32>(0.0, 1.0, 0.0))), 0.0) * 0.15;
  return vec4<f32>(emissive + u.glow.rgb * key, 1.0);
}
"#;

const FLOOR_WGSL: &str = r#"
struct FloorUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // x repeat, y fire intensity, z time, w opacity
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: FloorUniforms;
@group(0) @binding(1) var floor_map: texture_2d<f32>;
@group(0) @binding(2) var floor_sampler: sampler;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
};

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
) -> VsOut {
  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(position, 1.0);
  out.world = position;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // PlaneGeometry(400, 400) with `repeat = FLOOR_PLANE / FLOOR_TILE`.
  let uv = in.world.xz / u.params.x + vec2<f32>(0.5);
  let albedo = textureSample(floor_map, floor_sampler, uv).rgb;
  // A dim overhead spot above the ball (the JS key spotlight) plus the fire's
  // bounce; the floor's PBR roughness map is folded into the flat term.
  let to_light = vec3<f32>(u.camera_pos.x, 20.0, u.camera_pos.z) - in.world;
  let distance = length(to_light);
  let spot = max(dot(vec3<f32>(0.0, 1.0, 0.0), to_light / max(distance, 0.001)), 0.0)
    * 1.6 / (1.0 + distance * distance * 0.002);
  let fire = u.params.y * 0.12 / (1.0 + length(in.world.xz) * 0.15);
  let lit = albedo * (0.06 + spot) + albedo * fire;
  return vec4<f32>(lit, u.params.w);
}
"#;

const PUFF_WGSL: &str = r#"
struct PuffUniforms {
  view_proj: mat4x4<f32>,
  camera_right: vec4<f32>,
  camera_up: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: PuffUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) colour: vec3<f32>,
  @location(1) alpha: f32,
  @location(2) uv: vec2<f32>,
}

@vertex
fn vs_main(
  @location(2) corner: vec2<f32>,
  @location(3) uv: vec2<f32>,
  @location(4) instance_position: vec3<f32>,
  @location(5) instance_size: f32,
  @location(6) instance_colour: vec3<f32>,
  @location(7) instance_alpha: f32,
) -> VsOut {
  let world = instance_position
    + u.camera_right.xyz * corner.x * instance_size
    + u.camera_up.xyz * corner.y * instance_size;
  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(world, 1.0);
  out.colour = instance_colour;
  out.alpha = instance_alpha;
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let radius = length(in.uv * 2.0 - 1.0);
  let mask = pow(1.0 - smoothstep(0.0, 1.0, radius), 2.0);
  return vec4<f32>(in.colour, in.alpha * mask);
}
"#;

const FINAL_WGSL: &str = r#"
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
  // `_transparentOverlay`: alpha = max(rgb).
  return vec4<f32>(rgb, clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn beat_kick_gates_on_the_rise() {
        // A flat bass is not a beat.
        assert_eq!(beat_kick(0.5, 0.5, BALL_BEAT), 0.0);
        // A rise under the floor is not a beat.
        assert_eq!(beat_kick(0.05, 0.02, BALL_BEAT), 0.0);
        // A rise past 30% of the level and past `maxRise` clamps to 1.
        close(beat_kick(0.5, 0.3, BALL_BEAT), 1.0, 1e-6);
        // A rise under 30% of the level is not a beat even though it is loud.
        assert_eq!(beat_kick(0.5, 0.4, BALL_BEAT), 0.0);
        // A real but smaller rise scales linearly: rise 0.08 against the
        // 0.06 gate is 0.8.
        close(beat_kick(0.2, 0.12, BALL_BEAT), 0.8, 1e-6);
        // A fall is never a beat.
        assert_eq!(beat_kick(0.2, 0.4, BALL_BEAT), 0.0);
        assert_eq!(beat_kick(f32::NAN, f32::NAN, BALL_BEAT), 0.0);
    }

    #[test]
    fn roll_ball_integrates_damps_and_caps() {
        let ball = Ball::new();
        let rolled = roll_ball(
            ball,
            [10.0, 0.0, 0.0],
            0.1,
            ROLL_DAMP,
            BALL_BOUNCE,
            BALL_ROVE_RADIUS,
            ROLL_MAX_SPEED,
        );
        // vx = 10 * 5/7 * 0.1 * (1 - 0.03) = 0.6928..., x = 0.06928...
        close(rolled.vx, 10.0 * ROLL_FACTOR * 0.1 * 0.97, 1e-5);
        close(rolled.x, rolled.vx * 0.1, 1e-6);

        // The speed cap holds.
        let fast = roll_ball(
            Ball {
                x: 0.0,
                z: 0.0,
                vx: 100.0,
                vz: 0.0,
            },
            [0.0; 3],
            0.1,
            0.0,
            BALL_BOUNCE,
            BALL_ROVE_RADIUS,
            ROLL_MAX_SPEED,
        );
        close(fast.vx, ROLL_MAX_SPEED, 1e-4);
    }

    #[test]
    fn roll_ball_bounces_off_the_roving_rim() {
        let ball = Ball {
            x: BALL_ROVE_RADIUS - 0.1,
            z: 0.0,
            vx: 50.0,
            vz: 0.0,
        };
        let bounced = roll_ball(
            ball,
            [0.0; 3],
            0.1,
            ROLL_DAMP,
            BALL_BOUNCE,
            BALL_ROVE_RADIUS,
            ROLL_MAX_SPEED,
        );
        assert!(bounced.x <= BALL_ROVE_RADIUS + 1e-4);
        assert!(bounced.vx < 0.0, "the rim must reflect, vx {}", bounced.vx);

        // Non-finite input is a no-op.
        let bad = roll_ball(
            Ball::new(),
            [0.0; 3],
            f32::NAN,
            ROLL_DAMP,
            BALL_BOUNCE,
            BALL_ROVE_RADIUS,
            ROLL_MAX_SPEED,
        );
        assert_eq!(bad, Ball::new());
    }

    #[test]
    fn fire_size_ramps_on_hats_and_snares() {
        assert_eq!(fire_size(0.0, 0.0), 1.0);
        close(fire_size(0.1, 0.0), 1.0 + 0.1 * FIRE_HAT_K, 1e-6);
        close(fire_size(0.0, 0.1), 1.0 + 0.1 * FIRE_SNARE_K, 1e-6);
    }

    #[test]
    fn hi_sway_centroids_the_top_band() {
        let mut freq = [0.0_f32; AUDIO_SAMPLES];
        assert_eq!(hi_sway(&freq), 0.0);
        // Energy only in the last bin steers fully positive.
        freq[AUDIO_SAMPLES - 1] = 1.0;
        assert!(hi_sway(&freq) > 0.1);
        // And the first hat bin steers negative.
        let mut low = [0.0_f32; AUDIO_SAMPLES];
        let hi_start = AUDIO_SAMPLES - (AUDIO_SAMPLES as f32 * 0.15) as usize;
        low[hi_start] = 1.0;
        assert!(hi_sway(&low) < -0.1);
    }

    #[test]
    fn sphere_glow_fades_in_and_rides_the_bass() {
        assert_eq!(
            sphere_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, 0.0, 1.0),
            0.0
        );
        let quiet = sphere_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, 3.0, 0.0);
        let loud = sphere_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, 3.0, 1.0);
        assert!(quiet > 0.0);
        close(loud / quiet, (1.5 + 5.3) / 1.5, 1e-5);
    }

    #[test]
    fn value_noise_is_bounded_and_smooth() {
        for step in 0..100 {
            let value = value_noise(step as f32 * 0.37);
            assert!((-1.0..=1.0).contains(&value));
        }
        assert_eq!(value_noise(f32::NAN), value_noise(0.0));
        assert_eq!(value_noise(3.25), value_noise(3.25));
        // Interpolation stays between the lattice samples.
        assert_ne!(value_noise(0.25), value_noise(0.75));
    }

    #[test]
    fn puffs_spawn_around_the_sphere_and_advance() {
        let emitter = [1.0, 2.0, 3.0];
        let mut puff = spawn_puff(4, emitter, SPHERE_RADIUS);
        assert_eq!(puff.age, 0.0);
        let distance = ((puff.position[0] - emitter[0]).powi(2)
            + (puff.position[1] - emitter[1]).powi(2)
            + (puff.position[2] - emitter[2]).powi(2))
        .sqrt();
        assert!((distance - SPHERE_RADIUS).abs() < 1e-4);

        let start = puff.position;
        advance_puff(&mut puff, emitter, 0.1, 1.2, 1.5, SPHERE_RADIUS);
        assert!(puff.age > 0.0);
        assert_ne!(puff.position, start);
        assert!(puff.temperature < 1.5);

        puff.age = 1.0;
        advance_puff(&mut puff, emitter, 0.1, 1.2, 1.5, SPHERE_RADIUS);
        assert_eq!(puff.age, 0.0);
    }

    #[test]
    fn floor_mesh_is_a_two_triangle_quad_at_the_floor_height() {
        let mesh = build_floor(FLOOR_PLANE, FLOOR_Y);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
        for vertex in &mesh.vertices {
            assert_eq!(vertex.position[1], FLOOR_Y);
            assert_eq!(vertex.normal, [0.0, 1.0, 0.0]);
        }
        assert_eq!(FLOOR_Y, 0.8);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(
            (
                VOLUME_WORLD_SIZE_X,
                VOLUME_WORLD_SIZE_Y,
                VOLUME_WORLD_SIZE_Z
            ),
            (16.0, 16.0, 32.0)
        );
        assert_eq!((BLOOM_BASE, BLOOM_PER_BASS, BLOOM_MAX), (0.12, 3.6, 4.0));
        assert_eq!((FIRE_HAT_K, FIRE_SNARE_K), (2.5, 8.0));
        assert_eq!(
            (BALL_ACCEL_K, BALL_DIR_SPIN, BALL_DIR_SWAY),
            (60.0, 0.35, 2.0)
        );
        assert_eq!(
            (ROLL_DAMP, ROLL_MAX_SPEED, BALL_ROVE_RADIUS),
            (0.3, 30.0, 40.0)
        );
        assert_eq!(ROLL_FACTOR, 5.0 / 7.0);
        assert_eq!(
            BALL_BEAT,
            BeatGates {
                rise_frac: 0.3,
                floor: 0.04,
                max_rise: 0.1
            }
        );
        assert_eq!(
            (EMIT_DENSITY, EMIT_TEMPERATURE, FIRE_INTENSITY),
            (16.0, 14.0, 26.0)
        );
        assert_eq!((FLOOR_TILE, FLOOR_PLANE), (10.0, 400.0));
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(CAMERA_START, [9.0, 3.5, 2.8]);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<SceneUniforms>(), 128);
        assert_eq!(std::mem::size_of::<FloorUniforms>(), 96);
        assert_eq!(std::mem::size_of::<PuffUniforms>(), 96);
        assert_eq!(std::mem::size_of::<PuffInstance>(), 32);
        assert_eq!(std::mem::size_of::<FinalUniforms>(), 16);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("sphere", sphere_wgsl()),
            ("floor", floor_wgsl()),
            ("puff", puff_wgsl()),
            ("final", final_wgsl()),
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
    fn shaders_keep_the_ported_maths() {
        let sphere = sphere_wgsl();
        for needle in [
            "let n1 = p9_noise(p + flow) * 0.5 + 0.5;",
            "let lava_t = clamp(pow(noise, 2.5), 0.0, 1.0);",
            "u.glow.w",
        ] {
            assert!(sphere.contains(needle), "sphere lost {needle}");
        }
        let floor = floor_wgsl();
        assert!(floor.contains("in.world.xz / u.params.x + vec2<f32>(0.5)"));
        assert!(final_wgsl().contains("clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0)"));
    }

    #[test]
    fn props_and_meta_match_the_js_registry() {
        let scene = RollingFire::new();
        assert_eq!(scene.meta().name, "Point10 Rolling Fire");
        assert_eq!(scene.meta().category, "3d-scenes");
        assert_eq!(scene.clear_alpha(), 0.0);
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 7..],
            &[
                "smokeDensity",
                "fireIntensity",
                "smokeLifespan",
                "fireLifespan",
                "simulationSpeed",
                "sceneOffsetX",
                "sceneOffsetY",
            ]
        );
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = RollingFire::new();
        scene.apply_props(|key| match key {
            "smokeDensity" => Some(200.0),
            "fireIntensity" => Some(-3.0),
            "smokeLifespan" => Some(0.0),
            "fireLifespan" => Some(99.0),
            "simulationSpeed" => Some(9.0),
            "sceneOffsetX" => Some(-99.0),
            "sceneOffsetY" => Some(4.5),
            _ => None,
        });
        assert_eq!(scene.smoke_density, 100.0);
        assert_eq!(scene.fire_intensity, 0.0);
        assert_eq!(scene.smoke_lifespan, 0.1);
        assert_eq!(scene.fire_lifespan, 5.0);
        assert_eq!(scene.simulation_speed, 4.0);
        assert_eq!(scene.scene_offset, [-10.0, 4.5]);
    }

    #[test]
    fn update_rolls_the_ball_and_follows_it() {
        use crate::point10::audio_bridge::BandPayload;
        let mut scene = RollingFire::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag = [0.6; AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        for _ in 0..120 {
            scene.update(1.0 / 60.0, &bridge);
        }
        assert!(scene.bass() > 0.0);
        assert!(scene.fire_size() > 1.0);
        assert_ne!(scene.ball(), Ball::new());
        // The camera chases the ball, so it must have moved from its start.
        assert_ne!(scene.camera_position(), CAMERA_START);

        // Without audio the JS update body returns early.
        let frozen = scene.ball();
        scene.update(1.0 / 60.0, &AudioBridge::new());
        assert_eq!(scene.ball(), frozen);
    }
}
