//! RollingFire / BallOfFire — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/RollingFire.js`.
//!
//! The JS scene is **WebGPU/TSL**, a port of three's `webgpu_volume_fire`
//! example: a `100 x 100 x 200` velocity/dye grid (16 x 16 x 32 world units,
//! 6.25 voxels per unit) stepped with seven compute passes per
//! `1/120 * simulationSpeed` second, rendered by an **18-step**
//! `VolumeNodeMaterial` raymarch and bloomed. The teapot is a lava sphere that
//! **rolls with the music**: bass accelerates it, a bass-rise "beat" kicks it,
//! hats and snares puff the fire and the ROYGBIV hue flips on phrase crests.
//! The simulation grid, the camera and the key light all follow the ball.
//!
//! # The volume is the shared GPU simulation
//!
//! The earlier additive-puff substitute is gone. This scene configures
//! [`crate::point10::volume::VolumeSim`] / [`VolumeStage`] (the same engine
//! SmokeRoom and DvdRoom use) at RollingFire's grid, emitter and light values,
//! and runs the JS passes in the JS order: curl noise once at init; then per
//! step advect velocity, divergence, two Jacobi iterations, project, advect dye
//! and the per-vertex sphere emit; then the raymarch → blur → combine → bloom →
//! ACES chain.
//!
//! # What the shared module needed for this scene
//!
//! * **A movable key light.** SmokeRoom's `uKeyLightPos` is a fixed sun at
//!   `(0, 80, 0)` with a `400/d²` fire term; RollingFire tracks a 20 m
//!   overhead spot above the rolling ball with a `25/d²` term. Both are now
//!   [`VolumeUniforms::key_light`] (`xyz` position, `w` numerator).
//! * **A dynamic raymarch step count.** The room scenes march 16 steps,
//!   RollingFire 18 (`:756`); the count rides `VolumeUniforms::misc[1]`.
//! * **A movable volume box.** The raymarch intersected a box centred on the
//!   world origin; RollingFire's grid origin follows the ball, so the box is
//!   now centred on `uvw_to_world(0.5)` (`origin + (0, worldY / 2, 0)`). For
//!   the room scenes that is exactly the old origin.
//! * **The flame sway in the point light.** `distance_to_flame` now adds
//!   `uSway` (`VolumeUniforms::light2.yz`), which the JS applies to the flame
//!   segment both rooms and RollingFire use.
//! * **A scene-selected point-light intensity.** `uPointLightVolumeIntensity`
//!   is 2 in the room scenes and 1.4 here
//!   ([`crate::point10::volume::room::volume_point_light_scaled`]).
//! * **An overlay switch.** `_transparentOverlay` selects `alpha = max(rgb)`
//!   or an opaque frame in the stage's final pass.
//! * The fire ramp / hue maths moved to `volume/fire_colour.wgsl` so the lava
//!   sphere's emissive can apply the exact `fireRamp` + luminance-equalised
//!   `fireHue` the volume uses.
//!
//! # Ported exactly
//!
//! * the ball drive (`:102-176`), the scene's only pure simulation:
//!   [`beat_kick`] (the `riseFrac`/`floor`/`maxRise` gates), [`roll_ball`] (the
//!   `5/7` solid-sphere factor, `ROLL_DAMP`, the `ROLL_MAX_SPEED` cap, the
//!   `BALL_ROVE_RADIUS = 40` rim with `BALL_BOUNCE`), `BALL_ACCEL_K = 60`,
//!   `BALL_BEAT_K = 3`, `BALL_DIR_SPIN = 0.35`, `BALL_DIR_SWAY = 2.0`;
//! * the audio mapping (`:223-291`): bass = mean of the first five bins with
//!   `dt * 8` smoothing, the hi-hat band (top 15%) with `dt * 10`, the hi-hat
//!   spectral centroid `sway = ((c/w - hiStart) / (len - hiStart) - 0.5) * 0.25`
//!   with `BALL_DIR_SWAY`, the snare band (2%..13%) with `dt * 12`,
//!   `uFireSize = 1 + hi * 2.5 + snare * 8`, the `dt * 0.15` camera-yaw
//!   smoothing and the `dropColorPulse` hue flip eased on the shortest arc at
//!   `dt * 1.2`;
//! * the silence path (**R-34**): the JS analyser exists even with no signal,
//!   so `freqMag` is zeros rather than "no update". The scene therefore runs
//!   the same maths with a zero band vector — the bass/hi/snare smoothers decay
//!   to zero, the ball loses its acceleration and coasts to rest, and the sim,
//!   camera, spin and hue ease keep running — instead of freezing;
//! * the simulation clock (`:1329-1370`): the `1/120 * speed` accumulator
//!   capped at eight steps, `uTurbulence = 3.2 / sqrt(speed)`,
//!   `uDissipation = 1 / smokeLifespan`, `uCooling = 1 / fireLifespan`, and the
//!   grid parameters `100 x 100 x 200`, `16 x 16 x 32`, grid origin
//!   `(ball.x, ball.y - worldY/2, ball.z)`;
//! * the emitter (`:446-453`, `:683-749`, `:1297-1327`): a
//!   `SphereGeometry(0.9, 64, 40)` at the ball, with its world matrix (ball
//!   position × `rotationY(time * 0.25)`), the per-vertex noise flicker, the
//!   radial buoyancy and the CPU Perlin-driven flame height / sway / flicker /
//!   colour noise;
//! * the calibration (`:388-441`): `uEmitDensity 16`, `uEmitTemperature 14`,
//!   `uFireIntensity 26`, `uFireGlowSpread 5`, `uBuoyancy 3`, `uWeight 0.15`,
//!   `uTurbulenceDecay 0.1`, `uTurbFrequency 10`, `uVelDamping 0.25`,
//!   `uWindStrength 6.5`, `uShadowAbsorption 2`, `uShadowAmbient 0.5`,
//!   `uMultiScattering 1`, `uPowderStrength 0.59`, `uAsymmetry 0`,
//!   `uPointLightVolumeIntensity 1.4`, the `25/d²` key-light attenuation and
//!   the `fireRamp` stops `0xffe68c` / `0xff7305` / `0xff0000`;
//! * the lava sphere's emissive (`:1000-1034`): three `mx_noise_float` octaves
//!   (`p9_noise` here), `lavaT = noise^2.5`, `fireRamp(lavaT + 0.1)` hue
//!   equalised after `saturation(…, 1.1)`, the
//!   `(emitTemp / 33.36)^4 * emitDensity / 44.08 * fireIntensity * flicker *
//!   fadeIn * (1.5 + bass * 5.3)` scale;
//! * the camera (`:1427-1470`): the world-offset follow
//!   `(9, 3.5, 2.8)` rotated by the bass yaw, `lerp(k = delta * 1.2)`, the two
//!   `ImprovedNoise` jitter terms, `lookAt(ball - 1)` so the floor stays in
//!   frame, and the overhead key light at `ball + 20y`;
//! * the bloom (`:1373-1378`): `min(0.12 + bass * 3.6, 4.0)` at threshold 0.5 /
//!   radius 1, against the JS `bloom()` node (bloom scale 1.0);
//! * the renderer grade (`:335-336`): ACES filmic, exposure 2;
//! * the composite (`:1236-1241`): `alpha = max(rgb)` on transparent black;
//! * the props (`sceneProps.js:46-58`, `bindSmoke`): `smokeDensity` 16,
//!   `fireIntensity` 26, `smokeLifespan` 1.5, `fireLifespan` 0.9,
//!   `simulationSpeed` 1.2, `sceneOffsetX`/`sceneOffsetY`.
//!
//! # Deviations
//!
//! * **No shadow map.** The JS renders a `castShadow` volume mesh with a
//!   custom `volumeCastShadow` node and a 1024² spotlight shadow map onto the
//!   floor, and the sphere `receiveShadow`s it. This renderer has no shadow
//!   map: the *shadow march inside the raymarch* (`scatteringNode`, the Beer /
//!   multi-scatter / powder transmittance toward the key light) is ported —
//!   that is what shades the smoke — while the projected floor shadow of the
//!   fire volume is not drawn. The floor is a rubber-textured plane lit by the
//!   same moving key spot and the fire's bounce light instead.
//! * **No PMREM environment / PBR.** The sphere's `MeshStandardMaterial` base
//!   is black in the JS, so only its emissive node reads; the port keeps the
//!   emissive and a small key term, and the floor uses a flat diffuse +
//!   roughness-folded shading instead of a PMREM studio environment.
//! * `mx_noise_float` becomes [`p9_noise`](crate::point10::shared::p9_glsl) (the
//!   substitution AttractorParticles documents); the sphere's
//!   `moon_04_diff` map is not sampled because the base colour is black.
//! * The point light's *surface* projection uniforms
//!   (`uPointLightProjection*`, `uLightNear/FarIntensity`,
//!   `uPointLightSurfaceIntensity`) only drive the JS's non-volume light
//!   branch, which this renderer does not shade with; the volume branch (the
//!   `volume_point_light` uniform + the 10..15 distance ramp) is ported.
//! * The JS copies `sphere.matrixWorld` into the sim uniforms *before* it
//!   integrates the ball in `_animate`, so the sim lags the ball by one frame.
//!   The port integrates the ball in `update` and steps the sim in `render`
//!   from that fresh state; the one-frame lag is dropped (invisible at 60 Hz
//!   and it makes the ball/camera tests deterministic).
//!
//! # Ball of Fire
//!
//! [`BallOfFire`] is the JS subclass: the same scene with `_freezeBall = true`
//! (the ball stays at its start position while the fire, bloom, colour drops
//! and camera stay fully audio-reactive) and `role = "standalone"` instead of
//! RollingFire's foreground pin. Both are transparent overlays.
//!
//! [`UnrealBloomPass`]: crate::point10::postfx::UnrealBloomPass

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge};
use crate::point10::scenes::cube_mesh::{
    Mat4, Mesh, MeshGpu, Vertex, look_at, mat4_mul, perspective, rotation_y, translation,
};
use crate::point10::scenes::fire_common::build_sphere;
use crate::point10::shared::assets::{ColourSpace, Wrap, load_texture};
use crate::point10::shared::phrase_color::{DropColorState, drop_color_pulse, ease_phrase_hue};
use crate::point10::shared::random::seeded_rand;
use crate::point10::volume::room::{
    CAMERA_FAR, CAMERA_FOV, CAMERA_NEAR, FIRE_END_HEX, FIRE_MID_HEX, FIRE_START_HEX, FireTemporal,
    ImprovedNoise, SPHERE_RADIUS, SimTick, SimulationClock, band, emission_scale, fade_in,
    fire_temporal, snare_band, treble_band, volume_point_light_scaled,
};
use crate::point10::volume::stage::{Material, VolumeStage, volumetric_size};
use crate::point10::volume::{FIRE_COLOUR_WGSL, VolumeGrid, VolumeSim, VolumeUniforms};
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`RollingFire.js:63-138`)
// ---------------------------------------------------------------------------

