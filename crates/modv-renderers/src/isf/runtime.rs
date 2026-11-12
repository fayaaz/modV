//! Multi-pass ISF execution on wgpu.
//!
//! Port of `ISFRenderer` (`interactive-shader-format/src/ISFRenderer.js`) and
//! `ISFBuffer` (`ISFBuffer.js`): per-pass render targets, the ping-pong
//! persistent-buffer pair, per-pass `RENDERSIZE`/`PASSINDEX`, the implicit
//! final pass to the destination and the paint-to-screen blit.
//!
//! # Frame semantics (faithful to the vendored renderer)
//!
//! For each frame:
//!
//! 1. Every buffer's **read** view is bound for sampling; every pass renders
//!    into its buffer's **write** view (`ISFRenderer.draw`, lines 352-387).
//! 2. After *all* passes run, **every** buffer flips (`buffers[i].flip()`).
//!    A persistent buffer therefore feeds the previous frame's contents back
//!    into this frame; within a frame, pass N+1 reads the *previous* frame's
//!    pass N output, not this frame's. That one-frame lag is in the
//!    JavaScript renderer and is preserved here.
//! 3. If the **final** pass wrote to a target, that buffer is blitted to the
//!    destination (`paintToScreen`) — sampling the just-written texture,
//!    because the flip happens first. If the final pass is implicit it
//!    already rendered into the destination, so *no* blit happens: a
//!    pre-final buffer is never painted over the result (30 corpus shaders
//!    end in an implicit pass).
//!
//! # Buffer formats (`float-buffer-support.js`)
//!
//! `FLOAT: true` buffers allocate `Rgba32Float`, which is not filterable, so
//! their sampler is `Nearest` and the bind-group layout uses
//! `TextureSampleType::UnfilterableFloat` + `SamplerBindingType::NonFiltering`.
//! Everything else is `Rgba8Unorm` with a linear filtering sampler. This
//! replaces the WebGL monkey-patch natively.
//!
//! # Bind group contract (what a WGSL override must implement)
//!
//! * `@group(0) @binding(0)` — `var<uniform> global: IsfUniforms`, dynamic
//!   offset, one `RUNTIME_UNIFORM_STRIDE`-byte slice per pass. The member list
//!   and offsets come from [`super::source::uniform_members`] /
//!   [`super::source::uniform_layout`], in that order.
//! * For each texture in [`super::source::texture_inputs`] (input images and
//!   audio first, then pass targets), a pair: `@binding(2*i+1)` a
//!   `texture_2d<f32>`, `@binding(2*i+2)` a `sampler`.
//! * Entry point `main`, returning `@location(0) vec4<f32>`.
//!
//! # Vertex stage
//!
//! [`DEFAULT_VERTEX_WGSL`] draws the full-screen triangle and outputs
//! `@location(0) isf_FragNormCoord` (top-left origin). Translated `.vs`
//! sidecars are not consumed by the runtime yet (see
//! [`super::translate::translate_vertex`]); the translated fragment never
//! declares extra entry-point inputs (see [`super::source`]), so the built-in
//! vertex always links.
//!
//! # Not in this module
//!
//! Prop coercion/clamping and input links (WS-1) belong to the engine. This
//! runtime takes already-coerced values ([`IsfValues`]) plus the ISF defaults
//! from the parsed document.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use super::parser::{IsfDocument, IsfPass};
use super::props::truthy;
use super::source::{
    self, SizeVars, TextureBinding, UniformKind, UniformLayout, UniformSource, evaluate_size,
    uniform_layout, uniform_members,
};
use super::translate::{self, ShaderOrigin, TranslateError};

/// Uniform-buffer stride per pass. 256 bytes is
/// `wgpu::Limits::default().min_uniform_buffer_offset_alignment`, so dynamic
/// offsets stay valid on every backend.
pub const RUNTIME_UNIFORM_STRIDE: u64 = 256;

/// The built-in full-screen-triangle vertex shader. `isf_FragNormCoord` is
/// top-left origin, matching `@builtin(position)` and texture sampling in
/// wgpu (see the module docs in [`super::source`]).
pub const DEFAULT_VERTEX_WGSL: &str = r#"
struct IsfVertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) isf_FragNormCoord: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> IsfVertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[index];
    var out: IsfVertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.isf_FragNormCoord = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return out;
}
"#;

/// The paint-to-screen blit (`ISFRenderer.paintToScreen`).
const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct BlitOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> BlitOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[index];
    var out: BlitOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
    return out;
}

@fragment
fn fs_main(input: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(source_texture, source_sampler, input.uv);
}
"#;

/// Per-frame clock values, from `ISFRenderer.setDateUniforms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IsfFrameUniforms {
    /// `TIME` — seconds since the engine started.
    pub time: f32,
    /// `TIMEDELTA` — seconds since the previous frame.
    pub time_delta: f32,
    /// `FRAMEINDEX` — 1-based frame counter.
    ///
    /// `ISFRenderer` initialises `frameIndex = 0` and does `frameIndex += 1`
    /// *before* the first draw (`setDateUniforms`, `ISFRenderer.js:402-418`),
    /// so the first presented frame sees `1`. [`Default`] matches that first
    /// frame rather than starting at 0, and callers feeding a 0-based frame
    /// number (the engine's own `FrameInfo::frame_index`) must add one.
    pub frame_index: i32,
    /// `DATE` — `[year, month 1-based, day, seconds since midnight]`.
    pub date: [f32; 4],
}

impl Default for IsfFrameUniforms {
    /// The first frame, as the JavaScript renderer presents it: time 0 and
    /// `FRAMEINDEX` 1.
    fn default() -> Self {
        Self {
            time: 0.0,
            time_delta: 0.0,
            frame_index: 1,
            date: [0.0; 4],
        }
    }
}

/// A coerced ISF input value for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IsfValue {
    /// `float`
    Float(f64),
    /// `long`
    Int(i32),
    /// `bool`/`event`
    Bool(bool),
    /// `point2D`
    Vec2([f32; 2]),
    /// `color`
    Vec4([f32; 4]),
}

impl IsfValue {
    /// Numeric view, for `$<input>` substitution in `WIDTH`/`HEIGHT`.
    pub fn as_f64(self) -> Option<f64> {
        match self {
            IsfValue::Float(value) => Some(value),
            IsfValue::Int(value) => Some(value as f64),
            IsfValue::Bool(value) => Some(if value { 1.0 } else { 0.0 }),
            IsfValue::Vec2(_) | IsfValue::Vec4(_) => None,
        }
    }

    /// Coerce a prop / preset / input-link JSON value for a `bool` or `event`
    /// input.
    ///
    /// This is JavaScript `!!value` ([`truthy`]), not `Value::as_bool()`:
    /// a numeric `1`, the string `"1"` and the string `"false"` are all
    /// **true**, `0`, `""` and `null` are false. `Value::as_bool()` returns
    /// `None` for every non-boolean, so `as_bool().unwrap_or(false)` used to
    /// turn every numeric `1` into `false` (R-26). Engine prop coercion must
    /// use this helper.
    pub fn coerce_bool(value: Option<&serde_json::Value>) -> IsfValue {
        IsfValue::Bool(value.is_some_and(truthy))
    }
}

/// Input values keyed by ISF `NAME`.
pub type IsfValues = BTreeMap<String, IsfValue>;

/// Errors from compilation and drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum IsfRuntimeError {
    /// Naga could not translate a pass and there is no override.
    Translate(TranslateError),
    /// A `WIDTH`/`HEIGHT` expression could not be evaluated.
    InvalidPassSize {
        /// Pass index.
        pass: usize,
        /// Underlying evaluation error.
        message: String,
    },
    /// The destination is zero-sized (wgpu rejects 0×0 textures).
    ZeroSize,
    /// An override produced a shader without the `main` entry point.
    MissingEntryPoint(String),
}

