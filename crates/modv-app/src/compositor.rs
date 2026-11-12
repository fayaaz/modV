//! GPU compositor — the M2/M3 render loop (WS-2 / WS-5).
//!
//! The compositor evaluates [`EngineState`] groups in
//! `group_index_render_order`, draws each group's modules (shader modules
//! through [`ShaderRenderer`], 2D modules through the
//! [`modv_renderers::two_d`] canvas facade), and composites the result onto the
//! main texture that is presented to the window and published as preview
//! frames.
//!
//! ## Frame shape (port of `worker/loop.js:239-663`)
//!
//! For every group in render order:
//!
//! 1. Skip when `enabled == GROUP_DISABLED`, the group has no modules, or
//!    `alpha < 0.001` (the transition exception is a documented no-op in M2 —
//!    see below).
//! 2. Clear the group target when `clearing` is set (or when the texture was
//!    just allocated — a fresh texture must not sample undefined memory).
//! 3. Inherit: resolve `inheritFrom` with
//!    [`EngineState::resolve_inherit_source`] and composite the resolved
//!    source into the group target at alpha 1 / `normal` (`loop.js:338-379`).
//!    The first group of a frame inherits nothing, never last frame's root
//!    canvas, so transparent scenes cannot accumulate trails.
//! 4. Modules, in `group.modules` order:
//!    * **Shader** (`meta.type == "shader"`): copy the group target into the
//!      module input texture, run the shader renderer into the module output
//!      texture (its `group_view` and `target_view` **must** differ — wgpu
//!      rejects sampling the attachment), then composite the module output
//!      back onto the group target with the module's `meta.alpha` and
//!      `meta.compositeOperation`.
//!    * **2D** (`meta.type == "2d"`): build/refresh the module's own
//!      [`Canvas`] at the output size, run `init` once, `resize` when the size
//!      changes, `update` each frame, `draw` into the canvas, `submit`, then
//!      composite the canvas texture onto the group target with the module's
//!      `meta.alpha`/`meta.compositeOperation` (`loop.js:409-470`,
//!      `renderers/2d/index.js:29-96`). A registered Point10 scene also
//!      reports `meta.type: "2d"` but takes the scene path instead — see
//!      "Point10 scenes (M4)" below.
//!
//! Then the main texture is composited from every group in render order,
//! skipping the Gallery group **by name** (`loop.js:628-637` — the order
//! guarantees the Gallery is last, not that the last entry is the Gallery),
//! groups that are not `GROUP_ENABLED`, groups with no modules, and groups at
//! alpha 0.
//!
//! ## 2D canvas semantics (M3 deviation, documented)
//!
//! The JS 2D renderer draws every module of a group into that group's single
//! `CanvasRenderingContext2D` (`loop.js:383`; `renderers/2d/index.js:75-95`),
//! so a module that samples "the canvas" (`drawImage(canvas, …)`) reads
//! whatever the *earlier modules of the same group* drew this frame on top of
//! the group's own persistent content. The port isolates each module with its
//! own output-sized canvas, which is required because the facade has no GPU
//! equivalent of sampling an in-flight render target for arbitrary modules and
//! because per-module canvases let `Canvas` keep its content between frames
//! without re-uploading the group target.
//!
//! Consequences:
//!
//! * Every self-sampling module (`Smear`, `Pixelate`, `GridStretch`, `Buffer`)
//!   samples **its own** previous output, which is the intended effect and
//!   matches the JS whenever it is the only self-sampling module in the group
//!   (it only ever reads what it drew).
//! * With two self-sampling modules in the same group, the later one does not
//!   see the earlier one's output inside its own `drawImage(canvas)` — it sees
//!   the composited group target instead once the compositor blends it. This
//!   is the one observable difference and is accepted for M3; fixing it would
//!   require a shared per-group canvas (a group-sized GPU canvas with a
//!   cross-module command list), which is a follow-up.
//! * Canvas content persists between frames exactly like the shared group
//!   canvas; `group.clearing` clears the group target **and** every 2D canvas
//!   the group owns, so the "clear then redraw" behaviour is preserved.
//! * `meta.alpha` and `meta.compositeOperation` are applied **once**, when the
//!   module's finished canvas is composited onto the group target. The JS sets
//!   `globalAlpha`/`globalCompositeOperation` on the shared group context
//!   *before* `module.draw` (`loop.js:409-412`), so there they apply to every
//!   individual canvas operation. The two are identical for opaque output; they
//!   differ when a module draws overlapping semi-transparent primitives with
//!   `meta.alpha < 1` or a non-`normal` blend (the JS accumulates per shape, the
//!   port blends the flat layer). This is the standard layer reading of a
//!   module and is accepted for M3.
//! * Each module instance owns an output-sized canvas (plus its MSAA and
//!   snapshot targets), so a group with N 2D modules costs roughly N+1 output
//!   textures instead of one. Acceptable for the corpus' handful of 2D layers;
//!   a shared per-group GPU canvas would remove both this and the sampling
//!   difference above.
//! * `pipeline` mode is the same code path. JS pipeline mode draws the first
//!   module into the group canvas, then every later module into a scratch that
//!   is composited back (`loop.js:391-402`, `renderers/2d/index.js:48-74`),
//!   costing an extra full-frame copy per module. The Rust loop already
//!   samples the accumulated group target and composites each module output
//!   back, so the visual result is identical (for shader modules) and no
//!   scratch ping-pong is needed; for 2D modules the per-module canvas replaces
//!   the group canvas entirely, so `pipeline` also changes nothing.
//!
//! ## Deliberate M3/M4 limitations
//!
//! * **ISF/three modules do not render.** They log once and are skipped (their
//!   renderers land in M4).
//! * **No `input_texture` for 2D modules.** `Webcam` and `Texture2D` draw
//!   nothing while `DrawContext::input_texture` is `None` — exactly what the
//!   JS does when the video/texture source is missing. Media inputs land with
//!   M6.
//! * **No transition ghosts or scene transitions.** A row fade keeps the old
//!   group alive with `transitionGhost: true`; M2/M3 treats such a group as an
//!   ordinary group. The alpha-cull exception in `loop.js:257-266` is
//!   implemented as `transition_ghost == Some(true)` so the check has the right
//!   shape, but there is no row-transition state to consult yet (M5 adds it).
//! * **The Gallery group is inert.** In JS the Gallery group renders its
//!   preview modules into an auxiliary canvas (the hovered item's) and the
//!   loop skips everything when that canvas is absent (`loop.js:332-338`,
//!   `:401-407`). The Rust loop has no auxiliary targets, so the group renders
//!   nothing, inherits nothing and never reaches the main canvas.
//! * **`hidden` only affects inheritance.** Faithful to JS: the group loop does
//!   not cull hidden groups, and the main composite does not check `hidden`
//!   either — it only stops a group from becoming `lastCanvas`
//!   (`loop.js:538-557`).
//!
//! ## Point10 scenes (M4)
//!
//! Point10 modules are registered with `meta.type: "2d"` (the JS
//! `createPoint10Module` meta), so the 2D branch of [`Compositor::render_module`]
//! refines the dispatch through [`ModuleRegistry::kind`] and sends a
//! [`ModuleKind::Point10`] module to [`Compositor::render_point10_module`]:
//!
//! 1. Build the scene once (`create_point10_instance`), `init` it with a
//!    [`SceneContext`] sized to the output, `resize` it on later size changes.
//! 2. Apply the module's props to the scene every frame (the JS
//!    `applyProps`/`readLevers` pair, `adapter.js:797-804`), then advance the
//!    [`AudioBridge`] with the resolved responsiveness levers.
//! 3. `update(dt, &bridge)` and `render` into the scene's own
//!    output-sized [`OutputTexture`].
//! 4. Composite that texture onto the group target with the module's
//!    `meta.alpha`/`meta.compositeOperation` — the same helper (and therefore
//!    the same canvas blend math) the 2D canvas path uses.
//!
//! The JS adapter draws each scene into an `OffscreenCanvas` and blits it into
//! the 2D group canvas one frame later (`adapter.js:818-836`); rendering into a
//! texture removes both the blit and that lag. The scene target is
//! `Rgba8Unorm` (the group format) because none of the batch-1 scenes use the
//! postfx chain: `Point10Scene::render` clears to transparent black and the
//! scene's own alpha blending lands directly on canvas-space values. The
//! [`modv_renderers::point10::postfx::Composer`] and
//! [`modv_renderers::point10::HDR_FORMAT`] stay available for the bloom scenes
//! in later batches — a scene that needs bloom will grow a flag on its meta or
//! schema and get a composer here, not a second render path.
//!
//! ### Interim audio mapping (M4; M6 replaces it)
//!
//! The engine has no analyser yet: `meyda/SET_BANDS` gives the store three AGC
//! bands (plus a zeroed beat), and [`FrameInfo::audio`] carries them. The
//! bridge wants the full `audio-bands.js` payload, so
//! [`interim_band_payload`] synthesises it: each band is replicated across its
//! four damping lanes and its grid cells ([`band_of_cell`]), `wave`/`freq_mag`
//! are the store's (still zeroed) arrays, `sample_rate` is reported as
//! 48 kHz, and the byte spectrum is whatever the frame carries (1024 zero bins
//! today, so scenes see silence rather than "no analyser"). M6 replaces this
//! function with the analyser's real payload and nothing else moves.
//!
//! ## Present boundary
//!
//! All internal textures are `Rgba8Unorm`; canvas blend math is done on the
//! same UNORM values the browser canvas stores, which keeps the compositor
//! pixel-compatible with the JS renderer (see `modv_render::blend`).
//! [`Compositor::present`] samples the main texture without any colour-space
//! conversion and draws it onto the swapchain. The surface format chosen by
//! [`modv_render::surface::create_surface`] is usually sRGB, so the hardware
//! applies its sRGB encode at this single boundary. That is the documented
//! "sRGB at present only" contract of the frame graph, not a per-pass
//! conversion: module-to-group and group-to-main composites never linearise.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use modv_engine::blend::BlendMode as EngineBlendMode;
use modv_engine::engine_state::{
    EngineState, GALLERY_GROUP_NAME, GROUP_DISABLED, GROUP_ENABLED, Group, ModuleInstance,
};
use modv_ipc::preview::{PreviewFrame, PreviewRing};
use modv_render::frame_graph::{
    Executor, FrameGraph, GraphError, MAIN_TEXTURE_ID, PassNode, TextureId, TextureSurfaces,
};
use modv_render::wgpu;
use modv_renderers::point10::audio_bridge::{
    AUDIO_SAMPLES, AudioBridge, BandPayload, LANES_PER_BAND, RawBands, ResponsivenessLevers,
    band_of_cell,
};
use modv_renderers::point10::{Point10Scene, SceneContext};
use modv_renderers::shader::ShaderModule;
use modv_renderers::shader::pipeline::{ShaderAudioUniforms, ShaderError, ShaderRenderer};
use modv_renderers::shader::uniforms::{PropKind, PropSlots, ShaderProp};
use modv_renderers::two_d::{AudioFrame, Canvas, DrawContext, ModuleContext, TwoDModule};

