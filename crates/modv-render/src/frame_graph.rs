//! Frame graph and compositor passes (WS-2, "Render graph" and
//! "Compositor" in `docs/rust-hybrid.md`).
//!
//! This milestone (M2b) implements the graph *executor* and the two
//! compositor passes that were previously done on canvas (`RowFade`, the
//! alpha/blend composite) with W3C-exact pixel semantics. Module draws are
//! logged-and-skipped; whole-scene transitions and presentation land in
//! later milestones.
//!
//! ## Coordinate and alpha semantics (ports of the JS)
//!
//! - All internal textures are **premultiplied** `Rgba8Unorm` (no sRGB); see
//!   `blend.rs` for why canvas math must not be linearised.
//! - `Composite` mirrors the `main.globalAlpha`/`globalCompositeOperation` +
//!   `drawImage` tail of the group loop in `worker/loop.js` (lines ~622-658):
//!   the source is drawn over the accumulating target with the given alpha
//!   and blend mode. The source's premultiplied colour and alpha are scaled
//!   by `alpha` before compositing (canvas `globalAlpha` semantics).
//! - `RowFade` is `renderRowFade` from `worker/row-transition.js` (lines
//!   71-115) ported literally: smoothstep easing of `progress`, the outgoing
//!   half composited first at `from_alpha × (1 - eased)`, then the incoming
//!   half at `to_alpha × eased`, both `source-over`. Pass 1 composites the
//!   ghost half into a pooled scratch over a transparent 1×1 DST (no
//!   pre-clear of `target` is possible: sampling it would conflict with the
//!   attachment); pass 2 blends the incoming half over that scratch directly
//!   into `target`. The caller owns assigning the same texture id to both
//!   the ghost and incoming group (WS-2 "two group ids"; `loop.js:530-535`).
//! - Pipeline mode: in the JS, each module after the first draws into a
//!   scratch that is composited back (`worker/loop.js:391-402`). The graph
//!   expresses a pipeline group as a **linear chain of Module nodes into the
//!   group's target** — no intermediate full-frame copy exists in Rust. The
//!   `Module` node is a no-op today (M3/M4 bind the real renderers), so this
//!   milestone only pins the *shape*; the copy elision is a design note, not
//!   an implemented behaviour, on purpose.
//!
//! ## Execution model
//!
//! One `execute()` call ticks the pool clock, runs every node in order
//! (top-to-bottom) in the caller's ordering, and finally evicts the pool to
//! a bounded number of entries. Every pass is submitted on its own (`queue`
//! ordering preserves pass order across submissions); this keeps the blend
//! uniform buffer single-use per pass and avoids wgpu feedback hazards. The
//! WS-2 batching optimisation (one encoder for the whole frame) is deferred.
//!
//! Textures the passes sample are registered in [`TextureSurfaces`] by the
//! caller, keyed by [`TextureId`]. The caller owns group texture ids; the
//! reserved id [`MAIN_TEXTURE_ID`] is cleared to transparent black at the
//! start of every `execute()` (the M2 default, mirroring
//! `worker/loop.js:564`'s `main.clearRect(0, 0, ...)` before compositing)
//! and is never presented by the graph itself.

use std::collections::HashMap;
use std::fmt;

use crate::blend::{BLEND_WGSL, BlendMode};
use crate::texture_pool::{PooledTexture, TexturePool, TexturePoolKey, UsageClass};

/// Number of pooled scratch entries kept after each frame. The compositor
/// needs at most a couple; 8 leaves slack for future pass types without
/// pinning group targets (which are not pooled yet — WS-2 "Texture pool").
const POOL_KEEP_AFTER_FRAME: usize = 8;

/// Texture id for the main/output accumulation texture. Cleared per frame.
pub const MAIN_TEXTURE_ID: &str = "main";

/// Identifies a texture registered in [`TextureSurfaces`].
///
/// Opaque to the executor; callers pick the scheme (e.g. `"group:<id>"`,
/// `"main"` for the accumulation texture — see [`MAIN_TEXTURE_ID`]).
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct TextureId(pub String);

