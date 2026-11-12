//! SmokeRoom — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/SmokeRoom.js`.
//!
//! The JS scene is **WebGPU/TSL**: a 150x150x300 voxel velocity/dye simulation
//! (seven compute passes per 1/120 s step) rendered by a 16-step
//! `VolumeNodeMaterial` raymarch inside a glass room, with a lava sphere at the
//! centre as the emitter. The music pulses the emission (`uFireSize` from the
//! hi-hat and snare bands), rotates the fire hue on bass phrase crests, and
//! drives the bloom.
//!
//! # Ported exactly
//!
//! * the room (`:60-88`, `:130-185`): `VOLUME_WORLD_SIZE 12 x 12 x 24`,
//!   `GLASS_PADDING 0.6`, `GLASS_TOP_MARGIN 0.75`, the bottom-anchored
//!   `glassFit`/`requiredGlassTop` stretch ([`room_top`] / [`glass_fit`]), and
//!   `boxScale`/`boxXYScale` applied to the room while the emitter radius stays
//!   on `boxScale` alone;
//! * the audio mapping (`:340-374`): bass = mean of the first five bins with
//!   `dt * 8` smoothing, the hi-hat band = mean of the top 15% with `dt * 10`,
//!   the snare band = mean of bins 2%..13% with `dt * 12`,
//!   `uFireSize = 1 + hi * 2.5 + snare * 8`, the `dropColorPulse` hue flip and
//!   the `dt * 1.2` shortest-arc ease;
//! * the fire ramp (`:850-858`) and its hue equalisation ([`fire_colour`]);
//! * the lava emitter's colour law (`:1034-1063`): the three-octave noise
//!   ramp, `pow(2.5)`, `fireRamp(t + 0.1)`, `saturation(…, 1.1)`, the
//!   `(emitTemperature / 33.36)^4` and `(emitDensity / 44.08)` scalars,
//!   `uFireIntensity`, the flicker and the 3-second fade-in, times `0.2`;
//! * the bloom (`:1394-1400`): `min(0.12 + bass * 2.2, 2.8)`;
//! * the renderer grade (`:417-418`): ACES filmic, exposure 2;
//! * the composite (`:1270-1276`): `alpha = max(rgb)` on transparent black;
//! * the prop binder (`sceneProps.js:18-38`, `bindSmoke`):
//!   `smokeDensity` → emit density, `fireIntensity`, the two lifespans,
//!   `simulationSpeed`, `boxScale`, `boxXYScale`.
//!
//! # Replaced TSL machinery — the smoke volume
//!
//! Like [`super::dvd_room`], the compute/storage-texture path does not exist in
//! this crate, so the **emission behaviour** is kept and the density field is
//! replaced with a procedural additive haze:
//!
//! | SmokeRoom (TSL) | SmokeRoom (Rust) |
//! |-----------------|------------------|
//! | 3D velocity/dye/pressure grids + seven compute passes | 44 CPU puffs launched radially from the emitter sphere |
//! | the grid boundary damps velocity to zero (the "room") | the puffs reflect off the scaled room box, so the fire stays inside |
//! | `uFireSize` scales the per-vertex emission | the same, scaling the puff brightness |
//! | `uEmitGain` (density normalisation) | kept as a brightness reference (the port emits 44 puffs, not 128k vertices) |
//! | the volume raymarch + bloom | additive puffs + [`UnrealBloomPass`] |
//!
//! Every puff is seeded with [`seeded_rand`] instead of `Math.random`, so a run
//! is deterministic. The emitter sphere, glass room, camera, palette, audio
//! mapping and post chain are ported as they are.
//!
//! # Other deviations
//!
//! * **The glass is a fresnel/env-gradient shader, not a PMREM studio.** No
//!   PMREM pass exists here; the port keeps the fixed `0.35` opacity, the blue
//!   tint and the fire-light term (the same substitution DVD Room documents).
//! * The lava sphere is a 32x16 UV sphere (the JS is 64x40): at the room scale
//!   the segment count is invisible.
//! * `RoundedBoxGeometry` becomes a six-pane box; the 1.2 corner radius only
//!   softens the silhouette.
//! * The point-light range fade-in (`:1417-1426`) is folded into the bloom and
//!   emissive brightness: the port has no shadow-casting point light.
//!
//! [`UnrealBloomPass`]: crate::point10::postfx::UnrealBloomPass

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge};
use crate::point10::postfx::{Composer, ToneMapping, UnrealBloomPass};
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, create_depth, depth_state, look_at, mat4_mul, perspective,
};
use crate::point10::scenes::dvd_room::{
    BASS_BINS, FIRE_HAT_K, FIRE_SNARE_K, GLASS_HALF_X, GLASS_HALF_Y, GLASS_HALF_Z, Puff,
    SPHERE_RADIUS, band, box_half_extents, fire_colour, glass_fit, room_top, treble_band,
};
use crate::point10::scenes::fire_common::build_sphere;
use crate::point10::shared::phrase_color::{DropColorState, drop_color_pulse, ease_phrase_hue};
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    UnitQuad, frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// `BLOOM_BASE`.
pub const BLOOM_BASE: f32 = 0.12;
/// `BLOOM_PER_BASS`.
pub const BLOOM_PER_BASS: f32 = 2.2;
/// The JS clamps the smoke-room bloom at 2.8.
pub const BLOOM_MAX: f32 = 2.8;

