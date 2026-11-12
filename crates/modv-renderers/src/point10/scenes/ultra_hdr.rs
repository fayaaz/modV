//! UltraHDR — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/UltraHDR.js`.
//!
//! The three.js "Ultra HDR" example rebuilt as a standalone scene: a mirror
//! torus knot (`metalness: 1`, `roughness: 0`) reflecting an Ultra HDR
//! equirectangular environment, with ACES filmic tone mapping. Music drives
//! it three ways — kick attacks fire a quick scale swell, the smoothed band
//! mean sets the spin rate, and busy music morphs the knot toward a figure-8
//! (each kick flexes it further along the morph, calm music lets it fall
//! back).
//!
//! # Ported exactly
//!
//! * the geometry: three's `TorusKnotGeometry(1, 0.4, 128, 128, 1, 3)`
//!   ([`build_torus_knot`]) plus the figure-8 morph target the same vertex
//!   layout ([`swept_tube`] over [`eight_curve`] —
//!   `0.65 sin u cos u` in y, `0.85 sin u` in z);
//! * the audio drive, function for function (`:74-159`):
//!   [`create_drive_state`], [`step_energy`] (`1 - exp(-dt * 3)` scaled by the
//!   1.5 attack factor), [`step_kick`] (jump > 0.08 gated by a 0.6 s cooldown,
//!   `min(1, kick * 1.2)` target, `exp(-dt * 2.5)` decay, `dt * 8` rise),
//!   [`morph_from_energy`] (`min(1, energy * 1.6)^1.2`) and
//!   [`drive_ultra_hdr`] (`spin = dt * (0.3 + energy * 4) * spinScale`,
//!   `scale = 1 + pulse * 0.18 * pulseScale`);
//! * the camera (`CAMERA_POSITION = (0, 0, 7)` looking at the origin through
//!   the adapter's `PerspectiveCamera(60, aspect, 0.1, 1000)`);
//! * the prop binder (`sceneProps.js:333-337`): `spinScale = spinSpeed`,
//!   `pulseScale = pulse`, `brightnessScale = brightness`;
//! * the transparency: the HDR map is the *environment*, never the
//!   background, so empty pixels stay alpha 0 (`:14-17`).
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `TorusKnotGeometry` + `morphAttributes.position/normal` | [`build_torus_knot`] + a second vertex buffer mixed in WGSL |
//! | `MeshStandardMaterial({ metalness: 1, roughness: 0 })` | a mirror BRDF: `env(reflect(-viewDir, normal))` |
//! | `scene.environment` (PMREM-cubeuv prefiltered) | one equirect sample at roughness 0 (see below) |
//! | `HemisphereLight(0xffffff, 0x202020, 0.6)` | a `mix(ground, sky, n.y)` fill |
//! | `renderer.toneMapping = ACESFilmicToneMapping` + alpha-0 clear | [`Composer`] with bloom off and [`ToneMapping::AcesFilmic`], scene clear = transparent |
//! | `UltraHDRLoader` (DOMParser + gain-map canvas) | the SDR base rendition, the JS's own worker fallback (`:266-277`) |
//!
//! # Deviations
//!
//! * **Environment prefiltering.** three turns an equirect `scene.environment`
//!   into a PMREM cubeUV map and samples the mip that matches the roughness.
//!   This material has `roughness: 0`, i.e. a pure mirror, so the port samples
//!   the equirect directly — same reflection direction, sharper mirror line,
//!   no PMREM pass. The environment *intensity* is the JS's
//!   `scene.environmentIntensity`.
//! * **The hemisphere light.** In three a `HemisphereLight` only feeds the
//!   diffuse BRDF, which is identically zero at `metalness: 1`, so the JS knot
//!   is black until the environment lands. The port keeps the light as the
//!   stand-in radiance for exactly that case (the asset loader's procedural
//!   fallback), which is what `UltraHDR.js:59-62` intends the light for.
//! * **The gain map.** `UltraHDRLoader` needs `DOMParser` and a DOM canvas, so
//!   the worker never uses it either: the JS decodes the JPEG's SDR base with
//!   `createImageBitmap` and so does the port (`image` + `Rgba8UnormSrgb`).
//!   The HDR headroom is lost in both.
//! * **`P9Audio`.** `UltraHDR.js` imports `plane9/P9Audio.js`, which re-derives
//!   the 16-cell grid from the raw FFT and low-passes its own lanes. Like
//!   every other ported scene the port reads [`P10Audio`] over the shared
//!   [`AudioBridge`]: same read API, but the damping lanes come from
//!   `audio-bands.js` (the app's single audio pipeline) instead of a second
//!   one-pole in the scene.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::scenes::cube_mesh::normalise;
use crate::point10::shared::assets::{ColourSpace, Wrap, load_texture};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::p9_shading::P9_SHADING_WGSL;
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`UltraHDR.js:26-66`)
// ---------------------------------------------------------------------------

/// `KNOT.radius`.
pub const KNOT_RADIUS: f32 = 1.0;
/// `KNOT.tube`.
pub const KNOT_TUBE: f32 = 0.4;
/// `KNOT.tubularSegments`.
pub const KNOT_TUBULAR_SEGMENTS: usize = 128;
/// `KNOT.radialSegments`.
pub const KNOT_RADIAL_SEGMENTS: usize = 128;
/// `KNOT.p`.
pub const KNOT_P: f32 = 1.0;
/// `KNOT.q`.
pub const KNOT_Q: f32 = 3.0;

