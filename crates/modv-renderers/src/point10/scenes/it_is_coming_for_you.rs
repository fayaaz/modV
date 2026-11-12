//! It Is Coming For You — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/ItIsComingForYou.js`.
//!
//! # What the JS actually draws
//!
//! The Plane9 "It Is Coming For You" preset (Joakim Dahl, CC BY-NC-SA 3.0):
//! 30 000 alpha-blended point sprites. Every point is a noise-driven cloud
//! position (`_noise(vec3(d * 2 + t, 0.12/1.23, 0))`) rotated by an axis-angle
//! quaternion around z, coloured from an HSV ramp, lit by two world-space
//! lamps, and given a depth-ring soft edge. The points render into
//! `RenderToTexture1`; `Bloom1` blurs the capture (strength 1, radius 0.85,
//! threshold 0.5) and `OutputPass` presents it with `NoToneMapping`.
//!
//! # The transparent background (explicit behaviour-parity requirement)
//!
//! The preset's background is fully transparent so the cloud composites over
//! whatever is beneath it (`ItIsComingForYou.js:10-13, 51-52, 243`):
//!
//! ```js
//! export const CLEAR_ALPHA = 0;
//! this.renderer.setClearColor(0x000000, CLEAR_ALPHA);
//! new RenderPass(this.group, this.camera, null, null, CLEAR_ALPHA);
//! ```
//!
//! The points themselves write `gl_FragColor = vec4(c * 5.0, blur)` with
//! `blur = smoothstep(ring + amount, ring - amount, length(p))`, so the centre
//! of a point is opaque and the rim fades out. The port keeps both halves:
//!
//! * the scene render pass clears to **transparent black**
//!   (`LoadOp::Clear(wgpu::Color::TRANSPARENT)`), never to the preset's Clear2
//!   colour (the Clear1 -> Shader3/Blur/Copy/Store chain is not connected to
//!   Screen1, so it is invisible and deliberately not implemented);
//! * the fragment returns `vec4(c * 5.0, blur)`, and the pipeline is normal
//!   alpha blending (`SrcAlpha`, `OneMinusSrcAlpha`), so empty pixels stay at
//!   alpha 0;
//! * the composer's bloom combine adds the bloom colour and alpha, but the
//!   bloom of an empty pixel is 0, so the background survives as alpha 0.
//!
//! A lavapipe test (`scene_tests::it_is_coming_for_you_keeps_the_background_transparent`)
//! asserts exactly that: some pixels are alpha 0 while the object pixels are
//! opaque.
//!
//! # Ported exactly
//!
//! * the point shader (`ItIsComingForYou.js:100-187`): `d = aInstance / 30000`,
//!   `t = gIn1.z * 0.1`, the two `_noise` reads, `gl_PointSize = 50 +
//!   _rand(d * 2) * 10`, `_axisAngleToQuaternion(vec3(0,0,1), id * 0.01 +
//!   gIn1.z)`, `_rotateVector`, the HSV ramp `* 2 * smoothstep(0.985, 1,
//!   rand) * gIn1.x` plus the `smoothstep(0.99, 1, sin(d * 20 + gTime * 4)) *
//!   gIn1.y` flush term and the `smoothstep(0.991, 0.99, d)` fade, `vDiffuse.a
//!   = d`, the two `_lightLambert` lamps, `_cubicpulse` depth ring and `blur`;
//! * the camera (`CAMERA = { fov: 45, near: 0.1, far: 10, position: [0, 0,
//!   -2.73], lookAt: [0, 0, 1] }`) and the preset bloom
//!   (`{ strength: 1, radius: 0.85, threshold: 0.5 }`) with `NoToneMapping`;
//! * Expression1 (`itIsComingDrive`, `:78-94`):
//!   `perm += (band(-1,0,0,0.1) * 20 + 0.1) * dt` (dt clamped to 0.1 s),
//!   `gIn1 = (min(mid * 2, 2) * 2, min(bass * 5, 1) * 0.25, perm + PERMRAND *
//!   10, 0)`, `gIn2 = (0.222, 0.087, 0)`; `PERMRAND = seededRand(18, 8)`;
//! * the prop binder (`sceneProps.js:389-395`): `speedScale = speed`,
//!   `glowScale = glow` (scales both `gIn1.x` and `gIn1.y`), `bloomScale =
//!   bloom`;
//! * `gTime = BaseScene.time` (`:306`).
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `THREE.Points`, `gl_PointSize`, `gl_PointCoord` | instanced billboard quads ([`UnitQuad`]) sized 50–60 px |
//! | `geometry.setAttribute("aInstance", …)` | `@builtin(instance_index)` |
//! | `projectionMatrix * modelViewMatrix` | the camera basis inlined: `ndc.x = -x * f / (z + 2.73)` (the look-at rotates the camera 180° about y) |
//! | `normalMatrix * normal` mixed with world-space `cameraPosition`/lamps (a JS quirk) | reproduced literally: the normal is the view-rotated world normal, the lamp directions and view direction stay world space |
//! | `EffectComposer` + `RenderPass` + `UnrealBloomPass` + `OutputPass` | [`Composer`] with `enable_bloom` and the default `ToneMapping::None` |
//! | `_noise`, `_rand`, `_hsv2rgb`, `_cubicpulse`, `_lightLambert`, quaternion helpers | [`P9_COMMON_WGSL`] |
//!
//! # Deviations
//!
//! * The point sprite is a quad with a radial `blur` mask instead of a hardware
//!   point. Same footprint (the mask is exactly what `gl_PointCoord` gives),
//!   same edge falloff.
//! * OutputPass in three sRGB-encodes the composer's linear buffer; the Rust
//!   [`Composer`] does the same (shader-side for a UNORM destination,
//!   hardware-side for an `*Srgb` one).
//! * No depth attachment (the JS disables depth test/write too).

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp, UnitQuad, read_scene_props,
    scene_props_with_levers,
};

