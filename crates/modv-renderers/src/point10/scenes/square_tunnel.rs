//! Square Tunnel — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/SquareTunnel.js`.
//!
//! # What the JS actually draws
//!
//! The Plane9 "Square Tunnel" preset (Joakim Dahl, CC BY-NC-SA 4.0) reduced to
//! one fullscreen fragment shader and a checkerboard texture. The shader builds
//! a squircle radius (`|p.x|^30 + |p.y|^30` to the `1/30`), turns it into polar
//! texture coordinates and samples the checkerboard with repeat wrapping, so
//! the square tube recedes forever. `Clear1` is **opaque black** — unlike the
//! other batch-2 scenes, this one does not composite over the layers below.
//! There is no bloom/composer in the preset chain, so three's single render is
//! all the JS does and no [`crate::point10::postfx::Composer`] is needed here.
//!
//! # Ported exactly
//!
//! * the shader maths (`SquareTunnel.js:60-82`): power-15 squircle radius,
//!   `uv.x = 0.3 * gIn1.y + 0.5 / r`, `uv.y = a / PI`, `texture(gTexture1,
//!   uv * 3.0)`, `col * r * (0.5 + gIn1.x) * length(p)`, alpha `1.0`;
//! * Expression1 (`squareTunnelDrive`, `:40-54`):
//!   `perm += (band(-1,0,0,0.1) * 20 + 0.01) * dt` with `dt` clamped to 0.1 s,
//!   `gIn1 = (beat(0) * 2, perm + PERMRAND * 10, 0, 0)`; `PERMRAND =
//!   seededRand(1, 0)`;
//! * the prop binder (`sceneProps.js:421-424`): `speedScale = scrollSpeed`,
//!   `brightnessScale = brightness`, applied to `gIn1.x` only — `gIn1.y` is
//!   the scroll phase and is not scaled by brightness in the JS;
//! * the opaque black clear (`:186`, `setClearColor(0x000000, 1)`).
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `PlaneGeometry(2, 2)` + clip-space vertex shader (`VERT`) | [`crate::point10::FULLSCREEN_TRIANGLE_WGSL`] + [`crate::point10::FULLSCREEN_UV_WGSL`]'s uv convention |
//! | `THREE.Vector4` uniform `gIn1` | [`SquareTunnelUniforms::in1`] |
//! | `loadTexture(checkerUrl)` + `SRGBColorSpace` + repeat wrap + generated mips | [`create_checkerboard_texture`]: a procedural `Rgba8UnormSrgb` checkerboard with a box-filtered mip chain and a repeat/trilinear sampler |
//! | `depthTest/depthWrite: false`, `DoubleSide` | one triangle, no depth attachment |
//!
//! # Deviations
//!
//! * **No asset loading.** `textures/checkerboard_diffuse.png` is a 256x256
//!   1-bit PNG whose only content is two 50 % blocks (black at the top-left,
//!   white at the top-right/bottom-left, black at the bottom-right). Asset
//!   loading is out of scope for the renderer crate (the engine hands scenes
//!   textures), so [`build_checkerboard`] rebuilds that pattern procedurally —
//!   the same 256² resolution and the same black/white palette, so the sampled
//!   result matches at every mip.
//! * `THREE.Texture` gets generated mipmaps (three's default filtering is
//!   `LinearMipmapLinearFilter`), which matters because `uv * 3.0` walks far
//!   past 1 and `0.5 / r` minifies heavily near the vanishing point. The Rust
//!   side builds the same box-filtered mip chain on the CPU.
//! * The JS sets `texture.anisotropy = maxAniso`; the Rust sampler keeps wgpu's
//!   default `anisotropy_clamp = 1`, because the engine's device limits are not
//!   visible here and `max_anisotropy` may legitimately be 1. The checkerboard
//!   is minified along the tunnel axis, so this is the one visible difference
//!   (slightly softer bands at grazing angles).
//! * The JS quad's `vUv` has `v = 0` at the **bottom** (PlaneGeometry); the
//!   fragment's `@builtin(position)` is top-left origin, so the shader flips
//!   `y` to keep the tunnel upright. Without the flip the pattern is mirrored
//!   vertically.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::shared::random::seeded_rand;
use crate::point10::{FULLSCREEN_TRIANGLE_WGSL, shared::p9_glsl::P9_COMMON_WGSL};
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, read_scene_props, scene_props_with_levers,
};

/// `SquareTunnel.js: export const TUNNEL_SIZE = 2` — clip space is -1..1, so a
/// 2x2 plane fills the frame. The Rust fullscreen triangle covers the same
/// area without a vertex buffer.
pub const TUNNEL_SIZE: f32 = 2.0;

