//! The canvas-2D-like facade implemented on wgpu.
//!
//! See [`crate::two_d`] for the design rationale. This file owns the GPU side:
//! an RGBA8 canvas texture, a reusable 4× MSAA target, a snapshot texture for
//! self-sampling, the four pipelines (solid, replace, image, mask) and the
//! host-side command list that [`Canvas::submit`] flushes.
//!
//! # Public surface
//!
//! The 22 canvas operations measured by the plan:
//!
//! ```text
//! arc, beginPath, clearRect, clip, closePath, drawImage, fill, fillRect,
//! fillStyle, fillText, globalAlpha, imageSmoothingEnabled, lineTo, lineWidth,
//! moveTo, restore, rotate, save, stroke, strokeStyle, translate
//! ```
//!
//! Mapped to Rust as [`Canvas::arc`], [`Canvas::begin_path`],
//! [`Canvas::clear_rect`], [`Canvas::clip`], [`Canvas::close_path`],
//! [`Canvas::draw_image`], [`Canvas::fill`], [`Canvas::fill_rect`],
//! [`Canvas::set_fill_style`], [`Canvas::fill_text`],
//! [`Canvas::set_global_alpha`], [`Canvas::set_image_smoothing`],
//! [`Canvas::line_to`], [`Canvas::set_line_width`], [`Canvas::move_to`],
//! [`Canvas::restore`], [`Canvas::rotate`], [`Canvas::save`],
//! [`Canvas::stroke`], [`Canvas::set_stroke_style`], [`Canvas::translate`],
//! plus `canvas` itself ([`Canvas::width`]/[`Canvas::height`]).
//!
//! # Additions forced by the Rust boundary
//!
//! * A `wgpu::TextureView` carries no dimensions, so the 9-argument
//!   `drawImage(source, sx, sy, sw, sh, dx, dy, dw, dh)` cannot express a
//!   *pixel* source rectangle. [`Canvas::draw_image_region`] takes the source
//!   rectangle in **normalised** `0..1` coordinates; the ported modules
//!   convert their pixel rects with the source size they already know.
//! * `drawImage(canvas, …)` (self-sampling: `Smear`, `Pixelate`,
//!   `GridStretch`, `Buffer`) is inexpressible as `&mut self` + `&self.view()`
//!   borrow, and sampling the resolve target in the same pass is a wgpu
//!   validation error. [`Canvas::draw_image_self`] /
//!   [`Canvas::draw_image_self_region`] take a snapshot of the canvas at that
//!   point in the command list (see `submit`); a module that copies the canvas
//!   into its own scratch ([`Canvas::snapshot`]) samples the snapshot of the
//!   last submitted frame instead of the live target.
//! * Text needs more state than colours: [`Canvas::set_text_size`],
//!   [`Canvas::set_text_family`], [`Canvas::set_text_bold`],
//!   [`Canvas::set_text_align`], [`Canvas::set_text_baseline`],
//!   [`Canvas::stroke_text`] and [`Canvas::text_block_height`] mirror
//!   `canvas-text-wrapper`'s configuration. These are not part of the 22-op
//!   surface: the JS sets them through `context.font`/`textAlign` and the
//!   wrapper computes `textBlockHeight` itself.
//! * [`Canvas::clip`] accepts both circles and rectangles (see
//!   `path::ClipShape`).

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::two_d::path::{
    Affine, Circle, Path, rect_geometry, stroke_geometry, transformed_bounds, triangulate_subpath,
};
use crate::two_d::text::{RasterisedText, TextEngine, TextStyle};

pub use crate::two_d::path::ClipShape;
pub use crate::two_d::text::{TextAlign, TextBaseline};

/// MSAA sample count for every 2D draw.
const SAMPLE_COUNT: u32 = 4;

/// Dynamic uniform offset/size alignment (`minUniformBufferOffsetAlignment`
/// is 256 on every native backend).
const UNIFORM_STRIDE: u32 = 256;

/// Size of one [`ClipUniform`] in bytes (48, padded to the 16-byte uniform
/// struct alignment).
const CLIP_UNIFORM_BYTES: u64 = 48;

/// Straight-alpha RGBA colour.
pub type Rgba = [f32; 4];

/// The built-in WGSL for every 2D pipeline.
///
/// Three fragment entry points share one vertex shader:
///
/// * `fs_solid` — vertex colour only (`fill`, `fillRect`, `stroke`, blit
///   replacement for `clearRect` with blending disabled).
/// * `fs_image` — samples a premultiplied RGBA source and scales it by the
///   vertex alpha (`drawImage`, canvas blit, self snapshots).
/// * `fs_mask` — samples an R8 coverage mask and tints it (`fillText`,
///   `strokeText`).
///
/// Clipping is analytic in the fragment shader (axis-aligned NDC rect or an
/// NDC ellipse, which is what a canvas-space circle becomes under the
/// non-uniform NDC mapping).
const CANVAS_WGSL: &str = r#"
struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) ndc: vec2<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.ndc = in.position;
    return out;
}

struct ClipUniform {
    rect: vec4<f32>,
    circle: vec4<f32>,
    kind: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> clip: ClipUniform;

fn clipped(p: vec2<f32>) -> bool {
    // `kind` is a bitmask: 1 = rect, 2 = circle. Canvas `clip()` intersects
    // with the current region, so both tests may be active at once.
    if (clip.kind & 1u) != 0u {
        if (p.x < clip.rect.x || p.y < clip.rect.y || p.x > clip.rect.z || p.y > clip.rect.w) {
            return true;
        }
    }
    if (clip.kind & 2u) != 0u {
        let radius = max(clip.circle.zw, vec2<f32>(1e-6, 1e-6));
        let d = (p - clip.circle.xy) / radius;
        if (dot(d, d) > 1.0) {
            return true;
        }
    }
    return false;
}

@fragment
fn fs_solid(in: VertexOut) -> @location(0) vec4<f32> {
    if (clipped(in.ndc)) {
        discard;
    }
    let c = in.color;
    return vec4<f32>(c.rgb * c.a, c.a);
}

@fragment
fn fs_image(in: VertexOut) -> @location(0) vec4<f32> {
    let s = textureSample(source_texture, source_sampler, in.uv);
    if (clipped(in.ndc)) {
        discard;
    }
    let a = s.a * in.color.a;
    return vec4<f32>(s.rgb * in.color.a, a);
}

@fragment
fn fs_mask(in: VertexOut) -> @location(0) vec4<f32> {
    let m = textureSample(source_texture, source_sampler, in.uv).r * in.color.a;
    if (clipped(in.ndc)) {
        discard;
    }
    return vec4<f32>(in.color.rgb * m, m);
}
"#;

/// One triangle-list vertex: NDC position, texture coordinates and a
/// straight-alpha colour.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Vertex {
    /// Clip-space position (already converted from canvas pixels).
    position: [f32; 2],
    /// Texture coordinates (`0,0` for solid geometry).
    uv: [f32; 2],
    /// Straight-alpha colour; the fragment shaders premultiply on output.
    color: [f32; 4],
}