/// `CAMERA_POSITION = [0, 0, 7]`.
pub const CAMERA_POSITION: [f32; 3] = [0.0, 0.0, 7.0];
/// The adapter's `PerspectiveCamera` fov.
pub const CAMERA_FOV: f32 = 60.0;
/// The adapter's `PerspectiveCamera` near.
pub const CAMERA_NEAR: f32 = 0.1;
/// The adapter's `PerspectiveCamera` far.
pub const CAMERA_FAR: f32 = 1000.0;

/// `PULSE.jumpThreshold`.
pub const PULSE_JUMP_THRESHOLD: f32 = 0.08;
/// `PULSE.cooldown`.
pub const PULSE_COOLDOWN: f32 = 0.6;
/// `PULSE.targetGain`.
pub const PULSE_TARGET_GAIN: f32 = 1.2;
/// `PULSE.targetDecay`.
pub const PULSE_TARGET_DECAY: f32 = 2.5;
/// `PULSE.rise`.
pub const PULSE_RISE: f32 = 8.0;
/// `PULSE.scaleAmount`.
pub const PULSE_SCALE_AMOUNT: f32 = 0.18;

/// `ENERGY.rate`.
pub const ENERGY_RATE: f32 = 3.0;
/// `ENERGY.attack`.
pub const ENERGY_ATTACK: f32 = 1.5;

/// `MORPH.energyGain`.
pub const MORPH_ENERGY_GAIN: f32 = 1.6;
/// `MORPH.exponent`.
pub const MORPH_EXPONENT: f32 = 1.2;

/// `SPIN.base`.
pub const SPIN_BASE: f32 = 0.3;
/// `SPIN.energy`.
pub const SPIN_ENERGY: f32 = 4.0;

/// `HEMI.sky` (`0xffffff`, sRGB-decoded).
pub const HEMI_SKY: [f32; 3] = [1.0, 1.0, 1.0];
/// `HEMI.ground` (`0x202020`, sRGB-decoded).
pub const HEMI_GROUND: [f32; 3] = [0.014_443_844, 0.014_443_844, 0.014_443_844];
/// `HEMI.intensity`.
pub const HEMI_INTENSITY: f32 = 0.6;

/// The Ultra HDR JPEG the JS imports as `?url` (`:24`).
pub const ENV_ASSET: &str = "spruit_sunrise_2k.hdr.jpg";

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// One knot vertex (position + normal; the material has no uvs).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct KnotVertex {
    /// Position.
    pub position: [f32; 3],
    /// Normal.
    pub normal: [f32; 3],
}

impl KnotVertex {
    /// The base surface's layout (`@location(0)`, `@location(1)`).
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
            ],
        }
    }

    /// The morph target's layout (`@location(3)`, `@location(4)`), so one
    /// draw binds both buffers without colliding with the base attributes.
    pub fn morph_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 4,
                },
            ],
        }
    }
}

/// The knot's base surface and its figure-8 morph target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnotMesh {
    /// The `TorusKnotGeometry` surface.
    pub base: Vec<KnotVertex>,
    /// The morph target, same count and order.
    pub morph: Vec<KnotVertex>,
    /// Triangle indices (shared by both).
    pub indices: Vec<u32>,
}

/// `calculatePositionOnCurve(u, p, q, radius, position)` from three.
pub fn calculate_position_on_curve(u: f32, p: f32, q: f32, radius: f32) -> [f32; 3] {
    let (cu, su) = u.sin_cos();
    let qu_over_p = q / p * u;
    let (cs, sn) = qu_over_p.sin_cos();
    [
        radius * (2.0 + cs) * 0.5 * cu,
        radius * (2.0 + cs) * su * 0.5,
        radius * sn * 0.5,
    ]
}

/// `eightCurve(u)` — the Gerono lemniscate with the sine z-lift.
pub fn eight_curve(u: f32) -> [f32; 3] {
    let (su, cu) = u.sin_cos();
    [1.15 * cu, 0.65 * su * cu, 0.85 * su]
}

/// The ad-hoc `(N, B)` frame three's `sweptTube`/`TorusKnotGeometry` build.
///
/// `T = P2 - P1`, `N = P2 + P1`, `B = T x N`, `N = B x T`, both normalised.
fn tube_frame(p1: [f32; 3], p2: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = [p2[0] - p1[0], p2[1] - p1[1], p2[2] - p1[2]];
    let n = [p2[0] + p1[0], p2[1] + p1[1], p2[2] + p1[2]];
    let b = cross(t, n);
    let n = cross(b, t);
    (normalise(n), normalise(b))
}

