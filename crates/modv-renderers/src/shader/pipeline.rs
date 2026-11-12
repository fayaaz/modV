//! Pipeline creation and per-frame rendering for the shader renderer.
//! Port of the pex-context pipeline machinery in
//! `src/renderer/src/application/renderers/shader/index.js`
//! (`createCanvas`, `makeProgram`, `render`, `removeActiveModule`).
//!
//! # Context split, translated
//!
//! The legacy renderer kept **two** WebGL2 contexts — `shader-buffer` (live)
//! and `shader-buffer-gallery` (preview) — so a gallery preview cannot trash
//! the live drawing buffer (`index.js:62`, `:71`). wgpu has one device, so
//! the equivalent isolation is _separate pipelines and prop buffers per
//! slot_, with the module id recorded twice (live and gallery) in
//! [`ShaderRenderer::setup_module`]. Callers pass separate `target_view`s
//! (the engine's live group texture vs. its preview texture) — see
//! docs/rust-hybrid.md WS-3 "Context split".
//!
//! # Upload elision
//!
//! The legacy path re-uploaded the whole group canvas into a fresh GL
//! texture every frame (`index.js:253-257`). Here the group texture is
//! already a GPU texture owned by the engine, so the "upload" is a
//! bind-group rebuild, not a copy. The per-frame cost is: two
//! `queue.write_buffer` calls (160 bytes + prop slots) plus one bind group.
//!
//! # Bind group layout (fixed for every module)
//!
//! | Binding | Resource | Type |
//! |---------|----------|------|
//! | 0 | shared uniforms ([`ShaderUniforms`](crate::shader::uniforms::ShaderUniforms)) | uniform buffer, 160 B |
//! | 1..=4 | `iChannel0..3` | sampled texture (the same view) |
//! | 5 | `u_modVCanvas` | sampled texture (same view again) |
//! | 6 | `u_fft` | sampled texture (512×1 RGBA8, or a 1×1 dummy) |
//! | 7 | module props | uniform buffer (per-module layout, or a 16 B dummy) |
//! | 8 | group sampler | `Filtering`, Repeat/Repeat |
//! | 9 | fft sampler | `Filtering`, Repeat/Repeat |
//!
//! Binding the same view at 1..=5 is legal wgpu — the layout declares five
//! distinct texture bindings and the bind group points them all at one view.
//! wgpu requires the bind group to provide an entry for **every** layout
//! binding, which is why the prop buffer exists even for prop-less modules
//! (they bind a zeroed 16-byte dummy) and why `fft_view = None` falls back
//! to an internally-created 1×1 texture.
//!
//! # Sampler choice
//!
//! `pex.texture2D({ wrap: pex.Wrap.Repeat })` in `index.js:268` /
//! `:276` maps to `AddressMode::Repeat` on both axes with `FilterMode::Linear`
//! min/mag — the legacy GL textures declare no mipmaps, so the sampler's
//! `mipmap_filter` is irrelevant. Both samplers are the caller's to supply:
//! the group sampler is mandatory, the FFT sampler falls back to an
//! equivalent internal one when the caller passes `None` (or when no FFT
//! texture is passed either — `fft_view = None` uses the dummy).
//!
//! # iChannel0..3 really do alias
//!
//! `index.js:103-111` binds the same `canvasTexture` to `iChannel0..3`,
//! `u_modVCanvas`. This port does the same — one view, five bindings. A GL
//! golden-image diff against the legacy renderer is therefore meaningful.
//!
//! # Two things wgpu cannot look up
//!
//! [`TextureView`] carries no dimensions, so `render` takes explicit
//! `resolution: [f32; 3]` (group `[width, height, dpr]` — feeds
//! `iResolution` and `iChannelResolution`) and `fft_resolution: f32`
//! (the FFT texture width — feeds `u_fftResolution`; pass `1.0` when no FFT
//! texture is bound, mirroring `fftTexture ? fftTexture.width : 1`). The
//! task sketch omitted both; they are unavoidable because `iResolution` is
//! load-bearing for every ported module.

#![forbid(unsafe_code)]

use std::fmt;

use crate::shader::modules::ShaderModule;
use crate::shader::uniforms::{
    PropLayout, PropSlots, SECONDS_TO_MILLISECONDS, SHADER_UNIFORMS_BYTES, ShaderUniforms,
};

