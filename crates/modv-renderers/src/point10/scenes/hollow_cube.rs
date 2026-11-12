//! HollowCube — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/HollowCube.js`.
//!
//! The JS scene is a **WebGL** `BaseScene` (not WebGPU): the Plane9 "Hollow
//! Cube" preset (Joakim Dahl, CC BY-NC-SA 4.0) as 15 000 instanced 0.01-unit
//! cubes on the six faces of a 50x50x50 lattice. `Shader1` positions every cube
//! from its instance id (the hollow-shell mapping), rotates it by a random
//! axis-angle quaternion and pulses it outward along the lattice normal; the
//! fragment is lit with Plane9's half-Lambert/Blinn-Phong pair. The cube chain
//! is captured into a `RenderToTexture`, bloomed, and `Shader2` composites the
//! capture, the bloom and a 256x256 fbm capture.
//!
//! # Ported exactly
//!
//! * the instance placement (`HollowCube.js:137-204`): `o` from
//!   `mod(id, 50)` / `mod(id / 50, 50)` and the six face branches, `p = o/50`,
//!   `up = (p - 0.5) * 0.5`, `pulse = _cubicpulse(mod(p.x+p.y+p.z+t, 1.5),
//!   0.6, 0.4)` with `t = gIn2.x * 0.1`, the dead interior branch, the
//!   axis-angle rotation `pulse * 6 * gIn1.z`, the constant `nv` noise
//!   direction (`d = 0.0001` in the JS — see the shader note), the
//!   `up * gIn2.y` offset and the per-instance `vDiffuse` (HSV 0/0.9/0.8 times
//!   `0.1 + 0.9 * _rand(vec2(id))`);
//! * the fragment (`:207-264`): `vDiffuse * gIn1.y`, the two
//!   `_lightBlinnPhongHalfLambert` terms at `specularHardness = 10`, the
//!   `(0, 1, 0)` sky light `(0.5, 0.8, 1.0)` * 1.15 at 0.04 and the
//!   `(0, -1, 0)` ground light `(0.8, 0, 0)` / white specular at 0.025;
//! * the fbm capture (`:266-277`): `_fbm(vec3(uv * 10, gTime * 0.1), 3)`;
//! * `Shader2` (`:279-298`): `mix(bg.rgb, _tonemapACES(c2.rgb), c2.a) +
//!   _tonemapACES(c1.rgb * c1.rgb)`, alpha `c2.a` (the cube capture's alpha);
//! * Expression1 (`hollowCubeDrive`, `:71-100`): the three bass reads at
//!   damping 0/1/2, `perm += min(0.5, band + 0.01) * 40 * dt` (0.1 s cap),
//!   `in1 = (min(1.2, e*2.2)*0.6, min(0.55, l*4 + 0.4), min(1, l*1.5))`,
//!   `in2 = (perm, 1 + min(0.2, l))`;
//! * the camera (`:44-51`): `PerspectiveCamera(50, aspect, 0.01, 100)` at
//!   `(0, 0, 2.2)` looking at `(0, 0, -1)`;
//! * the bloom (`:53-56`): `UnrealBloomPass(strength 0.51, radius 0.85,
//!   threshold 0)`;
//! * the prop binder (`sceneProps.js:312-317`): `spinScale = spinSpeed`,
//!   `explodeScale = explode`, `colorScale = color`, `bloomScale = bloom`.
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `InstancedBufferGeometry` + `aInstance` (15 000 floats) | `@builtin(instance_index)` — no attribute buffer |
//! | `BoxGeometry(0.01)` | [`build_box`](crate::point10::scenes::cube_mesh::build_box) |
//! | `projectionMatrix * modelViewMatrix` (identity model) | one view-projection uniform |
//! | `WebGLRenderTarget` cube capture + fbm capture | two `Rgba16Float` targets |
//! | `EffectComposer` (copy → bloom) | [`UnrealBloomPass`] fed from the cube target |
//! | `ShaderPass` + `OutputPass` | the port's `fs_main` composite + [`Composer`]'s output pass |
//! | `_fbm`, `_cubicpulse`, `_hsv2rgb`, `_rand`, `_rand3`, `_noise`, `_rotateVector`, `_axisAngleToQuaternion` | [`P9_COMMON_WGSL`] + [`P9_SHADING_WGSL`] (`p9_rand3`) |
//!
//! # Deviations
//!
//! * **`_rand3(vec2)` is local.** `P9Glsl.js:44` declares a `vec2` overload of
//!   `_rand3` that the shared WGSL subset does not carry (batch 2 only needed
//!   the scalar form). The scene keeps it as `hollow_rand3_vec2`, verbatim.
//! * **`nv` is computed per vertex, not per instance.** The JS writes
//!   `_noise(vec3(d * 50, axis, 0))` with `d = 0.0001`, which is a *constant*
//!   (the instance id never enters), so this is the same number with the same
//!   `sign(up.axis)` multiplier.
//! * **The fbm capture is `Rgba16Float`, not `UnsignedByte`.** The JS
//!   `RenderToTexture1` uses an 8-bit target; the port keeps the value in HDR
//!   like every other composer target (one less format to bind around).
//! * `_tonemapACES` is not part of the shared WGSL subset yet; the scene carries
//!   the exact Plane9 formula (`P9Glsl.js:454-462`).
//! * `Renderer.setClearColor(0, 0)` for the cube capture and the fbm capture is
//!   reproduced by the render-pass clears; the final `Shader2` alpha is the
//!   cube capture's, so empty space stays transparent (`CLEAR_ALPHA = 0`).
//! * `P9Audio` is `plane9/P9Audio.js` in the JS; the port reads [`P10Audio`]
//!   over the shared [`AudioBridge`], and all three Expression reads are
//!   `band(-1, damping, 0, 0.1)` — **bass** at damping lanes 0/1/2, not three
//!   different bands.
//!
//! [`UnrealBloomPass`]: crate::point10::postfx::UnrealBloomPass
//! [`Composer`]: crate::point10::postfx::Composer

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping, UnrealBloomPass};
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, create_depth, depth_state, look_at, mat4_mul, perspective,
};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::p9_shading::P9_SHADING_WGSL;
use crate::point10::{
    HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`HollowCube.js:39-69`)