/// `SquareTunnel.js: const MAX_DT = 0.1` — P9 deltatime clamps long frames so a
/// tab-suspend catch-up cannot jump the pattern.
pub const MAX_DT: f32 = 0.1;

/// `SquareTunnel.js: const PERMRAND = seededRand(1, 0)`.
///
/// A function rather than a `const` because [`seeded_rand`] is not `const fn`;
/// the value is stable, so the checkerboard phase is identical every run.
pub fn perm_rand() -> f32 {
    seeded_rand(1, 0)
}

/// `SquareTunnel.js: makePerm(seed = 1)` — the preset's `RandomSeed`.
pub fn make_perm(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { 1.0 }
}

/// One frame of Expression1 (`SquareTunnel.js:40-54`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SquareTunnelDrive {
    /// `perm` for the next frame.
    pub perm: f32,
    /// `gIn1.x` before the brightness prop: `beat * 2`.
    pub in1_x: f32,
    /// `gIn1.y`: `perm + PERMRAND * 10`, the checkerboard scroll phase.
    pub in1_y: f32,
}

/// Pure port of `squareTunnelDrive`.
///
/// `bass` is `P9Audio.band(-1, 0, 0, 0.1)`, `beat` is `P9Audio.beat(0)`,
/// `dt` is already multiplied by the scene's `speedScale` (both are what the
/// JS passes in).
pub fn square_tunnel_drive(bass: f32, beat: f32, dt: f32, perm: f32) -> SquareTunnelDrive {
    let bass = if bass.is_finite() { bass.max(0.0) } else { 0.0 };
    let beat = if beat.is_finite() {
        beat.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let step = if dt.is_finite() {
        dt.min(MAX_DT).max(0.0)
    } else {
        0.0
    };
    let base = make_perm(perm);
    // `perm += (band(-1, 0, 0, 0.1) * 20 + 0.01) * dt`
    let next = base + (bass * 20.0 + 0.01) * step;

    SquareTunnelDrive {
        perm: next,
        in1_x: beat * 2.0,
        in1_y: next + perm_rand() * 10.0,
    }
}

/// `SQUARE_TUNNEL_PROPS` (`sceneProps.js:122-125`).
const SQUARE_TUNNEL_PROPS: &[SceneProp] = &[
    SceneProp::float("scrollSpeed", "Scroll Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

/// Resolution of `textures/checkerboard_diffuse.png`.
pub const CHECKERBOARD_SIZE: u32 = 256;

/// Block edge of the 2x2 checker (the asset's palette blocks are 128 px).
pub const CHECKERBOARD_BLOCK: u32 = 128;

/// Procedural stand-in for `textures/checkerboard_diffuse.png`.
///
/// The asset is a 1-bit palette PNG (`000000`, `ffffff`) that is black at the
/// top-left, white at the top-right and bottom-left, black at the bottom-right.
/// This rebuilds it as tightly packed RGBA8 (`255` alpha throughout), at the
/// asset's 256² resolution and with the same 128 px blocks.
pub fn build_checkerboard(size: u32) -> Vec<u8> {
    let size = size.max(2);
    let block = (size / 2).max(1);
    let mut data = vec![0_u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let white = ((x / block) + (y / block)) % 2 == 1;
            let tone = if white { 255 } else { 0 };
            let index = ((y * size + x) * 4) as usize;
            data[index] = tone;
            data[index + 1] = tone;
            data[index + 2] = tone;
            data[index + 3] = 255;
        }
    }

    data
}

/// Box-filter one 2x2 step down, rounding like a GPU mip filter.
fn downsample_2x2(size: u32, source: &[u8]) -> (u32, Vec<u8>) {
    let source_size = size.max(1);
    let size = (source_size / 2).max(1);
    let mut data = vec![0_u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            for channel in 0..4 {
                let mut sum = 0_u32;
                let mut count = 0_u32;
                for offset_y in 0..2 {
                    for offset_x in 0..2 {
                        let sx = (x * 2 + offset_x).min(source_size - 1);
                        let sy = (y * 2 + offset_y).min(source_size - 1);
                        sum += source[((sy * source_size + sx) * 4) as usize + channel] as u32;
                        count += 1;
                    }
                }
                let average = ((sum + count / 2) / count) as u8;
                data[((y * size + x) * 4) as usize + channel] = average;
            }
        }
    }

    (size, data)
}

/// The checkerboard's full mip chain, level 0 first.
pub fn checkerboard_mips(size: u32) -> Vec<(u32, Vec<u8>)> {
    let mut levels = vec![(size.max(2), build_checkerboard(size))];
    while let Some((level_size, level)) = levels.last() {
        if *level_size <= 1 {
            break;
        }
        levels.push(downsample_2x2(*level_size, level));
    }
    levels
}

/// Upload the procedural checkerboard as an sRGB, mipmapped, repeat-wrapped
/// texture (`THREE.SRGBColorSpace` + `RepeatWrapping` + generated mipmaps).
pub fn create_checkerboard_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let levels = checkerboard_mips(CHECKERBOARD_SIZE);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("point10-square-tunnel-checkerboard"),
        size: wgpu::Extent3d {
            width: levels[0].0,
            height: levels[0].0,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // three decodes an SRGBColorSpace texture to linear before sampling.
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (level, (size, data)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(*size),
            },
            wgpu::Extent3d {
                width: *size,
                height: *size,
                depth_or_array_layers: 1,
            },
        );
    }

    texture
}