/// `GRID_SIZE_X`.
pub const GRID_SIZE_X: u32 = 100;
/// `GRID_SIZE_Y`.
pub const GRID_SIZE_Y: u32 = 100;
/// `GRID_SIZE_Z`.
pub const GRID_SIZE_Z: u32 = 200;

/// The JS simulation grid (`100 x 100 x 200`, 2M cells).
pub const GRID: VolumeGrid = VolumeGrid::new(GRID_SIZE_X, GRID_SIZE_Y, GRID_SIZE_Z);

/// `VOLUME_WORLD_SIZE_X`.
pub const VOLUME_WORLD_SIZE_X: f32 = 16.0;
/// `VOLUME_WORLD_SIZE_Y`.
pub const VOLUME_WORLD_SIZE_Y: f32 = 16.0;
/// `VOLUME_WORLD_SIZE_Z`.
pub const VOLUME_WORLD_SIZE_Z: f32 = 32.0;

/// `volumetricMaterial.steps = 18` (`:756`).
pub const RAYMARCH_STEPS: u32 = 18;

/// `uKeyLightPos = ball + 20y` (`:1457`) and the `25/d²` fire attenuation
/// (`:870`).
pub const KEY_LIGHT_HEIGHT: f32 = 20.0;
/// `25.0 / distance²`.
pub const KEY_LIGHT_ATTENUATION: f32 = 25.0;
/// `uPointLightVolumeIntensity` (`:426`).
pub const POINT_LIGHT_VOLUME_INTENSITY: f32 = 1.4;

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

/// `uEmitDensity` default.
pub const EMIT_DENSITY: f32 = 16.0;
/// `uEmitTemperature` default.
pub const EMIT_TEMPERATURE: f32 = 14.0;
/// `uFireIntensity` default.
pub const FIRE_INTENSITY: f32 = 26.0;
/// `uFireGlowSpread`.
pub const FIRE_GLOW_SPREAD: f32 = 5.0;
/// `uBuoyancy`.
pub const BUOYANCY: f32 = 3.0;
/// `uWeight`.
pub const WEIGHT: f32 = 0.15;
/// `uTurbulenceDecay`.
pub const TURBULENCE_DECAY: f32 = 0.1;
/// `uTurbFrequency`.
pub const TURB_FREQUENCY: f32 = 10.0;
/// `uVelDamping`.
pub const VEL_DAMPING: f32 = 0.25;
/// `uWindStrength`.
pub const WIND_STRENGTH: f32 = 6.5;
/// `uShadowAbsorption`.
pub const SHADOW_ABSORPTION: f32 = 2.0;
/// `uShadowAmbient`.
pub const SHADOW_AMBIENT: f32 = 0.5;
/// `uMultiScattering`.
pub const MULTI_SCATTERING: f32 = 1.0;
/// `uPowderStrength`.
pub const POWDER_STRENGTH: f32 = 0.59;
/// `uAsymmetry`.
pub const ASYMMETRY: f32 = 0.0;

/// `this._simSpeed`.
pub const SIMULATION_SPEED: f32 = 1.2;
/// `this._smokeLifespan`.
pub const SMOKE_LIFESPAN: f32 = 1.5;
/// `this._fireLifespan`.
pub const FIRE_LIFESPAN: f32 = 0.9;