// ---------------------------------------------------------------------------

/// `INSTANCE_COUNT` — `MeshInstancer1` NumInstances.
pub const INSTANCE_COUNT: u32 = 15_000;
/// `LATTICE_COUNT` — the 50x50x50 shell.
pub const LATTICE_COUNT: u32 = 50;
/// `CUBE_SIZE`.
pub const CUBE_SIZE: f32 = 0.01;

/// `CAMERA.fov`.
pub const CAMERA_FOV: f32 = 50.0;
/// `CAMERA.near`.
pub const CAMERA_NEAR: f32 = 0.01;
/// `CAMERA.far`.
pub const CAMERA_FAR: f32 = 100.0;
/// `CAMERA.position`.
pub const CAMERA_POSITION: [f32; 3] = [0.0, 0.0, 2.2];
/// `CAMERA.lookAt`.
pub const CAMERA_LOOK_AT: [f32; 3] = [0.0, 0.0, -1.0];

/// `BLOOM.strength`.
pub const BLOOM_STRENGTH: f32 = 0.51;
/// `BLOOM.radius` (Plane9 Radius 6 normalised by `UnrealBloomPass`).
pub const BLOOM_RADIUS: f32 = 0.85;
/// `BLOOM.threshold`.
pub const BLOOM_THRESHOLD: f32 = 0.0;
/// `BLOOM_SMOOTHNESS` (kept for parity; `UnrealBloomPass` hard-codes 0.01).
pub const BLOOM_SMOOTHNESS: f32 = 0.5;

/// `CLEAR_ALPHA` — Hollow Cube composites transparently over the layers below.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `FBM_SIZE` — `RenderToTexture1` WidthCustom/HeightCustom.
pub const FBM_SIZE: u32 = 256;
/// `FBM_OCTAVES`.
pub const FBM_OCTAVES: u32 = 3;

/// `MAX_DT` — Expression1's own 0.1 s step cap.
pub const MAX_DT: f32 = 0.1;

/// `makePerm(seed = 1)`.
pub fn make_perm(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { 1.0 }
}

/// One frame of Expression1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HollowCubeDrive {
    /// `perm` for the next frame.
    pub perm: f32,
    /// `gIn1` (`x` explosion, `y` colour, `z` rotation).
    pub in1: [f32; 3],
    /// `gIn2` (`x` phase, `y` cube scale).
    pub in2: [f32; 3],
}