/// `uEmitDensity` default.
pub const EMIT_DENSITY: f32 = 28.0;
/// `uEmitTemperature` default.
pub const EMIT_TEMPERATURE: f32 = 22.0;
/// `uFireIntensity` default.
pub const FIRE_INTENSITY: f32 = 40.0;

/// `camera.position.set(14.0, 5.5, 4.4)`.
pub const CAMERA_START: [f32; 3] = [14.0, 5.5, 4.4];
/// `PerspectiveCamera(60, aspect, 0.1, 100)`.
pub const CAMERA_FOV: f32 = 60.0;
/// Near plane.
pub const CAMERA_NEAR: f32 = 0.1;
/// Far plane.
pub const CAMERA_FAR: f32 = 100.0;

/// The haze puff count standing in for the 150x150x300 grid.
pub const PUFF_COUNT: usize = 44;
/// The per-puff brightness the density/bass scalars multiply. Calibrated so
/// one puff reads like DVD Room's (`0.09` times its emit gain).
pub const PUFF_GAIN: f32 = 0.09;

/// `scene.background = null`.
pub const CLEAR_ALPHA: f64 = 0.0;

/// The glass material's fixed opacity (`opacity: 0.35`).
pub const GLASS_OPACITY: f32 = 0.35;

/// `fireRamp`'s three stops (`0xffe68c` / `0xff7305` / `0xff0000`).
pub const FIRE_START_HEX: u32 = 0x00ff_e68c;
/// Mid stop.
pub const FIRE_MID_HEX: u32 = 0x00ff_7305;
/// End stop.
pub const FIRE_END_HEX: u32 = 0x00ff_0000;

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// `_applyBoxScale`'s room extents: `boxHalfExtents` already lives in
/// [`super::dvd_room`]; this is the emitter's scaled radius.
pub fn emitter_radius(box_scale: f32) -> f32 {
    let scale = if box_scale.is_finite() {
        box_scale.max(0.0)
    } else {
        1.0
    };
    SPHERE_RADIUS * scale
}

/// The room's half extents used to keep puffs inside (["the room"]).
pub fn room_bounds(box_scale: f32, box_xy_scale: f32) -> [f32; 3] {
    box_half_extents(box_scale, box_xy_scale)
}

/// `uFireSize = 1 + hi * FIRE_HAT_K + snare * FIRE_SNARE_K` (`:364-365`).
pub fn fire_size(hi: f32, snare: f32) -> f32 {
    1.0 + hi * FIRE_HAT_K + snare * FIRE_SNARE_K
}

/// The hi-hat band: mean of the top 15% of the FFT (`:353-359`).
pub fn hi_band(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    treble_band(freq_mag)
}

/// The snare band: mean of bins 2%..13% (`:358-363`).
pub fn snare_band(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    let lo = (AUDIO_SAMPLES as f32 * 0.02).floor() as usize;
    let hi = (AUDIO_SAMPLES as f32 * 0.13).floor() as usize;
    band(freq_mag, lo, hi)
}

/// The bass the smoke room reads: the first five bins (`:346-349`).
pub fn smoke_bass(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    band(freq_mag, 0, BASS_BINS)
}

/// The lava emitter's emissive scale (`:1056-1063`) before the fire ramp.
pub fn emitter_glow(
    emit_temperature: f32,
    emit_density: f32,
    fire_intensity: f32,
    time: f32,
) -> f32 {
    let temp_scale = (emit_temperature / 33.36).max(0.0);
    let temp_factor = temp_scale.powi(4);
    let density_scale = (emit_density / 44.08).max(0.0);
    let fade_in = smoothstep01((time / 3.0).clamp(0.0, 1.0));
    temp_factor * density_scale * fire_intensity * fade_in * 0.2
}

