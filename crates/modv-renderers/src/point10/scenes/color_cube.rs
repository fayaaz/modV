//! ColorCube — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/ColorCube.js`.
//!
//! The Plane9 "Color Cube" preset (Joakim Dahl, CC BY-NC-SA 3.0): an 8x8x8
//! (512) grid of 0.05 cubes. Each clone is positioned from its 1-based grid
//! coordinates plus an audio `dist` spread, coloured by the same coordinates
//! (`r = x/8`, `g = y/8`, `b = 1 - z/8`), and lit by `Shader1`'s directional +
//! point lights. Expression1 advances a `perm` phase from the beat envelope and
//! feeds `CloneExpression1.In1`, so every cube wobbles on the sine terms as the
//! grid spreads; `Rotator1` spins the whole object at 1.5/1/0.5 deg/s.
//!
//! # Ported exactly
//!
//! * `CloneExpression1` (`:84-195`): the 1-based grid walk (`z` slowest, then
//!   `y`, then `x`), [`clone_position`] (`x * spread * 0.05 + sin(x - y - z +
//!   perm) * dist * 0.025`, and the `+`, `(x+1)(y+1)(z+1)` variants), the
//!   colour ramp, and the bounding-box centre subtraction that puts the grid on
//!   the origin;
//! * Expression1 + Shader1's Expression (`:206-235`): `perm += (beat * 50 *
//!   turbulence + 0.01) * dt` with the 0.05 s cap Plane9's expression runtime
//!   uses, `dist = (0.3 + bass) * bassSmooth * 20`, `discard = min(0.15,
//!   bassSmooth * 10) * turbulence`, `in1 = (discard, 14.945, -3.827)`;
//! * the shaders (`:237-318`): the instanced vertex transform, the
//!   `_turbulencefast(world * 2, 5) < in1.x` discard, `_lightDirectional` with
//!   the `normalize(lightPosition - world)` direction and `vec3(1) * 2 *
//!   brightness`, the origin point light `(1, 0.5, 0.3) * 50` with the
//!   `pow(saturate(1 - pow(d/100, 4)), 2) / (d² + 1)` attenuation, `roughness
//!   0.4`, `metallic 0`, and `light + realAlbedo * gIn3.x`;
//! * the render chain (`:373-388`): RenderPass → UnrealBloomPass `(0.5, 0.85,
//!   0)` wrapped in `preserveBloomAlpha` → OutputPass, `NoToneMapping`, clear
//!   alpha 0;
//! * the camera (`:42-48`): `PerspectiveCamera(70, aspect, 0.01, 100)`, position
//!   `(0, 0, -1.17)` looking at `(0, 0, 1)`;
//! * `Rotator1` (`:50-51`) at 1.5/1/0.5 deg/s, scaled by `spinSpeed`;
//! * the prop binder (`sceneProps.js:318-323`).
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `InstancedMesh` + `instanceMatrix` (512 matrices) | one instance buffer (`position`, `colour`) and `+ instance_position` in the vertex shader |
//! | `setMatrixAt` per frame | [`compute_clone_grid`] rewrites the 512 positions |
//! | `InstancedBufferAttribute aColor` | the same instance buffer's colour half |
//! | `BoxGeometry(0.05)` | [`build_box`](crate::point10::scenes::cube_mesh::build_box) |
//! | `_turbulencefast`, `_lightDirectional` | [`P9_COMMON_WGSL`] + [`P9_SHADING_WGSL`] |
//! | `EffectComposer` + `preserveBloomAlpha` | [`Composer`] with `set_preserve_bloom_alpha(true)` |
//!
//! # Deviations
//!
//! * **Bloom alpha.** `preserveBloomAlpha` is imported from `Boxy.js` and
//!   applied here too, so the additive bloom cannot turn the transparent clear
//!   into an opaque veil; the port calls
//!   [`Composer::set_preserve_bloom_alpha(true)`] exactly like `Boxy`.
//! * three uploads a 4x4 instance matrix per cube; the port uploads the
//!   translation and the colour, because `CloneExpression1` never rotates or
//!   scales an instance (`_dummy.rotation.set(0, 0, 0)`,
//!   `_dummy.scale.set(1, 1, 1)`). Same transform, a quarter of the bandwidth.
//! * **Culling.** `FrontSide` becomes `cull_mode: None` plus a depth test (see
//!   `cube_mesh`); with 512 opaque cubes the visible surface is identical.
//! * The colour attribute is written once: `cloneColor` depends only on the
//!   grid coordinates, and the JS rewrites the identical values every frame.
//! * `P9Audio` is `plane9/P9Audio.js` in the JS; the port reads [`P10Audio`]
//!   over the shared [`AudioBridge`] like every other ported scene.
//!
//! [`Composer::set_preserve_bloom_alpha(true)`]: crate::point10::postfx::Composer::set_preserve_bloom_alpha

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, create_depth, depth_state, euler_xyz, look_at, mat4_mul, perspective,
};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::p9_shading::P9_SHADING_WGSL;
use crate::point10::{
    HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`ColorCube.js:36-73`)