/// Repeat-wrapped, trilinear sampler (`RepeatWrapping` + `LinearMipmapLinear`).
fn checkerboard_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("point10-square-tunnel-sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

/// Uniform block, matching [`SQUARE_TUNNEL_WGSL`] (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SquareTunnelUniforms {
    /// `gIn1` (`x` = brightness * beat * 2, `y` = scroll phase).
    in1: [f32; 4],
    /// Destination size, for the fragment's `uv`.
    resolution: [f32; 2],
    _pad: [f32; 2],
}

impl SquareTunnelUniforms {
    fn new(width: u32, height: u32, in1: [f32; 4]) -> Self {
        Self {
            in1,
            resolution: [width.max(1) as f32, height.max(1) as f32],
            _pad: [0.0; 2],
        }
    }
}

/// GPU resources built in `init`.
struct SquareTunnelGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    /// Kept alive for `dispose` clarity; the bind group already holds a view.
    _texture: wgpu::Texture,
}

/// SquareTunnel scene.
pub struct SquareTunnel {
    /// `this.p9audio`.
    audio: P10Audio,
    /// `this.perm`.
    perm: f32,
    /// `this.speedScale` (`scrollSpeed`).
    speed_scale: f32,
    /// `this.brightnessScale` (`brightness`).
    brightness_scale: f32,
    /// `gIn1` as last uploaded.
    in1: [f32; 4],
    size: (u32, u32),
    gpu: Option<SquareTunnelGpu>,
}

impl Default for SquareTunnel {
    fn default() -> Self {
        Self::new()
    }
}

impl SquareTunnel {
    /// Create the scene (`SquareTunnel.js:144-153` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            perm: make_perm(1.0),
            speed_scale: 1.0,
            brightness_scale: 1.0,
            in1: [0.0; 4],
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:421-424`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(SQUARE_TUNNEL_PROPS, props) {
            match key {
                "scrollSpeed" => self.speed_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// Current scroll phase (`this.perm`).
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// The `gIn1` uniform as last uploaded (for tests).
    pub fn in1(&self) -> [f32; 4] {
        self.in1
    }
}

impl Point10Scene for SquareTunnel {
    fn name(&self) -> &'static str {
        "Point10 Square Tunnel"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-square-tunnel-shader"),
            source: wgpu::ShaderSource::Wgsl(square_tunnel_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-square-tunnel-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
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

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-square-tunnel-uniforms"),
            size: std::mem::size_of::<SquareTunnelUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let texture = create_checkerboard_texture(&device, &queue);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = checkerboard_sampler(&device);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-square-tunnel-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-square-tunnel-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline =
            ctx.fullscreen_pipeline("point10-square-tunnel", &shader, &pipeline_layout, None);

        queue.write_buffer(
            &uniform_buffer,
            0,
            bytemuck::bytes_of(&SquareTunnelUniforms::new(
                self.size.0,
                self.size.1,
                self.in1,
            )),
        );

        self.gpu = Some(SquareTunnelGpu {
            queue,
            pipeline,
            bind_group,
            uniform_buffer,
            _texture: texture,
        });
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        // `this.p9audio.update(dt)`
        self.audio.update(dt, audio);

        // Expression1: bass advances the scroll phase, beat flashes the tube.
        let drive = square_tunnel_drive(
            self.audio.band(-1, 0, 0, 0.1),
            self.audio.beat(0.0),
            dt * self.speed_scale,
            self.perm,
        );
        self.perm = drive.perm;
        // `gIn1.set(drive.in1.x * this.brightnessScale, drive.in1.y, 0, 0)`
        self.in1 = [drive.in1_x * self.brightness_scale, drive.in1_y, 0.0, 0.0];
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-square-tunnel"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // `Clear1`: black, opaque (`:186`, `setClearColor(0x000000, 1)`).
                    // The quad covers the frame; the clear keeps the scene opaque
                    // at the edges/in a resize.
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(SQUARE_TUNNEL_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10SquareTunnel.js` sets no category, so the adapter default wins.
        SceneMeta::for_scene("Point10 Square Tunnel", "3d-scenes")
    }
}

