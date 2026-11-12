//! MirrorCube — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/MirrorCube.js`.
//!
//! The Plane9 "Mirror Cube" preset (Joakim Dahl, CC BY-NC-SA 4.0): a 1024x1024
//! render-to-texture carries the mirror. Its own fbm shader paints the base,
//! `Shader3` draws the inset edge frame over it, and the inner unit cube is
//! drawn on top sampling the **stored previous frame** (`StoreTexture`
//! feedback). The outer cube uses the same Expression rotation and samples the
//! **current** frame, so each face shows a nested reflection of the cube inside
//! it. No bloom, no PreviousLayer; the scene clears to alpha 0 so it composites
//! over the layers below it.
//!
//! # Ported exactly
//!
//! * the two 1024² write/read targets and the frame swap
//!   ([`RTT_SIZE`], [`MirrorCube::feedback_order`]): the inner cube samples
//!   `prev`, the outer cube samples the frame that was just written;
//! * Expression1 (`:97-112`): `perm += beat(0.1) * 0.1 * dt`, then
//!   `rotation.{x,y,z} = valueNoise2(perm, 0|1|2) * 360 * spinScale` degrees,
//!   with [`PERM_SEED`] = `seededRand(31, 8)` instead of `Math.random()`;
//! * Expression2 (`:119-122`): camera `z = -(band(-1,1,1,0.1) * 5 - 3) + 1.5`
//!   clamped to [`CAMERA_MIN_Z`] so loud mid bands cannot fly through the cube;
//! * the geometry (`:57-60`): unit cubes at z 1 / 1.2, the inner one scaled
//!   `(1.01251, 1.01251, 1)` on x/y;
//! * the shaders (`:135-214`): `_fbm(vec3(uv * 10, time * 0.1), 3)` for the
//!   base, the `smoothstep` frame bands (`1` inside 0.95, `0` past 0.96, plus
//!   the 0.04/0.05 inner cut), the inner cube's `dot(up, normal) * 0.5 + 0.5`
//!   shade times `0.5`, and the outer cube's `rgb * gBrightness`;
//! * the camera (`:40`): `PerspectiveCamera(45, aspect, 0.1, 1000)` at
//!   `(0, 0, z)` looking at `(0, 0, -1)`; the RTT camera uses aspect 1 and the
//!   same z;
//! * the clear: `CLEAR_ALPHA = 0` for the screen, opaque black for the RTT
//!   (`:429-432`);
//! * the composer: `RenderPass` → `OutputPass`, `NoToneMapping`, no bloom.
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `WebGLRenderTarget` ping-pong | two `Rgba8Unorm` targets + [`MirrorCube::swap_feedback`] |
//! | `BoxGeometry` | [`build_box`](crate::point10::scenes::cube_mesh::build_box) |
//! | `PlaneGeometry(2, 2)` + `OrthographicCamera` | the shared full-screen triangle |
//! | `ShaderMaterial` `modelViewMatrix`/`projectionMatrix` | one view-projection + model matrix uniform |
//! | `EffectComposer` (RenderPass, OutputPass) | [`Composer`] with no bloom and `NoToneMapping` |
//! | `valueNoise2` (Plane9 `noise2`) | [`value_noise2`] over [`seeded_rand`] |
//!
//! # Deviations
//!
//! * **The fbm base is invisible.** `Shader3` writes `vec4(vec3(v), 1.0)` with
//!   `transparent: false`, so it *replaces* the fbm pass instead of blending
//!   over it — the base is dead output in the JS too. The port keeps both
//!   passes so the chain matches the preset line for line (two full-screen
//!   triangles at 1024²; the cost is a fill-rate blip, not a behaviour
//!   difference).
//! * **Culling.** three's `FrontSide` becomes `cull_mode: None` plus a depth
//!   test. For the two closed cubes the visible surface is identical and a
//!   winding slip can never hide the scene (see `cube_mesh`).
//! * `P9Audio` is `plane9/P9Audio.js` in the JS (it re-derives the grid from
//!   the raw FFT and low-passes its own damping lanes). Like every other
//!   ported scene this reads [`P10Audio`] over the shared [`AudioBridge`], so
//!   `beat(0.1)` returns the JS's `nomusic` fallback in silence instead of 0.
//! * `texture.minFilter = LinearFilter` / `RepeatWrapping` is honoured by the
//!   RTT sampler; `generateMipmaps = false` too.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, create_depth, depth_state, look_at, mat4_mul, perspective, trs,
};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`MirrorCube.js:35-68`)
// ---------------------------------------------------------------------------