impl TextureId {
    /// Create a texture id from anything string-like.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TextureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for TextureId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for TextureId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// One node in the frame graph (WS-2 "Render graph").
#[derive(Clone, Debug)]
pub enum PassNode {
    /// One module draw into `target`.
    ///
    /// Modules do not exist yet (M3/M4); the graph only needs the slot
    /// shape so the executor is real. The `pipeline_slot` orders modules
    /// within a group when pipeline mode is used (WS-2 "Pipeline mode").
    Module {
        /// Owning group id (renderer-assigned).
        group: String,
        /// Module id (renderer-assigned).
        module: String,
        /// Position within the group's module chain.
        pipeline_slot: usize,
        /// Texture the module draws into.
        target: TextureId,
    },
    /// Draw `source` over `target` with `alpha` and `blend`.
    Composite {
        /// Texture to draw.
        source: TextureId,
        /// Accumulating output texture.
        target: TextureId,
        /// Canvas `globalAlpha` (clamped to `[0, 1]`; scales the source's
        /// premultiplied colour and alpha before blending).
        alpha: f64,
        /// Canvas `globalCompositeOperation`.
        blend: BlendMode,
    },
    /// Cross-fade the outgoing and incoming halves of a row into `target`.
    ///
    /// Direct port of `renderRowFade` (`worker/row-transition.js:71-115`):
    /// `progress` is smoothstep-eased, the `from` half is drawn first at
    /// `from_alpha × (1 - eased)`, then the `to` half at `to_alpha × eased`,
    /// both `source-over` onto a cleared target. The caller assigns the same
    /// `target` to both the incoming and ghost group ids (WS-2, "row
    /// transitions"); rows inherit the blend, not a single half.
    RowFade {
        /// Outgoing (ghost) group texture.
        from: TextureId,
        /// Incoming group texture.
        to: TextureId,
        /// Output texture receiving the blended cross-fade. The caller
        /// assigns the same id to the ghost and the incoming group so the
        /// next row inherits the blend (`loop.js:530-535`, "two ids").
        ///
        /// Must not alias the sampled `from`/`to` textures: a render pass
        /// may not sample the texture that is also its attachment. wgpu
        /// rejects that loudly with a validation error.
        target: TextureId,
        /// Transition progress in `[0, 1]` (clamped; non-finite → 0).
        progress: f64,
        /// The ghost group's own alpha (un-eased).
        from_alpha: f64,
        /// The incoming group's own alpha (un-eased).
        to_alpha: f64,
    },
    /// Whole-scene shader transition.
    ///
    /// Structure reserved; `kind` names an ISF transition (e.g.
    /// `"CrossZoom"`) and `progress` its timeline position. Execution is a
    /// logged no-op until WS-8/M5 (bounded render target, transition-sizing,
    /// ISF cache) is implemented.
    SceneTransition {
        /// Texture holding the from-side group composition.
        from: TextureId,
        /// Texture holding the to-side group composition.
        to: TextureId,
        /// Output texture.
        target: TextureId,
        /// ISF transition name.
        kind: String,
        /// Transition progress in `[0, 1]`.
        progress: f64,
    },
    /// Present `source` to the swapchain.
    ///
    /// Never presented by the graph: `modv-app` owns the surfaces and does
    /// the `queue.present` step. This node is a structural marker so the
    /// frame reports can attribute the frame boundary.
    Present {
        /// Texture the window should present.
        source: TextureId,
    },
}

/// A frame's pass list. Nodes execute in vector order (caller owns order —
/// render order is decided by the scene/graph builder, not here).
#[derive(Clone, Debug, Default)]
pub struct FrameGraph {
    /// The passes to execute, in order.
    pub nodes: Vec<PassNode>,
}

impl FrameGraph {
    /// Empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a pass.
    pub fn push(&mut self, node: PassNode) {
        self.nodes.push(node);
    }

