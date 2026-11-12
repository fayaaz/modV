//! Around Plus — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/AroundPlus.js`.
//!
//! The Plane9 "Around And Around" preset (Cell) as a standalone scene: one
//! inverted torus (the tube the camera sits inside), textured with ambientCG
//! Concrete044C, eight glow rings every 45°, water in the tube's bottom eighth,
//! and the preset's camera wander (a MinMax pitch plus a noise roll on top of a
//! fixed lookAt). The torus spins with the mid band; the water is driven by the
//! WaveSim height field the kick attacks disturb.
//!
//! # Ported exactly
//!
//! * [`build_around_torus`] — `AroundPlus.js:102-161` verbatim: `R = 5`,
//!   `r = 1`, 256x45 segments, tube phase +90°, `rotateX(π/2)` (hole up), the
//!   `uv` rescale `(1 - v) * 12.64, (1 - u) * 33.19` and the InvertMesh winding
//!   flip, so the camera inside the tube sees the wall;
//! * the torus shader (`:221-252`): concrete albedo `* abs(length(position) -
//!   2.7)`, the `_noise(v_tex * 0.2)` large-scale variation, the Reinhard soft
//!   knee on `gIn2.x * 6 / (gIn2.x + 5)` mixing into `pow(1 -
//!   dot(abs(normal), (0,1,0)), 8)`, and `col *= x * x` with `x =
//!   dot(abs(normal), normalize((1,0,1)))`;
//! * the ring rig (`:305-321`): `TorusGeometry(1.06, 0.07, 12, 96)` at radius
//!   5 every 45°, `rotation.y = -a`, parented to the torus, colour
//!   `equalizedHue(0xffe68c * 1.8, hue)` tightened to the phrase hue at
//!   `min(1, dt * 1.2)` and scaled by `ringBrightness`;
//! * the water rig (`:332-372`): `RingGeometry(WATER_INNER, WATER_OUTER, 128, 4)`
//!   rotated into XZ at `y = -0.75`, `alpha 0.6`, `distortionScale`, the
//!   height-field gradient injected into `surfaceNormal` exactly as the
//!   `onBeforeCompile` patch does (`waveSlope(worldPosition.xz)`), and
//!   [`water_uniforms`] (`:49-64`);
//! * the camera (`:474-499`): fov 60.5, near 0.1, far 1000, position
//!   `(-5, 0, 0)`, `lookAt(-5, 0, -1)`, then `rotateX(pitch)`,
//!   `rotateZ(roll)` with `roll = valueNoise(1, time * 0.01) * 5` degrees;
//! * Expression1 (`:424-431`): `perm += min(1, band(-1,1,1) * 10 + 0.1) * dt *
//!   20 * spinSpeed`, `torus.rotation.y = perm * DEG`;
//! * Shader1 (`:417-423`): `gIn2.x = band(-1, 1, 0) * 50`;
//! * the kick drops (`:454-467`): a bass **attack** above 0.06 re-arms and
//!   drops once, `strength = min(0.05, jump * 0.07)`;
//! * `EffectComposer`: RenderPass → UnrealBloomPass `(0.9, 0.4, 1.0)` →
//!   OutputPass (sRGB, no tone mapping).
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `buildAroundTorus()` `BufferGeometry` | [`build_around_torus`] (same vertices, normals, uvs and indices) |
//! | `TorusGeometry` rings | [`build_torus`] |
//! | `Water` (mirror render target + projected reflection) | a fresnel/sky/specular approximation sampling the WaveSim gradient + `waternormals.jpg` |
//! | `THREE.TextureLoader` | [`assets`] (`concrete044c_col_1k.jpg` sRGB repeat, `waternormals.jpg` linear repeat) |
//! | `MeshBasicMaterial` rings | flat-colour pipeline |
//! | `EffectComposer` | [`Composer`] with the same bloom and output pass |
//! | `valueNoise` (`plane9/P9Expression.js`) | [`value_noise1`] |
//! | `makePitchWander` (`Math.random`) | [`PitchWander`] fed by `seeded_rand` |
//!
//! # Deviations
//!
//! * **Water reflections.** `Water.js` renders the scene into a mirror target
//!   with a reflected camera and samples it with a projective coordinate. The
//!   Rust port has no planar-reflection pass; the water samples a procedural
//!   sky/horizon tinted by the ring colour instead, keeps the same fresnel
//!   (`rf0 = 0.3`), the same `noise.xzy * (1.5, 1.0, 1.5)` normal map read, the
//!   same `distortionScale` and the same wave-slope injection, so the motion
//!   (waves propagate, reflections churn, the waterline swells) is preserved
//!   while the mirror image itself is not.
//! * **Culling.** The JS relies on the InvertMesh winding; the port still flips
//!   the winding but draws double-sided (`cull_mode: None`), so a winding slip
//!   can never hide the wall.
//! * `texture.anisotropy = maxAniso` is not reproduced (the sampler keeps wgpu's
//!   default anisotropy — see [`assets`]).
//! * `Math.random()` in the drop scheduler and the pitch wander is replaced by
//!   [`seeded_rand`](crate::point10::shared::random::seeded_rand) with a frame
//!   counter, so a run is reproducible; the distribution is the same.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::shared::assets::{ColourSpace, Wrap, load_texture};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::phrase_color::{
    DropColorState, drop_color_pulse, ease_phrase_hue, equalized_hue, srgb_hex_to_linear,
};
use crate::point10::shared::random::seeded_rand;
use crate::point10::shared::water_wave::{WAVE_RES, WaveSim, create_wave_texture, wave_sampler};
use crate::point10::{
    HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

/// `AroundPlus.js: export const WATER_LEVEL = -0.75`.
pub const WATER_LEVEL: f32 = -0.75;
/// `WATER_INNER = 5 - sqrt(1 - level²)`.
pub const WATER_INNER: f32 = 4.338_562_3;
/// `WATER_OUTER = 5 + sqrt(1 - level²)`.
pub const WATER_OUTER: f32 = 5.661_437_5;
/// `AroundPlus.js: const WAVE_RES = 256`.
pub const WAVE_RESOLUTION: u32 = WAVE_RES;

/// `WATER_DRIVE` (`:49-55`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterDrive {
    /// Bass churn rate.
    pub speed: f32,
    /// Extra churn at full bass.
    pub churn: f32,
    /// Base distortion scale.
    pub distortion: f32,
    /// Extra distortion at full bass.
    pub shake: f32,
    /// Waterline swell at full bass.
    pub swell: f32,
}

/// `WATER_DRIVE` exactly.
pub const WATER_DRIVE: WaterDrive = WaterDrive {
    speed: 0.8,
    churn: 2.0,
    distortion: 2.0,
    shake: 6.0,
    swell: 0.1,
};

/// One frame of `waterUniforms(bass, smooth, dt)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterUniforms {
    /// The smoothed bass.
    pub smooth: f32,
    /// `dt * (speed + smooth * churn)`.
    pub dt_time: f32,
    /// `distortion + smooth * shake`.
    pub distortion_scale: f32,
    /// `WATER_LEVEL + min(swell, smooth * 0.3)`.
    pub level: f32,
}

