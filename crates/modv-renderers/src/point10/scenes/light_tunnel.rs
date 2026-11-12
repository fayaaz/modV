//! Light Tunnel — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/LightTunnel.js`.
//!
//! # What the JS actually draws
//!
//! The Plane9 "Light Tunnel" preset (Joakim Dahl, CC BY-NC-SA 3.0): one
//! fullscreen fragment shader built from 3D simplex turbulence, drawn into a
//! `RenderToTexture`, bloomed, and ACES-tonemapped on the way to the screen.
//! three's chain is `RenderPass -> UnrealBloomPass -> OutputPass`, i.e. exactly
//! [`crate::point10::postfx::Composer`].
//!
//! # Ported exactly
//!
//! * the tunnel shader (`LightTunnel.js:66-98`): `d = 0.25 / r + gIn1.x`, the
//!   `vec3(a+d, a-d, a+sin(d)*0.1)` colour ramp, `_turbulencefast` (5 and 3
//!   octaves) and `_fbmfast` (3 octaves), the `0.953/0.047/0.090 * 500` glow,
//!   the vignette and the final `* 0.125`, alpha `1.0`;
//! * Expression1 (`tunnelDrive`, `:46-60`):
//!   `perm += (band(-1,0,0,0.025) * 3 + 0.01) * dt` with `dt` clamped to 0.1 s,
//!   `gIn1 = (perm + PERMRAND * 10, min(1, band(-1,1,1,0)), 0, 0)`;
//!   `PERMRAND = seededRand(9, 8)`;
//! * the preset's bloom (`BLOOM = { strength: 0.17, radius: 0.85, threshold:
//!   0 }`, Plane9 Radius 5 / Intensity 0.17 normalised by UnrealBloomPass) and
//!   `renderer.toneMapping = ACESFilmicToneMapping` with exposure 1
//!   (`:134-135`);
//! * the prop binder (`sceneProps.js:396-400`): `speedScale = scrollSpeed`,
//!   `glowScale = glow`, `bloomScale = bloom` — the last one retunes the bloom
//!   pass every frame (`:188-190`).
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `PlaneGeometry(2, 2)` clip-space quad | fullscreen triangle |
//! | `EffectComposer` + `RenderPass` + `UnrealBloomPass` + `OutputPass` | [`Composer`](crate::point10::postfx::Composer) with `enable_bloom` + [`ToneMapping::AcesFilmic`](crate::point10::postfx::ToneMapping::AcesFilmic) |
//! | `_turbulencefast` / `_fbmfast` / `_noise(vec3)` | [`P9_COMMON_WGSL`] |
//! | `renderer.getSize()` / `composer.setSize` in `render()` | `resize()` forwards to [`Composer::resize`] |
//!
//! # Deviations
//!
//! * The JS re-sizes the composer from `renderer.domElement` inside `render()`
//!   (a per-frame `_cw/_ch` compare). The Rust contract has an explicit
//!   `resize`, so the check is dropped (the engine calls it on every size
//!   change).
//! * `captureTexture` / `_captureNextFrame` (the preset thumbnail path) has no
//!   Rust equivalent: the engine reads the composer's own target.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::shared::random::seeded_rand;
use crate::point10::{FULLSCREEN_TRIANGLE_WGSL, shared::p9_glsl::P9_COMMON_WGSL};
use crate::point10::{
    HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp, read_scene_props,
    scene_props_with_levers,
};

/// `LightTunnel.js: export const TUNNEL_SIZE = 2`.
pub const TUNNEL_SIZE: f32 = 2.0;

/// `LightTunnel.js: const MAX_DT = 0.1`.
pub const MAX_DT: f32 = 0.1;

/// `LightTunnel.js: export const BLOOM.strength` (Plane9 Intensity 0.17).
pub const BLOOM_STRENGTH: f32 = 0.17;
/// `LightTunnel.js: export const BLOOM.radius` (Plane9 Radius 5 -> 0.85).
pub const BLOOM_RADIUS: f32 = 0.85;
/// `LightTunnel.js: export const BLOOM.threshold`.
pub const BLOOM_THRESHOLD: f32 = 0.0;

/// `LightTunnel.js: const PERMRAND = seededRand(9, 8)`.
pub fn perm_rand() -> f32 {
    seeded_rand(9, 8)
}

/// `LightTunnel.js: makePerm(seed = 1)`.
pub fn make_perm(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { 1.0 }
}