/// `camera.position.set(9, 3.5, 2.8)`.
pub const CAMERA_START: [f32; 3] = [9.0, 3.5, 2.8];
/// `PerspectiveCamera(60, aspect, 0.1, 100)`.
pub const CAMERA_FOV_DEGREES: f32 = CAMERA_FOV;
/// Near plane.
pub const CAMERA_NEAR_PLANE: f32 = CAMERA_NEAR;
/// Far plane.
pub const CAMERA_FAR_PLANE: f32 = CAMERA_FAR;

/// `floorGroup.position.y = -VY/2 + 0.4 + VY/2 + 0.4`.
pub const FLOOR_Y: f32 = 0.8;
/// `FLOOR_TILE` — world metres per texture tile.
pub const FLOOR_TILE: f32 = 10.0;
/// `FLOOR_PLANE`.
pub const FLOOR_PLANE: f32 = 400.0;
/// `floorMat.opacity`.
pub const FLOOR_OPACITY: f32 = 0.45;

/// `scene.background = null` (`:343-345`).
pub const CLEAR_ALPHA: f64 = 0.0;

/// `rubber002_diff_1k.jpg`.
pub const FLOOR_DIFFUSE_ASSET: &str = "rubber002_diff_1k.jpg";

/// The sphere floats `SPHERE_RADIUS` above the level floor (`:1419-1423`).
pub const SPHERE_Y: f32 = FLOOR_Y + SPHERE_RADIUS;

// ---------------------------------------------------------------------------
// Pure maths (`RollingFire.js:102-176`)
// ---------------------------------------------------------------------------

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

/// The lava sphere's emissive scale (`:1000-1034`), including the flicker, the
/// 3-second fade-in and the bass self-glow `1.5 + bass * 5.3`.
pub fn sphere_glow(
    emit_temperature: f32,
    emit_density: f32,
    fire_intensity: f32,
    temporal: &FireTemporal,
    bass: f32,
) -> f32 {
    emission_scale(emit_temperature, emit_density, fire_intensity)
        * temporal.flicker
        * fade_in(temporal.time)
        * (1.5 + bass.max(0.0) * 5.3)
}

/// The `fireRamp` stops in linear-sRGB, as `THREE.Color(hex)` converts them.
pub fn ramp_stops() -> ([f32; 3], [f32; 3], [f32; 3]) {
    let linear = crate::point10::shared::phrase_color::srgb_hex_to_linear;
    (
        linear(FIRE_START_HEX),
        linear(FIRE_MID_HEX),
        linear(FIRE_END_HEX),
    )
}

/// `this.role`: which layer slot the scene claims in the JS adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneRole {
    /// Rolling Fire composites over another scene as the performance
    /// foreground layer (`:185`).
    Foreground,
    /// Ball of Fire plays standalone (`:1489`).
    Standalone,
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

/// The lava sphere's uniform block (208 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SphereUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// rgb `uFireStartColor`, w `uFireIntensity`.
    fire_start: [f32; 4],
    /// rgb `uFireMidColor`, w `uFireGlowSpread`.
    fire_mid: [f32; 4],
    /// rgb `uFireEndColor`, w `uFireHue`.
    fire_end: [f32; 4],
    /// x `uTime`, y emissive scale, z `_bass`, w `uFlicker`.
    params: [f32; 4],
}

/// The floor's uniform block (144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FloorUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    /// xyz key light position, w tile repeat (`FLOOR_PLANE / FLOOR_TILE`).
    light: [f32; 4],
    /// rgb fire bounce colour, w its intensity.
    fire: [f32; 4],
    /// x time, y opacity, z ball x, w ball z.
    params: [f32; 4],
    /// x ball y, yzw unused.
    misc: [f32; 4],
}

/// The floor: a textured plane needs a texture + sampler beside its uniform
/// block, which [`Material`] does not model.
struct FloorMaterial {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl FloorMaterial {
    fn new(
        device: &wgpu::Device,
        shader_source: &str,
        texture: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-rolling-fire-floor"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-rolling-fire-floor-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-rolling-fire-floor-uniforms"),
            size: std::mem::size_of::<FloorUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-rolling-fire-floor-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-rolling-fire-floor-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-rolling-fire-floor-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(crate::point10::scenes::cube_mesh::depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: crate::point10::HDR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniforms,
            bind_group,
        }
    }

    fn write(&self, queue: &wgpu::Queue, bytes: &[u8]) {
        queue.write_buffer(&self.uniforms, 0, bytes);
    }

    fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, mesh: &'pass MeshGpu) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        mesh.draw(pass);
    }
}

/// GPU resources built in `init`.
struct RollingFireGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    sim: VolumeSim,
    stage: VolumeStage,
    sphere: Material,
    floor: FloorMaterial,
    sphere_mesh: MeshGpu,
    floor_mesh: MeshGpu,
    _floor_texture: wgpu::Texture,
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
    /// `this.sphere.rotation.y` (`+= delta * 0.25`).
    ball_spin: f32,
    /// `this._uFireHue`.
    fire_hue: f32,
    /// `this._pulse`.
    pulse: DropColorState,
    /// The seed stream behind the drop detector.
    seed: u32,
    /// `this.time` (`BaseSceneWebGPU.update` accumulates the frame delta).
    time: f32,
    /// `this._simAccumulator` / `this._simulationTime`.
    clock: SimulationClock,
    /// `this._cpuNoise`.
    noise: ImprovedNoise,
    /// The raymarch dither's `frameId`.
    frame: u32,
    /// The volume uniforms for the steps accumulated this frame (built in
    /// `update`, submitted in `render`).
    pending_steps: Vec<VolumeUniforms>,
    /// The idle uniform refresh when this frame produced no simulation step.
    pending_idle: Option<VolumeUniforms>,
    /// The temporal state the geometry/materials were last built from.
    last_temporal: FireTemporal,
    /// `this._freezeBall` (Ball of Fire pins the ball in place).
    freeze_ball: bool,
    /// `this._transparentOverlay`.
    transparent_overlay: bool,
    /// Props.
    smoke_density: f32,
    fire_intensity: f32,
    smoke_lifespan: f32,
    fire_lifespan: f32,
    simulation_speed: f32,
    scene_offset: [f32; 2],
    camera_aspect: f32,
    size: (u32, u32),
    /// The simulation grid (`GRID` unless a test scales it).
    grid: VolumeGrid,
    gpu: Option<RollingFireGpu>,
}

impl Default for RollingFire {
    fn default() -> Self {
        Self::new()
    }
}

impl RollingFire {
    /// A scene with a different simulation grid (tests and the offline
    /// reference renderer use a smaller grid; the scene defaults to the JS
    /// `100 x 100 x 200`).
    pub fn with_grid(grid: VolumeGrid) -> Self {
        let mut scene = Self::new();
        scene.grid = grid;
        scene
    }

