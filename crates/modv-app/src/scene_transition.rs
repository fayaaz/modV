//! Whole-scene shader transitions (WS-8 "Whole-scene transition").
//!
//! Ports the six native ISF transitions from
//! `src/renderer/src/application/scene-transition-defs.js` (`NATIVE_TRANSITIONS`)
//! to WGSL and runs them over two full-resolution frames: the **captured**
//! outgoing Scene (`startImage`) and the incoming Scene the compositor just
//! rendered into its main texture (`endImage`). `Fade` is deliberately absent —
//! it stays the existing per-Group alpha ghost path in [`crate::performance`],
//! and `None` stays an instant switch.
//!
//! ## Shader ports
//!
//! Every effect is the same WGSL prefix (uniform block, `start`/`end` textures,
//! clamp/linear sampler, fullscreen triangle) plus one fragment entry point.
//! The maths is ported line-for-line from the ISF bodies:
//!
//! * **Three Crossfade** — `smoothstep(0, 1, progress)` mix of the two frames.
//! * **Three Dissolve** — a 32 × 18 hash grid; a cell flips to the incoming
//!   frame when its noise enters the `progress * 1.4 - 0.2` … `+0.14` band.
//! * **Three Pixelate** — both frames are point-sampled at
//!   `max(1, floor(size / 9))` cells per axis (9 px cells) and crossfaded.
//! * **Three Glitch** — 24 horizontal bands; bands whose hash exceeds 0.72
//!   shift by up to 0.08 uv, and the incoming frame's red/blue channels are
//!   displaced along `x` by the same amount while the two frames crossfade.
//! * **Three Wipe** — a hard edge: `step(uv.x, p)` selects the incoming frame
//!   for every column left of the eased progress.
//! * **Three Circle** — an aspect-corrected radial wipe: the incoming frame
//!   fills a disc of radius `smoothstep(0, 1, progress) * 1.2` with a 0.08
//!   soft edge.
//!
//! `uv` comes from `@builtin(position) / resolution`, so both inputs are
//! sampled in output orientation (the same convention [`crate::compositor`]'s
//! present blit uses) and a shader that writes the incoming frame untouched
//! looks exactly like the Fade path at progress 1.
//!
//! ## Capture and lifecycle
//!
//! The compositor's main texture after a presented frame *is* the outgoing
//! Scene, so [`crate::compositor::Compositor::capture_transition_source`]
//! copies it once into a persistent `from` texture before the first frame of
//! the incoming Scene overwrites main. The pass keeps a **single** output-sized
//! scratch target (the JS `getTransitionRenderSize` instead caps the transition
//! at 1920 × 1080 pixels; the port renders at output size, the documented
//! trade-off being fill rate for exactness) and one pipeline per effect, built
//! lazily. Resize drops the scratch (and the compositor drops the capture);
//! reset drops the whole pass.
//!
//! The shader path composites the two Scene frames itself, so the outgoing
//! Groups are **hidden** (not faded) while it runs: they stay in the state at
//! their recorded alpha but the compositor skips them until the transition
//! completes and removes them. That keeps the Fade fallback intact if the
//! capture ever disappears, without running two animations at once.

use modv_render::wgpu;

use crate::compositor::{OutputTexture, create_output_texture};

/// The internal texture format every transition target uses (the compositor's
/// group/main format).
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// One native whole-scene transition (`scene-transition-defs.js:19-125`).
///
/// `Fade` and `None` are intentionally not variants: the first is the alpha
/// ghost path, the second an instant switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransitionEffect {
    /// `Three Crossfade`.
    Crossfade,
    /// `Three Dissolve`.
    Dissolve,
    /// `Three Pixelate`.
    Pixelate,
    /// `Three Glitch`.
    Glitch,
    /// `Three Wipe`.
    Wipe,
    /// `Three Circle`.
    Circle,
}

impl TransitionEffect {
    /// Number of shader effects (one pipeline each).
    pub const COUNT: usize = 6;

    /// Every effect, in `NATIVE_TRANSITION_NAMES` order
    /// (`scene-transition-defs.js:123-125`).
    pub const ALL: [Self; Self::COUNT] = [
        Self::Crossfade,
        Self::Dissolve,
        Self::Pixelate,
        Self::Glitch,
        Self::Wipe,
        Self::Circle,
    ];