use crate::module_registry::{ModuleKind, ModuleRegistry, Point10SceneKind};

/// Preview frame width published to the UI (`modv-ipc` `VMP1` ring).
pub const PREVIEW_WIDTH: u32 = 320;
/// Preview frame height published to the UI.
pub const PREVIEW_HEIGHT: u32 = 180;

/// The alpha below which a group (and a module) is culled (`loop.js:268`,
/// `:390`).
const ALPHA_CULL: f64 = 0.001;

/// Everything a frame needs besides the state itself.
#[derive(Clone, Debug, Default)]
pub struct FrameInfo {
    /// Monotonic seconds since engine start (`iGlobalTime` family).
    pub time_seconds: f32,
    /// Frames presented since boot (`iFrame`).
    pub frame_index: u64,
    /// `[year, month (0-based), weekday (0 = Sunday), seconds]` — `iDate`.
    pub date: [f32; 4],
    /// The engine's audio frame (AGC bands today; wave/FFT once M6 lands).
    ///
    /// Filled from [`Store::audio`](crate::store_ops::Store::audio), which the
    /// renderer's audio pump updates through `meyda/SET_BANDS`. Shader modules
    /// read the bands via [`FrameInfo::shader_audio`], 2D modules receive the
    /// whole frame in `ModuleContext`/`DrawContext`.
    pub audio: AudioFrame,
}

impl FrameInfo {
    /// The shader uniform view of [`Self::audio`].
    ///
    /// `kick` stays `false`: beat detection is part of the audio analysis
    /// milestone (WS-9 / M6), and `meyda/SET_BANDS` does not carry a beat flag.
    pub fn shader_audio(&self) -> ShaderAudioUniforms {
        ShaderAudioUniforms {
            kick: false,
            bass: self.audio.bass,
            mid: self.audio.mid,
            treble: self.audio.treble,
        }
    }
}

/// Errors from [`Compositor::render_frame`] and the read-back helpers.
#[derive(Debug)]
pub enum CompositorError {
    /// A frame graph pass failed (missing texture, size mismatch, ...).
    Graph(GraphError),
    /// The shader renderer rejected a slot or prop payload.
    Shader(ShaderError),
    /// GPU read-back failed (map/poll/copy).
    ReadBack(String),
    /// The preview ring rejected the frame.
    Preview(String),
}

impl std::fmt::Display for CompositorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Graph(error) => write!(f, "{error}"),
            Self::Shader(error) => write!(f, "{error}"),
            Self::ReadBack(detail) => write!(f, "read-back failed: {detail}"),
            Self::Preview(detail) => write!(f, "preview write failed: {detail}"),
        }
    }
}

impl std::error::Error for CompositorError {}

impl From<GraphError> for CompositorError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl From<ShaderError> for CompositorError {
    fn from(error: ShaderError) -> Self {
        Self::Shader(error)
    }
}

/// An output-sized texture and its view.
struct OutputTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// One 2D module's live state: the ported instance plus its own canvas.
///
/// The JS keeps module state in the module definition's `data` object; the
/// port keeps it inside the [`TwoDModule`] instance (see
/// `modv_renderers::two_d::modules`), so this struct is the per-instance state
/// the store's `data` field deliberately does not mirror.
struct TwoDInstance {
    /// The module instance (owns `update`/`draw` state).
    module: Box<dyn TwoDModule>,
    /// The output-sized canvas, created on first draw and dropped on resize.
    canvas: Option<Canvas>,
    /// Size the module was last initialised/resized for.
    size: Option<(u32, u32)>,
    /// `init` runs once, on the first frame the module draws.
    initialised: bool,
    /// Frames drawn since activation (`ModuleContext::frame`).
    frame: u64,
    /// Set when the owning group cleared; the canvas is cleared as part of the
    /// module's next draw pass (see [`Compositor::request_two_d_clears`]).
    clear_requested: bool,
}

impl TwoDInstance {
    fn new(module: Box<dyn TwoDModule>) -> Self {
        Self {
            module,
            canvas: None,
            size: None,
            initialised: false,
            frame: 0,
            clear_requested: false,
        }
    }
}

/// One Point10 module's live state: the scene, its bridge and its target.
///
/// The JS adapter keeps one scene instance, one `OffscreenCanvas` and one
/// `AudioBridge` per module id (`adapter.js:296-323`); this struct is that
/// state. The target texture replaces the `OffscreenCanvas` + blit.
struct Point10Instance {
    /// The concrete scene wrapper (needs the inherent prop binder).
    scene: Point10SceneKind,
    /// Device/queue clones plus the shared quad, sized to the output.
    context: SceneContext,
    /// Per-instance audio bridge (lever state is per module in the JS).
    bridge: AudioBridge,
    /// The output-sized scene target, rebuilt when the output resizes.
    target: Option<OutputTexture>,
    /// Size the scene was last initialised/resized for.
    size: Option<(u32, u32)>,
    /// `init` runs once, on the first frame the scene draws.
    initialised: bool,
    /// `FrameInfo::time_seconds` of the previous frame, for the scene clock.
    last_time: Option<f32>,
}