/// `ItIsComingForYou.js: export const INSTANCE_COUNT = 30000`.
pub const INSTANCE_COUNT: u32 = 30_000;
/// `ItIsComingForYou.js: export const BLOOM.strength`.
pub const BLOOM_STRENGTH: f32 = 1.0;
/// `ItIsComingForYou.js: export const BLOOM.radius`.
pub const BLOOM_RADIUS: f32 = 0.85;
/// `ItIsComingForYou.js: export const BLOOM.threshold`.
pub const BLOOM_THRESHOLD: f32 = 0.5;
/// `ItIsComingForYou.js: export const CLEAR_ALPHA = 0` — no background.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `ItIsComingForYou.js: export const CAMERA` — fov 45, near 0.1, far 10,
/// position `(0, 0, -2.73)` looking at `(0, 0, 1)`.
pub const CAMERA_FOV_DEGREES: f32 = 45.0;
/// `CAMERA.near`.
pub const CAMERA_NEAR: f32 = 0.1;
/// `CAMERA.far`.
pub const CAMERA_FAR: f32 = 10.0;
/// `CAMERA.position[2]` — the camera sits behind the origin looking down +z.
pub const CAMERA_Z: f32 = -2.73;

/// `ItIsComingForYou.js: export const IN2 = { x: 0.222, y: 0.087, z: 0 }` —
/// the depth cubic-pulse band.
pub const IN2: [f32; 3] = [0.222, 0.087, 0.0];

/// `ItIsComingForYou.js: const MAX_DT = 0.1`.
pub const MAX_DT: f32 = 0.1;

/// `ItIsComingForYou.js: const PERMRAND = seededRand(18, 8)`.
pub fn perm_rand() -> f32 {
    seeded_rand(18, 8)
}

/// `ItIsComingForYou.js: makePerm(seed = 1)`.
pub fn make_perm(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { 1.0 }
}

/// One frame of Expression1 (`ItIsComingForYou.js:78-94`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItIsComingDrive {
    /// `perm` for the next frame.
    pub perm: f32,
    /// `gIn1.x`: the spot term `min(mid * 2, 2) * 2`.
    pub in1_x: f32,
    /// `gIn1.y`: the bass glow `min(bass * 5, 1) * 0.25`.
    pub in1_y: f32,
    /// `gIn1.z`: `perm + PERMRAND * 10`.
    pub in1_z: f32,
    /// `gIn2`: the fixed cubic-pulse band.
    pub in2: [f32; 3],
}