    /// The effect's index in [`Self::ALL`] (pipeline slot).
    pub fn index(self) -> usize {
        match self {
            Self::Crossfade => 0,
            Self::Dissolve => 1,
            Self::Pixelate => 2,
            Self::Glitch => 3,
            Self::Wipe => 4,
            Self::Circle => 5,
        }
    }

    /// The ISF/native name (`scene-transition-defs.js:21`, `:31`, …).
    pub fn name(self) -> &'static str {
        match self {
            Self::Crossfade => "Three Crossfade",
            Self::Dissolve => "Three Dissolve",
            Self::Pixelate => "Three Pixelate",
            Self::Glitch => "Three Glitch",
            Self::Wipe => "Three Wipe",
            Self::Circle => "Three Circle",
        }
    }

    /// Map a picked transition name to a shader effect.
    ///
    /// Returns `None` for `Fade`, `None` and any unknown/custom ISF name: the
    /// caller keeps Fade for those, which is the JS `fallbackToFade` path.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|effect| effect.name() == name)
    }
}

/// A whole-scene shader transition handed to the compositor for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneTransitionInfo {
    /// The shader to run.
    pub effect: TransitionEffect,
    /// Timeline position in `[0, 1]` (`transition.progress()`), un-eased in
    /// exactly the same way the Fade path is: every effect applies its own
    /// `smoothstep` where the ISF body did.
    pub progress: f64,
    /// Outgoing Scene Group ids. The compositor hides them while the shader
    /// owns the frame; the captured `from` texture stands in for them.
    pub from_groups: Vec<String>,
}

/// Clamp a transition progress to the `0..=1` the shaders expect.
///
/// A non-finite value becomes 0, matching the JS
/// `Math.min(1, Math.max(0, progress))` guards on a `NaN` input.
pub fn clamp_progress(progress: f64) -> f32 {
    if progress.is_finite() {
        progress.clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

/// The uniform block shared by every effect (`resolution`, `progress`).
///
/// 16 bytes because a `vec2<f32>` followed by an `f32` is padded to a 16-byte
/// block; the explicit `_pad` makes the layout obvious instead of implicit.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TransitionUniforms {
    /// Output size in pixels (`RENDERSIZE`).
    pub resolution: [f32; 2],
    /// Clamped transition progress.
    pub progress: f32,
    /// Unused padding (keeps the block at 16 bytes).
    pub _pad: f32,
}

impl TransitionUniforms {
    /// The 16-byte little-endian payload (no `bytemuck` dependency here).
    pub fn to_bytes(self) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[0..4].copy_from_slice(&self.resolution[0].to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.resolution[1].to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.progress.to_ne_bytes());
        bytes[12..16].copy_from_slice(&self._pad.to_ne_bytes());
        bytes
    }
}

/// The declarations, vertex stage and helpers every effect shares.
const TRANSITION_PREFIX: &str = r#"// modV whole-scene transition — WGSL port of the native ISF transitions.
struct TransitionUniforms {
  resolution: vec2<f32>,
  progress: f32,
  _pad: f32,
};

@group(0) @binding(0) var<uniform> uniforms: TransitionUniforms;
@group(0) @binding(1) var start_texture: texture_2d<f32>;
@group(0) @binding(2) var end_texture: texture_2d<f32>;
@group(0) @binding(3) var transition_sampler: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0),
  );
  return vec4<f32>(positions[vertex_index], 0.0, 1.0);
}

// `@builtin(position)` is in framebuffer space (y down), so dividing by the
// output size samples both inputs in output orientation.
fn transition_uv(position: vec4<f32>) -> vec2<f32> {
  return position.xy / uniforms.resolution;
}

fn hash2(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}
"#;

/// `Three Crossfade` (`scene-transition-defs.js:20-30`).
const CROSSFADE_BODY: &str = r#"
@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = transition_uv(position);
  let p = smoothstep(0.0, 1.0, clamp(uniforms.progress, 0.0, 1.0));
  let outgoing = textureSample(start_texture, transition_sampler, uv);
  let incoming = textureSample(end_texture, transition_sampler, uv);
  return mix(outgoing, incoming, p);
}
"#;