impl fmt::Display for IsfRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IsfRuntimeError::Translate(error) => write!(f, "ISF translation failed: {error}"),
            IsfRuntimeError::InvalidPassSize { pass, message } => {
                write!(f, "pass {pass} size is invalid: {message}")
            }
            IsfRuntimeError::ZeroSize => write!(f, "ISF render size must be non-zero"),
            IsfRuntimeError::MissingEntryPoint(name) => {
                write!(f, "ISF shader {name:?} has no usable WGSL")
            }
        }
    }
}

impl std::error::Error for IsfRuntimeError {}

impl From<TranslateError> for IsfRuntimeError {
    fn from(error: TranslateError) -> Self {
        IsfRuntimeError::Translate(error)
    }
}

/// A simple LRU cache with a hard capacity.
///
/// Used for compiled pipelines (live + gallery of two shaders is the common
/// case; [`IsfRuntime`] keeps four, mirroring the bounded ISF caches the
/// renderer already uses — see `application/renderers/disposal.js`).
#[derive(Clone, Debug)]
pub struct BoundedCache<K: PartialEq, V> {
    capacity: usize,
    clock: u64,
    entries: Vec<(K, V, u64)>,
}

impl<K: PartialEq, V> BoundedCache<K, V> {
    /// Create a cache that holds at most `capacity` entries.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            clock: 0,
            entries: Vec::new(),
        }
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The cache capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Look up `key`, bumping its recency.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self
            .entries
            .iter_mut()
            .find(|(candidate, _, _)| candidate == key)?;
        entry.2 = clock;
        Some(&entry.1)
    }

    /// Insert `key`, evicting the least-recently-used entry when full.
    pub fn insert(&mut self, key: K, value: V) {
        self.clock += 1;
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|(candidate, _, _)| *candidate == key)
        {
            entry.1 = value;
            entry.2 = self.clock;
            return;
        }
        if self.entries.len() >= self.capacity
            && let Some((index, _)) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, _, used))| *used)
        {
            self.entries.remove(index);
        }
        self.entries.push((key, value, self.clock));
    }

    /// Empty the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// One ping-pong buffer (`ISFBuffer`).
///
/// Two textures per buffer: one readable, one writable. They swap at the end
/// of every frame. `FLOAT` buffers are `Rgba32Float`; everything else is
/// `Rgba8Unorm`.
pub struct IsfBuffer {
    /// Buffer name (`TARGET`).
    pub name: String,
    /// `WIDTH` formula (defaults to `$WIDTH`).
    pub width_expr: String,
    /// `HEIGHT` formula (defaults to `$HEIGHT`).
    pub height_expr: String,
    /// `PERSISTENT` — contents survive across frames.
    pub persistent: bool,
    /// `FLOAT` — `Rgba32Float` storage.
    pub float: bool,
    /// Current width in pixels.
    pub width: u32,
    /// Current height in pixels.
    pub height: u32,
    textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    flip_flop: bool,
}

impl IsfBuffer {
    fn new(device: &wgpu::Device, name: String, pass: &IsfPass) -> Self {
        let format = if pass.float {
            wgpu::TextureFormat::Rgba32Float
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        let (textures, views) = create_pair(device, &name, format, 1, 1);
        Self {
            name,
            width_expr: pass.width.clone().unwrap_or_else(|| "$WIDTH".into()),
            height_expr: pass.height.clone().unwrap_or_else(|| "$HEIGHT".into()),
            persistent: pass.persistent,
            float: pass.float,
            width: 1,
            height: 1,
            textures,
            views,
            flip_flop: false,
        }
    }

    /// The texture the current frame samples.
    pub fn read_view(&self) -> &wgpu::TextureView {
        &self.views[usize::from(self.flip_flop)]
    }

    /// The texture the current frame renders into.
    pub fn write_view(&self) -> &wgpu::TextureView {
        &self.views[1 - usize::from(self.flip_flop)]
    }

    /// The readable texture handle.
    pub fn read_texture(&self) -> &wgpu::Texture {
        &self.textures[usize::from(self.flip_flop)]
    }

    /// Texture format used by this buffer.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.textures[0].format()
    }

    fn flip(&mut self) {
        self.flip_flop = !self.flip_flop;
    }

    fn ensure_size(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        let (textures, views) = create_pair(device, &self.name, self.format(), width, height);
        self.textures = textures;
        self.views = views;
        self.flip_flop = false;
    }
}

fn create_pair(
    device: &wgpu::Device,
    name: &str,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> ([wgpu::Texture; 2], [wgpu::TextureView; 2]) {
    let create = |index: usize| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("modV ISF buffer {name}[{index}]")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    };
    let (texture_a, view_a) = create(0);
    let (texture_b, view_b) = create(1);
    ([texture_a, texture_b], [view_a, view_b])
}

/// A compiled pass: where it renders and how big it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledPass {
    /// Index into [`CompiledIsf::buffers`], or `None` for the implicit
    /// destination pass.
    pub target: Option<usize>,
    /// `WIDTH` formula (`$WIDTH` when absent).
    pub width_expr: String,
    /// `HEIGHT` formula (`$HEIGHT` when absent).
    pub height_expr: String,
    /// `CLEAR` — clear the target before drawing.
    pub clear: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotKind {
    Image,
    Audio,
    Buffer(usize),
}

#[derive(Clone, Debug)]
struct TextureSlot {
    binding: TextureBinding,
    kind: SlotKind,
    unfilterable: bool,
}

/// A compiled ISF shader: one pipeline + the per-pass buffers.
pub struct CompiledIsf {
    /// Shader name (used for pipeline caching and diagnostics).
    pub name: String,
    /// Passes in render order.
    pub passes: Vec<CompiledPass>,
    /// Ping-pong buffers referenced by [`CompiledPass::target`].
    pub buffers: Vec<IsfBuffer>,
    /// The uniform block layout (member offsets are what the encoder fills).
    pub layout: UniformLayout,
    /// ISF defaults for inputs the caller does not supply.
    pub default_values: IsfValues,
    /// The uniform buffer, `passes.len() * uniform_stride` bytes.
    pub uniform_buffer: wgpu::Buffer,
    /// Per-pass uniform slice size. At least [`RUNTIME_UNIFORM_STRIDE`] and a
    /// multiple of the device's dynamic-offset alignment: shaders with many
    /// inputs (e.g. Bloom) have a uniform block larger than one 256-byte slot.
    pub uniform_stride: u64,
    /// The format pipelines were compiled for.
    pub target_format: wgpu::TextureFormat,
    /// The device's `max_texture_dimension_2d`, used to clamp pass sizes.
    pub max_texture_dimension: u32,
    /// The buffer painted to the destination after the passes: `Some` only
    /// when the **final** pass declares a `TARGET` (an implicit final pass
    /// already renders into the destination, so nothing is blitted).
    pub last_target: Option<usize>,
    pass_pipelines: Vec<wgpu::RenderPipeline>,
    bind_group_layout: wgpu::BindGroupLayout,
    slots: Vec<TextureSlot>,
}

impl fmt::Debug for CompiledIsf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledIsf")
            .field("name", &self.name)
            .field("passes", &self.passes)
            .field("buffers", &self.buffers.len())
            .field("layout_size", &self.layout.size)
            .finish_non_exhaustive()
    }
}