// ---------------------------------------------------------------------------

/// `GRID_SIZE` — `CloneExpression1` NumObjectsX/Y/Z.
pub const GRID_SIZE: usize = 8;
/// `INSTANCE_COUNT`.
pub const INSTANCE_COUNT: usize = GRID_SIZE * GRID_SIZE * GRID_SIZE;
/// `CUBE_SIZE`.
pub const CUBE_SIZE: f32 = 0.05;

/// `CAMERA.fov`.
pub const CAMERA_FOV: f32 = 70.0;
/// `CAMERA.near`.
pub const CAMERA_NEAR: f32 = 0.01;
/// `CAMERA.far`.
pub const CAMERA_FAR: f32 = 100.0;
/// `CAMERA.position`.
pub const CAMERA_POSITION: [f32; 3] = [0.0, 0.0, -1.17];
/// `CAMERA.lookAt`.
pub const CAMERA_LOOK_AT: [f32; 3] = [0.0, 0.0, 1.0];

/// `ROTATOR`, degrees per second.
pub const ROTATOR: [f32; 3] = [1.5, 1.0, 0.5];

/// `BLOOM.strength`.
pub const BLOOM_STRENGTH: f32 = 0.5;
/// `BLOOM.radius`.
pub const BLOOM_RADIUS: f32 = 0.85;
/// `BLOOM.threshold`.
pub const BLOOM_THRESHOLD: f32 = 0.0;

/// `CLEAR_ALPHA` — the scene composites transparently.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `LIGHT_POSITION` (`In1.y/z`; `In1.x` becomes the discard threshold).
pub const LIGHT_POSITION: [f32; 3] = [0.0, 14.945, -3.827];

/// `DISCARD_MAX` — Shader1's `in1.x = min(0.15, band * 10)`.
pub const DISCARD_MAX: f32 = 0.15;

/// `MAX_DT` — Expression1 caps elapsed time at 0.05 s.
pub const MAX_DT: f32 = 0.05;

/// `GRID_STEP`.
pub const GRID_STEP: f32 = 0.05;
/// `SINE_SCALE`.
pub const SINE_SCALE: f32 = 0.025;

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn non_negative(value: f32) -> f32 {
    finite_or(value, 0.0).max(0.0)
}

/// `makePerm(seed = 1)`.
pub fn make_perm(seed: f32) -> f32 {
    finite_or(seed, 1.0)
}

/// `writePosition` — the `CloneExpression1` position of one 1-based coordinate.
fn write_position(x: f32, y: f32, z: f32, dist: f32, perm: f32) -> [f32; 3] {
    let spread = 1.0 + dist;
    let wobble = dist * SINE_SCALE;

    [
        x * spread * GRID_STEP + (x - y - z + perm).sin() * wobble,
        y * spread * GRID_STEP + (x + y + z + perm).sin() * wobble,
        z * spread * GRID_STEP + ((x + 1.0) * (y + 1.0) * (z + 1.0) + perm).sin() * wobble,
    ]
}