/// `Three Dissolve` (`scene-transition-defs.js:32-50`).
const DISSOLVE_BODY: &str = r#"
@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = transition_uv(position);
  let cell = floor(uv * vec2<f32>(32.0, 18.0));
  let noise = hash2(cell);
  let edge = clamp(uniforms.progress, 0.0, 1.0) * 1.4 - 0.2;
  let incoming = smoothstep(edge, edge + 0.14, noise);
  let outgoing = textureSample(start_texture, transition_sampler, uv);
  let new_frame = textureSample(end_texture, transition_sampler, uv);
  return mix(outgoing, new_frame, 1.0 - incoming);
}
"#;

/// `Three Pixelate` (`scene-transition-defs.js:51-69`).
const PIXELATE_BODY: &str = r#"
fn pixel_cells(uv: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
  let count = max(vec2<f32>(1.0, 1.0), floor(size / 9.0));
  return (floor(uv * count) + 0.5) / count;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = transition_uv(position);
  let from_uv = pixel_cells(uv, vec2<f32>(textureDimensions(start_texture)));
  let to_uv = pixel_cells(uv, vec2<f32>(textureDimensions(end_texture)));
  let p = smoothstep(0.0, 1.0, clamp(uniforms.progress, 0.0, 1.0));
  let outgoing = textureSample(start_texture, transition_sampler, from_uv);
  let incoming = textureSample(end_texture, transition_sampler, to_uv);
  return mix(outgoing, incoming, p);
}
"#;

/// `Three Glitch` (`scene-transition-defs.js:70-92`).
const GLITCH_BODY: &str = r#"
@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let p = clamp(uniforms.progress, 0.0, 1.0);
  let base_uv = transition_uv(position);
  let band = floor(base_uv.y * 24.0);
  let seed = floor(uniforms.progress * 12.0);
  let amount = step(0.72, hash2(vec2<f32>(band, seed))) *
    (1.0 - abs(p * 2.0 - 1.0)) * 0.08;
  let uv = vec2<f32>(
    base_uv.x + (hash2(vec2<f32>(band, seed + 1.0)) - 0.5) * amount,
    base_uv.y,
  );

  let outgoing = textureSample(start_texture, transition_sampler, uv);
  let incoming = textureSample(end_texture, transition_sampler, uv);
  let red = textureSample(
    end_texture,
    transition_sampler,
    vec2<f32>(uv.x + amount, uv.y),
  ).r;
  let blue = textureSample(
    end_texture,
    transition_sampler,
    vec2<f32>(uv.x - amount, uv.y),
  ).b;
  let displaced = vec4<f32>(red, incoming.g, blue, incoming.a);
  return mix(outgoing, displaced, smoothstep(0.0, 1.0, p));
}
"#;

/// `Three Wipe` (`scene-transition-defs.js:93-105`).
const WIPE_BODY: &str = r#"
@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = transition_uv(position);
  let p = smoothstep(0.0, 1.0, clamp(uniforms.progress, 0.0, 1.0));
  let incoming = step(uv.x, p);
  let outgoing = textureSample(start_texture, transition_sampler, uv);
  let new_frame = textureSample(end_texture, transition_sampler, uv);
  return mix(outgoing, new_frame, incoming);
}
"#;

/// `Three Circle` (`scene-transition-defs.js:106-120`).
const CIRCLE_BODY: &str = r#"
@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = transition_uv(position);
  let aspect = uniforms.resolution.x / max(uniforms.resolution.y, 1.0);
  let centred = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(aspect, 1.0);
  let radius = smoothstep(0.0, 1.0, clamp(uniforms.progress, 0.0, 1.0)) * 1.2;
  let incoming = 1.0 - smoothstep(radius - 0.08, radius + 0.08, length(centred));
  let outgoing = textureSample(start_texture, transition_sampler, uv);
  let new_frame = textureSample(end_texture, transition_sampler, uv);
  return mix(outgoing, new_frame, incoming);
}
"#;

/// The complete WGSL program for one effect.
pub(crate) fn shader_source(effect: TransitionEffect) -> String {
    let body = match effect {
        TransitionEffect::Crossfade => CROSSFADE_BODY,
        TransitionEffect::Dissolve => DISSOLVE_BODY,
        TransitionEffect::Pixelate => PIXELATE_BODY,
        TransitionEffect::Glitch => GLITCH_BODY,
        TransitionEffect::Wipe => WIPE_BODY,
        TransitionEffect::Circle => CIRCLE_BODY,
    };
    format!("{TRANSITION_PREFIX}{body}")
}