/// `AroundPlus.js:56-64`.
pub fn water_uniforms(bass: f32, smooth: f32, dt: f32) -> WaterUniforms {
    let bass = if bass.is_finite() { bass } else { 0.0 };
    let smooth = if smooth.is_finite() { smooth } else { 0.0 };
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let s = smooth + (bass - smooth) * (dt * 10.0).min(1.0);

    WaterUniforms {
        smooth: s,
        dt_time: dt * (WATER_DRIVE.speed + s * WATER_DRIVE.churn),
        distortion_scale: WATER_DRIVE.distortion + s * WATER_DRIVE.shake,
        level: WATER_LEVEL + WATER_DRIVE.swell.min(s * 0.3),
    }
}

/// `annulusUv(x, z)` — world point on the water annulus → sim space.
pub fn annulus_uv(x: f32, z: f32) -> [f32; 2] {
    let u = z.atan2(x) / (std::f32::consts::PI * 2.0);
    [
        u - u.floor(),
        ((x * x + z * z).sqrt() - WATER_INNER) / (WATER_OUTER - WATER_INNER),
    ]
}

/// `valueNoise(1, p)` from `plane9/P9Expression.js`.
pub fn value_noise1(p: f32) -> f32 {
    let f = p - p.floor();
    let i = p.floor();
    let a = seeded_rand(i as i32, 1);
    let b = seeded_rand(i as i32 + 1, 1);
    let s = f * f * (3.0 - 2.0 * f);
    (a + (b - a) * s) * 2.0 - 1.0
}

/// `makePitchWander(min, max, itMin, itMax)` — the MinMax1 port.
///
/// A smoothed random wander between `min` and `max`, re-rolled every 3..20 s,
/// integrated so the value lands exactly when the interval ends. The JS reads
/// `Math.random()`; [`PitchWander::next`] takes the sample as an argument.
#[derive(Clone, Copy, Debug)]
pub struct PitchWander {
    min: f32,
    max: f32,
    it_min: f32,
    it_max: f32,
    t: f32,
    dur: f32,
    dir: f32,
    val: Option<f32>,
    last: Option<f32>,
}

impl PitchWander {
    /// `makePitchWander(-10, 10)`.
    pub fn new(min: f32, max: f32, it_min: f32, it_max: f32) -> Self {
        Self {
            min,
            max,
            it_min,
            it_max,
            t: 0.0,
            dur: 0.0,
            dir: 1.0,
            val: None,
            last: None,
        }
    }

    /// Advance to `now` (`rand` is the `[0,1)` sample for a fresh interval).
    pub fn next(&mut self, now: f32, rand: f32) -> f32 {
        let now = if now.is_finite() { now } else { 0.0 };
        let rand = if rand.is_finite() {
            rand.clamp(0.0, 1.0)
        } else {
            0.0
        };

        if self.val.is_none() {
            self.val = Some(self.min);
            self.last = Some(now);
            self.dur = self.it_min + rand * (self.it_max - self.it_min).max(0.0);
            self.t = now + self.dur;
        }

        let previous = self.last.unwrap_or(now);
        let ddt = (now - previous).clamp(0.0, 0.05);
        self.last = Some(now);

        if now >= self.t {
            self.dir = -self.dir;
            self.dur = self.it_min + rand * (self.it_max - self.it_min).max(0.0);
            self.t = now + self.dur;
        }

        let target = if self.dir > 0.0 { self.max } else { self.min };
        let remain = (self.t - now).max(0.001);
        let value = self.val.unwrap_or(self.min);
        let next = (value + ((target - value) / remain) * ddt).clamp(self.min, self.max);
        self.val = Some(next);
        next
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// One interleaved mesh vertex (`position`, `normal`, `uv`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Local position.
    pub position: [f32; 3],
    /// Local normal.
    pub normal: [f32; 3],
    /// Texture coordinate.
    pub uv: [f32; 2],
}

impl Vertex {
    /// Buffer layout (`@location(0..2)`).
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
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 24,
                    shader_location: 2,
                },
            ],
        }
    }
}

/// CPU-side mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// Vertices.
    pub vertices: Vec<Vertex>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