/// `RTT_SIZE` — `RenderToTexture1` is `WidthCustom`/`HeightCustom` 1024.
pub const RTT_SIZE: u32 = 1024;
/// `CAMERA.fov`.
pub const CAMERA_FOV: f32 = 45.0;
/// `CAMERA.near`.
pub const CAMERA_NEAR: f32 = 0.1;
/// `CAMERA.far`.
pub const CAMERA_FAR: f32 = 1000.0;
/// `CAMERA.z` — Expression2's output at silence.
pub const CAMERA_Z: f32 = 4.5;
/// `CAMERA_ZOOM_PER_BAND` — 5 units closer per band.
pub const CAMERA_ZOOM_PER_BAND: f32 = 5.0;
/// `CAMERA_MIN_Z` — keep the camera outside the outer cube.
pub const CAMERA_MIN_Z: f32 = 2.2;

/// `CLEAR_ALPHA` — the preset clears opaquely; modV composites transparently.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `FBM_OCTAVES`.
pub const FBM_OCTAVES: u32 = 3;
/// `FBM_SCALE`.
pub const FBM_SCALE: f32 = 10.0;
/// `FBM_TIME_SCALE`.
pub const FBM_TIME_SCALE: f32 = 0.1;

/// `INNER_POSITION` (z).
pub const INNER_Z: f32 = 1.0;
/// `OUTER_POSITION` (z).
pub const OUTER_Z: f32 = 1.2;
/// `INNER_SCALE` x/y — the inner cube is slightly oversized.
pub const INNER_SCALE_XY: f32 = 1.012_51;
/// `CUBE_SIZE`.
pub const CUBE_SIZE: f32 = 1.0;

/// `PERM_SEED = seededRand(31, 8)`.
pub fn perm_seed() -> f32 {
    seeded_rand(31, 8)
}

/// `makePerm(seed)`.
pub fn make_perm(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { perm_seed() }
}

/// Plane9's `smooth` (`t t (3 - 2t)`).
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// Plane9's value noise (`noise2` in expressions): a seeded hash over the four
/// integer cell corners, smoothstep-interpolated, mapped to `[-1, 1)`.
pub fn value_noise2(x: f32, y: f32) -> f32 {
    let ix = finite(x).floor();
    let iy = finite(y).floor();
    let fx = smooth(finite(x) - ix);
    let fy = smooth(finite(y) - iy);
    let mut value = 0.0;

    for dy in 0..=1 {
        for dx in 0..=1 {
            let corner = seeded_rand(ix as i32 + dx, iy as i32 + dy);
            value +=
                corner * if dx == 1 { fx } else { 1.0 - fx } * if dy == 1 { fy } else { 1.0 - fy };
        }
    }

    value * 2.0 - 1.0
}

/// One frame of Expression1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MirrorCubeDrive {
    /// `this.perm` for the next frame.
    pub perm: f32,
    /// `rotation.{x,y,z}` in degrees.
    pub rotation: [f32; 3],
}

/// Pure Expression1: the beat advances the phase and each axis reads `noise2`.
pub fn mirror_cube_drive(beat: f32, dt: f32, perm: f32, spin_scale: f32) -> MirrorCubeDrive {
    let beat = finite(beat).clamp(0.0, 1.0);
    let step = finite(dt).clamp(0.0, crate::point10::MAX_FRAME_SECONDS);
    let base = if perm.is_finite() { perm } else { perm_seed() };
    let next = base + beat * 0.1 * step;
    let scale = if spin_scale.is_finite() {
        spin_scale
    } else {
        1.0
    };

    MirrorCubeDrive {
        perm: next,
        rotation: [
            value_noise2(next, 0.0) * 360.0 * scale,
            value_noise2(next, 1.0) * 360.0 * scale,
            value_noise2(next, 2.0) * 360.0 * scale,
        ],
    }
}