/// `bandValue`: `Math.max(0, Number.isFinite(value) ? value : 0)`.
fn band_value(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

/// Pure Expression1 (`HollowCube.js:80-100`).
///
/// All three arguments are **bass** reads (`band(-1, damping, 0, 0.1)`) at
/// damping 0, 1 and 2: lane 0 advances the phase, lane 2 explodes the shell and
/// lane 1 drives colour/rotation/size.
pub fn hollow_cube_drive(
    perm_band: f32,
    level_band: f32,
    explode_band: f32,
    dt: f32,
    perm: f32,
) -> HollowCubeDrive {
    let p = band_value(perm_band);
    let l = band_value(level_band);
    let e = band_value(explode_band);
    let step = band_value(dt).min(MAX_DT);
    let base = if perm.is_finite() { perm } else { 1.0 };
    let next = base + (p + 0.01).min(0.5) * 40.0 * step;

    HollowCubeDrive {
        perm: next,
        in1: [
            (e * 2.2).min(1.2) * 0.6,
            (l * 4.0 + 0.4).min(0.55),
            (l * 1.5).min(1.0),
        ],
        in2: [next, 1.0 + l.min(0.2), 0.0],
    }
}

/// `MeshInstancer`'s hollow-cube mapping: instance id -> one of the six faces
/// of an `n x n x n` cell grid, normalised to `0..1` (`HollowCube.js:104-125`).
pub fn hollow_cube_pos(index: u32, count: u32) -> [f32; 3] {
    let count = count.max(1);
    let n = ((count as f32 / 6.0).sqrt().round() as u32).max(1);
    let per_face = n * n;
    let face = index / per_face;
    let j = index % per_face;
    let gx = j % n;
    let gy = j / n;
    let o = match face {
        0 => [0, gx, gy],
        1 => [n, gx, gy],
        2 => [gx, 0, gy],
        3 => [gx, n, gy],
        4 => [gx, gy, 0],
        _ => [gx, gy, n],
    };
    let n = n as f32;
    [o[0] as f32 / n, o[1] as f32 / n, o[2] as f32 / n]
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `HOLLOW_CUBE_PROPS` (`sceneProps.js:172-177`).
const HOLLOW_CUBE_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("explode", "Explosion", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("color", "Color", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
];

/// The cube shader's uniform block (112 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CubeUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    /// `gIn1`: explosion, colour, rotation.
    in1: [f32; 4],
    /// `gIn2`: phase, cube scale.
    in2: [f32; 4],
}

/// The fbm pass' uniform block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FbmUniforms {
    /// `gTime`.
    time: f32,
    _pad: f32,
    /// Target size, for `position.xy / resolution`.
    resolution: [f32; 2],
}

/// The `Shader2` pass' uniform block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CompositeUniforms {
    /// Target size, for `position.xy / resolution`.
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// A render target plus its view.
struct Target {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    fn new(device: &wgpu::Device, label: &str, width: u32, height: u32) -> Self {
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
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
struct HollowCubeGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    cube_pipeline: wgpu::RenderPipeline,
    cube_bind_group: wgpu::BindGroup,
    cube_uniforms: wgpu::Buffer,
    cube_target: Target,
    _cube_depth: wgpu::Texture,
    cube_depth_view: wgpu::TextureView,
    fbm_pipeline: wgpu::RenderPipeline,
    fbm_bind_group: wgpu::BindGroup,
    fbm_uniforms: wgpu::Buffer,
    fbm_target: Target,
    /// `composite_bind` samples the cube target, the bloom composite and the
    /// fbm target.
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,
    composite_bind_group: wgpu::BindGroup,
    composite_uniforms: wgpu::Buffer,
    /// The bloom of the cube capture (`_bloomPass`).
    bloom: UnrealBloomPass,
    mesh: MeshGpu,
    composer: Composer,
}

/// HollowCube scene.
pub struct HollowCube {
    audio: P10Audio,
    /// `this.perm`.
    perm: f32,
    /// `gIn1` after the prop scales.
    in1: [f32; 4],
    /// `gIn2`.
    in2: [f32; 4],
    /// `this.time` (the fbm clock).
    time: f32,
    spin_scale: f32,
    explode_scale: f32,
    color_scale: f32,
    bloom_scale: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<HollowCubeGpu>,
}

impl Default for HollowCube {
    fn default() -> Self {
        Self::new()
    }
}

impl HollowCube {
    /// Create the scene (`HollowCube.js:334-384` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            perm: make_perm(1.0),
            in1: [0.0, 1.1, 0.0, 0.0],
            in2: [0.0, 1.0, 0.0, 0.0],
            time: 0.0,
            spin_scale: 1.0,
            explode_scale: 1.0,
            color_scale: 1.0,
            bloom_scale: 1.0,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:312-317`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(HOLLOW_CUBE_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_scale = value,
                "explode" => self.explode_scale = value,
                "color" => self.color_scale = value,
                "bloom" => self.bloom_scale = value,
                _ => {}
            }
        }
    }

    /// `this.perm`.
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// `gIn1` after the prop scales.
    pub fn in1(&self) -> [f32; 4] {
        self.in1
    }

    /// `gIn2` as last uploaded.
    pub fn in2(&self) -> [f32; 4] {
        self.in2
    }

    /// `this.time`.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &look_at(CAMERA_POSITION, CAMERA_LOOK_AT, [0.0, 1.0, 0.0]),
        )
    }

    fn upload_cube_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.cube_uniforms,
            0,
            bytemuck::bytes_of(&CubeUniforms {
                view_proj: self.view_proj(),
                camera_pos: [
                    CAMERA_POSITION[0],
                    CAMERA_POSITION[1],
                    CAMERA_POSITION[2],
                    1.0,
                ],
                in1: self.in1,
                in2: self.in2,
            }),
        );
        gpu.queue.write_buffer(
            &gpu.fbm_uniforms,
            0,
            bytemuck::bytes_of(&FbmUniforms {
                time: self.time,
                _pad: 0.0,
                resolution: [FBM_SIZE as f32, FBM_SIZE as f32],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.composite_uniforms,
            0,
            bytemuck::bytes_of(&CompositeUniforms {
                resolution: [self.size.0 as f32, self.size.1 as f32],
                _pad: [0.0; 2],
            }),
        );
    }

    /// Re-create the size-dependent targets and re-bind the passes.
    fn rebuild_targets(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        gpu.cube_target = Target::new(
            &gpu.device,
            "point10-hollow-cube-capture",
            self.size.0,
            self.size.1,
        );
        let (depth, depth_view) = create_depth(
            &gpu.device,
            "point10-hollow-cube-depth",
            self.size.0,
            self.size.1,
        );
        gpu._cube_depth = depth;
        gpu.cube_depth_view = depth_view;
        gpu.bloom.resize(self.size.0, self.size.1);
        gpu.bloom.set_input(&gpu.cube_target.view);
        gpu.composer.resize(self.size.0, self.size.1);
        gpu.composite_bind_group = create_composite_bind_group(&gpu.device, gpu);
        self.upload_cube_uniforms();
    }
}

impl Point10Scene for HollowCube {
    fn name(&self) -> &'static str {
        "Point10 Hollow Cube"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);
        self.camera_aspect = ctx.aspect();

        let mesh = MeshGpu::new(
            &device,
            "point10-hollow-cube-mesh",
            &build_box(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
        );

        // --- cube capture --------------------------------------------------
        let cube_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-hollow-cube-shader"),
            source: wgpu::ShaderSource::Wgsl(hollow_cube_cube_wgsl().into()),
        });
        let cube_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-hollow-cube-layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let cube_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-hollow-cube-uniforms"),
            size: std::mem::size_of::<CubeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cube_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-hollow-cube-bind-group"),
            layout: &cube_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cube_uniforms.as_entire_binding(),
            }],
        });
        let cube_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-hollow-cube-pipeline-layout"),
            bind_group_layouts: &[Some(&cube_layout)],
            immediate_size: 0,
        });
        let cube_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-hollow-cube-pipeline"),
            layout: Some(&cube_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &cube_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(crate::point10::scenes::cube_mesh::Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Shader1 is `DoubleSide`.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &cube_shader,
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

        // --- fbm capture ---------------------------------------------------
        let fbm_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-hollow-cube-fbm-shader"),
            source: wgpu::ShaderSource::Wgsl(hollow_cube_fbm_wgsl().into()),
        });
        let fbm_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-hollow-cube-fbm-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::FRAGMENT)],
        });
        let fbm_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-hollow-cube-fbm-uniforms"),
            size: std::mem::size_of::<FbmUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fbm_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-hollow-cube-fbm-bind-group"),
            layout: &fbm_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: fbm_uniforms.as_entire_binding(),
            }],
        });
        let fbm_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-hollow-cube-fbm-pipeline",
            &fbm_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-hollow-cube-fbm-pipeline-layout"),
                bind_group_layouts: &[Some(&fbm_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );

        // --- Shader2 composite --------------------------------------------
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-hollow-cube-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(hollow_cube_composite_wgsl().into()),
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-hollow-cube-composite-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                sampler_entry(4),
            ],
        });
        let composite_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-hollow-cube-composite-uniforms"),
            size: std::mem::size_of::<CompositeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-hollow-cube-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let composite_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-hollow-cube-composite-pipeline",
            &composite_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-hollow-cube-composite-pipeline-layout"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );

        let cube_target = Target::new(
            &device,
            "point10-hollow-cube-capture",
            self.size.0,
            self.size.1,
        );
        let fbm_target = Target::new(&device, "point10-hollow-cube-fbm", FBM_SIZE, FBM_SIZE);
        let (cube_depth, cube_depth_view) = create_depth(
            &device,
            "point10-hollow-cube-depth",
            self.size.0,
            self.size.1,
        );

        // `_bloomComposer`: copy the cube capture into the bloom's bright pass.
        let mut bloom = UnrealBloomPass::new(&device, &queue, self.size.0, self.size.1);
        bloom.set_strength(BLOOM_STRENGTH);
        bloom.set_radius(BLOOM_RADIUS);
        bloom.set_threshold(BLOOM_THRESHOLD);
        bloom.set_input(&cube_target.view);

        // `composer = EffectComposer(renderer); addPass(_finalPass); addPass(OutputPass)`.
        // No bloom in the composer: the Shader2 pass already adds it, exactly
        // like the JS (which only adds `_finalPass` + `OutputPass` here).
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-hollow-cube-composite-bind-group"),
            layout: &composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: composite_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&cube_target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    // `composite_view()` is Some after `new` (the targets are
                    // rebuilt in the constructor), so the bloom target can be
                    // bound here; on resize `rebuild_targets` re-binds it.
                    resource: wgpu::BindingResource::TextureView(
                        bloom
                            .composite_view()
                            .expect("the bloom pass always has a composite target"),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&fbm_target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        self.gpu = Some(HollowCubeGpu {
            device,
            queue,
            cube_pipeline,
            cube_bind_group,
            cube_uniforms,
            cube_target,
            _cube_depth: cube_depth,
            cube_depth_view,
            fbm_pipeline,
            fbm_bind_group,
            fbm_uniforms,
            fbm_target,
            composite_pipeline,
            composite_layout,
            composite_sampler: sampler,
            composite_bind_group,
            composite_uniforms,
            bloom,
            mesh,
            composer,
        });
        self.upload_cube_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        self.rebuild_targets();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.audio.update(dt, audio);
        self.time += dt;

        let drive = hollow_cube_drive(
            self.audio.band(-1, 0, 0, 0.1),
            self.audio.band(-1, 1, 0, 0.1),
            self.audio.band(-1, 2, 0, 0.1),
            dt,
            self.perm,
        );
        self.perm = drive.perm;
        // `material.uniforms.gIn1.value.set(drive.in1.* * *Scale)`
        self.in1 = [
            drive.in1[0] * self.explode_scale,
            drive.in1[1] * self.color_scale,
            drive.in1[2] * self.spin_scale,
            0.0,
        ];
        self.in2 = [drive.in2[0], drive.in2[1], 0.0, 0.0];
        self.upload_cube_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        // Cube capture: clear transparent, depth clear, 15 000 instances.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-hollow-cube-capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.cube_target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.cube_depth_view,
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
            pass.set_pipeline(&gpu.cube_pipeline);
            pass.set_bind_group(0, &gpu.cube_bind_group, &[]);
            gpu.mesh.draw_instanced(&mut pass, INSTANCE_COUNT);
        }

        // fbm capture (`RenderToTexture1`).
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-hollow-cube-fbm"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.fbm_target.view,
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
            pass.set_pipeline(&gpu.fbm_pipeline);
            pass.set_bind_group(0, &gpu.fbm_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // `_bloomComposer.render()`: copy + UnrealBloomPass of the capture.
        // The bright pass is bound to the cube target in `init` and after
        // every resize (`rebuild_targets`).
        gpu.bloom.encode(encoder);

        // Shader2 → the composer's scene target, then OutputPass.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-hollow-cube-composite"),
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
            pass.set_pipeline(&gpu.composite_pipeline);
            pass.set_bind_group(0, &gpu.composite_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(HOLLOW_CUBE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10HollowCube.js` sets no category.
        SceneMeta::for_scene("Point10 Hollow Cube", "3d-scenes")
    }
}

/// Rebuild the composite bind group after [`Target`]s are recreated
/// (`resize`), because wgpu bind groups hold the texture views they were made
/// from.
fn create_composite_bind_group(device: &wgpu::Device, gpu: &HollowCubeGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-hollow-cube-composite-bind-group"),
        layout: &gpu.composite_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: gpu.composite_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&gpu.cube_target.view),
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
                resource: wgpu::BindingResource::TextureView(&gpu.fbm_target.view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&gpu.composite_sampler),
            },
        ],
    })
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