/// `buildAroundTorus()` — the preset's `Torus1` with the P9Nodes treatment.
pub fn build_around_torus() -> Mesh {
    const R: f32 = 5.0;
    const R_TUBE: f32 = 1.0;
    const SUB_AXIS: usize = 256;
    const SUB_HEIGHT: usize = 45;
    let arc = std::f32::consts::TAU;
    let phase = std::f32::consts::FRAC_PI_2;

    let mut mesh = Mesh::default();
    mesh.vertices.reserve((SUB_AXIS + 1) * (SUB_HEIGHT + 1));

    for i in 0..=SUB_HEIGHT {
        let v = (i as f32 / SUB_HEIGHT as f32) * std::f32::consts::TAU + phase;
        let (cv, sv) = (v.cos(), v.sin());
        let ring_r = R + R_TUBE * cv;
        for j in 0..=SUB_AXIS {
            let a = (j as f32 / SUB_AXIS as f32) * arc;
            let (ca, sa) = (a.cos(), a.sin());
            mesh.vertices.push(Vertex {
                position: [ring_r * ca, ring_r * sa, R_TUBE * sv],
                normal: [cv * ca, cv * sa, sv],
                uv: [j as f32 / SUB_AXIS as f32, i as f32 / SUB_HEIGHT as f32],
            });
        }
    }

    for j in 1..=SUB_AXIS {
        for i in 1..=SUB_HEIGHT {
            let a = (i * (SUB_AXIS + 1) + (j - 1)) as u32;
            let b = ((i - 1) * (SUB_AXIS + 1) + (j - 1)) as u32;
            let c = ((i - 1) * (SUB_AXIS + 1) + j) as u32;
            let d = (i * (SUB_AXIS + 1) + j) as u32;
            mesh.indices.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }

    // `g.rotateX(Math.PI / 2)`: (x, y, z) -> (x, -z, y).
    for vertex in &mut mesh.vertices {
        let [x, y, z] = vertex.position;
        vertex.position = [x, -z, y];
        let [nx, ny, nz] = vertex.normal;
        vertex.normal = [nx, -nz, ny];
    }

    // The rescaled uvs: `(1 - v) * su`, `(1 - u) * sv`.
    let (su, sv) = (12.64_f32, 33.19_f32);
    for vertex in &mut mesh.vertices {
        vertex.uv = [(1.0 - vertex.uv[1]) * su, (1.0 - vertex.uv[0]) * sv];
    }

    // The InvertMesh winding flip: swap the first and third index of every
    // triangle.
    for triangle in mesh.indices.chunks_exact_mut(3) {
        triangle.swap(0, 2);
    }

    mesh
}

/// `TorusGeometry(radius, tube, radialSegments, tubularSegments)` from three.
pub fn build_torus(
    radius: f32,
    tube: f32,
    radial_segments: usize,
    tubular_segments: usize,
) -> Mesh {
    let mut mesh = Mesh::default();
    for j in 0..=radial_segments {
        for i in 0..=tubular_segments {
            let u = (i as f32 / tubular_segments as f32) * std::f32::consts::TAU;
            let v = (j as f32 / radial_segments as f32) * std::f32::consts::TAU;
            let centre_x = radius * u.cos();
            let centre_y = radius * u.sin();
            let vertex_x = (radius + tube * v.cos()) * u.cos();
            let vertex_y = (radius + tube * v.cos()) * u.sin();
            let vertex_z = tube * v.sin();
            let normal = [vertex_x - centre_x, vertex_y - centre_y, vertex_z];
            let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
                .sqrt()
                .max(1e-6);
            mesh.vertices.push(Vertex {
                position: [vertex_x, vertex_y, vertex_z],
                normal: [normal[0] / length, normal[1] / length, normal[2] / length],
                uv: [
                    i as f32 / tubular_segments as f32,
                    j as f32 / radial_segments as f32,
                ],
            });
        }
    }

    for j in 1..=radial_segments {
        for i in 1..=tubular_segments {
            let a = ((tubular_segments + 1) * (j - 1) + (i - 1)) as u32;
            let b = ((tubular_segments + 1) * j + (i - 1)) as u32;
            let c = ((tubular_segments + 1) * j + i) as u32;
            let d = ((tubular_segments + 1) * (j - 1) + i) as u32;
            mesh.indices.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }

    mesh
}

/// `RingGeometry(inner, outer, 128, 4).rotateX(-π/2)` — the water surface.
pub fn build_water_annulus() -> Mesh {
    const THETA_SEGMENTS: usize = 128;
    const PHI_SEGMENTS: usize = 4;
    let mut mesh = Mesh::default();

    for phi in 0..=PHI_SEGMENTS {
        let radius = WATER_INNER + (WATER_OUTER - WATER_INNER) * phi as f32 / PHI_SEGMENTS as f32;
        for theta in 0..=THETA_SEGMENTS {
            let angle = theta as f32 / THETA_SEGMENTS as f32 * std::f32::consts::TAU;
            let (sin, cos) = angle.sin_cos();
            // RingGeometry lies in XY, rotateX(-π/2) maps (x, y, 0) -> (x, 0, -y).
            mesh.vertices.push(Vertex {
                position: [radius * cos, 0.0, -radius * sin],
                normal: [0.0, 1.0, 0.0],
                uv: [
                    0.5 + radius * cos / (WATER_OUTER * 2.0),
                    0.5 + radius * sin / (WATER_OUTER * 2.0),
                ],
            });
        }
    }

    let row = THETA_SEGMENTS + 1;
    for phi in 1..=PHI_SEGMENTS {
        for theta in 1..=THETA_SEGMENTS {
            let a = ((phi - 1) * row + (theta - 1)) as u32;
            let b = ((phi - 1) * row + theta) as u32;
            let c = (phi * row + theta) as u32;
            let d = (phi * row + (theta - 1)) as u32;
            mesh.indices.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }

    mesh
}

// ---------------------------------------------------------------------------
// Matrices (column-major, matching WGSL's `mat4x4<f32>`)
// ---------------------------------------------------------------------------

/// A column-major 4x4 matrix.
pub type Mat4 = [[f32; 4]; 4];

/// `a * b`.
pub fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0_f32; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            out[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    out
}

/// Translation.
pub fn translation(x: f32, y: f32, z: f32) -> Mat4 {
    let mut m = identity();
    m[3] = [x, y, z, 1.0];
    m
}

/// Identity.
pub fn identity() -> Mat4 {
    let mut m = [[0.0_f32; 4]; 4];
    for (index, column) in m.iter_mut().enumerate() {
        column[index] = 1.0;
    }
    m
}

/// Rotation about Y.
pub fn rotation_y(angle: f32) -> Mat4 {
    let (s, c) = angle.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Rotation about X.
pub fn rotation_x(angle: f32) -> Mat4 {
    let (s, c) = angle.sin_cos();
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, s, 0.0],
        [0.0, -s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Rotation about Z.
pub fn rotation_z(angle: f32) -> Mat4 {
    let (s, c) = angle.sin_cos();
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Transform a point by a column-major matrix, returning clip space.
pub fn transform_point(m: &Mat4, p: [f32; 3]) -> [f32; 4] {
    let mut out = [0.0_f32; 4];
    for (row, value) in out.iter_mut().enumerate() {
        *value = m[0][row] * p[0] + m[1][row] * p[1] + m[2][row] * p[2] + m[3][row];
    }
    out
}

/// Transpose of a rotation matrix (its inverse), translation column cleared.
pub fn transpose_rotation(m: &Mat4) -> Mat4 {
    let mut out = [[0.0_f32; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            out[column][row] = m[row][column];
        }
    }
    out
}

/// `PerspectiveCamera(fov, aspect, near, far).projectionMatrix`.
pub fn perspective(fov_y_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y_radians * 0.5).tan();
    let range = near - far;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (far + near) / range, -1.0],
        [0.0, 0.0, (2.0 * far * near) / range, 0.0],
    ]
}

/// `Matrix4.lookAt(eye, target, up)` (three's `view` matrix).
pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Mat4 {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let normalize = |v: [f32; 3]| {
        let length = dot(v, v).sqrt().max(1e-9);
        [v[0] / length, v[1] / length, v[2] / length]
    };

    let z = normalize(sub(eye, target));
    let x = normalize(cross(up, z));
    let y = cross(z, x);

    [
        [x[0], y[0], z[0], 0.0],
        [x[1], y[1], z[1], 0.0],
        [x[2], y[2], z[2], 0.0],
        [-dot(x, eye), -dot(y, eye), -dot(z, eye), 1.0],
    ]
}

// ---------------------------------------------------------------------------
// Uniform blocks
// ---------------------------------------------------------------------------

/// Torus uniforms (`AroundPlusUniforms` in the shader, 144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TorusUniforms {
    view_proj: Mat4,
    model: Mat4,
    in2: [f32; 4],
}

/// One ring's uniforms (144 bytes; slots are 256-byte aligned for the dynamic
/// offsets).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct RingUniforms {
    view_proj: Mat4,
    model: Mat4,
    color: [f32; 4],
}

/// Water uniforms (208 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct WaterMaterialUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    params: [f32; 4],
    water_color: [f32; 4],
    sun_direction: [f32; 4],
    ring_color: [f32; 4],
}

/// Dynamic uniform offset alignment (`wgpu::Limits::min_uniform_buffer_offset_alignment`).
const RING_SLOT: u64 = 256;
/// `AroundPlus.js:306`: `new THREE.TorusGeometry(1.06, 0.07, 12, 96)`.
pub const RING_RADIUS: f32 = 1.06;
/// Ring tube radius.
pub const RING_TUBE: f32 = 0.07;
/// Ring count (every 45°).
pub const RING_COUNT: usize = 8;
/// `AroundPlus.js:488-499` camera.
pub const CAMERA_FOV: f32 = 60.5;
/// Camera position.
pub const CAMERA_POSITION: [f32; 3] = [-5.0, 0.0, 0.0];

/// `AROUND_PLUS_PROPS` (`sceneProps.js:60-64`).
const AROUND_PLUS_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("ringBrightness", "Ring Brightness", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("waterChurn", "Water Churn", 1.0, 0.0, 3.0, 0.01),
];