impl Vertex {
    fn new(position: [f32; 2], uv: [f32; 2], color: [f32; 4]) -> Self {
        Self {
            position,
            uv,
            color,
        }
    }
}

/// Per-command clip uniform, in NDC.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct ClipUniform {
    /// `[min_x, min_y, max_x, max_y]` when bit 0 of `kind` is set.
    rect: [f32; 4],
    /// `[cx, cy, radius_x, radius_y]` when bit 1 of `kind` is set.
    circle: [f32; 4],
    /// Bitmask: 0 = no clip, 1 = rect, 2 = ellipse, 3 = both.
    kind: u32,
    /// Padding to 16-byte alignment.
    pad: [u32; 3],
}

impl ClipUniform {
    const NONE: Self = Self {
        rect: [0.0; 4],
        circle: [0.0; 4],
        kind: 0,
        pad: [0; 3],
    };
}

/// The accumulated clip region.
///
/// Canvas 2D's `clip()` **intersects** with the current clip, so the facade
/// keeps the tightest rectangle and the tightest circle rather than replacing
/// the previous shape. The shader tests the rectangle and the circle together
/// (see `ClipUniform::kind`); a second circle is approximated by the smaller
/// one, which is exact for the nested circles `clip()` normally produces.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ClipState {
    /// Intersection of every rectangular clip, if any.
    rect: Option<[f32; 4]>,
    /// The tightest circular clip, if any.
    circle: Option<Circle>,
}

impl ClipState {
    /// Intersect with an axis-aligned rectangle.
    fn intersect_rect(&mut self, [x0, y0, x1, y1]: [f32; 4]) {
        self.rect = Some(match self.rect {
            Some([ax0, ay0, ax1, ay1]) => [ax0.max(x0), ay0.max(y0), ax1.min(x1), ay1.min(y1)],
            None => [x0, y0, x1, y1],
        });
    }

    /// Intersect with a circle. Its bounding box joins the rect intersection,
    /// so a later rectangular clip still bounds the region.
    fn intersect_circle(&mut self, circle: Circle) {
        self.intersect_rect([
            circle.cx - circle.r,
            circle.cy - circle.r,
            circle.cx + circle.r,
            circle.cy + circle.r,
        ]);
        self.circle = Some(match self.circle {
            Some(previous) if previous.r <= circle.r => previous,
            _ => circle,
        });
    }

    /// The current region as a single [`ClipShape`], for diagnostics/tests.
    ///
    /// Returns the circle when both a circle and a rect are active; the rect
    /// is its bounding box in that case (it never clips more than the circle).
    fn shape(&self) -> Option<ClipShape> {
        if let Some(circle) = self.circle {
            Some(ClipShape::Circle {
                cx: circle.cx,
                cy: circle.cy,
                r: circle.r.abs(),
            })
        } else {
            self.rect
                .map(|[x0, y0, x1, y1]| ClipShape::Rect { x0, y0, x1, y1 })
        }
    }
}

/// Which pipeline a command uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandKind {
    /// Solid geometry, alpha blended.
    Solid,
    /// Solid geometry, straight replacement (no blending).
    Replace,
    /// Textured quad sampling a premultiplied RGBA source.
    Image,
    /// Textured quad sampling an R8 coverage mask.
    Mask,
}

/// Where a command samples from.
#[derive(Clone)]
enum CommandSource {
    /// Nothing (solid geometry binds the 1×1 white texture).
    White,
    /// An external view supplied by a module/engine.
    External(wgpu::TextureView),
    /// The canvas' own content at this point in the command list.
    SelfCanvas,
    /// A rasterised text mask.
    Mask(wgpu::TextureView),
}

/// One recorded draw: vertices plus the pipeline/source/clip it needs.
struct Command {
    kind: CommandKind,
    source: CommandSource,
    clip: ClipUniform,
    /// Whether the source sampler is linear (canvas `imageSmoothingEnabled`).
    smooth: bool,
    vertices: Vec<Vertex>,
}

/// The canvas style/transform stack entry (`save`/`restore`).
#[derive(Clone, Debug)]
struct State {
    transform: Affine,
    fill: Rgba,
    stroke: Rgba,
    line_width: f32,
    global_alpha: f32,
    smoothing: bool,
    clip: ClipState,
    text: TextStyle,
}

impl Default for State {
    fn default() -> Self {
        Self {
            transform: Affine::IDENTITY,
            fill: [0.0, 0.0, 0.0, 1.0],
            stroke: [0.0, 0.0, 0.0, 1.0],
            line_width: 1.0,
            global_alpha: 1.0,
            smoothing: true,
            clip: ClipState::default(),
            text: TextStyle::default(),
        }
    }
}

/// The 2D canvas facade.
///
/// Created from a device/queue pair, owns its target texture and all GPU state.
/// Record draw calls between [`Canvas::begin_frame`] and [`Canvas::submit`].
pub struct Canvas {
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,

    texture: wgpu::Texture,
    view: wgpu::TextureView,
    msaa_texture: wgpu::Texture,
    msaa_view: wgpu::TextureView,
    snapshot_texture: wgpu::Texture,
    snapshot_view: wgpu::TextureView,