    /// Number of passes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when no passes are scheduled.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Errors from [`Executor::execute`].
#[derive(Debug)]
pub enum GraphError {
    /// A node referenced a texture that was never registered in
    /// [`TextureSurfaces`] (or was removed since).
    MissingTexture {
        /// The unresolved id.
        id: TextureId,
        /// Which pass wanted it.
        pass: &'static str,
    },
    /// `source`/`target` (or the `RowFade` halves) are not the same size.
    /// Canvas `drawImage` scales; the graph does not — sizing mismatches
    /// are bugs in the Scene/graph builder and reported instead of scaled.
    SizeMismatch {
        /// Which pass failed.
        pass: &'static str,
        /// Human-readable dimensions, e.g. `"source 64x64 vs target 32x32"`.
        detail: String,
    },
    /// The target format cannot be used as a render attachment on this
    /// device ([`wgpu::TextureFormat::guaranteed_format_features`]).
    FormatNotRenderable {
        /// Which pass failed.
        pass: &'static str,
        /// The offending format.
        format: wgpu::TextureFormat,
    },
    /// A texture to be sampled cannot be bound as a texture on this device.
    FormatNotSampleable {
        /// Which pass failed.
        pass: &'static str,
        /// The offending format.
        format: wgpu::TextureFormat,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTexture { id, pass } => {
                write!(
                    f,
                    "{pass}: texture {id} is not registered in TextureSurfaces"
                )
            }
            Self::SizeMismatch { pass, detail } => write!(f, "{pass}: {detail}"),
            Self::FormatNotRenderable { pass, format } => {
                write!(f, "{pass}: format {format:?} cannot be a render attachment")
            }
            Self::FormatNotSampleable { pass, format } => {
                write!(f, "{pass}: format {format:?} cannot be sampled")
            }
        }
    }
}

impl std::error::Error for GraphError {}

/// Textures the executor may sample, keyed by [`TextureId`].
///
/// Owned by the caller (the engine), which holds one entry per group plus
/// the main accumulation texture. Textures are cloned handles (`wgpu`
/// `Arc`-like); the GPU allocation is shared.
#[derive(Default)]
pub struct TextureSurfaces {
    map: HashMap<TextureId, wgpu::Texture>,
}

impl TextureSurfaces {
    /// Empty surface set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a texture under `id`.
    pub fn insert(&mut self, id: TextureId, texture: wgpu::Texture) -> Option<wgpu::Texture> {
        self.map.insert(id, texture)
    }

    /// Look up a texture by id.
    pub fn get(&self, id: &TextureId) -> Option<&wgpu::Texture> {
        self.map.get(id)
    }

    /// True when `id` is registered.
    pub fn contains_key(&self, id: &TextureId) -> bool {
        self.map.contains_key(id)
    }

    /// Remove and return the texture for `id`.
    pub fn remove(&mut self, id: &TextureId) -> Option<wgpu::Texture> {
        self.map.remove(id)
    }

    /// Number of registered textures.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when no textures are registered.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl fmt::Debug for TextureSurfaces {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.map.iter().map(|(id, texture)| {
                (
                    id.as_str(),
                    format!(
                        "{}x{} {:?}",
                        texture.width(),
                        texture.height(),
                        texture.format()
                    ),
                )
            }))
            .finish()
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlendUniform {
    mode: u32,
    src_alpha_scale: f32,
}