/// `this._ringBase = new THREE.Color(0xffe68c).multiplyScalar(1.8)`.
fn ring_base_colour() -> [f32; 3] {
    let base = srgb_hex_to_linear(0xff_e6_8c);
    [base[0] * 1.8, base[1] * 1.8, base[2] * 1.8]
}

/// A mesh uploaded in `init`.
struct MeshGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

impl MeshGpu {
    fn new(device: &wgpu::Device, label: &str, mesh: &Mesh) -> Self {
        use wgpu::util::DeviceExt;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-vertices")),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-indices")),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
        }
    }

    fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// GPU resources built in `init`.
struct AroundPlusGpu {
    queue: wgpu::Queue,
    device: wgpu::Device,
    torus: MeshGpu,
    rings: MeshGpu,
    water: MeshGpu,
    torus_pipeline: wgpu::RenderPipeline,
    torus_bind_group: wgpu::BindGroup,
    torus_uniforms: wgpu::Buffer,
    ring_pipeline: wgpu::RenderPipeline,
    ring_bind_group: wgpu::BindGroup,
    ring_uniforms: wgpu::Buffer,
    water_pipeline: wgpu::RenderPipeline,
    water_bind_group: wgpu::BindGroup,
    water_uniforms: wgpu::Buffer,
    wave_texture: wgpu::Texture,
    _concrete: wgpu::Texture,
    _water_normals: wgpu::Texture,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    composer: Composer,
}

/// AroundPlus scene.
pub struct AroundPlus {
    audio: P10Audio,
    time: f32,
    /// `this.perm` — the torus spin in degrees.
    perm: f32,
    spin_speed: f32,
    ring_brightness: f32,
    water_churn: f32,
    /// `this._bassSm`.
    bass_sm: f32,
    /// `this._wkPrev`.
    wave_prev: f32,
    /// `this._wkArmed`.
    wave_armed: bool,
    /// `this._wave` — the CPU height field.
    sim: WaveSim,
    /// `this._pulse` — the ring phrase colour.
    phrase: DropColorState,
    /// `this._ringHue`.
    ring_hue: f32,
    /// `this._ringBase`.
    ring_base: [f32; 3],
    /// The equalised ring colour as last uploaded.
    ring_color: [f32; 3],
    /// `this._wander`.
    wander: PitchWander,
    /// `this.pitch` as last computed.
    pitch: f32,
    /// The water's animated clock (`water.material.uniforms.time`).
    water_time: f32,
    /// The waterline (`water.position.y`).
    water_level: f32,
    /// The distortion scale.
    distortion_scale: f32,
    frame: u32,
    size: (u32, u32),
    gpu: Option<AroundPlusGpu>,
}

impl Default for AroundPlus {
    fn default() -> Self {
        Self::new()
    }
}