/// One frame of Expression1 (`LightTunnel.js:46-60`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightTunnelDrive {
    /// `perm` for the next frame.
    pub perm: f32,
    /// `gIn1.x`: the tunnel scroll phase (`perm + PERMRAND * 10`).
    pub in1_x: f32,
    /// `gIn1.y`: `min(1, band(-1, 1, 1, 0))`, the red glow.
    pub in1_y: f32,
}

/// Pure port of `tunnelDrive`.
pub fn tunnel_drive(bass: f32, mid: f32, dt: f32, perm: f32) -> LightTunnelDrive {
    let bass = if bass.is_finite() { bass.max(0.0) } else { 0.0 };
    let mid = if mid.is_finite() { mid.max(0.0) } else { 0.0 };
    let step = if dt.is_finite() {
        dt.min(MAX_DT).max(0.0)
    } else {
        0.0
    };
    let base = if perm.is_finite() { perm } else { 0.0 };
    let next = base + (bass * 3.0 + 0.01) * step;

    LightTunnelDrive {
        perm: next,
        in1_x: next + perm_rand() * 10.0,
        in1_y: mid.min(1.0),
    }
}

/// `LIGHT_TUNNEL_PROPS` (`sceneProps.js:98-102`).
const LIGHT_TUNNEL_PROPS: &[SceneProp] = &[
    SceneProp::float("scrollSpeed", "Scroll Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("glow", "Glow", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
];

/// Uniform block, matching [`LIGHT_TUNNEL_WGSL`] (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct LightTunnelUniforms {
    /// `gIn1`.
    in1: [f32; 4],
    /// Destination size, for the fragment's `uv`.
    resolution: [f32; 2],
    _pad: [f32; 2],
}

impl LightTunnelUniforms {
    fn new(width: u32, height: u32, in1: [f32; 4]) -> Self {
        Self {
            in1,
            resolution: [width.max(1) as f32, height.max(1) as f32],
            _pad: [0.0; 2],
        }
    }
}

/// GPU resources built in `init`.
struct LightTunnelGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    /// `EffectComposer` (`:138-149`).
    composer: Composer,
}

/// LightTunnel scene.
pub struct LightTunnel {
    /// `this.p9audio`.
    audio: P10Audio,
    /// `this.perm`.
    perm: f32,
    /// `this.speedScale` (`scrollSpeed`).
    speed_scale: f32,
    /// `this.glowScale` (`glow`).
    glow_scale: f32,
    /// `this.bloomScale` (`bloom`).
    bloom_scale: f32,
    /// `gIn1` as last uploaded.
    in1: [f32; 4],
    size: (u32, u32),
    gpu: Option<LightTunnelGpu>,
}

impl Default for LightTunnel {
    fn default() -> Self {
        Self::new()
    }
}

impl LightTunnel {
    /// Create the scene (`LightTunnel.js:151-160` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            perm: make_perm(1.0),
            speed_scale: 1.0,
            glow_scale: 1.0,
            bloom_scale: 1.0,
            in1: [0.0; 4],
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:396-400`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(LIGHT_TUNNEL_PROPS, props) {
            match key {
                "scrollSpeed" => self.speed_scale = value,
                "glow" => self.glow_scale = value,
                "bloom" => self.bloom_scale = value,
                _ => {}
            }
        }
    }

    /// Current scroll phase.
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// The `gIn1` uniform as last uploaded.
    pub fn in1(&self) -> [f32; 4] {
        self.in1
    }

    /// The live bloom strength (`BLOOM.strength * bloomScale`), or `None`
    /// before `init`.
    pub fn bloom_strength(&self) -> Option<f32> {
        self.gpu
            .as_ref()
            .and_then(|gpu| gpu.composer.bloom().map(|bloom| bloom.strength()))
    }
}

impl Point10Scene for LightTunnel {
    fn name(&self) -> &'static str {
        "Point10 Light Tunnel"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-light-tunnel-shader"),
            source: wgpu::ShaderSource::Wgsl(light_tunnel_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-light-tunnel-layout"),
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

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-light-tunnel-uniforms"),
            size: std::mem::size_of::<LightTunnelUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-light-tunnel-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-light-tunnel-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // The scene draws into the composer's HDR target, not the engine's
        // destination, so the pipeline format is the composer's scene format.
        let pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-light-tunnel",
            &shader,
            &pipeline_layout,
            HDR_FORMAT,
            None,
        );

        queue.write_buffer(
            &uniform_buffer,
            0,
            bytemuck::bytes_of(&LightTunnelUniforms::new(
                self.size.0,
                self.size.1,
                self.in1,
            )),
        );

        // `renderer.toneMapping = ACESFilmicToneMapping` + the preset bloom.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.enable_bloom(BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);

        self.gpu = Some(LightTunnelGpu {
            queue,
            pipeline,
            bind_group,
            uniform_buffer,
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

        // Expression1: bass advances perm, band 1 adds the red glow.
        let drive = tunnel_drive(
            self.audio.band(-1, 0, 0, 0.025),
            self.audio.band(-1, 1, 1, 0.0),
            dt * self.speed_scale,
            self.perm,
        );
        self.perm = drive.perm;
        // `gIn1.set(drive.in1.x, drive.in1.y * this.glowScale, 0, 0)`
        self.in1 = [drive.in1_x, drive.in1_y * self.glow_scale, 0.0, 0.0];
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

        // `RenderPass(this.group, this.camera)` draws the quad into the
        // composer's read buffer; the shader writes alpha 1 over the whole
        // frame, so the clear colour never reaches the destination.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-light-tunnel"),
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
            pass.draw(0..3, 0..1);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(LIGHT_TUNNEL_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10LightTunnel.js` sets no category.
        SceneMeta::for_scene("Point10 Light Tunnel", "3d-scenes")
    }
}

