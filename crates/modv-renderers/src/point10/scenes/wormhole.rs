//! Wormhole — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/Wormhole.js`.
//!
//! # What the JS actually draws
//!
//! The Plane9 "Wormhole" preset (Joakim Dahl, CC BY-NC-SA 4.0): 100 000
//! additive point sprites whose positions come from a per-point 2-arm spiral
//! (`MeshInstancer1`), bloomed, ACES-composited. `RenderToTexture1` is fed by
//! `RenderObject1.Render`, so the capture is the point chain and not the node's
//! noise shader. `PreviousLayer1` resolves to black in standalone playback, so
//! there is no frame feedback.
//!
//! # Ported exactly
//!
//! * the point shader (`Wormhole.js:85-141`): `d = aId / 100000`,
//!   `t = fract(d - gIn1.z * 0.01)`, `tt = t * 300`, `paramSet = floor(d +
//!   gIn1.z * 0.5)`, `k`/`l` from `_rand(vec2(paramSet, …))`, the two-arm
//!   spiral `nv.xy`, `nv.z = fract(d + gIn1.z * 0.5 + gIn3.x) * 20 - 20`,
//!   `pos = (nv.xy * 2, nv.z * 1.5)`, `gl_PointSize = 5`, the HSV-ramp colour
//!   `* 0.25` and the three alpha fades;
//! * the additive `ONE/ONE` blending with `transparent: true` and the
//!   transparent clear (`CLEAR_ALPHA = 0`, `:48-53`);
//! * `gl_FragColor = (saturate(mask * vDiffuse.rgb) * vDiffuse.a,
//!   saturate(mask * vDiffuse.a))` with `mask = smoothstep(1.0, 0.9,
//!   length(p))` (`:125-141`) — the point's own alpha reaches the frame, so the
//!   empty space around the points stays transparent;
//! * the camera (`CAMERA = { fov: 60, near: 0.01, far: 50 }`, position at the
//!   origin looking down -z) and the preset bloom (`strength: 0.1, radius:
//!   0.85, threshold: 0`) with ACES tone mapping (`:178-201`);
//! * Expression1 (`wormholeDrive`, `:65-79`):
//!   `perm += min(2, band(-1,0,0,0.1) * 25 + 0.25) * dt`,
//!   `gIn1 = (PERMRAND * 100, 0, perm + PERMRAND * 10, 0)`, `gIn3 = 0`;
//!   `PERMRAND = seededRand(21, 8)`;
//! * the prop binder (`sceneProps.js:436-439`): `speedScale = warpSpeed`,
//!   `bloomScale = bloom` (retunes the bloom pass each frame).
//!
//! # `preserveBloomAlpha`
//!
//! `Wormhole.js:26` imports `preserveBloomAlpha` from `Boxy.js` and wraps its
//! bloom pass with it (`:192-199`). The JS helper rewrites three's bloom
//! composite blend state (`Boxy.js:73-86`):
//!
//! ```text
//! colour: Add, src = ONE,        dst = ONE         // bloom RGB is added
//! alpha:  Add, srcAlpha = ZERO,  dstAlpha = ONE    // destination alpha wins
//! ```
//!
//! Bloom `threshold: 0` means every pixel receives bloom; without this the
//! additive alpha would turn the transparent clear into an opaque veil and the
//! module would stop compositing over the layers below. In Rust the composite
//! is [`crate::point10::postfx::Composer`]'s combine pass, so the port calls
//! [`Composer::set_preserve_bloom_alpha(true)`](crate::point10::postfx::Composer::set_preserve_bloom_alpha)
//! which selects the `fs_preserve_alpha` entry point (`base.rgb + bloom.rgb`,
//! `base.a`). The arithmetic is identical to the JS blend state.
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `THREE.Points` + `ShaderMaterial` `gl_PointSize` / `gl_PointCoord` | instanced billboard quads ([`UnitQuad`]), 5 px, `uv` in place of `gl_PointCoord` |
//! | `geometry.setAttribute("aId", …)` (100k floats) | `@builtin(instance_index)` — no attribute buffer at all |
//! | `projectionMatrix * modelViewMatrix` | `fov_scale = 1/tan(30°)`, `aspect`, camera at the origin |
//! | `_rand`, `_hsv2rgb`, `saturate` | [`P9_COMMON_WGSL`] |
//! | `EffectComposer` + `RenderPass` + `UnrealBloomPass` + `OutputPass` | [`Composer`] with `enable_bloom` + `set_preserve_bloom_alpha` + ACES |
//!
//! # Deviations
//!
//! * `gl_PointSize` is in device pixels and the JS never attenuates it, so the
//!   billboards are a constant 5 px — same as the point sprites.
//! * WebGL clamps `gl_PointSize` to the implementation's point-size range;
//!   wgpu has no point primitives, so the port rasterises a quad instead. At
//!   5 px the two are indistinguishable.
//! * `k` can be exactly 0 (`_rand` returns `[0,1)`), which makes `kInv / k`
//!   infinite in both implementations. Kept as-is rather than guarded: the
//!   point is a single 5 px sprite.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp, UnitQuad, read_scene_props,
    scene_props_with_levers,
};

