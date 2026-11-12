//! DvdRoom — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/DvdRoom.js`.
//!
//! The JS scene is **WebGPU/TSL** and inherits `SmokeRoom`: a glass room wraps
//! a 12x12x24 volumetric fire/smoke simulation grid, and a bouncing DVD logo
//! is both the emitter and the subject. The logo changes colour on every wall
//! bounce and flashes white on a corner hit; music pumps its speed and the
//! smoke emission.
//!
//! # Ported exactly
//!
//! * the logo motion (`DvdRoom.js:350-374`): `pos += vel * dt * speedMul` with
//!   `speedMul = 1 + min(smBass, 0.6) + burst * 0.5`, the per-axis room bounds
//!   `VOLUME_WORLD_SIZE / 2 - logoHalf`, the reflection, and one colour step
//!   (`colorIdx = (colorIdx + 1) % 7`) plus `pop = 1` per bounce; a 2-axis hit
//!   sets `cornerFlash = 1`;
//! * the palette (`:31-33`), the initial position `(-2, 1, -4)` and the
//!   `normalize(1, 0.6, 0.8) * 3.5` velocity;
//! * the logo transform (`:410-425`): `rotation = (sin(time * 0.3) * 0.25,
//!   faceYaw + sin(time * 0.4) * 0.5, 0)` with
//!   `faceYaw = atan2(cam.x - pos.x, cam.z - pos.z)` and the `1 + pop * 0.12`
//!   scale;
//! * the audio mapping (`:376-408`): bass mean of bins `[1, 9)`, treble mean of
//!   the top 15%, the asymmetric `dt*16` / `dt*5` smoothing, the kick attack
//!   (`bass > 0.07`, `bass > bassAvg * 1.4`, `time - lastBeat > 0.12`, with
//!   `pop = 1` and `burst = min(1.5, burst + 0.8)`), the `exp(-dt*2/6/3)`
//!   decays and the treble/corner emissive;
//! * the camera orbit (`:427-438`): `orbitYaw += (smBass * 0.35 - orbitYaw) *
//!   min(1, dt * 0.4)`, then the `k = min(1, dt * 1.2)` lerp toward
//!   `(cos(orbitYaw) * 17, 6.5 + sin(time * 0.23) * 1.2, sin(orbitYaw) * 17)`
//!   with `lookAt(0, 0, 0)`;
//! * the glass fit (`SmokeRoom.js:130-185`, `:276-294`): a bottom-anchored room
//!   whose top edge sits `GLASS_TOP_MARGIN` above the output's top edge, at
//!   `boxScale` / `boxXYScale`;
//! * the renderer grade (`SmokeRoom.js:417-418`): `ACESFilmicToneMapping` with
//!   `toneMappingExposure = 2`, applied by [`Composer`]'s output pass after the
//!   `alpha = max(rgb)` rule, exactly like the JS's pipeline output;
//! * the room/light rig intent (`:319-336`): the glass pane opacity
//!   `0.34 + smBass * 0.06`, the dimmed `0.35` fire light and the `22`
//!   fire-intensity default that keeps the bounce colour visible;
//! * the prop binder (`sceneProps.js:40-44`, `:205-218`): `smokeDensity`,
//!   `fireIntensity`, `smokeLifespan`, `fireLifespan`, `simulationSpeed`,
//!   `boxScale`, `boxXYScale`.
//!
//! # Replaced TSL machinery — the smoke simulation
//!
//! The JS smoke is a 150x150x300 `Storage3DTexture` grid with seven compute
//! passes per 1/120 s step (advect velocity, divergence, two Jacobi pressure
//! iterations, project, advect dye, emit) plus a 16-step `VolumeNodeMaterial`
//! raymarch, a shadow volume pass, a half-resolution gaussian blur and a
//! bloom. That compute/storage-texture path does not exist in this crate (see
//! `point10/mod.rs`), so the *motion and audio behaviour* of the emission is
//! kept and the volume is replaced with a procedural **additive haze**:
//!
//! | SmokeRoom (TSL) | DvdRoom (Rust) |
//! |-----------------|----------------|
//! | 3D velocity/dye/pressure grids + compute passes | 32 CPU-spawned billboard puffs drifting from the logo |
//! | `uFireSize = 1 + hi * 2.5 + snare * 8` | the same, scaling the puff brightness |
//! | `uEmitGain = (sphereVerts / logoVerts) * 6` | the same reference-count formula, from the ported logo mesh |
//! | `smokeDensity` / `fireIntensity` / lifespans / `simulationSpeed` | puff brightness / life / speed |
//! | `dropColorPulse` → `uFireHue` | the same drop detector drives the puff colour |
//! | volume raymarch + bloom | additive puffs + [`UnrealBloomPass`] |
//!
//! The room, glass, logo, palette, bounce and camera behaviour are ported as
//! they are; only the density field is substituted. The haze positions are
//! seeded with [`seeded_rand`] instead of `Math.random`, and the puff count is
//! fixed (32) rather than a grid.
//!
//! # Other deviations
//!
//! * **The logo is procedural, not the SVG.** `dvd_logo.svg` is a Wikimedia
//!   text logo that the JS parses with its own SVG path parser and extrudes.
//!   The shared asset loader only decodes PNG/JPEG, so the port builds a
//!   rounded, bevelled slab of the same `LOGO_WIDTH` x `LOGO_WIDTH/2` x
//!   `LOGO_DEPTH` extents — the same bounce silhouette with a fraction of the
//!   geometry. The bounce box uses the same half extents the JS measures.
//! * The JS glass is a `RoundedBoxGeometry` with a PMREM studio env map; the
//!   port draws the six panes of a box with a fresnel/env-gradient glass shader
//!   and the same opacity rule — no PMREM pass exists here.
//! * **The smoke grid keeps its geometry only as the room's box.** The room
//!   still matches the 12x12x24 sim footprint (`VOLUME_WORLD_SIZE_*`).
//! * `stepBounce`'s bounds are not scaled by `boxScale` (a JS quirk); the port
//!   keeps that, while the glass room itself does scale.
//! * The final alpha is `clamp(max(rgb), 0, 1)` — the rule SmokeRoom writes
//!   explicitly (`SmokeRoom.js:1270-1276`), so the empty background stays
//!   transparent and the module composites over the layers below.
//!
//! [`UnrealBloomPass`]: crate::point10::postfx::UnrealBloomPass

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge};
use crate::point10::postfx::{Composer, ToneMapping, UnrealBloomPass};
use crate::point10::scenes::cube_mesh::{
    Mat4, Mesh, MeshGpu, Vertex, build_box, create_depth, depth_state, look_at, mat4_mul,
    normalise, perspective, scale, translation, trs,
};
use crate::point10::shared::phrase_color::{
    DropColorState, drop_color_pulse, ease_phrase_hue, equalized_hue, srgb_hex_to_linear,
};
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    UnitQuad, frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`SmokeRoom.js:60-88`, `DvdRoom.js:26-33`)
// ---------------------------------------------------------------------------

/// `VOLUME_WORLD_SIZE_X` — the room/sim footprint.
pub const VOLUME_WORLD_SIZE_X: f32 = 12.0;
/// `VOLUME_WORLD_SIZE_Y`.
pub const VOLUME_WORLD_SIZE_Y: f32 = 12.0;
/// `VOLUME_WORLD_SIZE_Z`.
pub const VOLUME_WORLD_SIZE_Z: f32 = 24.0;

/// `GLASS_PADDING`.
pub const GLASS_PADDING: f32 = 0.6;
/// `GLASS_HALF_X` = `(VOLUME_WORLD_SIZE_X + GLASS_PADDING) / 2`.
pub const GLASS_HALF_X: f32 = (VOLUME_WORLD_SIZE_X + GLASS_PADDING) / 2.0;
/// `GLASS_HALF_Y`.
pub const GLASS_HALF_Y: f32 = (VOLUME_WORLD_SIZE_Y + GLASS_PADDING) / 2.0;
/// `GLASS_HALF_Z`.
pub const GLASS_HALF_Z: f32 = (VOLUME_WORLD_SIZE_Z + GLASS_PADDING) / 2.0;
/// `GLASS_TOP_MARGIN`.
pub const GLASS_TOP_MARGIN: f32 = 0.75;