/// The GPU side of whole-scene shader transitions: one pipeline per effect, a
/// shared uniform buffer and a single output-sized scratch target.
pub(crate) struct SceneTransitionPass {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    pipelines: [Option<wgpu::RenderPipeline>; TransitionEffect::COUNT],
    /// Effects whose pipeline could not be built; never retried.
    failed: [bool; TransitionEffect::COUNT],
    scratch: Option<OutputTexture>,
    /// Size of the current scratch target and uniform `resolution`.
    size: (u32, u32),
}

impl SceneTransitionPass {
    /// Build the pass against the compositor's device and queue.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("modv-scene-transition-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // Clamp like the ISF image inputs: a glitch offset that leaves the
        // frame samples the edge texel instead of wrapping to the far side.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modv-scene-transition-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modv-scene-transition-uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device: device.clone(),
            queue: queue.clone(),
            layout,
            sampler,
            uniforms,
            pipelines: Default::default(),
            failed: [false; TransitionEffect::COUNT],
            scratch: None,
            size: (0, 0),
        }
    }

    /// Ensure the scratch target matches `width × height`, dropping it when the
    /// output resized.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if self.size == (width, height) && self.scratch.is_some() {
            return;
        }
        self.size = (width, height);
        self.scratch = (width > 0 && height > 0).then(|| {
            create_output_texture(&self.device, "modv-scene-transition-target", width, height)
        });
    }

    /// The scratch view the compositor hands to [`Self::encode`] as its
    /// destination.
    pub(crate) fn target_view(&self) -> Option<wgpu::TextureView> {
        self.scratch.as_ref().map(|target| target.view.clone())
    }

    /// The scratch texture, so the caller can copy the result back into main.
    pub(crate) fn target_texture(&self) -> Option<wgpu::Texture> {
        self.scratch.as_ref().map(|target| target.texture.clone())
    }

    /// Drop the scratch target (resize, transition end).
    pub(crate) fn drop_scratch(&mut self) {
        self.scratch = None;
        self.size = (0, 0);
    }

    /// Run one effect into `destination`.
    ///
    /// Both inputs must be the current output size; the pass records a
    /// validation-time mismatch as a failure so the caller can fall back to
    /// Fade. Returns `false` when the effect's pipeline could not be built.
    pub(crate) fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        effect: TransitionEffect,
        from: &wgpu::TextureView,
        to: &wgpu::TextureView,
        progress: f64,
        destination: &wgpu::TextureView,
    ) -> bool {
        let (width, height) = self.size;
        if width == 0 || height == 0 || !self.ensure_pipeline(effect) {
            return false;
        }
        let Some(pipeline) = &self.pipelines[effect.index()] else {
            return false;
        };

        let uniforms = TransitionUniforms {
            resolution: [width as f32, height as f32],
            progress: clamp_progress(progress),
            _pad: 0.0,
        };
        self.queue
            .write_buffer(&self.uniforms, 0, &uniforms.to_bytes());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("modv-scene-transition-bind-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(from),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(to),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("modv-scene-transition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        true
    }

    /// Build the effect's pipeline on first use; remember a failure.
    fn ensure_pipeline(&mut self, effect: TransitionEffect) -> bool {
        let index = effect.index();
        if self.pipelines[index].is_some() {
            return true;
        }
        if self.failed[index] {
            return false;
        }

        let source = shader_source(effect);
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("modv-scene-transition-shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("modv-scene-transition-pipeline-layout"),
                bind_group_layouts: &[Some(&self.layout)],
                immediate_size: 0,
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("modv-scene-transition-pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: OUTPUT_FORMAT,
                        // The shader produces the finished frame, including
                        // the mixed alpha; writing replaces the scratch.
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines[index] = Some(pipeline);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::read_back_texture;
    use modv_engine::scene_transition_pick::{
        FADE_TRANSITION, NATIVE_TRANSITION_NAMES, NONE_TRANSITION,
    };
    use modv_render::wgpu::naga;
    use modv_render::{Engine, EngineConfig};

    const WIDTH: u32 = 128;
    const HEIGHT: u32 = 72;

    #[test]
    fn uniform_block_is_sixteen_bytes() {
        assert_eq!(std::mem::size_of::<TransitionUniforms>(), 16);
        let bytes = TransitionUniforms {
            resolution: [128.0, 72.0],
            progress: 0.5,
            _pad: 0.0,
        }
        .to_bytes();
        assert_eq!(bytes.len(), 16);
        let first = f32::from_ne_bytes(bytes[0..4].try_into().unwrap());
        let second = f32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        let progress = f32::from_ne_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!((first, second, progress), (128.0, 72.0, 0.5));
    }

    #[test]
    fn progress_is_clamped_to_the_unit_interval() {
        assert_eq!(clamp_progress(-3.0), 0.0);
        assert_eq!(clamp_progress(0.25), 0.25);
        assert_eq!(clamp_progress(4.0), 1.0);
        assert_eq!(clamp_progress(f64::NAN), 0.0);
        assert_eq!(clamp_progress(f64::INFINITY), 0.0);
    }

    #[test]
    fn effect_names_map_to_the_defs_module_and_fade_stays_out() {
        assert_eq!(TransitionEffect::ALL.len(), NATIVE_TRANSITION_NAMES.len());
        for (effect, name) in TransitionEffect::ALL.iter().zip(NATIVE_TRANSITION_NAMES) {
            assert_eq!(effect.name(), *name);
            assert_eq!(TransitionEffect::from_name(name), Some(*effect));
        }
        assert_eq!(TransitionEffect::from_name(FADE_TRANSITION), None);
        assert_eq!(TransitionEffect::from_name(NONE_TRANSITION), None);
        assert_eq!(TransitionEffect::from_name("E2E Missing Transition"), None);
        // Names are matched exactly, like the JS `Map` lookup.
        assert_eq!(TransitionEffect::from_name("three wipe"), None);
    }

    #[test]
    fn every_effect_program_parses_and_validates() {
        for effect in TransitionEffect::ALL {
            let source = shader_source(effect);
            let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| {
                panic!(
                    "{effect:?} does not parse:\n{}",
                    error.emit_to_string(&source)
                )
            });
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{effect:?} is not valid: {error:?}"));
        }
    }

    fn try_engine() -> Option<Engine> {
        match Engine::new(&EngineConfig {
            force_fallback_adapter: true,
        }) {
            Ok(engine) => Some(engine),
            Err(error) => {
                eprintln!("skipping scene-transition pixel test: no adapter ({error})");
                None
            }
        }
    }

    fn solid_frame(engine: &Engine, label: &str, pixel: [u8; 4]) -> wgpu::Texture {
        let texture = engine.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut data = vec![0_u8; (WIDTH * HEIGHT * 4) as usize];
        for chunk in data.as_chunks_mut::<4>().0 {
            *chunk = pixel;
        }
        engine.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    fn run_effect(
        engine: &Engine,
        pass: &mut SceneTransitionPass,
        effect: TransitionEffect,
        from: &wgpu::TextureView,
        to: &wgpu::TextureView,
        progress: f64,
    ) -> Vec<u8> {
        pass.resize(WIDTH, HEIGHT);
        let destination = pass.target_view().expect("scratch target exists");
        let mut encoder = engine
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("scene-transition-test"),
            });
        assert!(
            pass.encode(&mut encoder, effect, from, to, progress, &destination),
            "{effect:?} encodes"
        );
        engine.queue.submit(Some(encoder.finish()));
        let target = pass.target_texture().expect("scratch target exists");
        read_back_texture(&engine.device, &engine.queue, &target, WIDTH, HEIGHT)
            .expect("transition target reads back")
    }

    /// Mean of one channel over an optional column range.
    fn mean_channel(pixels: &[u8], channel: usize, columns: Option<(u32, u32)>) -> f64 {
        let range = columns.unwrap_or((0, WIDTH));
        let mut total = 0.0;
        let mut count = 0_u32;
        for y in 0..HEIGHT {
            for x in range.0..range.1 {
                let index = ((y * WIDTH + x) * 4) as usize + channel;
                total += f64::from(pixels[index]);
                count += 1;
            }
        }
        total / f64::from(count)
    }

    fn assert_dominant(pixels: &[u8], red: bool, context: &str) {
        let r = mean_channel(pixels, 0, None);
        let b = mean_channel(pixels, 2, None);
        if red {
            assert!(
                r > 200.0 && b < 45.0,
                "{context}: expected a red frame (r={r:.1}, b={b:.1})"
            );
        } else {
            assert!(
                b > 200.0 && r < 45.0,
                "{context}: expected a blue frame (r={r:.1}, b={b:.1})"
            );
        }
    }

    #[test]
    fn each_effect_moves_from_red_to_blue_through_a_mix() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut pass = SceneTransitionPass::new(&engine.device, &engine.queue);
        let red = solid_frame(&engine, "transition-red", [255, 0, 0, 255]);
        let blue = solid_frame(&engine, "transition-blue", [0, 0, 255, 255]);
        let red_view = red.create_view(&wgpu::TextureViewDescriptor::default());
        let blue_view = blue.create_view(&wgpu::TextureViewDescriptor::default());

        for effect in TransitionEffect::ALL {
            let start = run_effect(&engine, &mut pass, effect, &red_view, &blue_view, 0.0);
            assert_dominant(&start, true, &format!("{effect:?} at progress 0"));

            let end = run_effect(&engine, &mut pass, effect, &red_view, &blue_view, 1.0);
            assert_dominant(&end, false, &format!("{effect:?} at progress 1"));

            let middle = run_effect(&engine, &mut pass, effect, &red_view, &blue_view, 0.5);
            let r = mean_channel(&middle, 0, None);
            let b = mean_channel(&middle, 2, None);
            if effect == TransitionEffect::Crossfade
                || effect == TransitionEffect::Pixelate
                || effect == TransitionEffect::Circle
            {
                // Smooth blends: both frames are present in bulk.
                assert!(
                    (100.0..=156.0).contains(&r) && (100.0..=156.0).contains(&b),
                    "{effect:?} at 0.5 mixes evenly (r={r:.1}, b={b:.1})"
                );
            } else {
                // Dissolve/glitch pick per-cell or per-band: both frames cover
                // a meaningful share even though the split is not even.
                assert!(
                    r > 30.0 && b > 30.0,
                    "{effect:?} at 0.5 shows both frames (r={r:.1}, b={b:.1})"
                );
            }
        }
    }

    #[test]
    fn wipe_splits_hard_at_the_eased_half_way_point() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut pass = SceneTransitionPass::new(&engine.device, &engine.queue);
        let red = solid_frame(&engine, "transition-red", [255, 0, 0, 255]);
        let blue = solid_frame(&engine, "transition-blue", [0, 0, 255, 255]);
        let red_view = red.create_view(&wgpu::TextureViewDescriptor::default());
        let blue_view = blue.create_view(&wgpu::TextureViewDescriptor::default());

        let middle = run_effect(
            &engine,
            &mut pass,
            TransitionEffect::Wipe,
            &red_view,
            &blue_view,
            0.5,
        );
        let left_blue = mean_channel(&middle, 2, Some((0, WIDTH / 2)));
        let left_red = mean_channel(&middle, 0, Some((0, WIDTH / 2)));
        let right_red = mean_channel(&middle, 0, Some((WIDTH / 2, WIDTH)));
        let right_blue = mean_channel(&middle, 2, Some((WIDTH / 2, WIDTH)));
        assert!(
            left_blue > 250.0 && left_red < 5.0,
            "left half is the incoming frame (r={left_red:.1}, b={left_blue:.1})"
        );
        assert!(
            right_red > 250.0 && right_blue < 5.0,
            "right half is the outgoing frame (r={right_red:.1}, b={right_blue:.1})"
        );
    }

    #[test]
    fn circle_reveals_the_incoming_frame_from_the_centre() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut pass = SceneTransitionPass::new(&engine.device, &engine.queue);
        let red = solid_frame(&engine, "transition-red", [255, 0, 0, 255]);
        let blue = solid_frame(&engine, "transition-blue", [0, 0, 255, 255]);
        let red_view = red.create_view(&wgpu::TextureViewDescriptor::default());
        let blue_view = blue.create_view(&wgpu::TextureViewDescriptor::default());

        let middle = run_effect(
            &engine,
            &mut pass,
            TransitionEffect::Circle,
            &red_view,
            &blue_view,
            0.5,
        );
        let centre = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
        let corner = 0_usize;
        assert!(
            middle[centre + 2] > 200 && middle[centre] < 40,
            "the disc centre shows the incoming frame ({:?})",
            &middle[centre..centre + 4]
        );
        assert!(
            middle[corner] > 200 && middle[corner + 2] < 40,
            "the corner still shows the outgoing frame ({:?})",
            &middle[corner..corner + 4]
        );
    }
}
