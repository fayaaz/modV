//! Isolated Gallery-thumbnail rendering — the engine side of WS-14
//! ("Gallery thumbnails and previews: request a frame from Rust and blit it").
//!
//! The renderer's Gallery tiles poll
//! `window.engine.requestModulePreview({ moduleName, width, height })`, which
//! reaches the engine as a `preview/module` request. The supervisor cannot
//! render (it owns no GPU device), so it queues the request and the window loop
//! answers it through [`Compositor::render_module_preview`] after
//! `render_frame`; the reply is `{ width, height, pixels }` with `pixels` the
//! base64 of a tightly packed `width × height` RGBA8 frame.
//!
//! ## Why an isolated instance
//!
//! A thumbnail must never touch the live composition. The JS renderer keeps a
//! separate "gallery" shader/ISF context and a per-module `OffscreenCanvas`
//! (`renderers/*/index.js`, `adapter.js:296-323`); the port keeps a
//! [`ModulePreviews`] cache of standalone module instances — the same
//! arrangement as a Scene hover preview
//! ([`modv_engine::scene_preview_data::create_preview_module`]) — so a hover
//! cannot resize the live ISF pass buffers, steal a shader pipeline slot or
//! advance a live module's animation state.
//!
//! ## What one request renders
//!
//! Each registered kind takes the same route the live loop takes (see
//! [`crate::compositor`]):
//!
//! * **shader** — a slot on a dedicated [`ShaderRenderer`] (so preview slots
//!   can never collide with, or free, a live module's slot), sampling a
//!   transparent full-size input texture and writing the shader output.
//! * **2D** — a [`Canvas`] at the requested size, `init` once then
//!   `update`/`draw` per request, `submit`.
//! * **Point10** — the scene (init once), its [`AudioBridge`] and a target
//!   texture at the requested size.
//! * **ISF** — compiled once per slot through [`IsfRuntime`], drawn straight
//!   into the target.
//!
//! The 2D canvas and the Point10 scene target are then composited onto the
//! cached transparent target with the fresh module's defaults (alpha 1,
//! `normal`), which is exactly the blend the live loop applies per module.
//!
//! ISF image inputs have no group canvas to sample, so every `image` input is
//! bound to one transparent 1 × 1 dummy ([`IsfRuntime`]'s documented fallback
//! shape). A filter therefore sees a fully transparent input, which is the
//! honest "nothing underneath" thumbnail for a top-of-stack effect.
//!
//! ## Cost
//!
//! A slot is cached by `(module name, width, height)` in a small LRU
//! ([`MAX_CACHED_PREVIEWS`]), so repeated hover polls re-render only the frame:
//!
//! | kind | first request per key | later requests |
//! | --- | --- | --- |
//! | shader | WGSL pipeline creation (~ms) | full-frame draw + read-back |
//! | 2D | canvas + instance `init` | module `update`/`draw` + copy |
//! | Point10 | scene `init` (pipelines, buffers) | scene update/draw + copy |
//! | ISF | GLSL → WGSL translate + pipeline | pass(es) + read-back |
//!
//! The read-back dominates a warm request: it is a `copy_texture_to_buffer`
//! plus `Device::poll` for `width × height × 4` bytes (≤ 1 MiB at 512²). The
//! window loop caps how many previews it answers per frame so a burst of hover
//! requests cannot stall presentation.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use modv_engine::engine_state::ModuleInstance;
use modv_render::frame_graph::{Executor, FrameGraph, PassNode, TextureId, TextureSurfaces};
use modv_render::wgpu;
use modv_renderers::isf::{CompiledIsf, IsfFrameUniforms, IsfRuntime};
use modv_renderers::point10::audio_bridge::{AudioBridge, ResponsivenessLevers};
use modv_renderers::point10::{Point10Scene, SceneContext};
use modv_renderers::shader::pipeline::ShaderRenderer;
use modv_renderers::two_d::{Canvas, DrawContext, ModuleContext, TwoDModule};

use crate::compositor::{
    FrameInfo, MAX_POINT10_FRAME_SECONDS, OutputTexture, clear_texture, create_output_texture,
    encode_props, interim_band_payload, isf_values, isf_wall_clock_date, read_back_texture,
};
use crate::module_registry::{ModuleKind, ModuleRegistry, Point10SceneKind};