/// `buildHollowCubeVertexShader` + `buildHollowCubeFragmentShader` with the
/// Plane9 helpers in front.
fn hollow_cube_cube_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{P9_SHADING_WGSL}\n{HOLLOW_CUBE_CUBE_WGSL}")
}

/// `buildHollowCubeFbmFragment`.
fn hollow_cube_fbm_wgsl() -> String {
    format!(
        "{}\n{}\n{HOLLOW_CUBE_FBM_WGSL}",
        crate::point10::FULLSCREEN_TRIANGLE_WGSL,
        P9_COMMON_WGSL
    )
}

/// `buildHollowCubeShader2Fragment`.
fn hollow_cube_composite_wgsl() -> String {
    format!(
        "{}\n{P9_ACES_WGSL}\n{HOLLOW_CUBE_COMPOSITE_WGSL}",
        crate::point10::FULLSCREEN_TRIANGLE_WGSL
    )
}

/// `_tonemapACES` (`P9Glsl.js:454-462`), verbatim.
const P9_ACES_WGSL: &str = r#"
fn p9_saturate3(color: vec3<f32>) -> vec3<f32> {
  return clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn p9_tonemap_aces(color: vec3<f32>) -> vec3<f32> {
  let a = 2.51;
  let b = 0.03;
  let c = 2.43;
  let d = 0.59;
  let e = 0.14;
  return p9_saturate3((color * (a * color + b)) / (color * (c * color + d) + e));
}
"#;

const HOLLOW_CUBE_CUBE_WGSL: &str = r#"
struct CubeUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // x explosion, y colour, z rotation
  in1: vec4<f32>,
  // x phase, y cube scale
  in2: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: CubeUniforms;

// `vec3 _rand3(vec2 p)` (`P9Glsl.js:44`) — the vec2 overload is not in the
// shared subset yet.
fn hollow_rand3_vec2(p0: vec2<f32>) -> vec3<f32> {
  var p = fract(p0 * vec2<f32>(5.3987, 5.4421));
  p = p + vec2<f32>(dot(p.yx, p.xy + vec2<f32>(21.5351, 14.3137)));
  return fract(vec3<f32>(
    p.x * p.y * 95.4307,
    p.x * p.y * 97.5901,
    p.x * p.y * 93.8369,
  ));
}

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) diffuse: vec4<f32>,
  @location(1) w_normal: vec3<f32>,
  @location(2) w_pos: vec3<f32>,
  @location(3) view_dir: vec3<f32>,
}