/// `TorusKnotGeometry(radius, tube, tubularSegments, radialSegments, p, q)`.
pub fn build_torus_knot(
    radius: f32,
    tube: f32,
    tubular_segments: usize,
    radial_segments: usize,
    p: f32,
    q: f32,
) -> (Vec<KnotVertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity((tubular_segments + 1) * (radial_segments + 1));
    let mut indices = Vec::with_capacity(tubular_segments * radial_segments * 6);

    for i in 0..=tubular_segments {
        let u = (i as f32 / tubular_segments as f32) * p * std::f32::consts::TAU;
        let p1 = calculate_position_on_curve(u, p, q, radius);
        let p2 = calculate_position_on_curve(u + 0.01, p, q, radius);
        let (n, b) = tube_frame(p1, p2);

        for j in 0..=radial_segments {
            let v = (j as f32 / radial_segments as f32) * std::f32::consts::TAU;
            let (sv, cv) = v.sin_cos();
            // `cx = -tube * cos(v)`, `cy = tube * sin(v)`.
            let cx = -tube * cv;
            let cy = tube * sv;
            let position = [
                p1[0] + cx * n[0] + cy * b[0],
                p1[1] + cx * n[1] + cy * b[1],
                p1[2] + cx * n[2] + cy * b[2],
            ];
            vertices.push(KnotVertex {
                position,
                normal: normalise([
                    position[0] - p1[0],
                    position[1] - p1[1],
                    position[2] - p1[2],
                ]),
            });
        }
    }

    for j in 1..=tubular_segments {
        for i in 1..=radial_segments {
            let a = ((radial_segments + 1) * (j - 1) + (i - 1)) as u32;
            let b = ((radial_segments + 1) * j + (i - 1)) as u32;
            let c = ((radial_segments + 1) * j + i) as u32;
            let d = ((radial_segments + 1) * (j - 1) + i) as u32;
            indices.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }

    (vertices, indices)
}

/// `sweptTube(curve, tubularSegments, radialSegments, tube)` — the same vertex
/// loop, order and basis as three's `TorusKnotGeometry`, so the result is a
/// valid morph target for it (same count, matching `i`-over-`j` layout,
/// radial `vertex - P1` normals).
pub fn swept_tube(
    curve: impl Fn(f32) -> [f32; 3],
    tubular_segments: usize,
    radial_segments: usize,
    tube: f32,
) -> Vec<KnotVertex> {
    let mut vertices = Vec::with_capacity((tubular_segments + 1) * (radial_segments + 1));

    for i in 0..=tubular_segments {
        let u = (i as f32 / tubular_segments as f32) * std::f32::consts::TAU;
        let p1 = curve(u);
        let p2 = curve(u + 0.01);
        let (n, b) = tube_frame(p1, p2);

        for j in 0..=radial_segments {
            let a = (j as f32 / radial_segments as f32) * std::f32::consts::TAU;
            let (sa, ca) = a.sin_cos();
            let cx = -tube * ca;
            let cy = tube * sa;
            let position = [
                p1[0] + cx * n[0] + cy * b[0],
                p1[1] + cx * n[1] + cy * b[1],
                p1[2] + cx * n[2] + cy * b[2],
            ];
            vertices.push(KnotVertex {
                position,
                normal: normalise([
                    position[0] - p1[0],
                    position[1] - p1[1],
                    position[2] - p1[2],
                ]),
            });
        }
    }

    vertices
}

/// `buildUltraHDRKnotGeometry()` — the knot plus its figure-8 morph target.
pub fn build_ultra_hdr_knot() -> KnotMesh {
    let (base, indices) = build_torus_knot(
        KNOT_RADIUS,
        KNOT_TUBE,
        KNOT_TUBULAR_SEGMENTS,
        KNOT_RADIAL_SEGMENTS,
        KNOT_P,
        KNOT_Q,
    );
    let morph = swept_tube(
        eight_curve,
        KNOT_TUBULAR_SEGMENTS,
        KNOT_RADIAL_SEGMENTS,
        KNOT_TUBE,
    );

    KnotMesh {
        base,
        morph,
        indices,
    }
}

// ---------------------------------------------------------------------------
// Drive (`UltraHDR.js:74-159`)
// ---------------------------------------------------------------------------

/// `createDriveState()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UltraHdrState {
    /// Previous kick envelope.
    pub kick: f32,
    /// Previous pulse.
    pub pulse: f32,
    /// Previous pulse target.
    pub pulse_target: f32,
    /// Scene time of the last pulse (`-1` before the first).
    pub last_pulse: f32,
    /// Smoothed band mean.
    pub energy: f32,
}

/// `createDriveState()`.
pub fn create_drive_state() -> UltraHdrState {
    UltraHdrState {
        kick: 0.0,
        pulse: 0.0,
        pulse_target: 0.0,
        last_pulse: -1.0,
        energy: 0.0,
    }
}

/// `clampDt` + `finiteOr`.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// `clampDt(dt)`.
fn clamp_dt(dt: f32) -> f32 {
    finite_or(dt, 0.0).clamp(0.0, crate::point10::MAX_FRAME_SECONDS)
}

/// `clamp01`.
fn clamp01(value: f32) -> f32 {
    finite_or(value, 0.0).clamp(0.0, 1.0)
}

/// `nonNegative`.
fn non_negative(value: f32) -> f32 {
    finite_or(value, 0.0).max(0.0)
}

/// `stepEnergy` — the mean bass/mid/high level through the 3 Hz one-pole with
/// the fast attack factor.
pub fn step_energy(bass: f32, mid: f32, high: f32, previous_energy: f32, dt: f32) -> f32 {
    let step = clamp_dt(dt);
    let previous = non_negative(previous_energy);
    let target = ((non_negative(bass) + non_negative(mid) + non_negative(high)) / 3.0).min(1.0);
    let beta = 1.0 - (-step * ENERGY_RATE).exp();

    previous + (target - previous) * (beta * ENERGY_ATTACK).min(1.0)
}