/// Pure port of `itIsComingDrive`.
pub fn it_is_coming_drive(bass: f32, mid: f32, dt: f32, perm: f32) -> ItIsComingDrive {
    let bass = if bass.is_finite() { bass.max(0.0) } else { 0.0 };
    let mid = if mid.is_finite() { mid.max(0.0) } else { 0.0 };
    let step = if dt.is_finite() {
        dt.min(MAX_DT).max(0.0)
    } else {
        0.0
    };
    let base = make_perm(perm);
    let next = base + (bass * 20.0 + 0.1) * step;

    ItIsComingDrive {
        perm: next,
        in1_x: (mid * 2.0).min(2.0) * 2.0,
        in1_y: (bass * 5.0).min(1.0) * 0.25,
        in1_z: next + perm_rand() * 10.0,
        in2: IN2,
    }
}

/// `IT_IS_COMING_PROPS` (`sceneProps.js:92-96`).
const IT_IS_COMING_PROPS: &[SceneProp] = &[
    SceneProp::float("speed", "Cloud Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("glow", "Glow", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
];

/// Uniform block, matching [`IT_IS_COMING_WGSL`] (64 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ItIsComingUniforms {
    /// `gIn1`.
    in1: [f32; 4],
    /// `gIn2`.
    in2: [f32; 4],
    /// `gTime` (`BaseScene.time`).
    time: f32,
    /// `gZNear`.
    z_near: f32,
    /// `gZFar`.
    z_far: f32,
    /// `gTargetSize.x / gTargetSize.y`.
    aspect: f32,
    /// Destination size, for the billboard offset.
    resolution: [f32; 2],
    /// `1 / tan(fov / 2)`.
    fov_scale: f32,
    /// Camera z (`-2.73`).
    camera_z: f32,
}

impl ItIsComingUniforms {
    fn new(width: u32, height: u32, time: f32, in1: [f32; 4], in2: [f32; 4]) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            in1,
            in2,
            time,
            z_near: CAMERA_NEAR,
            z_far: CAMERA_FAR,
            aspect: width as f32 / height as f32,
            resolution: [width as f32, height as f32],
            fov_scale: 1.0 / (CAMERA_FOV_DEGREES.to_radians() * 0.5).tan(),
            camera_z: CAMERA_Z,
        }
    }
}

/// GPU resources built in `init`.
struct ItIsComingGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    quad: UnitQuad,
    composer: Composer,
}

/// ItIsComingForYou scene.
pub struct ItIsComingForYou {
    /// `this.p9audio`.
    audio: P10Audio,
    /// `this.perm`.
    perm: f32,
    /// `this.speedScale` (`speed`).
    speed_scale: f32,
    /// `this.glowScale` (`glow`).
    glow_scale: f32,
    /// `this.bloomScale` (`bloom`).
    bloom_scale: f32,
    /// `BaseScene.time` — advanced every frame, drives `gTime`.
    time: f32,
    /// `gIn1` as last uploaded.
    in1: [f32; 4],
    /// `gIn2` as last uploaded.
    in2: [f32; 4],
    size: (u32, u32),
    gpu: Option<ItIsComingGpu>,
}

impl Default for ItIsComingForYou {
    fn default() -> Self {
        Self::new()
    }
}