@vertex
fn vs_main(
  @builtin(instance_index) instance: u32,
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
) -> VsOut {
  var out: VsOut;
  let instance_id = f32(instance);
  let d = 0.0001;
  let dt = instance_id / 15000.0;
  let obj_count = 50.0;
  let t = u.in2.x * 0.1;

  // `attr float aInstance` is `@builtin(instance_index)` here.
  let index = vec2<f32>(
    p9_mod(instance_id, obj_count),
    p9_mod(instance_id / obj_count, obj_count),
  );

  var o = vec3<f32>(0.0);
  if (instance_id < obj_count * obj_count) {
    o = vec3<f32>(0.0, index.x, index.y);
  } else if (instance_id < obj_count * obj_count * 2.0) {
    o = vec3<f32>(obj_count, index.x, index.y);
  } else if (instance_id < obj_count * obj_count * 3.0) {
    o = vec3<f32>(index.x, 0.0, index.y);
  } else if (instance_id < obj_count * obj_count * 4.0) {
    o = vec3<f32>(index.x, obj_count, index.y);
  } else if (instance_id < obj_count * obj_count * 5.0) {
    o = vec3<f32>(index.x, index.y, 0.0);
  } else {
    o = vec3<f32>(index.x, index.y, obj_count);
  }

  let p = o / 50.0;
  let up = (p - vec3<f32>(0.5)) * 0.5;
  var pulse = p9_cubicpulse(p9_mod(p.x + p.y + p.z + t, 1.5), 0.6, 0.4);
  var xx = 1.0;

  // The JS interior test never fires (every instance lies on the shell); kept
  // so the shader reads like the preset.
  if (o.x >= 1.0 && o.x < obj_count - 1.0 &&
      o.y >= 1.0 && o.y < obj_count - 1.0 &&
      o.z >= 1.0 && o.z < obj_count - 1.0) {
    pulse = 0.0;
    xx = 0.0;
  }

  let rotation = vec4<f32>(
    normalize(hollow_rand3_vec2(vec2<f32>(instance_id))),
    dt + u.in2.x,
  );
  let q_rotation = p9_axis_angle_to_quaternion(rotation.xyz, pulse * 6.0 * u.in1.z);
  var v = p9_rotate_vector(vec4<f32>(position, 1.0).xyz, q_rotation);

  // `nv` is a constant: `d * 50.0` never depends on the instance.
  var nv = vec3<f32>(
    (p9_noise(vec3<f32>(d * 50.0, 0.0, 0.0)) * 0.5 + 0.5) * sign(up.x),
    (p9_noise(vec3<f32>(d * 50.0, 1.0, 0.0)) * 0.5 + 0.5) * sign(up.y),
    (p9_noise(vec3<f32>(d * 50.0, 2.0, 0.0)) * 0.5 + 0.5) * sign(up.z),
  );

  v = v + nv * pulse * u.in1.x;
  v = v + up * u.in2.y * xx;

  out.position = u.view_proj * vec4<f32>(v, 1.0);
  out.diffuse = vec4<f32>(p9_hsv2rgb(0.0, 0.9, 0.8), 1.0)
    * (0.1 + 0.9 * p9_rand2(vec2<f32>(instance_id)));
  // `mat3(modelMatrix)` is the identity.
  out.w_normal = normalize(p9_rotate_vector(normal, q_rotation));
  out.w_pos = v;
  out.view_dir = u.camera_pos.xyz - v;
  return out;
}