impl AroundPlus {
    /// Create the scene (`AroundPlus.js:267-405` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            time: 0.0,
            perm: 1.0,
            spin_speed: 1.0,
            ring_brightness: 1.0,
            water_churn: 1.0,
            bass_sm: 0.0,
            wave_prev: 0.0,
            wave_armed: true,
            sim: WaveSim::new(WAVE_RESOLUTION),
            phrase: DropColorState::new(),
            ring_hue: 0.0,
            ring_base: ring_base_colour(),
            ring_color: equalized_hue(ring_base_colour(), 0.0),
            wander: PitchWander::new(-10.0, 10.0, 3.0, 20.0),
            pitch: 0.0,
            water_time: 0.0,
            water_level: WATER_LEVEL,
            distortion_scale: WATER_DRIVE.distortion,
            frame: 0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:343-347`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(AROUND_PLUS_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_speed = value,
                "ringBrightness" => self.ring_brightness = value,
                "waterChurn" => self.water_churn = value,
                _ => {}
            }
        }
    }

    /// The torus spin phase (`this.perm`, degrees).
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// The water field.
    pub fn wave(&self) -> &WaveSim {
        &self.sim
    }

    /// The equalised ring colour as last uploaded.
    pub fn ring_color(&self) -> [f32; 3] {
        self.ring_color
    }

    /// The camera pitch (degrees).
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// The water clock (`water.material.uniforms.time`).
    pub fn water_time(&self) -> f32 {
        self.water_time
    }

    /// The waterline.
    pub fn water_level(&self) -> f32 {
        self.water_level
    }

    /// The distortion scale.
    pub fn distortion_scale(&self) -> f32 {
        self.distortion_scale
    }

    /// The camera's view-projection for the current pose.
    fn camera_matrix(&self) -> Mat4 {
        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        let projection = perspective(CAMERA_FOV.to_radians(), aspect, 0.1, 1000.0);
        // `cam.lookAt(-5, 0, -1)` from `(-5, 0, 0)` with up +y is the identity
        // rotation (looking down -z), so the view is just the inverse of the
        // local rotateX(pitch) * rotateY(0) * rotateZ(roll) around the eye.
        let roll = value_noise1(self.time * 0.01) * 5.0;
        let rotation = mat4_mul(
            &rotation_x(self.pitch.to_radians()),
            &rotation_z(roll.to_radians()),
        );
        let inverse = mat4_mul(
            &transpose_rotation(&rotation),
            &translation(
                -CAMERA_POSITION[0],
                -CAMERA_POSITION[1],
                -CAMERA_POSITION[2],
            ),
        );
        mat4_mul(&projection, &inverse)
    }

    /// The torus model matrix (`torus.rotation.y = perm * DEG`).
    fn torus_matrix(&self) -> Mat4 {
        rotation_y(self.perm.to_radians())
    }

    fn ring_model(&self, index: usize) -> Mat4 {
        let a = (index as f32 / RING_COUNT as f32) * std::f32::consts::TAU;
        let torus = self.torus_matrix();
        let local = mat4_mul(
            &translation(a.cos() * 5.0, 0.0, a.sin() * 5.0),
            &rotation_y(-a),
        );
        mat4_mul(&torus, &local)
    }

    fn water_model(&self) -> Mat4 {
        mat4_mul(
            &self.torus_matrix(),
            &translation(0.0, self.water_level, 0.0),
        )
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let view_proj = self.camera_matrix();
        let torus_model = self.torus_matrix();
        let in2 = [self.audio.band(-1, 1, 0, 0.0) * 50.0, 0.0, 0.0, 0.0];

        gpu.queue.write_buffer(
            &gpu.torus_uniforms,
            0,
            bytemuck::bytes_of(&TorusUniforms {
                view_proj,
                model: torus_model,
                in2,
            }),
        );

        for index in 0..RING_COUNT {
            let offset = index as u64 * RING_SLOT;
            gpu.queue.write_buffer(
                &gpu.ring_uniforms,
                offset,
                bytemuck::bytes_of(&RingUniforms {
                    view_proj,
                    model: self.ring_model(index),
                    color: [
                        self.ring_color[0] * self.ring_brightness,
                        self.ring_color[1] * self.ring_brightness,
                        self.ring_color[2] * self.ring_brightness,
                        1.0,
                    ],
                }),
            );
        }

        gpu.queue.write_buffer(
            &gpu.water_uniforms,
            0,
            bytemuck::bytes_of(&WaterMaterialUniforms {
                view_proj,
                model: self.water_model(),
                camera_pos: [
                    CAMERA_POSITION[0],
                    CAMERA_POSITION[1],
                    CAMERA_POSITION[2],
                    1.0,
                ],
                params: [
                    self.water_time,
                    self.distortion_scale,
                    0.6, // Water.js `alpha: 0.6`
                    1.0 / WAVE_RESOLUTION as f32,
                ],
                // `waterColor: 0x001e0f` through three's sRGB decode.
                water_color: {
                    let c = srgb_hex_to_linear(0x00_1e_0f);
                    [c[0], c[1], c[2], 1.0]
                },
                // `sunDirection: new THREE.Vector3(0, 1, 0)`, shininess 100.
                sun_direction: [0.0, 1.0, 0.0, 100.0],
                ring_color: [
                    self.ring_color[0] * self.ring_brightness,
                    self.ring_color[1] * self.ring_brightness,
                    self.ring_color[2] * self.ring_brightness,
                    1.0,
                ],
            }),
        );
    }
}

impl Point10Scene for AroundPlus {
    fn name(&self) -> &'static str {
        "Point10 Around Plus"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let loader = ctx.asset_loader();
        let concrete = load_texture(
            &device,
            &queue,
            &loader,
            "concrete044c_col_1k.jpg",
            ColourSpace::Srgb,
            Wrap::Repeat,
        );
        let water_normals = load_texture(
            &device,
            &queue,
            &loader,
            "waternormals.jpg",
            ColourSpace::Linear,
            Wrap::Repeat,
        );

        // --- torus pipeline ------------------------------------------------
        let torus_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-around-plus-torus-shader"),
            source: wgpu::ShaderSource::Wgsl(around_torus_wgsl().into()),
        });
        let torus_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-around-plus-torus-layout"),
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
        let torus_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-around-plus-torus-uniforms"),
            size: std::mem::size_of::<TorusUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let torus_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-around-plus-torus-bind-group"),
            layout: &torus_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: torus_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&concrete.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&concrete.sampler),
                },
            ],
        });
        let torus_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-around-plus-torus-pipeline-layout"),
                bind_group_layouts: &[Some(&torus_layout)],
                immediate_size: 0,
            });
        let torus_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-around-plus-torus-pipeline"),
            layout: Some(&torus_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &torus_shader,
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
                module: &torus_shader,
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

        // --- ring pipeline -------------------------------------------------
        let ring_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-around-plus-ring-shader"),
            source: wgpu::ShaderSource::Wgsl(RING_WGSL.into()),
        });
        let ring_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-around-plus-ring-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<RingUniforms>() as u64
                    ),
                },
                count: None,
            }],
        });
        let ring_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-around-plus-ring-uniforms"),
            size: RING_SLOT * RING_COUNT as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ring_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-around-plus-ring-bind-group"),
            layout: &ring_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &ring_uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<RingUniforms>() as u64),
                }),
            }],
        });
        let ring_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-around-plus-ring-pipeline-layout"),
            bind_group_layouts: &[Some(&ring_layout)],
            immediate_size: 0,
        });
        let ring_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-around-plus-ring-pipeline"),
            layout: Some(&ring_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &ring_shader,
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
                module: &ring_shader,
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

        // --- water pipeline ------------------------------------------------
        let water_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-around-plus-water-shader"),
            source: wgpu::ShaderSource::Wgsl(water_wgsl().into()),
        });
        let water_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-around-plus-water-layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let wave_texture = create_wave_texture(&device, self.sim.res());
        let wave_view = wave_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let wave_sampler = wave_sampler(&device);
        let water_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-around-plus-water-uniforms"),
            size: std::mem::size_of::<WaterMaterialUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let water_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-around-plus-water-bind-group"),
            layout: &water_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: water_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&water_normals.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&water_normals.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&wave_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&wave_sampler),
                },
            ],
        });
        let water_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-around-plus-water-pipeline-layout"),
                bind_group_layouts: &[Some(&water_layout)],
                immediate_size: 0,
            });
        let water_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-around-plus-water-pipeline"),
            layout: Some(&water_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &water_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                // The water is transparent and must not occlude the rings.
                depth_write_enabled: Some(false),
                ..depth_state()
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &water_shader,
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

        // --- depth + scene targets ----------------------------------------
        let (depth, depth_view) = create_depth(&device, self.size.0, self.size.1);

        let torus = MeshGpu::new(&device, "point10-around-plus-torus", &build_around_torus());
        let rings = MeshGpu::new(
            &device,
            "point10-around-plus-ring",
            &build_torus(RING_RADIUS, RING_TUBE, 12, 96),
        );
        let water = MeshGpu::new(&device, "point10-around-plus-water", &build_water_annulus());

        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.enable_bloom(0.9, 0.4, 1.0);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        self.sim.upload(&queue, &wave_texture);
        self.gpu = Some(AroundPlusGpu {
            queue,
            device,
            torus,
            rings,
            water,
            torus_pipeline,
            torus_bind_group,
            torus_uniforms,
            ring_pipeline,
            ring_bind_group,
            ring_uniforms,
            water_pipeline,
            water_bind_group,
            water_uniforms,
            wave_texture,
            _concrete: concrete.texture,
            _water_normals: water_normals.texture,
            depth,
            depth_view,
            composer,
        });
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.resize(self.size.0, self.size.1);
            let (depth, depth_view) = create_depth(&gpu.device, self.size.0, self.size.1);
            gpu.depth = depth;
            gpu.depth_view = depth_view;
        }
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.time += dt;
        self.frame = self.frame.wrapping_add(1);
        self.audio.update(dt, audio);

        let bass = self.audio.bands()[0];
        let mid = self.audio.bands()[1];

        // Expression1: the mid band spins the tube.
        self.perm += (mid * 10.0 + 0.1).min(1.0) * dt * 20.0 * self.spin_speed;

        // Ring colour on musical phrases.
        let rand = seeded_rand(self.frame as i32, 23);
        drop_color_pulse(&mut self.phrase, bass, dt, rand);
        self.ring_hue = ease_phrase_hue(self.ring_hue, self.phrase.hue, dt, 1.2);
        self.ring_color = equalized_hue(self.ring_base, self.ring_hue);

        // Water: bass churn, distortion and the waterline swell.
        let drive = water_uniforms(bass, self.bass_sm, dt);
        self.bass_sm = drive.smooth;
        self.water_time += drive.dt_time * self.water_churn;
        self.distortion_scale = drive.distortion_scale;
        self.water_level = drive.level;

        // Kicks disturb the height field on the bass attack only.
        let jump = bass - self.wave_prev;
        if jump <= 0.0 {
            self.wave_armed = true;
        }
        if self.wave_armed && jump > 0.06 {
            let x = seeded_rand(self.frame as i32, 3);
            let y = 0.3 + seeded_rand(self.frame as i32, 5) * 0.4;
            self.sim.drop(x, y, (jump * 0.07).min(0.05), 0.04);
            self.wave_armed = false;
        }
        self.wave_prev = bass;
        self.sim.update(dt);

        // Camera: MinMax pitch + the noise roll (`camera_matrix`).
        self.pitch = self
            .wander
            .next(self.time, seeded_rand(self.frame as i32, 11));

        if let Some(gpu) = self.gpu.as_ref() {
            self.sim.upload(&gpu.queue, &gpu.wave_texture);
        }
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-around-plus"),
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

            pass.set_pipeline(&gpu.torus_pipeline);
            pass.set_bind_group(0, &gpu.torus_bind_group, &[]);
            gpu.torus.draw(&mut pass);

            pass.set_pipeline(&gpu.ring_pipeline);
            for index in 0..RING_COUNT {
                let offset = (index as u64 * RING_SLOT) as u32;
                pass.set_bind_group(0, &gpu.ring_bind_group, &[offset]);
                gpu.rings.draw(&mut pass);
            }

            pass.set_pipeline(&gpu.water_pipeline);
            pass.set_bind_group(0, &gpu.water_bind_group, &[]);
            gpu.water.draw(&mut pass);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(AROUND_PLUS_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10AroundPlus.js` sets no category.
        SceneMeta::for_scene("Point10 Around Plus", "3d-scenes")
    }
}

