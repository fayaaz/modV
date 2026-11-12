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
//! * **three modules do not render.** They log once and are skipped (their
//!   renderer is out of scope for the hybrid engine).
//! * **No `input_texture` for 2D modules.** `Webcam` and `Texture2D` draw
//!   nothing while `DrawContext::input_texture` is `None` — exactly what the
//!   JS does when the video/texture source is missing. Media inputs land with
//!   M6.
//! * **Row crossfades (M5c).** A row fade keeps the old group alive with
//!   `transitionGhost: true`. The compositor renders both halves even when
//!   their alpha would be culled (the transition-group exception,
//!   `loop.js:257-266`), blends them with `modv_render::frame_graph`'s
//!   `RowFade` pass, and assigns the blend to **both** group ids so a later
//!   row of the same frame inherits the crossfade (`loop.js:498-537`). The
//!   ghost itself is never composited to main and never becomes `last_canvas`;
//!   the incoming group composites the blend instead.
//! * **Whole-scene Fade is per-group alpha.** `applySceneNow`'s Fade branch
//!   keeps the outgoing groups as ordinary (non-ghost) layers and crossfades
//!   every group's alpha; the compositor's normal `group.alpha` composite is
//!   therefore the whole-scene fade. Shader/ISF transitions are WS-4 (the
//!   frame graph keeps the `SceneTransition` hook).
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
//! ### Audio mapping (WS-9)
//!
//! The renderer performs the analysis and commits the full `audio-bands.js`
//! payload through `meyda/SET_BANDS`; [`FrameInfo::audio`] carries it.
//! [`interim_band_payload`] converts the store's [`AudioFrame`] into the
//! `BandPayload` the Point10 bridge consumes — damping lanes, the 16-cell grid,
//! the AGC-free raw twins, wave, linear FFT magnitudes, beat envelope, sample
//! rate and the byte spectrum all come from the analyser, not a three-band
//! replication. ISF modules with `audio`/`audioFFT` inputs get a per-instance
//! 512×2 texture pair ([`IsfAudioTextures`]) uploaded every frame; 2D modules
//! receive the frame through `ModuleContext`/`DrawContext`.
//!
//! ## ISF corpus (WS-4)
//!
//! `meta.type == "isf"` modules are registered from the vendored corpus (see
//! [`crate::module_registry`]) and rendered through
//! [`modv_renderers::isf`]. The first time a module id draws:
//!
//! 1. The registry's parsed [`IsfDocument`] is compiled with
//!    [`IsfRuntime::compile`] for `Rgba8Unorm` (the group format). Compilation
//!    is per **instance**, while the translated pipeline is cached in the
//!    runtime's bounded LRU; the [`CompiledIsf`] owns the pass buffers and the
//!    uniform buffer.
//! 2. A module-sized target texture is allocated (rebuilt when the output
//!    resizes; the pass buffers themselves are resized inside
//!    [`IsfRuntime::draw`], so a resize never recompiles).
//! 3. Every frame `IsfRuntime::draw` renders the passes into that target with
//!    `TIME`/`TIMEDELTA`/`FRAMEINDEX`/`DATE` (wall clock, month 1-based,
//!    `DATE.w` = seconds since midnight) and the module's coerced prop values.
//! 4. The target is composited onto the group with the module's
//!    `meta.alpha`/`meta.compositeOperation`, the same helper the shader, 2D
//!    and Point10 paths use.
//!
//! Input images: every declared `image` input receives the **group canvas**
//! view, the JS default when no texture prop is resolved
//! (`renderers/isf/index.js:84-91`). There is no texture registry yet, so a
//! texture prop's value cannot be turned into a GPU texture; the group-canvas
//! default is the implemented behaviour for every image input.
//!
//! Audio inputs: documents that declare `audio`/`audioFFT` get a per-instance
//! 512×2 RGBA8 texture pair ([`IsfAudioTextures`]) uploaded from
//! [`FrameInfo::audio`] every frame — the time-domain wave and the byte
//! frequency spectrum, exactly like `ISFAudio.js`. Documents without audio
//! inputs never allocate the textures.
//!
//! Compile-on-activate cost: the first frame after a module is added runs the
//! GLSL → WGSL translation and pipeline creation on the render thread. The
//! corpus translates in a few milliseconds per shader on the reference
//! machine, but a heavy multi-pass shader can stall that frame; predictive
//! prewarming (like Point10's) is a follow-up.
//!
//! ## Gallery thumbnails (WS-14)
//!
//! [`Compositor::render_module_preview`] answers `preview/module`: it renders
//! one registered module into a `width × height` RGBA8 frame **without**
//! touching the live groups/modules. The isolated instance, its target and the
//! per-kind scratch state live in [`crate::module_preview`] (a small LRU keyed
//! by module name and size), and every kind takes the same path the live loop
//! takes: shader via a dedicated [`ShaderRenderer`] slot, 2D through a
//! [`Canvas`], Point10 through its scene + target, ISF through [`IsfRuntime`].
//! The finished layer is composited onto a transparent target (alpha 1,
//! `normal` — a fresh module's defaults) and read back tightly packed.
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
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
use modv_renderers::isf::{
    CompiledIsf, IsfDocument, IsfFrameUniforms, IsfRuntime, IsfValue, IsfValues,
};
use modv_renderers::point10::audio_bridge::{
    AudioBridge, BandPayload, RawBands, ResponsivenessLevers,
};
use modv_renderers::point10::{Point10Scene, SceneContext};
use modv_renderers::shader::ShaderModule;
use modv_renderers::shader::pipeline::{ShaderAudioUniforms, ShaderError, ShaderRenderer};
use modv_renderers::shader::uniforms::{PropKind, PropSlots, ShaderProp};
use modv_renderers::two_d::{AudioFrame, Canvas, DrawContext, ModuleContext, TwoDModule};

use crate::module_preview::{ModulePreviews, PreviewError};
use crate::module_registry::{ModuleKind, ModuleRegistry, Point10SceneKind};

/// Preview frame width published to the UI (`modv-ipc` `VMP1` ring).
pub const PREVIEW_WIDTH: u32 = 320;
/// Preview frame height published to the UI.
pub const PREVIEW_HEIGHT: u32 = 180;

/// The alpha below which a group (and a module) is culled (`loop.js:268`,
/// `:390`).
const ALPHA_CULL: f64 = 0.001;

/// One in-flight row crossfade, handed to the compositor for this frame
/// (`rowTransitions[row]`, `scenes.js:2502-2512`).
///
/// The runtime updates the Groups' alphas; the compositor additionally blends
/// the outgoing ghost and incoming Group textures so later rows inherit the
/// crossfade instead of just the incoming half (`loop.js:498-537`).
#[derive(Clone, Debug, PartialEq)]
pub struct RowFadeInfo {
    /// Row index.
    pub row: usize,
    /// The incoming Group id.
    pub group_id: String,
    /// The outgoing ghost Group id.
    pub ghost_group_id: String,
    /// The ghost's starting alpha.
    pub from_alpha: f64,
    /// The incoming Group's target alpha.
    pub target_alpha: f64,
    /// Transition progress in `[0, 1]`.
    pub progress: f64,
}

/// Per-frame row-fade bookkeeping.
///
/// `by_group` maps both halves' Group ids to their row so the alpha-cull
/// exemption and the main composite can find the fade; `sources` is the "two
/// ids" assignment (`loop.js:530-535`) — once both halves have rendered, both
/// ids resolve to the same blended texture for inheritance, so a later row
/// samples the crossfade rather than just the incoming canvas.
#[derive(Default)]
struct RowFadeState {
    by_row: HashMap<usize, RowFadeInfo>,
    by_group: HashMap<String, usize>,
    sources: HashMap<String, TextureId>,
    blended: HashSet<usize>,
    halves: HashMap<usize, (Option<TextureId>, Option<TextureId>)>,
}