    /// Create the scene (`RollingFire.js:178-221` defaults).
    pub fn new() -> Self {
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
            ball_spin: 0.0,
            fire_hue: 0.0,
            pulse: DropColorState::new(),
            seed: 0,
            time: 0.0,
            clock: SimulationClock::default(),
            noise: ImprovedNoise,
            frame: 0,
            pending_steps: Vec::new(),
            pending_idle: None,
            last_temporal: fire_temporal(&ImprovedNoise, 0.0),
            freeze_ball: false,
            transparent_overlay: true,
            smoke_density: EMIT_DENSITY,
            fire_intensity: FIRE_INTENSITY,
            smoke_lifespan: SMOKE_LIFESPAN,
            fire_lifespan: FIRE_LIFESPAN,
            simulation_speed: SIMULATION_SPEED,
            scene_offset: [0.0; 2],
            camera_aspect: 1.0,
            size: (1, 1),
            grid: GRID,
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

    /// `this._ballDir`.
    pub fn ball_direction(&self) -> f32 {
        self.ball_direction
    }

    /// `this._kickX`/`_kickZ` (zeroed once consumed).
    pub fn kick(&self) -> [f32; 3] {
        self.kick
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

    /// `this._camYaw`.
    pub fn camera_yaw(&self) -> f32 {
        self.camera_yaw
    }

    /// The simulation clock's accumulated time.
    pub fn simulation_time(&self) -> f32 {
        self.clock.time()
    }

    /// The simulation grid the scene runs (`GRID` by default).
    pub fn grid(&self) -> VolumeGrid {
        self.grid
    }

    /// `this._freezeBall`.
    pub fn frozen(&self) -> bool {
        self.freeze_ball
    }

    /// `this._transparentOverlay`.
    pub fn transparent_overlay(&self) -> bool {
        self.transparent_overlay
    }

    /// `this.role`.
    pub fn role(&self) -> SceneRole {
        SceneRole::Foreground
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
        [self.ball.x, SPHERE_Y, self.ball.z]
    }

    /// `uGridOrigin` — the grid's bottom-centre, `worldY / 2` below the ball
    /// (`:1322-1326`).
    pub fn grid_origin(&self) -> [f32; 3] {
        let sphere = self.sphere_position();
        [sphere[0], sphere[1] - VOLUME_WORLD_SIZE_Y / 2.0, sphere[2]]
    }

    /// `this._keyLight.position` (`:1457`).
    pub fn key_light_position(&self) -> [f32; 3] {
        let sphere = self.sphere_position();
        [sphere[0], sphere[1] + KEY_LIGHT_HEIGHT, sphere[2]]
    }

    /// The camera's look-at target (`:1449-1453`).
    pub fn camera_target(&self) -> [f32; 3] {
        let sphere = self.sphere_position();
        [
            sphere[0] + self.scene_offset[0],
            sphere[1] - FLOOR_AIM_DOWN + self.scene_offset[1],
            sphere[2],
        ]
    }

    /// `uSphereMatrix`: `translation(ball) * rotationY(spin)`.
    pub fn emitter_model(&self) -> Mat4 {
        let sphere = self.sphere_position();
        mat4_mul(
            &translation(sphere[0], sphere[1], sphere[2]),
            &rotation_y(self.ball_spin),
        )
    }

    /// The lava sphere's emissive scale for a temporal state (`:1000-1034`).
    pub fn sphere_glow(&self, temporal: &FireTemporal) -> f32 {
        sphere_glow(
            EMIT_TEMPERATURE,
            self.smoke_density,
            self.fire_intensity,
            temporal,
            self.bass,
        )
    }

    /// The fire bounce light the floor picks up (`:1051-1171`).
    pub fn fire_light(&self, temporal: &FireTemporal) -> [f32; 3] {
        volume_point_light_scaled(
            temporal,
            EMIT_TEMPERATURE,
            self.smoke_density,
            self.fire_intensity,
            self.fire_hue,
            POINT_LIGHT_VOLUME_INTENSITY,
        )
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV_DEGREES.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR_PLANE,
                CAMERA_FAR_PLANE,
            ),
            &look_at(self.camera, self.camera_target(), [0.0, 1.0, 0.0]),
        )
    }

    fn volume_uniforms(&self, temporal: &FireTemporal, tick: &SimTick) -> VolumeUniforms {
        let sphere = self.sphere_position();
        let mut uniforms = VolumeUniforms::for_grid(self.grid);
        uniforms.grid[3] = tick.step;
        uniforms.world = [
            VOLUME_WORLD_SIZE_X,
            VOLUME_WORLD_SIZE_Y,
            VOLUME_WORLD_SIZE_Z,
            temporal.time,
        ];
        let origin = self.grid_origin();
        uniforms.origin = [origin[0], origin[1], origin[2], 1.0];
        // `uSpherePosition`; `uSphereSpeed` and `uSphereVelocity` are declared
        // in the JS but never written, so they stay 0 (the emitter's wind is
        // therefore still, exactly like the JS).
        uniforms.sphere_pos = [sphere[0], sphere[1], sphere[2], 0.0];
        uniforms.sphere_vel = [0.0, 0.0, 0.0, 0.25];
        uniforms.camera = [
            self.camera[0],
            self.camera[1],
            self.camera[2],
            self.frame as f32,
        ];
        uniforms.set_camera(self.camera, self.camera_target(), [0.0, 1.0, 0.0]);

        let (start, mid, end) = ramp_stops();
        uniforms.fire_start = [start[0], start[1], start[2], self.fire_intensity];
        uniforms.fire_mid = [mid[0], mid[1], mid[2], FIRE_GLOW_SPREAD];
        uniforms.fire_end = [end[0], end[1], end[2], self.fire_hue];

        uniforms.turb = [BUOYANCY, WEIGHT, tick.turbulence, TURBULENCE_DECAY];
        uniforms.turb2 = [TURB_FREQUENCY, VEL_DAMPING, tick.cooling, tick.dissipation];
        uniforms.emit = [self.smoke_density, EMIT_TEMPERATURE, self.fire_size, 1.0];
        uniforms.emit2 = [WIND_STRENGTH, temporal.flicker, temporal.flame_height, 0.0];
        uniforms.light = [
            SHADOW_ABSORPTION,
            SHADOW_AMBIENT,
            MULTI_SCATTERING,
            POWDER_STRENGTH,
        ];
        // `uAsymmetry` plus the `uSway` the point light's flame segment reads.
        uniforms.light2 = [ASYMMETRY, temporal.sway[0], temporal.sway[2], 0.0];
        let fire = self.fire_light(temporal);
        uniforms.point_light = [fire[0], fire[1], fire[2], 0.0];
        uniforms.set_key_light(self.key_light_position(), KEY_LIGHT_ATTENUATION);
        let (volume_width, volume_height) = volumetric_size(self.size.0, self.size.1);
        uniforms.view = [
            (CAMERA_FOV_DEGREES.to_radians() * 0.5).tan(),
            self.camera_aspect,
            volume_width as f32,
            volume_height as f32,
        ];
        uniforms.set_raymarch_steps(RAYMARCH_STEPS);
        uniforms.set_emitter_model(self.emitter_model());
        uniforms
    }

    fn sphere_uniforms(&self, temporal: &FireTemporal) -> SphereUniforms {
        let (start, mid, end) = ramp_stops();
        SphereUniforms {
            view_proj: self.view_proj(),
            model: self.emitter_model(),
            camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
            fire_start: [start[0], start[1], start[2], self.fire_intensity],
            fire_mid: [mid[0], mid[1], mid[2], FIRE_GLOW_SPREAD],
            fire_end: [end[0], end[1], end[2], self.fire_hue],
            params: [
                temporal.time,
                self.sphere_glow(temporal),
                self.bass,
                temporal.flicker,
            ],
        }
    }