/// `Wormhole.js: export const POINT_COUNT = 100000`.
pub const POINT_COUNT: u32 = 100_000;
/// `Wormhole.js: export const POINT_SIZE = 5` (device pixels).
pub const POINT_SIZE: f32 = 5.0;
/// `Wormhole.js: export const POINT_DIST = 300`.
pub const POINT_DIST: f32 = 300.0;
/// `Wormhole.js: export const SPEED = 0.5`.
pub const SPEED: f32 = 0.5;
/// `Wormhole.js: export const CLEAR_ALPHA = 0` — the point cloud is sparse, so
/// empty space stays transparent and the scene composites over the layers below.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `Wormhole.js: export const BLOOM.strength`.
pub const BLOOM_STRENGTH: f32 = 0.1;
/// `Wormhole.js: export const BLOOM.radius`.
pub const BLOOM_RADIUS: f32 = 0.85;
/// `Wormhole.js: export const BLOOM.threshold`.
pub const BLOOM_THRESHOLD: f32 = 0.0;

/// `Wormhole.js: export const CAMERA = { fov: 60, near: 0.01, far: 50 }`.
pub const CAMERA_FOV_DEGREES: f32 = 60.0;

/// `Wormhole.js: const PERMRAND = seededRand(21, 8)`.
pub fn perm_rand() -> f32 {
    seeded_rand(21, 8)
}

/// `Wormhole.js: makePerm(seed = 1)`.
pub fn make_perm(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { 1.0 }
}

/// One frame of Expression1 (`Wormhole.js:65-79`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WormholeDrive {
    /// `perm` for the next frame.
    pub perm: f32,
    /// `gIn1.x`: `PERMRAND * 100` (unused by the shader, kept for parity).
    pub in1_x: f32,
    /// `gIn1.y`: always 0.
    pub in1_y: f32,
    /// `gIn1.z`: the tunnel phase.
    pub in1_z: f32,
}

/// Pure port of `wormholeDrive`.
///
/// Note the JS does **not** clamp `dt` here (`Math.max(0, dt)` only), unlike
/// the other batch-2 scenes, so a long frame really does advance the whole
/// wormhole in one step. That is kept.
pub fn wormhole_drive(bass: f32, dt: f32, perm: f32) -> WormholeDrive {
    let bass = if bass.is_finite() { bass.max(0.0) } else { 0.0 };
    let step = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let base = if perm.is_finite() { perm } else { 0.0 };
    // `perm += Math.min(2, b * 25 + 0.25) * dt`
    let next = base + (bass * 25.0 + 0.25).min(2.0) * step;

    WormholeDrive {
        perm: next,
        in1_x: perm_rand() * 100.0,
        in1_y: 0.0,
        in1_z: next + perm_rand() * 10.0,
    }
}

/// `WORMHOLE_PROPS` (`sceneProps.js:140-143`).
const WORMHOLE_PROPS: &[SceneProp] = &[
    SceneProp::float("warpSpeed", "Warp Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
];

/// Uniform block, matching [`WORMHOLE_WGSL`] (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct WormholeUniforms {
    /// `gIn1`.
    in1: [f32; 4],
    /// `gIn3` (always zero in this scene).
    in3: [f32; 4],
    /// Destination size, for the billboard offset.
    resolution: [f32; 2],
    /// `width / height`.
    aspect: f32,
    /// `1 / tan(fov / 2)`.
    fov_scale: f32,
}