/// Largest edge a preview may request (mirrors the main process'
/// `MODULE_PREVIEW_MAX_SIZE`).
pub const MAX_PREVIEW_DIMENSION: u32 = 512;

/// Number of `(module, width, height)` preview instances kept alive.
///
/// The Gallery only polls tiles inside the viewport now (~20 at once), so this
/// covers the visible set and a little scroll headroom without accumulating
/// hundreds of instances. Volumetric Point10 scenes are *not* retained (see
/// [`ModulePreviews::volumetric_frames`]).
pub const MAX_CACHED_PREVIEWS: usize = 24;

/// Rendered frames cached for the volumetric Point10 scenes.
///
/// A volumetric instance owns eight 150x150x300 `Rgba16Float` textures
/// (~432 MB). Rebuilding one per Gallery poll was the dominant engine-mode
/// stutter (18–40 ms per miss while the 4-slot cache thrashed). A thumbnail is
/// static in the Gallery, so the first frame is cached here and the temporary
/// instance is disposed immediately.
pub const MAX_VOLUMETRIC_FRAMES: usize = 32;

/// Translated ISF pipelines kept by the preview runtime's bounded LRU.
const ISF_PIPELINE_CACHE: usize = 4;

/// Format of every preview target (the group/main format).
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Frame-graph ids used while compositing a module onto its preview target.
const PREVIEW_SOURCE_ID: &str = "preview:source";
const PREVIEW_TARGET_ID: &str = "preview:target";

/// Clamp one requested preview edge into `1..=MAX_PREVIEW_DIMENSION`.
pub fn clamp_preview_dimension(value: u32) -> u32 {
    value.clamp(1, MAX_PREVIEW_DIMENSION)
}