/// Depth attachment state shared by the three pipelines.
fn depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// Create the depth texture and view.
fn create_depth(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("point10-around-plus-depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// The torus shader with the Plane9 helpers in front.
fn around_torus_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{TORUS_WGSL}")
}

/// The water shader with the Plane9 helpers in front.
fn water_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{WATER_WGSL}")
}

const TORUS_WGSL: &str = r#"
struct AroundUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  in2: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: AroundUniforms;
@group(0) @binding(1) var u_map: texture_2d<f32>;
@group(0) @binding(2) var u_map_sampler: sampler;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) wpos: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) tex: vec2<f32>,
  @location(3) diffuse: vec4<f32>,
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
  out.wpos = world.xyz;
  out.normal = (u.model * vec4<f32>(normal, 0.0)).xyz;
  out.tex = uv;
  // v_diffuse = vec4(abs(length(position) - 2.7))
  out.diffuse = vec4<f32>(abs(length(position) - 2.7), abs(length(position) - 2.7), abs(length(position) - 2.7), abs(length(position) - 2.7));
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // Concrete044C albedo; the rescaled uvs tile it around the tube.
  var col = textureSample(u_map, u_map_sampler, in.tex).rgb * in.diffuse.rgb;

  // Large scale variations
  col = col * vec3<f32>(0.5 + p9_noise(vec3<f32>(in.tex * 0.2, 0.0)) * 0.5);

  // Line - gIn2.x is band*50; the Reinhard soft knee keeps every level
  // distinct while bounding the side-wall bloom feed to ~6x threshold.
  let flash = u.in2.x * 6.0 / (u.in2.x + 5.0);
  col = mix(
    col,
    col + vec3<f32>(flash),
    pow(1.0 - dot(abs(in.normal), vec3<f32>(0.0, 1.0, 0.0)), 8.0),
  );

  // Dark top bottom
  let x = dot(abs(in.normal), normalize(vec3<f32>(1.0, 0.0, 1.0)));
  col = col * vec3<f32>(x * x);

  return vec4<f32>(col, 1.0);
}
"#;

const RING_WGSL: &str = r#"
struct RingUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: RingUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> VsOut {
  var out: VsOut;
  // MeshBasicMaterial: the flat colour, no lighting.
  out.position = u.view_proj * (u.model * vec4<f32>(position, 1.0));
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  return u.color;
}
"#;

const WATER_WGSL: &str = r#"
struct WaterUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // x time, y distortion, z alpha, w 1 / wave resolution
  params: vec4<f32>,
  water_color: vec4<f32>,
  // xyz sun direction, w shininess
  sun_direction: vec4<f32>,
  ring_color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: WaterUniforms;
@group(0) @binding(1) var normal_map: texture_2d<f32>;
@group(0) @binding(2) var normal_sampler: sampler;
@group(0) @binding(3) var wave_tex: texture_2d<f32>;
@group(0) @binding(4) var wave_sampler: sampler;

const WATER_INNER: f32 = 4.338562;
const WATER_OUTER: f32 = 5.6614375;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) world: vec3<f32>,
  @location(1) normal: vec3<f32>,
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
  out.world = world.xyz;
  out.normal = normalize((u.model * vec4<f32>(normal, 0.0)).xyz);
  return out;
}