/// Audio payload the renderer cannot derive itself. `kick` is the beat
/// detector's edge flag; the bands are AGC-normalised 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShaderAudioUniforms {
    /// Beat flag — `u_kick` as `u32` 0/1 in the uniform block.
    pub kick: bool,
    /// AGC bass band, 0..1.
    pub bass: f32,
    /// AGC mid band, 0..1.
    pub mid: f32,
    /// AGC treble band, 0..1.
    pub treble: f32,
}

/// A module that was compiled into a pipeline via [`ShaderRenderer::setup_module`],
/// with the buffers the renderer owns for it.
struct ModulePipeline {
    /// The compiled pipeline for the module's fragment WGSL.
    pipeline: wgpu::RenderPipeline,
    /// Shared-uniform buffer, exactly [`SHADER_UNIFORMS_BYTES`] bytes.
    uniform_buffer: wgpu::Buffer,
    /// Per-module prop buffer; [`PropLayout::buffer_size`] bytes.
    prop_buffer: wgpu::Buffer,
    /// The prop layout the prop buffer was sized and filled from.
    prop_layout: PropLayout,
    /// Registry name, for diagnostics.
    module_name: String,
}

/// Owns the pipelines, buffers and bind group layout for every active
/// shader module. The single [`Self::render`] entry point binds the group
/// texture (or another source view) and draws the full-screen triangle.
pub struct ShaderRenderer {
    /// The engine's device (kept for buffer and bind group creation).
    device: wgpu::Device,
    /// The engine's queue (kept for uniform buffer uploads).
    queue: wgpu::Queue,
    /// Format handed to [`ShaderRenderer::new`]. Stored as the documented
    /// default; [`ShaderRenderer::setup_module`] takes its own
    /// `target_format` so live and gallery targets may differ.
    target_format: wgpu::TextureFormat,
    /// One fixed layout used by every pipeline this renderer builds.
    bind_group_layout: wgpu::BindGroupLayout,
    /// Pipelines by slot index; `None` for freed slots. Prop-less modules
    /// get their own zeroed 16-byte prop buffer — wgpu buffers are not
    /// clonable handles, so a shared dummy cannot be lent between slots.
    modules: Vec<Option<ModulePipeline>>,
    /// Slot of the live module, if any. Separate from the gallery slot —
    /// the wgpu analogue of the two GL contexts.
    live_slot: Option<usize>,
    /// Slot of the gallery preview module, if any.
    gallery_slot: Option<usize>,
    /// 1×1 RGBA8 fallback for `u_fft` when no FFT texture is available.
    /// Kept solely to keep `dummy_fft_view` alive; never bound directly.
    _dummy_fft_texture: wgpu::Texture,
    /// View of [`Self::_dummy_fft_texture`].
    dummy_fft_view: wgpu::TextureView,
    /// Sampler used for the FFT binding when the caller does not supply one.
    default_fft_sampler: wgpu::Sampler,
}

/// Errors produced by the shader renderer.
///
/// Pipeline/shader creation is fallible only on shape errors that can be
/// checked cheaply up front; actual WGSL validity is asserted by the
/// naga-based test suite (see `super::tests`) against every module in the
/// registry. Anything that reaches `create_shader_module` invalid raises a
/// device validation error exactly as it would for any other wgpu misuse.
#[derive(Debug)]
pub enum ShaderError {
    /// [`ShaderRenderer::render`] or [`ShaderRenderer::remove_slot`] was
    /// given a slot that is not live.
    InvalidSlot(usize),
    /// The encoded prop payload does not match the module's prop layout;
    /// carries the same information as the underlying encode error.
    PropLength {
        /// Expected lane count.
        expected: usize,
        /// Lane count supplied.
        got: usize,
    },
    /// [`ShaderRenderer::setup_module`] received a module whose fragment
    /// source is empty or does not declare an `fs_main` entry point. This is
    /// a registry bug, not a runtime condition — the in-tree modules all do.
    RejectedModule(String),
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShaderError::InvalidSlot(slot) => write!(f, "invalid shader module slot {slot}"),
            ShaderError::PropLength { expected, got } => {
                write!(
                    f,
                    "prop payload length mismatch: expected {expected} float lanes, got {got}"
                )
            }
            ShaderError::RejectedModule(name) => {
                write!(f, "shader module {name:?} has no fs_main entry point")
            }
        }
    }
}