/// Pure Expression2: `-(band * 5 - 3) + 1.5`, kept outside the cube.
pub fn mirror_camera_z(mid_band: f32) -> f32 {
    let band = finite(mid_band).clamp(0.0, 1.0);
    (CAMERA_Z - band * CAMERA_ZOOM_PER_BAND).clamp(CAMERA_MIN_Z, CAMERA_Z)
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `MIRROR_CUBE_PROPS` (`sceneProps.js:160-163`).
const MIRROR_CUBE_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

/// `gTime` + the target size for the full-screen passes (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadUniforms {
    time: f32,
    _pad: f32,
    resolution: [f32; 2],
}

/// The cube shaders' uniform block (144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CubeUniforms {
    view_proj: Mat4,
    model: Mat4,
    /// `outerMaterial.uniforms.gBrightness` (ignored by the inner cube).
    brightness: [f32; 4],
}

/// A render target plus its view.
struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// GPU resources built in `init`.
struct MirrorCubeGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    fbm_pipeline: wgpu::RenderPipeline,
    fbm_bind_group: wgpu::BindGroup,
    fbm_uniforms: wgpu::Buffer,
    frame_pipeline: wgpu::RenderPipeline,
    inner_pipeline: wgpu::RenderPipeline,
    outer_pipeline: wgpu::RenderPipeline,
    /// `inner_bind[i]` samples target `i` as the previous frame.
    inner_bind: [wgpu::BindGroup; 2],
    /// `outer_bind[i]` samples target `i` as the current frame.
    outer_bind: [wgpu::BindGroup; 2],
    inner_uniforms: wgpu::Buffer,
    outer_uniforms: wgpu::Buffer,
    mesh: MeshGpu,
    targets: [Target; 2],
    _rtt_depth: wgpu::Texture,
    rtt_depth_view: wgpu::TextureView,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    composer: Composer,
}

/// MirrorCube scene.
pub struct MirrorCube {
    audio: P10Audio,
    time: f32,
    /// `this.perm` — the Expression1 phase.
    perm: f32,
    /// The cube rotation in radians (`rotation * DEG`).
    rotation: [f32; 3],
    /// `this._cameraZ` — Expression2's last output.
    camera_z: f32,
    spin_scale: f32,
    brightness_scale: f32,
    /// The target the inner cube samples (`_prevTarget`).
    prev_index: usize,
    /// The target the feedback pass writes (`_curTarget`).
    cur_index: usize,
    size: (u32, u32),
    gpu: Option<MirrorCubeGpu>,
}

impl Default for MirrorCube {
    fn default() -> Self {
        Self::new()
    }
}