impl SquareTunnel {
    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.uniform_buffer,
            0,
            bytemuck::bytes_of(&SquareTunnelUniforms::new(
                self.size.0,
                self.size.1,
                self.in1,
            )),
        );
    }
}

/// The fullscreen tunnel shader (`SquareTunnel.js:60-82`), with the shared
/// Plane9 helpers in front of it for `P9_PI`.
fn square_tunnel_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{P9_COMMON_WGSL}\n{SQUARE_TUNNEL_WGSL}")
}

const SQUARE_TUNNEL_WGSL: &str = r#"
struct SquareTunnelUniforms {
  in1: vec4<f32>,
  resolution: vec2<f32>,
  _pad: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: SquareTunnelUniforms;
@group(0) @binding(1) var checker: texture_2d<f32>;
@group(0) @binding(2) var checker_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv01 = position.xy / u.resolution;
  // PlaneGeometry's vUv has v = 0 at the bottom; the fragment position has it
  // at the top, so flip y to keep the tunnel upright.
  let p = vec2<f32>(uv01.x * 2.0 - 1.0, 1.0 - uv01.y * 2.0);
  let a = atan2(p.y, p.x);

  // Shader1, verbatim: power-15 squircle radius -> polar texture lookup.
  let power = 15.0;
  let r = pow(pow(p.x * p.x, power) + pow(p.y * p.y, power), 1.0 / (2.0 * power));
  let uv = vec2<f32>(0.3 * u.in1.y + 0.5 / r, a / P9_PI);