    bind_group_layout: wgpu::BindGroupLayout,
    solid_pipeline: wgpu::RenderPipeline,
    replace_pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
    mask_pipeline: wgpu::RenderPipeline,

    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    uniform_buffer: wgpu::Buffer,
    uniform_capacity: u64,

    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,

    /// Kept alive so the 1×1 white view below stays valid.
    _white_texture: wgpu::Texture,
    white_view: wgpu::TextureView,

    commands: Vec<Command>,
    path: Path,
    state: State,
    stack: Vec<State>,
    pending_clear_all: bool,

    text: Option<TextEngine>,
    text_unavailable: bool,
}

impl Canvas {
    /// Create a canvas of `width × height` pixels.
    ///
    /// Panics when a dimension is zero (`wgpu` rejects zero-sized textures);
    /// the engine clamps the group size before calling this.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        assert!(width > 0 && height > 0, "canvas size must be non-zero");

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("modV 2d canvas bind group layout"),
            entries: &bind_group_layout_entries(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("modV 2d canvas pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let (solid_pipeline, replace_pipeline, image_pipeline, mask_pipeline) =
            create_pipelines(device, &pipeline_layout);

        let (texture, view, msaa_texture, msaa_view, snapshot_texture, snapshot_view) =
            create_targets(device, width, height);
        // wgpu does not zero-initialise allocations: without this the first
        // frame samples undefined pixels (and a self-sampling module reads
        // garbage). The JS canvas starts transparent black.
        zero_fill_texture(queue, &texture, width, height);
        zero_fill_texture(queue, &snapshot_texture, width, height);

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modV 2d canvas linear sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("modV 2d canvas nearest sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let white_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("modV 2d canvas white"),
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
            white_texture.as_image_copy(),
            &[255u8, 255, 255, 255],
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
        let white_view = white_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modV 2d canvas vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modV 2d canvas clip uniforms"),
            size: UNIFORM_STRIDE as u64 * 4,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            device: device.clone(),
            queue: queue.clone(),
            width,
            height,
            texture,
            view,
            msaa_texture,
            msaa_view,
            snapshot_texture,
            snapshot_view,
            bind_group_layout,
            solid_pipeline,
            replace_pipeline,
            image_pipeline,
            mask_pipeline,
            vertex_buffer,
            vertex_capacity: 4096,
            uniform_buffer,
            uniform_capacity: UNIFORM_STRIDE as u64 * 4,
            linear_sampler,
            nearest_sampler,
            _white_texture: white_texture,
            white_view,
            commands: Vec::new(),
            path: Path::default(),
            state: State::default(),
            stack: Vec::new(),
            pending_clear_all: false,
            text: None,
            text_unavailable: false,
        }
    }

    /// Canvas width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Canvas height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The canvas texture (premultiplied RGBA8), for the frame graph.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// The canvas view, for the frame graph / `drawImage` from another canvas.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Snapshot the canvas content as of its **last `submit`** and return a
    /// view of it.
    ///
    /// This is the cross-canvas half of the self-sampling path. A module that
    /// copies the canvas into its own scratch ([`Canvas::draw_image`] with the
    /// canvas as the source) cannot bind [`Canvas::view`]: that texture is the
    /// one the group writes this frame, so sampling it is undefined. It samples
    /// this snapshot instead — frame N therefore reads frame N-1's content,
    /// never the live target (the `drawImage(canvas, …)` semantics, like
    /// [`Canvas::draw_image_self`]). Pending commands are deliberately not
    /// flushed; the snapshot is a copy of the last submitted texture.
    pub fn snapshot(&mut self) -> wgpu::TextureView {
        let extent = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("modV 2d canvas snapshot"),
            });
        encoder.copy_texture_to_texture(
            self.texture.as_image_copy(),
            self.snapshot_texture.as_image_copy(),
            extent,
        );
        self.queue.submit(Some(encoder.finish()));
        self.snapshot_view.clone()
    }

    /// Whether font discovery failed, in which case text silently does not
    /// draw. Tests use this to skip the text assertions on fontless systems.
    pub fn text_unavailable(&self) -> bool {
        self.text_unavailable
    }

    /// Resize the canvas. Content is discarded, like assigning to
    /// `canvas.width` in JS.
    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        assert!(width > 0 && height > 0, "canvas size must be non-zero");
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        let (texture, view, msaa_texture, msaa_view, snapshot_texture, snapshot_view) =
            create_targets(device, width, height);
        // A resized canvas is a fresh allocation: zero it so the next frame
        // cannot sample undefined pixels (`canvas.width = …` clears in JS).
        zero_fill_texture(queue, &texture, width, height);
        zero_fill_texture(queue, &snapshot_texture, width, height);
        self.texture = texture;
        self.view = view;
        self.msaa_texture = msaa_texture;
        self.msaa_view = msaa_view;
        self.snapshot_texture = snapshot_texture;
        self.snapshot_view = snapshot_view;
        self.commands.clear();
        self.path.clear();
        self.stack.clear();
        self.state = State::default();
        self.pending_clear_all = true;
    }

    /// Reset per-frame path/style state.
    ///
    /// Canvas content persists unless [`Canvas::clear_all`]/[`Canvas::clear_rect`]
    /// is called — the JS renderer wraps `module.draw` in
    /// `context.save()`/`context.restore()` instead, which this replaces with a
    /// full state reset.
    pub fn begin_frame(&mut self) {
        self.path.clear();
        self.stack.clear();
        self.state = State::default();
        self.commands.clear();
        self.pending_clear_all = false;
    }

    /// Clear the whole canvas to transparent black.
    ///
    /// Earlier recorded draws are dropped: they would be invisible anyway.
    pub fn clear_all(&mut self) {
        self.commands.clear();
        self.pending_clear_all = true;
    }

    /// Canvas `clearRect`.
    ///
    /// Erases as transparent black with a `Replace` draw (not by dropping the
    /// commands recorded so far this frame), so earlier draws survive outside
    /// the rect and the current clip still applies.
    pub fn clear_rect(&mut self, x: f32, y: f32, width: f32, height: f32) {
        if !(width > 0.0) || !(height > 0.0) {
            return;
        }
        self.push_solid_rect(
            x,
            y,
            width,
            height,
            [0.0, 0.0, 0.0, 0.0],
            CommandKind::Replace,
        );
    }

    /// Push the current state (canvas `save`).
    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
    }

    /// Pop the last saved state (canvas `restore`). A no-op on an empty stack.
    pub fn restore(&mut self) {
        if let Some(state) = self.stack.pop() {
            self.state = state;
        }
    }

    /// Canvas `translate`: the translation acts in the current space.
    pub fn translate(&mut self, x: f32, y: f32) {
        self.state.transform = self.state.transform.multiply(&Affine::translate(x, y));
    }

    /// Canvas `rotate` (radians).
    pub fn rotate(&mut self, radians: f32) {
        self.state.transform = self.state.transform.multiply(&Affine::rotate(radians));
    }

    /// Canvas `moveTo`: begins a new subpath.
    pub fn move_to(&mut self, x: f32, y: f32) {
        let point = self.state.transform.transform_point([x, y]);
        self.path.move_to(point);
    }

    /// Canvas `lineTo`: appends to the current subpath.
    pub fn line_to(&mut self, x: f32, y: f32) {
        let point = self.state.transform.transform_point([x, y]);
        self.path.line_to(point);
    }

    /// Canvas `arc`.
    ///
    /// Canvas' implicit `lineTo` from the current point is honoured; a
    /// standalone full-turn arc marks the subpath as a circle, enabling the
    /// exact fan fill and the analytic circular clip (`X-Drips.js:81-88`).
    pub fn arc(&mut self, x: f32, y: f32, radius: f32, start: f32, end: f32, ccw: bool) {
        let radius = radius.abs();
        if radius == 0.0 {
            return;
        }
        let transform = self.state.transform;
        let was_empty = self.path.last_subpath_len() == 0;
        let start_point =
            transform.transform_point([x + radius * start.cos(), y + radius * start.sin()]);
        if was_empty {
            self.path.move_to(start_point);
        } else {
            self.path.line_to(start_point);
        }
        for point in crate::two_d::path::flatten_arc(x, y, radius, start, end, ccw) {
            self.path.line_to(transform.transform_point(point));
        }
        if was_empty && crate::two_d::path::arc_is_full_turn(start, end, ccw) {
            let centre = transform.transform_point([x, y]);
            self.path.set_last_circle(crate::two_d::path::Circle {
                cx: centre[0],
                cy: centre[1],
                r: radius * transform.scale_factor(),
            });
        }
    }

    /// Canvas `closePath`.
    pub fn close_path(&mut self) {
        self.path.close();
    }

    /// Canvas `beginPath`.
    pub fn begin_path(&mut self) {
        self.path.clear();
    }

    /// Canvas `clip`, using the current path.
    ///
    /// A single full-turn arc clips to its circle (analytic, exact); every
    /// other path clips to its transformed axis-aligned bounds, as the plan
    /// specifies for the rect-only minimum. Like Canvas2D, the new shape
    /// **intersects** with the current clip region instead of replacing it.
    pub fn clip(&mut self) {
        if let Some(circle) = self.path.sole_circle() {
            self.state.clip.intersect_circle(Circle {
                cx: circle.cx,
                cy: circle.cy,
                r: circle.r.abs(),
            });
        } else if let Some([x0, y0, x1, y1]) = self.path.bounds() {
            self.state.clip.intersect_rect([x0, y0, x1, y1]);
        }
    }

    /// Rectangular clip (the plan's minimum clip shape), intersected with the
    /// current region like [`Canvas::clip`].
    pub fn clip_rect(&mut self, x: f32, y: f32, width: f32, height: f32) {
        let [x0, y0, x1, y1] = transformed_bounds(x, y, width, height, &self.state.transform);
        self.state.clip.intersect_rect([x0, y0, x1, y1]);
    }

    /// Canvas `fill` — fills the current path.
    pub fn fill(&mut self) {
        let colour = self.effective(self.state.fill);
        let mut vertices = Vec::new();
        for subpath in &self.path.subpaths {
            for point in triangulate_subpath(subpath) {
                vertices.push(self.to_ndc(point));
            }
        }
        self.push_triangles(&vertices, colour, CommandKind::Solid, CommandSource::White);
    }

    /// Canvas `stroke` — strokes the current path.
    pub fn stroke(&mut self) {
        let colour = self.effective(self.state.stroke);
        let half_width =
            (self.state.line_width.max(0.0) / 2.0) * self.state.transform.scale_factor();
        if half_width <= 0.0 {
            return;
        }
        let mut vertices = Vec::new();
        for subpath in &self.path.subpaths {
            for point in stroke_geometry(&subpath.points, subpath.closed, half_width) {
                vertices.push(self.to_ndc(point));
            }
        }
        self.push_triangles(&vertices, colour, CommandKind::Solid, CommandSource::White);
    }

    /// Canvas `fillRect`.
    pub fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32) {
        let colour = self.effective(self.state.fill);
        self.push_solid_rect(x, y, width, height, colour, CommandKind::Solid);
    }

    /// Record an axis-aligned-in-user-space rect as two transformed triangles.
    fn push_solid_rect(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        colour: Rgba,
        kind: CommandKind,
    ) {
        let points = rect_geometry(x, y, width, height, &self.state.transform);
        if points.is_empty() {
            return;
        }
        let vertices: Vec<[f32; 2]> = points.into_iter().map(|p| self.to_ndc(p)).collect();
        self.push_triangles(&vertices, colour, kind, CommandSource::White);
    }

    /// Canvas `drawImage(source, dx, dy, dw, dh)` — the whole source.
    pub fn draw_image(&mut self, source: &wgpu::TextureView, dx: f32, dy: f32, dw: f32, dh: f32) {
        self.draw_image_region(source, 0.0, 0.0, 1.0, 1.0, dx, dy, dw, dh);
    }

    /// Canvas `drawImage(source, sx, sy, sw, sh, dx, dy, dw, dh)` with the
    /// source rectangle in **normalised** `0..1` coordinates.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_image_region(
        &mut self,
        source: &wgpu::TextureView,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    ) {
        let quad = self.image_quad([sx, sy], [sx + sw, sy + sh], dx, dy, dw, dh);
        let Some(quad) = quad else {
            return;
        };
        self.push_textured_quad(quad, CommandSource::External(source.clone()));
    }

    /// `drawImage(canvas, dx, dy, dw, dh)` — samples the canvas' own content
    /// as of this command.
    pub fn draw_image_self(&mut self, dx: f32, dy: f32, dw: f32, dh: f32) {
        self.draw_image_self_region(0.0, 0.0, 1.0, 1.0, dx, dy, dw, dh);
    }

    /// 9-argument `drawImage(canvas, …)` with a **normalised** source rect.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_image_self_region(
        &mut self,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    ) {
        let quad = self.image_quad([sx, sy], [sx + sw, sy + sh], dx, dy, dw, dh);
        let Some(quad) = quad else {
            return;
        };
        self.push_textured_quad(quad, CommandSource::SelfCanvas);
    }

    /// Build the four transformed corners of a textured quad plus its UVs.
    ///
    /// Returns `[position, uv; 4]` in draw order (top-left, top-right,
    /// bottom-right, bottom-left).
    fn image_quad(
        &self,
        uv0: [f32; 2],
        uv1: [f32; 2],
        dx: f32,
        dy: f32,
        dw: f32,
        dh: f32,
    ) -> Option<[([f32; 2], [f32; 2]); 4]> {
        if !(dw > 0.0) || !(dh > 0.0) {
            return None;
        }
        let transform = &self.state.transform;
        let corners = [
            (transform.transform_point([dx, dy]), [uv0[0], uv0[1]]),
            (transform.transform_point([dx + dw, dy]), [uv1[0], uv0[1]]),
            (
                transform.transform_point([dx + dw, dy + dh]),
                [uv1[0], uv1[1]],
            ),
            (transform.transform_point([dx, dy + dh]), [uv0[0], uv1[1]]),
        ];
        Some(corners)
    }

    /// Record a textured quad (two triangles) with the current alpha/clip.
    fn push_textured_quad(&mut self, quad: [([f32; 2], [f32; 2]); 4], source: CommandSource) {
        let alpha = self.clamped_alpha(self.state.global_alpha);
        let colour = [1.0, 1.0, 1.0, alpha];
        let positions: Vec<[f32; 2]> = quad.iter().map(|(p, _)| self.to_ndc(*p)).collect();
        let uvs: Vec<[f32; 2]> = quad.iter().map(|(_, uv)| *uv).collect();
        let vertices = vec![
            Vertex::new(positions[0], uvs[0], colour),
            Vertex::new(positions[1], uvs[1], colour),
            Vertex::new(positions[2], uvs[2], colour),
            Vertex::new(positions[0], uvs[0], colour),
            Vertex::new(positions[2], uvs[2], colour),
            Vertex::new(positions[3], uvs[3], colour),
        ];
        self.push_command(CommandKind::Image, source, vertices);
    }

    /// Canvas `fillText`.
    ///
    /// `max_width` enables word wrapping (`canvas-text-wrapper`'s
    /// `MAX_TXT_WIDTH` behaviour); `None` lays out a single line.
    pub fn fill_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) {
        let raster = self.rasterise_text(text, max_width, x, y);
        let Some(raster) = raster else {
            return;
        };
        let colour = self.effective(self.state.fill);
        self.record_mask(&raster, colour, [0.0, 0.0]);
    }

    /// Canvas `strokeText`.
    ///
    /// The glyph coverage mask is stamped around a ring of radius
    /// `lineWidth / 2` behind the fill, approximating canvas' stroked outline
    /// without a second rasteriser. `Text.js:176-183` only enables it when the
    /// `stroke` prop is on.
    pub fn stroke_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) {
        let width = self.state.line_width.max(0.0);
        if width <= 0.0 {
            return;
        }
        let raster = self.rasterise_text(text, max_width, x, y);
        let Some(raster) = raster else {
            return;
        };
        let colour = self.effective(self.state.stroke);
        let radius = width / 2.0;
        let stamps = 12;
        for index in 0..stamps {
            let angle = index as f32 / stamps as f32 * std::f32::consts::TAU;
            self.record_mask(
                &raster,
                colour,
                [angle.cos() * radius, angle.sin() * radius],
            );
        }
    }

    /// Height of the text block that [`Canvas::fill_text`] would draw
    /// (`canvas-text-wrapper`'s `textBlockHeight`), for vertical centring.
    pub fn text_block_height(&mut self, text: &str, max_width: Option<f32>) -> f32 {
        let style = self.state.text.clone();
        if !self.ensure_text_engine() {
            self.text_unavailable = true;
            return 0.0;
        }
        match self.text.as_mut() {
            Some(engine) => engine.block_height(text, max_width, &style),
            None => 0.0,
        }
    }

    fn ensure_text_engine(&mut self) -> bool {
        if self.text.is_none() {
            self.text = TextEngine::new();
        }
        self.text.is_some()
    }

    fn rasterise_text(
        &mut self,
        text: &str,
        max_width: Option<f32>,
        x: f32,
        y: f32,
    ) -> Option<RasterisedText> {
        let style = self.state.text.clone();
        if text.trim().is_empty() {
            return None;
        }
        if !self.ensure_text_engine() {
            self.text_unavailable = true;
            return None;
        }
        self.text
            .as_mut()
            .and_then(|engine| engine.rasterise(text, max_width, &style, x, y))
    }

    /// Upload a rasterised text block as an R8 mask and record its quad(s).
    fn record_mask(&mut self, raster: &RasterisedText, colour: Rgba, offset: [f32; 2]) {
        let Some(view) = self.upload_mask(raster) else {
            return;
        };
        let origin = [raster.origin[0] + offset[0], raster.origin[1] + offset[1]];
        let quad = self.image_quad(
            [0.0, 0.0],
            [1.0, 1.0],
            origin[0],
            origin[1],
            raster.width as f32,
            raster.height as f32,
        );
        let Some(quad) = quad else {
            return;
        };
        let positions: Vec<[f32; 2]> = quad.iter().map(|(p, _)| self.to_ndc(*p)).collect();
        let uvs: Vec<[f32; 2]> = quad.iter().map(|(_, uv)| *uv).collect();
        let vertices = vec![
            Vertex::new(positions[0], uvs[0], colour),
            Vertex::new(positions[1], uvs[1], colour),
            Vertex::new(positions[2], uvs[2], colour),
            Vertex::new(positions[0], uvs[0], colour),
            Vertex::new(positions[2], uvs[2], colour),
            Vertex::new(positions[3], uvs[3], colour),
        ];
        self.push_command(CommandKind::Mask, CommandSource::Mask(view), vertices);
    }

    /// Create an R8 texture holding the mask and return its view.
    fn upload_mask(&self, raster: &RasterisedText) -> Option<wgpu::TextureView> {
        if raster.width == 0 || raster.height == 0 {
            return None;
        }
        let unpadded = raster.width as usize;
        let align = 256usize;
        let padded = unpadded.div_ceil(align) * align;
        let mut data = vec![0u8; padded * raster.height as usize];
        for row in 0..raster.height as usize {
            let source = &raster.pixels[row * unpadded..(row + 1) * unpadded];
            let target = &mut data[row * padded..row * padded + unpadded];
            target.copy_from_slice(source);
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("modV 2d text mask"),
            size: wgpu::Extent3d {
                width: raster.width,
                height: raster.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(raster.height),
            },
            wgpu::Extent3d {
                width: raster.width,
                height: raster.height,
                depth_or_array_layers: 1,
            },
        );
        Some(texture.create_view(&wgpu::TextureViewDescriptor::default()))
    }

    /// Canvas `fillStyle` (straight-alpha RGBA).
    pub fn set_fill_style(&mut self, rgba: Rgba) {
        self.state.fill = clamp_colour(rgba);
    }

    /// Canvas `strokeStyle` (straight-alpha RGBA).
    pub fn set_stroke_style(&mut self, rgba: Rgba) {
        self.state.stroke = clamp_colour(rgba);
    }

    /// Canvas `lineWidth`.
    pub fn set_line_width(&mut self, width: f32) {
        if width.is_finite() && width > 0.0 {
            self.state.line_width = width;
        }
    }

    /// Canvas `globalAlpha`.
    pub fn set_global_alpha(&mut self, alpha: f32) {
        self.state.global_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Canvas `imageSmoothingEnabled`.
    pub fn set_image_smoothing(&mut self, enabled: bool) {
        self.state.smoothing = enabled;
    }

    /// `context.font`'s size in pixels.
    pub fn set_text_size(&mut self, size: f32) {
        if size.is_finite() && size > 0.0 {
            self.state.text.size = size;
        }
    }

    /// `context.font`'s family (`"sans-serif"`, `"serif"`, `"monospace"`, or a
    /// font name).
    pub fn set_text_family(&mut self, family: &str) {
        self.state.text.family = family.to_owned();
    }

    /// `context.font`'s weight (`bold` vs. normal).
    pub fn set_text_bold(&mut self, bold: bool) {
        self.state.text.bold = bold;
    }

    /// `context.textAlign`.
    pub fn set_text_align(&mut self, align: TextAlign) {
        self.state.text.align = align;
    }

    /// `context.textBaseline`.
    pub fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.state.text.baseline = baseline;
    }

    /// Mirror of `context.font = "<weight> <size>px <family>"` parsing, for
    /// `Text.js:176-183`.
    pub fn set_font(&mut self, weight: &str, size: f32, family: &str) {
        self.set_text_bold(
            weight.eq_ignore_ascii_case("bold") || weight.parse::<u32>().is_ok_and(|w| w >= 600),
        );
        self.set_text_size(size);
        self.set_text_family(family);
    }

    /// The current text style (diagnostics/tests).
    pub fn text_style(&self) -> &TextStyle {
        &self.state.text
    }

    /// The current fill style.
    pub fn fill_style(&self) -> Rgba {
        self.state.fill
    }

    /// The current stroke style.
    pub fn stroke_style(&self) -> Rgba {
        self.state.stroke
    }

    /// The current line width.
    pub fn line_width(&self) -> f32 {
        self.state.line_width
    }

    /// The current global alpha.
    pub fn global_alpha(&self) -> f32 {
        self.state.global_alpha
    }

    /// The current clip shape, if any.
    pub fn clip_shape(&self) -> Option<ClipShape> {
        self.state.clip.shape()
    }

    fn effective(&self, colour: Rgba) -> Rgba {
        let mut out = colour;
        out[3] = self.clamped_alpha(colour[3] * self.state.global_alpha);
        clamp_colour(out)
    }

    fn clamped_alpha(&self, alpha: f32) -> f32 {
        if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Canvas pixels → normalised device coordinates (y flipped).
    fn to_ndc(&self, point: [f32; 2]) -> [f32; 2] {
        [
            point[0] / self.width as f32 * 2.0 - 1.0,
            1.0 - point[1] / self.height as f32 * 2.0,
        ]
    }

    fn clip_uniform(&self) -> ClipUniform {
        let mut uniform = ClipUniform::NONE;
        if let Some([x0, y0, x1, y1]) = self.state.clip.rect {
            let ax = self.to_ndc([x0, y0]);
            let bx = self.to_ndc([x1, y1]);
            uniform.rect = [
                ax[0].min(bx[0]),
                ax[1].min(bx[1]),
                ax[0].max(bx[0]),
                ax[1].max(bx[1]),
            ];
            uniform.kind |= 1;
        }
        if let Some(circle) = self.state.clip.circle {
            let centre = self.to_ndc([circle.cx, circle.cy]);
            uniform.circle = [
                centre[0],
                centre[1],
                (circle.r.abs() / self.width as f32) * 2.0,
                (circle.r.abs() / self.height as f32) * 2.0,
            ];
            uniform.kind |= 2;
        }
        uniform
    }

    fn push_triangles(
        &mut self,
        positions: &[[f32; 2]],
        colour: Rgba,
        kind: CommandKind,
        source: CommandSource,
    ) {
        if positions.is_empty() {
            return;
        }
        let vertices: Vec<Vertex> = positions
            .iter()
            .map(|position| Vertex::new(*position, [0.0, 0.0], colour))
            .collect();
        self.push_command(kind, source, vertices);
    }

    fn push_command(&mut self, kind: CommandKind, source: CommandSource, vertices: Vec<Vertex>) {
        if vertices.is_empty() {
            return;
        }
        self.commands.push(Command {
            kind,
            source,
            clip: self.clip_uniform(),
            smooth: self.state.smoothing,
            vertices,
        });
    }

    /// Flush the recorded commands to the GPU.
    ///
    /// Encoding:
    ///
    /// 1. Concatenate every command's vertices into one buffer and every
    ///    clip uniform into one dynamic-offset buffer.
    /// 2. If the canvas has prior content, blit it into the MSAA target
    ///    (a separate pass, because the canvas is sampled there and resolved
    ///    into later).
    /// 3. Draw the commands in order into the MSAA target, resolving into the
    ///    canvas texture at the end of each pass.
    /// 4. A command that samples the canvas itself splits the pass: resolve,
    ///    copy the canvas into the snapshot texture, then resume with the
    ///    snapshot bound. That keeps the "read what has been drawn so far"
    ///    semantics of `drawImage(canvas, …)` without sampling the resolve
    ///    target.
    pub fn submit(&mut self) {
        if self.commands.is_empty() && !self.pending_clear_all {
            return;
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("modV 2d canvas submit"),
            });

        if self.pending_clear_all && self.commands.is_empty() {
            // Clear-only frame: a pass with no draws and `LoadOp::Clear` on
            // the canvas view. (`CommandEncoder::clear_texture` would require
            // the CLEAR_TEXTURE device feature, which the engine does not
            // request; the pass path needs no extra feature. When there are
            // draw commands the first draw pass clears instead.)
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("modV 2d canvas clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
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
        }

        // 1. Vertex + uniform buffers.
        let needs_blit = !self.pending_clear_all;
        let mut vertices: Vec<Vertex> = Vec::new();
        if needs_blit {
            vertices.extend_from_slice(&fullscreen_quad());
        }
        let blit_vertices = vertices.len() as u32;

        let mut ranges: Vec<(u32, u32)> = Vec::with_capacity(self.commands.len());
        for command in &self.commands {
            let first = vertices.len() as u32;
            vertices.extend_from_slice(&command.vertices);
            ranges.push((first, command.vertices.len() as u32));
        }

        self.ensure_vertex_capacity((vertices.len() * std::mem::size_of::<Vertex>()) as u64);
        self.queue
            .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

        let uniform_slots = self.commands.len() + 1;
        self.ensure_uniform_capacity(UNIFORM_STRIDE as u64 * uniform_slots as u64);
        let mut uniforms = vec![0u8; UNIFORM_STRIDE as usize * uniform_slots];
        uniforms[0..CLIP_UNIFORM_BYTES as usize]
            .copy_from_slice(bytemuck::bytes_of(&ClipUniform::NONE));
        for (index, command) in self.commands.iter().enumerate() {
            let offset = (index + 1) * UNIFORM_STRIDE as usize;
            uniforms[offset..offset + CLIP_UNIFORM_BYTES as usize]
                .copy_from_slice(bytemuck::bytes_of(&command.clip));
        }
        self.queue.write_buffer(&self.uniform_buffer, 0, &uniforms);

        // 2. Bind groups, one per command (they outlive the passes).
        let bind_groups: Vec<wgpu::BindGroup> = self
            .commands
            .iter()
            .enumerate()
            .map(|(_, command)| {
                let (view, sampler) = match &command.source {
                    CommandSource::White => (&self.white_view, &self.nearest_sampler),
                    CommandSource::External(view) => {
                        if command.smooth {
                            (view, &self.linear_sampler)
                        } else {
                            (view, &self.nearest_sampler)
                        }
                    }
                    CommandSource::SelfCanvas => {
                        if command.smooth {
                            (&self.snapshot_view, &self.linear_sampler)
                        } else {
                            (&self.snapshot_view, &self.nearest_sampler)
                        }
                    }
                    CommandSource::Mask(view) => {
                        if command.smooth {
                            (view, &self.linear_sampler)
                        } else {
                            (view, &self.nearest_sampler)
                        }
                    }
                };
                self.create_bind_group(view, sampler)
            })
            .collect();

        let extent = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };

        // 3. Blit prior content into the MSAA target.
        if needs_blit {
            let blit_bind_group = self.create_bind_group(&self.view, &self.nearest_sampler);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("modV 2d canvas blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa_view,
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
            pass.set_pipeline(&self.image_pipeline);
            pass.set_bind_group(0, &blit_bind_group, &[0]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..blit_vertices, 0..1);
        }

        // 4. Segment the commands: a self-sampling command gets its own
        //    segment so the snapshot copy is in submission order.
        let mut segments: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
        {
            let mut start = 0usize;
            for (index, command) in self.commands.iter().enumerate() {
                if matches!(command.source, CommandSource::SelfCanvas) {
                    if start < index {
                        segments.push((start..index, false));
                    }
                    segments.push((index..index + 1, true));
                    start = index + 1;
                }
            }
            if start < self.commands.len() {
                segments.push((start..self.commands.len(), false));
            }
        }

        let mut first_pass = true;
        for (range, needs_snapshot) in segments {
            if needs_snapshot {
                // Resolve/copy happens between passes, so the snapshot holds
                // everything drawn so far.
                encoder.copy_texture_to_texture(
                    self.texture.as_image_copy(),
                    self.snapshot_texture.as_image_copy(),
                    extent,
                );
            }
            let load = if first_pass && self.pending_clear_all {
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
            } else {
                wgpu::LoadOp::Load
            };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("modV 2d canvas draw"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.msaa_view,
                        depth_slice: None,
                        resolve_target: Some(&self.view),
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                for index in range {
                    let command = &self.commands[index];
                    pass.set_pipeline(self.pipeline_for(command.kind));
                    pass.set_bind_group(
                        0,
                        &bind_groups[index],
                        &[((index + 1) as u32) * UNIFORM_STRIDE],
                    );
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    let (first, count) = ranges[index];
                    pass.draw(first..first + count, 0..1);
                }
            }
            first_pass = false;
        }

        self.queue.submit(Some(encoder.finish()));
        self.commands.clear();
        self.path.clear();
        self.pending_clear_all = false;
    }

    fn pipeline_for(&self, kind: CommandKind) -> &wgpu::RenderPipeline {
        match kind {
            CommandKind::Solid => &self.solid_pipeline,
            CommandKind::Replace => &self.replace_pipeline,
            CommandKind::Image => &self.image_pipeline,
            CommandKind::Mask => &self.mask_pipeline,
        }
    }

    fn create_bind_group(
        &self,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("modV 2d canvas bind group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniform_buffer,
                        offset: 0,
                        size: NonZeroU64::new(CLIP_UNIFORM_BYTES),
                    }),
                },
            ],
        })
    }

    fn ensure_vertex_capacity(&mut self, bytes: u64) {
        if bytes <= self.vertex_capacity {
            return;
        }
        let capacity = bytes.next_power_of_two().max(4096);
        self.vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modV 2d canvas vertices"),
            size: capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.vertex_capacity = capacity;
    }

    fn ensure_uniform_capacity(&mut self, bytes: u64) {
        if bytes <= self.uniform_capacity {
            return;
        }
        let capacity = bytes.next_power_of_two().max(1024);
        self.uniform_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modV 2d canvas clip uniforms"),
            size: capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uniform_capacity = capacity;
    }

    /// Read the canvas texture back as tightly packed RGBA8.
    ///
    /// Test/diagnostic helper; premultiplied, matching the stored format.
    #[cfg(test)]
    pub(crate) fn read_pixels(&self) -> Vec<u8> {
        let unpadded = self.width as u64 * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
        let padded = unpadded.div_ceil(align) * align;
        let size = padded * self.height as u64;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("modV 2d canvas read back"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("modV 2d canvas read back"),
            });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        receiver.recv().expect("map callback").expect("map");

        let mapped = slice.get_mapped_range().expect("mapped range");
        let mut out = Vec::with_capacity((unpadded * self.height as u64) as usize);
        for row in 0..self.height as u64 {
            let start = (row * padded) as usize;
            let end = start + unpadded as usize;
            out.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }
}