impl CompiledIsf {
    /// Evaluate a pass target size for this frame.
    ///
    /// `None` for the implicit destination pass (`target.is_none()`) — its
    /// size is the destination size.
    pub fn evaluate_pass_size(
        &self,
        pass_index: usize,
        target_size: [u32; 2],
        uniforms: IsfFrameUniforms,
        values: &IsfValues,
    ) -> Result<Option<[u32; 2]>, IsfRuntimeError> {
        let pass = self
            .passes
            .get(pass_index)
            .ok_or(IsfRuntimeError::InvalidPassSize {
                pass: pass_index,
                message: "no such pass".into(),
            })?;
        if pass.target.is_none() {
            return Ok(None);
        }
        // `$<input>` substitution uses the same values the shader sees, with
        // the document's defaults for anything the caller did not supply (the
        // JS initialises every uniform to its DEFAULT at load).
        let mut merged = self.default_values.clone();
        merged.extend(values.iter().map(|(name, value)| (name.clone(), *value)));
        let vars = size_vars(
            target_size,
            pass_index,
            uniforms,
            &merged,
            self.max_texture_dimension,
        );
        let width = evaluate_size(&pass.width_expr, &vars).map_err(|error| {
            IsfRuntimeError::InvalidPassSize {
                pass: pass_index,
                message: format!("WIDTH {:?}: {error}", pass.width_expr),
            }
        })?;
        let height = evaluate_size(&pass.height_expr, &vars).map_err(|error| {
            IsfRuntimeError::InvalidPassSize {
                pass: pass_index,
                message: format!("HEIGHT {:?}: {error}", pass.height_expr),
            }
        })?;
        Ok(Some([width, height]))
    }

    /// The buffer with `name`, when compiled from a `TARGET`.
    pub fn buffer(&self, name: &str) -> Option<&IsfBuffer> {
        self.buffers.iter().find(|buffer| buffer.name == name)
    }
}

fn size_vars(
    target_size: [u32; 2],
    pass_index: usize,
    uniforms: IsfFrameUniforms,
    values: &IsfValues,
    max_dimension: u32,
) -> SizeVars {
    SizeVars {
        width: target_size[0] as f64,
        height: target_size[1] as f64,
        time: f64::from(uniforms.time),
        time_delta: f64::from(uniforms.time_delta),
        frame_index: f64::from(uniforms.frame_index),
        pass_index: pass_index as f64,
        inputs: values
            .iter()
            .filter_map(|(name, value)| value.as_f64().map(|value| (name.clone(), value)))
            .collect(),
        max_dimension,
    }
}

/// The engine's ISF runtime: device, queue, sampler defaults and a bounded
/// pipeline cache.
pub struct IsfRuntime {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Compiled fragment pipelines, keyed by
    /// `"<name>|<format>|<uniform layout signature>"`.
    pipelines: BoundedCache<String, wgpu::RenderPipeline>,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    dummy_texture: wgpu::Texture,
    dummy_view: wgpu::TextureView,
    blit_pipelines: BoundedCache<wgpu::TextureFormat, wgpu::RenderPipeline>,
    blit_layout: wgpu::BindGroupLayout,
}