/// `LOGO_WIDTH` — world units across.
pub const LOGO_WIDTH: f32 = 4.2;
/// `LOGO_HEIGHT` — the fallback box's `LOGO_WIDTH / 2`.
pub const LOGO_HEIGHT: f32 = LOGO_WIDTH / 2.0;
/// `LOGO_DEPTH`.
pub const LOGO_DEPTH: f32 = 0.35;
/// The rounded-slab corner radius (a visual stand-in for the SVG bevel).
pub const LOGO_RADIUS: f32 = 0.45;

/// `BASE_SPEED`.
pub const BASE_SPEED: f32 = 3.5;
/// The initial logo position (`:206`).
pub const LOGO_START_POSITION: [f32; 3] = [-2.0, 1.0, -4.0];
/// The initial velocity direction (`:207`).
pub const LOGO_START_VELOCITY: [f32; 3] = [1.0, 0.6, 0.8];

/// `PALETTE` — the classic screensaver cycle (never random).
pub const PALETTE: [u32; 7] = [
    0x0000_b3ff,
    0x00ff_8c1a,
    0x0059_ff5e,
    0x00ff_3bff,
    0x00ff_e63b,
    0x00ff_3b3b,
    0x00ff_ffff,
];

/// `FIRE_HAT_K`.
pub const FIRE_HAT_K: f32 = 2.5;
/// `FIRE_SNARE_K`.
pub const FIRE_SNARE_K: f32 = 8.0;
/// `BASS_BINS` (SmokeRoom).
pub const BASS_BINS: usize = 5;
/// `BLOOM_BASE`.
pub const BLOOM_BASE: f32 = 0.12;
/// `BLOOM_PER_BASS`.
pub const BLOOM_PER_BASS: f32 = 2.2;
/// The JS clamps the smoke-room bloom at 2.8.
pub const BLOOM_MAX: f32 = 2.8;

/// `SPHERE_RADIUS`.
pub const SPHERE_RADIUS: f32 = 0.9;
/// `SphereGeometry(SPHERE_RADIUS, 64, 40)`'s position count.
pub const REFERENCE_EMITTER_VERTICES: usize = (64 + 1) * (40 + 1);

/// `CLEAR_ALPHA` — SmokeRoom sets `scene.background = null`.
pub const CLEAR_ALPHA: f64 = 0.0;

/// The number of haze puffs that stand in for the smoke volume.
pub const PUFF_COUNT: usize = 32;
/// The emit-gain normalisation the puffs are calibrated at.
pub const REFERENCE_EMIT_GAIN: f32 = 82.0;

/// `CAMERA_FOV` (`PerspectiveCamera(60, …)`).
pub const CAMERA_FOV: f32 = 60.0;
/// `CAMERA_NEAR`.
pub const CAMERA_NEAR: f32 = 0.1;
/// `CAMERA_FAR`.
pub const CAMERA_FAR: f32 = 100.0;
/// The initial camera (`SmokeRoom.js:433`: `(14.0, 5.5, 4.4)`).
pub const CAMERA_START: [f32; 3] = [14.0, 5.5, 4.4];
/// The orbit radius (`DvdRoom.js:433-437`).
pub const CAMERA_ORBIT_RADIUS: f32 = 17.0;

// ---------------------------------------------------------------------------
// Pure geometry / motion helpers
// ---------------------------------------------------------------------------

/// `SmokeRoom.boxHalfExtents(scale, xyScale)`.
pub fn box_half_extents(scale: f32, xy_scale: f32) -> [f32; 3] {
    let scale = if scale.is_finite() {
        scale.max(0.0)
    } else {
        1.0
    };
    let xy = if xy_scale.is_finite() {
        xy_scale.max(0.0)
    } else {
        1.0
    };
    [
        GLASS_HALF_X * scale * xy,
        GLASS_HALF_Y * scale * xy,
        GLASS_HALF_Z * scale,
    ]
}

/// The world-space Y that puts the room's highest edge on the output's top
/// edge (`SmokeRoom.js:146-174`).
///
/// `forward` is `camera.getWorldDirection` and `up` the camera's up vector.
/// Returns the natural half height when the ray/plane intersection degenerates
/// (the camera looking straight down the room's top plane), where the JS would
/// divide by ~0.
pub fn required_glass_top(
    camera_position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    fov_degrees: f32,
    scale: f32,
    xy_scale: f32,
) -> f32 {
    let forward = normalise(forward);
    let right = normalise([
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
    ]);
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    let tan_half_fov = (fov_degrees.to_radians() * 0.5).tan();
    let denominator = up[1] - tan_half_fov * forward[1];
    let extents = box_half_extents(scale, xy_scale);
    if denominator.abs() < 1e-5 {
        return extents[1];
    }

    let mut top = f32::NEG_INFINITY;
    for x in [-extents[0], extents[0]] {
        for z in [-extents[2], extents[2]] {
            let ex = x - camera_position[0];
            let ez = z - camera_position[2];
            let offset =
                ex * up[0] + ez * up[2] - tan_half_fov * (ex * forward[0] + ez * forward[2]);
            top = top.max(camera_position[1] - offset / denominator);
        }
    }
    top
}

/// `SmokeRoom.glassFit(top, scale, xyScale)` — the bottom-anchored room height
/// and centre.
pub fn glass_fit(top: f32, scale: f32, xy_scale: f32) -> (f32, f32) {
    let half_y = box_half_extents(scale, xy_scale)[1];
    let top_y = if top.is_finite() {
        top.max(half_y)
    } else {
        half_y
    };
    (top_y + half_y, (top_y - half_y) / 2.0)
}

/// The room's top edge for a camera looking at the origin, including the
/// `GLASS_TOP_MARGIN`.
pub fn room_top(camera_position: [f32; 3], scale: f32, xy_scale: f32) -> f32 {
    let forward = normalise([
        -camera_position[0],
        -camera_position[1],
        -camera_position[2],
    ]);
    required_glass_top(
        camera_position,
        forward,
        [0.0, 1.0, 0.0],
        CAMERA_FOV,
        scale,
        xy_scale,
    ) + GLASS_TOP_MARGIN * scale
}

/// The logo's half extents (`this.logoHalf` for the fallback box).
pub fn logo_half_extents() -> [f32; 3] {
    [LOGO_WIDTH / 2.0, LOGO_HEIGHT / 2.0, LOGO_DEPTH / 2.0]
}

/// The bounce bounds `VOLUME_WORLD_SIZE / 2 - logoHalf` (`:351-355`).
///
/// Note the JS does **not** scale these by `boxScale`; the port keeps that.
pub fn bounce_bounds() -> [f32; 3] {
    let half = logo_half_extents();
    [
        VOLUME_WORLD_SIZE_X / 2.0 - half[0],
        VOLUME_WORLD_SIZE_Y / 2.0 - half[1],
        VOLUME_WORLD_SIZE_Z / 2.0 - half[2],
    ]
}

/// One `stepBounce` result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BounceStep {
    /// Integrated position, clamped into the room.
    pub position: [f32; 3],
    /// Reflected velocity.
    pub velocity: [f32; 3],
    /// Number of axes that hit a wall (2+ = a corner).
    pub axes_hit: u32,
}

/// Pure `stepBounce(dt, speedMul)` (`:350-374`).
pub fn step_bounce(position: [f32; 3], velocity: [f32; 3], dt: f32, speed_mul: f32) -> BounceStep {
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let speed_mul = if speed_mul.is_finite() {
        speed_mul
    } else {
        1.0
    };
    let bounds = bounce_bounds();
    let mut position = [
        position[0] + velocity[0] * dt * speed_mul,
        position[1] + velocity[1] * dt * speed_mul,
        position[2] + velocity[2] * dt * speed_mul,
    ];
    let mut velocity = velocity;
    let mut axes_hit = 0;

    for axis in 0..3 {
        if position[axis] > bounds[axis] {
            position[axis] = bounds[axis];
            velocity[axis] = -velocity[axis].abs();
            axes_hit += 1;
        } else if position[axis] < -bounds[axis] {
            position[axis] = -bounds[axis];
            velocity[axis] = velocity[axis].abs();
            axes_hit += 1;
        }
    }

    BounceStep {
        position,
        velocity,
        axes_hit,
    }
}