    fn floor_uniforms(&self, temporal: &FireTemporal) -> FloorUniforms {
        let light = self.key_light_position();
        let fire = self.fire_light(temporal);
        FloorUniforms {
            view_proj: self.view_proj(),
            camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
            light: [light[0], light[1], light[2], FLOOR_PLANE / FLOOR_TILE],
            fire: [
                fire[0],
                fire[1],
                fire[2],
                self.fire_intensity / FIRE_INTENSITY,
            ],
            params: [self.time, FLOOR_OPACITY, self.ball.x, self.ball.z],
            misc: [self.sphere_position()[1], 0.0, 0.0, 0.0],
        }
    }
}

#[cfg(test)]
impl RollingFire {
    /// The live simulation, for the debug readback in the reference renderer.
    pub(crate) fn debug_sim(&self) -> Option<&VolumeSim> {
        self.gpu.as_ref().map(|gpu| &gpu.sim)
    }
}

#[cfg(test)]
impl BallOfFire {
    /// The live simulation, for the debug readback in the reference renderer.
    pub(crate) fn debug_sim(&self) -> Option<&VolumeSim> {
        self.inner.debug_sim()
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

        // The emitter mesh is also the simulation's seed geometry: the JS's
        // `SphereGeometry(SPHERE_RADIUS, 64, 40)` (`:447`), 65 x 41 vertices.
        let sphere = build_sphere(SPHERE_RADIUS, 64, 40);
        let emitter_positions: Vec<[f32; 3]> = sphere
            .vertices
            .iter()
            .map(|vertex| vertex.position)
            .collect();

        let sim = VolumeSim::new(&device, &queue, self.grid, &emitter_positions);
        let mut stage =
            VolumeStage::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        stage.set_overlay(&queue, self.transparent_overlay);

        let loader = ctx.asset_loader();
        let floor_texture = load_texture(
            &device,
            &queue,
            &loader,
            FLOOR_DIFFUSE_ASSET,
            ColourSpace::Srgb,
            Wrap::Repeat,
        );

        let sphere_material = Material::new(
            &device,
            "point10-rolling-fire-sphere",
            &sphere_program(),
            std::mem::size_of::<SphereUniforms>() as u64,
            Some(Vertex::layout()),
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            Some(crate::point10::scenes::cube_mesh::depth_state()),
            None,
        );
        let floor_material = FloorMaterial::new(
            &device,
            FLOOR_WGSL,
            &floor_texture.view,
            &floor_texture.sampler,
        );

        let sphere_mesh = MeshGpu::new(&device, "point10-rolling-fire-sphere-mesh", &sphere);
        let floor_mesh = MeshGpu::new(
            &device,
            "point10-rolling-fire-floor-mesh",
            &build_floor(FLOOR_PLANE, FLOOR_Y),
        );

        self.gpu = Some(RollingFireGpu {
            device,
            queue,
            sim,
            stage,
            sphere: sphere_material,
            floor: floor_material,
            sphere_mesh,
            floor_mesh,
            _floor_texture: floor_texture.texture,
        });
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let device = gpu.device.clone();
        let queue = gpu.queue.clone();
        let (width, height) = self.size;
        gpu.stage.resize(&device, &queue, width, height);
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

        // R-34: the JS analyser exists even with no signal, so a silent input
        // is a zero `freqMag`, not a skipped update. Using a zero band vector
        // keeps the smoothers decaying, the ball coasting, the hue easing, the
        // spin turning and the sim stepping — exactly what the JS does in
        // silence — while every drive term that divides by a band energy is
        // naturally zero (the `hi_sway` guard returns 0).
        let zero_bands = [0.0_f32; AUDIO_SAMPLES];
        let freq_mag = if audio.is_available() {
            &audio.freq_mag
        } else {
            &zero_bands
        };

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

        let snare = snare_band(freq_mag);
        self.snare += (snare - self.snare) * (delta * 12.0).min(1.0);
        self.fire_size = fire_size(self.hi, self.snare);

        // The drop detector + shortest-arc hue ease (`:287-290`).
        let sample = seeded_rand(0, self.seed as i32);
        if drop_color_pulse(&mut self.pulse, self.bass, delta, sample) {
            self.seed = self.seed.wrapping_add(1);
        }
        self.fire_hue = ease_phrase_hue(self.fire_hue, self.pulse.hue, delta, 1.2);

        // The ball (`:1403-1424`): bass push + beat kick, rolled on the level
        // floor. Ball of Fire skips the whole block, so its ball is pinned at
        // the start position.
        if !self.freeze_ball {
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
        }
        self.kick = [0.0; 3];
        self.ball_spin += delta * 0.25;

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
        let (noise_x, noise_z) = (
            self.noise.noise(self.time * 0.25, 0.0, 0.0),
            self.noise.noise(0.0, 0.0, self.time * 0.3),
        );
        self.camera = [
            self.camera[0] + (desired[0] - self.camera[0]) * lerp + noise_x * 0.15,
            self.camera[1] + (desired[1] - self.camera[1]) * lerp,
            self.camera[2] + (desired[2] - self.camera[2]) * lerp + noise_z * 0.15,
        ];

        // The JS `_animate` accumulator (`:1335-1370`): `1/120 * speed` steps,
        // capped at eight per frame, each with its own temporal uniforms. The
        // steps are recorded here (so the clock advances with the scene) and
        // submitted by `render`, which owns the queue.
        let tick = self.clock.pending(
            delta,
            self.simulation_speed,
            self.smoke_lifespan,
            self.fire_lifespan,
        );
        self.pending_steps.clear();
        let mut temporal = fire_temporal(&self.noise, self.clock.time());
        for index in 0..tick.steps {
            let step_temporal = fire_temporal(&self.noise, tick.time_of(index));
            self.pending_steps
                .push(self.volume_uniforms(&step_temporal, &tick));
            temporal = step_temporal;
        }
        // With no step this frame, still refresh the uniforms so the raymarch
        // and the emitters keep animating.
        self.pending_idle = if self.pending_steps.is_empty() {
            Some(self.volume_uniforms(&temporal, &tick))
        } else {
            None
        };
        self.last_temporal = temporal;
        self.frame = self.frame.wrapping_add(1);
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let sphere_uniforms = self.sphere_uniforms(&self.last_temporal);
        let floor_uniforms = self.floor_uniforms(&self.last_temporal);
        let bloom_strength = self.bloom_strength();
        let idle_uniforms = self.pending_idle.take();
        let steps = std::mem::take(&mut self.pending_steps);

        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let device = gpu.device.clone();
        let queue = gpu.queue.clone();

        if let Some(uniforms) = idle_uniforms {
            gpu.sim.write(&queue, &uniforms);
        }
        for uniforms in &steps {
            let mut sim_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("point10-rolling-fire-sim"),
            });
            gpu.sim.write(&queue, uniforms);
            gpu.sim.step(&mut sim_encoder);
            queue.submit(Some(sim_encoder.finish()));
        }

        gpu.sphere
            .write(&queue, bytemuck::bytes_of(&sphere_uniforms));
        gpu.floor.write(&queue, bytemuck::bytes_of(&floor_uniforms));

        {
            // The opaque lava sphere, then the semi-transparent floor over it
            // (three renders opaque geometry before transparent geometry).
            let mut pass = gpu.stage.begin_scene(encoder);
            gpu.sphere.draw(&mut pass, &gpu.sphere_mesh);
            gpu.floor.draw(&mut pass, &gpu.floor_mesh);
        }

        gpu.stage
            .encode_post(encoder, &gpu.sim, bloom_strength, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(ROLLING_FIRE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 Rolling Fire", "3d-scenes")
    }
}