// The injected `waveSlope` from AroundPlus.js:83-92: sample the heightfield at
// the fragment's polar coords and turn its gradient into a normal bend.
fn wave_slope(p: vec2<f32>) -> vec3<f32> {
  let uv = vec2<f32>(
    atan2(p.y, p.x) / 6.2831853,
    (length(p) - WATER_INNER) / (WATER_OUTER - WATER_INNER),
  );
  let d = vec2<f32>(u.params.w, 0.0);
  let h = textureSample(wave_tex, wave_sampler, uv).r;
  let hu = textureSample(wave_tex, wave_sampler, uv + d).r - h;
  let hv = textureSample(wave_tex, wave_sampler, uv + d.yx).r - h;
  let radial = normalize(p);
  let tangent = vec2<f32>(-radial.y, radial.x);
  let s = tangent * hu + radial * hv;
  return vec3<f32>(s.x, 0.0, s.y) * 60.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let eye = u.camera_pos.xyz;
  let to_eye = eye - in.world;
  let eye_direction = normalize(to_eye);
  let surface_normal = normalize(in.normal + wave_slope(in.world.xz));

  // Water.js reads the normal map as `noise.xzy * (1.5, 1.0, 1.5)`; the map
  // scrolls with the water clock.
  let noise = textureSample(
    normal_map,
    normal_sampler,
    in.world.xz * 0.0075 + vec2<f32>(u.params.x * 0.025),
  ).rgb;
  let normal = normalize(noise.xzy * vec3<f32>(1.5, 1.0, 1.5) + wave_slope(in.world.xz));

  // No mirror pass in the port: the reflection is a horizon gradient tinted by
  // the glow rings, reflected about the surface.
  let reflected = reflect(-eye_direction, normal);
  let horizon = clamp(reflected.y * 0.5 + 0.5, 0.0, 1.0);
  var reflection_sample = mix(
    vec3<f32>(0.015, 0.02, 0.03),
    vec3<f32>(0.32, 0.3, 0.27),
    horizon,
  );
  reflection_sample = mix(reflection_sample, u.ring_color.rgb * 0.5, 0.12);

  let sun_direction = normalize(u.sun_direction.xyz);
  let sun_color = vec3<f32>(1.0);
  let diffuse_light = max(dot(normal, sun_direction), 0.0) * sun_color;
  let sun_reflection = reflect(-sun_direction, normal);
  let specular_light = pow(
    clamp(dot(eye_direction, sun_reflection), 0.0, 1.0),
    u.sun_direction.w,
  ) * sun_color;

  let reflectance = 0.3 + 0.7 * pow(1.0 - clamp(dot(eye_direction, normal), 0.0, 1.0), 5.0);
  let scatter = max(dot(normal, eye_direction), 0.0) * u.water_color.rgb;
  let albedo = mix(
    sun_color * diffuse_light * 0.3 + scatter,
    reflection_sample * 0.9 + specular_light * 0.5,
    reflectance,
  );

  return vec4<f32>(albedo, u.params.z);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_bounds_match_the_js() {
        let level = WATER_LEVEL;
        let inner = 5.0 - (1.0 - level * level).sqrt();
        let outer = 5.0 + (1.0 - level * level).sqrt();
        assert!(
            (WATER_INNER - inner).abs() < 1e-5,
            "{WATER_INNER} vs {inner}"
        );
        assert!((WATER_OUTER - outer).abs() < 1e-5);
        assert_eq!(WATER_LEVEL, -0.75);
    }

    #[test]
    fn water_uniforms_match_the_js() {
        let silent = water_uniforms(0.0, 0.0, 1.0 / 60.0);
        assert_eq!(silent.smooth, 0.0);
        assert!((silent.dt_time - (1.0 / 60.0) * 0.8).abs() < 1e-6);
        assert_eq!(silent.distortion_scale, 2.0);
        assert_eq!(silent.level, WATER_LEVEL);

        let loud = water_uniforms(1.0, 0.0, 1.0);
        // A 1 s frame closes the one-pole completely: k = min(1, dt * 10) = 1.
        assert_eq!(loud.smooth, 1.0);
        assert_eq!(loud.dt_time, 2.8);
        assert_eq!(loud.distortion_scale, 8.0);
        assert!((loud.level - (WATER_LEVEL + 0.1)).abs() < 1e-6);
    }

    #[test]
    fn water_swell_is_capped() {
        let long = water_uniforms(1.0, 0.9, 10.0);
        assert!((long.level - (WATER_LEVEL + 0.1)).abs() < 1e-6);
    }

    #[test]
    fn annulus_uv_maps_angle_and_radius() {
        // +X is u = 0, -X is the wrap point (u just below 1).
        let plus_x = annulus_uv(WATER_OUTER, 0.0);
        assert!((plus_x[0] - 1.0).abs() < 1e-5 || plus_x[0].abs() < 1e-5);
        assert!((plus_x[1] - 1.0).abs() < 1e-5);

        let minus_x = annulus_uv(-WATER_OUTER, 0.0);
        assert!((minus_x[0] - 0.5).abs() < 1e-5);

        let mid = annulus_uv(0.0, (WATER_INNER + WATER_OUTER) * 0.5);
        assert!((mid[0] - 0.25).abs() < 1e-5, "{mid:?}");
        assert!((mid[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn value_noise_matches_the_js_reference() {
        // Node 24 values for valueNoise(1, p).
        let cases = [
            (0.0_f32, 0.536_549_1),
            (0.5, 0.432_125_7),
            (12.3, 0.021_938_75),
            (137.37, 0.357_807_48),
            (999.9, -0.292_220_85),
        ];
        for (input, expected) in cases {
            let actual = value_noise1(input);
            assert!((actual - expected).abs() < 1e-5, "{input} -> {actual}");
        }
        assert_eq!(value_noise1(0.5), value_noise1(0.5));
        // The noise is the JS `* 2 - 1` range.
        for probe in [0.0, 3.25, 42.42, 999.9] {
            assert!((-1.0..=1.0).contains(&value_noise1(probe)));
        }
    }

    #[test]
    fn pitch_wander_stays_inside_its_bounds() {
        let mut wander = PitchWander::new(-10.0, 10.0, 3.0, 20.0);
        let mut time = 0.0_f32;
        let mut frame = 0_u32;
        for _ in 0..4000 {
            time += 1.0 / 60.0;
            frame += 1;
            let pitch = wander.next(time, seeded_rand(frame as i32, 11));
            assert!((-10.0..=10.0).contains(&pitch), "{pitch} out of range");
        }
        // The wander actually moves (it is a wander, not a constant).
        assert!(wander.next(time + 1.0, 0.0) != wander.next(time + 2.0, 0.0));
    }

    #[test]
    fn pitch_wander_reverses_at_the_interval_end() {
        let mut wander = PitchWander::new(-10.0, 10.0, 3.0, 3.0);
        // Starts at min and moves toward max; after the first 3 s interval it
        // turns around.
        assert_eq!(wander.next(0.0, 0.0), -10.0);
        let mut value = -10.0;
        for step in 1..=180 {
            value = wander.next(step as f32 / 60.0, 0.0);
        }
        assert!(value > -10.0, "the wander must climb, got {value}");
        let after = wander.next(3.1, 0.0);
        assert!(after <= 10.0);
    }

    #[test]
    fn around_torus_geometry_matches_the_js_build() {
        let mesh = build_around_torus();
        assert_eq!(mesh.vertices.len(), 257 * 46);
        assert_eq!(mesh.indices.len(), 256 * 45 * 6);

        // rotateX(π/2) maps the tube's +y to +z... verify the ring lies in XZ:
        // every vertex has |(x, z)| between 4 and 6 and |y| <= 1.
        for vertex in &mesh.vertices {
            let radius = (vertex.position[0] * vertex.position[0]
                + vertex.position[2] * vertex.position[2])
                .sqrt();
            assert!((4.0..=6.0).contains(&radius), "{radius}");
            assert!(vertex.position[1].abs() <= 1.0001);
        }

        // The uvs were rescaled to the concrete tiling.
        let max_u = mesh.vertices.iter().map(|v| v.uv[0]).fold(0.0, f32::max);
        let max_v = mesh.vertices.iter().map(|v| v.uv[1]).fold(0.0, f32::max);
        assert!((max_u - 12.64).abs() < 1e-4);
        assert!((max_v - 33.19).abs() < 1e-4);

        // The winding flip swapped the first and third index of every triangle.
        let first_original = {
            let mut original = mesh.indices.clone();
            for triangle in original.chunks_exact_mut(3) {
                triangle.swap(0, 2);
            }
            original
        };
        for (flipped, original) in mesh.indices.chunks(3).zip(first_original.chunks(3)) {
            assert_eq!(flipped[0], original[2]);
            assert_eq!(flipped[1], original[1]);
            assert_eq!(flipped[2], original[0]);
        }
    }

    #[test]
    fn ring_geometry_has_the_expected_scale() {
        let mesh = build_torus(RING_RADIUS, RING_TUBE, 12, 96);
        assert_eq!(mesh.vertices.len(), 13 * 97);
        for vertex in &mesh.vertices {
            let radius = (vertex.position[0] * vertex.position[0]
                + vertex.position[1] * vertex.position[1])
                .sqrt();
            assert!(
                (RING_RADIUS - RING_TUBE - 0.001..=RING_RADIUS + RING_TUBE + 0.001)
                    .contains(&radius)
            );
        }
    }

    #[test]
    fn water_annulus_spans_the_chord() {
        let mesh = build_water_annulus();
        let mut min_radius = f32::INFINITY;
        let mut max_radius: f32 = 0.0;
        for vertex in &mesh.vertices {
            assert_eq!(vertex.normal, [0.0, 1.0, 0.0]);
            assert_eq!(vertex.position[1], 0.0);
            let radius = (vertex.position[0] * vertex.position[0]
                + vertex.position[2] * vertex.position[2])
                .sqrt();
            min_radius = min_radius.min(radius);
            max_radius = max_radius.max(radius);
        }
        assert!((min_radius - WATER_INNER).abs() < 1e-4);
        assert!((max_radius - WATER_OUTER).abs() < 1e-4);
    }

    #[test]
    fn matrices_project_like_three() {
        // A point on the camera's near plane maps to clip z = -1; a far point
        // to +1.
        let projection = perspective(60.5_f32.to_radians(), 1.0, 0.1, 1000.0);
        let near = transform_point(&projection, [0.0, 0.0, -0.1]);
        assert!((near[2] / near[3] + 1.0).abs() < 1e-5, "{near:?}");
        let far = transform_point(&projection, [0.0, 0.0, -1000.0]);
        assert!((far[2] / far[3] - 1.0).abs() < 1e-4, "{far:?}");

        // The camera looks down -z from (-5, 0, 0): the target maps to
        // (0, 0, -1) in view space.
        let view = look_at([-5.0, 0.0, 0.0], [-5.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        let point = transform_point(&view, [-5.0, 0.0, -1.0]);
        assert!(point[0].abs() < 1e-5, "{point:?}");
        assert!(point[1].abs() < 1e-5, "{point:?}");
        assert!((point[2] + 1.0).abs() < 1e-5, "{point:?}");

        // Rotation composition: translate(1, 0, 0) * rotation_y(90°) maps the
        // origin to (1, 0, 0).
        let model = mat4_mul(
            &translation(1.0, 0.0, 0.0),
            &rotation_y(std::f32::consts::FRAC_PI_2),
        );
        let origin = transform_point(&model, [0.0, 0.0, 0.0]);
        assert!((origin[0] - 1.0).abs() < 1e-6 && origin[1].abs() < 1e-6);

        // A rotation preserves the axis it rotates about.
        let spun = transform_point(&rotation_y(1.0), [0.0, 2.0, 0.0]);
        assert!((spun[1] - 2.0).abs() < 1e-5 && spun[0].abs() < 1e-5);
    }

    #[test]
    fn expression_advances_the_spin_from_the_mid_band() {
        let mut scene = AroundPlus::new();
        let mut bridge = AudioBridge::new();
        scene.update(0.0, &bridge);
        assert_eq!(scene.perm(), 1.0);

        // Silence: band 0, so the creep is min(1, 0.1) * dt * 20.
        scene.update(0.1, &bridge);
        assert!((scene.perm() - (1.0 + 0.1 * 0.1 * 20.0)).abs() < 1e-5);

        // Full mid band pins the step at 1 * dt * 20.
        let mut payload = crate::point10::audio_bridge::BandPayload::silent(44_100.0);
        payload.lanes[1] = 1.0;
        payload.lanes[0] = 0.5;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        let before = scene.perm();
        scene.update(0.05, &bridge);
        assert!((scene.perm() - before - 1.0 * 0.05 * 20.0).abs() < 1e-4);
    }

    #[test]
    fn spin_speed_scales_the_spin() {
        let mut scene = AroundPlus::new();
        scene.apply_props(|key| (key == "spinSpeed").then_some(2.0));
        let bridge = AudioBridge::new();
        let before = scene.perm();
        scene.update(0.1, &bridge);
        assert!((scene.perm() - before - 0.1 * 0.1 * 20.0 * 2.0).abs() < 1e-5);
    }

    #[test]
    fn bass_attacks_drop_the_water() {
        let mut scene = AroundPlus::new();
        let mut bridge = AudioBridge::new();
        scene.update(0.0, &bridge);
        assert!(!scene.wave().data().iter().any(|value| *value != 0.0));

        let mut payload = crate::point10::audio_bridge::BandPayload::silent(44_100.0);
        payload.lanes[0] = 1.0;
        payload.lanes[1] = 0.0;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);

        assert!(
            scene.wave().data().iter().any(|value| *value != 0.0),
            "the attack must drop the height field"
        );
        // The water clock advances with the churn drive.
        assert!(scene.water_time() > 0.0);
        assert!(scene.distortion_scale() > WATER_DRIVE.distortion);
    }

    #[test]
    fn time_is_capped_at_a_tenth_of_a_second() {
        let mut scene = AroundPlus::new();
        let bridge = AudioBridge::new();
        scene.update(4.0, &bridge);
        assert!((scene.time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = AroundPlus::new();
        scene.apply_props(|key| match key {
            "spinSpeed" => Some(9.0),
            "ringBrightness" => Some(1.5),
            "waterChurn" => Some(-1.0),
            _ => None,
        });
        assert_eq!(scene.spin_speed, 4.0);
        assert_eq!(scene.ring_brightness, 1.5);
        assert_eq!(scene.water_churn, 0.0);
    }

    #[test]
    fn ring_colour_uses_the_phrase_palette() {
        let scene = AroundPlus::new();
        let expected = equalized_hue(ring_base_colour(), 0.0);
        assert!((scene.ring_color()[0] - expected[0]).abs() < 1e-6);
        assert!((scene.ring_color()[2] - expected[2]).abs() < 1e-6);
        // 0xffe68c in linear space, scaled past 1 so only the rings bloom.
        let base = srgb_hex_to_linear(0xff_e6_8c);
        assert!((base[0] - 1.0).abs() < 1e-5);
        assert!(base[0] * 1.8 > 1.0);
    }

    #[test]
    fn uniforms_fit_the_shader_layouts() {
        assert_eq!(std::mem::size_of::<TorusUniforms>(), 144);
        assert_eq!(std::mem::size_of::<RingUniforms>(), 144);
        assert!(std::mem::size_of::<RingUniforms>() as u64 <= RING_SLOT);
        assert_eq!(std::mem::size_of::<WaterMaterialUniforms>(), 208);
        assert_eq!(std::mem::size_of::<Vertex>(), 32);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("torus", around_torus_wgsl()),
            ("ring", RING_WGSL.to_string()),
            ("water", water_wgsl()),
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
    fn shader_keeps_the_ported_maths() {
        let torus = around_torus_wgsl();
        for needle in [
            "abs(length(position) - 2.7)",
            "0.5 + p9_noise(vec3<f32>(in.tex * 0.2, 0.0)) * 0.5",
            "u.in2.x * 6.0 / (u.in2.x + 5.0)",
            "pow(1.0 - dot(abs(in.normal), vec3<f32>(0.0, 1.0, 0.0)), 8.0)",
            "x * x",
        ] {
            assert!(torus.contains(needle), "torus lost {needle}");
        }

        let water = water_wgsl();
        for needle in [
            "atan2(p.y, p.x) / 6.2831853",
            "return vec3<f32>(s.x, 0.0, s.y) * 60.0;",
            "noise.xzy * vec3<f32>(1.5, 1.0, 1.5)",
            "0.3 + 0.7 * pow(1.0 - clamp(dot(eye_direction, normal), 0.0, 1.0), 5.0)",
            "vec4<f32>(albedo, u.params.z)",
        ] {
            assert!(water.contains(needle), "water lost {needle}");
        }
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = AroundPlus::new();
        assert_eq!(scene.meta().name, "Point10 Around Plus");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 3..],
            &["spinSpeed", "ringBrightness", "waterChurn"]
        );
        assert_eq!(names[0], "gain");
    }
}