impl MirrorCube {
    /// Create the scene (`MirrorCube.js:227-356` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            time: 0.0,
            perm: make_perm(perm_seed()),
            rotation: [0.0; 3],
            camera_z: CAMERA_Z,
            spin_scale: 1.0,
            brightness_scale: 1.0,
            prev_index: 0,
            cur_index: 1,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:302-305`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(MIRROR_CUBE_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// `this.perm`.
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// The cube rotation in radians.
    pub fn rotation(&self) -> [f32; 3] {
        self.rotation
    }

    /// `this._cameraZ`.
    pub fn camera_z(&self) -> f32 {
        self.camera_z
    }

    /// Which target the inner cube samples (`_prevTarget`).
    pub fn prev_index(&self) -> usize {
        self.prev_index
    }

    /// Which target the feedback pass writes (`_curTarget`).
    pub fn cur_index(&self) -> usize {
        self.cur_index
    }

    /// The target index pairs the renderer uses this frame `(prev, cur)`.
    pub fn feedback_order(&self) -> (usize, usize) {
        (self.prev_index, self.cur_index)
    }

    /// `_swapFeedback()`: the frame just written becomes the next source.
    pub fn swap_feedback(&mut self) {
        std::mem::swap(&mut self.prev_index, &mut self.cur_index);
    }

    fn view_proj(&self, aspect: f32) -> Mat4 {
        mat4_mul(
            &perspective(CAMERA_FOV.to_radians(), aspect, CAMERA_NEAR, CAMERA_FAR),
            &look_at([0.0, 0.0, self.camera_z], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        )
    }

    fn inner_model(&self) -> Mat4 {
        trs(
            [0.0, 0.0, INNER_Z],
            self.rotation,
            [INNER_SCALE_XY, INNER_SCALE_XY, 1.0],
        )
    }

    fn outer_model(&self) -> Mat4 {
        trs([0.0, 0.0, OUTER_Z], self.rotation, [1.0, 1.0, 1.0])
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };

        gpu.queue.write_buffer(
            &gpu.fbm_uniforms,
            0,
            bytemuck::bytes_of(&QuadUniforms {
                time: self.time,
                _pad: 0.0,
                resolution: [RTT_SIZE as f32, RTT_SIZE as f32],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.inner_uniforms,
            0,
            bytemuck::bytes_of(&CubeUniforms {
                // `_applyCamera(w, h, z)` also runs on the RTT camera, with
                // aspect 1 and the same Expression2 z.
                view_proj: self.view_proj(1.0),
                model: self.inner_model(),
                brightness: [1.0, 0.0, 0.0, 0.0],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.outer_uniforms,
            0,
            bytemuck::bytes_of(&CubeUniforms {
                view_proj: self.view_proj(self.size.0 as f32 / self.size.1.max(1) as f32),
                model: self.outer_model(),
                brightness: [self.brightness_scale, 0.0, 0.0, 0.0],
            }),
        );
    }
}

impl Point10Scene for MirrorCube {
    fn name(&self) -> &'static str {
        "Point10 Mirror Cube"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);
        // `_prevTarget = _rttA; _curTarget = _rttB;`
        self.prev_index = 0;
        self.cur_index = 1;

        let quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-mirror-cube-quad-shader"),
            source: wgpu::ShaderSource::Wgsl(mirror_cube_quad_wgsl().into()),
        });
        let cube_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-mirror-cube-shader"),
            source: wgpu::ShaderSource::Wgsl(mirror_cube_cube_wgsl().into()),
        });

        // --- full-screen passes (fbm + frame) ------------------------------
        let quad_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-mirror-cube-quad-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let fbm_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-mirror-cube-fbm-uniforms"),
            size: std::mem::size_of::<QuadUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fbm_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-mirror-cube-fbm-bind-group"),
            layout: &quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: fbm_uniforms.as_entire_binding(),
            }],
        });
        let fbm_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-mirror-cube-fbm-pipeline-layout"),
            bind_group_layouts: &[Some(&quad_layout)],
            immediate_size: 0,
        });
        let fbm_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-mirror-cube-fbm-pipeline",
            &quad_shader,
            &fbm_pipeline_layout,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
        );

        let frame_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-mirror-cube-frame-pipeline-layout"),
                bind_group_layouts: &[Some(&quad_layout)],
                immediate_size: 0,
            });
        // `fs_frame` reads the same target size from the same uniform block, so
        // the frame pipeline shares the fbm layout and bind group.
        let frame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-mirror-cube-frame-pipeline"),
            layout: Some(&frame_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &quad_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &quad_shader,
                entry_point: Some("fs_frame"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // --- cube passes ---------------------------------------------------
        let cube_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-mirror-cube-cube-layout"),
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

        let inner_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-mirror-cube-inner-uniforms"),
            size: std::mem::size_of::<CubeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let outer_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-mirror-cube-outer-uniforms"),
            size: std::mem::size_of::<CubeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // `makeRenderTarget(1024)`: UnsignedByte, repeat + linear, no mips.
        let make_target = |label: &str| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: RTT_SIZE,
                    height: RTT_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            Target {
                _texture: texture,
                view,
            }
        };
        let targets = [
            make_target("point10-mirror-cube-rtt-a"),
            make_target("point10-mirror-cube-rtt-b"),
        ];
        // `target.texture.wrapS = wrapT = RepeatWrapping`, LinearFilter.
        let rtt_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-mirror-cube-rtt-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let make_bind = |uniforms: &wgpu::Buffer, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("point10-mirror-cube-cube-bind-group"),
                layout: &cube_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&rtt_sampler),
                    },
                ],
            })
        };
        let inner_bind = [
            make_bind(&inner_uniforms, &targets[0].view),
            make_bind(&inner_uniforms, &targets[1].view),
        ];
        let outer_bind = [
            make_bind(&outer_uniforms, &targets[0].view),
            make_bind(&outer_uniforms, &targets[1].view),
        ];

        let cube_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-mirror-cube-cube-pipeline-layout"),
            bind_group_layouts: &[Some(&cube_layout)],
            immediate_size: 0,
        });
        let build_cube_pipeline = |label: &str, entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&cube_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &cube_shader,
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
                    module: &cube_shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let inner_pipeline = build_cube_pipeline(
            "point10-mirror-cube-inner-pipeline",
            "fs_inner",
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let outer_pipeline =
            build_cube_pipeline("point10-mirror-cube-outer-pipeline", "fs_outer", HDR_FORMAT);

        let (rtt_depth, rtt_depth_view) =
            create_depth(&device, "point10-mirror-cube-rtt-depth", RTT_SIZE, RTT_SIZE);
        let (depth, depth_view) = create_depth(
            &device,
            "point10-mirror-cube-depth",
            self.size.0,
            self.size.1,
        );

        // `EffectComposer`: RenderPass -> OutputPass, NoToneMapping, no bloom.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        let mesh = MeshGpu::new(
            &device,
            "point10-mirror-cube-box",
            &build_box(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
        );

        self.gpu = Some(MirrorCubeGpu {
            device,
            queue,
            fbm_pipeline,
            fbm_bind_group,
            fbm_uniforms,
            frame_pipeline,
            inner_pipeline,
            outer_pipeline,
            inner_bind,
            outer_bind,
            inner_uniforms,
            outer_uniforms,
            mesh,
            targets,
            _rtt_depth: rtt_depth,
            rtt_depth_view,
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
            let (depth, depth_view) = create_depth(
                &gpu.device,
                "point10-mirror-cube-depth",
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

        let drive = mirror_cube_drive(
            self.audio.band(-1, 1, 0, 0.1),
            dt,
            self.perm,
            self.spin_scale,
        );
        self.perm = drive.perm;
        self.rotation = [
            drive.rotation[0].to_radians(),
            drive.rotation[1].to_radians(),
            drive.rotation[2].to_radians(),
        ];
        self.camera_z = mirror_camera_z(self.audio.band(-1, 1, 1, 0.1));
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (prev, cur) = self.feedback_order();

        // `_renderFeedback()`: fbm + frame + the inner cube, into `cur`.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-mirror-cube-feedback-quad"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.targets[cur].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // `setClearColor(0x000000, 1)` + clear.
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.fbm_pipeline);
            pass.set_bind_group(0, &gpu.fbm_bind_group, &[]);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&gpu.frame_pipeline);
            pass.set_bind_group(0, &gpu.fbm_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-mirror-cube-feedback-cube"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.targets[cur].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.rtt_depth_view,
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
            pass.set_pipeline(&gpu.inner_pipeline);
            pass.set_bind_group(0, &gpu.inner_bind[prev], &[]);
            gpu.mesh.draw(&mut pass);
        }

        // The outer cube samples the frame just written, then the ping-pong
        // flips for the next frame.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-mirror-cube"),
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
            pass.set_pipeline(&gpu.outer_pipeline);
            pass.set_bind_group(0, &gpu.outer_bind[cur], &[]);
            gpu.mesh.draw(&mut pass);
        }

        self.swap_feedback();

        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.render(encoder, target);
        }
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(MIRROR_CUBE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10MirrorCube.js` sets no category.
        SceneMeta::for_scene("Point10 Mirror Cube", "3d-scenes")
    }
}

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