fn light_half_lambert(normal: vec3<f32>, light_dir: vec3<f32>, diffuse: vec3<f32>) -> vec3<f32> {
  return (dot(light_dir, normal) * 0.5 + 0.5) * diffuse;
}

fn light_blinn_phong_half_lambert(
  normal: vec3<f32>,
  light_dir: vec3<f32>,
  view_dir: vec3<f32>,
  diffuse: vec3<f32>,
  specular: vec3<f32>,
  specular_hardness: f32,
) -> vec3<f32> {
  let half_vec = normalize(light_dir + view_dir);
  let spec = pow(p9_saturate(dot(normal, half_vec)), specular_hardness)
    * ((specular_hardness + 4.0) / 8.0);
  return light_half_lambert(normal, light_dir, diffuse) + specular * spec;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.w_normal);
  let view_dir = normalize(in.view_dir);
  let specular_hardness = 10.0;

  var c = in.diffuse.rgb * u.in1.y;

  var light_dir = normalize(vec3<f32>(0.0, 1.0, 0.0) - in.w_pos);
  c = c + light_blinn_phong_half_lambert(
    normal,
    light_dir,
    view_dir,
    vec3<f32>(0.5, 0.8, 1.0),
    vec3<f32>(0.5, 0.8, 1.0) * 1.15,
    specular_hardness,
  ) * 0.04;

  light_dir = normalize(vec3<f32>(0.0, -1.0, 0.0) - in.w_pos);
  c = c + light_blinn_phong_half_lambert(
    normal,
    light_dir,
    view_dir,
    vec3<f32>(0.8, 0.0, 0.0),
    vec3<f32>(1.0, 1.0, 1.0) * 0.15,
    specular_hardness,
  ) * 0.025;

  return vec4<f32>(max(vec3<f32>(0.0), c), 1.0);
}
"#;

const HOLLOW_CUBE_FBM_WGSL: &str = r#"
struct FbmUniforms {
  time: f32,
  _pad: f32,
  resolution: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: FbmUniforms;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  // `_fbm(vec3(vUv.xy * vec2(10.0), gTime * 0.1), 3.0)`
  let v = vec3<f32>(p9_fbm(vec3<f32>(uv * vec2<f32>(10.0), u.time * 0.1), 3u));
  return vec4<f32>(v, 1.0);
}
"#;