// ---------------------------------------------------------------------------
// Ball of Fire (`RollingFire.js:1477-1491`)
// ---------------------------------------------------------------------------

/// `Ball of Fire` — RollingFire with `_freezeBall = true`: the ball is pinned
/// at its start position, floating above the floor at a constant height, while
/// the fire, bloom, colour drops, camera and floor stay fully audio-reactive.
///
/// It writes its own meta (`"Point10 Ball Of Fire"`) and identical props
/// (`Point10BallOfFire.js` passes the same `BallOfFire` class to
/// `createPoint10Module`, so `sceneProps.js` resolves the same schema).
pub struct BallOfFire {
    inner: RollingFire,
}

impl Default for BallOfFire {
    fn default() -> Self {
        Self::new()
    }
}

impl BallOfFire {
    /// A frozen-ball scene at the JS grid.
    pub fn new() -> Self {
        let mut inner = RollingFire::new();
        inner.freeze_ball = true;
        // Not a layer: plays standalone (only Rolling Fire itself is
        // foreground-pinned), and its canvas has no background of its own
        // either (`:1486-1489`).
        inner.transparent_overlay = true;
        Self { inner }
    }

    /// A frozen-ball scene at a custom grid (tests / the reference renderer).
    pub fn with_grid(grid: VolumeGrid) -> Self {
        let mut scene = Self::new();
        scene.inner.grid = grid;
        scene
    }

    /// The shared scene state.
    pub fn inner(&self) -> &RollingFire {
        &self.inner
    }

    /// `this.role` — `"standalone"`, not the foreground pin.
    pub fn role(&self) -> SceneRole {
        SceneRole::Standalone
    }

    /// `this._freezeBall`.
    pub fn frozen(&self) -> bool {
        self.inner.freeze_ball
    }

    /// `this._transparentOverlay`.
    pub fn transparent_overlay(&self) -> bool {
        self.inner.transparent_overlay
    }

    /// Apply the shared RollingFire prop schema (the two scenes are identical
    /// apart from the frozen ball).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        self.inner.apply_props(props);
    }
}

impl Point10Scene for BallOfFire {
    fn name(&self) -> &'static str {
        "Point10 Ball Of Fire"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        self.inner.init(ctx);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.inner.resize(width, height);
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        self.inner.update(dt, audio);
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        self.inner.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.inner.dispose();
    }

    fn props(&self) -> Vec<SceneProp> {
        self.inner.props()
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 Ball Of Fire", "3d-scenes")
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

/// The lava sphere: `fire_colour.wgsl` + the Plane9 noise + the sphere material.
fn sphere_program() -> String {
    format!(
        "{FIRE_COLOUR_WGSL}\n{}\n{SPHERE_WGSL}",
        crate::point10::shared::p9_glsl::P9_COMMON_WGSL
    )
}

/// The sphere/floor shader sources, for the naga validation test.
#[cfg(test)]
fn shader_programs() -> Vec<(&'static str, String)> {
    vec![
        ("sphere", sphere_program()),
        ("floor", FLOOR_WGSL.to_string()),
    ]
}

const SPHERE_WGSL: &str = r#"
struct SphereUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // rgb = uFireStartColor, w = uFireIntensity
  fire_start: vec4<f32>,
  // rgb = uFireMidColor, w = uFireGlowSpread
  fire_mid: vec4<f32>,
  // rgb = uFireEndColor, w = uFireHue
  fire_end: vec4<f32>,
  // x uTime, y emissive scale, z bass, w uFlicker
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: SphereUniforms;

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
  let world = u.model * vec4<f32>(position, 1.0);
  var out: VsOut;
  out.position = u.view_proj * world;
  out.normal = normalize((u.model * vec4<f32>(normal, 0.0)).xyz);
  out.local = position;
  out.world = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // `emissiveNode` (`RollingFire.js:1000-1034`): three mx_noise_float octaves
  // flowing down the sphere, `lavaT = noise^2.5`, `fireRamp(lavaT + 0.1)`.
  let p = in.local * 0.5;
  let flow = vec3<f32>(0.0, -u.params.x, 0.0);
  let n1 = p9_noise(p + flow) * 0.5 + 0.5;
  let p2 = p * 2.0 - flow * 1.5;
  let n2 = p9_noise(p2 + vec3<f32>(n1 * 0.4)) * 0.5 + 0.5;
  let p3 = p * 4.0 + flow * 2.5;
  let n3 = p9_noise(p3) * 0.5 + 0.5;
  let noise = n1 * 0.5 + n2 * 0.35 + n3 * 0.15;
  let lava_t = clamp(pow(noise, 2.5), 0.0, 1.0);

  let ramp = fire_ramp_stops(
    clamp(lava_t + 0.1, 0.0, 1.0),
    u.fire_start.rgb,
    u.fire_mid.rgb,
    u.fire_end.rgb,
  );
  // `fireHue(saturation(fireRamp(...), 1.1), uFireHue)`.
  let coloured = fire_hue_stops(saturation(ramp, 1.1), u.fire_end.w);
  // `u.params.y` is the whole CPU-side scalar
  // `(emitTemp/33.36)^4 * emitDensity/44.08 * uFireIntensity * uFlicker *
  // fadeIn * (1.5 + bass * 5.3)`.
  let emissive = coloured * u.params.y;
  // `MeshStandardMaterial({ color: 0x000000, roughness: 1 })`: a nearly black
  // base, so only a small key term keeps the dark side from being a hole.
  let normal = normalize(in.normal);
  let key = max(dot(normal, normalize(vec3<f32>(0.0, 1.0, 0.35))), 0.0) * 0.05;
  return vec4<f32>(emissive + u.fire_start.rgb * key * u.params.y, 1.0);
}
"#;