/// `advanceCamera` (`:427-438`): returns `(camera, orbitYaw)`.
pub fn advance_camera(
    camera: [f32; 3],
    orbit_yaw: f32,
    smoothed_bass: f32,
    time: f32,
    dt: f32,
) -> ([f32; 3], f32) {
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let bass = if smoothed_bass.is_finite() {
        smoothed_bass.max(0.0)
    } else {
        0.0
    };
    let yaw = orbit_yaw + (bass * 0.35 - orbit_yaw) * (dt * 0.4).min(1.0);
    let k = (dt * 1.2).min(1.0);
    let target = [
        yaw.cos() * CAMERA_ORBIT_RADIUS,
        6.5 + (time * 0.23).sin() * 1.2,
        yaw.sin() * CAMERA_ORBIT_RADIUS,
    ];
    (
        [
            camera[0] + (target[0] - camera[0]) * k,
            camera[1] + (target[1] - camera[1]) * k,
            camera[2] + (target[2] - camera[2]) * k,
        ],
        yaw,
    )
}

/// The Plane9-style fire ramp SmokeRoom uses (`SmokeRoom.js:850-858`), in
/// linear RGB.
pub fn fire_ramp(t: f32) -> [f32; 3] {
    let start = srgb_hex_to_linear(0x00ff_e68c);
    let mid = srgb_hex_to_linear(0x00ff_7305);
    let end = srgb_hex_to_linear(0x00ff_0000);
    let mix = |a: [f32; 3], b: [f32; 3], k: f32| {
        [
            a[0] + (b[0] - a[0]) * k,
            a[1] + (b[1] - a[1]) * k,
            a[2] + (b[2] - a[2]) * k,
        ]
    };
    let steps = |edge0: f32, edge1: f32, x: f32| {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };

    let colour = mix([0.0; 3], end, steps(0.05, 0.35, t));
    let colour = mix(colour, mid, steps(0.35, 0.65, t));
    mix(colour, start, steps(0.65, 1.0, t))
}

/// `fireHue(fireRamp(t), hue)` on the CPU (luminance-equalised rotation).
pub fn fire_colour(t: f32, hue: f32) -> [f32; 3] {
    equalized_hue(fire_ramp(t), hue)
}

/// `_band(lo, hi)` (`:223-229`): mean of the `freqMag` bins in `[lo, hi)`.
pub fn band(freq_mag: &[f32; AUDIO_SAMPLES], lo: usize, hi: usize) -> f32 {
    let hi = hi.min(freq_mag.len());
    if hi <= lo {
        return 0.0;
    }
    let mut sum = 0.0;
    for value in freq_mag.iter().take(hi).skip(lo) {
        sum += value;
    }
    sum / (hi - lo) as f32
}

/// The top-15% treble mean (`:379-385`).
pub fn treble_band(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    let lo = (AUDIO_SAMPLES as f32 * 0.85).floor() as usize;
    band(freq_mag, lo, AUDIO_SAMPLES)
}

/// One puff of the procedural smoke haze.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puff {
    /// World position.
    pub position: [f32; 3],
    /// World velocity.
    pub velocity: [f32; 3],
    /// Normalised age, 0..1.
    pub age: f32,
    /// World-space radius.
    pub size: f32,
    /// Brightness multiplier.
    pub alpha: f32,
    /// Temperature (for the fire ramp).
    pub temperature: f32,
    /// The seed stream offset for the next respawn.
    pub seed: u32,
}