/// The GPU compositor: shader renderer, frame-graph executor, textures.
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader: ShaderRenderer,
    executor: Executor,
    surfaces: TextureSurfaces,
    registry: ModuleRegistry,
    size: (u32, u32),
    main: Option<OutputTexture>,
    module_input: Option<OutputTexture>,
    module_output: Option<OutputTexture>,
    /// Ids of the groups that have a texture registered in [`TextureSurfaces`].
    group_targets: HashSet<String>,
    /// Pipeline slot per active module id.
    slots: HashMap<String, usize>,
    /// Modules whose slot creation failed; skipped without retrying each frame.
    failed_modules: HashSet<String>,
    /// Live 2D module instances keyed by module id.
    two_d: HashMap<String, TwoDInstance>,
    /// 2D module ids whose instance could not be created; skipped without
    /// retrying each frame.
    failed_two_d: HashSet<String>,
    /// Live Point10 scene instances keyed by module id.
    point10: HashMap<String, Point10Instance>,
    /// Point10 module ids whose scene could not be created; skipped without
    /// retrying each frame.
    failed_point10: HashSet<String>,
    /// One-shot log keys (unknown blend mode, skipped module type, ...).
    warned: HashSet<String>,
    /// Sampler for module input textures — Repeat/Linear, matching
    /// `renderers/shader/index.js:271-280`.
    group_sampler: wgpu::Sampler,
    /// Sampler for the present pass — clamp/linear, a 1:1 blit.
    present_sampler: wgpu::Sampler,
    present_layout: wgpu::BindGroupLayout,
    present_pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Compositor {
    /// Build the compositor against the engine's device and queue.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = ShaderRenderer::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let executor = Executor::new(device, queue);

        let group_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modv-group-sampler"),
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
        let present_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modv-present-sampler"),
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
        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("modv-present-bind-group-layout"),
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
            ],
        });

        Self {
            device: device.clone(),
            queue: queue.clone(),
            shader,
            executor,
            surfaces: TextureSurfaces::new(),
            registry: ModuleRegistry::builtin(),
            size: (0, 0),
            main: None,
            module_input: None,
            module_output: None,
            group_targets: HashSet::new(),
            slots: HashMap::new(),
            failed_modules: HashSet::new(),
            two_d: HashMap::new(),
            failed_two_d: HashSet::new(),
            point10: HashMap::new(),
            failed_point10: HashSet::new(),
            warned: HashSet::new(),
            group_sampler,
            present_sampler,
            present_layout,
            present_pipeline: None,
        }
    }

    /// Resize the output. Every output-sized texture is dropped and lazily
    /// reallocated at the new size on the next frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if (width, height) == self.size {
            return;
        }
        self.size = (width, height);
        self.drop_output_textures();
        self.group_targets.clear();
        self.surfaces = TextureSurfaces::new();
        // 2D canvases are output-sized too. Dropping them (rather than
        // resizing in place) keeps the next frame on one creation path; the
        // module's `resize` runs when its canvas is rebuilt at the new size.
        for instance in self.two_d.values_mut() {
            instance.canvas = None;
            instance.size = None;
        }
        // Point10 scene targets are output-sized as well. The scenes keep their
        // own pipelines (only `resize` re-derives size-dependent state), so the
        // next frame rebuilds the target and calls `resize` on the scene.
        for instance in self.point10.values_mut() {
            instance.target = None;
            instance.size = None;
        }
    }

    /// Drop all GPU-side state (used after `engine/restore`).
    ///
    /// Module pipelines are rebuilt lazily; a restored state can reference
    /// different modules and different groups, so stale slots, textures and 2D
    /// instances must not survive the swap.
    pub fn reset(&mut self) {
        self.drop_output_textures();
        self.group_targets.clear();
        self.surfaces = TextureSurfaces::new();
        for slot in std::mem::take(&mut self.slots).into_values() {
            self.shader.remove_slot(slot);
        }
        self.failed_modules.clear();
        for (_, mut instance) in self.two_d.drain() {
            instance.module.dispose();
        }
        self.failed_two_d.clear();
        for (_, mut instance) in self.point10.drain() {
            instance.scene.dispose();
        }
        self.failed_point10.clear();
    }

    fn drop_output_textures(&mut self) {
        self.main = None;
        self.module_input = None;
        self.module_output = None;
        self.surfaces.remove(&TextureId::from(MAIN_TEXTURE_ID));
        self.surfaces.remove(&module_output_id());
    }

    /// Render one frame from `state`.
    pub fn render_frame(
        &mut self,
        state: &EngineState,
        frame: &FrameInfo,
    ) -> Result<(), CompositorError> {
        if self.size.0 == 0 || self.size.1 == 0 {
            return Ok(());
        }
        self.ensure_output_textures();
        self.prune(state);

        // The frame graph clears whatever is registered as `main` at the start
        // of every `execute()`. Group passes must not pay that cost, so main is
        // unregistered while groups render and re-registered for the final
        // composite — the (only) texture the graph clears is then exactly the
        // one this frame presents.
        let main_texture = self.surfaces.remove(&main_id());
        let mut rendered: HashSet<String> = HashSet::new();
        let groups = self.render_groups(state, frame, &mut rendered);
        if let Some(texture) = main_texture {
            self.surfaces.insert(main_id(), texture);
        }
        groups?;

        self.compose_main(state)?;
        Ok(())
    }

    /// Group loop (`worker/loop.js:239-558`).
    fn render_groups(
        &mut self,
        state: &EngineState,
        frame: &FrameInfo,
        rendered: &mut HashSet<String>,
    ) -> Result<(), CompositorError> {
        let order = state.group_index_render_order();

        for (position, &index) in order.iter().enumerate() {
            let Some(group) = state.groups.get(index) else {
                continue;
            };
            let is_gallery_group = group.name == GALLERY_GROUP_NAME;

            // M2 has no auxiliary targets; the JS skips the Gallery group's
            // modules, inheritance and main composite in that case
            // (`loop.js:332-338`, `:401-407`, `:635`).
            if is_gallery_group {
                continue;
            }

            // `loop.js:257-266`: a fading group may dip below the alpha cull;
            // M2 has no row transitions yet, so `transitionGhost` is the only
            // (currently unreachable) exception shape.
            let is_transition_group = group.transition_ghost == Some(true);

            if group.enabled == GROUP_DISABLED
                || group.modules.is_empty()
                || (group.alpha < ALPHA_CULL && !is_transition_group)
            {
                continue;
            }

            let target_id = self.group_target(&group.id);
            if group.clearing {
                if let Some(texture) = self.surfaces.get(&target_id) {
                    self.clear_texture(texture, "group-clear");
                }
                // JS groups share one 2D context, so clearing the group wipes
                // every module's drawing surface (`loop.js:319-321` has no
                // per-module equivalent). With per-module canvases the
                // equivalent is clearing each canvas the group owns.
                self.request_two_d_clears(group);
            }

            // `loop.js:338-379`: inherit from `lastCanvas` (-1) or an explicit
            // group. `rendered` only holds groups that became `lastCanvas`, so
            // culled groups never feed the next row (`loop.js:538-557`).
            if group.inherit {
                // The resolver's third argument is an `IndexMap<String, ()>`
                // (insertion = draw order in the JS). `modv-app` does not depend
                // on `indexmap`, so the map is built by inference from the
                // resolver's signature: a sentinel entry guarantees the "frame
                // context" branch, and an all-culled row then inherits nothing
                // (`lastCanvas == null`) instead of falling back to the
                // positional previous group.
                let rendered_map = std::iter::once((String::new(), ()))
                    .chain(rendered.iter().map(|id| (id.clone(), ())))
                    .collect();
                if let Some(source) =
                    state.resolve_inherit_source(&group.inherit_from, position, &rendered_map)
                {
                    if source.id != group.id {
                        let source_id = self.group_target(&source.id);
                        self.composite(source_id, target_id.clone(), 1.0, RenderBlendMode::Normal)?;
                    }
                }
            }

            // `loop.js:381-491`: module chain.
            let module_ids = group.modules.clone();
            for module_id in module_ids {
                self.render_module(state, group, &module_id, &target_id, frame)?;
            }

            // `loop.js:538-557`: only a visible, fully-enabled, non-ghost group
            // becomes the inheritance source for the next row.
            if !group.hidden
                && group.enabled == GROUP_ENABLED
                && group.transition_ghost != Some(true)
            {
                rendered.insert(group.id.clone());
            }
        }

        Ok(())
    }

    /// One module draw + its composite back onto the group target
    /// (`loop.js:381-491`, `renderers/shader/index.js:319-345`).
    fn render_module(
        &mut self,
        state: &EngineState,
        group: &Group,
        module_id: &str,
        target: &TextureId,
        frame: &FrameInfo,
    ) -> Result<(), CompositorError> {
        let Some(module) = state.modules.get(module_id) else {
            // `loop.js:385-394`: a group can outlive its module; skip it.
            return Ok(());
        };
        if module.meta.get("enabled").and_then(Value::as_bool) != Some(true) {
            return Ok(());
        }
        let alpha = module
            .meta
            .get("alpha")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        if alpha < ALPHA_CULL || !module.status.is_empty() {
            return Ok(());
        }

        let module_type = module
            .meta
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("shader");
        match module_type {
            // Point10 scenes register as `2d` modules (the JS meta) but render
            // through their own scene + target, so the registry decides.
            "2d" => {
                if self.registry.kind(&module.module_name) == Some(ModuleKind::Point10) {
                    return self.render_point10_module(module, module_id, target, frame);
                }
                return self.render_two_d_module(module, module_id, target, frame);
            }
            "shader" => {}
            other => {
                self.warn_once(
                    format!("module-type:{other}"),
                    &format!(
                        "engine: WARN module type '{other}' is not rendered in M3; skipping \
                         (first: {} / {})",
                        group.name, module.module_name
                    ),
                );
                return Ok(());
            }
        }

        let Some(slot) = self.slot_for(&module.module_name, module_id) else {
            return Ok(());
        };

        // Pull the prop payload before any mutable borrow of `self`.
        let params = {
            let Some(definition) = self.registry.shader_module(&module.module_name) else {
                return Ok(());
            };
            encode_props(definition, &module.props)
        };

        let (Some(input), Some(output)) = (&self.module_input, &self.module_output) else {
            return Ok(());
        };
        let input_texture = input.texture.clone();
        let output_texture = output.texture.clone();

        // The shader renderer samples `group_view` and writes `target_view`;
        // they must differ (wgpu validation rejects sampling an attachment).
        let Some(source_texture) = self.surfaces.get(target).cloned() else {
            return Ok(());
        };
        self.copy_texture(&source_texture, &input_texture);

        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (width, height) = self.size;

        self.shader.render(
            slot,
            &input_view,
            &self.group_sampler,
            // No FFT texture yet (WS-9 / M6); passing `None` binds the
            // renderer's 1×1 zero dummy and `u_fftResolution` must then be 1.0.
            None,
            None,
            frame.shader_audio(),
            frame.time_seconds,
            frame.frame_index as i32,
            frame.date,
            [width as f32, height as f32, 1.0],
            1.0,
            &params,
            &output_view,
        )?;

        let blend = self.render_blend_mode(
            module
                .meta
                .get("compositeOperation")
                .and_then(Value::as_str)
                .unwrap_or("normal"),
        );
        self.composite(module_output_id(), target.clone(), alpha, blend)
    }

    /// Mark every 2D canvas owned by `group`'s modules for clearing.
    ///
    /// The JS group clears the shared group context, so every module surface is
    /// empty at the start of the frame (`loop.js:319-321`). The port keeps one
    /// canvas per module, so each is flagged here and cleared inside its own
    /// draw pass ([`Canvas::clear_all`] before the draws, which folds the
    /// clear into the module's single submit). A module that does not render
    /// keeps the flag until it does, so a disabled module never composites
    /// stale content when it is re-enabled.
    fn request_two_d_clears(&mut self, group: &Group) {
        for module_id in &group.modules {
            let Some(instance) = self.two_d.get_mut(module_id) else {
                continue;
            };
            if instance.canvas.is_some() {
                instance.clear_requested = true;
            }
        }
    }

    /// Create the 2D instance for `module_id` if it does not exist yet.
    ///
    /// Returns `false` when `module_name` is not a registered 2D module; the
    /// caller skips the module. The failure is remembered so a group that
    /// outlives its registration does not warn every frame.
    fn create_two_d_instance(&mut self, module_name: &str, module_id: &str) -> bool {
        if self.failed_two_d.contains(module_id) {
            return false;
        }
        match self.registry.create_two_d(module_name) {
            Some(module) => {
                self.two_d
                    .insert(module_id.to_string(), TwoDInstance::new(module));
                true
            }
            None => {
                self.warn_once(
                    format!("unregistered-2d:{module_name}"),
                    &format!(
                        "engine: WARN 2D module '{module_name}' is not in the registry; skipping"
                    ),
                );
                self.failed_two_d.insert(module_id.to_string());
                false
            }
        }
    }

    /// Draw one 2D module into its canvas and composite the result onto the
    /// group target (`loop.js:409-470`, `renderers/2d/index.js:29-96`).
    ///
    /// Deliberately no `input_texture` in M3: `Webcam`/`Texture2D` draw
    /// nothing until media inputs land (M6), exactly like the JS with an unset
    /// source. The module instance owns its `data`-equivalent state, so
    /// `update` mutates the instance rather than the store.
    fn render_two_d_module(
        &mut self,
        module: &ModuleInstance,
        module_id: &str,
        target: &TextureId,
        frame: &FrameInfo,
    ) -> Result<(), CompositorError> {
        if !self.two_d.contains_key(module_id)
            && !self.create_two_d_instance(&module.module_name, module_id)
        {
            return Ok(());
        }

        // `loop.js:410-412`: alpha and composite operation come from the meta.
        let alpha = module
            .meta
            .get("alpha")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        let blend_token = module
            .meta
            .get("compositeOperation")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .to_string();
        let blend = self.render_blend_mode(&blend_token);

        let (width, height) = self.size;
        // Cheap handle clones (Arc): they keep the instance borrow below
        // separate from the rest of `self`.
        let device = self.device.clone();
        let queue = self.queue.clone();
        let texture_id = two_d_texture_id(module_id);
        let props = &module.props;
        let audio = &frame.audio;
        // The JS `delta` argument is the `requestAnimationFrame` timestamp
        // (`renderers/2d/index.js:24`), i.e. a monotonic millisecond clock —
        // not a frame delta; `Bar.js:62` and `Concentrics.js:78` integrate it.
        let delta_ms = frame.time_seconds * 1000.0;

        let Some(instance) = self.two_d.get_mut(module_id) else {
            return Ok(());
        };

        // Canvas: created on first draw, rebuilt when the output resizes.
        if instance.canvas.is_none() {
            instance.canvas = Some(Canvas::new(&device, &queue, width, height));
        }
        let size_changed = instance.size != Some((width, height));
        if size_changed && instance.initialised {
            if let Some(canvas) = instance.canvas.as_mut() {
                canvas.resize(&device, &queue, width, height);
            }
        }
        let Some(canvas) = instance.canvas.as_ref() else {
            return Ok(());
        };
        // Register the canvas so the frame-graph composite can sample it.
        self.surfaces
            .insert(texture_id.clone(), canvas.texture().clone());

        // `init` once on creation, `resize` when the output size changed.
        if !instance.initialised {
            instance.initialised = true;
            instance.size = Some((width, height));
            let mut ctx = ModuleContext {
                canvas_width: width,
                canvas_height: height,
                delta: delta_ms,
                audio,
                frame: instance.frame,
                props,
                device: &device,
                queue: &queue,
            };
            instance.module.init(&mut ctx);
        } else if size_changed {
            instance.size = Some((width, height));
            let mut ctx = ModuleContext {
                canvas_width: width,
                canvas_height: height,
                delta: delta_ms,
                audio,
                frame: instance.frame,
                props,
                device: &device,
                queue: &queue,
            };
            instance.module.resize(&mut ctx);
        }

        // The JS calls `update` every frame before `draw` (`loop.js:424-445`);
        // the returned data is the instance's own state in the port.
        instance.frame += 1;
        let frame_index = instance.frame;
        let mut ctx = ModuleContext {
            canvas_width: width,
            canvas_height: height,
            delta: delta_ms,
            audio,
            frame: frame_index,
            props,
            device: &device,
            queue: &queue,
        };
        instance.module.update(&mut ctx);

        let Some(canvas) = instance.canvas.as_mut() else {
            return Ok(());
        };
        // The JS renderer wraps `draw` in `save()`/`restore()`; `begin_frame`
        // is the facade's equivalent state reset. A pending group clear is
        // folded into the draw pass (`clear_all` makes the first pass load with
        // `Clear`), so clearing costs no extra submit.
        canvas.begin_frame();
        if instance.clear_requested {
            instance.clear_requested = false;
            canvas.clear_all();
        }
        {
            let mut draw = DrawContext {
                canvas,
                props,
                audio,
                delta: delta_ms,
                frame: frame_index,
                device: &device,
                queue: &queue,
                // Media inputs arrive in M6; until then `Webcam`/`Texture2D`
                // draw nothing, like the JS with no video/texture source.
                input_texture: None,
                input_size: None,
                is_gallery: false,
            };
            instance.module.draw(&mut draw);
        }
        if let Some(canvas) = instance.canvas.as_mut() {
            canvas.submit();
        }

        self.composite(texture_id, target.clone(), alpha, blend)
    }

    /// Create the Point10 scene instance for `module_id` if it does not exist.
    ///
    /// Returns `false` when `module_name` is not a registered Point10 scene;
    /// the failure is remembered so a Scene that outlives its registration does
    /// not warn every frame.
    fn create_point10_instance(&mut self, module_name: &str, module_id: &str) -> bool {
        if self.failed_point10.contains(module_id) {
            return false;
        }
        match self.registry.create_point10_instance(module_name) {
            Some(scene) => {
                // The scene target is the group format: none of the batch-1
                // scenes run behind the postfx chain (see the module docs).
                let context =
                    SceneContext::new(&self.device, &self.queue, wgpu::TextureFormat::Rgba8Unorm);
                self.point10.insert(
                    module_id.to_string(),
                    Point10Instance {
                        scene,
                        context,
                        bridge: AudioBridge::new(),
                        target: None,
                        size: None,
                        initialised: false,
                        last_time: None,
                    },
                );
                true
            }
            None => {
                self.warn_once(
                    format!("unregistered-point10:{module_name}"),
                    &format!(
                        "engine: WARN Point10 scene '{module_name}' has no ported scene wrapper; \
                         skipping"
                    ),
                );
                self.failed_point10.insert(module_id.to_string());
                false
            }
        }
    }

    /// Render one Point10 scene into its target and composite it onto the group
    /// target (`adapter.js:780-836`, `renderers/2d/index.js:29-96`).
    ///
    /// The per-instance frame clock matches `point10/timing.js`: the first
    /// frame is `dt = 0` and later frames are clamped to 0.1 s, because Point10
    /// scenes integrate `dt` while the 2D facade passes an absolute
    /// `requestAnimationFrame` timestamp as its `delta`.
    fn render_point10_module(
        &mut self,
        module: &ModuleInstance,
        module_id: &str,
        target: &TextureId,
        frame: &FrameInfo,
    ) -> Result<(), CompositorError> {
        if !self.point10.contains_key(module_id)
            && !self.create_point10_instance(&module.module_name, module_id)
        {
            return Ok(());
        }

        // `loop.js:410-412`: alpha and composite operation come from the meta.
        let alpha = module
            .meta
            .get("alpha")
            .and_then(Value::as_f64)
            .unwrap_or(1.0);
        let blend_token = module
            .meta
            .get("compositeOperation")
            .and_then(Value::as_str)
            .unwrap_or("normal")
            .to_string();
        let blend = self.render_blend_mode(&blend_token);

        let (width, height) = self.size;
        let texture_id = point10_texture_id(module_id);

        // Rebuild the scene target when it is missing or the output resized.
        // The allocation needs `&self`, so it happens before the instance is
        // borrowed mutably.
        let needs_target = self
            .point10
            .get(module_id)
            .map(|instance| instance.target.is_none() || instance.size != Some((width, height)))
            .unwrap_or(false);
        let new_target = if needs_target {
            let texture =
                self.create_output_texture(&format!("modv-point10-{module_id}"), width, height);
            // wgpu does not zero-initialise allocations; every scene clears its
            // own target on render, but a scene that declines to draw must still
            // composite transparent black rather than undefined memory.
            self.clear_texture(&texture.texture, "point10-allocate");
            Some(texture)
        } else {
            None
        };

        // Register whichever texture the scene draws into so the frame-graph
        // composite can sample it.
        let target_texture = match &new_target {
            Some(texture) => Some(texture.texture.clone()),
            None => self
                .point10
                .get(module_id)
                .and_then(|instance| instance.target.as_ref())
                .map(|texture| texture.texture.clone()),
        };
        let Some(target_texture) = target_texture else {
            return Ok(());
        };
        self.surfaces.insert(texture_id.clone(), target_texture);

        // Cheap handle clone (Arc) so the submit below does not borrow `self`
        // while the instance is borrowed.
        let queue = self.queue.clone();
        let time = frame.time_seconds;
        let payload = interim_band_payload(&frame.audio);
        let props = &module.props;

        {
            let Some(instance) = self.point10.get_mut(module_id) else {
                return Ok(());
            };
            if let Some(texture) = new_target {
                instance.target = Some(texture);
            }

            // `init` once, `resize` when the output size changed.
            if instance.size != Some((width, height)) {
                instance.size = Some((width, height));
                instance.context.resize(width, height);
                if instance.initialised {
                    instance.scene.resize(width, height);
                } else {
                    instance.scene.init(&mut instance.context);
                    instance.initialised = true;
                }
            }

            // `timing.js: createFrameClock` — first frame 0, then clamped
            // elapsed seconds.
            let dt = match instance.last_time {
                Some(previous) if (time - previous).is_finite() => {
                    (time - previous).clamp(0.0, MAX_POINT10_FRAME_SECONDS)
                }
                Some(_) => 0.0,
                None => 0.0,
            };
            instance.last_time = Some(time);

            // `adapter.js:797-804`: scene props, then the bridge levers, then
            // `update`. Both read the live prop bag; the schema supplies the
            // defaults and the clamps.
            let mut get = |key: &str| {
                props
                    .get(key)
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite())
                    .map(|value| value as f32)
            };
            instance.scene.apply_props(&mut get);
            let levers = ResponsivenessLevers::from_getter(|key| get(key));
            instance.bridge.update(dt, Some(&payload), &levers);
            instance.scene.update(dt, &instance.bridge);

            let Some(texture) = instance.target.as_ref() else {
                return Ok(());
            };
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("modv-point10-scene"),
                });
            instance.scene.render(&mut encoder, &texture.view);
            queue.submit(Some(encoder.finish()));
        }

        self.composite(texture_id, target.clone(), alpha, blend)
    }

    /// Main composite (`loop.js:555-663`).
    fn compose_main(&mut self, state: &EngineState) -> Result<(), CompositorError> {
        let order = state.group_index_render_order();
        let mut graph = FrameGraph::new();

        for &index in &order {
            let Some(group) = state.groups.get(index) else {
                continue;
            };
            // Skip the Gallery group by name, not by position: a layout
            // without the Gallery panel renders every row (`loop.js:628-637`).
            if group.name == GALLERY_GROUP_NAME {
                continue;
            }
            if group.enabled != GROUP_ENABLED || group.modules.is_empty() || !(group.alpha > 0.0) {
                continue;
            }

            let source = self.group_target(&group.id);
            let blend = self.render_blend_mode(&group.composite_operation);
            graph.push(PassNode::Composite {
                source,
                target: main_id(),
                alpha: group.alpha,
                blend,
            });
        }

        self.executor.execute(&graph, &mut self.surfaces)?;
        Ok(())
    }

    /// Ensure the output-sized textures exist and are registered.
    fn ensure_output_textures(&mut self) {
        let (width, height) = self.size;
        if self.main.is_none() {
            let main = self.create_output_texture("modv-main", width, height);
            self.surfaces.insert(main_id(), main.texture.clone());
            self.main = Some(main);
        }
        if self.module_input.is_none() {
            let input = self.create_output_texture("modv-module-input", width, height);
            self.module_input = Some(input);
        }
        if self.module_output.is_none() {
            let output = self.create_output_texture("modv-module-output", width, height);
            self.surfaces
                .insert(module_output_id(), output.texture.clone());
            self.module_output = Some(output);
        }
    }

    /// A group's texture, allocated on demand and registered for the graph.
    ///
    /// A freshly allocated texture is cleared immediately. wgpu does not
    /// zero-initialise allocations, and an inheritance source can be resolved
    /// (and sampled) before its own group renders — clearing at allocation
    /// keeps that path defined and matches the JS canvas, which starts empty.
    fn group_target(&mut self, group_id: &str) -> TextureId {
        let id = group_texture_id(group_id);
        if self.group_targets.contains(group_id) {
            return id;
        }

        let (width, height) = self.size;
        let texture = self.create_output_texture(&format!("modv-group-{group_id}"), width, height);
        self.clear_texture(&texture.texture, "group-allocate");
        self.surfaces.insert(id.clone(), texture.texture);
        self.group_targets.insert(group_id.to_string());
        id
    }

    fn create_output_texture(&self, label: &str, width: u32, height: u32) -> OutputTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        OutputTexture { texture, view }
    }

    /// Drop group textures and module slots for objects that left the state.
    fn prune(&mut self, state: &EngineState) {
        let live_groups: HashSet<&str> =
            state.groups.iter().map(|group| group.id.as_str()).collect();
        let stale_groups: Vec<String> = self
            .group_targets
            .iter()
            .filter(|id| !live_groups.contains(id.as_str()))
            .cloned()
            .collect();
        for id in stale_groups {
            self.group_targets.remove(&id);
            self.surfaces.remove(&group_texture_id(&id));
        }

        let live_modules: HashSet<&str> = state.modules.keys().map(String::as_str).collect();
        let stale_modules: Vec<String> = self
            .slots
            .keys()
            .filter(|id| !live_modules.contains(id.as_str()))
            .cloned()
            .collect();
        for id in stale_modules {
            if let Some(slot) = self.slots.remove(&id) {
                self.shader.remove_slot(slot);
            }
        }
        self.failed_modules
            .retain(|id| live_modules.contains(id.as_str()));

        let stale_two_d: Vec<String> = self
            .two_d
            .keys()
            .filter(|id| !live_modules.contains(id.as_str()))
            .cloned()
            .collect();
        for id in stale_two_d {
            if let Some(mut instance) = self.two_d.remove(&id) {
                instance.module.dispose();
            }
            self.surfaces.remove(&two_d_texture_id(&id));
        }
        self.failed_two_d
            .retain(|id| live_modules.contains(id.as_str()));

        let stale_point10: Vec<String> = self
            .point10
            .keys()
            .filter(|id| !live_modules.contains(id.as_str()))
            .cloned()
            .collect();
        for id in stale_point10 {
            if let Some(mut instance) = self.point10.remove(&id) {
                instance.scene.dispose();
            }
            self.surfaces.remove(&point10_texture_id(&id));
        }
        self.failed_point10
            .retain(|id| live_modules.contains(id.as_str()));
    }

    /// Set up (once) and return the shader pipeline slot for a module id.
    fn slot_for(&mut self, module_name: &str, module_id: &str) -> Option<usize> {
        if let Some(slot) = self.slots.get(module_id) {
            return Some(*slot);
        }
        if self.failed_modules.contains(module_id) {
            return None;
        }

        let Some(definition) = self.registry.shader_module(module_name) else {
            self.warn_once(
                format!("unregistered:{module_name}"),
                &format!(
                    "engine: WARN shader module '{module_name}' is not in the registry; skipping"
                ),
            );
            self.failed_modules.insert(module_id.to_string());
            return None;
        };

        match self
            .shader
            .setup_module(definition, wgpu::TextureFormat::Rgba8Unorm, false)
        {
            Ok(slot) => {
                self.slots.insert(module_id.to_string(), slot);
                Some(slot)
            }
            Err(error) => {
                eprintln!("engine: WARN could not set up shader module '{module_name}': {error}");
                self.failed_modules.insert(module_id.to_string());
                None
            }
        }
    }

    fn copy_texture(&self, from: &wgpu::Texture, to: &wgpu::Texture) {
        if from.width() != to.width() || from.height() != to.height() {
            return;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("modv-module-input-copy"),
            });
        encoder.copy_texture_to_texture(
            from.as_image_copy(),
            to.as_image_copy(),
            wgpu::Extent3d {
                width: from.width(),
                height: from.height(),
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
    }

    /// Clear `texture` to transparent black — the canvas `clearRect` state
    /// (`loop.js:319-321`).
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
                        // Canvas `clearRect` is transparent black
                        // (`loop.js:319-321`).
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

    /// Run one `Composite` node through the frame graph executor, so canvas
    /// blend semantics live in exactly one place (`modv_render::frame_graph`).
    fn composite(
        &mut self,
        source: TextureId,
        target: TextureId,
        alpha: f64,
        blend: RenderBlendMode,
    ) -> Result<(), CompositorError> {
        let mut graph = FrameGraph::new();
        graph.push(PassNode::Composite {
            source,
            target,
            alpha,
            blend,
        });
        self.executor.execute(&graph, &mut self.surfaces)?;
        Ok(())
    }

    /// Parse a preset blend token into the renderer's enum.
    ///
    /// Parsing goes through [`modv_engine::blend::BlendMode::from_str`] (the
    /// JS token list) so an unknown token is detected as such and warned about
    /// once; the variants then map 1:1 onto `modv_render::blend::BlendMode`.
    fn render_blend_mode(&mut self, token: &str) -> RenderBlendMode {
        match parse_blend_mode(token) {
            Ok(mode) => mode,
            Err(unknown) => {
                self.warn_once(
                    format!("blend:{unknown}"),
                    &format!("engine: WARN unknown composite operation '{unknown}'; using normal"),
                );
                RenderBlendMode::Normal
            }
        }
    }

    fn warn_once(&mut self, key: String, message: &str) {
        if self.warned.insert(key) {
            eprintln!("{message}");
        }
    }

    /// Present the main texture to the swapchain's `target` view.
    ///
    /// The pass clears the target to the M0 near-black first, so an empty state
    /// still presents a defined colour, then draws a fullscreen triangle
    /// sampling main with premultiplied source-over ("normal") blending. No
    /// colour-space conversion happens here; see the module docs on the sRGB
    /// present boundary.
    pub fn present(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        let Some(main) = &self.main else {
            return;
        };

        if self
            .present_pipeline
            .as_ref()
            .map(|(pipeline_format, _)| *pipeline_format != format)
            .unwrap_or(true)
        {
            let pipeline = self.create_present_pipeline(format);
            self.present_pipeline = Some((format, pipeline));
        }
        let Some((_, pipeline)) = &self.present_pipeline else {
            return;
        };

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("modv-present-bind-group"),
            layout: &self.present_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&main.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.present_sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("modv-present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Same near-black (#0b0b0f) the M0 window clears to.
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.043,
                        g: 0.043,
                        b: 0.059,
                        a: 1.0,
                    }),
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
    }

    fn create_present_pipeline(&self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("modv-present-shader"),
                source: wgpu::ShaderSource::Wgsl(PRESENT_WGSL.into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("modv-present-pipeline-layout"),
                bind_group_layouts: &[Some(&self.present_layout)],
                immediate_size: 0,
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("modv-present-pipeline"),
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
                        format,
                        // Premultiplied canvas values composited over the
                        // cleared surface — the canvas "normal" mode.
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    /// Read the main texture back as tightly packed RGBA8
    /// (`width * height * 4` bytes).
    ///
    /// This is a synchronised read-back and only runs at the preview cadence
    /// (≤ 30 fps). The no-readback hot path promised by WS-2 applies to
    /// presentation; shared-memory / DMA-BUF preview transport lands later.
    pub fn read_main_rgba8(&mut self) -> Result<Vec<u8>, CompositorError> {
        let (width, height) = self.size;
        let Some(main) = &self.main else {
            return Ok(Vec::new());
        };
        read_back_texture(&self.device, &self.queue, &main.texture, width, height)
            .map_err(CompositorError::ReadBack)
    }

    /// Copy the main texture into the preview ring, nearest-neighbour
    /// downsampled to the ring's 320 × 180 RGBA8 frame.
    ///
    /// Returns the written [`PreviewFrame`] so the supervisor can publish the
    /// `preview/frame` event, or `None` when there is nothing to publish (no
    /// groups have ever rendered yet).
    pub fn read_preview(
        &mut self,
        ring: &mut PreviewRing,
    ) -> Result<Option<PreviewFrame>, CompositorError> {
        let (width, height) = self.size;
        if width == 0 || height == 0 || self.main.is_none() {
            return Ok(None);
        }

        let pixels = self.read_main_rgba8()?;
        if pixels.len() != (width as usize * height as usize * 4) {
            return Ok(None);
        }

        let mut preview = vec![0u8; (PREVIEW_WIDTH as usize) * (PREVIEW_HEIGHT as usize) * 4];
        for y in 0..PREVIEW_HEIGHT {
            let source_y = (y as u64 * height as u64 / PREVIEW_HEIGHT as u64) as usize;
            for x in 0..PREVIEW_WIDTH {
                let source_x = (x as u64 * width as u64 / PREVIEW_WIDTH as u64) as usize;
                let source = ((source_y * width as usize) + source_x) * 4;
                let target = ((y as usize * PREVIEW_WIDTH as usize) + x as usize) * 4;
                preview[target..target + 4].copy_from_slice(&pixels[source..source + 4]);
            }
        }

        ring.write_frame(&preview)
            .map(Some)
            .map_err(|error| CompositorError::Preview(error.to_string()))
    }
}

/// WGSL for the present blit: one fullscreen triangle sampling main.
///
/// `textureDimensions` supplies the uv scale so no uniform buffer is needed;
/// the sampler is clamp/linear, and the sampled premultiplied value is written
/// as-is (the surface format, often sRGB, is the present boundary).
const PRESENT_WGSL: &str = r#"// modV present blit — main texture to the swapchain.
@group(0) @binding(0) var main_texture: texture_2d<f32>;
@group(0) @binding(1) var main_sampler: sampler;

@vertex fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0),
  );
  return vec4<f32>(positions[vertex_index], 0.0, 1.0);
}

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let size = vec2<f32>(textureDimensions(main_texture));
  return textureSample(main_texture, main_sampler, position.xy / size);
}
"#;