/// Clamp colour components to `0..1`, zeroing non-finite values.
fn clamp_colour(colour: Rgba) -> Rgba {
    let mut out = colour;
    for channel in &mut out {
        *channel = if channel.is_finite() {
            channel.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    out
}

/// The two triangles of a full-screen NDC quad, sampling the whole source.
fn fullscreen_quad() -> [Vertex; 6] {
    let colour = [1.0, 1.0, 1.0, 1.0];
    let tl = Vertex::new([-1.0, 1.0], [0.0, 0.0], colour);
    let tr = Vertex::new([1.0, 1.0], [1.0, 0.0], colour);
    let br = Vertex::new([1.0, -1.0], [1.0, 1.0], colour);
    let bl = Vertex::new([-1.0, -1.0], [0.0, 1.0], colour);
    [tl, tr, br, tl, br, bl]
}

/// The fixed bind group layout: source texture, sampler and dynamic clip
/// uniform.
fn bind_group_layout_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
    use wgpu::{
        BindingType, BufferBindingType, ShaderStages, TextureSampleType, TextureViewDimension,
    };

    [
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: NonZeroU64::new(CLIP_UNIFORM_BYTES),
            },
            count: None,
        },
    ]
}

/// Create the canvas, MSAA and snapshot textures for a given size.
type Targets = (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::Texture,
    wgpu::TextureView,
);