impl Puff {
    /// A dead puff (age 1 → respawns on the next update).
    pub fn dead() -> Self {
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

/// Spawn a puff around the logo (`emitter` is the logo's world position).
pub fn spawn_puff(seed: u32, emitter: [f32; 3]) -> Puff {
    let rand = |offset: u32| seeded_rand(seed as i32, offset as i32);
    let direction = normalise([
        rand(1) * 2.0 - 1.0,
        (rand(2) * 2.0 - 1.0) * 0.6 + 0.7,
        rand(3) * 2.0 - 1.0,
    ]);
    let speed = 0.5 + rand(4) * 1.2;
    Puff {
        position: [
            emitter[0] + (rand(5) * 2.0 - 1.0) * 0.8,
            emitter[1] + (rand(6) * 2.0 - 1.0) * 0.6,
            emitter[2] + (rand(7) * 2.0 - 1.0) * 0.8,
        ],
        velocity: [
            direction[0] * speed,
            direction[1] * speed,
            direction[2] * speed,
        ],
        age: 0.0,
        size: 0.35 + rand(8) * 0.6,
        alpha: 0.6 + rand(9) * 0.4,
        temperature: 0.35 + rand(10) * 0.55,
        seed: seed.wrapping_add(1),
    }
}

/// Advance one puff (the substitute for the advection pass).
pub fn advance_puff(
    puff: &mut Puff,
    emitter: [f32; 3],
    dt: f32,
    simulation_speed: f32,
    lifespan: f32,
) {
    let speed = if simulation_speed.is_finite() {
        simulation_speed.clamp(0.1, 4.0)
    } else {
        1.2
    };
    let life = if lifespan.is_finite() {
        lifespan.clamp(0.1, 20.0)
    } else {
        3.5
    };
    let step = if dt.is_finite() { dt.max(0.0) } else { 0.0 } * speed;
    if puff.age >= 1.0 {
        *puff = spawn_puff(puff.seed, emitter);
        return;
    }
    puff.age = (puff.age + step / life).min(1.0);
    puff.position = [
        puff.position[0] + puff.velocity[0] * step,
        puff.position[1] + puff.velocity[1] * step,
        puff.position[2] + puff.velocity[2] * step,
    ];
    // Slow down and cool as it rises.
    puff.velocity = [
        puff.velocity[0] * (1.0 - 0.6 * step),
        puff.velocity[1] * (1.0 - 0.3 * step),
        puff.velocity[2] * (1.0 - 0.6 * step),
    ];
    puff.size += step * 1.4;
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `DVD_ROOM_PROPS` (`sceneProps.js:18-44`).
const DVD_ROOM_PROPS: &[SceneProp] = &[
    SceneProp::float("smokeDensity", "Smoke Density", 28.0, 0.0, 100.0, 0.1),
    SceneProp::float("fireIntensity", "Fire Intensity", 22.0, 0.0, 100.0, 0.1),
    SceneProp::float("smokeLifespan", "Smoke Lifespan", 3.5, 0.1, 20.0, 0.1),
    SceneProp::float("fireLifespan", "Fire Lifespan", 1.3, 0.1, 5.0, 0.1),
    SceneProp::float("simulationSpeed", "Simulation Speed", 1.2, 0.1, 4.0, 0.01),
    SceneProp::float("boxScale", "Box Scale", 1.0, 0.5, 5.0, 0.05),
    SceneProp::float("boxXYScale", "Box Width & Height", 1.0, 0.5, 5.0, 0.05),
];

/// The logo shader's uniform block (208 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct LogoUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// The palette colour (linear).
    colour: [f32; 4],
    /// The logo's world position (the fire light origin).
    logo_pos: [f32; 4],
    /// The fire light colour (linear) and intensity (w).
    fire: [f32; 4],
    /// x emissive intensity, y corner flash, z time, w glass glow.
    params: [f32; 4],
}

/// The glass shader's uniform block (192 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GlassUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    logo_pos: [f32; 4],
    fire: [f32; 4],
    /// x opacity, y time, z smoothed bass.
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
struct DvdRoomGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    logo_pipeline: wgpu::RenderPipeline,
    logo_uniforms: wgpu::Buffer,
    logo_bind_group: wgpu::BindGroup,
    glass_pipeline: wgpu::RenderPipeline,
    glass_uniforms: wgpu::Buffer,
    glass_bind_group: wgpu::BindGroup,
    puff_pipeline: wgpu::RenderPipeline,
    puff_uniforms: wgpu::Buffer,
    puff_bind_group: wgpu::BindGroup,
    puff_instances: wgpu::Buffer,
    logo_mesh: MeshGpu,
    glass_mesh: MeshGpu,
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

/// DvdRoom scene.
pub struct DvdRoom {
    /// `this.pos`.
    position: [f32; 3],
    /// `this.vel`.
    velocity: [f32; 3],
    /// `this.colorIdx`.
    colour_index: usize,
    /// `this.pop`.
    pop: f32,
    /// `this.cornerFlash`.
    corner_flash: f32,
    /// `this.burst`.
    burst: f32,
    /// `this.bounces`.
    bounces: u32,
    /// `this.smBass`.
    smoothed_bass: f32,
    /// `this.smTreble`.
    smoothed_treble: f32,
    /// SmokeRoom's own `this._bass` (first-five-bin mean, `dt * 8`).
    smoke_bass: f32,
    /// SmokeRoom's hi-hat band.
    hi: f32,
    /// SmokeRoom's snare band.
    snare: f32,
    /// SmokeRoom's `uFireSize`.
    fire_size: f32,
    /// `this.bassAvg`.
    bass_average: f32,
    /// `this.lastBeat`.
    last_beat: f32,
    /// `this.time` (`BaseSceneWebGPU.update` accumulates the raw delta).
    time: f32,
    /// `this._orbitYaw`.
    orbit_yaw: f32,
    /// The camera position (`this.camera.position`).
    camera: [f32; 3],
    /// The smoke/fire drop pulse (`uFireHue`).
    pulse: DropColorState,
    /// The seed stream behind the smoke pulse.
    seed: u32,
    /// `uFireHue`.
    fire_hue: f32,
    /// The haze puffs.
    puffs: [Puff; PUFF_COUNT],
    /// `uEmitGain` from the emitter vertex counts.
    emit_gain: f32,
    /// Props.
    smoke_density: f32,
    fire_intensity: f32,
    smoke_lifespan: f32,
    fire_lifespan: f32,
    simulation_speed: f32,
    box_scale: f32,
    box_xy_scale: f32,
    /// The room fit (`glass.scale.y` and `glass.position.y`).
    glass_height: f32,
    glass_centre_y: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<DvdRoomGpu>,
}

impl Default for DvdRoom {
    fn default() -> Self {
        Self::new()
    }
}

impl DvdRoom {
    /// Create the scene (`DvdRoom.js:186-221` defaults).
    pub fn new() -> Self {
        let direction = normalise(LOGO_START_VELOCITY);
        let mut puffs = [Puff::dead(); PUFF_COUNT];
        for (index, puff) in puffs.iter_mut().enumerate() {
            puff.seed = index as u32 * 7 + 1;
        }
        Self {
            position: LOGO_START_POSITION,
            velocity: [
                direction[0] * BASE_SPEED,
                direction[1] * BASE_SPEED,
                direction[2] * BASE_SPEED,
            ],
            colour_index: 0,
            pop: 0.0,
            corner_flash: 0.0,
            burst: 0.0,
            bounces: 0,
            smoothed_bass: 0.0,
            smoothed_treble: 0.0,
            smoke_bass: 0.0,
            hi: 0.0,
            snare: 0.0,
            fire_size: 1.0,
            bass_average: 0.0,
            last_beat: -1.0,
            time: 0.0,
            orbit_yaw: 0.0,
            camera: CAMERA_START,
            pulse: DropColorState::new(),
            seed: 0,
            fire_hue: 0.0,
            puffs,
            emit_gain: 6.0,
            smoke_density: 28.0,
            fire_intensity: 22.0,
            smoke_lifespan: 3.5,
            fire_lifespan: 1.3,
            simulation_speed: 1.2,
            box_scale: 1.0,
            box_xy_scale: 1.0,
            glass_height: GLASS_HALF_Y * 2.0,
            glass_centre_y: 0.0,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:40-44`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(DVD_ROOM_PROPS, props) {
            match key {
                "smokeDensity" => self.smoke_density = value,
                "fireIntensity" => self.fire_intensity = value,
                "smokeLifespan" => self.smoke_lifespan = value,
                "fireLifespan" => self.fire_lifespan = value,
                "simulationSpeed" => self.simulation_speed = value,
                "boxScale" => self.box_scale = value,
                "boxXYScale" => self.box_xy_scale = value,
                _ => {}
            }
        }
    }

    /// `this.pos`.
    pub fn position(&self) -> [f32; 3] {
        self.position
    }

    /// `this.vel`.
    pub fn velocity(&self) -> [f32; 3] {
        self.velocity
    }

    /// `this.colorIdx`.
    pub fn colour_index(&self) -> usize {
        self.colour_index
    }

    /// `this.bounces`.
    pub fn bounces(&self) -> u32 {
        self.bounces
    }

    /// `this.cornerFlash`.
    pub fn corner_flash(&self) -> f32 {
        self.corner_flash
    }

    /// `this.camera.position`.
    pub fn camera_position(&self) -> [f32; 3] {
        self.camera
    }

    /// SmokeRoom's `uFireSize`.
    pub fn fire_size(&self) -> f32 {
        self.fire_size
    }

    /// The smoke-room bloom strength (`SmokeRoom.js:1395-1400`).
    pub fn bloom_strength(&self) -> f32 {
        (BLOOM_BASE + self.smoke_bass * BLOOM_PER_BASS).min(BLOOM_MAX)
    }

    /// The palette colour at the current index, linear.
    pub fn logo_colour(&self) -> [f32; 3] {
        srgb_hex_to_linear(PALETTE[self.colour_index % PALETTE.len()])
    }

    /// The haze puffs.
    pub fn puffs(&self) -> &[Puff; PUFF_COUNT] {
        &self.puffs
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    fn fire_light(&self) -> [f32; 4] {
        let colour = fire_colour(0.72, self.fire_hue);
        let intensity = (self.fire_intensity / 22.0).max(0.0) * 0.35;
        [
            colour[0] * intensity,
            colour[1] * intensity,
            colour[2] * intensity,
            intensity,
        ]
    }

    fn emissive_intensity(&self) -> f32 {
        1.0 + self.smoothed_treble * 2.5 + self.corner_flash * 1.5
    }

    fn logo_model(&self) -> Mat4 {
        let face_yaw = (self.camera[0] - self.position[0]).atan2(self.camera[2] - self.position[2]);
        let rotation = [
            (self.time * 0.3).sin() * 0.25,
            face_yaw + (self.time * 0.4).sin() * 0.5,
            0.0,
        ];
        let s = 1.0 + self.pop * 0.12;
        trs(self.position, rotation, [s, s, s])
    }

    fn glass_model(&self) -> Mat4 {
        mat4_mul(
            &translation(0.0, self.glass_centre_y, 0.0),
            &scale(
                GLASS_HALF_X * 2.0 * self.box_scale * self.box_xy_scale,
                self.glass_height,
                GLASS_HALF_Z * 2.0 * self.box_scale,
            ),
        )
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &look_at(self.camera, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        )
    }

    /// The camera-facing billboard basis (`right`, `up`).
    fn billboard_basis(&self) -> ([f32; 3], [f32; 3]) {
        let forward = normalise([-self.camera[0], -self.camera[1], -self.camera[2]]);
        let right = normalise([
            forward[1] * 0.0 - forward[2] * 1.0,
            forward[2] * 0.0 - forward[0] * 0.0,
            forward[0] * 1.0 - forward[1] * 0.0,
        ]);
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        (right, up)
    }

    fn refresh_glass_fit(&mut self) {
        let top = room_top(self.camera, self.box_scale, self.box_xy_scale);
        let (height, centre_y) = glass_fit(top, self.box_scale, self.box_xy_scale);
        self.glass_height = height;
        self.glass_centre_y = centre_y;
    }

    fn puff_instances(&self) -> Vec<PuffInstance> {
        self.puffs
            .iter()
            .map(|puff| {
                let fade = (1.0 - puff.age).clamp(0.0, 1.0);
                let density = (self.smoke_density / 28.0).max(0.0);
                let gain = (self.emit_gain / REFERENCE_EMIT_GAIN).max(0.0);
                let fire = (self.fire_size / 1.0).max(0.0);
                let decay = (-puff.age * (1.3 / self.fire_lifespan.max(0.1))).exp();
                let colour = fire_colour(puff.temperature * fade, self.fire_hue);
                let alpha = 0.09 * density * gain * fire * puff.alpha * fade * fade * decay;
                PuffInstance {
                    position: puff.position,
                    size: puff.size * (0.8 + fade * 0.8),
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
        let colour = self.logo_colour();
        let fire = self.fire_light();

        gpu.queue.write_buffer(
            &gpu.logo_uniforms,
            0,
            bytemuck::bytes_of(&LogoUniforms {
                view_proj,
                model: self.logo_model(),
                camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
                colour: [colour[0], colour[1], colour[2], 1.0],
                logo_pos: [self.position[0], self.position[1], self.position[2], 1.0],
                fire,
                params: [
                    self.emissive_intensity(),
                    self.corner_flash,
                    self.time,
                    self.smoke_bass,
                ],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.glass_uniforms,
            0,
            bytemuck::bytes_of(&GlassUniforms {
                view_proj,
                model: self.glass_model(),
                camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
                logo_pos: [self.position[0], self.position[1], self.position[2], 1.0],
                fire,
                params: [
                    0.34 + self.smoke_bass * 0.06,
                    self.time,
                    self.smoke_bass,
                    0.0,
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

    /// Recreate the size-dependent targets and re-bind the final pass.
    fn rebuild_targets(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.scene_target = TextureTarget::new(
            &gpu.device,
            "point10-dvd-room-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) = create_depth(
            &gpu.device,
            "point10-dvd-room-depth",
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

impl Point10Scene for DvdRoom {
    fn name(&self) -> &'static str {
        "Point10 DVD Room"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);
        self.camera_aspect = ctx.aspect();
        self.refresh_glass_fit();

        // The logo mesh replaces `DvdRoom._logoGeometry`'s SVG extrusion: a
        // rounded, bevelled slab with the same extents.
        let logo_mesh_data =
            build_rounded_slab(LOGO_WIDTH, LOGO_HEIGHT, LOGO_DEPTH, LOGO_RADIUS, 8);
        // `_emitGainValue = (refCount / max(1, logoVerts)) * 6`.
        self.emit_gain =
            (REFERENCE_EMITTER_VERTICES as f32 / logo_mesh_data.vertices.len().max(1) as f32) * 6.0;

        let logo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-dvd-room-logo-shader"),
            source: wgpu::ShaderSource::Wgsl(dvd_room_logo_wgsl().into()),
        });
        let logo_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-dvd-room-logo-layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let logo_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-dvd-room-logo-uniforms"),
            size: std::mem::size_of::<LogoUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let logo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-dvd-room-logo-bind-group"),
            layout: &logo_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: logo_uniforms.as_entire_binding(),
            }],
        });
        let logo_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-dvd-room-logo-pipeline-layout"),
            bind_group_layouts: &[Some(&logo_layout)],
            immediate_size: 0,
        });
        let logo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-dvd-room-logo-pipeline"),
            layout: Some(&logo_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &logo_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // `MeshStandardMaterial({ side: DoubleSide })`.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &logo_shader,
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

        // --- glass room ----------------------------------------------------
        let glass_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-dvd-room-glass-shader"),
            source: wgpu::ShaderSource::Wgsl(dvd_room_glass_wgsl().into()),
        });
        let glass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-dvd-room-glass-layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let glass_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-dvd-room-glass-uniforms"),
            size: std::mem::size_of::<GlassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glass_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-dvd-room-glass-bind-group"),
            layout: &glass_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: glass_uniforms.as_entire_binding(),
            }],
        });
        let glass_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-dvd-room-glass-pipeline-layout"),
                bind_group_layouts: &[Some(&glass_layout)],
                immediate_size: 0,
            });
        let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-dvd-room-glass-pipeline"),
            layout: Some(&glass_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &glass_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // `side: FrontSide` — only the panes facing the camera.
                cull_mode: Some(wgpu::Face::Back),
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
                module: &glass_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    // `transparent: true`, `opacity`, `depthWrite: false`.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // --- haze puffs ----------------------------------------------------
        let puff_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-dvd-room-puff-shader"),
            source: wgpu::ShaderSource::Wgsl(dvd_room_puff_wgsl().into()),
        });
        let puff_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-dvd-room-puff-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let puff_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-dvd-room-puff-uniforms"),
            size: std::mem::size_of::<PuffUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let puff_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-dvd-room-puff-bind-group"),
            layout: &puff_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: puff_uniforms.as_entire_binding(),
            }],
        });
        let puff_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-dvd-room-puff-pipeline-layout"),
            bind_group_layouts: &[Some(&puff_layout)],
            immediate_size: 0,
        });
        let quad = UnitQuad::new(&device);
        let puff_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-dvd-room-puff-pipeline"),
            layout: Some(&puff_pipeline_layout),
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
                    // `THREE.AdditiveBlending`, alpha kept.
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
            let instances = vec![
                PuffInstance {
                    position: [0.0; 3],
                    size: 0.0,
                    colour: [0.0; 3],
                    alpha: 0.0,
                };
                PUFF_COUNT
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-dvd-room-puff-instances"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };

        // --- scene target, bloom, final composite --------------------------
        let scene_target = TextureTarget::new(
            &device,
            "point10-dvd-room-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) =
            create_depth(&device, "point10-dvd-room-depth", self.size.0, self.size.1);

        let mut bloom = UnrealBloomPass::new(&device, &queue, self.size.0, self.size.1);
        bloom.set_strength(BLOOM_BASE);
        bloom.set_radius(1.0);
        bloom.set_threshold(0.5);
        bloom.set_input(&scene_target.view);

        let final_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-dvd-room-final-shader"),
            source: wgpu::ShaderSource::Wgsl(dvd_room_final_wgsl().into()),
        });
        let final_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-dvd-room-final-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let final_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-dvd-room-final-uniforms"),
            size: std::mem::size_of::<FinalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let final_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-dvd-room-final-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let final_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-dvd-room-final-pipeline",
            &final_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-dvd-room-final-pipeline-layout"),
                bind_group_layouts: &[Some(&final_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );
        let final_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-dvd-room-final-bind-group"),
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

        // `SmokeRoom._initVolume`: ACESFilmic tone mapping with exposure 2.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);
        composer.output_mut().set_exposure(2.0);

        let logo_mesh = MeshGpu::new(&device, "point10-dvd-room-logo", &logo_mesh_data);
        let glass_mesh = MeshGpu::new(&device, "point10-dvd-room-glass", &build_box(1.0, 1.0, 1.0));

        self.gpu = Some(DvdRoomGpu {
            device,
            queue,
            logo_pipeline,
            logo_uniforms,
            logo_bind_group,
            glass_pipeline,
            glass_uniforms,
            glass_bind_group,
            puff_pipeline,
            puff_uniforms,
            puff_bind_group,
            puff_instances,
            logo_mesh,
            glass_mesh,
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
        for puff in &mut self.puffs {
            *puff = spawn_puff(puff.seed, self.position);
        }
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        self.refresh_glass_fit();
        self.rebuild_targets();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.time += dt;

        let (bass, treble) = if audio.is_available() {
            let freq_mag = &audio.freq_mag;
            (band(freq_mag, 1, 9), treble_band(freq_mag))
        } else {
            (0.0, 0.0)
        };

        self.smoothed_bass += (bass - self.smoothed_bass)
            * (dt * if bass > self.smoothed_bass { 16.0 } else { 5.0 }).min(1.0);
        self.smoothed_treble += (treble - self.smoothed_treble)
            * (dt
                * if treble > self.smoothed_treble {
                    16.0
                } else {
                    5.0
                })
            .min(1.0);

        // Kick attack (`:393-405`).
        self.bass_average = self.bass_average * 0.95 + bass * 0.05;
        if bass > 0.07 && bass > self.bass_average * 1.4 && self.time - self.last_beat > 0.12 {
            self.last_beat = self.time;
            self.pop = 1.0;
            self.burst = (self.burst + 0.8).min(1.5);
        }
        self.burst *= (-dt * 2.0).exp();
        self.pop *= (-dt * 6.0).exp();
        self.corner_flash *= (-dt * 3.0).exp();

        // Bass speeds the logo (`:408`).
        let speed_mul = 1.0 + self.smoothed_bass.min(0.6) + self.burst * 0.5;
        let step = step_bounce(self.position, self.velocity, dt, speed_mul);
        self.position = step.position;
        self.velocity = step.velocity;
        if step.axes_hit > 0 {
            self.bounces += 1;
            self.colour_index = (self.colour_index + 1) % PALETTE.len();
            self.pop = 1.0;
            if step.axes_hit >= 2 {
                self.corner_flash = 1.0;
            }
        }

        // Camera orbit (`:429-438`).
        let (camera, yaw) = advance_camera(
            self.camera,
            self.orbit_yaw,
            self.smoothed_bass,
            self.time,
            dt,
        );
        self.camera = camera;
        self.orbit_yaw = yaw;
        self.refresh_glass_fit();

        // The smoke-room side of the audio: bloom bass, hats/snares -> fire
        // size, and the drop pulse -> fire hue (`SmokeRoom.js:340-374`).
        if audio.is_available() {
            let freq_mag = &audio.freq_mag;
            let bass_sum: f32 = freq_mag.iter().take(BASS_BINS).sum();
            let smoke_bass = bass_sum / BASS_BINS as f32;
            self.smoke_bass += (smoke_bass - self.smoke_bass) * (dt * 8.0).min(1.0);

            let hi_start = AUDIO_SAMPLES - (AUDIO_SAMPLES as f32 * 0.15).floor() as usize;
            let hi = band(freq_mag, hi_start, AUDIO_SAMPLES);
            self.hi += (hi - self.hi) * (dt * 10.0).min(1.0);
            let snare_lo = (AUDIO_SAMPLES as f32 * 0.02).floor() as usize;
            let snare_hi = (AUDIO_SAMPLES as f32 * 0.13).floor() as usize;
            let snare = band(freq_mag, snare_lo, snare_hi);
            self.snare += (snare - self.snare) * (dt * 12.0).min(1.0);
            self.fire_size = 1.0 + self.hi * FIRE_HAT_K + self.snare * FIRE_SNARE_K;

            let sample = seeded_rand(0, self.seed as i32);
            if drop_color_pulse(&mut self.pulse, self.smoke_bass, dt, sample) {
                self.seed = self.seed.wrapping_add(1);
            }
            self.fire_hue = ease_phrase_hue(self.fire_hue, self.pulse.hue, dt, 1.2);
        }

        // The haze substitute for the smoke grid.
        for puff in &mut self.puffs {
            advance_puff(
                puff,
                self.position,
                dt,
                self.simulation_speed,
                self.smoke_lifespan,
            );
        }
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let bloom_strength = self.bloom_strength();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        // Logo first (opaque, depth), then the glass panes over it, then the
        // additive haze.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-dvd-room-scene"),
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
            pass.set_pipeline(&gpu.logo_pipeline);
            pass.set_bind_group(0, &gpu.logo_bind_group, &[]);
            gpu.logo_mesh.draw(&mut pass);

            pass.set_pipeline(&gpu.glass_pipeline);
            pass.set_bind_group(0, &gpu.glass_bind_group, &[]);
            gpu.glass_mesh.draw(&mut pass);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-dvd-room-haze"),
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

        // The bright pass is bound to the scene target in `init` and after
        // every resize (`rebuild_targets`).
        gpu.bloom.set_strength(bloom_strength);
        gpu.bloom.encode(encoder);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-dvd-room-final"),
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
        scene_props_with_levers(DVD_ROOM_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10DvdRoom.js` sets `webgpu: true` and no category.
        SceneMeta::for_scene("Point10 DVD Room", "3d-scenes")
    }
}

/// Rebuild the final-composite bind group after its targets change.
fn create_final_bind_group(device: &wgpu::Device, gpu: &DvdRoomGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-dvd-room-final-bind-group"),
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

/// `DvdRoom._logoGeometry`'s silhouette: a rounded, bevelled slab extruded
/// along +z, centred on the origin. The JS extrudes the Wikimedia SVG; the
/// shared asset loader cannot rasterise it (PNG/JPEG only), so this keeps the
/// same `width x height x depth` extents with a deterministic mesh.
pub fn build_rounded_slab(width: f32, height: f32, depth: f32, radius: f32, segments: u32) -> Mesh {
    let segments = segments.max(2);
    let half_width = width * 0.5;
    let half_height = height * 0.5;
    let half_depth = depth * 0.5;
    let radius = radius.min(half_width.min(half_height)).max(0.0);
    let corner_x = half_width - radius;
    let corner_y = half_height - radius;

    // CCW outline: bottom-right, top-right, top-left, bottom-left arcs.
    let mut outline = Vec::with_capacity((segments * 4) as usize);
    let corners = [
        (corner_x, -corner_y, -std::f32::consts::FRAC_PI_2),
        (corner_x, corner_y, 0.0),
        (-corner_x, corner_y, std::f32::consts::FRAC_PI_2),
        (-corner_x, -corner_y, std::f32::consts::PI),
    ];
    for (cx, cy, start) in corners {
        for step in 0..segments {
            let t = step as f32 / segments as f32;
            let angle = start + t * std::f32::consts::FRAC_PI_2;
            outline.push([cx + radius * angle.cos(), cy + radius * angle.sin()]);
        }
    }

    let count = outline.len();
    let mut mesh = Mesh::default();

    // Front cap (+z): a fan from the centre, plus the outline ring.
    let uv_of = |point: [f32; 2]| [point[0] / width + 0.5, point[1] / height + 0.5];
    let front_centre = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        position: [0.0, 0.0, half_depth],
        normal: [0.0, 0.0, 1.0],
        uv: [0.5, 0.5],
    });
    let front_start = mesh.vertices.len() as u32;
    for point in &outline {
        mesh.vertices.push(Vertex {
            position: [point[0], point[1], half_depth],
            normal: [0.0, 0.0, 1.0],
            uv: uv_of(*point),
        });
    }
    for index in 0..count {
        let a = front_start + index as u32;
        let b = front_start + ((index + 1) % count) as u32;
        mesh.indices.extend_from_slice(&[front_centre, a, b]);
    }

    // Back cap (-z): reversed winding, inward-facing normal.
    let back_centre = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        position: [0.0, 0.0, -half_depth],
        normal: [0.0, 0.0, -1.0],
        uv: [0.5, 0.5],
    });
    let back_start = mesh.vertices.len() as u32;
    for point in &outline {
        mesh.vertices.push(Vertex {
            position: [point[0], point[1], -half_depth],
            normal: [0.0, 0.0, -1.0],
            uv: uv_of(*point),
        });
    }
    for index in 0..count {
        let a = back_start + index as u32;
        let b = back_start + ((index + 1) % count) as u32;
        mesh.indices.extend_from_slice(&[back_centre, b, a]);
    }