/// Pure `CloneExpression1` position for one 1-based grid coordinate.
///
/// `dist` is the audio spread, `perm` the Expression1 phase feeding `In1`.
pub fn clone_position(x: f32, y: f32, z: f32, dist: f32, perm: f32) -> [f32; 3] {
    write_position(x, y, z, finite_or(dist, 0.0), finite_or(perm, 0.0))
}

/// `CloneExpression1` colour: `r = x/nx`, `g = y/ny`, `b = 1 - z/nz`.
pub fn clone_color(x: f32, y: f32, z: f32) -> [f32; 3] {
    [
        x / GRID_SIZE as f32,
        y / GRID_SIZE as f32,
        1.0 - z / GRID_SIZE as f32,
    ]
}

/// `computeCloneGrid(perm, dist)`'s result.
#[derive(Clone, Debug, PartialEq)]
pub struct CloneGrid {
    /// The centred instance positions.
    pub positions: Vec<[f32; 3]>,
    /// The per-instance colours (constant for a given grid).
    pub colors: Vec<[f32; 3]>,
    /// The bounding-box midpoint that was subtracted.
    pub center: [f32; 3],
}

/// Builds the 512 clone positions and colours.
///
/// Coordinates are 1-based, matching P9Nodes' `CloneExpression`; `Center: true`
/// then subtracts the bounding-box midpoint, so the grid sits on the origin.
pub fn compute_clone_grid(perm: f32, dist: f32) -> CloneGrid {
    let perm = finite_or(perm, 0.0);
    let dist = finite_or(dist, 0.0);
    let mut positions = vec![[0.0_f32; 3]; INSTANCE_COUNT];
    let mut colors = vec![[0.0_f32; 3]; INSTANCE_COUNT];
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for (index, (position, color)) in positions.iter_mut().zip(colors.iter_mut()).enumerate() {
        let z = (index / (GRID_SIZE * GRID_SIZE)) + 1;
        let row = index % (GRID_SIZE * GRID_SIZE);
        let x = (row % GRID_SIZE) + 1;
        let y = (row / GRID_SIZE) + 1;

        let p = write_position(x as f32, y as f32, z as f32, dist, perm);
        *position = p;
        *color = clone_color(x as f32, y as f32, z as f32);

        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    for position in &mut positions {
        for axis in 0..3 {
            position[axis] -= center[axis];
        }
    }

    CloneGrid {
        positions,
        colors,
        center,
    }
}

/// One frame of Expression1 + Shader1's Expression.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorCubeDrive {
    /// `this.perm` for the next frame.
    pub perm: f32,
    /// `CloneExpression1.In1` — the audio spread.
    pub dist: f32,
    /// Shader1's turbulence discard threshold.
    pub discard: f32,
    /// `gIn1 = (discard, lightY, lightZ)`.
    pub in1: [f32; 3],
}

/// Pure frame drive (`ColorCube.js:206-235`).
pub fn color_cube_drive(
    bass: f32,
    bass_smooth: f32,
    beat: f32,
    dt: f32,
    perm: f32,
    turbulence_scale: f32,
) -> ColorCubeDrive {
    let bass = non_negative(bass);
    let bass_smooth = non_negative(bass_smooth);
    let beat = non_negative(beat).min(1.0);
    let step = non_negative(dt).min(MAX_DT);
    let turbulence = finite_or(turbulence_scale, 1.0);
    let scale = turbulence.max(0.0);
    let base = finite_or(perm, 1.0);
    let next = base + (beat * 50.0 * scale + 0.01) * step;
    let dist = (0.3 + bass) * bass_smooth * 20.0;
    let discard = (bass_smooth * 10.0).min(DISCARD_MAX) * scale;

    ColorCubeDrive {
        perm: next,
        dist,
        discard,
        in1: [discard, LIGHT_POSITION[1], LIGHT_POSITION[2]],
    }
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `COLOR_CUBE_PROPS` (`sceneProps.js:179-184`).
const COLOR_CUBE_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("turbulence", "Turbulence", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

/// One instance: translation + colour (24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    position: [f32; 3],
    color: [f32; 3],
}

impl Instance {
    /// Buffer layout for `@location(4)`/`@location(5)`, one step per instance.
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
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 5,
                },
            ],
        }
    }
}