/// Fill an RGBA8 texture with the preview test card.
///
/// Effects need *something* to filter: the legacy Gallery renders a filter
/// over "nothing" and shows a blank tile. Thumbnails instead feed a colourful
/// UV gradient + checker grid (with a white border), so fisheye/CRT/bloom-style
/// effects visibly transform it. Rows are padded to the 256-byte
/// `COPY_BYTES_PER_ROW_ALIGNMENT`.
fn write_preview_pattern(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) {
    let _ = device;
    let row_bytes = (width * 4) as usize;
    let stride = row_bytes.div_ceil(256) * 256;
    let mut data = vec![0u8; stride * height as usize];
    for y in 0..height {
        for x in 0..width {
            let u = x as f32 / width.max(1) as f32;
            let v = y as f32 / height.max(1) as f32;
            let checker = ((x / 16) + (y / 16)) % 2 == 0;
            let border = x < 2 || y < 2 || x + 2 >= width || y + 2 >= height;
            let (r, g, b) = if border {
                (255, 255, 255)
            } else {
                (
                    (u * 255.0) as u8,
                    (v * 255.0) as u8,
                    if checker { 230 } else { 48 },
                )
            };
            let offset = y as usize * stride + x as usize * 4;
            data[offset..offset + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride as u32),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Fill the two 512 × 2 ISF audio textures with a synthetic frame so
/// audio-reactive shaders animate in a thumbnail (a silent live frame would
/// leave them black). Row 0 and 1 are identical (mono), like the live pair.
fn write_preview_audio(queue: &wgpu::Queue, wave: &wgpu::Texture, fft: &wgpu::Texture) {
    const WIDTH: u32 = 512;
    let mut wave_data = Vec::with_capacity((WIDTH * 2 * 4) as usize);
    let mut fft_data = Vec::with_capacity((WIDTH * 2 * 4) as usize);
    for x in 0..WIDTH {
        let sample = ((x as f32 / 16.0).sin() * 0.5 + 0.5) * 255.0;
        let spectrum = (255.0 * (-(x as f32) / 90.0).exp()).clamp(0.0, 255.0);
        wave_data.extend_from_slice(&[sample as u8, sample as u8, sample as u8, 255]);
        fft_data.extend_from_slice(&[spectrum as u8, spectrum as u8, spectrum as u8, 255]);
    }
    for data in [&wave_data, &fft_data] {
        // Two identical rows.
        let mut rows = Vec::with_capacity(data.len() * 2);
        rows.extend_from_slice(data);
        rows.extend_from_slice(data);
        let texture = if std::ptr::eq(data, &wave_data) {
            wave
        } else {
            fft
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rows,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Why a preview request could not be answered.
///
/// Both preview kinds share this type: `preview/module` produces
/// [`PreviewError::ModuleNotFound`]/[`PreviewError::Render`], and
/// `preview/scene` adds [`PreviewError::SceneNotFound`]/
/// [`PreviewError::SceneRowEmpty`]. The renderer treats every code as "no
/// preview yet", keeps the last frame and retries on a short backoff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewError {
    /// The module name is not registered with the engine (the renderer backs
    /// off when it sees this).
    ModuleNotFound(String),
    /// The module is registered but its isolated frame could not be rendered.
    Render(String),
    /// No Scene with that id is in the store (it may have been removed or
    /// renamed since the UI last synced).
    SceneNotFound(String),
    /// The Scene exists but the requested row is out of range, holes, or has
    /// no Module the engine can render.
    SceneRowEmpty {
        /// The Scene the request referenced.
        scene_id: String,
        /// The requested grid row.
        row: usize,
    },
}

impl PreviewError {
    /// The machine-readable code sent in the IPC error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ModuleNotFound(_) => "module_not_found",
            Self::Render(_) => "preview_failed",
            Self::SceneNotFound(_) => "scene_not_found",
            Self::SceneRowEmpty { .. } => "scene_row_empty",
        }
    }

    /// The human-readable message sent in the IPC error.
    pub fn message(&self) -> String {
        match self {
            Self::ModuleNotFound(name) => {
                format!("module \"{name}\" is not registered with the engine")
            }
            Self::Render(detail) => detail.clone(),
            Self::SceneNotFound(id) => format!("no Scene with id \"{id}\" is in the engine store"),
            Self::SceneRowEmpty { scene_id, row } => {
                format!("Scene \"{scene_id}\" has no renderable Group at row {row}")
            }
        }
    }
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModuleNotFound(name) => write!(f, "module \"{name}\" is not registered"),
            Self::Render(detail) => write!(f, "{detail}"),
            Self::SceneNotFound(id) => write!(f, "no Scene with id \"{id}\""),
            Self::SceneRowEmpty { scene_id, row } => {
                write!(
                    f,
                    "Scene \"{scene_id}\" has no renderable Group at row {row}"
                )
            }
        }
    }
}

impl std::error::Error for PreviewError {}

/// Base64-encode bytes with the standard RFC 4648 alphabet and `=` padding.
///
/// The engine has no base64 dependency and this is the only place it is
/// needed: `preview/module` carries the RGBA8 frame as text because the control
/// socket is JSON. Padded output is what the renderer's `atob`-based decoder
/// expects.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// One cached preview instance and its target.
struct PreviewSlot {
    /// Registry name of the module (`$moduleName`).
    name: String,
    /// Target width the slot was created for.
    width: u32,
    /// Target height the slot was created for.
    height: u32,
    /// The standalone instance: its registry meta and default props are what
    /// the renderer reads, exactly like a live module.
    module: ModuleInstance,
    /// Per-kind renderer state.
    state: PreviewState,
    /// The transparent `width × height` target the finished layer lands in.
    target: OutputTexture,
    /// Frames drawn through this slot (the per-module frame counter).
    frames: u64,
}

/// Per-kind isolated renderer state.
enum PreviewState {
    /// A shader pipeline slot on the dedicated renderer plus the transparent
    /// input the shader samples as `u_modVCanvas`/`iChannel0`.
    Shader {
        /// Slot index in [`ModulePreviews::shader`].
        pipeline: usize,
        /// Full-size transparent input (the empty "group canvas").
        input: OutputTexture,
    },
    /// A 2D module instance and the canvas it draws into.
    TwoD {
        /// The module implementation (owns `update`/`draw` state).
        module: Box<dyn TwoDModule>,
        /// Requested-size canvas; its texture is composited onto the target.
        canvas: Canvas,
        /// `init` runs once, on the first request.
        initialised: bool,
    },
    /// A Point10 scene, its bridge and its target texture.
    Point10 {
        /// The concrete scene wrapper (needs the inherent prop binder).
        scene: Point10SceneKind,
        /// Device/queue clones plus the shared quad, sized to the request.
        context: SceneContext,
        /// Per-instance audio bridge (lever state is per module in the JS).
        bridge: AudioBridge,
        /// The scene's own render target, composited onto the slot target.
        scratch: OutputTexture,
        /// Clock of the previous request, for the scene's `dt`.
        last_time: Option<f32>,
    },
    /// A compiled ISF shader; it draws straight into the slot target.
    Isf {
        /// The compiled passes, uniforms and buffers.
        compiled: CompiledIsf,
        /// Full-size test card bound to every `image` input.
        input: OutputTexture,
        /// Synthetic `audio`/`audioFFT` pair when the shader declares them.
        audio: Option<(OutputTexture, OutputTexture)>,
        /// Clock of the previous request, for `TIMEDELTA`.
        last_time: Option<f32>,
    },
}

impl PreviewSlot {
    /// Whether this slot serves `name` at the requested size.
    fn matches(&self, name: &str, width: u32, height: u32) -> bool {
        self.name == name && self.width == width && self.height == height
    }

    /// Release everything the instance owns.
    ///
    /// The shader slot, the 2D instance and the Point10 scene all own GPU
    /// resources that must be freed explicitly (a dropped handle alone does
    /// not free a pipeline slot, and the scenes/modules are API-disposable).
    fn dispose(&mut self, shader: &mut ShaderRenderer) {
        match &mut self.state {
            PreviewState::Shader { pipeline, .. } => {
                shader.remove_slot(*pipeline);
            }
            PreviewState::TwoD { module, .. } => module.dispose(),
            PreviewState::Point10 { scene, .. } => scene.dispose(),
            PreviewState::Isf { .. } => {}
        }
    }
}

/// The isolated thumbnail renderer, owned by the compositor.
pub struct ModulePreviews {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Dedicated pipeline-slot owner, so preview shaders never share indices
    /// with the live renderer.
    shader: ShaderRenderer,
    /// Translated ISF pipeline cache for preview shaders.
    isf_runtime: IsfRuntime,
    /// Repeat/Linear sampler, matching the live group sampler
    /// (`renderers/shader/index.js:271-280`).
    sampler: wgpu::Sampler,
    /// Frame-graph executor for the (rare) module → target composites.
    executor: Executor,
    /// Textures registered for those composites; emptied after each one.
    surfaces: TextureSurfaces,
    /// Cache, most recently used first.
    slots: Vec<PreviewSlot>,
    /// Static thumbnails for the volumetric Point10 scenes, keyed by
    /// `(module name, width, height)`. Their instances are never retained.
    volumetric_frames: HashMap<(String, u32, u32), Vec<u8>>,
}

impl ModulePreviews {
    /// Build the isolated renderer (one extra shader renderer and one ISF
    /// runtime; the per-slot test cards are output-sized).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = ShaderRenderer::new(device.clone(), queue.clone(), TARGET_FORMAT);
        let isf_runtime = IsfRuntime::new(device.clone(), queue.clone(), ISF_PIPELINE_CACHE);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modv-preview-sampler"),
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
        let executor = Executor::new(device, queue);

        Self {
            device: device.clone(),
            queue: queue.clone(),
            shader,
            isf_runtime,
            sampler,
            executor,
            surfaces: TextureSurfaces::new(),
            slots: Vec::new(),
            volumetric_frames: HashMap::new(),
        }
    }

    /// Drop every cached instance, pipeline and target.
    pub fn reset(&mut self) {
        let shader = &mut self.shader;
        for mut slot in self.slots.drain(..) {
            slot.dispose(shader);
        }
        // Translated pipelines are keyed by shader source and never expire;
        // a restored state can reference different modules, so rebuild the
        // runtime exactly like the compositor's live one.
        self.isf_runtime =
            IsfRuntime::new(self.device.clone(), self.queue.clone(), ISF_PIPELINE_CACHE);
        self.volumetric_frames.clear();
    }

    /// Number of cached `(module, size)` instances (test/diagnostic use).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cached_slots(&self) -> usize {
        self.slots.len()
    }

    /// Render one frame of `module_name` at `width × height` and return
    /// tightly packed RGBA8 pixels.
    ///
    /// The size is clamped with [`clamp_preview_dimension`], so an out-of-range
    /// request never allocates or reads back more than
    /// [`MAX_PREVIEW_DIMENSION`]².
    pub fn render(
        &mut self,
        registry: &ModuleRegistry,
        module_name: &str,
        width: u32,
        height: u32,
        frame: &FrameInfo,
    ) -> Result<Vec<u8>, PreviewError> {
        let width = clamp_preview_dimension(width);
        let height = clamp_preview_dimension(height);

        if registry.kind(module_name).is_none() {
            return Err(PreviewError::ModuleNotFound(module_name.to_string()));
        }

        let volumetric = matches!(registry.kind(module_name), Some(ModuleKind::Point10))
            && Point10SceneKind::is_volumetric_name(module_name);
        if volumetric {
            return self.render_volumetric_frame(registry, module_name, width, height, frame);
        }

        let index = match self
            .slots
            .iter()
            .position(|slot| slot.matches(module_name, width, height))
        {
            Some(index) => {
                if index > 0 {
                    let slot = self.slots.remove(index);
                    self.slots.insert(0, slot);
                }
                0
            }
            None => self.create_slot(registry, module_name, width, height)?,
        };

        self.draw_slot(registry, index, frame)?;

        let slot = &self.slots[index];
        read_back_texture(
            &self.device,
            &self.queue,
            &slot.target.texture,
            width,
            height,
        )
        .map_err(PreviewError::Render)
    }

    /// Render (once) and cache a volumetric Point10 thumbnail.
    ///
    /// The temporary slot is disposed before returning, so the 432 MB volume
    /// textures never stay resident per Gallery tile. Repeated polls are a map
    /// lookup; the thumbnail is intentionally static, like a poster frame.
    fn render_volumetric_frame(
        &mut self,
        registry: &ModuleRegistry,
        module_name: &str,
        width: u32,
        height: u32,
        frame: &FrameInfo,
    ) -> Result<Vec<u8>, PreviewError> {
        let key = (module_name.to_string(), width, height);
        if let Some(pixels) = self.volumetric_frames.get(&key) {
            return Ok(pixels.clone());
        }

        let index = self.create_slot(registry, module_name, width, height)?;
        self.draw_slot(registry, index, frame)?;
        let pixels = read_back_texture(
            &self.device,
            &self.queue,
            &self.slots[index].target.texture,
            width,
            height,
        )
        .map_err(PreviewError::Render)?;

        // Drop the huge instance immediately; only the RGBA8 frame is kept.
        if let Some(position) = self
            .slots
            .iter()
            .position(|slot| slot.matches(module_name, width, height))
        {
            let mut slot = self.slots.remove(position);
            slot.dispose(&mut self.shader);
        }

        if self.volumetric_frames.len() >= MAX_VOLUMETRIC_FRAMES {
            self.volumetric_frames.clear();
        }
        self.volumetric_frames.insert(key, pixels.clone());
        Ok(pixels)
    }

    /// Build (and front-insert) a slot for `module_name`, evicting the least
    /// recently used entry when the cache is full.
    fn create_slot(
        &mut self,
        registry: &ModuleRegistry,
        module_name: &str,
        width: u32,
        height: u32,
    ) -> Result<usize, PreviewError> {
        if self.slots.len() >= MAX_CACHED_PREVIEWS {
            if let Some(mut oldest) = self.slots.pop() {
                oldest.dispose(&mut self.shader);
            }
        }

        let module = build_preview_module(registry, module_name)?;
        let kind = registry
            .kind(module_name)
            .ok_or_else(|| PreviewError::ModuleNotFound(module_name.to_string()))?;

        let target = create_output_texture(
            &self.device,
            &format!("modv-preview-target-{module_name}"),
            width,
            height,
        );
        clear_texture(
            &self.device,
            &self.queue,
            &target.texture,
            "preview-target-clear",
        );

        let state = match kind {
            ModuleKind::Shader => {
                let definition = registry.shader_module(module_name).ok_or_else(|| {
                    PreviewError::Render(format!(
                        "shader module \"{module_name}\" is not registered"
                    ))
                })?;
                // `false`: this is a dedicated renderer with no live module,
                // so the live/gallery slot markers are left alone.
                let pipeline = self
                    .shader
                    .setup_module(definition, TARGET_FORMAT, false)
                    .map_err(|error| PreviewError::Render(error.to_string()))?;
                let input = create_output_texture(
                    &self.device,
                    &format!("modv-preview-input-{module_name}"),
                    width,
                    height,
                );
                write_preview_pattern(&self.device, &self.queue, &input.texture, width, height);
                PreviewState::Shader { pipeline, input }
            }
            ModuleKind::TwoD => {
                let instance = registry.create_two_d(module_name).ok_or_else(|| {
                    PreviewError::Render(format!(
                        "2D module \"{module_name}\" has no implementation"
                    ))
                })?;
                let canvas = Canvas::new(&self.device, &self.queue, width, height);
                PreviewState::TwoD {
                    module: instance,
                    canvas,
                    initialised: false,
                }
            }
            ModuleKind::Point10 => {
                let mut scene = registry
                    .create_point10_instance(module_name)
                    .ok_or_else(|| {
                        PreviewError::Render(format!(
                            "Point10 scene \"{module_name}\" has no implementation"
                        ))
                    })?;
                let mut context = SceneContext::new(&self.device, &self.queue, TARGET_FORMAT);
                context.resize(width, height);
                scene.init(&mut context);
                let scratch = create_output_texture(
                    &self.device,
                    &format!("modv-preview-scene-{module_name}"),
                    width,
                    height,
                );
                clear_texture(
                    &self.device,
                    &self.queue,
                    &scratch.texture,
                    "preview-scene-clear",
                );
                PreviewState::Point10 {
                    scene,
                    context,
                    bridge: AudioBridge::new(),
                    scratch,
                    last_time: None,
                }
            }
            ModuleKind::Isf => {
                let definition = registry.isf_definition(module_name).ok_or_else(|| {
                    PreviewError::Render(format!("ISF shader \"{module_name}\" is not registered"))
                })?;
                let compiled = self
                    .isf_runtime
                    .compile(&definition.document, &definition.name, TARGET_FORMAT)
                    .map_err(|error| PreviewError::Render(error.to_string()))?;
                let input = create_output_texture(
                    &self.device,
                    &format!("modv-preview-isf-input-{module_name}"),
                    width,
                    height,
                );
                write_preview_pattern(&self.device, &self.queue, &input.texture, width, height);
                let needs_audio = definition
                    .document
                    .inputs
                    .iter()
                    .any(|input| input.type_ == "audio" || input.type_ == "audioFFT");
                let audio = needs_audio.then(|| {
                    let wave = create_output_texture(
                        &self.device,
                        &format!("modv-preview-isf-wave-{module_name}"),
                        512,
                        2,
                    );
                    let fft = create_output_texture(
                        &self.device,
                        &format!("modv-preview-isf-fft-{module_name}"),
                        512,
                        2,
                    );
                    write_preview_audio(&self.queue, &wave.texture, &fft.texture);
                    (wave, fft)
                });
                PreviewState::Isf {
                    compiled,
                    input,
                    audio,
                    last_time: None,
                }
            }
        };

        self.slots.insert(
            0,
            PreviewSlot {
                name: module_name.to_string(),
                width,
                height,
                module,
                state,
                target,
                frames: 0,
            },
        );
        Ok(0)
    }

    /// Draw one frame of the cached slot into its target.
    fn draw_slot(
        &mut self,
        registry: &ModuleRegistry,
        index: usize,
        frame: &FrameInfo,
    ) -> Result<(), PreviewError> {
        let Self {
            device,
            queue,
            shader,
            isf_runtime,
            sampler,
            executor,
            surfaces,
            slots,
            ..
        } = self;

        let slot = &mut slots[index];
        let PreviewSlot {
            module,
            width,
            height,
            state,
            target,
            frames,
            ..
        } = slot;
        let (width, height) = (*width, *height);
        *frames += 1;
        let frame_index = *frames;
        let audio = &frame.audio;
        let props = &module.props;
        let module_name = module.module_name.as_str();

        match state {
            PreviewState::Shader { pipeline, input } => {
                let definition = registry.shader_module(module_name).ok_or_else(|| {
                    PreviewError::Render(format!(
                        "shader module \"{module_name}\" is not registered"
                    ))
                })?;
                let params = encode_props(definition, props);
                shader
                    .render(
                        *pipeline,
                        &input.view,
                        sampler,
                        // No FFT texture feed yet (WS-9/M6): the renderer's
                        // 1 × 1 zero dummy is bound and `u_fftResolution` is 1.
                        None,
                        None,
                        frame.shader_audio(),
                        frame.time_seconds,
                        frame_index as i32,
                        frame.date,
                        [width as f32, height as f32, 1.0],
                        1.0,
                        &params,
                        &target.view,
                    )
                    .map_err(|error| PreviewError::Render(error.to_string()))?;
            }
            PreviewState::TwoD {
                module: instance,
                canvas,
                initialised,
            } => {
                // The JS `delta` argument is the `requestAnimationFrame`
                // timestamp, i.e. a monotonic millisecond clock
                // (`renderers/2d/index.js:24`).
                let delta_ms = frame.time_seconds * 1000.0;

                if !*initialised {
                    *initialised = true;
                    let mut ctx = ModuleContext {
                        canvas_width: width,
                        canvas_height: height,
                        delta: delta_ms,
                        audio,
                        frame: 0,
                        props,
                        device,
                        queue,
                    };
                    instance.init(&mut ctx);
                }

                let mut ctx = ModuleContext {
                    canvas_width: width,
                    canvas_height: height,
                    delta: delta_ms,
                    audio,
                    frame: frame_index,
                    props,
                    device,
                    queue,
                };
                instance.update(&mut ctx);

                canvas.begin_frame();
                // A thumbnail starts empty every time; the canvas otherwise
                // keeps its previous frame (a live module's deliberate
                // self-sampling behaviour).
                canvas.clear_all();
                {
                    let mut draw = DrawContext {
                        canvas,
                        props,
                        audio,
                        delta: delta_ms,
                        frame: frame_index,
                        device,
                        queue,
                        // No media input for a thumbnail; `Webcam`/`Texture2D`
                        // draw nothing, exactly like a live module with no
                        // source.
                        input_texture: None,
                        input_size: None,
                        is_gallery: true,
                    };
                    instance.draw(&mut draw);
                }
                canvas.submit();
                let source = canvas.texture().clone();
                composite_into(
                    device,
                    queue,
                    executor,
                    surfaces,
                    &source,
                    target,
                    "preview-2d",
                )?;
            }
            PreviewState::Point10 {
                scene,
                context,
                bridge,
                scratch,
                last_time,
            } => {
                context.resize(width, height);

                // `adapter.js:797-804`: scene props, then the bridge levers.
                let mut get = |key: &str| {
                    props
                        .get(key)
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite())
                        .map(|value| value as f32)
                };
                scene.apply_props(&mut get);
                let levers = ResponsivenessLevers::from_getter(|key| get(key));
                let payload = interim_band_payload(audio);

                // `timing.js: createFrameClock` — first frame `dt = 0`, then
                // clamped elapsed seconds.
                let dt = match *last_time {
                    Some(previous) if (frame.time_seconds - previous).is_finite() => {
                        (frame.time_seconds - previous).clamp(0.0, MAX_POINT10_FRAME_SECONDS)
                    }
                    _ => 0.0,
                };
                *last_time = Some(frame.time_seconds);

                bridge.update(dt, Some(&payload), &levers);
                scene.update(dt, bridge);

                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("modv-preview-point10"),
                });
                scene.render(&mut encoder, &scratch.view);
                queue.submit(Some(encoder.finish()));

                let source = scratch.texture.clone();
                composite_into(
                    device,
                    queue,
                    executor,
                    surfaces,
                    &source,
                    target,
                    "preview-point10",
                )?;
            }
            PreviewState::Isf {
                compiled,
                input,
                audio: preview_audio,
                last_time,
            } => {
                let definition = registry.isf_definition(module_name).ok_or_else(|| {
                    PreviewError::Render(format!("ISF shader \"{module_name}\" is not registered"))
                })?;

                let time_delta = match *last_time {
                    Some(previous) if (frame.time_seconds - previous).is_finite() => {
                        (frame.time_seconds - previous).max(0.0)
                    }
                    _ => 0.0,
                };
                *last_time = Some(frame.time_seconds);

                // No group canvas exists for a thumbnail: filters sample the
                // test card so an effect is visible instead of a blank tile.
                let image_inputs: Vec<(&str, &wgpu::TextureView)> = definition
                    .document
                    .inputs
                    .iter()
                    .filter(|input| input.type_ == "image")
                    .map(|decl| (decl.name.as_str(), &input.view))
                    .collect();
                let values = isf_values(&definition.document, props);
                let uniforms = IsfFrameUniforms {
                    time: frame.time_seconds,
                    time_delta,
                    frame_index: frame_index as i32,
                    date: isf_wall_clock_date(),
                };

                isf_runtime
                    .draw(
                        compiled,
                        &target.view,
                        [width, height],
                        uniforms,
                        &values,
                        &image_inputs,
                        // Audio-reactive filters get a synthetic spectrum so
                        // a thumbnail animates even when the live frame is
                        // silent.
                        &preview_audio
                            .as_ref()
                            .map(|(wave, fft)| vec![("audio", &wave.view), ("audioFFT", &fft.view)])
                            .unwrap_or_default(),
                    )
                    .map_err(|error| PreviewError::Render(error.to_string()))?;
            }
        }

        Ok(())
    }
}