impl LightTunnel {
    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.uniform_buffer,
            0,
            bytemuck::bytes_of(&LightTunnelUniforms::new(
                self.size.0,
                self.size.1,
                self.in1,
            )),
        );
    }
}

/// The tunnel shader (`LightTunnel.js:66-98`) with the Plane9 helpers in front.
fn light_tunnel_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{P9_COMMON_WGSL}\n{LIGHT_TUNNEL_WGSL}")
}

const LIGHT_TUNNEL_WGSL: &str = r#"
struct LightTunnelUniforms {
  in1: vec4<f32>,
  resolution: vec2<f32>,
  _pad: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: LightTunnelUniforms;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv01 = position.xy / u.resolution;
  // PlaneGeometry's vUv is bottom-up; the fragment position is top-down.
  let p = vec2<f32>(uv01.x * 2.0 - 1.0, 1.0 - uv01.y * 2.0);
  let r = length(p);
  let a = atan2(p.y, p.x) / P9_PI2;

  let d = 0.25 / r + u.in1.x;

  let uv = vec3<f32>(a + d, a - d, a + sin(d) * 0.1);
  let r2 = vec3<f32>(0.5) + cos(uv * P9_PI * vec3<f32>(2.0, 2.0, 6.0)) * 0.5;
  let v = p9_turbulence(r2 * 5.0, 5u) + 0.5;
  var xx: vec3<f32>;
  xx.x = p9_turbulence(r2 * 5.0 + vec3<f32>(v) + vec3<f32>(1.2, 4.2, 1.2), 3u) * 0.125;
  xx.y = p9_turbulence(r2 * 5.0 + vec3<f32>(v) + vec3<f32>(2.2, 3.2, 2.2), 3u) * 0.125;
  xx.z = p9_turbulence(r2 * 5.0 + vec3<f32>(v) + vec3<f32>(1.1, 2.2, 1.4), 3u) * 0.125;
  var c = mix(
    vec3<f32>(1.0, 0.86, 0.94),
    vec3<f32>(0.1, 0.1, 0.2),
    pow(r2.x + xx.x, 0.2),
  ) * 1.75;
  c = c + mix(
    vec3<f32>(0.84, 0.91, 1.0),
    vec3<f32>(0.1, 0.1, 0.2),
    pow(r2.y + xx.y, 1.5),
  ) * 0.25;
  c = c + mix(
    vec3<f32>(0.91, 0.85, 1.0),
    vec3<f32>(0.1, 0.2, 0.2),
    pow(r2.z + xx.z, 1.5),
  ) * 0.25;

  // Glow parts
  let glow = p9_smoothstep(0.2, 0.5, p9_fbm(r2 * vec3<f32>(15.0, 1.0, 1.0), 3u))
    * u.in1.y
    * p9_smoothstep(0.05, 0.2, r);
  c = mix(c, vec3<f32>(0.953, 0.047, 0.090) * vec3<f32>(500.0), glow);