impl ItIsComingForYou {
    /// Create the scene (`ItIsComingForYou.js:226-236` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            perm: make_perm(1.0),
            speed_scale: 1.0,
            glow_scale: 1.0,
            bloom_scale: 1.0,
            time: 0.0,
            in1: [0.0; 4],
            in2: [IN2[0], IN2[1], IN2[2], 0.0],
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:389-395`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(IT_IS_COMING_PROPS, props) {
            match key {
                "speed" => self.speed_scale = value,
                "glow" => self.glow_scale = value,
                "bloom" => self.bloom_scale = value,
                _ => {}
            }
        }
    }

    /// Current cloud phase.
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// `BaseScene.time`.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// The `gIn1` uniform as last uploaded.
    pub fn in1(&self) -> [f32; 4] {
        self.in1
    }

    /// The `gIn2` uniform as last uploaded.
    pub fn in2(&self) -> [f32; 4] {
        self.in2
    }

    /// The destination alpha the scene clears to (`CLEAR_ALPHA`).
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }
}

impl Point10Scene for ItIsComingForYou {
    fn name(&self) -> &'static str {
        "Point10 It Is Coming For You"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-it-is-coming-shader"),
            source: wgpu::ShaderSource::Wgsl(it_is_coming_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-it-is-coming-layout"),
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

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-it-is-coming-uniforms"),
            size: std::mem::size_of::<ItIsComingUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-it-is-coming-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-it-is-coming-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let quad = UnitQuad::new(&device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-it-is-coming-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(UnitQuad::vertex_layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    // `transparent: true`, `blending: THREE.NormalBlending`.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        queue.write_buffer(
            &uniform_buffer,
            0,
            bytemuck::bytes_of(&ItIsComingUniforms::new(
                self.size.0,
                self.size.1,
                self.time,
                self.in1,
                self.in2,
            )),
        );

        // `renderer.toneMapping = NoToneMapping` (three's default; the Rust
        // OutputPass defaults to the same) + the preset bloom.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.enable_bloom(BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD);
        // The cloud is deliberately background-free (transparent clear). The
        // additive bloom stage would otherwise write bloom alpha across the
        // whole frame and turn that transparent background into a veil; keep
        // the scene's own alpha (Boxy's preserveBloomAlpha semantics, the same
        // mechanism Wormhole uses).
        composer.set_preserve_bloom_alpha(true);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        self.gpu = Some(ItIsComingGpu {
            queue,
            pipeline,
            bind_group,
            uniform_buffer,
            quad,
            composer,
        });
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.resize(self.size.0, self.size.1);
        }
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        // `super.update(dt, audioData)` -> `this.time += dt`, always.
        if dt.is_finite() && dt > 0.0 {
            self.time += dt;
        }
        self.audio.update(dt, audio);

        let drive = it_is_coming_drive(
            self.audio.band(-1, 0, 0, 0.1),
            self.audio.band(-1, 0, 1, 1.0),
            dt * self.speed_scale,
            self.perm,
        );
        self.perm = drive.perm;
        // `gIn1.set(drive.in1.x * glowScale, drive.in1.y * glowScale,
        // drive.in1.z, 0)`, `gIn2.set(drive.in2.xy, drive.in2.z, 0)`.
        self.in1 = [
            drive.in1_x * self.glow_scale,
            drive.in1_y * self.glow_scale,
            drive.in1_z,
            0.0,
        ];
        self.in2 = [drive.in2[0], drive.in2[1], drive.in2[2], 0.0];
        self.upload_uniforms();

        // `this._bloomPass.strength = BLOOM.strength * this.bloomScale`
        if let Some(gpu) = self.gpu.as_mut() {
            if let Some(bloom) = gpu.composer.bloom_mut() {
                bloom.set_strength(BLOOM_STRENGTH * self.bloom_scale);
            }
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        // `RenderPass(this.group, this.camera, null, null, CLEAR_ALPHA)`: the
        // clear is fully transparent, so the cloud composites over the layers
        // below instead of the preset's Clear2 colour.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-it-is-coming"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gpu.composer.scene_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: CLEAR_ALPHA,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            gpu.quad.draw_instanced(&mut pass, INSTANCE_COUNT);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(IT_IS_COMING_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10ItIsComingForYou.js` sets no category.
        SceneMeta::for_scene("Point10 It Is Coming For You", "3d-scenes")
    }
}

impl ItIsComingForYou {
    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.uniform_buffer,
            0,
            bytemuck::bytes_of(&ItIsComingUniforms::new(
                self.size.0,
                self.size.1,
                self.time,
                self.in1,
                self.in2,
            )),
        );
    }
}

/// The cloud shader (`ItIsComingForYou.js:100-187`) with the Plane9 helpers.
fn it_is_coming_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{IT_IS_COMING_WGSL}")
}