impl IsfRuntime {
    /// Create a runtime. `pipelines` is the capacity of the compiled-shader
    /// cache (4 matches the existing `createBoundedCache(4)` caches).
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, pipelines: usize) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modV ISF linear sampler"),
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
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modV ISF nearest sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 0.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });
        let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("modV ISF dummy texture"),
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
                texture: &dummy_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0u8; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("modV ISF blit layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });

        Self {
            device,
            queue,
            pipelines: BoundedCache::new(pipelines),
            linear_sampler,
            nearest_sampler,
            dummy_texture,
            dummy_view,
            blit_pipelines: BoundedCache::new(4),
            blit_layout,
        }
    }

    /// The device the runtime was created with.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Compiled pipeline count (for tests/metrics).
    pub fn cached_pipelines(&self) -> usize {
        self.pipelines.len()
    }

    /// Compile `doc` for `target_format`.
    ///
    /// Translation uses [`translate::translate_named_with_overrides`] with
    /// [`ShaderOrigin::External`]: `<name>.wgsl` overrides apply only when the
    /// shader body matches the fingerprint manifest, so a user media shader
    /// that shares a corpus name does not silently inherit corpus behaviour.
    /// The pipeline is cached in the runtime's bounded LRU; the returned
    /// [`CompiledIsf`] owns its buffers and uniform buffer and keeps a cloned
    /// pipeline handle.
    pub fn compile(
        &mut self,
        doc: &IsfDocument,
        name: &str,
        target_format: wgpu::TextureFormat,
    ) -> Result<CompiledIsf, IsfRuntimeError> {
        let translated = translate::translate_named_with_overrides(
            name,
            doc,
            Some(&translate::default_overrides_dir()),
            ShaderOrigin::External,
        )?;
        let wgsl = translated
            .wgsl
            .ok_or_else(|| IsfRuntimeError::MissingEntryPoint(name.to_string()))?;
        // A real parse + entry-point lookup, not `wgsl.contains("fn main")`:
        // a comment could satisfy a substring check (R-26).
        if !translate::has_fragment_main(&wgsl)? {
            return Err(IsfRuntimeError::MissingEntryPoint(name.to_string()));
        }

        let members = uniform_members(doc);
        let layout = uniform_layout(&members);
        let bindings = source::texture_bindings(doc);

        // Float buffers are unfilterable (`Rgba32Float`).
        let float_buffers: HashSet<String> = doc
            .passes
            .iter()
            .filter(|pass| pass.float)
            .filter_map(|pass| pass.target.clone())
            .collect();

        let mut slots = Vec::with_capacity(bindings.len());
        for binding in bindings {
            let is_buffer = doc
                .passes
                .iter()
                .any(|pass| pass.target.as_deref() == Some(binding.name.as_str()));
            let kind = if is_buffer {
                SlotKind::Buffer(0) // patched after the buffers exist
            } else if doc
                .inputs
                .iter()
                .any(|input| input.name == binding.name && input.type_ == "image")
            {
                SlotKind::Image
            } else if doc.inputs.iter().any(|input| {
                input.name == binding.name && (input.type_ == "audio" || input.type_ == "audioFFT")
            }) {
                SlotKind::Audio
            } else {
                // An `IMPORTED` key: nothing supplies it in-tree, so it uses
                // the dummy texture.
                SlotKind::Image
            };
            slots.push(TextureSlot {
                unfilterable: float_buffers.contains(&binding.name),
                binding,
                kind,
            });
        }

        let mut layout_entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(layout.size as u64),
            },
            count: None,
        }];
        for slot in &slots {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: slot.binding.texture_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float {
                        filterable: !slot.unfilterable,
                    },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: slot.binding.sampler_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(if slot.unfilterable {
                    wgpu::SamplerBindingType::NonFiltering
                } else {
                    wgpu::SamplerBindingType::Filtering
                }),
                count: None,
            });
        }

        let bind_group_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("modV ISF bind group layout"),
                    entries: &layout_entries,
                });
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(name),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let vertex = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("modV ISF vertex"),
                source: wgpu::ShaderSource::Wgsl(DEFAULT_VERTEX_WGSL.into()),
            });
        let fragment = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(wgsl.clone().into()),
            });

        let mut buffers: Vec<IsfBuffer> = Vec::new();
        let mut passes = Vec::with_capacity(doc.passes.len());
        for pass in &doc.passes {
            let target = pass.target.as_ref().map(|target| {
                if let Some(index) = buffers.iter().position(|buffer| &buffer.name == target) {
                    index
                } else {
                    buffers.push(IsfBuffer::new(&self.device, target.clone(), pass));
                    buffers.len() - 1
                }
            });
            passes.push(CompiledPass {
                target,
                width_expr: pass.width.clone().unwrap_or_else(|| "$WIDTH".into()),
                height_expr: pass.height.clone().unwrap_or_else(|| "$HEIGHT".into()),
                clear: pass.clear,
            });
        }
        for slot in slots.iter_mut() {
            if let SlotKind::Image | SlotKind::Audio = slot.kind {
                continue;
            }
            if let Some(index) = buffers
                .iter()
                .position(|buffer| buffer.name == slot.binding.name)
            {
                slot.kind = SlotKind::Buffer(index);
            }
        }

        // Dynamic uniform offsets must be aligned to the device limit (256 on
        // desktop GPUs); the block itself can exceed the base stride when a
        // shader declares many inputs.
        let alignment = self
            .device
            .limits()
            .min_uniform_buffer_offset_alignment
            .max(16) as u64;
        let uniform_stride = round_up_u64(
            layout.size.max(RUNTIME_UNIFORM_STRIDE as usize) as u64,
            alignment,
        );

        let uniform_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modV ISF uniform buffer"),
            size: (passes.len() as u64).max(1) * uniform_stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Only the **final** pass decides the paint-to-screen blit: `ISFRenderer.draw`
        // resets `lastTarget = null` on every implicit pass, so a shader that
        // ends with an implicit pass already rendered into the destination and
        // must not have a pre-final buffer blitted over it (R-1: 30 corpus
        // shaders have the `[{TARGET: …}, {…}, {}]` shape).
        let last_target = passes.last().and_then(|pass| pass.target);
        let default_values = default_values(doc);

        // A pipeline is fixed to one colour-target format, and the
        // destination (usually `Rgba8Unorm`) differs from a `FLOAT` pass
        // buffer (`Rgba32Float`), so every pass gets the pipeline for *its*
        // target format. The bounded cache keys on the format.
        let mut pass_pipelines = Vec::with_capacity(passes.len());
        for pass in &passes {
            let format = match pass.target {
                Some(index) => buffers[index].format(),
                None => target_format,
            };
            let pipeline = self.fragment_pipeline(
                name,
                format,
                layout.size,
                slots.len(),
                &pipeline_layout,
                &vertex,
                &fragment,
            );
            pass_pipelines.push(pipeline);
        }

        Ok(CompiledIsf {
            name: name.to_string(),
            passes,
            buffers,
            layout,
            default_values,
            uniform_buffer,
            uniform_stride,
            target_format,
            max_texture_dimension: self.device.limits().max_texture_dimension_2d.max(1),
            last_target,
            pass_pipelines,
            bind_group_layout,
            slots,
        })
    }

    /// Get (or create) the fragment pipeline for one target format.
    #[allow(clippy::too_many_arguments)]
    fn fragment_pipeline(
        &mut self,
        name: &str,
        format: wgpu::TextureFormat,
        uniform_size: usize,
        texture_slots: usize,
        pipeline_layout: &wgpu::PipelineLayout,
        vertex: &wgpu::ShaderModule,
        fragment: &wgpu::ShaderModule,
    ) -> wgpu::RenderPipeline {
        let cache_key = format!("{name}|{format:?}|{uniform_size}|{texture_slots}");
        if let Some(pipeline) = self.pipelines.get(&cache_key) {
            return pipeline.clone();
        }
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: vertex,
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
                    module: fragment,
                    entry_point: Some("main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert(cache_key, pipeline.clone());
        pipeline
    }

    /// Render one frame.
    ///
    /// * `target_view` / `target_size` — the destination (the group canvas or
    ///   an offscreen target). Must not alias an input image bind group entry.
    /// * `values` — coerced props; missing inputs use the document defaults.
    /// * `input_images` — `(NAME, view)` for `image` inputs. A missing name
    ///   binds a 1×1 transparent dummy.
    /// * `audio_textures` — `(NAME, view)` for `audio`/`audioFFT` inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        compiled: &mut CompiledIsf,
        target_view: &wgpu::TextureView,
        target_size: [u32; 2],
        uniforms: IsfFrameUniforms,
        values: &IsfValues,
        input_images: &[(&str, &wgpu::TextureView)],
        audio_textures: &[(&str, &wgpu::TextureView)],
    ) -> Result<(), IsfRuntimeError> {
        let target_size = [target_size[0].max(1), target_size[1].max(1)];

        // 1. Size every buffer for this frame.
        for index in 0..compiled.passes.len() {
            if let Some([width, height]) =
                compiled.evaluate_pass_size(index, target_size, uniforms, values)?
            {
                let target = compiled.passes[index].target.expect("target was Some");
                compiled.buffers[target].ensure_size(&self.device, width, height);
            }
        }

        // 2. Resolve texture metadata (`_name_imgSize`).
        let mut texture_sizes: BTreeMap<&str, [f32; 2]> = BTreeMap::new();
        for slot in &compiled.slots {
            let view = match slot.kind {
                SlotKind::Image => {
                    lookup(input_images, &slot.binding.name).unwrap_or(&self.dummy_view)
                }
                SlotKind::Audio => {
                    lookup(audio_textures, &slot.binding.name).unwrap_or(&self.dummy_view)
                }
                SlotKind::Buffer(index) => compiled.buffers[index].read_view(),
            };
            let texture = view.texture();
            texture_sizes.insert(
                slot.binding.name.as_str(),
                [texture.width() as f32, texture.height() as f32],
            );
        }

        // 3. Encode every pass's uniform slice.
        let stride = compiled.uniform_stride as usize;
        let mut uniform_bytes = vec![0u8; compiled.passes.len().max(1) * stride];
        for (index, pass) in compiled.passes.iter().enumerate() {
            let render_size = match pass.target {
                Some(target) => {
                    let buffer = &compiled.buffers[target];
                    [buffer.width as f32, buffer.height as f32]
                }
                None => [target_size[0] as f32, target_size[1] as f32],
            };
            encode_uniforms(
                &compiled.layout,
                &mut uniform_bytes[index * stride..],
                uniforms,
                index as i32,
                render_size,
                &compiled.default_values,
                values,
                &texture_sizes,
            );
        }
        self.queue
            .write_buffer(&compiled.uniform_buffer, 0, &uniform_bytes);

        // 4. One bind group: reads are constant across passes.
        let mut entries = Vec::with_capacity(1 + compiled.slots.len() * 2);
        entries.push(wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &compiled.uniform_buffer,
                offset: 0,
                size: wgpu::BufferSize::new(compiled.layout.size as u64),
            }),
        });
        for slot in &compiled.slots {
            let view = match slot.kind {
                SlotKind::Image => {
                    lookup(input_images, &slot.binding.name).unwrap_or(&self.dummy_view)
                }
                SlotKind::Audio => {
                    lookup(audio_textures, &slot.binding.name).unwrap_or(&self.dummy_view)
                }
                SlotKind::Buffer(index) => compiled.buffers[index].read_view(),
            };
            entries.push(wgpu::BindGroupEntry {
                binding: slot.binding.texture_binding,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: slot.binding.sampler_binding,
                resource: wgpu::BindingResource::Sampler(if slot.unfilterable {
                    &self.nearest_sampler
                } else {
                    &self.linear_sampler
                }),
            });
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("modV ISF bind group"),
            layout: &compiled.bind_group_layout,
            entries: &entries,
        });

        // 5. Run the passes.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("modV ISF frame"),
            });
        for (index, pass) in compiled.passes.iter().enumerate() {
            let view = match pass.target {
                Some(target) => compiled.buffers[target].write_view(),
                None => target_view,
            };
            let load = if pass.clear {
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
            } else {
                wgpu::LoadOp::Load
            };
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("modV ISF pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            render_pass.set_pipeline(&compiled.pass_pipelines[index]);
            render_pass.set_bind_group(
                0,
                &bind_group,
                &[(index as u64 * compiled.uniform_stride) as u32],
            );
            render_pass.draw(0..3, 0..1);
        }

        // 6. Flip *all* buffers, as ISFRenderer.draw does.
        for buffer in compiled.buffers.iter_mut() {
            buffer.flip();
        }

        // 7. Paint the final target to the destination. An implicit final pass
        //    already rendered straight into it, so there is nothing to paint
        //    (`last_target` is `None`) and a pre-final buffer is never blitted
        //    over the result.
        if let Some(target) = compiled.last_target {
            let source = compiled.buffers[target].read_view();
            self.blit(&mut encoder, source, target_view, compiled.target_format);
        }

        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn blit(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        let pipeline = match self.blit_pipelines.get(&format) {
            Some(pipeline) => pipeline.clone(),
            None => {
                let shader = self
                    .device
                    .create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("modV ISF blit"),
                        source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
                    });
                let pipeline_layout =
                    self.device
                        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("modV ISF blit"),
                            bind_group_layouts: &[Some(&self.blit_layout)],
                            immediate_size: 0,
                        });
                let pipeline =
                    self.device
                        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some("modV ISF blit"),
                            layout: Some(&pipeline_layout),
                            vertex: wgpu::VertexState {
                                module: &shader,
                                entry_point: Some("vs_main"),
                                compilation_options: wgpu::PipelineCompilationOptions::default(),
                                buffers: &[],
                            },
                            primitive: wgpu::PrimitiveState::default(),
                            depth_stencil: None,
                            multisample: wgpu::MultisampleState::default(),
                            fragment: Some(wgpu::FragmentState {
                                module: &shader,
                                entry_point: Some("fs_main"),
                                compilation_options: wgpu::PipelineCompilationOptions::default(),
                                targets: &[Some(wgpu::ColorTargetState {
                                    format,
                                    blend: None,
                                    write_mask: wgpu::ColorWrites::ALL,
                                })],
                            }),
                            multiview_mask: None,
                            cache: None,
                        });
                self.blit_pipelines.insert(format, pipeline.clone());
                pipeline
            }
        };

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("modV ISF blit"),
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.nearest_sampler),
                },
            ],
        });
        let _ = &self.dummy_texture;
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: destination,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("modV ISF blit pass"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render_pass.set_pipeline(&pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

fn lookup<'a>(
    textures: &'a [(&str, &'a wgpu::TextureView)],
    name: &str,
) -> Option<&'a wgpu::TextureView> {
    textures
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, view)| *view)
}