const HOLLOW_CUBE_COMPOSITE_WGSL: &str = r#"
struct CompositeUniforms {
  resolution: vec2<f32>,
  _pad2: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: CompositeUniforms;
@group(0) @binding(1) var t_cube: texture_2d<f32>;
@group(0) @binding(2) var t_bloom: texture_2d<f32>;
@group(0) @binding(3) var t_fbm: texture_2d<f32>;
@group(0) @binding(4) var source_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let c1 = textureSample(t_bloom, source_sampler, uv);
  let c2 = textureSample(t_cube, source_sampler, uv);
  let bg = textureSample(t_fbm, source_sampler, uv);

  let col = mix(bg.rgb, p9_tonemap_aces(c2.rgb), c2.a)
    + p9_tonemap_aces(c1.rgb * c1.rgb);

  return vec4<f32>(col, c2.a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-hollow-cube.test.js.

    #[test]
    fn perm_starts_at_the_preset_seed_and_falls_back() {
        assert_eq!(make_perm(1.0), 1.0);
        assert_eq!(make_perm(4.0), 4.0);
        assert_eq!(make_perm(f32::NAN), 1.0);
    }

    #[test]
    fn drive_accumulates_perm_from_the_bass_band_and_the_creep() {
        let quiet = hollow_cube_drive(0.0, 0.0, 0.0, 0.1, 1.0);
        let loud = hollow_cube_drive(0.1, 0.0, 0.0, 0.1, 0.0);
        let louder = hollow_cube_drive(5.0, 0.0, 0.0, 0.1, 0.0);

        assert!((quiet.perm - (1.0 + 0.01 * 40.0 * 0.1)).abs() < 1e-6);
        assert!((loud.perm - (0.1 + 0.01) * 40.0 * 0.1).abs() < 1e-6);
        // min(0.5, band + 0.01) * 40 caps the advance at 20 units per second.
        assert!((louder.perm - 0.5 * 40.0 * 0.1).abs() < 1e-6);
    }

    #[test]
    fn drive_maps_the_damped_bass_lanes_to_in1_and_in2() {
        let drive = hollow_cube_drive(0.0, 0.1, 0.5, 0.1, 0.0);

        assert!((drive.in1[0] - (0.5_f32 * 2.2).min(1.2) * 0.6).abs() < 1e-6);
        assert!((drive.in1[1] - (0.1_f32 * 4.0 + 0.4).min(0.55)).abs() < 1e-6);
        assert!((drive.in1[2] - (0.1_f32 * 1.5).min(1.0)).abs() < 1e-6);
        assert!((drive.in2[0] - drive.perm).abs() < 1e-6);
        assert!((drive.in2[1] - (1.0 + 0.1_f32.min(0.2))).abs() < 1e-6);
    }

    #[test]
    fn drive_clamps_the_colour_size_and_rotation_terms() {
        let drive = hollow_cube_drive(10.0, 10.0, 10.0, 0.1, 0.0);

        assert!((drive.in1[0] - 0.72).abs() < 1e-6);
        assert!((drive.in1[1] - 0.55).abs() < 1e-6);
        assert!((drive.in1[2] - 1.0).abs() < 1e-6);
        assert!((drive.in2[1] - 1.2).abs() < 1e-6);
    }

    #[test]
    fn drive_sanitises_negative_non_finite_and_oversized_steps() {
        let negative = hollow_cube_drive(-1.0, -2.0, -3.0, -1.0, f32::NAN);
        let bad = hollow_cube_drive(f32::NAN, f32::NAN, f32::NAN, f32::NAN, f32::NAN);
        let long_frame = hollow_cube_drive(1.0, 1.0, 1.0, 10.0, 0.0);

        assert!((negative.perm - 1.0).abs() < 1e-6);
        assert_eq!(negative.in1, [0.0, 0.4, 0.0]);
        assert!((bad.perm - 1.0).abs() < 1e-6);
        // P9 deltatime is clamped, so a suspended tab cannot skip the pulse.
        assert!((long_frame.perm - 0.5 * 40.0 * 0.1).abs() < 1e-6);
    }

    #[test]
    fn lattice_is_the_presets_15000_instances_on_six_faces() {
        assert_eq!(INSTANCE_COUNT, 15_000);
        assert_eq!(LATTICE_COUNT, 50);
        assert_eq!(CUBE_SIZE, 0.01);

        let per_face = LATTICE_COUNT * LATTICE_COUNT;
        assert_eq!(hollow_cube_pos(0, INSTANCE_COUNT), [0.0, 0.0, 0.0]);
        assert_eq!(hollow_cube_pos(1, INSTANCE_COUNT), [0.0, 1.0 / 50.0, 0.0]);
        assert_eq!(hollow_cube_pos(per_face, INSTANCE_COUNT), [1.0, 0.0, 0.0]);
        assert_eq!(
            hollow_cube_pos(per_face * 4, INSTANCE_COUNT),
            [0.0, 0.0, 0.0]
        );
        assert_eq!(
            hollow_cube_pos(INSTANCE_COUNT - 1, INSTANCE_COUNT),
            [49.0 / 50.0, 49.0 / 50.0, 1.0]
        );
    }

    #[test]
    fn lattice_stays_on_the_shell() {
        for index in (0..INSTANCE_COUNT).step_by(137) {
            let pos = hollow_cube_pos(index, INSTANCE_COUNT);
            assert!(
                pos.iter().any(|value| *value == 0.0 || *value == 1.0),
                "instance {index} left the shell: {pos:?}"
            );
        }
    }

    #[test]
    fn clears_to_a_fully_transparent_background() {
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(HollowCube::new().clear_alpha(), 0.0);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(CAMERA_FOV, 50.0);
        assert_eq!((CAMERA_NEAR, CAMERA_FAR), (0.01, 100.0));
        assert_eq!(CAMERA_POSITION, [0.0, 0.0, 2.2]);
        assert_eq!(CAMERA_LOOK_AT, [0.0, 0.0, -1.0]);
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (0.51, 0.85, 0.0)
        );
        assert_eq!(BLOOM_SMOOTHNESS, 0.5);
        assert_eq!((FBM_SIZE, FBM_OCTAVES), (256, 3));
        assert_eq!(MAX_DT, 0.1);
    }