/// Build the isolated [`ModuleInstance`] a preview renders.
///
/// This is the engine's `makeActiveModule` shape without touching the store:
/// the registry supplies `meta` (`meta_fields`) and the prop defaults
/// (`default_props`), the caller-style defaults fill in `alpha`/`enabled`/
/// `compositeOperation`, and the module is flagged `isGallery` like the JS
/// gallery context.
fn build_preview_module(
    registry: &ModuleRegistry,
    module_name: &str,
) -> Result<ModuleInstance, PreviewError> {
    let mut meta = registry.meta_fields(module_name).unwrap_or_default();
    meta.insert("alpha".to_string(), json!(1.0));
    meta.insert("enabled".to_string(), json!(true));
    meta.insert("compositeOperation".to_string(), json!("normal"));
    meta.insert("isGallery".to_string(), json!(true));

    let props = registry.default_props(module_name).ok_or_else(|| {
        PreviewError::Render(format!(
            "module \"{module_name}\" has no prop defaults in the registry"
        ))
    })?;

    Ok(ModuleInstance {
        id: format!("preview:{module_name}"),
        module_name: module_name.to_string(),
        meta: Value::Object(meta),
        props,
        data: json!({}),
        dollar_props: json!({}),
        status: Vec::new(),
        extra: Map::new(),
    })
}