/// Executes [`FrameGraph`]s: pool management, blend passes, copies.
pub struct Executor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Scratch texture pool (WS-2 "Texture pool").
    pub pool: TexturePool,
    /// Shared sampler (clamp-to-edge, linear — the canvas default for 1:1 blits).
    sampler: wgpu::Sampler,
    /// The blend pipeline; recreated if the attachment format changes
    /// (there is exactly one format in practice: `Rgba8Unorm`).
    blend_pipeline: Option<wgpu::RenderPipeline>,
    /// Format `blend_pipeline` was built for.
    blend_pipeline_format: Option<wgpu::TextureFormat>,
    /// Layout shared by every blend pipeline (DST/SRC textures + sampler + uniform).
    blend_bind_group_layout: wgpu::BindGroupLayout,
    /// 8-byte uniform holding `{ mode, src_alpha_scale }` for the blend pass.
    blend_uniform_buffer: wgpu::Buffer,
    /// 1×1 transparent black texture; bound as the DST sample for passes
    /// whose source must composite over *nothing* (the first RowFade half).
    transparent_texel: wgpu::Texture,
    /// One-shot flags for the unimplemented passes (avoid per-frame spam).
    module_warned: bool,
    scene_transition_warned: bool,
}

impl Executor {
    /// Create an executor. `device`/`queue` are cloned handles; the caller
    /// (the engine) keeps ownership of the originals.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modv-blend-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let blend_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("modv-blend-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let blend_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modv-blend-uniform"),
            size: std::mem::size_of::<BlendUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let transparent_texel = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("modv-transparent-texel"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &transparent_texel,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0u8, 0, 0, 0],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pool: TexturePool::new(device),
            sampler,
            blend_pipeline: None,
            blend_pipeline_format: None,
            blend_bind_group_layout,
            blend_uniform_buffer,
            transparent_texel,
            module_warned: false,
            scene_transition_warned: false,
        }
    }

    /// Execute the graph. The `main` surface ([`MAIN_TEXTURE_ID`]), when
    /// registered, is cleared to transparent black first — the M2 default,
    /// mirroring `worker/loop.js:564` (`main.clearRect(0, 0, ...)`) which
    /// runs before the group composite.
    pub fn execute(
        &mut self,
        graph: &FrameGraph,
        surfaces: &mut TextureSurfaces,
    ) -> Result<(), GraphError> {
        self.pool.tick();
        if let Some(main) = surfaces.get(&TextureId::from(MAIN_TEXTURE_ID)) {
            self.clear_texture(main, "main-clear");
        }
        for node in &graph.nodes {
            self.execute_node(node, surfaces)?;
        }
        self.pool.evict_lru(POOL_KEEP_AFTER_FRAME);
        Ok(())
    }

    fn execute_node(
        &mut self,
        node: &PassNode,
        surfaces: &mut TextureSurfaces,
    ) -> Result<(), GraphError> {
        match node {
            PassNode::Module {
                group,
                module,
                pipeline_slot,
                ..
            } => {
                if !self.module_warned {
                    eprintln!(
                        "engine: WARN module renderer not bound yet (M3/M4); \
                         skipping Module nodes (first: {group}/{module}, pipeline slot {pipeline_slot})"
                    );
                    self.module_warned = true;
                }
                Ok(())
            }
            PassNode::Present { .. } => Ok(()),
            PassNode::SceneTransition { kind, progress, .. } => {
                if !self.scene_transition_warned {
                    eprintln!(
                        "engine: WARN scene transition '{kind}' (progress {progress}) not implemented yet; \
                         skipping (WS-8/M5)"
                    );
                    self.scene_transition_warned = true;
                }
                Ok(())
            }
            PassNode::Composite {
                source,
                target,
                alpha,
                blend,
            } => self.run_composite(source, target, *alpha, *blend, surfaces),
            PassNode::RowFade {
                from,
                to,
                target,
                progress,
                from_alpha,
                to_alpha,
            } => self.run_row_fade(
                from,
                to,
                target,
                *progress,
                *from_alpha,
                *to_alpha,
                surfaces,
            ),
        }
    }

    /// `Composite`: blend `source` over `target` with `alpha` and `blend`.
    ///
    /// Ping-pongs through a pooled scratch (sampling `target` while it would
    /// be the render attachment is illegal), then copies the scratch back.
    fn run_composite(
        &mut self,
        source: &TextureId,
        target: &TextureId,
        alpha: f64,
        blend: BlendMode,
        surfaces: &mut TextureSurfaces,
    ) -> Result<(), GraphError> {
        let pass = "composite";
        let alpha = clamp_unit(alpha, 1.0) as f32;
        let src_texture = self.texture_of(surfaces, source, pass)?;
        let dst_texture = self.texture_of(surfaces, target, pass)?;
        self.check_same_size(src_texture, dst_texture, pass)?;
        self.check_format(
            src_texture.format(),
            wgpu::TextureUsages::TEXTURE_BINDING,
            pass,
        )?;
        self.check_format(
            dst_texture.format(),
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            pass,
        )?;

        let scratch = self.acquire_scratch(
            src_texture.width(),
            src_texture.height(),
            dst_texture.format(),
            UsageClass::PipelineScratch,
        );
        self.record_blend_pass(
            scratch.texture(),
            dst_texture,
            src_texture,
            blend,
            alpha,
            "composite",
        );
        self.copy_texture(scratch.texture(), dst_texture, "composite-copy-back");
        Ok(())
    }

    /// `RowFade`: smoothstep-ease `progress` (port of `getRowFadeAlphas` in
    /// `worker/row-transition.js:39-62`), then draw the ghost half at
    /// `from_alpha × (1 - eased)` followed by the incoming half at
    /// `to_alpha × eased`, both `source-over`, into `target`.
    ///
    /// Pass 1 composites `from` over a transparent DST (bound via a 1×1
    /// texel) into a pooled `RowFadeScratch`; pass 2 blends `to` over that
    /// scratch **directly into `target`** — no feedback, no copy-back. The
    /// caller assigns `target` to both group ids (the "two ids" rule,
    /// `loop.js:530-535`).
    fn run_row_fade(
        &mut self,
        from: &TextureId,
        to: &TextureId,
        target: &TextureId,
        progress: f64,
        from_alpha: f64,
        to_alpha: f64,
        surfaces: &mut TextureSurfaces,
    ) -> Result<(), GraphError> {
        let pass = "row-fade";
        let (from_eased, to_eased) = row_fade_alphas(progress, from_alpha, to_alpha);
        let from_texture = self.texture_of(surfaces, from, pass)?;
        let to_texture = self.texture_of(surfaces, to, pass)?;
        let target_texture = self.texture_of(surfaces, target, pass)?;
        self.check_same_size(from_texture, to_texture, pass)?;
        self.check_same_size(from_texture, target_texture, pass)?;
        self.check_format(
            from_texture.format(),
            wgpu::TextureUsages::TEXTURE_BINDING,
            pass,
        )?;
        self.check_format(
            to_texture.format(),
            wgpu::TextureUsages::TEXTURE_BINDING,
            pass,
        )?;
        self.check_format(
            target_texture.format(),
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            pass,
        )?;

        let scratch = self.acquire_scratch(
            from_texture.width(),
            from_texture.height(),
            from_texture.format(),
            UsageClass::RowFadeScratch,
        );
        // The 1×1 transparent texel is the DST for the first half: drawing
        // `from` with source-over over *nothing* must reproduce a cleared
        // scratch, so no feedback or pre-clear is needed.
        let transparent = self.transparent_texel.clone();
        self.record_blend_pass(
            scratch.texture(),
            &transparent,
            from_texture,
            BlendMode::SourceOver,
            from_eased,
            "row-fade-from",
        );
        self.record_blend_pass(
            target_texture,
            scratch.texture(),
            to_texture,
            BlendMode::SourceOver,
            to_eased,
            "row-fade-to",
        );
        Ok(())
    }

    fn texture_of<'surfaces>(
        &self,
        surfaces: &'surfaces TextureSurfaces,
        id: &TextureId,
        pass: &'static str,
    ) -> Result<&'surfaces wgpu::Texture, GraphError> {
        surfaces.get(id).ok_or_else(|| GraphError::MissingTexture {
            id: id.clone(),
            pass,
        })
    }

    fn check_same_size(
        &self,
        a: &wgpu::Texture,
        b: &wgpu::Texture,
        pass: &'static str,
    ) -> Result<(), GraphError> {
        if a.width() == b.width() && a.height() == b.height() {
            Ok(())
        } else {
            Err(GraphError::SizeMismatch {
                pass,
                detail: format!(
                    "source is {}x{}, target is {}x{}",
                    a.width(),
                    a.height(),
                    b.width(),
                    b.height()
                ),
            })
        }
    }

    fn check_format(
        &self,
        format: wgpu::TextureFormat,
        required: wgpu::TextureUsages,
        pass: &'static str,
    ) -> Result<(), GraphError> {
        let guaranteed = format
            .guaranteed_format_features(self.device.features())
            .allowed_usages;
        if guaranteed.contains(required) {
            return Ok(());
        }
        if required.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
            Err(GraphError::FormatNotRenderable { pass, format })
        } else {
            Err(GraphError::FormatNotSampleable { pass, format })
        }
    }

    fn acquire_scratch(
        &mut self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage_class: UsageClass,
    ) -> PooledTexture {
        self.pool.acquire(
            &self.device,
            &self.queue,
            TexturePoolKey {
                width,
                height,
                format,
                usage_class,
            },
        )
    }

    fn clear_texture(&self, texture: &wgpu::Texture, label: &str) {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// Render the blend blit into `output`: `dst` is the accumulated
    /// canvas contents (sampled), `src` the drawn element (sampled). The
    /// output attachment starts cleared to transparent black (the fullscreen
    /// triangle overwrites every pixel anyway).
    #[allow(clippy::too_many_arguments)]
    fn record_blend_pass(
        &mut self,
        output: &wgpu::Texture,
        dst: &wgpu::Texture,
        src: &wgpu::Texture,
        mode: BlendMode,
        src_alpha_scale: f32,
        label: &str,
    ) {
        if self.blend_pipeline_format != Some(output.format()) {
            self.blend_pipeline = Some(self.create_blend_pipeline(output.format(), label));
            self.blend_pipeline_format = Some(output.format());
        }
        let output_view = output.create_view(&wgpu::TextureViewDescriptor::default());
        let dst_view = dst.create_view(&wgpu::TextureViewDescriptor::default());
        let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());

        let uniform = BlendUniform {
            mode: mode.mode_index(),
            src_alpha_scale,
        };
        self.queue
            .write_buffer(&self.blend_uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.blend_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&dst_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.blend_uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
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
            if let Some(pipeline) = self.blend_pipeline.as_ref() {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                // Full-screen triangle; no vertex buffer.
                pass.draw(0..3, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
    }

    fn create_blend_pipeline(
        &self,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("modv-blend-shader"),
                source: wgpu::ShaderSource::Wgsl(BLEND_WGSL.into()),
            });
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("modv-blend-pipeline-layout"),
                bind_group_layouts: &[Some(&self.blend_bind_group_layout)],
                immediate_size: 0,
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                        format,
                        // Blending happens in the shader; fixed-function off.
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    fn copy_texture(&self, from: &wgpu::Texture, to: &wgpu::Texture, label: &str) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: from,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: to,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: from.width().min(to.width()),
                height: from.height().min(to.height()),
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
    }
}

/// `getRowFadeAlphas` port (`worker/row-transition.js:39-62`):
/// clamp `progress` (non-finite → 0), smoothstep `t²(3-2t)`, and scale the
/// group alphas by `1-eased` / `eased`.
fn row_fade_alphas(progress: f64, from_alpha: f64, to_alpha: f64) -> (f32, f32) {
    let clamped = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let eased = clamped * clamped * (3.0 - 2.0 * clamped);
    let from = if from_alpha.is_finite() {
        from_alpha.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let to = if to_alpha.is_finite() {
        to_alpha.clamp(0.0, 1.0)
    } else {
        1.0
    };
    ((from * (1.0 - eased)) as f32, (to * eased) as f32)
}

/// Clamp canvas `globalAlpha`-style values to `[0, 1]`; non-finite → 1.
fn clamp_unit(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}