fn smoothstep01(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Spawn a puff on the emitter sphere, travelling radially outward (`:683-749`
/// emits from every sphere vertex; the port samples the surface).
pub fn spawn_room_puff(seed: u32, radius: f32) -> Puff {
    let rand = |offset: u32| seeded_rand(seed as i32, offset as i32);
    // A uniform point on the sphere (golden-angle spiral).
    let fraction = rand(1);
    let y = 1.0 - 2.0 * fraction;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    let theta = rand(2) * std::f32::consts::TAU;
    let direction = [theta.cos() * ring, y, theta.sin() * ring];
    let speed = 1.4 + rand(3) * 2.2;
    Puff {
        position: [
            direction[0] * radius,
            direction[1] * radius,
            direction[2] * radius,
        ],
        velocity: [
            direction[0] * speed,
            direction[1] * speed + 0.35,
            direction[2] * speed,
        ],
        age: 0.0,
        size: 0.5 + rand(4) * 0.9,
        alpha: 0.6 + rand(5) * 0.4,
        temperature: 0.45 + rand(6) * 0.5,
        seed: seed.wrapping_add(1),
    }
}

/// Advance a puff and keep it inside the room (`:586-670`'s boundary damping,
/// as a reflective clamp). Dead puffs respawn at the emitter.
pub fn advance_room_puff(
    puff: &mut Puff,
    dt: f32,
    simulation_speed: f32,
    lifespan: f32,
    radius: f32,
    bounds: [f32; 3],
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
        *puff = spawn_room_puff(puff.seed, radius);
        return;
    }
    puff.age = (puff.age + step / life).min(1.0);
    puff.position = [
        puff.position[0] + puff.velocity[0] * step,
        puff.position[1] + puff.velocity[1] * step,
        puff.position[2] + puff.velocity[2] * step,
    ];
    puff.velocity = [
        puff.velocity[0] * (1.0 - 0.35 * step),
        puff.velocity[1] * (1.0 - 0.2 * step) + 0.35 * step,
        puff.velocity[2] * (1.0 - 0.35 * step),
    ];
    puff.size += step * 1.5;

    // The room: the JS damps velocity to zero at the grid edge so the smoke
    // never escapes; a reflected clamp reads the same and keeps the volume
    // full.
    for axis in 0..3 {
        let limit = (bounds[axis] - puff.size * 0.5).max(0.05);
        if puff.position[axis] > limit {
            puff.position[axis] = limit;
            puff.velocity[axis] = -puff.velocity[axis].abs() * 0.5;
        } else if puff.position[axis] < -limit {
            puff.position[axis] = -limit;
            puff.velocity[axis] = puff.velocity[axis].abs() * 0.5;
        }
    }
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `SMOKE_ROOM_PROPS` (`sceneProps.js:18-38`).
const SMOKE_ROOM_PROPS: &[SceneProp] = &[
    SceneProp::float("smokeDensity", "Smoke Density", 28.0, 0.0, 100.0, 0.1),
    SceneProp::float("fireIntensity", "Fire Intensity", 40.0, 0.0, 100.0, 0.1),
    SceneProp::float("smokeLifespan", "Smoke Lifespan", 3.5, 0.1, 20.0, 0.1),
    SceneProp::float("fireLifespan", "Fire Lifespan", 1.3, 0.1, 5.0, 0.1),
    SceneProp::float("simulationSpeed", "Simulation Speed", 1.2, 0.1, 4.0, 0.01),
    SceneProp::float("boxScale", "Box Scale", 1.0, 0.5, 5.0, 0.05),
    SceneProp::float("boxXYScale", "Box Width & Height", 1.0, 0.5, 5.0, 0.05),
];

/// The glass shader's uniform block (176 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GlassUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// rgb = fire colour, w = fire intensity.
    fire: [f32; 4],
    /// x opacity, y time, z bass.
    params: [f32; 4],
}

/// The emitter shader's uniform block (176 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct EmitterUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// rgb = fire start, w = glow scale.
    glow: [f32; 4],
    /// x time, y flame height, z flicker, w unused.
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
struct SmokeRoomGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    glass_pipeline: wgpu::RenderPipeline,
    glass_uniforms: wgpu::Buffer,
    glass_bind_group: wgpu::BindGroup,
    emitter_pipeline: wgpu::RenderPipeline,
    emitter_uniforms: wgpu::Buffer,
    emitter_bind_group: wgpu::BindGroup,
    puff_pipeline: wgpu::RenderPipeline,
    puff_uniforms: wgpu::Buffer,
    puff_bind_group: wgpu::BindGroup,
    puff_instances: wgpu::Buffer,
    glass_mesh: MeshGpu,
    emitter_mesh: MeshGpu,
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

/// SmokeRoom scene.
pub struct SmokeRoom {
    /// `this._bass`.
    bass: f32,
    /// `this._hi`.
    hi: f32,
    /// `this._snare`.
    snare: f32,
    /// `uFireSize`.
    fire_size: f32,
    /// `this._uFireHue`.
    fire_hue: f32,
    /// `this._pulse`.
    pulse: DropColorState,
    /// The seed stream behind the drop detector and puff seeding.
    seed: u32,
    /// `this.time` (`BaseSceneWebGPU.update` accumulates the frame delta).
    time: f32,
    /// The haze puffs.
    puffs: [Puff; PUFF_COUNT],
    /// Props.
    smoke_density: f32,
    fire_intensity: f32,
    smoke_lifespan: f32,
    fire_lifespan: f32,
    simulation_speed: f32,
    box_scale: f32,
    box_xy_scale: f32,
    /// The glass fit (`glass.scale.y` / `glass.position.y`).
    glass_height: f32,
    glass_centre_y: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<SmokeRoomGpu>,
}

impl Default for SmokeRoom {
    fn default() -> Self {
        Self::new()
    }
}

impl SmokeRoom {
    /// Create the scene (`SmokeRoom.js:99-127` defaults).
    pub fn new() -> Self {
        let mut puffs = [Puff::dead(); PUFF_COUNT];
        for (index, puff) in puffs.iter_mut().enumerate() {
            puff.seed = index as u32 * 11 + 3;
        }
        Self {
            bass: 0.0,
            hi: 0.0,
            snare: 0.0,
            fire_size: 1.0,
            fire_hue: 0.0,
            pulse: DropColorState::new(),
            seed: 0,
            time: 0.0,
            puffs,
            smoke_density: EMIT_DENSITY,
            fire_intensity: FIRE_INTENSITY,
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

    /// Apply the resolved modV props (`sceneProps.js:18-38`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(SMOKE_ROOM_PROPS, props) {
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
        self.refresh_glass_fit();
    }

    /// `uFireSize` (`:364-365`).
    pub fn fire_size(&self) -> f32 {
        self.fire_size
    }

    /// `uFireHue`.
    pub fn fire_hue(&self) -> f32 {
        self.fire_hue
    }

    /// `this._bass`.
    pub fn bass(&self) -> f32 {
        self.bass
    }

    /// The haze puffs.
    pub fn puffs(&self) -> &[Puff; PUFF_COUNT] {
        &self.puffs
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// The smoke-room bloom strength (`:1394-1400`).
    pub fn bloom_strength(&self) -> f32 {
        (BLOOM_BASE + self.bass * BLOOM_PER_BASS).min(BLOOM_MAX)
    }

    /// The fire light the glass reflects (`:499`, `:953`).
    pub fn fire_light(&self) -> [f32; 4] {
        let colour = fire_colour(0.72, self.fire_hue);
        // uPointLightVolumeIntensity = 2, uFireIntensity = 40.
        let intensity = (self.fire_intensity / 40.0).max(0.0) * 2.0;
        [
            colour[0] * intensity * 0.15,
            colour[1] * intensity * 0.15,
            colour[2] * intensity * 0.15,
            intensity * 0.15,
        ]
    }

    /// The lava emitter's emissive scale.
    pub fn emitter_glow(&self) -> f32 {
        emitter_glow(
            EMIT_TEMPERATURE,
            self.smoke_density,
            self.fire_intensity,
            self.time,
        )
    }

    fn emitter_radius(&self) -> f32 {
        emitter_radius(self.box_scale)
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &look_at(CAMERA_START, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        )
    }

    fn glass_model(&self) -> Mat4 {
        let mut model =
            crate::point10::scenes::cube_mesh::translation(0.0, self.glass_centre_y, 0.0);
        model = mat4_mul(
            &model,
            &crate::point10::scenes::cube_mesh::scale(
                GLASS_HALF_X * 2.0 * self.box_scale * self.box_xy_scale,
                self.glass_height,
                GLASS_HALF_Z * 2.0 * self.box_scale,
            ),
        );
        model
    }

    fn refresh_glass_fit(&mut self) {
        let top = room_top(CAMERA_START, self.box_scale, self.box_xy_scale);
        let (height, centre_y) = glass_fit(top, self.box_scale, self.box_xy_scale);
        self.glass_height = height;
        self.glass_centre_y = centre_y;
    }

    fn billboard_basis(&self) -> ([f32; 3], [f32; 3]) {
        let forward = crate::point10::scenes::cube_mesh::normalise([
            -CAMERA_START[0],
            -CAMERA_START[1],
            -CAMERA_START[2],
        ]);
        let right = crate::point10::scenes::cube_mesh::normalise([-forward[2], 0.0, forward[0]]);
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
                let fire = (self.fire_size / 1.0).max(0.0);
                let decay = (-puff.age * (1.3 / self.fire_lifespan.max(0.1))).exp();
                let colour = fire_colour(puff.temperature * fade, self.fire_hue);
                let alpha = PUFF_GAIN * density * fire * puff.alpha * fade * fade * decay;
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
        let fire = self.fire_light();
        gpu.queue.write_buffer(
            &gpu.glass_uniforms,
            0,
            bytemuck::bytes_of(&GlassUniforms {
                view_proj,
                model: self.glass_model(),
                camera_pos: [CAMERA_START[0], CAMERA_START[1], CAMERA_START[2], 1.0],
                fire,
                params: [GLASS_OPACITY, self.time, self.bass, 0.0],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.emitter_uniforms,
            0,
            bytemuck::bytes_of(&EmitterUniforms {
                view_proj,
                model: crate::point10::scenes::cube_mesh::identity(),
                camera_pos: [CAMERA_START[0], CAMERA_START[1], CAMERA_START[2], 1.0],
                glow: [
                    fire_colour(0.55, self.fire_hue)[0],
                    fire_colour(0.55, self.fire_hue)[1],
                    fire_colour(0.55, self.fire_hue)[2],
                    self.emitter_glow(),
                ],
                params: [self.time, 3.5, 1.0, 0.0],
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
            "point10-smoke-room-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) = create_depth(
            &gpu.device,
            "point10-smoke-room-depth",
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

impl Point10Scene for SmokeRoom {
    fn name(&self) -> &'static str {
        "Point10 Smoke Room"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width.max(1), ctx.height.max(1));
        self.camera_aspect = ctx.aspect();
        self.refresh_glass_fit();

        let glass_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-smoke-room-glass-shader"),
            source: wgpu::ShaderSource::Wgsl(glass_wgsl().into()),
        });
        let glass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-smoke-room-glass-layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let glass_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-smoke-room-glass-uniforms"),
            size: std::mem::size_of::<GlassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glass_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-smoke-room-glass-bind-group"),
            layout: &glass_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: glass_uniforms.as_entire_binding(),
            }],
        });
        let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-smoke-room-glass-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-smoke-room-glass-pipeline-layout"),
                    bind_group_layouts: &[Some(&glass_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &glass_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(crate::point10::scenes::cube_mesh::Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // --- lava emitter ---------------------------------------------------
        let emitter_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-smoke-room-emitter-shader"),
            source: wgpu::ShaderSource::Wgsl(emitter_wgsl().into()),
        });
        let emitter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-smoke-room-emitter-layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let emitter_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-smoke-room-emitter-uniforms"),
            size: std::mem::size_of::<EmitterUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let emitter_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-smoke-room-emitter-bind-group"),
            layout: &emitter_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: emitter_uniforms.as_entire_binding(),
            }],
        });
        let emitter_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-smoke-room-emitter-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-smoke-room-emitter-pipeline-layout"),
                    bind_group_layouts: &[Some(&emitter_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &emitter_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(crate::point10::scenes::cube_mesh::Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &emitter_shader,
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

        // --- haze puffs -----------------------------------------------------
        let puff_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-smoke-room-puff-shader"),
            source: wgpu::ShaderSource::Wgsl(puff_wgsl().into()),
        });
        let puff_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-smoke-room-puff-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let puff_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-smoke-room-puff-uniforms"),
            size: std::mem::size_of::<PuffUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let puff_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-smoke-room-puff-bind-group"),
            layout: &puff_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: puff_uniforms.as_entire_binding(),
            }],
        });
        let quad = UnitQuad::new(&device);
        let puff_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-smoke-room-puff-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-smoke-room-puff-pipeline-layout"),
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
                label: Some("point10-smoke-room-puff-instances"),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };

        // --- targets, bloom, final composite --------------------------------
        let scene_target = TextureTarget::new(
            &device,
            "point10-smoke-room-scene",
            self.size.0,
            self.size.1,
            HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (depth, depth_view) = create_depth(
            &device,
            "point10-smoke-room-depth",
            self.size.0,
            self.size.1,
        );

        let mut bloom = UnrealBloomPass::new(&device, &queue, self.size.0, self.size.1);
        bloom.set_strength(BLOOM_BASE);
        bloom.set_radius(1.0);
        bloom.set_threshold(0.5);
        bloom.set_input(&scene_target.view);

        let final_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-smoke-room-final-shader"),
            source: wgpu::ShaderSource::Wgsl(final_wgsl().into()),
        });
        let final_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-smoke-room-final-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let final_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-smoke-room-final-uniforms"),
            size: std::mem::size_of::<FinalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let final_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-smoke-room-final-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let final_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-smoke-room-final-pipeline",
            &final_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-smoke-room-final-pipeline-layout"),
                bind_group_layouts: &[Some(&final_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );
        let final_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-smoke-room-final-bind-group"),
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

        let glass_mesh = MeshGpu::new(
            &device,
            "point10-smoke-room-glass",
            &build_box(1.0, 1.0, 1.0),
        );
        let emitter_mesh = MeshGpu::new(
            &device,
            "point10-smoke-room-emitter",
            &build_sphere(SPHERE_RADIUS, 32, 16),
        );

        self.gpu = Some(SmokeRoomGpu {
            device,
            queue,
            glass_pipeline,
            glass_uniforms,
            glass_bind_group,
            emitter_pipeline,
            emitter_uniforms,
            emitter_bind_group,
            puff_pipeline,
            puff_uniforms,
            puff_bind_group,
            puff_instances,
            glass_mesh,
            emitter_mesh,
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
        let radius = self.emitter_radius();
        for puff in &mut self.puffs {
            *puff = spawn_room_puff(puff.seed, radius);
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

        if audio.is_available() {
            let freq_mag = &audio.freq_mag;
            let bass = smoke_bass(freq_mag);
            self.bass += (bass - self.bass) * (dt * 8.0).min(1.0);

            let hi = hi_band(freq_mag);
            let snare = snare_band(freq_mag);
            self.hi += (hi - self.hi) * (dt * 10.0).min(1.0);
            self.snare += (snare - self.snare) * (dt * 12.0).min(1.0);
            self.fire_size = fire_size(self.hi, self.snare);

            // The drop detector + shortest-arc hue ease (`:370-373`).
            let sample = seeded_rand(0, self.seed as i32);
            if drop_color_pulse(&mut self.pulse, self.bass, dt, sample) {
                self.seed = self.seed.wrapping_add(1);
            }
            self.fire_hue = ease_phrase_hue(self.fire_hue, self.pulse.hue, dt, 1.2);
        }

        let radius = self.emitter_radius();
        let bounds = room_bounds(self.box_scale, self.box_xy_scale);
        for puff in &mut self.puffs {
            // The smoke room's emission follows smokeLifespan (the "smoke");
            // fireLifespan only sets the colour decay, like the JS's
            // uDissipation / uCooling split.
            advance_room_puff(
                puff,
                dt,
                self.simulation_speed,
                self.smoke_lifespan,
                radius,
                bounds,
            );
        }
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let bloom_strength = self.bloom_strength();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        // Glass first (transparent, geometry pass with depth), then the lava
        // sphere, then the additive haze.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-smoke-room-scene"),
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
            pass.set_pipeline(&gpu.glass_pipeline);
            pass.set_bind_group(0, &gpu.glass_bind_group, &[]);
            gpu.glass_mesh.draw(&mut pass);

            pass.set_pipeline(&gpu.emitter_pipeline);
            pass.set_bind_group(0, &gpu.emitter_bind_group, &[]);
            gpu.emitter_mesh.draw(&mut pass);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-smoke-room-haze"),
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
                label: Some("point10-smoke-room-final"),
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
        scene_props_with_levers(SMOKE_ROOM_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 Smoke Room", "3d-scenes")
    }
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

fn create_final_bind_group(device: &wgpu::Device, gpu: &SmokeRoomGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-smoke-room-final-bind-group"),
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

fn glass_wgsl() -> String {
    GLASS_WGSL.to_string()
}

fn emitter_wgsl() -> String {
    emitter_program()
}

fn puff_wgsl() -> String {
    PUFF_WGSL.to_string()
}

fn final_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{FINAL_WGSL}")
}

fn emitter_program() -> String {
    format!(
        "{}\n{EMITTER_WGSL}",
        crate::point10::shared::p9_glsl::P9_COMMON_WGSL
    )
}

const GLASS_WGSL: &str = r#"
struct GlassUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  fire: vec4<f32>,
  // x opacity, y time, z bass
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: GlassUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) world: vec3<f32>,
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
  out.world = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.world);
  let fresnel = pow(1.0 - clamp(abs(dot(normal, view_dir)), 0.0, 1.0), 3.0);
  // `MeshStandardMaterial({ color: 0x9fc4e8, envMapIntensity: 2.5 })` against
  // the dark studio: the port keeps the tint, the vertical env gradient and the
  // fire light playing across the panes.
  let tint = vec3<f32>(0.35, 0.55, 0.82);
  let env = mix(vec3<f32>(0.01, 0.015, 0.03), vec3<f32>(0.24, 0.32, 0.52), normal.y * 0.5 + 0.5);
  let to_fire = -in.world;
  let fire_distance = length(to_fire);
  let fire_dir = to_fire / max(fire_distance, 0.001);
  let fire_term = max(dot(normal, fire_dir), 0.0) * u.fire.a / (1.0 + fire_distance * fire_distance);
  let colour = tint * env * 2.5 + u.fire.rgb * fire_term + tint * fresnel * 0.4;
  return vec4<f32>(colour, u.params.x + fresnel * 0.12);
}
"#;

const EMITTER_WGSL: &str = r#"
struct EmitterUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // rgb = fire colour, w = glow scale
  glow: vec4<f32>,
  // x time, y flame height, z flicker
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: EmitterUniforms;

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
  out.normal = normalize(normal);
  out.local = position;
  out.world = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // `positionLocal.mul(0.5)` + the three-octave mx_noise flow (`:1036-1044`).
  let p = in.local * 0.5;
  let flow = vec3<f32>(0.0, -u.params.x, 0.0);
  let n1 = p9_noise(p + flow) * 0.5 + 0.5;
  let p2 = p * 2.0 - flow * 1.5;
  let n2 = p9_noise(p2 + vec3<f32>(n1 * 0.4)) * 0.5 + 0.5;
  let p3 = p * 4.0 + flow * 2.5;
  let n3 = p9_noise(p3) * 0.5 + 0.5;
  let noise = n1 * 0.5 + n2 * 0.35 + n3 * 0.15;
  let lava_t = clamp(pow(noise, 2.5), 0.0, 1.0);
  // The CPU already applied the fire ramp and the hue at `lava_t + 0.1`; the
  // noise keeps the lava's mottling, the glow scale carries the brightness.
  let emissive = u.glow.rgb * u.glow.w * (0.55 + lava_t * 0.9) * u.params.z;
  // A dark base under the emissive (`color: 0x000000`, roughness 1).
  return vec4<f32>(emissive, 1.0);
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
  // SmokeRoom's output node: `alpha = max(rgb)`.
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
    fn room_extents_and_emitter_follow_the_box_scale() {
        assert_eq!((GLASS_HALF_X, GLASS_HALF_Y, GLASS_HALF_Z), (6.3, 6.3, 12.3));
        assert_eq!(room_bounds(1.0, 1.0), [6.3, 6.3, 12.3]);
        let scaled = room_bounds(2.5, 1.6);
        close(scaled[0], 6.3 * 2.5 * 1.6, 1e-5);
        close(scaled[1], 6.3 * 2.5 * 1.6, 1e-5);
        close(scaled[2], 12.3 * 2.5, 1e-5);
        // The emitter radius stays on boxScale alone (`:632`).
        close(emitter_radius(2.0), SPHERE_RADIUS * 2.0, 1e-6);
        close(emitter_radius(f32::NAN), SPHERE_RADIUS, 1e-6);
    }

    #[test]
    fn glass_fit_is_bottom_anchored_to_the_output_top() {
        let top = room_top(CAMERA_START, 1.0, 1.0);
        assert!(top > GLASS_HALF_Y);
        let (height, centre_y) = glass_fit(top, 1.0, 1.0);
        close(centre_y - height * 0.5, -GLASS_HALF_Y, 1e-5);
        close(centre_y + height * 0.5, top, 1e-5);
    }

    #[test]
    fn the_fire_size_ramps_on_hats_and_snares() {
        assert_eq!(fire_size(0.0, 0.0), 1.0);
        close(fire_size(0.1, 0.0), 1.0 + 0.1 * FIRE_HAT_K, 1e-6);
        close(fire_size(0.0, 0.1), 1.0 + 0.1 * FIRE_SNARE_K, 1e-6);
        assert!(fire_size(0.1, 0.1) > 1.8);
    }

    #[test]
    fn bands_use_the_js_bin_ranges() {
        let mut freq = [0.0_f32; AUDIO_SAMPLES];
        for (index, value) in freq.iter_mut().enumerate().take(BASS_BINS) {
            *value = 0.5 + index as f32 * 0.1;
        }
        close(
            smoke_bass(&freq),
            (0..BASS_BINS).map(|i| 0.5 + i as f32 * 0.1).sum::<f32>() / BASS_BINS as f32,
            1e-6,
        );
        // Snare: bins 2%..13%; hi-hat: the top 15%.
        let snare_lo = (AUDIO_SAMPLES as f32 * 0.02) as usize;
        freq[snare_lo] = 1.0;
        assert!(snare_band(&freq) > 0.0);
        freq[AUDIO_SAMPLES - 1] = 1.0;
        assert!(hi_band(&freq) > 0.0);
    }

    #[test]
    fn emitter_glow_scales_with_temperature_density_and_fire_intensity() {
        let base = emitter_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, 10.0);
        assert!(base > 0.0 && base < 1.5, "glow {base}");
        // The fade-in is smoothstep(t / 3).
        assert_eq!(
            emitter_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, 0.0),
            0.0
        );
        assert!(emitter_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY * 2.0, 10.0) > base);
        assert!(emitter_glow(EMIT_TEMPERATURE, EMIT_DENSITY * 2.0, FIRE_INTENSITY, 10.0) > base);
    }

    #[test]
    fn puffs_spawn_on_the_sphere_and_stay_inside_the_room() {
        let mut puff = spawn_room_puff(7, SPHERE_RADIUS);
        assert_eq!(puff.age, 0.0);
        let radius =
            (puff.position[0].powi(2) + puff.position[1].powi(2) + puff.position[2].powi(2)).sqrt();
        assert!((radius - SPHERE_RADIUS).abs() < 1e-4, "radius {radius}");

        let bounds = room_bounds(1.0, 1.0);
        for _ in 0..2000 {
            advance_room_puff(&mut puff, 1.0 / 60.0, 1.2, 3.5, SPHERE_RADIUS, bounds);
            for axis in 0..3 {
                assert!(
                    puff.position[axis].abs() <= bounds[axis] + 1e-4,
                    "puff escaped on axis {axis}: {}",
                    puff.position[axis]
                );
            }
        }
    }

    #[test]
    fn dead_puffs_respawn_at_the_emitter() {
        let mut puff = spawn_room_puff(3, SPHERE_RADIUS);
        puff.age = 1.0;
        advance_room_puff(
            &mut puff,
            0.1,
            1.2,
            3.5,
            SPHERE_RADIUS,
            room_bounds(1.0, 1.0),
        );
        assert_eq!(puff.age, 0.0);
        let radius =
            (puff.position[0].powi(2) + puff.position[1].powi(2) + puff.position[2].powi(2)).sqrt();
        assert!((radius - SPHERE_RADIUS).abs() < 1e-4);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!((BLOOM_BASE, BLOOM_PER_BASS, BLOOM_MAX), (0.12, 2.2, 2.8));
        assert_eq!(
            (EMIT_DENSITY, EMIT_TEMPERATURE, FIRE_INTENSITY),
            (28.0, 22.0, 40.0)
        );
        assert_eq!((FIRE_HAT_K, FIRE_SNARE_K), (2.5, 8.0));
        assert_eq!(crate::point10::scenes::dvd_room::GLASS_TOP_MARGIN, 0.75);
        assert_eq!(crate::point10::scenes::dvd_room::VOLUME_WORLD_SIZE_X, 12.0);
        assert_eq!(crate::point10::scenes::dvd_room::VOLUME_WORLD_SIZE_Y, 12.0);
        assert_eq!(crate::point10::scenes::dvd_room::VOLUME_WORLD_SIZE_Z, 24.0);
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(GLASS_OPACITY, 0.35);
        assert_eq!(CAMERA_START, [14.0, 5.5, 4.4]);
        assert_eq!(FIRE_START_HEX, 0xffe68c);
        assert_eq!(FIRE_MID_HEX, 0xff7305);
        assert_eq!(FIRE_END_HEX, 0xff0000);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<GlassUniforms>(), 176);
        assert_eq!(std::mem::size_of::<EmitterUniforms>(), 176);
        assert_eq!(std::mem::size_of::<PuffUniforms>(), 96);
        assert_eq!(std::mem::size_of::<PuffInstance>(), 32);
        assert_eq!(std::mem::size_of::<FinalUniforms>(), 16);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("glass", glass_wgsl()),
            ("emitter", emitter_wgsl()),
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
        assert!(final_wgsl().contains("clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0)"));
        assert!(glass_wgsl().contains("u.params.x + fresnel * 0.12"));
        let emitter = emitter_wgsl();
        for needle in [
            "p9_noise(p + flow)",
            "p9_noise(p2 + vec3<f32>(n1 * 0.4))",
            "pow(noise, 2.5)",
        ] {
            assert!(emitter.contains(needle), "emitter lost {needle}");
        }
        assert!(puff_wgsl().contains("pow(1.0 - smoothstep(0.0, 1.0, radius), 2.0)"));
    }

    #[test]
    fn props_and_meta_match_the_js_registry() {
        let scene = SmokeRoom::new();
        assert_eq!(scene.meta().name, "Point10 Smoke Room");
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
                "boxScale",
                "boxXYScale",
            ]
        );
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn update_maps_the_audio_and_props() {
        use crate::point10::audio_bridge::BandPayload;
        let mut scene = SmokeRoom::new();
        scene.apply_props(|key| match key {
            "fireIntensity" => Some(80.0),
            "boxScale" => Some(2.0),
            "boxXYScale" => Some(1.5),
            "smokeDensity" => Some(56.0),
            _ => None,
        });
        assert_eq!(scene.fire_intensity, 80.0);
        assert_eq!(scene.box_scale, 2.0);
        assert_eq!(scene.box_xy_scale, 1.5);

        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag = [0.5; AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());

        scene.update(1.0 / 60.0, &bridge);
        assert!(scene.bass() > 0.0);
        assert!(scene.fire_size() > 1.0);
    }
}