type RenderBlendMode = modv_render::blend::BlendMode;

/// Read a texture back as tightly packed RGBA8, padding rows to
/// `COPY_BYTES_PER_ROW_ALIGNMENT` on the GPU and de-padding here.
///
/// Ported locally rather than reusing
/// [`modv_render::headless::read_back_rgba8`]: that helper takes `&Engine`
/// (instance + adapter + device + queue), while the compositor deliberately
/// owns only cloned device/queue handles. The copy/map/poll dance is identical.
fn read_back_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let unpadded_bytes_per_row = u64::from(width) * 4;
    let align = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
    let buffer_size = padded_bytes_per_row * u64::from(height);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("modv-preview-read-back"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("modv-preview-read-back"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|error| error.to_string())?;
    receiver
        .recv()
        .map_err(|error| format!("map callback channel closed: {error}"))?
        .map_err(|error| error.to_string())?;

    let mapped = slice
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    let mut out = Vec::with_capacity((unpadded_bytes_per_row * u64::from(height)) as usize);
    for row in 0..u64::from(height) {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + unpadded_bytes_per_row as usize;
        out.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(out)
}

/// Encode the module's prop values as the float lanes its prop buffer expects,
/// in declaration order (`uniforms::PropLayout`).
fn encode_props(definition: &ShaderModule, props: &Value) -> PropSlots {
    let mut floats = Vec::new();
    for prop in &definition.props {
        let value = props.get(prop.name).unwrap_or(&Value::Null);
        push_prop_lanes(&mut floats, prop, value);
    }
    PropSlots { floats }
}

fn push_prop_lanes(floats: &mut Vec<f32>, prop: &ShaderProp, value: &Value) {
    let lane = |value: Option<f64>| value.unwrap_or(prop.default) as f32;
    match prop.kind {
        PropKind::Float => floats.push(lane(value.as_f64())),
        PropKind::Int => floats.push(lane(value.as_f64()).round()),
        PropKind::Enum => floats.push(lane(value.as_f64()).round()),
        PropKind::Bool => floats.push(if value.as_bool() == Some(true) {
            1.0
        } else {
            0.0
        }),
        PropKind::Vec2 | PropKind::Vec3 | PropKind::Vec4 => {
            let count = prop.kind.float_lanes();
            let entries = value.as_array();
            for index in 0..count {
                let entry = entries
                    .and_then(|entries| entries.get(index))
                    .and_then(Value::as_f64);
                floats.push(lane(entry));
            }
        }
        PropKind::Color => {
            for key in ["r", "g", "b", "a"] {
                let entry = value.get(key).and_then(Value::as_f64);
                floats.push(lane(entry));
            }
        }
    }
}

/// Parse a JS blend token (`composite-operations.js`) into the renderer enum.
///
/// `Err` carries the offending token so the caller can warn once.
fn parse_blend_mode(token: &str) -> Result<RenderBlendMode, String> {
    EngineBlendMode::from_str(token)
        .map(map_blend_mode)
        .map_err(|error| error.0)
}

/// Map the engine's blend enum (the JS token set) onto the renderer's.
fn map_blend_mode(mode: EngineBlendMode) -> RenderBlendMode {
    use modv_render::blend::BlendMode as Render;
    match mode {
        EngineBlendMode::Normal => Render::Normal,
        EngineBlendMode::Multiply => Render::Multiply,
        EngineBlendMode::Overlay => Render::Overlay,
        EngineBlendMode::Darken => Render::Darken,
        EngineBlendMode::Lighten => Render::Lighten,
        EngineBlendMode::ColorDodge => Render::ColorDodge,
        EngineBlendMode::ColorBurn => Render::ColorBurn,
        EngineBlendMode::HardLight => Render::HardLight,
        EngineBlendMode::SoftLight => Render::SoftLight,
        EngineBlendMode::Difference => Render::Difference,
        EngineBlendMode::Exclusion => Render::Exclusion,
        EngineBlendMode::Hue => Render::Hue,
        EngineBlendMode::Saturation => Render::Saturation,
        EngineBlendMode::Color => Render::Color,
        EngineBlendMode::Luminosity => Render::Luminosity,
        EngineBlendMode::Clear => Render::Clear,
        EngineBlendMode::Copy => Render::Copy,
        EngineBlendMode::Destination => Render::Destination,
        EngineBlendMode::SourceOver => Render::SourceOver,
        EngineBlendMode::DestinationOver => Render::DestinationOver,
        EngineBlendMode::SourceIn => Render::SourceIn,
        EngineBlendMode::DestinationIn => Render::DestinationIn,
        EngineBlendMode::SourceOut => Render::SourceOut,
        EngineBlendMode::DestinationOut => Render::DestinationOut,
        EngineBlendMode::SourceAtop => Render::SourceAtop,
        EngineBlendMode::DestinationAtop => Render::DestinationAtop,
        EngineBlendMode::Xor => Render::Xor,
        EngineBlendMode::Lighter => Render::Lighter,
    }
}

fn main_id() -> TextureId {
    TextureId::from(MAIN_TEXTURE_ID)
}

fn module_output_id() -> TextureId {
    TextureId::from("module-output")
}

fn group_texture_id(group_id: &str) -> TextureId {
    TextureId::from(format!("group:{group_id}"))
}

/// The frame-graph texture id of a 2D module's canvas.
fn two_d_texture_id(module_id: &str) -> TextureId {
    TextureId::from(format!("two-d:{module_id}"))
}

/// The frame-graph texture id of a Point10 scene's target.
fn point10_texture_id(module_id: &str) -> TextureId {
    TextureId::from(format!("point10:{module_id}"))
}

/// Longest frame the Point10 scene clock reports, mirroring
/// `MAX_FRAME_SECONDS` in `point10/timing.js`.
const MAX_POINT10_FRAME_SECONDS: f32 = 0.1;

/// Sample rate reported by the interim audio payload.
///
/// The store's [`AudioFrame`] carries no sample rate yet (the renderer's
/// `AudioBands` runs at the `AudioContext` rate); 48 kHz is the engine's
/// assumption until the analyser lands in M6. No batch-1 scene reads it for
/// anything but the chromatic ring-buffer append count
/// (`ChromaticLine.js:168`), so a wrong value only changes how densely the
/// wave texture is filled, never a pitch or a level.
const INTERIM_SAMPLE_RATE: f32 = 48_000.0;

/// Build the `audio-bands.js` payload the Point10 [`AudioBridge`] consumes from
/// the engine's interim audio frame.
///
/// M4 has no analyser: `meyda/SET_BANDS` publishes three AGC bands and the
/// store keeps them in [`AudioFrame`]. The bridge's payload has twelve damping
/// lanes (`[damping * 3 + band]`) and sixteen log-spaced cells, so each band is
/// replicated across its own lanes and cells — bands 0/1/2 own cells `0..=4`,
/// `5..=9` and `10..=15` ([`band_of_cell`]), and lanes `band`, `3 + band`,
/// `6 + band`, `9 + band`. The replication gives every scene the same value
/// across damping, i.e. the payload is already as smooth as the three-band
/// source allows; the levers' `smoothing` still low-passes it per module.
///
/// `wave` and `freq_mag` are the frame's arrays (zeroed until M6) and the byte
/// spectrum is the frame's 1024 bins (also zeroed today — non-empty, so scenes
/// take the "silent audio" branch rather than the "no analyser" one). M6
/// replaces this function with the real `AudioBands` payload; nothing else
/// changes.
fn interim_band_payload(audio: &AudioFrame) -> BandPayload {
    let bands = [audio.bass, audio.mid, audio.treble];

    let mut lanes = [0.0; LANES_PER_BAND * 3];
    for damping in 0..LANES_PER_BAND {
        for (band, value) in bands.iter().enumerate() {
            lanes[damping * 3 + band] = *value;
        }
    }

    let mut cells = [0.0; 16];
    for (cell, store) in cells.iter_mut().enumerate() {
        *store = bands[band_of_cell(cell)];
    }

    BandPayload {
        sample_rate: INTERIM_SAMPLE_RATE,
        lanes,
        cells,
        beat: audio.beat,
        wave: audio.wave,
        freq_mag: [0.0; AUDIO_SAMPLES],
        // The three-band source has no AGC-free twin, so `raw` mirrors the
        // bands: `useAgc: false` must not silence every scene.
        raw: RawBands { lanes, cells },
        byte_frequency_data: audio.byte_frequency_data.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modv_engine::engine_state::ModuleInstance;
    use modv_render::{Engine, EngineConfig};
    use modv_renderers::shader::all_modules;
    use serde_json::json;

    const SIZE: u32 = 64;

    /// Point10 scenes are rendered a bit larger: their `LineStrip` strips and
    /// glow tails need more than 64 px to cover pixels reliably.
    const POINT10_SIZE: u32 = 128;

    fn try_engine() -> Option<Engine> {
        match Engine::new(&EngineConfig {
            force_fallback_adapter: true,
        }) {
            Ok(engine) => Some(engine),
            Err(error) => {
                eprintln!("skipping compositor pixel test: no adapter ({error})");
                None
            }
        }
    }

    fn frame_info() -> FrameInfo {
        FrameInfo {
            time_seconds: 0.0,
            frame_index: 0,
            date: [2026.0, 10.0, 4.0, 0.0],
            audio: AudioFrame::default(),
        }
    }

    /// One group (`Effects`) with one `Point10 Invert` module. The group clears
    /// to transparent black, so Invert's `1 - canvas` yields opaque white.
    fn invert_state(module_alpha: f64) -> EngineState {
        let group: Group = serde_json::from_value(json!({
            "id": "group-a",
            "name": "Effects",
            "hidden": false,
            "modules": ["module-a"],
            "enabled": GROUP_ENABLED,
            "alpha": 1,
            "inherit": true,
            "inheritFrom": -1,
            "pipeline": false,
            "clearing": true,
            "compositeOperation": "normal"
        }))
        .expect("group decodes");

        let module: ModuleInstance = serde_json::from_value(json!({
            "$id": "module-a",
            "$moduleName": "Point10 Invert",
            "meta": {
                "name": "Point10 Invert",
                "type": "shader",
                "enabled": true,
                "alpha": module_alpha,
                "compositeOperation": "normal"
            },
            "props": {},
            "data": {}
        }))
        .expect("module decodes");

        let mut state = EngineState::new();
        state.groups.push(group);
        state.modules.insert("module-a".to_string(), module);
        state
    }

    fn assert_pixels_close(actual: &[u8], expected: [u8; 4], tolerance: i16, context: &str) {
        assert_eq!(
            actual.len(),
            (SIZE * SIZE * 4) as usize,
            "{context}: unexpected read-back length"
        );
        for (index, chunk) in actual.chunks_exact(4).enumerate() {
            for channel in 0..4 {
                let diff = chunk[channel] as i16 - expected[channel] as i16;
                assert!(
                    diff.abs() <= tolerance,
                    "{context}: pixel {index} channel {channel} is {} (expected {expected:?} \
                     ±{tolerance})",
                    chunk[channel]
                );
            }
        }
    }

    #[test]
    fn shader_module_renders_through_copy_and_composite_chain() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(SIZE, SIZE);

        let state = invert_state(1.0);
        compositor
            .render_frame(&state, &frame_info())
            .expect("frame renders");

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        assert_pixels_close(
            &pixels,
            [255, 255, 255, 255],
            2,
            "invert over cleared group",
        );
    }

    #[test]
    fn module_alpha_scales_the_composite() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(SIZE, SIZE);

        compositor
            .render_frame(&invert_state(0.5), &frame_info())
            .expect("frame renders");

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        // Opaque white at globalAlpha 0.5 over transparent black: premultiplied
        // (0.5, 0.5, 0.5, 0.5) -> every channel 128.
        assert_pixels_close(&pixels, [128, 128, 128, 128], 2, "module alpha 0.5");
    }

    #[test]
    fn preview_reads_back_a_downsampled_frame() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(SIZE, SIZE);
        compositor
            .render_frame(&invert_state(1.0), &frame_info())
            .expect("frame renders");

        let path = std::env::temp_dir().join(format!(
            "modv-app-preview-test-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut ring = PreviewRing::create(Some(path), 3, PREVIEW_WIDTH, PREVIEW_HEIGHT)
            .expect("preview ring is created");
        let slot_bytes = ring.handshake()["slotBytes"]
            .as_u64()
            .expect("handshake carries slotBytes") as usize;
        let frame = compositor
            .read_preview(&mut ring)
            .expect("preview read-back succeeds")
            .expect("a frame is published");
        assert_eq!(frame.width, PREVIEW_WIDTH);
        assert_eq!(frame.height, PREVIEW_HEIGHT);

        let (header, pixels) = PreviewRing::read_slot(ring.path(), slot_bytes, frame.slot)
            .expect("slot reads")
            .expect("slot holds a consistent frame");
        assert_eq!(header.width, PREVIEW_WIDTH);
        assert_eq!(pixels.len(), (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize);
        let first = &pixels[0..4];
        assert!(first[0] >= 250 && first[1] >= 250 && first[2] >= 250 && first[3] >= 250);
    }

    #[test]
    fn gallery_group_never_reaches_the_main_texture() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(SIZE, SIZE);

        let mut state = invert_state(1.0);
        // Rename the group to the Gallery name: its module must not render and
        // its canvas must not composite to main.
        state.groups[0].name = GALLERY_GROUP_NAME.to_string();
        compositor
            .render_frame(&state, &frame_info())
            .expect("frame renders");

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        assert_pixels_close(&pixels, [0, 0, 0, 0], 0, "gallery group is inert");
    }

    #[test]
    fn blend_tokens_map_and_unknown_tokens_are_rejected() {
        assert_eq!(parse_blend_mode("multiply"), Ok(RenderBlendMode::Multiply));
        assert_eq!(
            parse_blend_mode("source-over"),
            Ok(RenderBlendMode::SourceOver)
        );
        assert_eq!(parse_blend_mode("normal"), Ok(RenderBlendMode::Normal));
        assert_eq!(
            parse_blend_mode("definitely-not-a-blend"),
            Err("definitely-not-a-blend".to_string())
        );
    }

    #[test]
    fn prop_lanes_follow_the_registry_declaration_order() {
        let registry = all_modules();
        let wobble = registry
            .iter()
            .find(|module| module.name == "Wobble")
            .expect("Wobble is registered");
        // Wobble declares `strength` then `size`.
        let props = json!({"strength": 0.02, "size": 7.0});
        let slots = encode_props(wobble, &props);
        assert_eq!(slots.floats, vec![0.02_f32, 7.0_f32]);

        // Missing values fall back to the declared default.
        let slots = encode_props(wobble, &json!({}));
        assert_eq!(slots.floats, vec![0.001_f32, 1.0_f32]);
    }

    // ————————————————— 2D module path —————————————————

    /// A group holding `module_ids`, all render flags explicit.
    fn group_with(
        id: &str,
        name: &str,
        module_ids: &[&str],
        enabled: i64,
        alpha: f64,
        clearing: bool,
        inherit: bool,
        inherit_from: Value,
    ) -> Group {
        serde_json::from_value(json!({
            "id": id,
            "name": name,
            "hidden": false,
            "modules": module_ids,
            "enabled": enabled,
            "alpha": alpha,
            "inherit": inherit,
            "inheritFrom": inherit_from,
            "pipeline": false,
            "clearing": clearing,
            "compositeOperation": "normal"
        }))
        .expect("group decodes")
    }

    /// A 2D module record (`meta.type == "2d"`).
    fn two_d_module(module_id: &str, name: &str, props: Value) -> ModuleInstance {
        serde_json::from_value(json!({
            "$id": module_id,
            "$moduleName": name,
            "meta": {
                "name": name,
                "type": "2d",
                "enabled": true,
                "alpha": 1.0,
                "compositeOperation": "normal"
            },
            "props": props,
            "data": {}
        }))
        .expect("2D module decodes")
    }

    fn lit_pixels(pixels: &[u8]) -> usize {
        pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0).count()
    }

    fn read_group_texture(compositor: &Compositor, group_id: &str) -> Vec<u8> {
        let texture = compositor
            .surfaces
            .get(&group_texture_id(group_id))
            .expect("the group rendered a texture");
        read_back_texture(&compositor.device, &compositor.queue, texture, SIZE, SIZE)
            .expect("group texture reads back")
    }

    #[test]
    fn two_d_module_renders_through_the_canvas_facade() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(SIZE, SIZE);

        let mut state = EngineState::new();
        state.groups.push(group_with(
            "group-a",
            "Effects",
            &["module-a"],
            GROUP_ENABLED,
            1.0,
            true,
            true,
            json!(-1),
        ));
        state.modules.insert(
            "module-a".to_string(),
            two_d_module(
                "module-a",
                "Ball",
                json!({
                    // A resolved palette value so the balls are opaque white
                    // (`props.color.value`; the module falls back to the raw
                    // palette's first entry otherwise).
                    "amount": 12,
                    "speed": 2.0,
                    "wrap": false,
                    "size": 4,
                    "intensity": 15,
                    "soundType": false,
                    "color": { "value": [255.0, 255.0, 255.0] },
                }),
            ),
        );

        compositor
            .render_frame(&state, &frame_info())
            .expect("frame renders");

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        let lit = lit_pixels(&pixels);
        assert!(lit > 0, "Ball drew nothing through the 2D canvas path");
        assert!(
            compositor.two_d.contains_key("module-a"),
            "the 2D instance is kept per module id"
        );

        // `reset` drops the 2D instances and canvases with the rest of the
        // GPU state.
        compositor.reset();
        assert!(compositor.two_d.is_empty());
    }

    #[test]
    fn two_d_group_inheritance_carries_the_previous_group_pixels() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(SIZE, SIZE);

        let mut state = EngineState::new();
        state.groups.push(group_with(
            "group-a",
            "Effects",
            &["module-a"],
            GROUP_ENABLED,
            1.0,
            true,
            true,
            json!(-1),
        ));
        // The second group clears, inherits the first and draws nothing of its
        // own (`Texture 2D` without an input texture is a no-op), so whatever
        // lands on its target came through inheritance (`loop.js:338-379`).
        state.groups.push(group_with(
            "group-b",
            "Foreground",
            &["module-b"],
            GROUP_ENABLED,
            1.0,
            true,
            true,
            json!(-1),
        ));
        state.modules.insert(
            "module-a".to_string(),
            two_d_module(
                "module-a",
                "Ball",
                json!({
                    "amount": 12,
                    "speed": 2.0,
                    "wrap": false,
                    "size": 4,
                    "intensity": 15,
                    "soundType": false,
                    "color": { "value": [255.0, 255.0, 255.0] },
                }),
            ),
        );
        state.modules.insert(
            "module-b".to_string(),
            two_d_module("module-b", "Texture 2D", json!({})),
        );

        compositor
            .render_frame(&state, &frame_info())
            .expect("frame renders");

        let first = read_group_texture(&compositor, "group-a");
        let second = read_group_texture(&compositor, "group-b");
        assert!(lit_pixels(&first) > 0, "the first group drew nothing");
        assert!(
            lit_pixels(&second) > 0,
            "the second group inherited nothing from the first"
        );
        assert_eq!(
            lit_pixels(&first),
            lit_pixels(&second),
            "inheriting at alpha 1 copies the first group's pixels exactly"
        );
    }

    #[test]
    fn set_bands_reaches_the_store_and_the_shader_audio_uniforms() {
        let mut store = crate::store_ops::Store::new();
        store
            .dispatch(
                "meyda/SET_BANDS",
                json!({
                    "bass": 0.75,
                    "mid": 0.5,
                    "treble": 0.25,
                    // The 16 band cells have no M3 consumer; they are ignored.
                    "cells": [0.1, 0.2, 0.3],
                }),
            )
            .expect("meyda/SET_BANDS dispatches");

        assert_eq!(store.audio().bass, 0.75);
        assert_eq!(store.audio().mid, 0.5);
        assert_eq!(store.audio().treble, 0.25);
        // The interim source carries bands only; wave/FFT stay silent.
        assert!(store.audio().wave.iter().all(|sample| *sample == 0.0));
        assert_eq!(store.audio().beat, 0.0);

        // The window builds `FrameInfo` from the store; the shader uniforms and
        // the 2D module contexts both read it.
        let frame = FrameInfo {
            audio: store.audio().clone(),
            ..frame_info()
        };
        let uniforms = frame.shader_audio();
        assert_eq!(uniforms.bass, 0.75);
        assert_eq!(uniforms.mid, 0.5);
        assert_eq!(uniforms.treble, 0.25);
        assert!(!uniforms.kick, "beat detection is WS-9 / M6");
    }

    // ————————————————— Point10 scene path (M4) —————————————————

    /// A Point10 module record (`meta.type == "2d"`, as the JS wrapper sets it).
    fn point10_module(module_id: &str, name: &str, props: Value) -> ModuleInstance {
        serde_json::from_value(json!({
            "$id": module_id,
            "$moduleName": name,
            "meta": {
                "name": name,
                "type": "2d",
                "enabled": true,
                "alpha": 1.0,
                "compositeOperation": "normal"
            },
            "props": props,
            "data": {}
        }))
        .expect("Point10 module decodes")
    }

    /// One group holding one Point10 module, sized by the caller's
    /// `Compositor::resize`.
    fn point10_state(name: &str, props: Value) -> EngineState {
        let mut state = EngineState::new();
        state.groups.push(group_with(
            "group-a",
            "Effects",
            &["module-a"],
            GROUP_ENABLED,
            1.0,
            true,
            true,
            json!(-1),
        ));
        state.modules.insert(
            "module-a".to_string(),
            point10_module("module-a", name, props),
        );
        state
    }

    /// Two frames, so the second one proves per-frame scene state survives.
    fn render_two_frames(compositor: &mut Compositor, state: &EngineState) {
        for (index, time) in [(0_u64, 0.0_f32), (1, 1.0 / 60.0)] {
            let frame = FrameInfo {
                time_seconds: time,
                frame_index: index,
                ..frame_info()
            };
            compositor
                .render_frame(state, &frame)
                .expect("Point10 frame renders");
        }
    }

    #[test]
    fn point10_chromatic_wave_renders_through_the_scene_path() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(POINT10_SIZE, POINT10_SIZE);

        let state = point10_state("Point10 Chromatic Wave", json!({}));
        render_two_frames(&mut compositor, &state);

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        let lit = lit_pixels(&pixels);
        assert!(
            lit > 0,
            "Point10 Chromatic Wave drew nothing through the scene path"
        );
        assert!(
            compositor.point10.contains_key("module-a"),
            "the scene instance is kept per module id"
        );
        // The scene target is registered for the composite pass.
        assert!(
            compositor
                .surfaces
                .get(&point10_texture_id("module-a"))
                .is_some(),
            "the scene target is registered with the frame graph"
        );

        // `reset` drops the scene and its target with the rest of the GPU
        // state.
        compositor.reset();
        assert!(compositor.point10.is_empty());
        assert!(
            compositor
                .surfaces
                .get(&point10_texture_id("module-a"))
                .is_none(),
            "reset drops the scene target"
        );
    }

    #[test]
    fn point10_waveform_renders_through_the_scene_path() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(POINT10_SIZE, POINT10_SIZE);

        // The prop bag flows through the scene binder; the default opacity is
        // already 1, so setting it explicitly also exercises that path.
        let state = point10_state("Point10 Waveform", json!({"opacity": 1.0}));
        render_two_frames(&mut compositor, &state);

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        assert!(
            lit_pixels(&pixels) > 0,
            "Point10 Waveform (line strips) drew nothing"
        );
    }

    #[test]
    fn point10_instances_are_pruned_when_the_module_leaves_the_state() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(POINT10_SIZE, POINT10_SIZE);

        let mut state = point10_state("Point10 Waveform", json!({}));
        render_two_frames(&mut compositor, &state);
        assert!(compositor.point10.contains_key("module-a"));

        // The module id survives in the group but the module is gone, exactly
        // like `loop.js:385-394`: the instance and its target are pruned.
        state.modules.clear();
        render_two_frames(&mut compositor, &state);
        assert!(compositor.point10.is_empty());
        assert!(
            compositor
                .surfaces
                .get(&point10_texture_id("module-a"))
                .is_none()
        );
    }

    #[test]
    fn interim_audio_payload_replicates_bands_across_lanes_and_cells() {
        let audio = AudioFrame {
            bass: 0.5,
            mid: 0.25,
            treble: 0.75,
            beat: 0.4,
            ..AudioFrame::default()
        };
        let payload = interim_band_payload(&audio);

        assert_eq!(payload.sample_rate, INTERIM_SAMPLE_RATE);
        // Twelve lanes: `damping * 3 + band`, each band replicated.
        assert_eq!(
            payload.lanes,
            [
                0.5, 0.25, 0.75, // damping 0
                0.5, 0.25, 0.75, // damping 1
                0.5, 0.25, 0.75, // damping 2
                0.5, 0.25, 0.75, // damping 3
            ]
        );
        // Sixteen cells: bass owns 0..=4, mid 5..=9, treble 10..=15
        // (`band_of_cell`).
        assert_eq!(&payload.cells[0..5], &[0.5; 5]);
        assert_eq!(&payload.cells[5..10], &[0.25; 5]);
        assert_eq!(&payload.cells[10..16], &[0.75; 6]);
        // `useAgc: false` must not silence the scenes: `raw` mirrors the bands.
        assert_eq!(payload.raw.lanes, payload.lanes);
        assert_eq!(payload.raw.cells, payload.cells);
        assert_eq!(payload.beat, 0.4);
        // Wave/FFT stay zeroed until M6, but the byte spectrum keeps the
        // analyser's 1024 bins so scenes take the "silence" branch.
        assert_eq!(payload.freq_mag, [0.0; AUDIO_SAMPLES]);
        assert!(payload.wave.iter().all(|sample| *sample == 0.0));
        assert_eq!(payload.byte_frequency_data, vec![0_u8; AUDIO_SAMPLES]);
    }
}