/// The two full-screen passes (`mirrorCubeFbmShader` + `mirrorCubeFrameShader`).
fn mirror_cube_quad_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{P9_COMMON_WGSL}\n{QUAD_WGSL}")
}

/// The inner + outer cube (`buildMirrorCubeInnerVertexShader` and friends).
fn mirror_cube_cube_wgsl() -> String {
    CUBE_WGSL.to_string()
}

const QUAD_WGSL: &str = r#"
struct QuadUniforms {
  time: f32,
  _pad: f32,
  resolution: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: QuadUniforms;

// `buildMirrorCubeFbmShader`: RenderToTexture1's own `_fbm(vec3(uv * 10,
// time * 0.1), 3)` shader.
@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let v = vec3<f32>(p9_fbm(vec3<f32>(uv * vec2<f32>(10.0), u.time * 0.1), 3u));
  return vec4<f32>(v, 1.0);
}

// `buildMirrorCubeFrameShader`: Shader3's inset edge frame. The JS writes
// `smoothstep(0.96, 0.95, |p|)` with reversed edges (undefined in GLSL); the
// port spells out the same ramp with standard edges.
@fragment
fn fs_frame(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let p = (position.xy / u.resolution) * vec2<f32>(2.0) - vec2<f32>(1.0);
  let band = (1.0 - p9_smoothstep(0.95, 0.96, abs(p.x)))
    * (1.0 - p9_smoothstep(0.95, 0.96, abs(p.y)));
  let inner = p9_smoothstep(0.04, 0.05, abs(p.x))
    * p9_smoothstep(0.04, 0.05, abs(p.y));
  return vec4<f32>(vec3<f32>(band * inner), 1.0);
}
"#;