/// ISF defaults from the parsed inputs.
fn default_values(doc: &IsfDocument) -> IsfValues {
    let mut values = IsfValues::new();
    for input in &doc.inputs {
        let value = match input.type_.as_str() {
            "float" => IsfValue::Float(input.default.as_f64().unwrap_or(0.0)),
            "long" => IsfValue::Int(
                input
                    .default
                    .as_i64()
                    .or_else(|| input.default.as_f64().map(|value| value as i64))
                    .unwrap_or(0) as i32,
            ),
            "bool" | "event" => IsfValue::Bool(truthy(&input.default)),
            "point2D" => {
                let array = input.default.as_array();
                IsfValue::Vec2([
                    array
                        .and_then(|array| array.first())
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0) as f32,
                    array
                        .and_then(|array| array.get(1))
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0) as f32,
                ])
            }
            "color" => {
                let array = input.default.as_array();
                let mut colour = [0.0f32; 4];
                if let Some(array) = array {
                    for (index, slot) in colour.iter_mut().enumerate() {
                        *slot = array
                            .get(index)
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0) as f32;
                    }
                }
                IsfValue::Vec4(colour)
            }
            _ => continue,
        };
        values.insert(input.name.clone(), value);
    }
    values
}

/// Encode one pass's uniform slice.
///
/// The member order/offsets come from [`UniformLayout`]; the WGSL struct was
/// generated from the same member list, so member names line up exactly.
#[allow(clippy::too_many_arguments)]
fn encode_uniforms(
    layout: &UniformLayout,
    destination: &mut [u8],
    uniforms: IsfFrameUniforms,
    pass_index: i32,
    render_size: [f32; 2],
    defaults: &IsfValues,
    values: &IsfValues,
    texture_sizes: &BTreeMap<&str, [f32; 2]>,
) {
    for member in &layout.members {
        let offset = member.offset;
        match &member.source {
            UniformSource::Date => write_vec4(destination, offset, uniforms.date),
            UniformSource::RenderSize => write_vec2(destination, offset, render_size),
            UniformSource::PassIndex => write_i32(destination, offset, pass_index),
            UniformSource::FrameIndex => write_i32(destination, offset, uniforms.frame_index),
            UniformSource::Time => write_f32(destination, offset, uniforms.time),
            UniformSource::TimeDelta => write_f32(destination, offset, uniforms.time_delta),
            UniformSource::Input(name) => {
                let value = values.get(name).or_else(|| defaults.get(name)).copied();
                match (member.kind, value) {
                    (UniformKind::Float, Some(value)) => {
                        write_f32(destination, offset, value.as_f64().unwrap_or(0.0) as f32)
                    }
                    (UniformKind::Int, Some(value)) => write_i32(
                        destination,
                        offset,
                        match value {
                            IsfValue::Int(value) => value,
                            IsfValue::Bool(value) => i32::from(value),
                            other => other.as_f64().unwrap_or(0.0) as i32,
                        },
                    ),
                    (UniformKind::Vec2, Some(IsfValue::Vec2(value))) => {
                        write_vec2(destination, offset, value)
                    }
                    (UniformKind::Vec4, Some(IsfValue::Vec4(value))) => {
                        write_vec4(destination, offset, value)
                    }
                    (UniformKind::Vec2, _) => write_vec2(destination, offset, [0.0, 0.0]),
                    (UniformKind::Vec4, _) => write_vec4(destination, offset, [0.0; 4]),
                    (UniformKind::Float, None) => write_f32(destination, offset, 0.0),
                    (UniformKind::Int, None) => write_i32(destination, offset, 0),
                }
            }
            UniformSource::ImageSize(name) => write_vec2(
                destination,
                offset,
                texture_sizes
                    .get(name.as_str())
                    .copied()
                    .unwrap_or([1.0, 1.0]),
            ),
            UniformSource::ImageRect(_) => write_vec4(destination, offset, [0.0, 0.0, 1.0, 1.0]),
            UniformSource::ImageFlip(_) => write_i32(destination, offset, 0),
        }
    }
}

fn round_up_u64(value: u64, align: u64) -> u64 {
    if align == 0 {
        return value;
    }
    value.div_ceil(align) * align
}

fn write_f32(destination: &mut [u8], offset: usize, value: f32) {
    let bytes = value.to_ne_bytes();
    destination[offset..offset + 4].copy_from_slice(&bytes);
}

fn write_i32(destination: &mut [u8], offset: usize, value: i32) {
    let bytes = value.to_ne_bytes();
    destination[offset..offset + 4].copy_from_slice(&bytes);
}

fn write_vec2(destination: &mut [u8], offset: usize, value: [f32; 2]) {
    write_f32(destination, offset, value[0]);
    write_f32(destination, offset + 4, value[1]);
}

fn write_vec4(destination: &mut [u8], offset: usize, value: [f32; 4]) {
    for (index, component) in value.iter().enumerate() {
        write_f32(destination, offset + index * 4, *component);
    }
}