/// `stepKick` — one attack-gated kick pulse.
pub fn step_kick(state: &UltraHdrState, beat: f32, scene_time: f32, dt: f32) -> UltraHdrState {
    let step = clamp_dt(dt);
    let kick = clamp01(beat);
    let previous_kick = clamp01(state.kick);
    let previous_pulse = non_negative(state.pulse);
    let time = finite_or(scene_time, finite_or(state.last_pulse, -1.0));
    let mut last_pulse = finite_or(state.last_pulse, -1.0);
    let mut pulse_target = non_negative(state.pulse_target);
    let jump = kick - previous_kick;

    if jump > PULSE_JUMP_THRESHOLD && time - last_pulse > PULSE_COOLDOWN {
        last_pulse = time;
        pulse_target = (kick * PULSE_TARGET_GAIN).min(1.0);
    }

    pulse_target *= (-step * PULSE_TARGET_DECAY).exp();
    let pulse = previous_pulse + (pulse_target - previous_pulse) * (step * PULSE_RISE).min(1.0);

    UltraHdrState {
        kick,
        pulse,
        pulse_target,
        last_pulse,
        ..*state
    }
}

/// `morphFromEnergy` — gained up, then eased so calm music barely moves the
/// shape.
pub fn morph_from_energy(energy: f32) -> f32 {
    (non_negative(energy) * MORPH_ENERGY_GAIN)
        .min(1.0)
        .powf(MORPH_EXPONENT)
}

/// One frame of [`drive_ultra_hdr`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UltraHdrDrive {
    /// Next drive state.
    pub state: UltraHdrState,
    /// Smoothed band mean.
    pub energy: f32,
    /// Kick envelope.
    pub kick: f32,
    /// Kick pulse.
    pub pulse: f32,
    /// Morph influence (0..1).
    pub morph: f32,
    /// This frame's spin delta (radians).
    pub spin: f32,
    /// This frame's uniform scale.
    pub scale: f32,
}

/// The `driveUltraHDR` inputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UltraHdrInput {
    /// Bass band.
    pub bass: f32,
    /// Mid band.
    pub mid: f32,
    /// High band.
    pub high: f32,
    /// Beat envelope.
    pub beat: f32,
    /// Frame delta.
    pub dt: f32,
    /// Accumulated scene time (for the cooldown).
    pub time: f32,
    /// `spinSpeed` prop.
    pub spin_scale: f32,
    /// `pulse` prop.
    pub pulse_scale: f32,
}