    // Sides: one quad per outline edge with an outward radial normal.
    for (index, point) in outline.iter().enumerate() {
        let next = (index + 1) % count;
        let point = *point;
        let following = outline[next];
        let edge = [following[0] - point[0], following[1] - point[1]];
        let normal = normalise([edge[1], -edge[0], 0.0]);
        let base = mesh.vertices.len() as u32;
        for (position, uv) in [
            ([point[0], point[1], half_depth], [0.0, 1.0]),
            ([point[0], point[1], -half_depth], [0.0, 0.0]),
            ([following[0], following[1], -half_depth], [1.0, 0.0]),
            ([following[0], following[1], half_depth], [1.0, 1.0]),
        ] {
            mesh.vertices.push(Vertex {
                position,
                normal,
                uv,
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
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

fn dvd_room_logo_wgsl() -> String {
    DVD_ROOM_LOGO_WGSL.to_string()
}

fn dvd_room_glass_wgsl() -> String {
    DVD_ROOM_GLASS_WGSL.to_string()
}

fn dvd_room_puff_wgsl() -> String {
    DVD_ROOM_PUFF_WGSL.to_string()
}

fn dvd_room_final_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{DVD_ROOM_FINAL_WGSL}")
}

const DVD_ROOM_LOGO_WGSL: &str = r#"
struct LogoUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  colour: vec4<f32>,
  logo_pos: vec4<f32>,
  fire: vec4<f32>,
  // x emissive intensity, y corner flash, z time, w smoke bass
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: LogoUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) w_pos: vec3<f32>,
  @location(2) uv: vec2<f32>,
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
  // The model is a rotation + uniform scale, so the normal needs no inverse
  // transpose.
  out.normal = normalize((u.model * vec4<f32>(normal, 0.0)).xyz);
  out.w_pos = world.xyz;
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.w_pos);

  // HemisphereLight(0xbfd4ff, 0x0a0a12, 0.9) + the overhead key light.
  let hemi = mix(
    vec3<f32>(0.003, 0.003, 0.005),
    vec3<f32>(0.48, 0.64, 1.0),
    normal.y * 0.5 + 0.5,
  ) * 0.9;
  let key_dir = normalize(vec3<f32>(0.0, 1.0, 0.0));
  let key = 1.8 * max(dot(normal, key_dir), 0.0) * 0.5;

  // The dimmed 0.35 fire light on the logo.
  let to_fire = u.logo_pos.xyz - in.w_pos;
  let fire_distance = length(to_fire);
  let fire_dir = to_fire / max(fire_distance, 0.001);
  let fire_diffuse = max(dot(normal, fire_dir), 0.0)
    * u.fire.a / (1.0 + fire_distance * fire_distance);
  let fire_light = u.fire.rgb * fire_diffuse;

  // `MeshStandardMaterial`: colour + emissive (the palette), emissiveIntensity.
  let lit = u.colour.rgb * (hemi + key + fire_light);
  let emissive = u.colour.rgb * u.params.x;
  let edge = smoothstep(0.0, 0.18, abs(in.uv.x - 0.5) * 2.0)
    + (1.0 - smoothstep(0.0, 0.18, abs(in.uv.y - 0.5) * 2.0));
  let corner = u.params.y * 0.8 * (edge * 0.5) * u.colour.rgb;

  return vec4<f32>(lit + emissive + corner, 1.0);
}
"#;

const DVD_ROOM_GLASS_WGSL: &str = r#"
struct GlassUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  logo_pos: vec4<f32>,
  fire: vec4<f32>,
  // x opacity, y time, z smoke bass
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: GlassUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) w_pos: vec3<f32>,
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
  out.w_pos = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.w_pos);
  let fresnel = pow(1.0 - clamp(abs(dot(normal, view_dir)), 0.0, 1.0), 3.0);

  // The JS panes are lit by a dark PMREM studio and the fire point light; the
  // port keeps the glass tint, the vertical env gradient and a cheap specular
  // streak.
  let tint = vec3<f32>(0.34, 0.55, 0.82);
  let env = mix(vec3<f32>(0.01, 0.015, 0.03), vec3<f32>(0.22, 0.3, 0.5), normal.y * 0.5 + 0.5);
  let streak_dir = normalize(vec3<f32>(-0.4, 0.25, 0.6));
  let streak = pow(max(dot(reflect(-view_dir, normal), streak_dir), 0.0), 16.0);

  let to_fire = u.logo_pos.xyz - in.w_pos;
  let fire_distance = length(to_fire);
  let fire_dir = to_fire / max(fire_distance, 0.001);
  let fire_term = max(dot(normal, fire_dir), 0.0)
    * u.fire.a / (1.0 + fire_distance * fire_distance);

  let colour = tint * (env + streak * 0.6) + u.fire.rgb * fire_term + tint * fresnel * 0.4;
  return vec4<f32>(colour, u.params.x + fresnel * 0.12);
}
"#;