/// Read an `Rgba8Unorm`/`Rgba32Float` texture back for tests and golden
/// checks. Kept in the public API because the engine's preview path needs the
/// same helper.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<u8>, String> {
    let width = texture.width();
    let height = texture.height();
    let bytes_per_pixel: u64 = match texture.format() {
        wgpu::TextureFormat::Rgba32Float => 16,
        _ => 4,
    };
    let unpadded = width as u64 * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("modV ISF read-back"),
        size: padded * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("modV ISF read-back"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
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
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let mapped = slice
        .get_mapped_range()
        .map_err(|error| error.to_string())?;
    let mut out = Vec::with_capacity((unpadded * height as u64) as usize);
    for row in 0..height as u64 {
        let start = (row * padded) as usize;
        let end = start + unpadded as usize;
        out.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isf::parser::parse_isf;

    fn doc(source: &str) -> IsfDocument {
        parse_isf(source, None).expect("test shader parses")
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll};
        let waker = std::task::Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn try_runtime() -> Option<IsfRuntime> {
        let instance = wgpu::Instance::default();
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: true,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("modv-isf-test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .ok()?;
        Some(IsfRuntime::new(device, queue, 4))
    }

    fn target(runtime: &IsfRuntime, size: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        runtime.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("modV ISF test target"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn solid_input(
        runtime: &IsfRuntime,
        size: u32,
        rgba: [u8; 4],
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = runtime.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("modV ISF test input"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let data = rgba.repeat((size * size) as usize);
        runtime.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    #[test]
    fn bounded_cache_evicts_the_least_recently_used() {
        let mut cache: BoundedCache<String, u32> = BoundedCache::new(2);
        cache.insert("a".into(), 1);
        cache.insert("b".into(), 2);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&"a".into()), Some(&1), "a is now most recent");
        cache.insert("c".into(), 3);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&"b".into()), None, "b was evicted");
        assert_eq!(cache.get(&"a".into()), Some(&1));
        assert_eq!(cache.get(&"c".into()), Some(&3));
        cache.insert("a".into(), 10);
        assert_eq!(cache.get(&"a".into()), Some(&10));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn bounded_cache_reports_its_capacity() {
        let cache: BoundedCache<u8, u8> = BoundedCache::new(0);
        assert_eq!(cache.capacity(), 1, "capacity is clamped to one");
        assert!(cache.is_empty());
    }

    #[test]
    fn soft_blur_formulas_size_the_compiled_passes() {
        let shader = r#"/*{
            "PASSES": [
                { "TARGET": "smaller", "WIDTH": "max(floor($WIDTH*0.02),1.0)", "HEIGHT": "max(floor($HEIGHT*0.02),1.0)" },
                { "TARGET": "small", "WIDTH": "max(floor($WIDTH*0.25),1.0)", "HEIGHT": "max(floor($HEIGHT*0.25),1.0)" },
                { }
            ]
        }*/
        void main() { gl_FragColor = vec4(1.0); }"#;
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF size test: no adapter");
            return;
        };
        let document = doc(shader);
        let compiled = runtime
            .compile(
                &document,
                "Soft Blur Sizes",
                wgpu::TextureFormat::Rgba8Unorm,
            )
            .expect("compiles");
        assert_eq!(
            compiled.last_target, None,
            "Soft Blur ends with an implicit pass, so nothing is blitted"
        );
        assert_eq!(
            compiled
                .evaluate_pass_size(
                    0,
                    [1280, 720],
                    IsfFrameUniforms::default(),
                    &IsfValues::new(),
                )
                .expect("evaluates"),
            Some([25, 14])
        );
        assert_eq!(
            compiled
                .evaluate_pass_size(
                    1,
                    [1280, 720],
                    IsfFrameUniforms::default(),
                    &IsfValues::new(),
                )
                .expect("evaluates"),
            Some([320, 180])
        );
        assert_eq!(
            compiled
                .evaluate_pass_size(
                    2,
                    [1280, 720],
                    IsfFrameUniforms::default(),
                    &IsfValues::new(),
                )
                .expect("evaluates"),
            None,
            "the implicit final pass uses the destination size"
        );

        // A `$<input>` in the formula is substituted from the frame values.
        let with_input = doc(
            r#"/*{ "INPUTS": [ { "NAME": "scale", "TYPE": "float", "DEFAULT": 0.5 } ],
                "PASSES": [ { "TARGET": "a", "WIDTH": "floor($WIDTH * $scale)", "HEIGHT": "$HEIGHT" } ] }*/
            void main() { gl_FragColor = vec4(1.0); }"#,
        );
        let compiled = runtime
            .compile(&with_input, "Scaled Sizes", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");
        assert_eq!(
            compiled
                .evaluate_pass_size(0, [100, 40], IsfFrameUniforms::default(), &IsfValues::new(),)
                .expect("evaluates"),
            Some([50, 40]),
            "the DEFAULT is used when no value is supplied"
        );
        let mut values = IsfValues::new();
        values.insert("scale".into(), IsfValue::Float(0.25));
        assert_eq!(
            compiled
                .evaluate_pass_size(0, [100, 40], IsfFrameUniforms::default(), &values,)
                .expect("evaluates"),
            Some([25, 40])
        );
    }

    #[test]
    fn compiling_the_same_shader_reuses_the_cached_pipeline() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF cache test: no adapter");
            return;
        };
        let document = doc("void main() { gl_FragColor = vec4(1.0); }");
        runtime
            .compile(&document, "Cache Test", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");
        assert_eq!(runtime.cached_pipelines(), 1);
        runtime
            .compile(&document, "Cache Test", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles again");
        assert_eq!(runtime.cached_pipelines(), 1, "the cache is reused");
    }

    #[test]
    fn compile_reports_translation_failures() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF compile test: no adapter");
            return;
        };
        let document = doc("uniform sampler3D vol;\nvoid main() { gl_FragColor = vec4(1.0); }");
        let error = runtime
            .compile(
                &document,
                "Broken Test Shader",
                wgpu::TextureFormat::Rgba8Unorm,
            )
            .expect_err("sampler3D cannot translate");
        assert!(matches!(error, IsfRuntimeError::Translate(_)), "{error:?}");
    }

    #[test]
    fn invert_shader_renders_the_expected_pixels() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF render test: no adapter");
            return;
        };
        let shader = r#"/*{ "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ] }*/
        void main() { gl_FragColor = vec4(1.0) - IMG_THIS_NORM_PIXEL(inputImage); }"#;
        let document = doc(shader);
        let mut compiled = runtime
            .compile(&document, "Invert Test", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");

        let (input_texture, input_view) = solid_input(&runtime, 64, [64, 128, 192, 255]);
        let destination = target(&runtime, 64, wgpu::TextureFormat::Rgba8Unorm);
        let destination_view = destination.create_view(&wgpu::TextureViewDescriptor::default());

        runtime
            .draw(
                &mut compiled,
                &destination_view,
                [64, 64],
                IsfFrameUniforms {
                    time: 0.0,
                    time_delta: 0.0,
                    frame_index: 1,
                    date: [2026.0, 11.0, 12.0, 0.0],
                },
                &IsfValues::new(),
                &[("inputImage", &input_view)],
                &[],
            )
            .expect("draws");
        let _ = input_texture;

        let pixels =
            read_texture(runtime.device(), &runtime.queue, &destination).expect("read back");
        // 1.0 - 64/255 = 191/255, 1 - 128/255 = 127, 1 - 192/255 = 63.
        for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
            assert!(
                (pixel[0] as i16 - 191).abs() <= 1,
                "pixel {index} r = {}",
                pixel[0]
            );
            assert!(
                (pixel[1] as i16 - 127).abs() <= 1,
                "pixel {index} g = {}",
                pixel[1]
            );
            assert!(
                (pixel[2] as i16 - 63).abs() <= 1,
                "pixel {index} b = {}",
                pixel[2]
            );
        }
    }

    #[test]
    fn many_inputs_grow_the_uniform_stride_beyond_one_256_byte_slot() {
        // Regression: the per-pass uniform slice was a fixed 256 bytes, so a
        // shader with many inputs (e.g. Bloom's ten image inputs) wrote past
        // the slot and panicked. The stride must be the block size rounded up
        // to the dynamic-offset alignment.
        let mut inputs = Vec::new();
        for index in 0..10 {
            inputs.push(format!(r#"{{ "NAME": "image{index}", "TYPE": "image" }}"#));
        }
        for index in 0..20 {
            inputs.push(format!(
                r#"{{ "NAME": "float{index}", "TYPE": "float", "DEFAULT": 0.5 }}"#
            ));
        }
        for index in 0..6 {
            inputs.push(format!(
                r#"{{ "NAME": "colour{index}", "TYPE": "color", "DEFAULT": [1,0,0,1] }}"#
            ));
        }
        for index in 0..6 {
            inputs.push(format!(
                r#"{{ "NAME": "point{index}", "TYPE": "point2D", "DEFAULT": [0.5,0.5] }}"#
            ));
        }
        let shader = format!(
            "/*{{ \"INPUTS\": [{}] }}*/\nvoid main() {{ gl_FragColor = IMG_NORM_PIXEL(image0, vec2(0.5)); }}",
            inputs.join(",")
        );
        let document = doc(&shader);

        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF uniform-stride test: no adapter");
            return;
        };
        let mut compiled = runtime
            .compile(&document, "Many Inputs", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");
        assert!(
            compiled.layout.size > RUNTIME_UNIFORM_STRIDE as usize,
            "test shader must exceed one uniform slot (layout {})",
            compiled.layout.size
        );
        assert!(compiled.uniform_stride >= compiled.layout.size as u64);
        assert_eq!(
            compiled.uniform_stride % 16,
            0,
            "dynamic offsets must stay aligned"
        );

        let destination = target(&runtime, 8, wgpu::TextureFormat::Rgba8Unorm);
        let destination_view = destination.create_view(&wgpu::TextureViewDescriptor::default());
        runtime
            .draw(
                &mut compiled,
                &destination_view,
                [8, 8],
                IsfFrameUniforms {
                    time: 0.0,
                    time_delta: 0.0,
                    frame_index: 0,
                    date: [2026.0, 11.0, 12.0, 0.0],
                },
                &IsfValues::new(),
                &[],
                &[],
            )
            .expect("draws without overflowing the uniform slot");
    }

    #[test]
    fn persistent_float_buffer_accumulates_across_frames() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF persistent test: no adapter");
            return;
        };
        let shader = r#"/*{
            "PASSES": [ { "TARGET": "acc", "PERSISTENT": true, "FLOAT": true } ]
        }*/
        void main() {
            vec4 previous = IMG_NORM_PIXEL(acc, vec2(0.5));
            gl_FragColor = previous + vec4(1.0 / 255.0);
        }"#;
        let document = doc(shader);
        let mut compiled = runtime
            .compile(
                &document,
                "Persistent Test",
                wgpu::TextureFormat::Rgba8Unorm,
            )
            .expect("compiles");
        assert!(compiled.buffers[0].float);
        assert_eq!(
            compiled.buffers[0].format(),
            wgpu::TextureFormat::Rgba32Float,
            "FLOAT buffers must be Rgba32Float"
        );

        let destination = target(&runtime, 8, wgpu::TextureFormat::Rgba8Unorm);
        let destination_view = destination.create_view(&wgpu::TextureViewDescriptor::default());

        let mut draw = |compiled: &mut CompiledIsf, frame: i32| {
            runtime
                .draw(
                    compiled,
                    &destination_view,
                    [8, 8],
                    IsfFrameUniforms {
                        time: frame as f32 / 60.0,
                        time_delta: 1.0 / 60.0,
                        frame_index: frame,
                        date: [2026.0, 11.0, 12.0, 0.0],
                    },
                    &IsfValues::new(),
                    &[],
                    &[],
                )
                .expect("draws");
        };
        draw(&mut compiled, 1);
        draw(&mut compiled, 2);

        // Frame 1 writes 1/255 into the persistent buffer; frame 2 reads it
        // and writes 2/255, then blits it to the destination. 2/255 × 255 = 2.
        let pixels =
            read_texture(runtime.device(), &runtime.queue, &destination).expect("read back");
        let pixel = &pixels[..4];
        for (channel, value) in pixel.iter().take(3).enumerate() {
            assert!(
                (*value as i16 - 2).abs() <= 1,
                "channel {channel} = {value} after two accumulate frames"
            );
        }
    }

    #[test]
    fn implicit_final_pass_renders_to_the_destination() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF implicit-pass test: no adapter");
            return;
        };
        let document = doc("void main() { gl_FragColor = vec4(0.0, 1.0, 0.0, 1.0); }");
        let mut compiled = runtime
            .compile(&document, "Green Test", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");
        assert!(compiled.last_target.is_none());
        let destination = target(&runtime, 8, wgpu::TextureFormat::Rgba8Unorm);
        let view = destination.create_view(&wgpu::TextureViewDescriptor::default());
        runtime
            .draw(
                &mut compiled,
                &view,
                [8, 8],
                IsfFrameUniforms::default(),
                &IsfValues::new(),
                &[],
                &[],
            )
            .expect("draws");
        let pixels =
            read_texture(runtime.device(), &runtime.queue, &destination).expect("read back");
        assert_eq!(&pixels[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn uniform_values_reach_the_shader() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF uniform test: no adapter");
            return;
        };
        let shader = r#"/*{ "INPUTS": [
            { "NAME": "bright", "TYPE": "float", "DEFAULT": 0.0 }
        ] }*/
        void main() { gl_FragColor = vec4(bright, 0.0, 0.0, 1.0); }"#;
        let document = doc(shader);
        let mut compiled = runtime
            .compile(&document, "Uniform Test", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");
        let destination = target(&runtime, 8, wgpu::TextureFormat::Rgba8Unorm);
        let view = destination.create_view(&wgpu::TextureViewDescriptor::default());
        let mut values = IsfValues::new();
        values.insert("bright".into(), IsfValue::Float(0.5));
        runtime
            .draw(
                &mut compiled,
                &view,
                [8, 8],
                IsfFrameUniforms::default(),
                &values,
                &[],
                &[],
            )
            .expect("draws");
        let pixels =
            read_texture(runtime.device(), &runtime.queue, &destination).expect("read back");
        assert!(
            (pixels[0] as i16 - 128).abs() <= 1,
            "bright 0.5 → red {}",
            pixels[0]
        );
    }

    #[test]
    fn multi_pass_shader_compiles_and_sizes_its_buffers() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF multi-pass test: no adapter");
            return;
        };
        let shader = r#"/*{
            "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ],
            "PASSES": [
                { "TARGET": "quarter", "WIDTH": "max(floor($WIDTH*0.25),1.0)", "HEIGHT": "max(floor($HEIGHT*0.25),1.0)" },
                { "TARGET": "out" }
            ]
        }*/
        void main() {
            vec4 colour = IMG_THIS_NORM_PIXEL(inputImage);
            if (PASSINDEX == 0) { gl_FragColor = colour; }
            else { gl_FragColor = IMG_NORM_PIXEL(quarter, isf_FragNormCoord); }
        }"#;
        let document = doc(shader);
        let mut compiled = runtime
            .compile(
                &document,
                "Multi Pass Test",
                wgpu::TextureFormat::Rgba8Unorm,
            )
            .expect("compiles");
        assert_eq!(compiled.passes.len(), 2);
        assert_eq!(compiled.buffers.len(), 2);
        assert_eq!(compiled.last_target, Some(1));

        let (input_texture, input_view) = solid_input(&runtime, 16, [255, 255, 255, 255]);
        let destination = target(&runtime, 64, wgpu::TextureFormat::Rgba8Unorm);
        let view = destination.create_view(&wgpu::TextureViewDescriptor::default());
        // Two frames: pass 1 reads pass 0's buffer through the *read* set,
        // which only contains the previous frame's write (the vendored
        // renderer flips buffers once per frame).
        for frame in 0..2 {
            runtime
                .draw(
                    &mut compiled,
                    &view,
                    [64, 64],
                    IsfFrameUniforms {
                        frame_index: frame,
                        ..Default::default()
                    },
                    &IsfValues::new(),
                    &[("inputImage", &input_view)],
                    &[],
                )
                .expect("draws");
        }
        let _ = input_texture;
        let quarter = compiled.buffer("quarter").expect("quarter buffer");
        assert_eq!((quarter.width, quarter.height), (16, 16));
        let pixels =
            read_texture(runtime.device(), &runtime.queue, &destination).expect("read back");
        assert_eq!(&pixels[..4], &[255, 255, 255, 255]);
    }

    /// Regression (R-1): a shader whose **final** pass is implicit must not be
    /// overwritten by a blit of the last *targeted* buffer.
    ///
    /// `ISFRenderer.draw` (`ISFRenderer.js`, `lastTarget` reset on every
    /// implicit pass) only paints a buffer to the screen when the *final* pass
    /// declares a `TARGET`; otherwise the implicit final pass already rendered
    /// straight into the destination. Taking the last targeted pass instead
    /// (Soft Blur, Glow, Lens Flare, rgbtimeglitch, …) blits the pre-final
    /// buffer over the result, discarding the final pass.
    #[test]
    fn trailing_implicit_final_pass_is_not_overwritten_by_a_blit() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF trailing-implicit test: no adapter");
            return;
        };
        let shader = r#"/*{
            "PASSES": [ { "TARGET": "tinted" }, { } ]
        }*/
        void main() {
            if (PASSINDEX == 0) {
                gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0);
            } else {
                gl_FragColor = IMG_NORM_PIXEL(tinted, isf_FragNormCoord)
                    + vec4(0.0, 0.0, 1.0, 0.0);
            }
        }"#;
        let document = doc(shader);
        let mut compiled = runtime
            .compile(
                &document,
                "Trailing Implicit Test",
                wgpu::TextureFormat::Rgba8Unorm,
            )
            .expect("compiles");
        assert_eq!(compiled.passes.len(), 2);
        assert_eq!(
            compiled.last_target, None,
            "the final pass is implicit, so nothing is painted over the destination"
        );

        let destination = target(&runtime, 16, wgpu::TextureFormat::Rgba8Unorm);
        let view = destination.create_view(&wgpu::TextureViewDescriptor::default());
        // Frame 1 fills `tinted`; frame 2's final pass reads it (the vendored
        // renderer flips buffers once per frame).
        for frame in 0..2 {
            runtime
                .draw(
                    &mut compiled,
                    &view,
                    [16, 16],
                    IsfFrameUniforms {
                        frame_index: frame,
                        ..Default::default()
                    },
                    &IsfValues::new(),
                    &[],
                    &[],
                )
                .expect("draws");
        }

        let pixels =
            read_texture(runtime.device(), &runtime.queue, &destination).expect("read back");
        let pixel = &pixels[..4];
        assert_eq!(
            pixel,
            &[255, 0, 255, 255],
            "the destination must hold the implicit final pass (red + blue), not \
             the pre-final buffer (red)"
        );
    }

    #[test]
    fn clear_false_preserves_the_buffer() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF clear test: no adapter");
            return;
        };
        let shader = r#"/*{
            "PASSES": [ { "TARGET": "acc", "PERSISTENT": true, "FLOAT": true, "CLEAR": false } ]
        }*/
        void main() {
            vec4 previous = IMG_NORM_PIXEL(acc, vec2(0.5));
            gl_FragColor = max(previous, vec4(0.25));
        }"#;
        let document = doc(shader);
        let mut compiled = runtime
            .compile(&document, "Clear Test", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");
        let destination = target(&runtime, 8, wgpu::TextureFormat::Rgba8Unorm);
        let view = destination.create_view(&wgpu::TextureViewDescriptor::default());
        for frame in 0..2 {
            runtime
                .draw(
                    &mut compiled,
                    &view,
                    [8, 8],
                    IsfFrameUniforms::default(),
                    &IsfValues::new(),
                    &[],
                    &[],
                )
                .unwrap_or_else(|error| panic!("frame {frame}: {error}"));
        }
        let pixels =
            read_texture(runtime.device(), &runtime.queue, &destination).expect("read back");
        // 0.25 × 255 = 63.75 → 64 on the way out of the float buffer.
        assert!(
            (pixels[0] as i16 - 64).abs() <= 1,
            "green channel {}",
            pixels[0]
        );
    }

    #[test]
    fn frame_uniform_defaults_match_the_first_javascript_frame() {
        // `ISFRenderer` increments `frameIndex` to 1 before the first draw, so
        // a default-constructed frame is frame 1, not 0 (R-26).
        let defaults = IsfFrameUniforms::default();
        assert_eq!(defaults.frame_index, 1);
        assert_eq!(defaults.time, 0.0);
        assert_eq!(defaults.time_delta, 0.0);
    }

    #[test]
    fn bool_inputs_use_javascript_truthiness() {
        // `!!value`, not `Value::as_bool()`: numeric 1 and the strings "1" and
        // "false" are all true (R-26).
        let one = serde_json::json!(1);
        let zero = serde_json::json!(0);
        let text = serde_json::json!("1");
        let words = serde_json::json!("false");
        let empty = serde_json::json!("");
        let nothing = serde_json::Value::Null;
        assert_eq!(IsfValue::coerce_bool(Some(&one)), IsfValue::Bool(true));
        assert_eq!(IsfValue::coerce_bool(Some(&zero)), IsfValue::Bool(false));
        assert_eq!(IsfValue::coerce_bool(Some(&text)), IsfValue::Bool(true));
        assert_eq!(IsfValue::coerce_bool(Some(&words)), IsfValue::Bool(true));
        assert_eq!(IsfValue::coerce_bool(Some(&empty)), IsfValue::Bool(false));
        assert_eq!(IsfValue::coerce_bool(Some(&nothing)), IsfValue::Bool(false));
        assert_eq!(IsfValue::coerce_bool(None), IsfValue::Bool(false));

        // The document default path agrees.
        let document = doc(
            r#"/*{ "INPUTS": [ { "NAME": "on", "TYPE": "bool", "DEFAULT": 1 } ] }*/
            void main() { gl_FragColor = vec4(on ? 1.0 : 0.0); }"#,
        );
        assert_eq!(
            default_values(&document).get("on"),
            Some(&IsfValue::Bool(true))
        );
    }

    #[test]
    fn pass_sizes_clamp_to_the_device_limit_and_see_runtime_scalars() {
        let Some(mut runtime) = try_runtime() else {
            eprintln!("skipping ISF size clamp test: no adapter");
            return;
        };
        let shader = r#"/*{
            "PASSES": [
                { "TARGET": "huge", "WIDTH": "$WIDTH * 1000000", "HEIGHT": "$HEIGHT * 1000000" },
                { "TARGET": "small", "WIDTH": "floor($FRAMEINDEX * 4.0)", "HEIGHT": "$PASSINDEX + 2.0" }
            ]
        }*/
        void main() { gl_FragColor = vec4(1.0); }"#;
        let document = doc(shader);
        let compiled = runtime
            .compile(&document, "Size Clamp", wgpu::TextureFormat::Rgba8Unorm)
            .expect("compiles");
        let limit = runtime.device().limits().max_texture_dimension_2d.max(1);
        assert_eq!(compiled.max_texture_dimension, limit);
        assert_eq!(
            compiled
                .evaluate_pass_size(0, [64, 64], IsfFrameUniforms::default(), &IsfValues::new(),)
                .expect("evaluates"),
            Some([limit, limit]),
            "a gigapixel formula clamps to max_texture_dimension_2d"
        );
        let uniforms = IsfFrameUniforms {
            frame_index: 3,
            ..IsfFrameUniforms::default()
        };
        assert_eq!(
            compiled
                .evaluate_pass_size(1, [64, 64], uniforms, &IsfValues::new())
                .expect("evaluates"),
            Some([12, 3]),
            "$FRAMEINDEX and $PASSINDEX are substituted"
        );
    }
}