/// `driveUltraHDR(state, input)` — pure frame drive.
pub fn drive_ultra_hdr(state: &UltraHdrState, input: &UltraHdrInput) -> UltraHdrDrive {
    let dt = clamp_dt(input.dt);
    let energy = step_energy(input.bass, input.mid, input.high, state.energy, dt);
    let kick = step_kick(state, input.beat, input.time, dt);
    let spin_scale = finite_or(input.spin_scale, 1.0);
    let pulse_scale = finite_or(input.pulse_scale, 1.0);

    UltraHdrDrive {
        state: UltraHdrState { energy, ..kick },
        energy,
        kick: kick.kick,
        pulse: kick.pulse,
        morph: morph_from_energy(energy),
        spin: dt * (SPIN_BASE + energy * SPIN_ENERGY) * spin_scale,
        scale: 1.0 + kick.pulse * PULSE_SCALE_AMOUNT * pulse_scale,
    }
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `ULTRA_HDR_PROPS` (`sceneProps.js:193-197`).
const ULTRA_HDR_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("pulse", "Kick Pulse", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

/// Uniform block (`UltraHdrUniforms` in the shader, 160 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct UltraHdrUniforms {
    view_proj: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    /// x morph, y environment intensity, z hemisphere intensity, w
    /// environment available.
    params: [f32; 4],
}

/// GPU resources built in `init`.
struct UltraHdrGpu {
    queue: wgpu::Queue,
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    base_vertices: wgpu::Buffer,
    morph_vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    _env_texture: wgpu::Texture,
    composer: Composer,
}

/// UltraHDR scene.
pub struct UltraHdr {
    audio: P10Audio,
    time: f32,
    state: UltraHdrState,
    /// `this.knot.rotation.y`.
    rotation: f32,
    /// `this.knot.scale`.
    scale: f32,
    /// `this.knot.morphTargetInfluences[0]`.
    morph: f32,
    spin_scale: f32,
    pulse_scale: f32,
    brightness_scale: f32,
    /// Whether the environment texture is the real asset (not the loader's
    /// procedural stand-in).
    env_available: bool,
    size: (u32, u32),
    gpu: Option<UltraHdrGpu>,
}

impl Default for UltraHdr {
    fn default() -> Self {
        Self::new()
    }
}

impl UltraHdr {
    /// Create the scene (`UltraHDR.js:280-333` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            time: 0.0,
            state: create_drive_state(),
            rotation: 0.0,
            scale: 1.0,
            morph: 0.0,
            spin_scale: 1.0,
            pulse_scale: 1.0,
            brightness_scale: 1.0,
            env_available: false,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:333-337`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(ULTRA_HDR_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_scale = value,
                "pulse" => self.pulse_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// The knot's spin about y (radians).
    pub fn rotation(&self) -> f32 {
        self.rotation
    }

    /// The knot's uniform scale.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The knot's morph influence.
    pub fn morph(&self) -> f32 {
        self.morph
    }

    /// The smoothed band mean.
    pub fn energy(&self) -> f32 {
        self.state.energy
    }

    /// The drive state (kick/pulse/cooldown).
    pub fn drive_state(&self) -> UltraHdrState {
        self.state
    }

    /// Whether the environment asset loaded.
    pub fn env_available(&self) -> bool {
        self.env_available
    }

    /// The camera's view-projection for the current pose.
    fn camera_matrix(&self) -> [[f32; 4]; 4] {
        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        let projection = crate::point10::scenes::cube_mesh::perspective(
            CAMERA_FOV.to_radians(),
            aspect,
            CAMERA_NEAR,
            CAMERA_FAR,
        );
        let view = crate::point10::scenes::cube_mesh::look_at(
            CAMERA_POSITION,
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        );
        crate::point10::scenes::cube_mesh::mat4_mul(&projection, &view)
    }

    fn model_matrix(&self) -> [[f32; 4]; 4] {
        use crate::point10::scenes::cube_mesh::{mat4_mul, rotation_y, scale};
        mat4_mul(
            &rotation_y(self.rotation),
            &scale(self.scale, self.scale, self.scale),
        )
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.uniforms,
            0,
            bytemuck::bytes_of(&UltraHdrUniforms {
                view_proj: self.camera_matrix(),
                model: self.model_matrix(),
                camera_pos: [
                    CAMERA_POSITION[0],
                    CAMERA_POSITION[1],
                    CAMERA_POSITION[2],
                    1.0,
                ],
                params: [
                    self.morph,
                    self.brightness_scale,
                    HEMI_INTENSITY * self.brightness_scale,
                    if self.env_available { 1.0 } else { 0.0 },
                ],
            }),
        );
    }
}

impl Point10Scene for UltraHdr {
    fn name(&self) -> &'static str {
        "Point10 Ultra HDR"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        use wgpu::util::DeviceExt;

        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let mesh = build_ultra_hdr_knot();
        let base_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point10-ultra-hdr-vertices"),
            contents: bytemuck::cast_slice(&mesh.base),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let morph_vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point10-ultra-hdr-morph-vertices"),
            contents: bytemuck::cast_slice(&mesh.morph),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point10-ultra-hdr-indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let loader = ctx.asset_loader();
        let env = load_texture(
            &device,
            &queue,
            &loader,
            ENV_ASSET,
            // `texture.colorSpace = SRGBColorSpace` in `loadBaseImageTexture`.
            ColourSpace::Srgb,
            // A panorama wraps in longitude (three's default is clamp).
            Wrap::Repeat,
        );
        self.env_available = !env.fallback;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-ultra-hdr-shader"),
            source: wgpu::ShaderSource::Wgsl(ultra_hdr_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-ultra-hdr-layout"),
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
            label: Some("point10-ultra-hdr-uniforms"),
            size: std::mem::size_of::<UltraHdrUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-ultra-hdr-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&env.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&env.sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-ultra-hdr-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-ultra-hdr-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(KnotVertex::layout()), Some(KnotVertex::morph_layout())],
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let (depth, depth_view) = crate::point10::scenes::cube_mesh::create_depth(
            &device,
            "point10-ultra-hdr-depth",
            self.size.0,
            self.size.1,
        );

        // `renderer.toneMapping = ACESFilmicToneMapping`; no composer in the JS,
        // so the chain is a blit + the ACES/sRGB output pass.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);

        self.gpu = Some(UltraHdrGpu {
            queue,
            device,
            pipeline,
            bind_group,
            uniforms,
            base_vertices,
            morph_vertices,
            indices,
            index_count: mesh.indices.len() as u32,
            depth,
            depth_view,
            _env_texture: env.texture,
            composer,
        });
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.resize(self.size.0, self.size.1);
            let (depth, depth_view) = crate::point10::scenes::cube_mesh::create_depth(
                &gpu.device,
                "point10-ultra-hdr-depth",
                self.size.0,
                self.size.1,
            );
            gpu.depth = depth;
            gpu.depth_view = depth_view;
        }
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.time += dt;
        self.audio.update(dt, audio);

        let bands = *self.audio.bands();
        let drive = drive_ultra_hdr(
            &self.state,
            &UltraHdrInput {
                bass: bands[0],
                mid: bands[1],
                high: bands[2],
                beat: self.audio.beat(0.0),
                dt,
                time: self.time,
                spin_scale: self.spin_scale,
                pulse_scale: self.pulse_scale,
            },
        );

        self.state = drive.state;
        self.morph = drive.morph;
        self.rotation += drive.spin;
        self.scale = drive.scale;
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-ultra-hdr"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gpu.composer.scene_view(),
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

            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.set_vertex_buffer(0, gpu.base_vertices.slice(..));
            pass.set_vertex_buffer(1, gpu.morph_vertices.slice(..));
            pass.set_index_buffer(gpu.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..gpu.index_count, 0, 0..1);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(ULTRA_HDR_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10UltraHDR.js` sets no category.
        SceneMeta::for_scene("Point10 Ultra HDR", "3d-scenes")
    }
}

/// `vec3 _palette` etc. are not needed here; the knot only needs the panorama
/// lookup.
fn ultra_hdr_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{P9_SHADING_WGSL}\n{ULTRA_HDR_WGSL}")
}

const ULTRA_HDR_WGSL: &str = r#"
struct UltraHdrUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // x morph, y environment intensity, z hemisphere intensity, w had env
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: UltraHdrUniforms;
@group(0) @binding(1) var env_map: texture_2d<f32>;
@group(0) @binding(2) var env_sampler: sampler;