impl std::error::Error for ShaderError {}

impl From<crate::shader::uniforms::PropEncodeError> for ShaderError {
    fn from(error: crate::shader::uniforms::PropEncodeError) -> Self {
        match error {
            crate::shader::uniforms::PropEncodeError::LengthMismatch { expected, got } => {
                ShaderError::PropLength { expected, got }
            }
        }
    }
}

impl ShaderRenderer {
    /// Create a renderer against `device`. `target_format` is stored as the
    /// documented default for pipelines (see [`Self::default_target_format`]).
    /// The fixed bind group layout, the dummy FFT texture/sampler and the
    /// dummy prop buffer are created eagerly.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("modV shader renderer bind group layout"),
            entries: &bind_group_layout_entries(),
        });

        let dummy_fft_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("modV shader dummy FFT texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Filterable, like every other float texture this renderer binds.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_fft_view = dummy_fft_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // FFT textures are PointContained/Linear in the legacy path and the
        // group canvas texture uses Repeat/Linear (`index.js:271`, `:280`).
        let default_fft_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modV shader default FFT sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });

        Self {
            device,
            queue,
            target_format,
            bind_group_layout,
            modules: Vec::new(),
            live_slot: None,
            gallery_slot: None,
            _dummy_fft_texture: dummy_fft_texture,
            dummy_fft_view,
            default_fft_sampler,
        }
    }

    /// The target format this renderer was constructed with. The format that
    /// actually governs pipelines is the one passed to [`Self::setup_module`];
    /// this is a documentation/convenience accessor.
    pub fn default_target_format(&self) -> wgpu::TextureFormat {
        self.target_format
    }

    /// Build (or rebuild) the pipeline for `module` and return its slot
    /// index. `is_gallery = true` allocates in the gallery family — the wgpu
    /// analogue of `shader-buffer-gallery` — otherwise in the live family.
    /// Existing pipelines are kept; replacing a module means calling
    /// [`Self::remove_slot`] first.
    ///
    /// `target_format` is the format of the render pass's target texture;
    /// main (live) and gallery targets may use different formats.
    ///
    /// # Errors
    ///
    /// Returns [`ShaderError::RejectedModule`] when the module's fragment
    /// source is empty or lacks an `fs_main` entry — a registry bug, caught
    /// here before the driver raises a validation error. Well-formed
    /// in-tree modules never see this.
    pub fn setup_module(
        &mut self,
        module: &ShaderModule,
        target_format: wgpu::TextureFormat,
        is_gallery: bool,
    ) -> Result<usize, ShaderError> {
        if module.fragment_wgsl.is_empty() || !module.fragment_wgsl.contains("fs_main") {
            return Err(ShaderError::RejectedModule(module.name.to_string()));
        }

        let prop_layout = PropLayout::from_props(&module.props);

        // One shader module holding both entry points: the shared full-screen
        // triangle vertex and the module's fragment. The fragment source
        // arrives complete (prelude included) from the module registry.
        let source = format!(
            "{}\n{}",
            crate::shader::default_shader::DEFAULT_VERTEX,
            module.fragment_wgsl
        );
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(module.name),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });

        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(module.name),
                bind_group_layouts: &[Some(&self.bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(module.name),
                layout: Some(&pipeline_layout),
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
                        format: target_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });

        let uniform_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modV shader uniform buffer"),
            size: SHADER_UNIFORMS_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let prop_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            // Prop-less modules bind a zeroed 16-byte dummy — wgpu rejects
            // 0-sized uniform buffers.
            label: Some("modV shader prop buffer"),
            size: prop_layout.buffer_size as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline_slot = ModulePipeline {
            pipeline,
            uniform_buffer,
            prop_buffer,
            prop_layout,
            module_name: module.name.to_string(),
        };

        // Reuse a tombstoned slot if one exists, else append.
        let slot = self
            .modules
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.modules.push(None);
                self.modules.len() - 1
            });
        self.modules[slot] = Some(pipeline_slot);

        if is_gallery {
            self.gallery_slot = Some(slot);
        } else {
            self.live_slot = Some(slot);
        }

        Ok(slot)
    }

    /// Destroy the pipeline and buffers owned by `slot`, returning the
    /// module's registry name. Mirrors `removeActiveModule` in
    /// `index.js:319-340` — after the slot is freed, [`Self::render`] with
    /// it fails with [`ShaderError::InvalidSlot`].
    pub fn remove_slot(&mut self, slot: usize) -> Option<String> {
        let removed = self.modules.get_mut(slot).and_then(Option::take)?;
        if self.live_slot == Some(slot) {
            self.live_slot = None;
        }
        if self.gallery_slot == Some(slot) {
            self.gallery_slot = None;
        }
        Some(removed.module_name)
    }

    /// Whether `slot` is currently bound as the live module.
    pub fn is_live_module(&self, slot: usize) -> bool {
        self.live_slot == Some(slot)
    }

    /// Whether `slot` is currently bound as the gallery preview module.
    pub fn is_gallery_module(&self, slot: usize) -> bool {
        self.gallery_slot == Some(slot)
    }

    /// Draw `module` once into `target_view`, sampling `group_view`.
    ///
    /// # Arguments
    ///
    /// * `slot` — a slot returned by [`Self::setup_module`].
    /// * `group_view` — the texture the module reads (the previous canvas /
    ///   lastCanvas equivalent).
    /// * `group_sampler` — sampler for the group view and the `iChannel*`
    ///   bindings (Repeat/Linear in the legacy path).
    /// * `fft_view` — the 512×1 RGBA8 FFT texture, or `None` to bind a 1×1
    ///   zero dummy (u_fftResolution must then stay `1.0`).
    /// * `fft_sampler` — sampler for the FFT texture; falls back to
    ///   [`Self`]'s built-in Repeat/Linear sampler when `None`.
    /// * `audio` — beat flag and band payload (`u_kick`/`u_bass`/`u_mid`/`u_treble`).
    /// * `time_seconds` — monotonic seconds (`iGlobalTime` family); the
    ///   millisecond-based `u_time` is derived as `time_seconds * 1000.0`.
    /// * `frame` — frames since boot (`iFrame`).
    /// * `date` — `[year, month (0-based), weekday (0 = Sunday), seconds]`.
    /// * `resolution` — `[width, height, dpr]` of the group texture; feeds
    ///   `iResolution` and `iChannelResolution`.
    /// * `fft_resolution` — width of the FFT texture, e.g. `512.0`; `1.0`
    ///   when `fft_view` is `None`.
    /// * `params` — module props pre-encoded as float lanes in declaration
    ///   order; see [`PropSlots`].
    /// * `target_view` — the texture the module's output is rendered into.
    ///   **Must not alias `group_view`**: binding a texture as a render
    ///   target and as a sampled source in the same pass is a wgpu
    ///   validation error. The engine keeps separate input/output textures
    ///   (or renders into a scratch) for exactly this reason.
    ///
    /// # Error handling
    ///
    /// Returns [`ShaderError::InvalidSlot`] for a freed/absent slot and
    /// [`ShaderError::PropLength`] when `params` does not encode to the
    /// module's prop layout. The pass is begun only after both checks, so a
    /// rejected frame leaves the target untouched.
    pub fn render(
        &self,
        slot: usize,
        group_view: &wgpu::TextureView,
        group_sampler: &wgpu::Sampler,
        fft_view: Option<&wgpu::TextureView>,
        fft_sampler: Option<&wgpu::Sampler>,
        audio: ShaderAudioUniforms,
        time_seconds: f32,
        frame: i32,
        date: [f32; 4],
        resolution: [f32; 3],
        fft_resolution: f32,
        params: &PropSlots,
        target_view: &wgpu::TextureView,
    ) -> Result<(), ShaderError> {
        let module = self
            .modules
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or(ShaderError::InvalidSlot(slot))?;

        let uniforms = ShaderUniforms {
            i_global_time: time_seconds,
            i_frame: frame,
            // JS drift, kept: iTime is the same clock as iGlobalTime.
            i_time: time_seconds,
            // JS drift, kept: iTimeDelta is NOT a delta (index.js:118).
            i_time_delta: time_seconds,
            i_date: date,
            i_resolution: resolution,
            _pad0: 0.0,
            // Declared in both GLSL and WGSL; nothing ever uploads it.
            i_channel_time: [0.0; 4],
            i_channel_resolution: [[resolution[0], resolution[1], resolution[2], 0.0]; 4],
            // JS drift, kept: u_delta is the same clock as iGlobalTime.
            u_delta: time_seconds,
            // u_time is absolute MILLISECONDS (performance.now()), unlike
            // the seconds-based fields above.
            u_time: time_seconds * SECONDS_TO_MILLISECONDS,
            u_fft_resolution: fft_resolution,
            u_kick: audio.kick as u32,
            u_bass: audio.bass,
            u_mid: audio.mid,
            u_treble: audio.treble,
            _pad1: 0.0,
        };

        let prop_bytes = module.prop_layout.encode(params)?;

        self.queue
            .write_buffer(&module.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        self.queue.write_buffer(&module.prop_buffer, 0, &prop_bytes);

        let fft_view = fft_view.unwrap_or(&self.dummy_fft_view);
        let fft_sampler = fft_sampler.unwrap_or(&self.default_fft_sampler);

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("modV shader bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                // 0 — shared uniforms
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: module.uniform_buffer.as_entire_binding(),
                },
                // 1..=4 — iChannel0..3, all aliasing the group view
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(group_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(group_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(group_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(group_view),
                },
                // 5 — u_modVCanvas
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(group_view),
                },
                // 6 — u_fft (or the 1×1 dummy)
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(fft_view),
                },
                // 7 — module props (or the zeroed dummy)
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: module.prop_buffer.as_entire_binding(),
                },
                // 8 — group sampler
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(group_sampler),
                },
                // 9 — fft sampler
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::Sampler(fft_sampler),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("modV shader render"),
            });
        {
            // The legacy pipeline clears to opaque black before the module
            // draws (`index.js:59-63`) — carried over here.
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("modV shader render pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&module.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

/// The fixed bind group layout, declared once. Kept separate from
/// [`ShaderRenderer::new`] so the reflection-style test
/// (`bind_group_layout_entries_are_stable`) can pin the exact binding map.
pub(crate) fn bind_group_layout_entries() -> [wgpu::BindGroupLayoutEntry; 10] {
    use wgpu::{
        BindingType, BufferBindingType, ShaderStages, TextureSampleType, TextureViewDimension,
    };

    let uniform =
        |binding: u32, min_binding_size: Option<wgpu::BufferSize>| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size,
            },
            count: None,
        };

    let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Texture {
            sample_type: TextureSampleType::Float { filterable: true },
            view_dimension: TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };

    let filtering_sampler = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    };

    [
        // 0 — shared uniforms, exact 160-byte block.
        uniform(0, wgpu::BufferSize::new(SHADER_UNIFORMS_BYTES as u64)),
        // 1..=4 — iChannel0..3
        texture(1),
        texture(2),
        texture(3),
        texture(4),
        // 5 — u_modVCanvas
        texture(5),
        // 6 — u_fft
        texture(6),
        // 7 — per-module prop buffer (unsized: modules differ)
        uniform(7, None),
        // 8 — group sampler
        filtering_sampler(8),
        // 9 — fft sampler
        filtering_sampler(9),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_group_layout_entries_are_stable() {
        let entries = bind_group_layout_entries();
        assert_eq!(entries.len(), 10);
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(entry.binding, index as u32);
            assert!(entry.visibility.contains(wgpu::ShaderStages::FRAGMENT));
        }
        match &entries[0].ty {
            wgpu::BindingType::Buffer {
                min_binding_size, ..
            } => {
                assert_eq!(min_binding_size.map(|s| s.get()), Some(160))
            }
            _ => panic!("binding 0 must be the shared uniform buffer"),
        }
        match &entries[7].ty {
            wgpu::BindingType::Buffer {
                min_binding_size, ..
            } => {
                assert_eq!(
                    *min_binding_size, None,
                    "prop binding is unsized on purpose"
                )
            }
            _ => panic!("binding 7 must be the prop uniform buffer"),
        }
    }
}