  c = c * p9_smoothstep(0.01, 0.05, r);
  c = c * p9_smoothstep(1.6, 0.5, length(p)); // Vignetting
  return vec4<f32>(v * c * p9_saturate(r) * 0.125, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::{AUDIO_SAMPLES, BandPayload};

    // Port of tests/unit/point10-light-tunnel.test.js.

    #[test]
    fn perm_starts_at_the_preset_seed() {
        assert_eq!(make_perm(1.0), 1.0);
        assert_eq!(make_perm(4.0), 4.0);
        assert_eq!(make_perm(f32::NAN), 1.0);
    }

    #[test]
    fn advances_perm_by_the_bass_band_and_the_creep() {
        let next = tunnel_drive(0.5, 0.0, 0.1, 1.0);
        assert!((next.perm - (1.0 + (0.5 * 3.0 + 0.01) * 0.1)).abs() < 1e-5);
        assert!(next.perm > 1.0);
    }

    #[test]
    fn creeps_slowly_in_silence_and_stays_bounded() {
        let next = tunnel_drive(0.0, 0.0, 1.0, 0.0);
        assert!(next.perm > 0.0);
        assert!(next.perm <= 0.01 + 1e-6, "long frames clamp to 0.1 s");
    }

    #[test]
    fn in1_x_is_perm_plus_the_seeded_offset() {
        let next = tunnel_drive(0.2, 0.0, 0.5, 2.0);
        assert!((next.in1_x - (next.perm + perm_rand() * 10.0)).abs() < 1e-5);
        assert!(next.in1_x > next.perm);
    }

    #[test]
    fn in1_y_clamps_band_one_to_one_and_ignores_negatives() {
        assert_eq!(tunnel_drive(0.0, 2.0, 0.1, 0.0).in1_y, 1.0);
        assert!((tunnel_drive(0.0, 0.4, 0.1, 0.0).in1_y - 0.4).abs() < 1e-6);
        assert_eq!(tunnel_drive(0.0, -1.0, 0.1, 0.0).in1_y, 0.0);
        assert_eq!(tunnel_drive(0.0, f32::NAN, 0.1, 0.0).in1_y, 0.0);
    }

    #[test]
    fn permrand_is_stable_and_in_the_unit_interval() {
        assert_eq!(perm_rand(), perm_rand());
        assert!((0.0..1.0).contains(&perm_rand()));
    }

    #[test]
    fn preset_geometry_and_bloom_match_the_js() {
        assert_eq!(TUNNEL_SIZE, 2.0);
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (0.17, 0.85, 0.0)
        );
    }

    #[test]
    fn shader_keeps_the_tunnel_constants() {
        let program = light_tunnel_wgsl();
        for needle in [
            "0.25 / r",
            "p9_turbulence",
            "0.953, 0.047, 0.090",
            "vec3<f32>(500.0)",
            "p9_fbm",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
    }

    #[test]
    fn uniforms_are_32_bytes() {
        assert_eq!(std::mem::size_of::<LightTunnelUniforms>(), 32);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = LightTunnel::new();
        scene.apply_props(|key| match key {
            "scrollSpeed" => Some(9.0),
            "glow" => Some(2.0),
            "bloom" => Some(-1.0),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.glow_scale, 2.0);
        assert_eq!(scene.bloom_scale, 0.0);
    }

    #[test]
    fn update_reads_the_bass_and_mid_bands() {
        let mut scene = LightTunnel::new();
        let bridge = AudioBridge::new();
        // No audio: `band(-1,0,0,0.025)` and `band(-1,1,1,0)` both fall back.
        scene.update(0.1, &bridge);
        assert!((scene.perm() - (1.0 + (0.025 * 3.0 + 0.01) * 0.1)).abs() < 1e-5);
        assert_eq!(scene.in1()[1], 0.0);

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 0.5; // bass, damping 0
        payload.lanes[4] = 0.9; // mid, damping 1 (`1 * 3 + 1`)
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let before = scene.perm();
        scene.update(0.1, &bridge);
        assert!((scene.perm() - before - (0.5 * 3.0 + 0.01) * 0.1).abs() < 1e-5);
        assert!((scene.in1()[1] - 0.9).abs() < 1e-5);
    }

    #[test]
    fn glow_prop_scales_only_the_glow_lane() {
        let mut scene = LightTunnel::new();
        scene.apply_props(|key| (key == "glow").then_some(2.0));

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[4] = 0.9;
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.1, &bridge);
        assert!((scene.in1()[1] - 1.8).abs() < 1e-5);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = LightTunnel::new();
        assert_eq!(scene.meta().name, "Point10 Light Tunnel");
        assert_eq!(scene.meta().category, "3d-scenes");
        assert_eq!(scene.meta().audio_features, &["bass", "mid", "treble"]);
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(&names[names.len() - 3..], &["scrollSpeed", "glow", "bloom"]);
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = light_tunnel_wgsl();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("LIGHT_TUNNEL_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("LIGHT_TUNNEL_WGSL must validate");
    }
}