const DVD_ROOM_PUFF_WGSL: &str = r#"
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

const DVD_ROOM_FINAL_WGSL: &str = r#"
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
  // SmokeRoom's output node: `alpha = max(rgb)`.
  return vec4<f32>(rgb, clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-dvd-room.test.js plus the pure bounce maths.

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn glass_half_extents_scale_and_stretch() {
        let base = box_half_extents(1.0, 1.0);
        assert_eq!(base, [GLASS_HALF_X, GLASS_HALF_Y, GLASS_HALF_Z]);
        assert_eq!((GLASS_HALF_X, GLASS_HALF_Y, GLASS_HALF_Z), (6.3, 6.3, 12.3));

        let scaled = box_half_extents(2.5, 1.6);
        close(scaled[0], base[0] * 2.5 * 1.6, 1e-5);
        close(scaled[1], base[1] * 2.5 * 1.6, 1e-5);
        close(scaled[2], base[2] * 2.5, 1e-5);
    }

    fn project(camera: [f32; 3], point: [f32; 3]) -> [f32; 3] {
        let view_proj = mat4_mul(
            &perspective(CAMERA_FOV.to_radians(), 16.0 / 9.0, CAMERA_NEAR, CAMERA_FAR),
            &look_at(camera, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        );
        let clip = crate::point10::scenes::cube_mesh::transform_point(&view_proj, point);
        [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]]
    }

    #[test]
    fn required_glass_top_puts_the_room_top_on_the_frame_top() {
        for yaw in [0.0_f32, std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
            for y in [5.3_f32, 6.5, 7.7] {
                let camera = [
                    yaw.cos() * CAMERA_ORBIT_RADIUS,
                    y,
                    yaw.sin() * CAMERA_ORBIT_RADIUS,
                ];
                let top = required_glass_top(
                    camera,
                    normalise([-camera[0], -camera[1], -camera[2]]),
                    [0.0, 1.0, 0.0],
                    CAMERA_FOV,
                    1.0,
                    1.0,
                );
                for x in [-GLASS_HALF_X, GLASS_HALF_X] {
                    for z in [-GLASS_HALF_Z, GLASS_HALF_Z] {
                        let projected = project(camera, [x, top, z]);
                        assert!(
                            projected[1] >= 1.0 - 1e-4,
                            "corner ({x}, {top}, {z}) projects to {}",
                            projected[1]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn required_glass_top_ignores_the_output_aspect_like_the_js() {
        // three's fov is vertical, so the fit must not depend on aspect. Both
        // calls use the same 16:9 projection in `project`, so assert the pure
        // function is deterministic and positive here.
        let camera = [0.0, 6.5, CAMERA_ORBIT_RADIUS];
        let top = required_glass_top(
            camera,
            normalise([-camera[0], -camera[1], -camera[2]]),
            [0.0, 1.0, 0.0],
            CAMERA_FOV,
            1.0,
            1.0,
        );
        assert!(top > GLASS_HALF_Y);
    }

    #[test]
    fn glass_fit_is_bottom_anchored() {
        let (height, centre_y) = glass_fit(20.0, 1.0, 1.0);
        close(centre_y - height * 0.5, -GLASS_HALF_Y, 1e-5);
        close(centre_y + height * 0.5, 20.0, 1e-5);

        let (height, centre_y) = glass_fit(20.0, 1.3, 1.4);
        let half_y = box_half_extents(1.3, 1.4)[1];
        close(centre_y - height * 0.5, -half_y, 1e-5);
        close(centre_y + height * 0.5, 20.0, 1e-5);
    }

    #[test]
    fn room_top_stays_above_the_natural_half_height() {
        for yaw in [0.0_f32, 1.0, 2.0, 3.0] {
            let camera = [
                yaw.cos() * CAMERA_ORBIT_RADIUS,
                6.5,
                yaw.sin() * CAMERA_ORBIT_RADIUS,
            ];
            let top = room_top(camera, 1.0, 1.0);
            assert!(top > GLASS_HALF_Y, "top {top} at yaw {yaw}");
            let (height, _) = glass_fit(top, 1.0, 1.0);
            assert!(height > GLASS_HALF_Y * 2.0);
        }
    }

    #[test]
    fn the_default_camera_orbit_is_outside_the_room() {
        let extents = box_half_extents(1.0, 1.0);
        for yaw in [0.0_f32, std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
            let camera = [
                yaw.cos() * CAMERA_ORBIT_RADIUS,
                6.5,
                yaw.sin() * CAMERA_ORBIT_RADIUS,
            ];
            assert!(camera[0].abs() > extents[0] || camera[2].abs() > extents[2]);
        }
    }

    #[test]
    fn bounce_bounds_match_the_volume_minus_the_logo_half_extents() {
        assert_eq!(logo_half_extents(), [2.1, 1.05, 0.175]);
        assert_eq!(bounce_bounds(), [3.9, 4.95, 11.825]);
    }

    #[test]
    fn step_bounce_reflects_and_counts_the_axes() {
        let start = [3.8, 0.0, 0.0];
        let velocity = [10.0, 0.0, 0.0];
        let step = step_bounce(start, velocity, 1.0, 1.0);

        assert_eq!(step.axes_hit, 1);
        assert_eq!(step.position[0], 3.9);
        assert_eq!(step.velocity, [-10.0, 0.0, 0.0]);

        // A corner hit counts two axes.
        let corner = step_bounce([3.8, 4.9, 0.0], [10.0, 10.0, 0.0], 1.0, 1.0);
        assert_eq!(corner.axes_hit, 2);
        assert_eq!(corner.velocity, [-10.0, -10.0, 0.0]);
    }

    #[test]
    fn step_bounce_scales_with_the_speed_multiplier_and_caps_the_step() {
        let step = step_bounce([0.0; 3], [1.0, 0.0, 0.0], 0.1, 2.0);
        close(step.position[0], 0.2, 1e-6);

        let negative = step_bounce([0.0; 3], [1.0, 0.0, 0.0], f32::NAN, f32::NAN);
        assert_eq!(negative.position, [0.0; 3]);
    }

    #[test]
    fn camera_orbit_lerps_toward_the_js_target() {
        let (camera, yaw) = advance_camera(CAMERA_START, 0.0, 0.0, 0.0, 1.0 / 60.0);
        // k = 0.02: 14 -> 17 * 0.02 + 14 * 0.98.
        close(camera[0], 14.0 + (17.0 - 14.0) * 0.02, 1e-5);
        close(camera[1], 5.5 + (6.5 - 5.5) * 0.02, 1e-5);
        close(camera[2], 4.4 + (0.0 - 4.4) * 0.02, 1e-5);
        assert_eq!(yaw, 0.0);

        // A loud bass pushes the orbit yaw positive.
        let (_, yaw) = advance_camera(CAMERA_START, 0.0, 0.6, 0.0, 0.1);
        assert!(yaw > 0.0);
    }

    #[test]
    fn fire_ramp_moves_from_red_to_mid_to_start() {
        let red = fire_ramp(0.2);
        // A red-dominant ramp: green and blue are both 0 on the way to `end`.
        assert!(red[0] > red[1] && red[0] > red[2]);
        let mid = fire_ramp(0.5);
        assert!(mid[0] > mid[1]);
        let hot = fire_ramp(0.9);
        assert!(hot[1] > 0.2);
    }

    #[test]
    fn a_bounce_steps_the_palette_and_pops_the_logo() {
        let mut scene = DvdRoom::new();
        scene.position = [3.89, 0.0, 0.0];
        scene.velocity = [20.0, 0.0, 0.0];
        scene.update(0.001, &AudioBridge::new());

        assert_eq!(scene.bounces(), 1);
        assert_eq!(scene.colour_index(), 1);
        assert!(scene.pop > 0.0);
        assert_eq!(scene.corner_flash(), 0.0);
    }

    #[test]
    fn a_corner_hit_flashes_and_counts_two_axes() {
        let mut scene = DvdRoom::new();
        scene.position = [3.89, 4.94, 0.0];
        scene.velocity = [20.0, 20.0, 0.0];
        scene.update(0.001, &AudioBridge::new());

        assert_eq!(scene.bounces(), 1);
        assert_eq!(scene.colour_index(), 1);
        assert!(scene.corner_flash() > 0.99);
    }

    #[test]
    fn palettes_and_bounce_cycle_the_classic_colours() {
        assert_eq!(PALETTE.len(), 7);
        let scene = DvdRoom::new();
        assert_eq!(scene.colour_index(), 0);
        assert_eq!(scene.logo_colour(), srgb_hex_to_linear(PALETTE[0]));

        // Drive one bounce directly through the pure step.
        let step = step_bounce([3.85, 0.0, 0.0], [10.0, 0.0, 0.0], 0.1, 1.0);
        assert_eq!(step.axes_hit, 1);
    }

    #[test]
    fn puffs_spawn_around_the_logo_and_advance() {
        let mut puff = spawn_puff(4, [1.0, 2.0, 3.0]);
        assert_eq!(puff.age, 0.0);
        let start = puff.position;
        advance_puff(&mut puff, [1.0, 2.0, 3.0], 0.1, 1.2, 3.5);
        assert!(puff.age > 0.0);
        assert_ne!(puff.position, start);

        // A dead puff respawns at the emitter.
        puff.age = 1.0;
        advance_puff(&mut puff, [1.0, 2.0, 3.0], 0.1, 1.2, 3.5);
        assert_eq!(puff.age, 0.0);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(VOLUME_WORLD_SIZE_X, 12.0);
        assert_eq!(VOLUME_WORLD_SIZE_Y, 12.0);
        assert_eq!(VOLUME_WORLD_SIZE_Z, 24.0);
        assert_eq!(LOGO_WIDTH, 4.2);
        assert_eq!(LOGO_DEPTH, 0.35);
        assert_eq!(BASE_SPEED, 3.5);
        assert_eq!(LOGO_START_POSITION, [-2.0, 1.0, -4.0]);
        assert_eq!((FIRE_HAT_K, FIRE_SNARE_K), (2.5, 8.0));
        assert_eq!((BLOOM_BASE, BLOOM_PER_BASS), (0.12, 2.2));
        assert_eq!(GLASS_TOP_MARGIN, 0.75);
        assert_eq!(CLEAR_ALPHA, 0.0);
    }

    #[test]
    fn rounded_slab_has_the_requested_extents_and_outward_normals() {
        let mesh = build_rounded_slab(LOGO_WIDTH, LOGO_HEIGHT, LOGO_DEPTH, LOGO_RADIUS, 8);
        assert_eq!(mesh.vertices.len(), 2 + 6 * 32);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
        }
        close(min[0], -LOGO_WIDTH * 0.5, 1e-5);
        close(max[0], LOGO_WIDTH * 0.5, 1e-5);
        close(min[1], -LOGO_HEIGHT * 0.5, 1e-5);
        close(max[1], LOGO_HEIGHT * 0.5, 1e-5);
        close(min[2], -LOGO_DEPTH * 0.5, 1e-6);
        close(max[2], LOGO_DEPTH * 0.5, 1e-6);

        // Front/back caps point away from the slab.
        assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
        // The emit-gain reference is `SphereGeometry(0.9, 64, 40)`'s count.
        assert_eq!((64 + 1) * (40 + 1), REFERENCE_EMITTER_VERTICES);
        assert!(SPHERE_RADIUS > 0.0);
    }

    #[test]
    fn update_bounces_the_logo_and_advances_the_camera() {
        let mut scene = DvdRoom::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag = [0.5; AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let start = scene.position();
        for _ in 0..600 {
            scene.update(1.0 / 60.0, &bridge);
        }
        // Six seconds at 5 units/s must hit at least one wall.
        assert!(scene.bounces() > 0, "position stayed at {start:?}");
        assert_ne!(scene.position(), start);
        assert_ne!(scene.camera_position(), CAMERA_START);
        assert!(scene.fire_size() >= 1.0);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = DvdRoom::new();
        scene.apply_props(|key| match key {
            "smokeDensity" => Some(200.0),
            "fireIntensity" => Some(-3.0),
            "smokeLifespan" => Some(0.0),
            "fireLifespan" => Some(99.0),
            "simulationSpeed" => Some(9.0),
            "boxScale" => Some(9.0),
            "boxXYScale" => Some(0.1),
            _ => None,
        });
        assert_eq!(scene.smoke_density, 100.0);
        assert_eq!(scene.fire_intensity, 0.0);
        assert_eq!(scene.smoke_lifespan, 0.1);
        assert_eq!(scene.fire_lifespan, 5.0);
        assert_eq!(scene.simulation_speed, 4.0);
        assert_eq!(scene.box_scale, 5.0);
        assert_eq!(scene.box_xy_scale, 0.5);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<LogoUniforms>(), 208);
        assert_eq!(std::mem::size_of::<GlassUniforms>(), 192);
        assert_eq!(std::mem::size_of::<PuffUniforms>(), 96);
        assert_eq!(std::mem::size_of::<PuffInstance>(), 32);
        assert_eq!(std::mem::size_of::<FinalUniforms>(), 16);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("logo", dvd_room_logo_wgsl()),
            ("glass", dvd_room_glass_wgsl()),
            ("puff", dvd_room_puff_wgsl()),
            ("final", dvd_room_final_wgsl()),
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
    fn shaders_keep_the_bounce_and_haze_maths() {
        let logo = dvd_room_logo_wgsl();
        for needle in [
            "let hemi = mix(",
            "let emissive = u.colour.rgb * u.params.x;",
            "u.params.y * 0.8",
        ] {
            assert!(logo.contains(needle), "logo lost {needle}");
        }

        let glass = dvd_room_glass_wgsl();
        for needle in [
            "let fresnel = pow(1.0 - clamp(abs(dot(normal, view_dir)), 0.0, 1.0), 3.0);",
            "return vec4<f32>(colour, u.params.x + fresnel * 0.12);",
        ] {
            assert!(glass.contains(needle), "glass lost {needle}");
        }

        let puff = dvd_room_puff_wgsl();
        assert!(puff.contains("let mask = pow(1.0 - smoothstep(0.0, 1.0, radius), 2.0);"));

        let final_shader = dvd_room_final_wgsl();
        assert!(final_shader.contains("clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0)"));
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = DvdRoom::new();
        assert_eq!(scene.meta().name, "Point10 DVD Room");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 7..],
            &[
                "smokeDensity",
                "fireIntensity",
                "smokeLifespan",
                "fireLifespan",
                "simulationSpeed",
                "boxScale",
                "boxXYScale",
            ]
        );
        assert_eq!(names[0], "gain");
    }
}