const CUBE_WGSL: &str = r#"
struct CubeUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  brightness: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: CubeUniforms;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
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
  out.uv = uv;
  // `normalize(mat3(modelMatrix) * normal)`
  out.normal = normalize((u.model * vec4<f32>(normal, 0.0)).xyz);
  return out;
}

// `buildMirrorCubeInnerFragmentShader`: the stored previous frame, dimmed by
// the world normal's +y component.
@fragment
fn fs_inner(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let v = dot(vec3<f32>(0.0, 1.0, 0.0), normal) * 0.5 + 0.5;
  return vec4<f32>(textureSample(source, source_sampler, in.uv).rgb * 0.5 * v, 1.0);
}

// `buildMirrorCubeOuterFragmentShader`: the current frame times gBrightness.
@fragment
fn fs_outer(in: VsOut) -> @location(0) vec4<f32> {
  let c = textureSample(source, source_sampler, in.uv);
  return vec4<f32>(c.rgb * u.brightness.x, c.a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-mirror-cube.test.js.

    #[test]
    fn perm_starts_at_the_seeded_phase_and_falls_back() {
        assert_eq!(make_perm(perm_seed()), perm_seed());
        assert_eq!(make_perm(4.0), 4.0);
        assert_eq!(make_perm(f32::NAN), perm_seed());
        assert!((0.0..1.0).contains(&perm_seed()));
    }

    #[test]
    fn drive_advances_perm_by_beat_times_a_tenth_of_a_tenth() {
        let silent = mirror_cube_drive(0.0, 0.1, 1.0, 1.0);
        let loud = mirror_cube_drive(0.5, 0.1, 1.0, 1.0);

        assert!((silent.perm - 1.0).abs() < 1e-6);
        assert!((loud.perm - (1.0 + 0.5 * 0.1 * 0.1)).abs() < 1e-6);
    }

    #[test]
    fn drive_clamps_the_frame_step_and_sanitises_bad_input() {
        let long_frame = mirror_cube_drive(1.0, 5.0, 1.0, 1.0);
        let negative = mirror_cube_drive(-1.0, -1.0, f32::NAN, 1.0);
        let bad = mirror_cube_drive(f32::NAN, f32::NAN, f32::NAN, 1.0);

        assert!((long_frame.perm - (1.0 + 0.1 * crate::point10::MAX_FRAME_SECONDS)).abs() < 1e-6);
        assert!((negative.perm - perm_seed()).abs() < 1e-6);
        assert!((bad.perm - perm_seed()).abs() < 1e-6);
    }

    #[test]
    fn drive_derives_each_rotation_axis_from_noise2_times_360() {
        let drive = mirror_cube_drive(0.2, 0.1, 2.0, 1.0);

        assert!((drive.rotation[0] - value_noise2(drive.perm, 0.0) * 360.0).abs() < 1e-4);
        assert!((drive.rotation[1] - value_noise2(drive.perm, 1.0) * 360.0).abs() < 1e-4);
        assert!((drive.rotation[2] - value_noise2(drive.perm, 2.0) * 360.0).abs() < 1e-4);
    }

    #[test]
    fn drive_scales_the_beat_rotation_with_spin_speed() {
        let drive = mirror_cube_drive(0.0, 0.0, 2.0, 2.0);

        assert!((drive.rotation[0] - value_noise2(2.0, 0.0) * 720.0).abs() < 1e-4);
        assert!((drive.rotation[1] - value_noise2(2.0, 1.0) * 720.0).abs() < 1e-4);
        assert!((drive.rotation[2] - value_noise2(2.0, 2.0) * 720.0).abs() < 1e-4);
    }

    #[test]
    fn value_noise_matches_plane9_at_integer_cells() {
        assert!((value_noise2(3.0, 5.0) - (seeded_rand(3, 5) * 2.0 - 1.0)).abs() < 1e-6);
        assert!((value_noise2(0.0, 0.0) - (seeded_rand(0, 0) * 2.0 - 1.0)).abs() < 1e-6);
    }

    #[test]
    fn value_noise_is_deterministic_and_inside_the_unit_interval() {
        let a = value_noise2(1.25, 7.5);
        assert_eq!(a, value_noise2(1.25, 7.5));
        for (x, y) in [(0.0, 0.0), (1.25, 7.5), (-3.4, 2.2), (137.3, 999.9)] {
            let value = value_noise2(x, y);
            assert!((-1.0..1.0).contains(&value), "({x}, {y}) -> {value}");
        }
    }

    #[test]
    fn camera_zooms_in_with_the_mid_band_and_stays_outside_the_cube() {
        assert!((mirror_camera_z(0.0) - CAMERA_Z).abs() < 1e-5);
        assert!((mirror_camera_z(0.2) - (CAMERA_Z - 1.0)).abs() < 1e-5);
        assert!((mirror_camera_z(5.0) - CAMERA_MIN_Z).abs() < 1e-5);
        assert!((mirror_camera_z(-1.0) - CAMERA_Z).abs() < 1e-5);
        assert!((mirror_camera_z(f32::NAN) - CAMERA_Z).abs() < 1e-5);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(RTT_SIZE, 1024);
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(FBM_OCTAVES, 3);
        assert_eq!(FBM_SCALE, 10.0);
        assert_eq!(CAMERA_FOV, 45.0);
        assert_eq!(CAMERA_NEAR, 0.1);
        assert_eq!(CAMERA_FAR, 1000.0);
        assert_eq!(CAMERA_Z, 4.5);
        assert_eq!(INNER_Z, 1.0);
        assert_eq!(OUTER_Z, 1.2);
        assert!((INNER_SCALE_XY - 1.012_51).abs() < 1e-6);
        assert_eq!(CUBE_SIZE, 1.0);
    }

    #[test]
    fn feedback_starts_with_prev_a_and_flips_every_frame() {
        let mut scene = MirrorCube::new();
        assert_eq!(scene.feedback_order(), (0, 1));
        scene.swap_feedback();
        assert_eq!(scene.feedback_order(), (1, 0));
        scene.swap_feedback();
        assert_eq!(scene.feedback_order(), (0, 1));
    }

    #[test]
    fn update_drives_the_rotation_and_the_camera() {
        let mut scene = MirrorCube::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[3] = 0.4; // damping lane 1 of the bass band (beat(0.1))
        payload.lanes[4] = 0.3; // damping lane 1 of the mid band (camera)
        payload.byte_frequency_data = vec![0; 1024];
        payload.freq_mag = [0.5; crate::point10::audio_bridge::AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.1, &bridge);
        let expected_perm = perm_seed() + 0.4 * 0.1 * 0.1;
        assert!((scene.perm() - expected_perm).abs() < 1e-5);
        assert!(
            (scene.rotation()[1] - value_noise2(expected_perm, 1.0).to_radians() * 360.0).abs()
                < 1e-4
        );
        // `mirrorCameraZ(0.3)`
        assert!((scene.camera_z() - (CAMERA_Z - 0.3 * CAMERA_ZOOM_PER_BAND)).abs() < 1e-5);
    }

    #[test]
    fn spin_speed_scales_the_rotation() {
        let mut scene = MirrorCube::new();
        scene.apply_props(|key| (key == "spinSpeed").then_some(2.0));
        scene.update(0.0, &AudioBridge::new());
        let expected = value_noise2(perm_seed(), 0.0) * 720.0;
        assert!((scene.rotation()[0] - expected.to_radians()).abs() < 1e-4);
    }

    #[test]
    fn brightness_binds_and_clamps() {
        let mut scene = MirrorCube::new();
        scene.apply_props(|key| (key == "brightness").then_some(9.0));
        assert_eq!(scene.brightness_scale, 3.0);
        scene.apply_props(|key| (key == "brightness").then_some(-1.0));
        assert_eq!(scene.brightness_scale, 0.0);
    }

    #[test]
    fn time_is_capped_at_a_tenth_of_a_second() {
        let mut scene = MirrorCube::new();
        scene.update(4.0, &AudioBridge::new());
        assert!((scene.time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn uniforms_fit_the_shader_layouts() {
        assert_eq!(std::mem::size_of::<QuadUniforms>(), 16);
        assert_eq!(std::mem::size_of::<CubeUniforms>(), 144);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("quad", mirror_cube_quad_wgsl()),
            ("cube", mirror_cube_cube_wgsl()),
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
        let quad = mirror_cube_quad_wgsl();
        for needle in [
            "p9_fbm(vec3<f32>(uv * vec2<f32>(10.0), u.time * 0.1), 3u)",
            "p9_smoothstep(0.95, 0.96, abs(p.x))",
            "p9_smoothstep(0.95, 0.96, abs(p.y))",
            "p9_smoothstep(0.04, 0.05, abs(p.x))",
            "p9_smoothstep(0.04, 0.05, abs(p.y))",
            "vec3<f32>(band * inner)",
        ] {
            assert!(quad.contains(needle), "quad lost {needle}");
        }

        let cube = mirror_cube_cube_wgsl();
        for needle in [
            "normalize((u.model * vec4<f32>(normal, 0.0)).xyz)",
            "dot(vec3<f32>(0.0, 1.0, 0.0), normal) * 0.5 + 0.5",
            "textureSample(source, source_sampler, in.uv).rgb * 0.5 * v",
            "c.rgb * u.brightness.x",
        ] {
            assert!(cube.contains(needle), "cube lost {needle}");
        }
        assert!(!cube.contains("gl_InstanceID"));
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = MirrorCube::new();
        assert_eq!(scene.meta().name, "Point10 Mirror Cube");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(&names[names.len() - 2..], &["spinSpeed", "brightness"]);
        assert_eq!(names[0], "gain");
    }
}