const IT_IS_COMING_WGSL: &str = r#"
struct ItIsComingUniforms {
  in1: vec4<f32>,
  in2: vec4<f32>,
  time: f32,
  z_near: f32,
  z_far: f32,
  aspect: f32,
  resolution: vec2<f32>,
  fov_scale: f32,
  camera_z: f32,
}
@group(0) @binding(0) var<uniform> u: ItIsComingUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) diffuse: vec4<f32>,
  @location(1) world_pos: vec3<f32>,
  @location(2) world_normal: vec3<f32>,
  @location(3) view_dir: vec3<f32>,
  @location(4) view_depth: f32,
  @location(5) uv: vec2<f32>,
}

@vertex
fn vs_main(
  @builtin(instance_index) instance: u32,
  @location(2) corner: vec2<f32>,
  @location(3) uv: vec2<f32>,
) -> VsOut {
  let instance_id = f32(instance);
  let t = u.in1.z * 0.1;
  let spacing = 2.0;
  let d = instance_id / 30000.0;
  let aspect = u.aspect;

  // vec4 rotation = vec4(0.0, 0.0, 1.0, instanceIDFloat * 0.01 + gIn1.z);
  let q_rotation = p9_axis_angle_to_quaternion(
    vec3<f32>(0.0, 0.0, 1.0),
    instance_id * 0.01 + u.in1.z,
  );

  let cloud = vec3<f32>(
    p9_noise(vec3<f32>(d * spacing + t, 0.12, 0.0)),
    p9_noise(vec3<f32>(d * spacing + t, 1.23, 0.0)),
    0.0,
  );
  let size_px = 50.0 + p9_rand2(vec2<f32>(d * spacing)) * 10.0;
  let point_offset = vec2<f32>(sin(d * 731871.0), sin(d * 8234981.0)) * 0.1;

  // `vec4 v = iPosition` (position is all zero), then `v.xy = offset`.
  var v = vec3<f32>(point_offset, 0.0);
  var normal = normalize(v);
  v = p9_rotate_vector(v, q_rotation);
  normal = p9_rotate_vector(normal, q_rotation);
  v = v + cloud * vec3<f32>(aspect, 1.0, 1.0);

  // Camera at (0, 0, -2.73) looking down +z: the look-at basis is a 180°
  // rotation about y, so camera-space x is mirrored and the view depth is
  // `world z + 2.73`.
  let dist = max(v.z - u.camera_z, 0.0001);
  let ndc = vec2<f32>(-v.x * u.fov_scale / aspect, v.y * u.fov_scale) / dist;
  let quad_offset = corner * size_px * 2.0 / u.resolution;

  var out: VsOut;
  out.position = vec4<f32>(ndc + quad_offset, 0.0, 1.0);

  var diffuse = p9_hsv2rgb(d * 0.001, 0.8, 0.8) * vec3<f32>(2.0)
    * p9_smoothstep(0.985, 1.0, p9_rand2(vec2<f32>(d)))
    * vec3<f32>(u.in1.x);
  diffuse = diffuse + vec3<f32>(
    p9_smoothstep(0.99, 1.0, sin(d * 20.0 + u.time * 4.0) * 0.5 + 0.5) * u.in1.y
  );
  diffuse = diffuse * p9_smoothstep(0.991, 0.99, d);

  out.diffuse = vec4<f32>(diffuse, d);
  // `vViewDepth = -(modelViewMatrix * vec4(v.x, v.y, d, 1.0)).z`
  out.view_depth = d - u.camera_z;
  out.world_pos = v;
  // `normalMatrix` is the view rotation; it is applied to the world normal
  // while the lamps and cameraPosition stay in world space (the JS does the
  // same, so the lighting keeps its quirk).
  out.world_normal = normalize(vec3<f32>(-normal.x, normal.y, -normal.z));
  out.view_dir = normalize(vec3<f32>(0.0, 0.0, u.camera_z) - v);
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // gl_PointCoord * 2 - 1 was the sprite-local coordinate.
  let p = in.uv * 2.0 - 1.0;

  let normal = normalize(in.world_normal);
  var c = in.diffuse.xyz;

  var light_dir = normalize(vec3<f32>(0.0, 1.0, 0.0) - in.world_pos);
  c = c + p9_light_lambert(normal, light_dir, vec3<f32>(0.5, 0.8, 1.0)) * 0.025;

  light_dir = normalize(vec3<f32>(0.0, -1.0, 1.0) - in.world_pos);
  c = c + p9_light_lambert(normal, light_dir, vec3<f32>(0.9, 0.5, 0.5)) * 0.05;

  let depth = (in.view_depth - u.z_near) / (u.z_far - u.z_near);
  let blur_amount = 1.0 - p9_cubicpulse(depth, u.in2.x, u.in2.y);
  let ring_size = 0.5;
  let clamped_blur = clamp(blur_amount * 0.4, 0.0, ring_size);
  let blur = p9_smoothstep(ring_size + clamped_blur, ring_size - clamped_blur, length(p));
  return vec4<f32>(c * 5.0, blur);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::{AUDIO_SAMPLES, BandPayload};

    // Port of tests/unit/point10-it-is-coming.test.js.

    #[test]
    fn perm_starts_at_the_preset_seed_and_falls_back() {
        assert_eq!(make_perm(1.0), 1.0);
        assert_eq!(make_perm(3.0), 3.0);
        assert_eq!(make_perm(f32::NAN), 1.0);
    }

    #[test]
    fn advances_perm_from_the_bass_band_with_the_creep() {
        let loud = it_is_coming_drive(0.5, 0.0, 0.1, 1.0);
        let silent = it_is_coming_drive(0.0, 0.0, 0.1, 0.0);
        assert!((loud.perm - (1.0 + (0.5 * 20.0 + 0.1) * 0.1)).abs() < 1e-5);
        assert!((silent.perm - 0.01).abs() < 1e-6);
    }

    #[test]
    fn caps_the_spot_term_at_four() {
        assert_eq!(it_is_coming_drive(0.0, 0.0, 0.0, 1.0).in1_x, 0.0);
        assert!((it_is_coming_drive(0.0, 0.25, 0.0, 1.0).in1_x - 1.0).abs() < 1e-6);
        assert_eq!(it_is_coming_drive(0.0, 1.0, 0.0, 1.0).in1_x, 4.0);
        assert_eq!(it_is_coming_drive(0.0, 5.0, 0.0, 1.0).in1_x, 4.0);
    }

    #[test]
    fn clamps_the_bass_glow_to_a_quarter() {
        assert!((it_is_coming_drive(0.1, 0.0, 0.0, 1.0).in1_y - 0.1 * 5.0 * 0.25).abs() < 1e-6);
        assert!((it_is_coming_drive(1.0, 0.0, 0.0, 1.0).in1_y - 0.25).abs() < 1e-6);
        assert!((it_is_coming_drive(9.0, 0.0, 0.0, 1.0).in1_y - 0.25).abs() < 1e-6);
        assert_eq!(it_is_coming_drive(-1.0, 0.0, 0.0, 1.0).in1_y, 0.0);
    }

    #[test]
    fn exposes_the_seeded_phase_in_in1_z() {
        for perm in [1.0, 2.5, 100.0] {
            let drive = it_is_coming_drive(0.0, 0.0, 0.0, perm);
            assert!((drive.in1_z - (perm + perm_rand() * 10.0)).abs() < 1e-5);
        }
        assert!((0.0..1.0).contains(&perm_rand()));
    }

    #[test]
    fn keeps_the_preset_fixed_in2_band() {
        let drive = it_is_coming_drive(0.2, 0.3, 0.1, 1.0);
        assert_eq!(drive.in2, [0.222, 0.087, 0.0]);
        assert_eq!(IN2, [0.222, 0.087, 0.0]);
    }

    #[test]
    fn sanitises_negative_and_non_finite_input() {
        let negative = it_is_coming_drive(-1.0, -1.0, -1.0, f32::NAN);
        let bad = it_is_coming_drive(f32::NAN, f32::NAN, f32::NAN, f32::NAN);
        assert_eq!(negative.perm, 1.0);
        assert_eq!(negative.in1_x, 0.0);
        assert_eq!(negative.in1_y, 0.0);
        assert_eq!(bad.perm, 1.0);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(INSTANCE_COUNT, 30_000);
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (1.0, 0.85, 0.5)
        );
        assert_eq!(CAMERA_FOV_DEGREES, 45.0);
        assert_eq!((CAMERA_NEAR, CAMERA_FAR, CAMERA_Z), (0.1, 10.0, -2.73));
    }

    #[test]
    fn renders_with_a_fully_transparent_background() {
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(ItIsComingForYou::new().clear_alpha(), 0.0);
    }

    #[test]
    fn shader_keeps_the_cloud_maths_and_transparency() {
        let program = it_is_coming_wgsl();
        for needle in [
            "instance_id / 30000.0",
            "50.0 + p9_rand2",
            "p9_axis_angle_to_quaternion",
            "p9_hsv2rgb(d * 0.001, 0.8, 0.8)",
            "p9_light_lambert",
            "p9_cubicpulse",
            "c * 5.0",
            "vec4<f32>(c * 5.0, blur)",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
        // The old shader forced the point alpha to 1.0.
        assert!(!program.contains("vec4<f32>(c * 5.0, 1.0)"));
    }

    #[test]
    fn uniforms_are_64_bytes() {
        assert_eq!(std::mem::size_of::<ItIsComingUniforms>(), 64);
    }

    #[test]
    fn uniforms_use_the_preset_camera() {
        let uniforms = ItIsComingUniforms::new(640, 320, 1.5, [0.0; 4], [0.2, 0.1, 0.0, 0.0]);
        assert_eq!(uniforms.aspect, 2.0);
        assert_eq!(uniforms.z_near, 0.1);
        assert_eq!(uniforms.z_far, 10.0);
        assert_eq!(uniforms.camera_z, -2.73);
        // 1 / tan(22.5°) = 2.4142135
        assert!((uniforms.fov_scale - 2.414_213_5).abs() < 1e-6);
        assert_eq!(uniforms.time, 1.5);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = ItIsComingForYou::new();
        scene.apply_props(|key| match key {
            "speed" => Some(9.0),
            "glow" => Some(2.0),
            "bloom" => Some(-1.0),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.glow_scale, 2.0);
        assert_eq!(scene.bloom_scale, 0.0);
    }

    #[test]
    fn update_advances_the_clock_and_the_cloud_from_bass_and_mid() {
        let mut scene = ItIsComingForYou::new();
        let bridge = AudioBridge::new();
        scene.update(0.1, &bridge);
        assert!((scene.time() - 0.1).abs() < 1e-6, "BaseScene.time advances");
        // No audio: `band(-1,0,0,0.1)` returns its 0.1 fallback and
        // `band(-1,0,1,1)` its 1.0 fallback (see `P10Audio::band`), which is
        // what the JS passes as `nomusic`.
        assert!((scene.perm() - (1.0 + (0.1 * 20.0 + 0.1) * 0.1)).abs() < 1e-5);
        assert!(
            (scene.in1()[0] - 4.0).abs() < 1e-6,
            "mid fallback 1.0 -> min(2, 2) * 2"
        );
        assert!(
            (scene.in1()[1] - 0.125).abs() < 1e-6,
            "bass fallback 0.1 -> min(0.5, 1) * 0.25"
        );

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 0.2; // bass
        payload.lanes[1] = 0.5; // mid
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let before = scene.perm();
        scene.update(0.1, &bridge);
        assert!((scene.perm() - before - (0.2 * 20.0 + 0.1) * 0.1).abs() < 1e-5);
        assert!((scene.in1()[0] - 2.0).abs() < 1e-5, "min(0.5*2,2)*2");
        assert!((scene.in1()[1] - 0.25).abs() < 1e-5, "min(0.2*5,1)*0.25");
        assert_eq!(scene.in2(), [0.222, 0.087, 0.0, 0.0]);
    }

    #[test]
    fn glow_scales_both_g_in1_terms() {
        let mut scene = ItIsComingForYou::new();
        scene.apply_props(|key| (key == "glow").then_some(2.0));

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 0.2;
        payload.lanes[1] = 0.5;
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.1, &bridge);
        assert!((scene.in1()[0] - 4.0).abs() < 1e-5);
        assert!((scene.in1()[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = ItIsComingForYou::new();
        assert_eq!(scene.meta().name, "Point10 It Is Coming For You");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(&names[names.len() - 3..], &["speed", "glow", "bloom"]);
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = it_is_coming_wgsl();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("IT_IS_COMING_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("IT_IS_COMING_WGSL must validate");
    }
}