impl RowFadeState {
    fn from_frame(frame: &FrameInfo) -> Self {
        let mut by_row = HashMap::new();
        let mut by_group = HashMap::new();
        for fade in &frame.row_fades {
            by_row.insert(fade.row, fade.clone());
            by_group.insert(fade.group_id.clone(), fade.row);
            by_group.insert(fade.ghost_group_id.clone(), fade.row);
        }
        Self {
            by_row,
            by_group,
            sources: HashMap::new(),
            blended: HashSet::new(),
            halves: HashMap::new(),
        }
    }
}

/// Everything a frame needs besides the state itself.
#[derive(Clone, Debug, Default)]
pub struct FrameInfo {
    /// Monotonic seconds since engine start (`iGlobalTime` family).
    pub time_seconds: f32,
    /// Frames presented since boot (`iFrame`).
    pub frame_index: u64,
    /// `[year, month (0-based), weekday (0 = Sunday), seconds]` — `iDate`.
    pub date: [f32; 4],
    /// The engine's full audio frame (bands, lanes/cells, wave, FFT
    /// magnitudes, sample rate and the Meyda scalars).
    ///
    /// Filled from [`Store::audio`](crate::store_ops::Store::audio), which the
    /// renderer's audio pump updates through `meyda/SET_BANDS`. Shader modules
    /// read the bands via [`FrameInfo::shader_audio`], 2D modules receive the
    /// whole frame in `ModuleContext`/`DrawContext`, ISF modules sample the
    /// per-instance audio textures and the Point10 bridge takes
    /// [`interim_band_payload`].
    pub audio: AudioFrame,
    /// Row crossfades running this frame (empty when performance is idle).
    pub row_fades: Vec<RowFadeInfo>,
}