  let col = textureSample(checker, checker_sampler, uv * 3.0).rgb;
  return vec4<f32>(col * r * (0.5 + u.in1.x) * length(p), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::{AUDIO_SAMPLES, BandPayload};

    // Port of tests/unit/point10-square-tunnel.test.js.

    #[test]
    fn perm_starts_at_the_preset_seed_and_falls_back() {
        assert_eq!(make_perm(1.0), 1.0);
        assert_eq!(make_perm(4.0), 4.0);
        assert_eq!(make_perm(f32::NAN), 1.0);
        assert!((square_tunnel_drive(0.0, 0.0, 0.1, f32::NAN).perm - 1.001).abs() < 1e-5);
    }

    #[test]
    fn advances_perm_by_bass_times_20_plus_the_creep() {
        let next = square_tunnel_drive(0.5, 0.0, 0.1, 1.0);
        assert!((next.perm - (1.0 + (0.5 * 20.0 + 0.01) * 0.1)).abs() < 1e-5);
        assert!(next.perm > 1.0);
    }

    #[test]
    fn creeps_in_silence_and_ignores_negative_bass_or_dt() {
        let silent = square_tunnel_drive(0.0, 0.0, 0.1, 0.0);
        let long_frame = square_tunnel_drive(0.0, 0.0, 1.0, 0.0);
        let negative = square_tunnel_drive(-2.0, 0.0, -1.0, 1.0);

        assert!((silent.perm - 0.001).abs() < 1e-6);
        // long frames are capped at 0.1 s like the P9 deltatime
        assert!((long_frame.perm - silent.perm).abs() < 1e-6);
        assert!((negative.perm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn in1_x_is_beat_times_two_clamped_to_the_p9_envelope() {
        assert!((square_tunnel_drive(0.0, 0.75, 0.1, 0.0).in1_x - 1.5).abs() < 1e-6);
        assert_eq!(square_tunnel_drive(0.0, 1.0, 0.1, 0.0).in1_x, 2.0);
        assert_eq!(square_tunnel_drive(0.0, -1.0, 0.1, 0.0).in1_x, 0.0);
        assert_eq!(square_tunnel_drive(0.0, f32::NAN, 0.1, 0.0).in1_x, 0.0);
    }

    #[test]
    fn in1_y_is_the_advanced_perm_plus_the_seeded_offset() {
        let next = square_tunnel_drive(0.2, 0.5, 0.5, 2.0);
        assert!((next.in1_y - (next.perm + perm_rand() * 10.0)).abs() < 1e-5);
        assert!((next.in1_x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn permrand_is_stable_and_in_the_unit_interval() {
        assert_eq!(perm_rand(), perm_rand());
        assert!((0.0..1.0).contains(&perm_rand()));
    }

    #[test]
    fn checkerboard_rebuilds_the_asset_pattern() {
        let map = build_checkerboard(CHECKERBOARD_SIZE);
        assert_eq!(
            map.len(),
            (CHECKERBOARD_SIZE * CHECKERBOARD_SIZE * 4) as usize
        );

        let pixel = |x: u32, y: u32| map[((y * CHECKERBOARD_SIZE + x) * 4) as usize];
        assert_eq!(pixel(10, 10), 0, "top-left block is black");
        assert_eq!(pixel(200, 10), 255, "top-right block is white");
        assert_eq!(pixel(10, 200), 255, "bottom-left block is white");
        assert_eq!(pixel(200, 200), 0, "bottom-right block is black");
        // Fully opaque: the shader writes its own alpha.
        assert_eq!(map[3], 255);
    }

    #[test]
    fn mip_chain_halves_to_one_pixel_and_stays_opaque() {
        let levels = checkerboard_mips(CHECKERBOARD_SIZE);
        assert_eq!(levels.len(), 9, "256 -> 1 is nine levels");
        assert_eq!(levels[0].0, 256);
        assert_eq!(levels[8].0, 1);
        for (size, data) in &levels {
            assert_eq!(data.len(), (size * size * 4) as usize, "level {size}");
            for pixel in data.chunks_exact(4) {
                assert_eq!(pixel[3], 255, "mips must keep the texture opaque");
            }
        }
        // The last level averages the 50/50 pattern, so it stays mid-grey.
        let top = levels[8].1[0];
        assert!((top as i32 - 128).abs() <= 2, "1x1 mip is {top}");
    }

    #[test]
    fn uniforms_are_32_bytes() {
        assert_eq!(std::mem::size_of::<SquareTunnelUniforms>(), 32);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = SquareTunnel::new();
        scene.apply_props(|key| match key {
            "scrollSpeed" => Some(9.0),
            "brightness" => Some(-1.0),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.brightness_scale, 0.0);

        let mut defaults = SquareTunnel::new();
        defaults.apply_props(|_| None);
        assert_eq!(defaults.speed_scale, 1.0);
        assert_eq!(defaults.brightness_scale, 1.0);
    }

    #[test]
    fn update_drives_the_shader_from_the_audio_bridge() {
        let mut scene = SquareTunnel::new();
        // No audio: `band(-1,0,0,0.1)` returns 0.1 and `beat(0)` returns 0, so
        // the phase creeps at (0.1 * 20 + 0.01) * dt and the flash stays off.
        let bridge = AudioBridge::new();
        scene.update(0.1, &bridge);
        assert!((scene.perm() - (1.0 + (0.1 * 20.0 + 0.01) * 0.1)).abs() < 1e-5);
        assert_eq!(scene.in1()[0], 0.0);

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 0.5;
        payload.beat = 1.0;
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        payload.wave = [0.0; AUDIO_SAMPLES];
        payload.freq_mag = [0.0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let before = scene.perm();
        scene.update(0.1, &bridge);
        assert!((scene.perm() - before - (0.5 * 20.0 + 0.01) * 0.1).abs() < 1e-4);
        assert!((scene.in1()[0] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn brightness_scales_only_the_beat_term() {
        let mut scene = SquareTunnel::new();
        scene.apply_props(|key| (key == "brightness").then_some(0.5));

        let mut payload = BandPayload::silent(44_100.0);
        payload.beat = 1.0;
        payload.byte_frequency_data = vec![0; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.1, &bridge);
        assert!((scene.in1()[0] - 1.0).abs() < 1e-5, "beat * 2 * 0.5");
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = SquareTunnel::new();
        assert_eq!(scene.meta().name, "Point10 Square Tunnel");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(&names[names.len() - 2..], &["scrollSpeed", "brightness"]);
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = square_tunnel_wgsl();
        for needle in [
            "let power = 15.0",
            "a / P9_PI",
            "uv * 3.0",
            "0.5 + u.in1.x",
            "length(p)",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }

        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("SQUARE_TUNNEL_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("SQUARE_TUNNEL_WGSL must validate");
    }

    #[test]
    fn pi_constant_matches_the_preset_define() {
        let pi = crate::point10::shared::p9_glsl::PI;
        assert!((pi - 3.141_592_7).abs() < 1e-6);
        assert_eq!(TUNNEL_SIZE, 2.0);
    }
}