/// Zero-fill a freshly created RGBA8 texture.
///
/// `queue.write_texture` does not require `COPY_BYTES_PER_ROW_ALIGNMENT`
/// padding (`copy_buffer_to_texture` does), but using the 256-byte stride
/// keeps the upload valid on every backend and costs at most 256 bytes per
/// row for the small canvases this facade creates.
fn zero_fill_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, width: u32, height: u32) {
    let unpadded = width as usize * 4;
    let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let zeros = vec![0u8; padded * height as usize];
    queue.write_texture(
        texture.as_image_copy(),
        &zeros,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(padded as u32),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn create_targets(device: &wgpu::Device, width: u32, height: u32) -> Targets {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let canvas = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modV 2d canvas"),
        size,
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
    let canvas_view = canvas.create_view(&wgpu::TextureViewDescriptor::default());

    let msaa = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modV 2d canvas msaa"),
        size,
        mip_level_count: 1,
        sample_count: SAMPLE_COUNT,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let msaa_view = msaa.create_view(&wgpu::TextureViewDescriptor::default());

    let snapshot = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modV 2d canvas snapshot"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let snapshot_view = snapshot.create_view(&wgpu::TextureViewDescriptor::default());

    (
        canvas,
        canvas_view,
        msaa,
        msaa_view,
        snapshot,
        snapshot_view,
    )
}

/// Build the four pipelines (solid, replace, image, mask).
fn create_pipelines(
    device: &wgpu::Device,
    pipeline_layout: &wgpu::PipelineLayout,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("modV 2d canvas shader"),
        source: wgpu::ShaderSource::Wgsl(CANVAS_WGSL.into()),
    });

    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 8,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 16,
                shader_location: 2,
            },
        ],
    };

    let premultiplied = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    };

    let build = |label: &str, entry: &str, blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(vertex_layout.clone())],
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
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };

    (
        build("modV 2d solid", "fs_solid", Some(premultiplied)),
        build("modV 2d replace", "fs_solid", None),
        build("modV 2d image", "fs_image", Some(premultiplied)),
        build("modV 2d mask", "fs_mask", Some(premultiplied)),
    )
}