impl FrameInfo {
    /// The shader uniform view of [`Self::audio`].
    ///
    /// `u_kick` is the analyser's kick envelope thresholded at 0.5 (the
    /// renderer forwards `features.p10.beat`; the shader uniform is a boolean
    /// in GLSL). `u_bass`/`u_mid`/`u_treble` are the AGC band features.
    pub fn shader_audio(&self) -> ShaderAudioUniforms {
        ShaderAudioUniforms {
            kick: self.audio.beat >= 0.5,
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
///
/// Shared with [`crate::module_preview`], which allocates its own targets at
/// the requested thumbnail size (the preview cache is independent of the
/// output size).
pub(crate) struct OutputTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
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

/// One ISF module's live GPU state.
///
/// The JS renderer keeps one `ISFRenderer` and one input list per module id
/// (`renderers/isf/index.js:283-291`); this struct is that state. The
/// [`CompiledIsf`] is built lazily on the module's first draw and then reused
/// (the runtime's bounded cache only holds the pipelines).
struct IsfInstance {
    /// The compiled shader, or `None` until the first draw.
    compiled: Option<CompiledIsf>,
    /// The module-sized render target, rebuilt when the output resizes.
    target: Option<OutputTexture>,
    /// Size the target was allocated for.
    size: Option<(u32, u32)>,
    /// `FrameInfo::time_seconds` of the previous frame, for `TIMEDELTA`.
    last_time: Option<f32>,
    /// The `audio`/`audioFFT` texture pair, allocated only when the compiled
    /// document declares audio inputs.
    audio: Option<IsfAudioTextures>,
}

/// The two ISF audio input textures (`ISFAudio.js`).
///
/// The JS `ISFAudioTexture` is `fftSize` (512) wide by 2 rows, RGBA with
/// `r = g = b = level` and `a = 255`. The analyser is mono here, so both rows
/// carry the same data (the JS duplicate of left/right also collapses to the
/// same values for a mono analysis). `audio` carries the time-domain wave,
/// `audioFFT` the byte frequency spectrum.
struct IsfAudioTextures {
    /// Backing texture for `audio` (kept for the view's lifetime).
    _wave: wgpu::Texture,
    /// `audio` input view.
    wave_view: wgpu::TextureView,
    /// Backing texture for `audioFFT`.
    _fft: wgpu::Texture,
    /// `audioFFT` input view.
    fft_view: wgpu::TextureView,
    /// Row-major pixel upload buffer (`width * height * 4`).
    pixels: Vec<u8>,
    /// One `audio` row, reused every frame.
    wave_row: Vec<u8>,
    /// One `audioFFT` row, reused every frame.
    fft_row: Vec<u8>,
}

impl IsfAudioTextures {
    /// ISF's audio textures are `fftSize` wide by two rows.
    const WIDTH: u32 = 512;
    const HEIGHT: u32 = 2;

    fn new(device: &wgpu::Device) -> Self {
        let create = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: Self::WIDTH,
                    height: Self::HEIGHT,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let wave = create("modv-isf-audio");
        let fft = create("modv-isf-audio-fft");
        Self {
            wave_view: wave.create_view(&wgpu::TextureViewDescriptor::default()),
            _wave: wave,
            fft_view: fft.create_view(&wgpu::TextureViewDescriptor::default()),
            _fft: fft,
            pixels: vec![0; (Self::WIDTH * Self::HEIGHT * 4) as usize],
            wave_row: vec![0; (Self::WIDTH * 4) as usize],
            fft_row: vec![0; (Self::WIDTH * 4) as usize],
        }
    }

    /// Upload one analyser frame into both textures.
    ///
    /// `audio` takes the last 512 wave samples (the renderer's
    /// `byteTimeDomainData` is `timeDomain.slice(-AUDIO_BUFFER_SIZE)`); the
    /// wave is `-1..1` and maps to `0..255`. `audioFFT` takes the first 512
    /// byte frequency bins. Both rows duplicate the mono data.
    fn upload(&mut self, queue: &wgpu::Queue, audio: &AudioFrame) {
        let row_bytes = (Self::WIDTH * 4) as usize;
        for (index, slot) in self.wave_row.iter_mut().enumerate() {
            // RGBA cycle: r, g, b = level, a = 255.
            if index % 4 == 3 {
                *slot = 255;
                continue;
            }
            let sample = 512 + index / 4;
            let value = audio.wave.get(sample).copied().unwrap_or(0.0);
            *slot = (((value.clamp(-1.0, 1.0) + 1.0) * 0.5) * 255.0).round() as u8;
        }
        for (index, slot) in self.fft_row.iter_mut().enumerate() {
            if index % 4 == 3 {
                *slot = 255;
                continue;
            }
            *slot = audio
                .byte_frequency_data
                .get(index / 4)
                .copied()
                .unwrap_or(0);
        }

        for row in 0..Self::HEIGHT as usize {
            self.pixels[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(&self.wave_row);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self._wave,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes as u32),
                rows_per_image: Some(Self::HEIGHT),
            },
            wgpu::Extent3d {
                width: Self::WIDTH,
                height: Self::HEIGHT,
                depth_or_array_layers: 1,
            },
        );

        for row in 0..Self::HEIGHT as usize {
            self.pixels[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(&self.fft_row);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self._fft,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes as u32),
                rows_per_image: Some(Self::HEIGHT),
            },
            wgpu::Extent3d {
                width: Self::WIDTH,
                height: Self::HEIGHT,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// The GPU compositor: shader renderer, frame-graph executor, textures.
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader: ShaderRenderer,
    executor: Executor,
    surfaces: TextureSurfaces,
    registry: Arc<ModuleRegistry>,
    isf_runtime: IsfRuntime,
    size: (u32, u32),
    main: Option<OutputTexture>,
    module_input: Option<OutputTexture>,
    module_output: Option<OutputTexture>,
    /// Ids of the groups that have a texture registered in [`TextureSurfaces`].
    group_targets: HashSet<String>,
    /// Row-fade blend targets keyed by row index (`loop.js:498-537`).
    row_fade_targets: HashMap<usize, OutputTexture>,
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
    /// Live ISF instances keyed by module id.
    isf: HashMap<String, IsfInstance>,
    /// ISF module ids whose compile failed; skipped without retrying each
    /// frame.
    failed_isf: HashSet<String>,
    /// One-shot log keys (unknown blend mode, skipped module type, ...).
    warned: HashSet<String>,
    /// Isolated gallery-thumbnail renderer (`preview/module`), created on the
    /// first request so headless/bench runs never allocate it.
    previews: Option<ModulePreviews>,
    /// The most recent frame's audio clock, reused by preview renders so a
    /// Gallery tile animates on the same clock (and with the same audio) as
    /// the live composition.
    preview_frame: FrameInfo,
    /// Sampler for module input textures — Repeat/Linear, matching
    /// `renderers/shader/index.js:271-280`.
    group_sampler: wgpu::Sampler,
    /// Sampler for the present pass — clamp/linear, a 1:1 blit.
    present_sampler: wgpu::Sampler,
    present_layout: wgpu::BindGroupLayout,
    present_pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Compositor {
    /// Build the compositor against the engine's device and queue with the
    /// built-in shader/2D/Point10 modules only (no ISF corpus).
    ///
    /// The engine path uses [`Compositor::with_registry`] so the compositor
    /// shares the store's registry and its scanned ISF definitions.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::with_registry(device, queue, Arc::new(ModuleRegistry::builtin()))
    }

    /// Build the compositor against a shared [`ModuleRegistry`].
    pub fn with_registry(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        registry: Arc<ModuleRegistry>,
    ) -> Self {
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
            registry,
            isf_runtime: IsfRuntime::new(device.clone(), queue.clone(), 4),
            size: (0, 0),
            main: None,
            module_input: None,
            module_output: None,
            group_targets: HashSet::new(),
            row_fade_targets: HashMap::new(),
            slots: HashMap::new(),
            failed_modules: HashSet::new(),
            two_d: HashMap::new(),
            failed_two_d: HashSet::new(),
            point10: HashMap::new(),
            failed_point10: HashSet::new(),
            isf: HashMap::new(),
            failed_isf: HashSet::new(),
            warned: HashSet::new(),
            previews: None,
            preview_frame: FrameInfo::default(),
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
        // ISF targets are output-sized too. The compiled shader owns the pass
        // buffers and resizes them itself in `draw`, so only the target is
        // dropped here.
        for instance in self.isf.values_mut() {
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
        self.isf.clear();
        self.failed_isf.clear();
        // Drop translated pipelines too: a restored state can reference
        // different shaders, and the cache is bounded but never expired.
        self.isf_runtime = IsfRuntime::new(self.device.clone(), self.queue.clone(), 4);
        // Preview instances hold pipelines (shader, ISF, Point10) and per-module
        // state tied to the restored state's module registry; release them.
        if let Some(previews) = &mut self.previews {
            previews.reset();
        }
    }

    fn drop_output_textures(&mut self) {
        self.main = None;
        self.module_input = None;
        self.module_output = None;
        self.surfaces.remove(&TextureId::from(MAIN_TEXTURE_ID));
        self.surfaces.remove(&module_output_id());
        for row in std::mem::take(&mut self.row_fade_targets).into_keys() {
            self.surfaces.remove(&row_fade_texture_id(row));
        }
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
        // Keep the last frame's clock/audio for `render_module_preview` (a
        // preview request is answered between frames, after this call).
        self.preview_frame = frame.clone();
        self.ensure_output_textures();
        self.prune(state);

        // Drop blend targets for rows that stopped fading.
        let live_rows: HashSet<usize> = frame.row_fades.iter().map(|fade| fade.row).collect();
        let stale_rows: Vec<usize> = self
            .row_fade_targets
            .keys()
            .copied()
            .filter(|row| !live_rows.contains(row))
            .collect();
        for row in stale_rows {
            self.row_fade_targets.remove(&row);
            self.surfaces.remove(&row_fade_texture_id(row));
        }

        // The frame graph clears whatever is registered as `main` at the start
        // of every `execute()`. Group passes must not pay that cost, so main is
        // unregistered while groups render and re-registered for the final
        // composite — the (only) texture the graph clears is then exactly the
        // one this frame presents.
        let main_texture = self.surfaces.remove(&main_id());
        let mut rendered: HashSet<String> = HashSet::new();
        let mut fades = RowFadeState::from_frame(frame);
        let groups = self.render_groups(state, frame, &mut rendered, &mut fades);
        if let Some(texture) = main_texture {
            self.surfaces.insert(main_id(), texture);
        }
        groups?;

        self.compose_main(state, &fades)?;
        Ok(())
    }

    /// Group loop (`worker/loop.js:239-558`).
    fn render_groups(
        &mut self,
        state: &EngineState,
        frame: &FrameInfo,
        rendered: &mut HashSet<String>,
        fades: &mut RowFadeState,
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

            // `loop.js:257-266`: a fading Group can dip below the alpha cull
            // at the start/end of its fade. Keep rendering both halves so the
            // inherited row canvas (and the blend) has both.
            let fade_row = fades.by_group.get(&group.id).copied();
            let is_transition_group = group.transition_ghost == Some(true) || fade_row.is_some();

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
                        // Both halves of a row crossfade resolve to the same
                        // blended texture; a ghost is never the inheritable
                        // Group itself (`loop.js:530-535`).
                        let source_id = fades
                            .sources
                            .get(&source.id)
                            .cloned()
                            .unwrap_or_else(|| self.group_target(&source.id));
                        self.composite(source_id, target_id.clone(), 1.0, RenderBlendMode::Normal)?;
                    }
                }
            }

            // `loop.js:381-491`: module chain.
            let module_ids = group.modules.clone();
            for module_id in module_ids {
                self.render_module(state, group, &module_id, &target_id, frame)?;
            }

            // `loop.js:498-537`: while this row crossfades, blend the outgoing
            // ghost and the incoming Group into the row's fade target. The
            // caller assigns the blend to *both* Group ids so a later row (or
            // an explicit inheritFrom target) samples the crossfade, not just
            // the incoming canvas.
            if let Some(row) = fade_row {
                let fade_target = self.row_fade_target(row);
                let entry = fades.halves.entry(row).or_default();
                if group.transition_ghost == Some(true) {
                    entry.0 = Some(target_id.clone());
                } else {
                    entry.1 = Some(target_id.clone());
                }
                if let (Some(from), Some(to)) = (entry.0.clone(), entry.1.clone()) {
                    if let Some(fade) = fades.by_row.get(&row).cloned() {
                        self.row_fade(
                            &from,
                            &to,
                            &fade_target,
                            fade.progress,
                            fade.from_alpha,
                            fade.target_alpha,
                        )?;
                        fades.blended.insert(row);
                        fades
                            .sources
                            .insert(fade.group_id.clone(), fade_target.clone());
                        fades
                            .sources
                            .insert(fade.ghost_group_id.clone(), fade_target);
                    }
                }
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
            // Vendored ISF shaders register with `meta.type: "isf"` and render
            // through the ISF runtime into their own target.
            "isf" => return self.render_isf_module(module, module_id, target, frame),
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
            // The shader uniform block carries only bands/kick; no shader
            // module in the registry declares `u_fft`, so the FFT texture slot
            // stays on the renderer's 1×1 zero dummy and `u_fftResolution`
            // must then be 1.0.
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

    /// Render one ISF shader into its module target and composite the result
    /// onto the group target (`renderers/isf/index.js:58-124`).
    ///
    /// The shader is compiled on the module's first draw and kept for the
    /// instance's lifetime (the runtime's bounded LRU only caches the
    /// pipelines). Frame uniforms follow `ISFRenderer.setDateUniforms`:
    /// `FRAMEINDEX` is 1-based, `DATE` is `[year, month 1-based, day, seconds
    /// since midnight]`. Audio inputs sample the per-instance
    /// [`IsfAudioTextures`] pair uploaded from the current [`FrameInfo`].
    fn render_isf_module(
        &mut self,
        module: &ModuleInstance,
        module_id: &str,
        target: &TextureId,
        frame: &FrameInfo,
    ) -> Result<(), CompositorError> {
        if self.failed_isf.contains(module_id) {
            return Ok(());
        }

        // Clone the `Arc` so the definition borrow does not conflict with the
        // mutable runtime/instance borrows below.
        let registry = Arc::clone(&self.registry);
        let Some(definition) = registry.isf_definition(&module.module_name) else {
            self.warn_once(
                format!("unregistered-isf:{}", module.module_name),
                &format!(
                    "engine: WARN ISF shader '{}' is not in the registry; skipping \
                     (was the corpus directory missing?)",
                    module.module_name
                ),
            );
            self.failed_isf.insert(module_id.to_string());
            return Ok(());
        };

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
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let texture_id = isf_texture_id(module_id);

        self.isf
            .entry(module_id.to_string())
            .or_insert(IsfInstance {
                compiled: None,
                target: None,
                size: None,
                last_time: None,
                audio: None,
            });

        // The module target is output-sized and rebuilt when the output
        // resizes. `IsfRuntime::draw` resizes the pass buffers itself.
        let needs_target = self
            .isf
            .get(module_id)
            .map(|instance| instance.target.is_none() || instance.size != Some((width, height)))
            .unwrap_or(false);
        let new_target = if needs_target {
            let texture =
                self.create_output_texture(&format!("modv-isf-{module_id}"), width, height);
            // The runtime blits the final pass with a clearing load, so this is
            // only a guard against undefined memory if the shader never draws.
            self.clear_texture(&texture.texture, "isf-allocate");
            Some(texture)
        } else {
            None
        };

        // Register whichever texture the shader draws into so the frame-graph
        // composite can sample it.
        let target_texture = match &new_target {
            Some(texture) => Some(texture.texture.clone()),
            None => self
                .isf
                .get(module_id)
                .and_then(|instance| instance.target.as_ref())
                .map(|texture| texture.texture.clone()),
        };
        let Some(target_texture) = target_texture else {
            return Ok(());
        };
        self.surfaces.insert(texture_id.clone(), target_texture);

        // Input images sample the group canvas — the JS default when no
        // texture prop resolves (`renderers/isf/index.js:84-91`). There is no
        // texture registry yet, so a texture prop cannot supply a GPU texture.
        let Some(group_texture) = self.surfaces.get(target).cloned() else {
            return Ok(());
        };
        let group_view = group_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let image_inputs: Vec<(&str, &wgpu::TextureView)> = definition
            .document
            .inputs
            .iter()
            .filter(|input| input.type_ == "image")
            .map(|input| (input.name.as_str(), &group_view))
            .collect();

        // Compile lazily. A compiled shader is tied to its target format; the
        // group format is fixed, so this only recompiles if that ever changes.
        let needs_compile = self
            .isf
            .get(module_id)
            .and_then(|instance| instance.compiled.as_ref())
            .map(|compiled| compiled.target_format != format)
            .unwrap_or(true);
        if needs_compile {
            match self
                .isf_runtime
                .compile(&definition.document, &definition.name, format)
            {
                Ok(compiled) => {
                    if let Some(instance) = self.isf.get_mut(module_id) {
                        instance.compiled = Some(compiled);
                    }
                }
                Err(error) => {
                    eprintln!(
                        "engine: WARN ISF shader '{}' failed to compile: {error}",
                        definition.name
                    );
                    self.failed_isf.insert(module_id.to_string());
                    self.isf.remove(module_id);
                    self.surfaces.remove(&texture_id);
                    return Ok(());
                }
            }
        }

        // Allocate the audio input pair only for documents that declare
        // `audio`/`audioFFT`, then upload the current analyser frame. The JS
        // renderer creates these textures in `ISFRenderer` per instance and
        // updates them every frame (`ISFAudio.js`).
        if definition.document.has_audio() {
            if self
                .isf
                .get(module_id)
                .and_then(|instance| instance.audio.as_ref())
                .is_none()
            {
                let audio = IsfAudioTextures::new(&self.device);
                if let Some(instance) = self.isf.get_mut(module_id) {
                    instance.audio = Some(audio);
                }
            }
            if let Some(instance) = self.isf.get_mut(module_id) {
                if let Some(audio_textures) = instance.audio.as_mut() {
                    audio_textures.upload(&self.queue, &frame.audio);
                }
            }
        }

        // Values present in the module's prop bag; the runtime fills anything
        // missing from the document defaults.
        let values = isf_values(&definition.document, &module.props);
        let date = isf_wall_clock_date();

        let draw_error = {
            // Split the borrows: the instance lives in `isf`, the compiler and
            // pipeline cache in `isf_runtime`.
            let Self {
                isf, isf_runtime, ..
            } = self;
            let Some(instance) = isf.get_mut(module_id) else {
                return Ok(());
            };
            if let Some(texture) = new_target {
                instance.target = Some(texture);
                instance.size = Some((width, height));
            }

            // `timing.js`-style per-instance delta: the first frame is 0.
            let time_delta = match instance.last_time {
                Some(previous) if (frame.time_seconds - previous).is_finite() => {
                    (frame.time_seconds - previous).max(0.0)
                }
                _ => 0.0,
            };
            instance.last_time = Some(frame.time_seconds);

            let uniforms = IsfFrameUniforms {
                time: frame.time_seconds,
                time_delta,
                // `frameIndex += 1` starts at 1 in the JS renderer.
                frame_index: frame.frame_index as i32 + 1,
                date,
            };

            let Some(compiled) = instance.compiled.as_mut() else {
                return Ok(());
            };
            let Some(render_target) = instance.target.as_ref() else {
                return Ok(());
            };
            let audio_inputs: Vec<(&str, &wgpu::TextureView)> = instance
                .audio
                .as_ref()
                .map(|audio| {
                    definition
                        .document
                        .inputs
                        .iter()
                        .filter(|input| input.type_ == "audio" || input.type_ == "audioFFT")
                        .map(|input| {
                            let view = if input.type_ == "audioFFT" {
                                &audio.fft_view
                            } else {
                                &audio.wave_view
                            };
                            (input.name.as_str(), view)
                        })
                        .collect()
                })
                .unwrap_or_default();
            isf_runtime
                .draw(
                    compiled,
                    &render_target.view,
                    [width, height],
                    uniforms,
                    &values,
                    &image_inputs,
                    &audio_inputs,
                )
                .err()
        };

        // A shader that fails at draw time (an invalid WIDTH formula, a GPU
        // validation error) must not take the render loop down with it: skip
        // the module for the rest of its life, like a compile failure.
        if let Some(error) = draw_error {
            eprintln!(
                "engine: WARN ISF shader '{}' failed to draw: {error}",
                definition.name
            );
            self.failed_isf.insert(module_id.to_string());
            self.isf.remove(module_id);
            self.surfaces.remove(&texture_id);
            return Ok(());
        }

        self.composite(texture_id, target.clone(), alpha, blend)
    }

    /// Main composite (`loop.js:555-663`).
    fn compose_main(
        &mut self,
        state: &EngineState,
        fades: &RowFadeState,
    ) -> Result<(), CompositorError> {
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
            // A row-transition ghost only exists for the crossfade; the blend
            // carries its outgoing contribution to main (`loop.js:498-537`).
            if group.transition_ghost == Some(true) {
                continue;
            }
            if group.enabled != GROUP_ENABLED || group.modules.is_empty() {
                continue;
            }

            // Once both halves rendered, the blended texture stands in for
            // the incoming Group: it already has the eased alphas baked in.
            let blended_row = fades
                .by_group
                .get(&group.id)
                .copied()
                .filter(|row| fades.blended.contains(row));

            let (source, alpha, blend) = match blended_row {
                Some(row) => (
                    row_fade_texture_id(row),
                    1.0,
                    self.render_blend_mode(&group.composite_operation),
                ),
                None => {
                    if !(group.alpha > 0.0) {
                        continue;
                    }
                    (
                        self.group_target(&group.id),
                        group.alpha,
                        self.render_blend_mode(&group.composite_operation),
                    )
                }
            };

            graph.push(PassNode::Composite {
                source,
                target: main_id(),
                alpha,
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

    /// A row's crossfade blend target, allocated on demand and registered for
    /// the graph (`loop.js:498-537`; one per fading row).
    fn row_fade_target(&mut self, row: usize) -> TextureId {
        let id = row_fade_texture_id(row);
        if self.row_fade_targets.contains_key(&row) {
            return id;
        }
        let (width, height) = self.size;
        let texture = self.create_output_texture(&format!("modv-row-fade-{row}"), width, height);
        self.surfaces.insert(id.clone(), texture.texture.clone());
        self.row_fade_targets.insert(row, texture);
        id
    }

    fn create_output_texture(&self, label: &str, width: u32, height: u32) -> OutputTexture {
        create_output_texture(&self.device, label, width, height)
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

        let stale_isf: Vec<String> = self
            .isf
            .keys()
            .filter(|id| !live_modules.contains(id.as_str()))
            .cloned()
            .collect();
        for id in stale_isf {
            self.isf.remove(&id);
            self.surfaces.remove(&isf_texture_id(&id));
        }
        self.failed_isf
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
        clear_texture(&self.device, &self.queue, texture, label);
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

    /// Run one `RowFade` node through the frame graph executor
    /// (`renderRowFade`, `worker/row-transition.js:71-115`). The blended
    /// result lands in `target`, which the caller assigns to both Group ids.
    fn row_fade(
        &mut self,
        from: &TextureId,
        to: &TextureId,
        target: &TextureId,
        progress: f64,
        from_alpha: f64,
        to_alpha: f64,
    ) -> Result<(), CompositorError> {
        let mut graph = FrameGraph::new();
        graph.push(PassNode::RowFade {
            from: from.clone(),
            to: to.clone(),
            target: target.clone(),
            progress,
            from_alpha,
            to_alpha,
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

    /// Render one Gallery thumbnail from an **isolated** module instance.
    ///
    /// This is the engine side of the renderer's
    /// `window.engine.requestModulePreview({ moduleName, width, height })`
    /// (WS-14): the live groups and modules are never touched, so hovering a
    /// tile cannot disturb the running composition. The request's size is
    /// clamped to `1..=512` on each edge, a frame is rendered exactly like the
    /// live loop renders that module kind (see [`crate::module_preview`]), and
    /// the result is returned as **tightly packed** `width × height` RGBA8.
    ///
    /// The frame clock and audio come from the last [`Self::render_frame`] call
    /// (the store's `AudioFrame`), so a tile animates and reacts like the live
    /// composition. Rendering before any frame presents uses a silent frame.
    pub fn render_module_preview(
        &mut self,
        registry: &ModuleRegistry,
        module_name: &str,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, PreviewError> {
        let frame = self.preview_frame.clone();
        let device = self.device.clone();
        let queue = self.queue.clone();
        let previews = self
            .previews
            .get_or_insert_with(|| ModulePreviews::new(&device, &queue));
        previews.render(registry, module_name, width, height, &frame)
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

/// Allocate an `Rgba8Unorm` texture usable as a render target, a sampled
/// source and a read-back copy source — the format every module target uses.
///
/// Shared with [`crate::module_preview`], which sizes its own targets to the
/// requested thumbnail.
pub(crate) fn create_output_texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
) -> OutputTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
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

/// Clear `texture` to transparent black — the canvas `clearRect` state
/// (`loop.js:319-321`).
///
/// wgpu does not zero-initialise allocations, so every newly allocated target
/// is cleared before it can be sampled or composited.
pub(crate) fn clear_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    label: &str,
) {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
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
    queue.submit(Some(encoder.finish()));
}

/// Read a texture back as tightly packed RGBA8, padding rows to
/// `COPY_BYTES_PER_ROW_ALIGNMENT` on the GPU and de-padding here.
///
/// Ported locally rather than reusing
/// [`modv_render::headless::read_back_rgba8`]: that helper takes `&Engine`
/// (instance + adapter + device + queue), while the compositor deliberately
/// owns only cloned device/queue handles. The copy/map/poll dance is identical.
pub(crate) fn read_back_texture(
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
pub(crate) fn encode_props(definition: &ShaderModule, props: &Value) -> PropSlots {
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

/// The frame-graph texture id of a row's crossfade blend target.
fn row_fade_texture_id(row: usize) -> TextureId {
    TextureId::from(format!("row-fade:{row}"))
}

/// The frame-graph texture id of a 2D module's canvas.
fn two_d_texture_id(module_id: &str) -> TextureId {
    TextureId::from(format!("two-d:{module_id}"))
}

/// The frame-graph texture id of a Point10 scene's target.
fn point10_texture_id(module_id: &str) -> TextureId {
    TextureId::from(format!("point10:{module_id}"))
}

/// The frame-graph texture id of an ISF module's target.
fn isf_texture_id(module_id: &str) -> TextureId {
    TextureId::from(format!("isf:{module_id}"))
}

/// Build the per-frame [`IsfValues`] from a module's prop bag.
///
/// Only inputs with a usable value are inserted; anything missing (or of the
/// wrong shape) is left out so the runtime falls back to the document
/// default. Image/audio inputs are textures and are not uniform values.
pub(crate) fn isf_values(document: &IsfDocument, props: &Value) -> IsfValues {
    let mut values = IsfValues::new();
    for input in &document.inputs {
        let value = props.get(&input.name);
        match input.type_.as_str() {
            "float" => {
                if let Some(number) = value
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite())
                {
                    values.insert(input.name.clone(), IsfValue::Float(number));
                }
            }
            "long" => {
                if let Some(value) = value {
                    values.insert(input.name.clone(), IsfValue::Int(isf_int(value)));
                }
            }
            "bool" | "event" => {
                // Events have no trigger plumbing yet; a stored truthy value
                // still maps to `true` (JS: `!!props[NAME]`).
                values.insert(
                    input.name.clone(),
                    IsfValue::Bool(value.and_then(Value::as_bool).unwrap_or(false)),
                );
            }
            "point2D" => {
                if let Some([x, y]) = json_vec2(value) {
                    values.insert(input.name.clone(), IsfValue::Vec2([x, y]));
                }
            }
            "color" => {
                if let Some(colour) = json_vec4(value) {
                    values.insert(input.name.clone(), IsfValue::Vec4(colour));
                }
            }
            // `image`/`audio`/`audioFFT` are texture inputs, not uniform members.
            _ => {}
        }
    }
    values
}

/// Read a `long` value as `i32`, tolerating numbers, numeric strings and bools
/// (presets and input links can carry any of them).
fn isf_int(value: &Value) -> i32 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(0.0) as i32,
        Value::String(text) => text.trim().parse().unwrap_or(0),
        Value::Bool(value) => i32::from(*value),
        _ => 0,
    }
}

/// Read a `point2D` prop (`[x, y]`) into two floats.
fn json_vec2(value: Option<&Value>) -> Option<[f32; 2]> {
    let entries = value?.as_array()?;
    Some([
        entries.first()?.as_f64()? as f32,
        entries.get(1)?.as_f64()? as f32,
    ])
}

/// Read a `color` prop (`[r, g, b, a]`) into four floats.
fn json_vec4(value: Option<&Value>) -> Option<[f32; 4]> {
    let entries = value?.as_array()?;
    let mut colour = [0.0f32; 4];
    for (index, slot) in colour.iter_mut().enumerate() {
        *slot = entries.get(index)?.as_f64()? as f32;
    }
    Some(colour)
}

/// `DATE` for [`IsfFrameUniforms`]: `[year, month 1-based, day, seconds since
/// midnight]`, read from the wall clock (the JS renderer's `setDateUniforms`).
/// The clock is UTC so the date and the seconds-since-midnight share a zone
/// (the engine has no timezone database); a visual shader only uses this for
/// slow day/night variation.
pub(crate) fn isf_wall_clock_date() -> [f32; 4] {
    let unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = unix_seconds.div_euclid(86_400);
    let seconds = unix_seconds.rem_euclid(86_400) as f32;
    let (year, month, day) = civil_from_days(days);
    [year as f32, month as f32, day as f32, seconds]
}

/// Gregorian date from days since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

/// Longest frame the Point10 scene clock reports, mirroring
/// `MAX_FRAME_SECONDS` in `point10/timing.js`.
pub(crate) const MAX_POINT10_FRAME_SECONDS: f32 = 0.1;

/// Build the `audio-bands.js` payload the Point10 [`AudioBridge`] consumes from
/// the engine's [`AudioFrame`].
///
/// WS-9 makes this a faithful conversion: the renderer's analyser already
/// produced the damping lanes, the 16-cell grid, the wave, the linear FFT
/// magnitudes and the AGC-free twins, and `meyda/SET_BANDS` stores them on the
/// frame. Only silent frames (before the first analyser result) carry zeros,
/// which is exactly what the legacy bridge saw before its first `update`.
///
/// `wave`/`freq_mag` are the renderer's 1024-sample arrays; the byte spectrum
/// is the frame's 1024 bins (the analyser sends 512 and the store pads).
pub(crate) fn interim_band_payload(audio: &AudioFrame) -> BandPayload {
    BandPayload {
        sample_rate: audio.sample_rate,
        lanes: audio.lanes,
        cells: audio.cells,
        beat: audio.beat,
        wave: audio.wave,
        freq_mag: audio.freq_mag,
        raw: RawBands {
            lanes: audio.raw_lanes,
            cells: audio.raw_cells,
        },
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
            row_fades: Vec::new(),
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
    fn row_crossfade_blends_both_halves_and_drops_the_ghost_after() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        compositor.resize(SIZE, SIZE);

        let ball_props = json!({
            "amount": 12,
            "speed": 2.0,
            "wrap": false,
            "size": 4,
            "intensity": 15,
            "soundType": false,
            "color": { "value": [255.0, 255.0, 255.0] },
        });

        let mut state = EngineState::new();
        // The incoming row is still at alpha 0 (the runtime exempts it from
        // the alpha cull via the active row fade) and draws nothing.
        let mut incoming = group_with(
            "incoming",
            "Effects",
            &["module-to"],
            GROUP_ENABLED,
            0.0,
            true,
            true,
            json!(-1),
        );
        incoming.scene_row = Some(0);
        // The outgoing ghost keeps the old visuals alive at its fade alpha.
        let mut ghost = group_with(
            "ghost",
            "Effects",
            &["module-ghost"],
            GROUP_ENABLED,
            0.5,
            true,
            true,
            json!(-1),
        );
        ghost.scene_row = Some(0);
        ghost.transition_ghost = Some(true);
        // A later row inherits the row's canvas: it must see the *blend*, not
        // the transparent incoming half.
        let mut follower = group_with(
            "follower",
            "Foreground",
            &["module-follow"],
            GROUP_ENABLED,
            1.0,
            true,
            true,
            json!(-1),
        );
        follower.scene_row = Some(1);
        state.groups.push(incoming);
        state.groups.push(ghost);
        state.groups.push(follower);
        state.modules.insert(
            "module-ghost".to_string(),
            two_d_module("module-ghost", "Ball", ball_props),
        );
        state.modules.insert(
            "module-to".to_string(),
            two_d_module("module-to", "Texture 2D", json!({})),
        );
        state.modules.insert(
            "module-follow".to_string(),
            two_d_module("module-follow", "Texture 2D", json!({})),
        );

        let mut frame = frame_info();
        frame.row_fades.push(RowFadeInfo {
            row: 0,
            group_id: "incoming".to_string(),
            ghost_group_id: "ghost".to_string(),
            from_alpha: 1.0,
            target_alpha: 1.0,
            progress: 0.5,
        });

        compositor
            .render_frame(&state, &frame)
            .expect("mid-fade frame renders");

        let main = compositor.read_main_rgba8().expect("main reads back");
        assert!(
            lit_pixels(&main) > 0,
            "the outgoing half must be visible mid-fade"
        );
        let follower_pixels = read_group_texture(&compositor, "follower");
        assert!(
            lit_pixels(&follower_pixels) > 0,
            "the next row inherits the blended crossfade"
        );
        assert_eq!(
            lit_pixels(&follower_pixels),
            lit_pixels(&main),
            "both Group ids resolve to the same blend"
        );

        // Fade finished: the ghost is gone and the incoming row is empty.
        state.groups.retain(|group| group.id != "ghost");
        compositor
            .render_frame(&state, &frame_info())
            .expect("post-fade frame renders");
        let main = compositor.read_main_rgba8().expect("main reads back");
        assert_eq!(
            lit_pixels(&main),
            0,
            "the outgoing contribution disappears with the ghost"
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
        // This payload carries bands only; wave/FFT stay at their default
        // (zeroed) values until the full analyser payload arrives.
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
        assert!(!uniforms.kick, "no beat in this payload");
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
    fn point10_chromatic_wave_reacts_to_the_audio_frame() {
        let Some(engine) = try_engine() else {
            return;
        };

        let render_with = |engine: &Engine, audio: AudioFrame| {
            let mut compositor = Compositor::new(&engine.device, &engine.queue);
            compositor.resize(POINT10_SIZE, POINT10_SIZE);
            let state = point10_state("Point10 Chromatic Wave", json!({}));
            for (index, time) in [(0_u64, 0.0_f32), (1, 1.0 / 60.0), (2, 2.0 / 60.0)] {
                let frame = FrameInfo {
                    time_seconds: time,
                    frame_index: index,
                    audio: audio.clone(),
                    ..frame_info()
                };
                compositor
                    .render_frame(&state, &frame)
                    .expect("Chromatic Wave frame renders");
            }
            compositor.read_main_rgba8().expect("main reads back")
        };

        let silent = render_with(&engine, AudioFrame::default());

        let mut loud = AudioFrame {
            bass: 1.0,
            mid: 1.0,
            treble: 1.0,
            beat: 1.0,
            ..AudioFrame::default()
        };
        loud.cells = [1.0; 16];
        loud.lanes = [1.0; 12];
        for (index, sample) in loud.wave.iter_mut().enumerate() {
            *sample = (index as f32 / 1024.0 * std::f32::consts::TAU * 8.0).sin() * 0.9;
        }
        for (index, bin) in loud.byte_frequency_data.iter_mut().enumerate() {
            *bin = (index.min(255)) as u8;
        }
        let loud = render_with(&engine, loud);

        assert_ne!(
            silent, loud,
            "the scene must react to the analyser frame (wave/bands), not render silence"
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

    // ————————————————— Gallery thumbnails (WS-14) —————————————————

    /// Render twice at the same size, advancing the isolated clock by one
    /// 60 Hz frame, so scene state (dt-driven) has one non-zero step.
    fn render_preview_twice(
        compositor: &mut Compositor,
        registry: &ModuleRegistry,
        name: &str,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let mut pixels = Vec::new();
        for (index, time) in [(0_u64, 0.0_f32), (1, 1.0 / 60.0)] {
            compositor.preview_frame = FrameInfo {
                time_seconds: time,
                frame_index: index,
                ..frame_info()
            };
            pixels = compositor
                .render_module_preview(registry, name, width, height)
                .unwrap_or_else(|error| panic!("{name} preview renders: {error}"));
            assert_eq!(pixels.len(), (width * height * 4) as usize);
        }
        pixels
    }

    #[test]
    fn module_preview_renders_a_shader_over_a_transparent_input() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = ModuleRegistry::builtin();
        let mut compositor = Compositor::new(&engine.device, &engine.queue);

        let pixels = compositor
            .render_module_preview(&registry, "Point10 Invert", 48, 32)
            .expect("shader preview renders");
        assert_eq!(pixels.len(), (48 * 32 * 4) as usize);
        // The shader pass clears its target to opaque black; Invert turns the
        // transparent input into opaque white (`c = 1 - canvas`, and the
        // mainImage wrapper forces alpha 1).
        for (index, chunk) in pixels.chunks_exact(4).enumerate() {
            assert!(
                chunk[0] >= 250 && chunk[1] >= 250 && chunk[2] >= 250 && chunk[3] >= 250,
                "pixel {index} is {chunk:?}, expected opaque white"
            );
        }

        // The isolated renderer never touches the live module state.
        assert!(compositor.slots.is_empty());
        assert!(compositor.two_d.is_empty());
        assert!(compositor.point10.is_empty());

        // An unregistered module is reported as `module_not_found`, not a
        // generic render failure (the renderer backs off on that code).
        assert!(matches!(
            compositor.render_module_preview(&registry, "definitely not registered", 16, 16),
            Err(PreviewError::ModuleNotFound(_))
        ));
    }

    #[test]
    fn module_preview_renders_a_two_d_module() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = ModuleRegistry::builtin();
        let mut compositor = Compositor::new(&engine.device, &engine.queue);

        let pixels = render_preview_twice(&mut compositor, &registry, "Ball", 32, 32);
        assert_eq!(pixels.len(), (32 * 32 * 4) as usize);
        assert!(
            pixels.iter().any(|byte| *byte != 0),
            "Ball drew nothing into the thumbnail"
        );
        // The second request reused the cached instance instead of building
        // another one.
        let previews = compositor.previews.as_ref().expect("previews exist");
        assert_eq!(previews.cached_slots(), 1);
    }

    #[test]
    fn module_preview_renders_a_point10_scene() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = ModuleRegistry::builtin();
        let mut compositor = Compositor::new(&engine.device, &engine.queue);

        let pixels = render_preview_twice(
            &mut compositor,
            &registry,
            "Point10 Chromatic Line",
            128,
            128,
        );
        assert!(
            lit_pixels(&pixels) > 0,
            "Point10 Chromatic Line drew nothing into the thumbnail"
        );
    }

    #[test]
    fn module_preview_renders_an_isf_module() {
        let Some(engine) = try_engine() else {
            return;
        };
        let Some(dir) = corpus_dir() else {
            return;
        };
        let registry = ModuleRegistry::build(Some(&dir));
        if registry.isf_definition("Color Invert").is_none() {
            eprintln!("skipping ISF preview test: Color Invert is not in the corpus");
            return;
        }

        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        let pixels = compositor
            .render_module_preview(&registry, "Color Invert", 32, 32)
            .expect("ISF preview renders");
        // A filter over a transparent 1×1 input may be fully transparent, so
        // the frame *shape* is the contract here: exactly one tightly packed
        // RGBA8 frame at the requested size.
        assert_eq!(pixels.len(), (32 * 32 * 4) as usize);
    }

    #[test]
    fn module_preview_clamps_requested_sizes() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = ModuleRegistry::builtin();
        let mut compositor = Compositor::new(&engine.device, &engine.queue);

        // Zero collapses to the smallest legal target.
        let tiny = compositor
            .render_module_preview(&registry, "Point10 Invert", 0, 0)
            .expect("clamped preview renders");
        assert_eq!(tiny.len(), 4, "0×0 clamps to 1×1");

        // 513 is clamped to the 512 cap on each edge (1 MiB of RGBA8).
        let big = compositor
            .render_module_preview(&registry, "Point10 Invert", 513, 513)
            .expect("oversized preview renders");
        assert_eq!(big.len(), (512 * 512 * 4) as usize);
    }

    #[test]
    fn compositor_reset_drops_the_preview_cache() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = ModuleRegistry::builtin();
        let mut compositor = Compositor::new(&engine.device, &engine.queue);

        compositor
            .render_module_preview(&registry, "Ball", 16, 16)
            .expect("preview renders");
        let previews = compositor.previews.as_ref().expect("previews exist");
        assert_eq!(previews.cached_slots(), 1);

        compositor.reset();
        let previews = compositor
            .previews
            .as_ref()
            .expect("previews still allocated");
        assert_eq!(
            previews.cached_slots(),
            0,
            "reset disposes cached preview instances"
        );
    }

    // ————————————————— ISF corpus path (WS-4) —————————————————

    /// The vendored corpus directory, or `None` when this checkout does not
    /// have it (the test then skips like a missing GPU adapter).
    fn corpus_dir() -> Option<std::path::PathBuf> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/renderer/src/application/renderers/isf/modules/isf");
        if dir.is_dir() {
            Some(dir)
        } else {
            eprintln!("skipping ISF compositor test: {} is missing", dir.display());
            None
        }
    }

    /// A registry with the corpus registered, or `None` on a missing corpus.
    fn corpus_registry() -> Option<Arc<ModuleRegistry>> {
        Some(Arc::new(ModuleRegistry::build(Some(&corpus_dir()?))))
    }

    /// An ISF module record (`meta.type == "isf"`).
    fn isf_module(module_id: &str, name: &str, props: Value) -> ModuleInstance {
        serde_json::from_value(json!({
            "$id": module_id,
            "$moduleName": name,
            "meta": {
                "name": name,
                "type": "isf",
                "enabled": true,
                "alpha": 1.0,
                "compositeOperation": "normal"
            },
            "props": props,
            "data": {}
        }))
        .expect("ISF module decodes")
    }

    /// One clearing `Effects` group with the given ISF modules, in draw order.
    fn isf_state(modules: &[(&str, &str, Value)]) -> EngineState {
        let ids: Vec<&str> = modules.iter().map(|(id, _, _)| *id).collect();
        let mut state = EngineState::new();
        state.groups.push(group_with(
            "group-a",
            "Effects",
            &ids,
            GROUP_ENABLED,
            1.0,
            true,
            true,
            json!(-1),
        ));
        for (id, name, props) in modules {
            state
                .modules
                .insert(id.to_string(), isf_module(id, name, props.clone()));
        }
        state
    }

    fn isf_compositor(engine: &Engine, registry: Arc<ModuleRegistry>) -> Compositor {
        let mut compositor = Compositor::with_registry(&engine.device, &engine.queue, registry);
        compositor.resize(SIZE, SIZE);
        compositor
    }

    #[test]
    fn isf_generator_renders_through_the_runtime() {
        let Some(engine) = try_engine() else {
            return;
        };
        let Some(registry) = corpus_registry() else {
            return;
        };
        let mut compositor = isf_compositor(&engine, registry);

        let state = isf_state(&[("module-a", "Checkerboard", json!({}))]);
        compositor
            .render_frame(&state, &frame_info())
            .expect("frame renders");

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        assert!(
            lit_pixels(&pixels) > 0,
            "Checkerboard (a pure ISF generator) drew nothing"
        );
        assert!(compositor.isf.contains_key("module-a"));
        assert!(compositor.isf["module-a"].compiled.is_some());
        assert!(
            compositor.isf["module-a"].audio.is_none(),
            "Checkerboard declares no audio inputs, so no audio textures are allocated"
        );
    }

    #[test]
    fn isf_audio_input_samples_the_live_fft_texture() {
        let Some(engine) = try_engine() else {
            return;
        };

        let dir = std::env::temp_dir().join(format!(
            "modv-app-isf-audio-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("Audio Probe.fs"),
            r#"/*{
                "CREDIT": "by test",
                "VSN": "1.0.0",
                "ISFVSN": "2",
                "INPUTS": [
                    { "NAME": "audioFFT", "TYPE": "audioFFT" }
                ]
            }*/
            void main() {
                gl_FragColor = IMG_NORM_PIXEL(audioFFT, vec2(0.01, 0.5));
            }
            "#,
        )
        .expect("shader written");

        let registry = Arc::new(ModuleRegistry::build(Some(&dir)));
        let mut compositor = isf_compositor(&engine, registry);
        let state = isf_state(&[("module-a", "Audio Probe", json!({}))]);

        let mut frame = frame_info();
        frame.audio.byte_frequency_data[5] = 0;
        compositor
            .render_frame(&state, &frame)
            .expect("dark frame renders");
        assert!(
            compositor.isf["module-a"].audio.is_some(),
            "an audio input allocates the 512x2 texture pair"
        );
        let dark = compositor.read_main_rgba8().expect("main reads back");

        frame.audio.byte_frequency_data[5] = 255;
        compositor
            .render_frame(&state, &frame)
            .expect("bright frame renders");
        let bright = compositor.read_main_rgba8().expect("main reads back");

        assert_ne!(
            dark, bright,
            "changing the FFT bin the shader samples must change the output"
        );
        let dark_luma: u64 = dark.chunks_exact(4).map(|p| u64::from(p[0])).sum();
        let bright_luma: u64 = bright.chunks_exact(4).map(|p| u64::from(p[0])).sum();
        assert!(
            bright_luma > dark_luma,
            "the brighter FFT bin must raise the sampled channel"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn isf_image_input_samples_the_group_canvas() {
        let Some(engine) = try_engine() else {
            return;
        };
        let Some(registry) = corpus_registry() else {
            return;
        };
        let mut compositor = isf_compositor(&engine, registry);

        // Checkerboard draws into the group; Color Invert then samples the
        // group canvas through its `inputImage` and inverts it — the JS
        // default binding for an image input (`renderers/isf/index.js:84-91`).
        let mut state = isf_state(&[("module-a", "Checkerboard", json!({}))]);
        compositor
            .render_frame(&state, &frame_info())
            .expect("checkerboard frame renders");
        let checkerboard = compositor.read_main_rgba8().expect("main reads back");

        state.modules.insert(
            "module-b".to_string(),
            isf_module("module-b", "Color Invert", json!({})),
        );
        state.groups[0].modules.push("module-b".to_string());
        compositor
            .render_frame(&state, &frame_info())
            .expect("invert frame renders");
        let inverted = compositor.read_main_rgba8().expect("main reads back");

        assert!(lit_pixels(&inverted) > 0, "the inverted frame is empty");
        assert_ne!(
            checkerboard, inverted,
            "Color Invert did not sample the group canvas"
        );
    }

    #[test]
    fn isf_multi_pass_filter_renders_through_the_runtime() {
        let Some(engine) = try_engine() else {
            return;
        };
        let Some(registry) = corpus_registry() else {
            return;
        };
        let mut compositor = isf_compositor(&engine, registry);

        // Bloom is a 7-pass filter: every pass renders into its own buffer and
        // the last one blits into the module target.
        let state = isf_state(&[
            ("module-a", "Checkerboard", json!({})),
            (
                "module-b",
                "Bloom",
                json!({"blurAmount": 6.0, "intensity": 1.0}),
            ),
        ]);
        compositor
            .render_frame(&state, &frame_info())
            .expect("frame renders");

        let pixels = compositor.read_main_rgba8().expect("main reads back");
        assert!(lit_pixels(&pixels) > 0, "Bloom produced an empty frame");
        let bloom = compositor.isf.get("module-b").expect("Bloom instance");
        let compiled = bloom.compiled.as_ref().expect("Bloom compiled");
        assert!(
            compiled.passes.len() >= 7,
            "Bloom should keep its multi-pass graph"
        );
    }

    #[test]
    fn interim_audio_payload_forwards_the_real_analyser_grid() {
        let mut audio = AudioFrame {
            bass: 0.5,
            mid: 0.25,
            treble: 0.75,
            beat: 0.4,
            sample_rate: 48_000.0,
            ..AudioFrame::default()
        };
        // Distinct values prove the payload is a conversion, not a band
        // replication: every lane/cell keeps its own analyser value.
        for (index, lane) in audio.lanes.iter_mut().enumerate() {
            *lane = index as f32 / 100.0;
        }
        for (index, cell) in audio.cells.iter_mut().enumerate() {
            *cell = 0.5 + index as f32 / 100.0;
        }
        for (index, lane) in audio.raw_lanes.iter_mut().enumerate() {
            *lane = 1.0 + index as f32 / 100.0;
        }
        for (index, cell) in audio.raw_cells.iter_mut().enumerate() {
            *cell = 2.0 + index as f32 / 100.0;
        }
        for (index, bin) in audio.freq_mag.iter_mut().enumerate() {
            *bin = index as f32;
        }
        audio.wave[0] = -0.5;
        audio.wave[1023] = 0.5;
        audio.byte_frequency_data[0] = 200;

        let payload = interim_band_payload(&audio);

        assert_eq!(payload.sample_rate, 48_000.0);
        assert_eq!(payload.lanes[0], 0.0);
        assert_eq!(payload.lanes[11], 0.11);
        assert_eq!(payload.cells[0], 0.5);
        assert_eq!(payload.cells[15], 0.65);
        assert_eq!(payload.raw.lanes[0], 1.0);
        assert_eq!(payload.raw.cells[15], 2.15);
        assert_eq!(payload.beat, 0.4);
        assert_eq!(payload.wave[0], -0.5);
        assert_eq!(payload.wave[1023], 0.5);
        assert_eq!(payload.freq_mag[0], 0.0);
        assert_eq!(payload.freq_mag[1023], 1023.0);
        assert_eq!(payload.byte_frequency_data[0], 200);
        assert!(
            payload.byte_frequency_data[1023..]
                .iter()
                .all(|byte| *byte == 0)
        );
    }

    #[test]
    fn silent_audio_payload_is_all_zeroes() {
        let payload = interim_band_payload(&AudioFrame::default());
        assert!(payload.lanes.iter().all(|value| *value == 0.0));
        assert!(payload.cells.iter().all(|value| *value == 0.0));
        assert!(payload.wave.iter().all(|value| *value == 0.0));
        assert!(payload.freq_mag.iter().all(|value| *value == 0.0));
        assert_eq!(payload.sample_rate, 44_100.0);
    }
}