    #[test]
    fn update_consumes_the_props_and_the_bass_lanes() {
        let mut scene = HollowCube::new();
        scene.apply_props(|key| match key {
            "spinSpeed" => Some(2.0),
            "explode" => Some(0.5),
            "color" => Some(1.5),
            "bloom" => Some(3.0),
            _ => None,
        });

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 0.1; // damping 0, bass
        payload.lanes[3] = 0.2; // damping 1, bass
        payload.lanes[6] = 0.3; // damping 2, bass
        payload.byte_frequency_data = vec![0; 1024];
        payload.freq_mag = [0.4; crate::point10::audio_bridge::AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.1, &bridge);

        // `in1 = (min(1.2, 0.3*2.2)*0.6 * 0.5, min(0.55, 0.2*4+0.4) * 1.5,
        //         min(1, 0.2*1.5) * 2)`
        assert!((scene.in1()[0] - (0.3_f32 * 2.2).min(1.2) * 0.6 * 0.5).abs() < 1e-5);
        // `min(0.55, 0.2 * 4 + 0.4)` clamps *before* the colour prop scales it.
        assert!((scene.in1()[1] - 0.55 * 1.5).abs() < 1e-5);
        assert!((scene.in1()[2] - 0.3 * 2.0).abs() < 1e-5);
        assert!((scene.in2()[0] - scene.perm()).abs() < 1e-6);
        assert!((scene.in2()[1] - 1.2).abs() < 1e-6);
        // The fbm clock uses the raw frame delta.
        assert!((scene.time() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn time_is_capped_where_the_expression_caps_it() {
        let mut scene = HollowCube::new();
        scene.update(4.0, &AudioBridge::new());
        // `frame_dt` caps at 0.1 s; with no audio the reads are the 0.1
        // `nomusic` fallback, so `perm += min(0.5, 0.11) * 40 * 0.1`.
        assert!((scene.perm() - (1.0 + 0.11 * 40.0 * 0.1)).abs() < 1e-5);
    }

    #[test]
    fn uniform_blocks_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<CubeUniforms>(), 112);
        assert_eq!(std::mem::size_of::<FbmUniforms>(), 16);
        assert_eq!(std::mem::size_of::<CompositeUniforms>(), 16);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("cube", hollow_cube_cube_wgsl()),
            ("fbm", hollow_cube_fbm_wgsl()),
            ("composite", hollow_cube_composite_wgsl()),
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
    fn cube_shader_keeps_the_preset_placement_and_lighting() {
        let program = hollow_cube_cube_wgsl();
        for needle in [
            "@builtin(instance_index) instance: u32",
            "let instance_id = f32(instance);",
            "p9_mod(instance_id, obj_count)",
            "p9_mod(instance_id / obj_count, obj_count)",
            "p9_cubicpulse(p9_mod(p.x + p.y + p.z + t, 1.5), 0.6, 0.4)",
            "p9_axis_angle_to_quaternion(rotation.xyz, pulse * 6.0 * u.in1.z)",
            "sign(up.x)",
            "u.in2.y * xx",
            "0.1 + 0.9 * p9_rand2(vec2<f32>(instance_id))",
            "let specular_hardness = 10.0;",
            "vec3<f32>(0.5, 0.8, 1.0) * 1.15",
            "vec3<f32>(0.8, 0.0, 0.0)",
        ] {
            assert!(program.contains(needle), "cube shader lost {needle}");
        }
        assert!(!program.contains("gl_InstanceID"));
    }

    #[test]
    fn fbm_and_composite_shaders_keep_the_preset_maths() {
        let fbm = hollow_cube_fbm_wgsl();
        assert!(fbm.contains("p9_fbm(vec3<f32>(uv * vec2<f32>(10.0), u.time * 0.1), 3u)"));
        assert!(fbm.contains("vec4<f32>(v, 1.0)"));

        let composite = hollow_cube_composite_wgsl();
        for needle in [
            "mix(bg.rgb, p9_tonemap_aces(c2.rgb), c2.a)",
            "p9_tonemap_aces(c1.rgb * c1.rgb)",
            "return vec4<f32>(col, c2.a)",
        ] {
            assert!(composite.contains(needle), "composite lost {needle}");
        }
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = HollowCube::new();
        assert_eq!(scene.meta().name, "Point10 Hollow Cube");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 4..],
            &["spinSpeed", "explode", "color", "bloom"]
        );
        assert_eq!(names[0], "gain");
    }
}