/// The uniform block (176 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorCubeUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// x discard, y/z light position.
    in1: [f32; 4],
    /// x brightness, y `gIn3.x`.
    params: [f32; 4],
}

/// GPU resources built in `init`.
struct ColorCubeGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    instances: wgpu::Buffer,
    mesh: MeshGpu,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    composer: Composer,
}

/// ColorCube scene.
pub struct ColorCube {
    audio: P10Audio,
    /// `this.perm`.
    perm: f32,
    /// `this.spinTime`.
    spin_time: f32,
    /// The last computed clone positions (kept so `render` can re-upload after
    /// a resize without recomputing the grid).
    positions: Vec<[f32; 3]>,
    spin_scale: f32,
    turbulence_scale: f32,
    bloom_scale: f32,
    brightness_scale: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<ColorCubeGpu>,
}

impl Default for ColorCube {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorCube {
    /// Create the scene (`ColorCube.js:320-392` defaults).
    pub fn new() -> Self {
        let grid = compute_clone_grid(1.0, 0.0);
        Self {
            audio: P10Audio::new(),
            perm: make_perm(1.0),
            spin_time: 0.0,
            positions: grid.positions,
            spin_scale: 1.0,
            turbulence_scale: 1.0,
            bloom_scale: 1.0,
            brightness_scale: 1.0,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:318-323`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(COLOR_CUBE_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_scale = value,
                "turbulence" => self.turbulence_scale = value,
                "bloom" => self.bloom_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// `this.perm`.
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// `this.spinTime`.
    pub fn spin_time(&self) -> f32 {
        self.spin_time
    }

    /// The instance positions as last computed.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
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

    /// `mesh.rotation.set(spinTime * ROTATOR * DEG)`.
    fn model(&self) -> Mat4 {
        euler_xyz(
            (self.spin_time * ROTATOR[0]).to_radians(),
            (self.spin_time * ROTATOR[1]).to_radians(),
            (self.spin_time * ROTATOR[2]).to_radians(),
        )
    }

    fn upload(&self, in1: [f32; 4]) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.uniforms,
            0,
            bytemuck::bytes_of(&ColorCubeUniforms {
                view_proj: self.view_proj(),
                model: self.model(),
                camera_pos: [
                    CAMERA_POSITION[0],
                    CAMERA_POSITION[1],
                    CAMERA_POSITION[2],
                    1.0,
                ],
                in1,
                // `gIn3 = (0, 10, 0, 0)`: only x reaches the shader.
                params: [self.brightness_scale, 0.0, 0.0, 0.0],
            }),
        );
        gpu.queue
            .write_buffer(&gpu.instances, 0, bytemuck::cast_slice(&self.instances()));
    }

    /// The interleaved instance buffer contents.
    fn instances(&self) -> Vec<Instance> {
        (0..INSTANCE_COUNT)
            .map(|index| {
                let z = (index / (GRID_SIZE * GRID_SIZE)) + 1;
                let row = index % (GRID_SIZE * GRID_SIZE);
                let x = (row % GRID_SIZE) + 1;
                let y = (row / GRID_SIZE) + 1;
                Instance {
                    position: self.positions[index],
                    color: clone_color(x as f32, y as f32, z as f32),
                }
            })
            .collect()
    }
}

impl Point10Scene for ColorCube {
    fn name(&self) -> &'static str {
        "Point10 Color Cube"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        use wgpu::util::DeviceExt;

        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);
        self.camera_aspect = ctx.aspect();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-color-cube-shader"),
            source: wgpu::ShaderSource::Wgsl(color_cube_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-color-cube-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-color-cube-uniforms"),
            size: std::mem::size_of::<ColorCubeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-color-cube-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-color-cube-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-color-cube-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(crate::point10::scenes::cube_mesh::Vertex::layout()),
                    Some(Instance::layout()),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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

        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point10-color-cube-instances"),
            contents: bytemuck::cast_slice(&self.instances()),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let (depth, depth_view) = create_depth(
            &device,
            "point10-color-cube-depth",
            self.size.0,
            self.size.1,
        );

        // `EffectComposer`: RenderPass -> preserveBloomAlpha(UnrealBloomPass)
        // -> OutputPass, NoToneMapping.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.enable_bloom(BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD);
        composer.set_preserve_bloom_alpha(true);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        let mesh = MeshGpu::new(
            &device,
            "point10-color-cube-mesh",
            &build_box(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
        );

        self.gpu = Some(ColorCubeGpu {
            device,
            queue,
            pipeline,
            bind_group,
            uniforms,
            instances,
            mesh,
            depth,
            depth_view,
            composer,
        });
        self.upload([DISCARD_MAX, LIGHT_POSITION[1], LIGHT_POSITION[2], 0.0]);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.resize(self.size.0, self.size.1);
            let (depth, depth_view) = create_depth(
                &gpu.device,
                "point10-color-cube-depth",
                self.size.0,
                self.size.1,
            );
            gpu.depth = depth;
            gpu.depth_view = depth_view;
        }
        self.upload([DISCARD_MAX, LIGHT_POSITION[1], LIGHT_POSITION[2], 0.0]);
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.audio.update(dt, audio);

        let drive = color_cube_drive(
            self.audio.band(-1, 1, 0, 0.0),
            self.audio.band(-1, 2, 0, 0.05),
            self.audio.beat(0.05),
            dt,
            self.perm,
            self.turbulence_scale,
        );
        self.perm = drive.perm;

        // `this.spinTime += nonNegative(dt) * spinScale` — the rotator is not
        // clamped to MAX_DT in the JS, only the drive is.
        self.spin_time += dt.max(0.0) * self.spin_scale;

        let grid = compute_clone_grid(self.perm, drive.dist);
        self.positions = grid.positions;

        self.upload([drive.in1[0], drive.in1[1], drive.in1[2], 0.0]);
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        if let Some(bloom) = gpu.composer.bloom_mut() {
            bloom.set_strength(BLOOM_STRENGTH * self.bloom_scale);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-color-cube"),
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
            pass.set_vertex_buffer(1, gpu.instances.slice(..));
            gpu.mesh.draw_instanced(&mut pass, INSTANCE_COUNT as u32);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(COLOR_CUBE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10ColorCube.js` sets no category.
        SceneMeta::for_scene("Point10 Color Cube", "3d-scenes")
    }
}

/// `buildColorCubeVertexShader` + `buildColorCubeFragmentShader` with the
/// Plane9 helpers in front.
fn color_cube_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{P9_SHADING_WGSL}\n{COLOR_CUBE_WGSL}")
}

const COLOR_CUBE_WGSL: &str = r#"
struct ColorCubeUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // x discard, y/z light position
  in1: vec4<f32>,
  // x brightness, y gIn3.x
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: ColorCubeUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) diffuse: vec4<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) view_dir: vec3<f32>,
  @location(3) world: vec3<f32>,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
  @location(4) instance_position: vec3<f32>,
  @location(5) instance_color: vec3<f32>,
) -> VsOut {
  // `modelMatrix * instanceMatrix * local`: CloneExpression1 only ever
  // translates an instance, so the matrix is `position + instance_position`.
  let local = vec4<f32>(position + instance_position, 1.0);
  let world = u.model * local;

  var out: VsOut;
  out.position = u.view_proj * world;
  out.diffuse = vec4<f32>(instance_color, 1.0);
  out.normal = normalize((u.model * vec4<f32>(normal, 0.0)).xyz);
  out.view_dir = normalize(u.camera_pos.xyz - world.xyz);
  out.world = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(in.view_dir);
  let light_position = u.in1.yzw;
  let albedo = in.diffuse.rgb;
  let roughness = 0.4;
  let metallic = 0.0;

  if (p9_turbulence(in.world * 2.0, 5u) < u.in1.x) {
    discard;
  }

  let real_albedo = albedo - albedo * vec3<f32>(metallic);
  let real_specular = mix(vec3<f32>(0.04), albedo, vec3<f32>(metallic));

  // Shader1's directional light: colour (1,1,1) * 2 * brightness, direction
  // from the packed In1 y/z light position.
  var light = p9_light_directional(
    real_albedo,
    real_specular,
    normal,
    roughness,
    vec3<f32>(1.0) * 2.0 * u.params.x,
    normalize(light_position - in.world),
    view_dir,
  );

  // The point light at the origin: `_lightPoint` inlined from Plane9's shader
  // library, colour (1, 0.5, 0.3) * 50, radius 100.
  let pixel_to_light = -in.world;
  let light_dist = length(pixel_to_light);
  let point_dir = pixel_to_light / max(light_dist, 1e-6);
  let attenuation = pow(p9_saturate(1.0 - pow(light_dist / 100.0, 4.0)), 2.0)
    / ((light_dist * light_dist) + 1.0);
  light = light + p9_light_directional(
    real_albedo,
    real_specular,
    normal,
    roughness,
    vec3<f32>(1.0, 0.5, 0.3) * 50.0 * u.params.x,
    point_dir,
    view_dir,
  ) * attenuation;

  let c = light + real_albedo * u.params.y;
  return vec4<f32>(c, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-color-cube.test.js.

    #[test]
    fn clone_position_matches_the_expression() {
        // No spread, no phase: the plain grid step.
        let base = clone_position(2.0, 3.0, 4.0, 0.0, 0.0);
        assert!((base[0] - 2.0 * GRID_STEP).abs() < 1e-6);
        assert!((base[1] - 3.0 * GRID_STEP).abs() < 1e-6);
        assert!((base[2] - 4.0 * GRID_STEP).abs() < 1e-6);

        // With spread: `x * (1 + dist) * 0.05 + sin(...) * dist * 0.025`.
        let dist = 2.0;
        let perm = 0.7;
        let spread = clone_position(2.0, 3.0, 4.0, dist, perm);
        assert!(
            (spread[0]
                - (2.0 * 3.0 * GRID_STEP + (2.0 - 3.0 - 4.0 + perm).sin() * dist * SINE_SCALE))
                .abs()
                < 1e-5
        );
        assert!(
            (spread[2]
                - (4.0 * 3.0 * GRID_STEP + (3.0 * 4.0 * 5.0 + perm).sin() * dist * SINE_SCALE))
                .abs()
                < 1e-5
        );

        // Bad input falls back to the plain grid.
        let bad = clone_position(1.0, 1.0, 1.0, f32::NAN, f32::NAN);
        assert!((bad[0] - GRID_STEP).abs() < 1e-6);
    }

    #[test]
    fn clone_colour_is_the_normalised_grid_ramp() {
        assert_eq!(clone_color(1.0, 1.0, 1.0), [0.125, 0.125, 0.875]);
        assert_eq!(clone_color(8.0, 8.0, 8.0), [1.0, 1.0, 0.0]);
    }

    #[test]
    fn clone_grid_has_512_centred_instances() {
        let grid = compute_clone_grid(0.0, 0.0);
        assert_eq!(grid.positions.len(), INSTANCE_COUNT);
        assert_eq!(grid.colors.len(), INSTANCE_COUNT);

        // Every axis is symmetric about the origin after the centring.
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for position in &grid.positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        for axis in 0..3 {
            assert!((min[axis] + max[axis]).abs() < 1e-5, "{min:?} / {max:?}");
        }
        // The unspread grid spans 1..8 * 0.05 = 0.35 with a 0.05 cube.
        assert!((max[0] - min[0] - 0.35).abs() < 1e-5);

        // The grid is ordered z-slowest, then y, then x.
        assert_eq!(grid.colors[0], clone_color(1.0, 1.0, 1.0));
        assert_eq!(grid.colors[1], clone_color(2.0, 1.0, 1.0));
        assert_eq!(grid.colors[8], clone_color(1.0, 2.0, 1.0));
        assert_eq!(grid.colors[64], clone_color(1.0, 1.0, 2.0));
    }

    #[test]
    fn clone_grid_spreads_with_dist() {
        let tight = compute_clone_grid(0.0, 0.0);
        let spread = compute_clone_grid(0.0, 1.0);
        let extent = |grid: &CloneGrid| {
            grid.positions
                .iter()
                .map(|position| position[0].abs())
                .fold(0.0_f32, f32::max)
        };
        assert!(extent(&spread) > extent(&tight) * 1.5);
    }

    #[test]
    fn drive_advances_perm_dist_and_discard() {
        let drive = color_cube_drive(0.5, 0.2, 0.5, 0.05, 1.0, 1.0);

        assert!((drive.perm - (1.0 + (0.5 * 50.0 + 0.01) * 0.05)).abs() < 1e-5);
        assert!((drive.dist - (0.3 + 0.5) * 0.2 * 20.0).abs() < 1e-5);
        assert!((drive.discard - 0.15).abs() < 1e-6);
        assert_eq!(drive.in1, [drive.discard, 14.945, -3.827]);
    }

    #[test]
    fn drive_caps_the_step_at_fifty_milliseconds() {
        let capped = color_cube_drive(0.0, 0.0, 1.0, 10.0, 1.0, 1.0);
        assert!((capped.perm - (1.0 + (50.0 + 0.01) * MAX_DT)).abs() < 1e-5);
    }

    #[test]
    fn drive_scales_perm_and_discard_with_turbulence() {
        let half = color_cube_drive(0.0, 0.3, 1.0, 0.05, 1.0, 0.5);
        assert!((half.perm - (1.0 + (50.0 * 0.5 + 0.01) * 0.05)).abs() < 1e-5);
        // `min(DISCARD_MAX, bs * 10) * scale`; bs * 10 = 3 is above the cap.
        assert!((half.discard - DISCARD_MAX * 0.5).abs() < 1e-5);

        let zero = color_cube_drive(0.0, 1.0, 1.0, 0.05, 1.0, 0.0);
        assert_eq!(zero.discard, 0.0);
        // `dist` is amplitude-driven, not turbulence-driven: (0.3 + 0) * 1 * 20.
        assert_eq!(zero.dist, 6.0);
    }

    #[test]
    fn drive_sanitises_bad_input() {
        let bad = color_cube_drive(f32::NAN, f32::NAN, f32::NAN, f32::NAN, f32::NAN, f32::NAN);
        assert_eq!(bad.perm, 1.0);
        assert_eq!(bad.dist, 0.0);
        assert_eq!(bad.discard, 0.0);
        assert!(bad.in1.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn discard_scales_with_turbulence_after_the_cap() {
        let drive = color_cube_drive(1.0, 10.0, 1.0, 0.05, 1.0, 3.0);
        assert!((drive.discard - DISCARD_MAX * 3.0).abs() < 1e-6);
    }

    #[test]
    fn perm_starts_at_one_and_falls_back() {
        assert_eq!(make_perm(1.0), 1.0);
        assert_eq!(make_perm(4.0), 4.0);
        assert_eq!(make_perm(f32::NAN), 1.0);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(GRID_SIZE, 8);
        assert_eq!(INSTANCE_COUNT, 512);
        assert_eq!(CUBE_SIZE, 0.05);
        assert_eq!(CAMERA_FOV, 70.0);
        assert_eq!(CAMERA_POSITION, [0.0, 0.0, -1.17]);
        assert_eq!(ROTATOR, [1.5, 1.0, 0.5]);
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (0.5, 0.85, 0.0)
        );
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(DISCARD_MAX, 0.15);
        assert_eq!(MAX_DT, 0.05);
        assert_eq!(GRID_STEP, 0.05);
        assert_eq!(SINE_SCALE, 0.025);
    }

    #[test]
    fn update_spins_and_respreads_the_grid() {
        let mut scene = ColorCube::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 0.5; // bass
        payload.lanes[6] = 0.25; // damping lane 2 of the bass band
        payload.beat = 0.4;
        payload.byte_frequency_data = vec![0; 1024];
        payload.freq_mag = [0.5; crate::point10::audio_bridge::AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let start_perm = scene.perm();
        let start_positions = scene.positions().to_vec();
        scene.update(0.05, &bridge);
        assert!(scene.perm() > start_perm);
        assert!((scene.spin_time() - 0.05).abs() < 1e-6);
        assert_eq!(scene.positions().len(), INSTANCE_COUNT);
        // The audio spread re-lays the grid out.
        assert_ne!(scene.positions(), &start_positions[..]);
    }

    #[test]
    fn spin_speed_scales_the_rotator_only() {
        let mut scene = ColorCube::new();
        scene.apply_props(|key| (key == "spinSpeed").then_some(2.0));
        scene.update(0.1, &AudioBridge::new());
        assert!((scene.spin_time() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = ColorCube::new();
        scene.apply_props(|key| match key {
            "spinSpeed" => Some(9.0),
            "turbulence" => Some(-1.0),
            "bloom" => Some(2.0),
            "brightness" => Some(1.5),
            _ => None,
        });
        assert_eq!(scene.spin_scale, 4.0);
        assert_eq!(scene.turbulence_scale, 0.0);
        assert_eq!(scene.bloom_scale, 2.0);
        assert_eq!(scene.brightness_scale, 1.5);
    }

    #[test]
    fn resize_tracks_the_camera_aspect() {
        let mut scene = ColorCube::new();
        scene.resize(800, 400);
        assert!((scene.camera_aspect - 2.0).abs() < 1e-6);
        scene.resize(0, 0);
        assert_eq!(scene.size, (1, 1));
        assert!((scene.camera_aspect - 1.0).abs() < 1e-6);
    }

    #[test]
    fn uniforms_and_instances_are_the_documented_size() {
        assert_eq!(std::mem::size_of::<ColorCubeUniforms>(), 176);
        assert_eq!(std::mem::size_of::<Instance>(), 24);
        assert_eq!(
            Instance::layout().array_stride,
            std::mem::size_of::<Instance>() as u64
        );
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = color_cube_wgsl();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("COLOR_CUBE_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("COLOR_CUBE_WGSL must validate");
    }

    #[test]
    fn shader_keeps_the_preset_maths() {
        let program = color_cube_wgsl();
        for needle in [
            "p9_turbulence(in.world * 2.0, 5u) < u.in1.x",
            "let roughness = 0.4;",
            "let metallic = 0.0;",
            "vec3<f32>(1.0) * 2.0 * u.params.x",
            "vec3<f32>(1.0, 0.5, 0.3) * 50.0 * u.params.x",
            "pow(p9_saturate(1.0 - pow(light_dist / 100.0, 4.0)), 2.0)",
            "let c = light + real_albedo * u.params.y;",
            "let local = vec4<f32>(position + instance_position, 1.0);",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = ColorCube::new();
        assert_eq!(scene.meta().name, "Point10 Color Cube");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 4..],
            &["spinSpeed", "turbulence", "bloom", "brightness"]
        );
        assert_eq!(names[0], "gain");
    }
}