impl WormholeUniforms {
    fn new(width: u32, height: u32, in1: [f32; 4]) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            in1,
            in3: [0.0; 4],
            resolution: [width as f32, height as f32],
            aspect: width as f32 / height as f32,
            fov_scale: 1.0 / (CAMERA_FOV_DEGREES.to_radians() * 0.5).tan(),
        }
    }
}

/// GPU resources built in `init`.
struct WormholeGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    quad: UnitQuad,
    composer: Composer,
}

/// Wormhole scene.
pub struct Wormhole {
    /// `this.p9audio`.
    audio: P10Audio,
    /// `this.perm`.
    perm: f32,
    /// `this.speedScale` (`warpSpeed`).
    speed_scale: f32,
    /// `this.bloomScale` (`bloom`).
    bloom_scale: f32,
    /// `gIn1` as last uploaded.
    in1: [f32; 4],
    size: (u32, u32),
    gpu: Option<WormholeGpu>,
}

impl Default for Wormhole {
    fn default() -> Self {
        Self::new()
    }
}

impl Wormhole {
    /// Create the scene (`Wormhole.js:204-213` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            perm: make_perm(1.0),
            speed_scale: 1.0,
            bloom_scale: 1.0,
            in1: [0.0; 4],
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:436-439`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(WORMHOLE_PROPS, props) {
            match key {
                "warpSpeed" => self.speed_scale = value,
                "bloom" => self.bloom_scale = value,
                _ => {}
            }
        }
    }

    /// Current tunnel phase.
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// The `gIn1` uniform as last uploaded.
    pub fn in1(&self) -> [f32; 4] {
        self.in1
    }

    /// The destination alpha the scene clears to.
    ///
    /// `applyWormholeClear(renderer)` (`:50-53`) is `setClearColor(0x000000, 0)`;
    /// the Rust equivalent is the render pass's `LoadOp::Clear` alpha.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }
}

impl Point10Scene for Wormhole {
    fn name(&self) -> &'static str {
        "Point10 Wormhole"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-wormhole-shader"),
            source: wgpu::ShaderSource::Wgsl(wormhole_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-wormhole-layout"),
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
            label: Some("point10-wormhole-uniforms"),
            size: std::mem::size_of::<WormholeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-wormhole-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-wormhole-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        // `transparent: true`, `blending: CustomBlending`, `blendSrc: OneFactor`,
        // `blendDst: OneFactor`, `AddEquation` (`:165-171`) — additive RGB *and*
        // alpha.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let quad = UnitQuad::new(&device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-wormhole-pipeline"),
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
                    blend: Some(additive),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        queue.write_buffer(
            &uniform_buffer,
            0,
            bytemuck::bytes_of(&WormholeUniforms::new(self.size.0, self.size.1, self.in1)),
        );

        // `renderer.toneMapping = ACESFilmicToneMapping` + the preset bloom.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.enable_bloom(BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD);
        // `preserveBloomAlpha(new UnrealBloomPass(...))` (`:192-199`).
        composer.set_preserve_bloom_alpha(true);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);

        self.gpu = Some(WormholeGpu {
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
        self.audio.update(dt, audio);

        let drive = wormhole_drive(
            self.audio.band(-1, 0, 0, 0.1),
            dt * self.speed_scale,
            self.perm,
        );
        self.perm = drive.perm;
        // `gIn1.set(drive.in1.x, drive.in1.y, drive.in1.z, 0)`, `gIn3 = 0`
        self.in1 = [drive.in1_x, drive.in1_y, drive.in1_z, 0.0];
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

        // `RenderPass(this.group, this.camera, null, null, CLEAR_ALPHA)`.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-wormhole"),
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
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            gpu.quad.draw_instanced(&mut pass, POINT_COUNT);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(WORMHOLE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10Wormhole.js` sets no category.
        SceneMeta::for_scene("Point10 Wormhole", "3d-scenes")
    }
}

impl Wormhole {
    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.uniform_buffer,
            0,
            bytemuck::bytes_of(&WormholeUniforms::new(self.size.0, self.size.1, self.in1)),
        );
    }
}

/// The point shader (`Wormhole.js:85-141`) with the Plane9 helpers in front.
fn wormhole_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{WORMHOLE_WGSL}")
}

const WORMHOLE_WGSL: &str = r#"
struct WormholeUniforms {
  in1: vec4<f32>,
  in3: vec4<f32>,
  resolution: vec2<f32>,
  aspect: f32,
  fov_scale: f32,
}
@group(0) @binding(0) var<uniform> u: WormholeUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) diffuse: vec4<f32>,
  @location(1) uv: vec2<f32>,
}