/// Composite `source` over the (cleared) preview `target` with a fresh
/// module's defaults: alpha 1, `normal`.
///
/// Uses the frame graph's composite pass so canvas blend math lives in exactly
/// one place (`modv_render::frame_graph`); the temporary registrations are
/// removed afterwards so evicted slots never keep a texture alive.
fn composite_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    executor: &mut Executor,
    surfaces: &mut TextureSurfaces,
    source: &wgpu::Texture,
    target: &OutputTexture,
    label: &str,
) -> Result<(), PreviewError> {
    clear_texture(device, queue, &target.texture, label);

    let source_id = TextureId::from(PREVIEW_SOURCE_ID);
    let target_id = TextureId::from(PREVIEW_TARGET_ID);
    surfaces.insert(source_id.clone(), source.clone());
    surfaces.insert(target_id.clone(), target.texture.clone());

    let mut graph = FrameGraph::new();
    graph.push(PassNode::Composite {
        source: source_id.clone(),
        target: target_id.clone(),
        alpha: 1.0,
        blend: modv_render::blend::BlendMode::Normal,
    });
    let result = executor.execute(&mut graph, surfaces);

    surfaces.remove(&source_id);
    surfaces.remove(&target_id);

    result.map_err(|error| PreviewError::Render(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 §10 test vectors plus a binary case with high bytes.
    #[test]
    fn base64_matches_the_rfc_4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_encodes_an_rgba_pixel() {
        // Transparent black, then opaque white: the two shapes the renderer
        // decodes back into `ImageData`.
        assert_eq!(base64_encode(&[0, 0, 0, 0]), "AAAAAA==");
        assert_eq!(base64_encode(&[255, 255, 255, 255]), "/////w==");
        // The renderer's own fixture: 2×2 red/green/blue/white.
        let pixels = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        assert_eq!(base64_encode(&pixels), "/wAA/wD/AP8AAP///////w==");
    }

    #[test]
    fn preview_dimensions_clamp_to_one_and_the_maximum() {
        assert_eq!(clamp_preview_dimension(0), 1);
        assert_eq!(clamp_preview_dimension(1), 1);
        assert_eq!(clamp_preview_dimension(128), 128);
        assert_eq!(clamp_preview_dimension(MAX_PREVIEW_DIMENSION), 512);
        assert_eq!(clamp_preview_dimension(513), 512);
        assert_eq!(clamp_preview_dimension(u32::MAX), 512);
    }
}