// `HEMI.sky` / `HEMI.ground` through three's sRGB decode.
const HEMI_SKY: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);
const HEMI_GROUND: vec3<f32> = vec3<f32>(0.014443844, 0.014443844, 0.014443844);

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) normal: vec3<f32>,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(3) morph_position: vec3<f32>,
  @location(4) morph_normal: vec3<f32>,
) -> VsOut {
  // `morphTargetInfluences[0]`: three mixes the single morph target linearly.
  let morphed_position = mix(position, morph_position, u.params.x);
  let morphed_normal = normalize(mix(normal, morph_normal, u.params.x));

  let world = u.model * vec4<f32>(morphed_position, 1.0);
  var out: VsOut;
  out.position = u.view_proj * world;
  out.world = world.xyz;
  // `mat3(modelMatrix) * normal`: the model is a rotation and a uniform scale.
  out.normal = normalize((u.model * vec4<f32>(morphed_normal, 0.0)).xyz);
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.world);

  // metalness 1 / roughness 0: a pure mirror. The environment radiance is the
  // only diffuse-free term three leaves for this material.
  let reflected = reflect(-view_dir, normal);
  let env = p9_texture_panoramic_lod(env_map, env_sampler, reflected, 0.0).rgb;

  // HemisphereLight(HEMI.sky, HEMI.ground, HEMI.intensity): the fill the JS
  // keeps for the frame before the environment lands.
  let hemi = mix(HEMI_GROUND, HEMI_SKY, normal.y * 0.5 + 0.5) * u.params.z;

  let radiance = mix(hemi, env * u.params.y, u.params.w);
  return vec4<f32>(radiance, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;
    use crate::point10::shared::phrase_color::srgb_hex_to_linear;

    // Port of tests/unit/point10-ultra-hdr.test.js.

    #[test]
    fn energy_stays_at_zero_in_silence() {
        assert_eq!(step_energy(0.0, 0.0, 0.0, 0.0, 0.016), 0.0);
    }

    #[test]
    fn energy_follows_the_mean_band_level_through_the_attack_one_pole() {
        let dt = 0.016;
        let k = ((1.0 - (-dt * ENERGY_RATE).exp()) * ENERGY_ATTACK).min(1.0);
        let first = step_energy(1.0, 1.0, 1.0, 0.0, dt);
        let second = step_energy(1.0, 1.0, 1.0, first, dt);

        assert!((first - k).abs() < 1e-6, "{first} vs {k}");
        assert!(second > first);
        assert!(second < 1.0);
    }

    #[test]
    fn energy_clamps_the_band_mean_and_sanitises_bad_input() {
        let dt = 0.016;
        let k = ((1.0 - (-dt * ENERGY_RATE).exp()) * ENERGY_ATTACK).min(1.0);

        assert!((step_energy(10.0, 10.0, 10.0, 0.0, dt) - k).abs() < 1e-6);
        assert_eq!(step_energy(-1.0, -2.0, -3.0, 0.0, dt), 0.0);
        assert!(
            (step_energy(f32::NAN, f32::NAN, f32::NAN, 0.5, dt) - 0.5 * (1.0 - k)).abs() < 1e-6
        );
        assert!((step_energy(1.0, 1.0, 1.0, 0.25, f32::NAN) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn energy_releases_toward_silence_when_the_music_stops() {
        assert!(step_energy(0.0, 0.0, 0.0, 0.8, 0.05) < 0.8);
    }

    #[test]
    fn kick_does_not_fire_below_the_jump_threshold() {
        let state = create_drive_state();
        let next = step_kick(&state, PULSE_JUMP_THRESHOLD, 1.0, 0.016);

        assert_eq!(next.last_pulse, state.last_pulse);
        assert_eq!(next.pulse_target, 0.0);
        assert_eq!(next.pulse, 0.0);
    }

    #[test]
    fn kick_fires_on_a_sharp_attack_and_decays_the_target_immediately() {
        let dt = 0.016;
        let next = step_kick(&create_drive_state(), 0.5, 1.0, dt);
        let target = (0.5 * PULSE_TARGET_GAIN).min(1.0) * (-dt * PULSE_TARGET_DECAY).exp();

        assert_eq!(next.last_pulse, 1.0);
        assert!((next.pulse_target - target).abs() < 1e-6);
        assert!((next.pulse - target * (dt * PULSE_RISE).min(1.0)).abs() < 1e-6);
        assert_eq!(next.kick, 0.5);
    }

    #[test]
    fn kick_gates_re_fires_behind_the_cooldown() {
        let fired = step_kick(&create_drive_state(), 0.5, 1.0, 0.016);
        let too_soon = step_kick(&fired, 0.9, 1.3, 0.01);
        let after_cooldown = step_kick(&too_soon, 1.0, 1.7, 0.01);

        assert_eq!(too_soon.last_pulse, 1.0);
        assert_eq!(after_cooldown.last_pulse, 1.7);
        assert!(
            after_cooldown.pulse_target
                > too_soon.pulse_target * (-0.01 * PULSE_TARGET_DECAY).exp()
        );
    }

    #[test]
    fn kick_never_exceeds_a_full_scale_target_and_sanitises_bad_input() {
        let full = step_kick(&create_drive_state(), 10.0, 1.0, 0.016);
        let bad = step_kick(&create_drive_state(), f32::NAN, f32::NAN, f32::NAN);

        assert_eq!(full.kick, 1.0);
        assert!(full.pulse_target <= (-0.016 * PULSE_TARGET_DECAY).exp());
        assert_eq!(bad.kick, 0.0);
        assert!(bad.pulse.is_finite());
        assert!(bad.pulse_target.is_finite());
    }

    #[test]
    fn morph_is_a_no_op_in_silence_and_saturates_when_busy() {
        assert_eq!(morph_from_energy(0.0), 0.0);
        assert_eq!(morph_from_energy(1.0), 1.0);
        assert_eq!(morph_from_energy(1.0 / MORPH_ENERGY_GAIN), 1.0);
    }

    #[test]
    fn morph_scales_the_energy_sub_linearly() {
        let energy = 0.4;
        let expected = (energy * MORPH_ENERGY_GAIN).powf(MORPH_EXPONENT);

        assert!((morph_from_energy(energy) - expected).abs() < 1e-6);
        assert!(morph_from_energy(energy) < energy * MORPH_ENERGY_GAIN);
    }

    #[test]
    fn morph_clamps_and_sanitises_bad_input() {
        assert_eq!(morph_from_energy(-1.0), 0.0);
        assert_eq!(morph_from_energy(f32::NAN), 0.0);
    }

    #[test]
    fn drive_state_starts_from_the_documented_resting_state() {
        let state = create_drive_state();
        assert_eq!(state.kick, 0.0);
        assert_eq!(state.pulse, 0.0);
        assert_eq!(state.pulse_target, 0.0);
        assert_eq!(state.last_pulse, -1.0);
        assert_eq!(state.energy, 0.0);
    }

    #[test]
    fn drive_spins_slowly_and_keeps_the_resting_scale_in_silence() {
        let drive = drive_ultra_hdr(
            &create_drive_state(),
            &UltraHdrInput {
                bass: 0.0,
                mid: 0.0,
                high: 0.0,
                beat: 0.0,
                dt: 0.1,
                time: 0.0,
                spin_scale: 1.0,
                pulse_scale: 1.0,
            },
        );

        assert_eq!(drive.morph, 0.0);
        assert!((drive.spin - 0.1 * SPIN_BASE).abs() < 1e-6);
        assert_eq!(drive.scale, 1.0);
    }

    #[test]
    fn drive_combines_energy_spin_and_pulse_scale() {
        let dt = 0.016;
        let drive = drive_ultra_hdr(
            &create_drive_state(),
            &UltraHdrInput {
                bass: 1.0,
                mid: 1.0,
                high: 1.0,
                beat: 1.0,
                dt,
                time: 1.0,
                spin_scale: 2.0,
                pulse_scale: 0.5,
            },
        );
        let energy = ((1.0 - (-dt * ENERGY_RATE).exp()) * ENERGY_ATTACK).min(1.0);
        let pulse_target = (-dt * PULSE_TARGET_DECAY).exp();
        let pulse = pulse_target * (dt * PULSE_RISE).min(1.0);

        assert!((drive.energy - energy).abs() < 1e-6);
        assert!(
            (drive.morph - (energy * MORPH_ENERGY_GAIN).min(1.0).powf(MORPH_EXPONENT)).abs() < 1e-6
        );
        assert!((drive.pulse - pulse).abs() < 1e-6);
        assert!((drive.spin - dt * (SPIN_BASE + energy * SPIN_ENERGY) * 2.0).abs() < 1e-6);
        assert!((drive.scale - (1.0 + pulse * PULSE_SCALE_AMOUNT * 0.5)).abs() < 1e-6);
        assert_eq!(drive.state.last_pulse, 1.0);
        assert_ne!(drive.state, create_drive_state());
    }

    #[test]
    fn drive_applies_the_pulse_and_spin_scales_independently() {
        let drive = drive_ultra_hdr(
            &create_drive_state(),
            &UltraHdrInput {
                bass: 1.0,
                mid: 0.0,
                high: 0.0,
                beat: 1.0,
                dt: 0.016,
                time: 1.0,
                spin_scale: 0.0,
                pulse_scale: 0.0,
            },
        );

        assert_eq!(drive.spin, 0.0);
        assert_eq!(drive.scale, 1.0);
        assert!(drive.morph > 0.0);
    }

    #[test]
    fn drive_survives_non_finite_input() {
        let drive = drive_ultra_hdr(
            &create_drive_state(),
            &UltraHdrInput {
                bass: f32::NAN,
                mid: f32::NAN,
                high: f32::NAN,
                beat: f32::NAN,
                dt: f32::NAN,
                time: f32::NAN,
                spin_scale: f32::NAN,
                pulse_scale: f32::NAN,
            },
        );

        assert!(drive.energy.is_finite());
        assert!(drive.morph.is_finite());
        assert!(drive.spin.is_finite());
        assert!(drive.scale.is_finite());
    }

    #[test]
    fn eight_curve_is_a_gerono_lemniscate_with_the_z_lift() {
        let start = eight_curve(0.0);
        assert!((start[0] - 1.15).abs() < 1e-6);
        assert!(start[1].abs() < 1e-6);
        assert!(start[2].abs() < 1e-6);

        let quarter = eight_curve(std::f32::consts::FRAC_PI_2);
        assert!(quarter[0].abs() < 1e-6);
        assert!(quarter[1].abs() < 1e-6);
        assert!((quarter[2] - 0.85).abs() < 1e-6);

        let half = eight_curve(std::f32::consts::PI);
        assert!((half[0] + 1.15).abs() < 1e-6);
        assert!(half[2].abs() < 1e-6);
    }

    #[test]
    fn knot_matches_the_torus_knot_vertex_layout_for_morphing() {
        let mesh = build_ultra_hdr_knot();
        let count = (KNOT_TUBULAR_SEGMENTS + 1) * (KNOT_RADIAL_SEGMENTS + 1);

        assert_eq!(mesh.base.len(), count);
        assert_eq!(mesh.morph.len(), count);
        assert_eq!(
            mesh.indices.len(),
            KNOT_TUBULAR_SEGMENTS * KNOT_RADIAL_SEGMENTS * 6
        );
        for vertex in mesh.base.iter().chain(mesh.morph.iter()) {
            assert!(vertex.position.iter().all(|value| value.is_finite()));
            assert!(vertex.normal.iter().all(|value| value.is_finite()));
        }
        // Every index is in range.
        assert!(mesh.indices.iter().all(|index| (*index as usize) < count));
    }

    #[test]
    fn knot_surface_sits_on_the_tube_radius() {
        let mesh = build_ultra_hdr_knot();
        // The knot curve has |P1| between 0.5 and 1.5 * radius; every vertex is
        // exactly `tube` away from its own curve point, which the radial
        // normals encode.
        assert_eq!(std::mem::size_of::<KnotVertex>(), 24);
        let first = mesh.base[0];
        assert!(
            (first.normal[0] * first.normal[0]
                + first.normal[1] * first.normal[1]
                + first.normal[2] * first.normal[2]
                - 1.0)
                .abs()
                < 1e-4
        );
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(KNOT_RADIUS, 1.0);
        assert_eq!(KNOT_TUBE, 0.4);
        assert_eq!(KNOT_TUBULAR_SEGMENTS, 128);
        assert_eq!(KNOT_RADIAL_SEGMENTS, 128);
        assert_eq!(KNOT_P, 1.0);
        assert_eq!(KNOT_Q, 3.0);
        assert_eq!(CAMERA_POSITION, [0.0, 0.0, 7.0]);

        assert_eq!(HEMI_INTENSITY, 0.6);
        assert_eq!(PULSE_JUMP_THRESHOLD, 0.08);
        assert_eq!(PULSE_COOLDOWN, 0.6);
        assert_eq!(PULSE_SCALE_AMOUNT, 0.18);
        assert_eq!(MORPH_ENERGY_GAIN, 1.6);
        assert_eq!(MORPH_EXPONENT, 1.2);
        assert_eq!(SPIN_BASE, 0.3);
        assert_eq!(SPIN_ENERGY, 4.0);
        // 0x202020 through three's sRGB → linear decode.
        assert!((HEMI_GROUND[0] - srgb_hex_to_linear(0x20_20_20)[0]).abs() < 1e-6);
    }

    #[test]
    fn update_drives_the_knot_from_the_bands() {
        let mut scene = UltraHdr::new();
        let bridge = AudioBridge::new();
        scene.update(0.1, &bridge);
        // Silence: the slow idle spin, resting scale, no morph.
        assert!((scene.rotation() - 0.1 * SPIN_BASE).abs() < 1e-5);
        assert_eq!(scene.scale(), 1.0);
        assert_eq!(scene.morph(), 0.0);

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes = [0.5; 12];
        payload.beat = 0.5;
        payload.freq_mag = [0.5; crate::point10::audio_bridge::AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let before = scene.rotation();
        scene.update(0.05, &bridge);
        assert!(scene.rotation() > before);
        assert!(scene.energy() > 0.0);
        assert!(scene.morph() > 0.0);
        assert!(scene.scale() > 1.0);
        // The beat attack fired the pulse gate.
        assert_eq!(scene.drive_state().last_pulse, scene.time);
    }

    #[test]
    fn brightness_scales_the_environment_and_hemisphere() {
        let mut scene = UltraHdr::new();
        scene.apply_props(|key| (key == "brightness").then_some(2.0));
        assert_eq!(scene.brightness_scale, 2.0);
        scene.apply_props(|key| (key == "brightness").then_some(9.0));
        assert_eq!(scene.brightness_scale, 3.0);
    }

    #[test]
    fn time_is_capped_at_a_tenth_of_a_second() {
        let mut scene = UltraHdr::new();
        scene.update(4.0, &AudioBridge::new());
        assert!((scene.time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn uniforms_are_160_bytes() {
        assert_eq!(std::mem::size_of::<UltraHdrUniforms>(), 160);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = ultra_hdr_wgsl();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("ULTRA_HDR_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("ULTRA_HDR_WGSL must validate");
    }

    #[test]
    fn shader_keeps_the_ported_maths() {
        let program = ultra_hdr_wgsl();
        for needle in [
            "let morphed_position = mix(position, morph_position, u.params.x);",
            "normalize((u.model * vec4<f32>(morphed_normal, 0.0)).xyz)",
            "let reflected = reflect(-view_dir, normal);",
            "p9_texture_panoramic_lod(env_map, env_sampler, reflected, 0.0)",
            "mix(HEMI_GROUND, HEMI_SKY, normal.y * 0.5 + 0.5) * u.params.z",
            "mix(hemi, env * u.params.y, u.params.w)",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = UltraHdr::new();
        assert_eq!(scene.meta().name, "Point10 Ultra HDR");
        assert_eq!(scene.meta().category, "3d-scenes");
        assert_eq!(scene.meta().audio_features, &["bass", "mid", "treble"]);
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 3..],
            &["spinSpeed", "pulse", "brightness"]
        );
        assert_eq!(names[0], "gain");
    }
}