// `aId` in the JS is a float attribute 0..99999; wgpu's instance index is the
// same number for free.
@vertex
fn vs_main(
  @builtin(instance_index) instance: u32,
  @location(2) corner: vec2<f32>,
  @location(3) uv: vec2<f32>,
) -> VsOut {
  let d = f32(instance) / 100000.0;

  let speed = 0.5;
  let t = fract(d - u.in1.z * 0.01); // Current point pos in the graph
  let point_dist = 300.0;
  let tt = t * point_dist;
  let param_set = floor(d + u.in1.z * speed);

  let k = p9_rand2(vec2<f32>(param_set, 0.123));
  let l = p9_rand2(vec2<f32>(param_set, 2.572));
  let scale = 3.0;

  let k_inv = 1.0 - k;
  let nv = vec3<f32>(
    (k_inv * cos(tt) + (l * k) * cos((k_inv / k) * tt)) * scale,
    (k_inv * sin(tt) - (l * k) * sin((k_inv / k) * tt)) * scale,
    fract(d + u.in1.z * speed + u.in3.x) * 20.0 - 20.0,
  );

  let pos = vec3<f32>(nv.x * 2.0, nv.y * 2.0, nv.z * 1.5);
  // Screen1: camera at the origin looking down -z, fov 60, aspect w/h.
  let dist = max(-pos.z, 0.0001);
  let ndc = vec2<f32>(pos.x * u.fov_scale / u.aspect, pos.y * u.fov_scale) / dist;

  // gl_PointSize = 5.0 -> a constant-size billboard (u.resolution is the
  // device-pixel size, the same units as gl_PointSize).
  let size_px = 5.0;
  let offset = corner * size_px * 2.0 / u.resolution;

  var out: VsOut;
  out.position = vec4<f32>(ndc + offset, 0.0, 1.0);

  var diffuse = vec4<f32>(p9_hsv2rgb(l, 0.75 + k * 0.25, 0.9), 1.0) * vec4<f32>(0.25);
  var alpha = p9_smoothstep(0.1, 0.2, length(nv.xy)); // Fade out at center
  alpha = alpha * p9_smoothstep(-20.0, -5.0, nv.z); // Fade by distance
  alpha = alpha * p9_smoothstep(0.0, -0.5, nv.z); // Fade by distance
  diffuse.a = alpha;
  out.diffuse = diffuse;
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // gl_PointCoord was 0..1 across the sprite; the quad's uv is the same.
  let p = in.uv * 2.0 - 1.0;
  let mask = p9_smoothstep(1.0, 0.9, length(p));
  // Keep the additive colour, but carry the point's own alpha through so the
  // empty space around the points stays transparent.
  return vec4<f32>(
    p9_saturate3(vec3<f32>(mask) * in.diffuse.rgb) * in.diffuse.a,
    p9_saturate(mask * in.diffuse.a),
  );
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::{AUDIO_SAMPLES, BandPayload};

    // Port of tests/unit/point10-wormhole.test.js.

    #[test]
    fn perm_starts_at_the_preset_seed_and_falls_back() {
        assert_eq!(make_perm(1.0), 1.0);
        assert_eq!(make_perm(4.0), 4.0);
        assert_eq!(make_perm(f32::NAN), 1.0);
    }

    #[test]
    fn advances_perm_by_the_bass_band_plus_the_creep() {
        let next = wormhole_drive(0.05, 0.5, 1.0);
        assert!((next.perm - (1.0 + (0.05 * 25.0 + 0.25) * 0.5)).abs() < 1e-5);
        assert!(next.perm > 1.0);
    }

    #[test]
    fn caps_the_bass_advance_at_two_units_per_second() {
        let loud = wormhole_drive(1.0, 0.5, 1.0);
        let louder = wormhole_drive(10.0, 0.5, 1.0);
        assert!((loud.perm - (1.0 + 2.0 * 0.5)).abs() < 1e-5);
        assert!((louder.perm - loud.perm).abs() < 1e-5);
        assert!((wormhole_drive(1.0, 1.0, 1.0).perm - 3.0).abs() < 1e-5);
    }

    #[test]
    fn creeps_in_silence_and_ignores_negative_bass_or_dt() {
        let silent = wormhole_drive(0.0, 0.1, 0.0);
        let negative = wormhole_drive(-1.0, -1.0, 1.0);
        let bad = wormhole_drive(f32::NAN, f32::NAN, f32::NAN);
        assert!((silent.perm - 0.025).abs() < 1e-6);
        assert!((negative.perm - 1.0).abs() < 1e-6);
        assert!(bad.perm.abs() < 1e-6);
    }

    #[test]
    fn in1_uses_permrand_for_x_and_the_perm_for_z() {
        let start = wormhole_drive(0.0, 0.0, 0.0);
        let next = wormhole_drive(0.2, 0.5, 2.0);
        assert!((start.in1_x - perm_rand() * 100.0).abs() < 1e-4);
        assert!((next.in1_x - perm_rand() * 100.0).abs() < 1e-4);
        assert!((next.in1_z - (next.perm + perm_rand() * 10.0)).abs() < 1e-5);
        assert_eq!(next.in1_y, 0.0);
    }

    #[test]
    fn permrand_is_stable_and_in_the_unit_interval() {
        assert_eq!(perm_rand(), perm_rand());
        assert!((0.0..1.0).contains(&perm_rand()));
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(POINT_COUNT, 100_000);
        assert_eq!(POINT_SIZE, 5.0);
        assert_eq!(POINT_DIST, 300.0);
        assert_eq!(SPEED, 0.5);
        assert_eq!(CAMERA_FOV_DEGREES, 60.0);
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (0.1, 0.85, 0.0)
        );
    }

    #[test]
    fn clears_to_a_fully_transparent_background() {
        // `applyWormholeClear(renderer) -> setClearColor(0x000000, 0)`
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(Wormhole::new().clear_alpha(), 0.0);
    }

    #[test]
    fn shader_keeps_the_wormhole_maths_and_round_point_falloff() {
        let program = wormhole_wgsl();
        for needle in [
            "f32(instance) / 100000.0",
            "fract(d - u.in1.z * 0.01)",
            "p9_rand2(vec2<f32>(param_set, 0.123))",
            "p9_hsv2rgb(l, 0.75 + k * 0.25, 0.9)",
            "p9_smoothstep(0.1, 0.2, length(nv.xy))",
            "size_px = 5.0",
            "p9_smoothstep(1.0, 0.9, length(p))",
            "p9_saturate(mask * in.diffuse.a)",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
    }

    #[test]
    fn uniforms_are_48_bytes() {
        assert_eq!(std::mem::size_of::<WormholeUniforms>(), 48);
    }

    #[test]
    fn uniforms_project_with_the_preset_camera() {
        let uniforms = WormholeUniforms::new(800, 400, [0.0; 4]);
        assert_eq!(uniforms.aspect, 2.0);
        // 1 / tan(30°) = 1.7320508
        assert!((uniforms.fov_scale - 1.732_050_8).abs() < 1e-6);
        assert_eq!(uniforms.resolution, [800.0, 400.0]);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = Wormhole::new();
        scene.apply_props(|key| match key {
            "warpSpeed" => Some(9.0),
            "bloom" => Some(-1.0),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.bloom_scale, 0.0);
    }

    #[test]
    fn update_advances_the_phase_from_the_bass_band() {
        let mut scene = Wormhole::new();
        let bridge = AudioBridge::new();
        // No audio: `band(-1,0,0,0.1)` mirrors the JS's 0.1 fallback, which
        // saturates the 2-units-per-second cap.
        scene.update(0.1, &bridge);
        assert!((scene.perm() - 1.2).abs() < 1e-5);

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 0.05;
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let before = scene.perm();
        scene.update(0.5, &bridge);
        assert!((scene.perm() - before - (0.05 * 25.0 + 0.25) * 0.5).abs() < 1e-5);
    }

    #[test]
    fn warp_speed_scales_the_phase_advance() {
        let mut scene = Wormhole::new();
        scene.apply_props(|key| (key == "warpSpeed").then_some(2.0));

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 1.0;
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.25, &bridge);
        // min(2, 1 * 25 + 0.25) * (0.25 * 2) = 1.0
        assert!((scene.perm() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = Wormhole::new();
        assert_eq!(scene.meta().name, "Point10 Wormhole");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(&names[names.len() - 2..], &["warpSpeed", "bloom"]);
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = wormhole_wgsl();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("WORMHOLE_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("WORMHOLE_WGSL must validate");
    }
}