const FLOOR_WGSL: &str = r#"
struct FloorUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // xyz key light position, w tile repeat
  light: vec4<f32>,
  // rgb fire bounce colour, w intensity
  fire: vec4<f32>,
  // x time, y opacity, z ball x, w ball z
  params: vec4<f32>,
  // x ball y
  misc: vec4<f32>,
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
  // `PlaneGeometry(400, 400)` with `repeat = FLOOR_PLANE / FLOOR_TILE`.
  let uv = in.world.xz / u.light.w + vec2<f32>(0.5);
  let albedo = textureSample(floor_map, floor_sampler, uv).rgb;

  // The key `SpotLight(0xffffff, 700, 0, 0.4, 0.6, 2)` aimed straight down at
  // the ball (`:1184-1196`): a 0.4 rad cone with a 0.6 penumbra, so only a pool
  // below the fire is lit — the rest of the 400 m plane stays in the void.
  let to_key = u.light.xyz - in.world;
  let key_d2 = max(dot(to_key, to_key), 1.0);
  let key_dir = to_key * inverseSqrt(key_d2);
  let cos_theta = clamp(-key_dir.y, -1.0, 1.0);
  // three: smoothstep(cos(angle), cos(angle * (1 - penumbra)), cosTheta).
  let outer = cos(0.4);
  let inner = cos(0.4 * (1.0 - 0.6));
  let cone = smoothstep(outer, inner, cos_theta);
  // Irradiance `700 / d²` (candela) and the Lambert `1 / PI` response.
  let key = 700.0 / key_d2 * 0.31830989 * cone * max(cos_theta, 0.0);

  // The fire's bounce: the volume point light sits on the ball.
  let to_fire = vec3<f32>(u.params.z, u.misc.x, u.params.w) - in.world;
  let fire_d2 = dot(to_fire, to_fire);
  let fire_dir = to_fire * inverseSqrt(max(fire_d2, 1e-4));
  let fire = u.fire.w * 2.2 / (1.0 + fire_d2 * 0.6) * max(fire_dir.y, 0.0);

  let lit = albedo * (0.02 + key) + albedo * u.fire.rgb * fire;
  return vec4<f32>(lit, u.params.y);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    fn loud_bridge() -> AudioBridge {
        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag = [0.6; AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());
        bridge
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
        let noise = ImprovedNoise;
        let early = fire_temporal(&noise, 0.0);
        assert_eq!(
            sphere_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, &early, 1.0),
            0.0
        );
        let late = fire_temporal(&noise, 3.0);
        let quiet = sphere_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, &late, 0.0);
        let loud = sphere_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, &late, 1.0);
        assert!(quiet > 0.0);
        close(loud / quiet, (1.5 + 5.3) / 1.5, 1e-5);
        // The JS's double flicker (the emissive node itself multiplies it).
        let flat = FireTemporal {
            flicker: 1.0,
            ..late
        };
        assert!(sphere_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, &flat, 0.0) > quiet);
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
        close(SPHERE_Y, 1.7, 1e-6);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!((GRID_SIZE_X, GRID_SIZE_Y, GRID_SIZE_Z), (100, 100, 200));
        assert_eq!(GRID.cell_count(), 2_000_000);
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
        assert_eq!((RAYMARCH_STEPS, KEY_LIGHT_HEIGHT), (18, 20.0));
        assert_eq!(KEY_LIGHT_ATTENUATION, 25.0);
        assert_eq!(POINT_LIGHT_VOLUME_INTENSITY, 1.4);
        assert_eq!((SIMULATION_SPEED, SMOKE_LIFESPAN), (1.2, 1.5));
        assert_eq!(FIRE_LIFESPAN, 0.9);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<SphereUniforms>(), 208);
        assert_eq!(std::mem::size_of::<FloorUniforms>(), 144);
    }

    #[test]
    fn the_volume_uniforms_carry_the_scene_state() {
        let mut scene = RollingFire::new();
        scene.camera_aspect = 16.0 / 9.0;
        scene.size = (256, 128);
        scene.fire_size = 2.0;
        scene.fire_hue = 1.25;
        scene.ball = Ball {
            x: 3.0,
            z: -4.0,
            vx: 0.0,
            vz: 0.0,
        };
        scene.ball_spin = std::f32::consts::FRAC_PI_2;
        let temporal = fire_temporal(&ImprovedNoise, 10.0);
        let tick = SimTick {
            steps: 2,
            step: 0.01,
            start_time: 0.0,
            turbulence: 3.2 / 1.2_f32.sqrt(),
            dissipation: 1.0 / 1.5,
            cooling: 1.0 / 0.9,
        };
        let uniforms = scene.volume_uniforms(&temporal, &tick);

        assert_eq!(uniforms.grid, [100.0, 100.0, 200.0, 0.01]);
        assert_eq!(uniforms.world, [16.0, 16.0, 32.0, temporal.time]);
        // The grid rides the ball; the ball sits worldY/2 above the origin.
        close(uniforms.origin[0], 3.0, 1e-6);
        close(uniforms.origin[1], SPHERE_Y - 8.0, 1e-6);
        close(uniforms.origin[2], -4.0, 1e-6);
        close(uniforms.origin[3], 1.0, 1e-6);
        assert_eq!(uniforms.sphere_pos, [3.0, SPHERE_Y, -4.0, 0.0]);
        // `uSphereSpeed` / `uSphereVelocity` are never written in the JS.
        assert_eq!(uniforms.sphere_pos[3], 0.0);
        assert_eq!(uniforms.sphere_vel, [0.0, 0.0, 0.0, 0.25]);
        // Key light is 20 m above the ball with RollingFire's 25/d² term.
        assert_eq!(uniforms.key_light, [3.0, SPHERE_Y + 20.0, -4.0, 25.0]);
        assert_eq!(uniforms.misc[1], 18.0);
        close(uniforms.emit[0], 16.0, 1e-6);
        close(uniforms.emit[1], 14.0, 1e-6);
        close(uniforms.emit[2], 2.0, 1e-6);
        close(uniforms.fire_start[3], 26.0, 1e-6);
        close(uniforms.fire_mid[3], 5.0, 1e-6);
        close(uniforms.fire_end[3], 1.25, 1e-6);
        assert_eq!(uniforms.turb, [3.0, 0.15, tick.turbulence, 0.1]);
        assert_eq!(uniforms.turb2, [10.0, 0.25, tick.cooling, tick.dissipation]);
        assert_eq!(uniforms.light, [2.0, 0.5, 1.0, 0.59]);
        close(uniforms.light2[1], temporal.sway[0], 1e-6);
        close(uniforms.light2[2], temporal.sway[2], 1e-6);
        // Half-resolution raymarch target of a 256x128 frame.
        assert_eq!(uniforms.view[2..4], [128.0, 64.0]);
        close(uniforms.view[0], (60.0_f32.to_radians() * 0.5).tan(), 1e-6);
        // The emitter model is the ball's world matrix.
        let model = uniforms.emitter_model;
        for row in model {
            assert!(row.iter().all(|value| value.is_finite()));
        }
        close(model[3][0], 3.0, 1e-6);
        close(model[3][1], SPHERE_Y, 1e-6);
        close(model[3][2], -4.0, 1e-6);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in shader_programs() {
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
        let sphere = sphere_program();
        for needle in [
            "let n1 = p9_noise(p + flow) * 0.5 + 0.5;",
            "let lava_t = clamp(pow(noise, 2.5), 0.0, 1.0);",
            "fire_hue_stops(saturation(ramp, 1.1), u.fire_end.w)",
            "u.params.y",
        ] {
            assert!(sphere.contains(needle), "sphere lost {needle}");
        }
        let floor = FLOOR_WGSL;
        assert!(floor.contains("in.world.xz / u.light.w + vec2<f32>(0.5)"));
        assert!(floor.contains("700.0 / key_d2"));
        assert!(floor.contains("let outer = cos(0.4);"));
        assert!(floor.contains("u.fire.w * 2.2 / (1.0 + fire_d2 * 0.6)"));
    }

    #[test]
    fn props_and_meta_match_the_js_registry() {
        let scene = RollingFire::new();
        assert_eq!(scene.meta().name, "Point10 Rolling Fire");
        assert_eq!(scene.meta().category, "3d-scenes");
        assert_eq!(scene.clear_alpha(), 0.0);
        assert_eq!(scene.role(), SceneRole::Foreground);
        assert!(scene.transparent_overlay());
        assert!(!scene.frozen());
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
    fn ball_of_fire_is_the_frozen_standalone_variant() {
        let scene = BallOfFire::new();
        assert_eq!(scene.name(), "Point10 Ball Of Fire");
        assert_eq!(scene.meta().name, "Point10 Ball Of Fire");
        assert_eq!(scene.meta().author, "Point10");
        assert_eq!(scene.meta().version, "1.0.0");
        assert_eq!(scene.meta().module_type, "2d");
        assert_eq!(scene.meta().category, "3d-scenes");
        assert_eq!(scene.meta().audio_features, &["bass", "mid", "treble"]);
        assert_eq!(scene.role(), SceneRole::Standalone);
        assert!(scene.frozen());
        assert!(scene.transparent_overlay());
        // Identical props (`Point10BallOfFire.js` uses the same class, so the
        // adapter resolves the same `ROLLING_FIRE_PROPS` schema).
        assert_eq!(scene.props(), RollingFire::new().props());
        assert_eq!(scene.inner().grid(), GRID);
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
    fn update_maps_the_audio_bands() {
        let mut scene = RollingFire::new();
        let bridge = loud_bridge();
        scene.update(1.0 / 60.0, &bridge);

        // bass = mean(first five bins) = 0.6, smoothed by `min(1, dt * 8)`.
        close(scene.bass(), 0.6 * (1.0 / 60.0 * 8.0), 1e-6);
        // hi-hats = mean(top 15%) = 0.6, smoothed by `dt * 10`.
        close(scene.hi, 0.6 * (1.0 / 60.0 * 10.0), 1e-6);
        // snares = bins 2%..13% = 0.6, smoothed by `dt * 12`.
        close(scene.snare, 0.6 * (1.0 / 60.0 * 12.0), 1e-6);
        // `uFireSize = 1 + hi * 2.5 + snare * 8`.
        close(scene.fire_size(), fire_size(scene.hi, scene.snare), 1e-6);
        // The flat spectrum steers at the band's middle, so the azimuth only
        // advances by `BALL_DIR_SPIN` (the centroid sway is ~0).
        close(scene.ball_direction(), BALL_DIR_SPIN * (1.0 / 60.0), 1e-4);
        // The first bass rise is a beat (0.08 past the 0.04 floor), so the
        // kick is applied to the ball this frame.
        assert_ne!(scene.ball(), Ball::new());
        // The camera yaw eases toward `bass * 0.15` at `dt * 0.15`.
        close(
            scene.camera_yaw(),
            scene.bass() * 0.15 * (1.0 / 60.0 * 0.15),
            1e-9,
        );
    }

    /// The simulation clock is the JS `_animate` accumulator: `1/120 * speed`
    /// steps from a frame delta capped at 1/30 s, at most eight per frame.
    #[test]
    fn the_simulation_clock_runs_the_js_fixed_step() {
        let mut scene = RollingFire::new();
        let silent = AudioBridge::new();
        scene.update(1.0 / 60.0, &silent);
        // 1/60 s * 1.2 = 0.02 s of sim time at 0.01 s per step: two steps.
        close(scene.simulation_time(), 0.02, 1e-6);
        close(scene.last_temporal.time, 0.02, 1e-6);
        // A one-second frame is capped to 1/30 s: four steps, 0.04 s.
        scene.update(1.0, &silent);
        close(scene.simulation_time(), 0.06, 1e-6);
        // A zero delta is the JS `this._dt || 1 / 60` fallback, so the sim
        // still steps (the JS never stalls on a paused clock).
        scene.update(0.0, &silent);
        close(scene.simulation_time(), 0.08, 1e-6);
    }

    #[test]
    fn update_rolls_the_ball_and_follows_it() {
        let mut scene = RollingFire::new();
        let bridge = loud_bridge();

        for _ in 0..120 {
            scene.update(1.0 / 60.0, &bridge);
        }
        assert!(scene.bass() > 0.0);
        assert!(scene.fire_size() > 1.0);
        assert_ne!(scene.ball(), Ball::new());
        assert!(scene.ball_direction() > 0.0);
        // The camera chases the ball, so it must have moved from its start.
        assert_ne!(scene.camera_position(), CAMERA_START);
        // The key light and grid origin track the ball (`:1457`, `:1322`).
        let sphere = scene.sphere_position();
        assert_eq!(scene.key_light_position()[1], sphere[1] + 20.0);
        assert_eq!(scene.grid_origin()[1], sphere[1] - 8.0);
    }

    /// R-34: with no audio the JS keeps animating (its analyser exists in
    /// silence, reporting zero bands). The port must advance the simulation
    /// clock, the ball/camera/hue/spin path and the fire hull, not return
    /// early.
    #[test]
    fn no_audio_idles_like_the_js_instead_of_freezing() {
        let mut scene = RollingFire::new();
        let silent = AudioBridge::new();
        assert!(!silent.is_available());

        for _ in 0..120 {
            scene.update(1.0 / 60.0, &silent);
        }
        // The sim clock advances (two 1/120 s steps per 1/60 s frame at 1.2x).
        assert!(scene.simulation_time() > 0.0);
        // The steer spins and the camera settles onto the follow offset even
        // in silence.
        assert!(scene.ball_direction() > 0.0);
        assert_ne!(scene.camera_position(), CAMERA_START);
        // It lerps toward the follow offset at `delta * 1.2` per frame, so 120
        // frames get most of the way there even without any music.
        assert!((scene.camera_position()[1] - (SPHERE_Y + CAMERA_START[1])).abs() < 0.3);
        // Zero bands mean zero acceleration, so the ball has not moved.
        assert_eq!(scene.ball(), Ball::new());
        // The smoothers decayed to zero and the emission scale is the idle
        // multiplier of 1.
        assert_eq!(scene.bass(), 0.0);
        assert_eq!(scene.fire_size(), 1.0);
        assert_eq!(scene.fire_hue(), 0.0);

        // Momentum from the loud run keeps moving under silence: run loud,
        // then one silent frame must still advance the ball's position and the
        // clock.
        let mut rolling = RollingFire::new();
        let bridge = loud_bridge();
        for _ in 0..120 {
            rolling.update(1.0 / 60.0, &bridge);
        }
        let ball = rolling.ball();
        let time = rolling.simulation_time();
        let spin = rolling.ball_direction();
        rolling.update(1.0 / 60.0, &silent);
        assert_ne!(rolling.ball(), ball, "the ball must keep coasting");
        assert!(rolling.simulation_time() > time);
        assert!(rolling.ball_direction() > spin);
    }

    #[test]
    fn ball_of_fire_freezes_the_ball_but_keeps_the_scene_alive() {
        let mut scene = BallOfFire::new();
        let bridge = loud_bridge();
        for _ in 0..120 {
            scene.update(1.0 / 60.0, &bridge);
        }
        // The ball never leaves the start position, whatever the audio does.
        assert_eq!(scene.inner().ball(), Ball::new());
        // But everything else reacts: the camera orbits, the fire pulses, the
        // hue can flip and the grid still emits around the frozen ball.
        assert!(scene.inner().bass() > 0.0);
        assert!(scene.inner().fire_size() > 1.0);
        assert_ne!(scene.inner().camera_position(), CAMERA_START);
        // The simulation clock advances with the scene (the GPU steps are
        // submitted by `render`, but the accumulator lives in `update`).
        assert!(scene.inner().simulation_time() > 0.0);
    }
}
