//! Point10 scenes — port of `src/renderer/src/application/point10/`.
//! See docs/rust-hybrid.md, "WS-7 Point10 scenes".
//!
//! # Layout
//!
//! ```text
//! point10/
//!   mod.rs                 this file: Point10Scene, SceneContext, prop/meta schemas,
//!                          all_scenes()
//!   audio_bridge.rs        AudioBridge (audioBridge.js) + P10Audio reader (p10Audio.js)
//!   shared/                particle_alpha (particleAlpha/particleSprite), random,
//!                          p9_glsl (the P9Glsl.js helpers batch 2 uses, as WGSL),
//!                          p9_shading (the P9Glsl.js lighting/colour half),
//!                          assets (texture loading + procedural fallbacks)
//!   chromatic/             shared ring-buffer sim + the ChromaticLine/ChromaticWave scenes
//!   chromatic_line.rs      compatibility re-export of chromatic/line.rs
//!   postfx/                RenderPass, UnrealBloomPass, ShaderPass, OutputPass, Composer
//!   scenes/                ParticleField, SpiralParticles, Waveform, SquareTunnel,
//!                          LightTunnel, Wormhole, ItIsComingForYou, WaterRipple,
//!                          GlassLens, Mycelium, AroundPlus, UltraHdr, MirrorCube,
//!                          ColorCube, Boxy, SoundCube, HollowCube,
//!                          AttractorParticles, DvdRoom (+ the cube_mesh and
//!                          particle_quad helpers)
//! ```
//!
//! # Batch order
//!
//! The port follows the inventory order in docs/rust-hybrid.md (dependency
//! order, not complexity):
//!
//! | Batch | Scenes | Why here |
//! |------:|--------|----------|
//! | — | shared helpers | `particleAlpha`, `particleSprite`, `random` have no scene dependency and three later scenes need them |
//! | — | audio bridge | every scene reads `P10Audio`; the bridge is the only audio entry point |
//! | — | postfx chain | 20 of 25 scenes use `EffectComposer` (RenderPass → UnrealBloom → ShaderPass → OutputPass) |
//! | 1 | **ChromaticWave**, ParticleField, SpiralParticles, Waveform | uses everything above, no `P9Glsl`/texture assets |
//! | 2 | **SquareTunnel**, LightTunnel, Wormhole, ItIsComingForYou | needs `plane9/P9Glsl` + `shared/textures` + `shared/random` |
//! | 3 | WaterRipple, GlassLens, Mycelium, AroundPlus | needs `shared/waterWave`, `shared/phraseColor`, `shared/assets` |
//! | 4 | **UltraHDR**, MirrorCube, ColorCube, Boxy, SoundCube | `P9Glsl` lighting (`shared/p9_shading`), `BoxGeometry`, `preserveBloomAlpha` |
//! | 5a | HollowCube, AttractorParticles, DvdRoom | first of the WebGPU/TSL group; the TSL compute passes become fragment/CPU substitutes |
//! | 5b | GeometricTunnel, RollingFire, SmokeRoom, LatentBackrooms | SSAO / Afterimage passes still to land |
//!
//! ChromaticLine was ported ahead of this batch (`chromatic_line.rs`, a
//! fixture-driven port with its own `SyntheticSine`); it now shares its
//! ring-buffer simulation with ChromaticWave through [`chromatic`] and its
//! update path is bridge-driven like every other scene, so [`all_scenes`]
//! registers it first, followed by the four **batch 1** scenes. `SyntheticSine`
//! stays available for tests and the headless frame harness, but it only fills
//! a `BandPayload` — the scene itself never reads it.
//!
//! # Scene contract
//!
//! `BaseScene.js` (WebGL) and `BaseSceneWebGPU.js` (WebGPU) both expose
//! `_init`, `update(dt, audioData)`, `render()`, `resize(w, h)`, `dispose()`.
//! The Rust contract is [`Point10Scene`]; the differences are deliberate:
//!
//! * `init` receives a [`SceneContext`] (device, queue, format, size) instead
//!   of a renderer/camera pair, because wgpu has no ambient renderer.
//! * `update` receives the [`AudioBridge`] rather than the adapter-prepared
//!   byte spectrum: scenes that want the byte array read
//!   [`AudioBridge::spectrum`] themselves (the JS adapter does exactly that at
//!   `adapter.js:809-816`), scenes that want bands use
//!   [`audio_bridge::P10Audio`].
//! * `render` takes a command encoder + a target view instead of using
//!   `renderer.render(group, camera)`. Every scene clears its own target to
//!   transparent black (`adapter.js:297`: `setClearColor(0x000000, 0)` with
//!   `alpha: true`), so a scene target composites over the layers below — the
//!   same "empty frame stays transparent" rule the JS scenes rely on.
//!
//! There is no camera object: the WebGL scenes are written against
//! `PerspectiveCamera(60, aspect, 0.1, 1000)` at `z = 15` looking at the
//! origin (`adapter.js:299-301`). The scenes that need a projection
//! ([`scenes::ParticleField`], [`scenes::SpiralParticles`],
//! [`scenes::Waveform`]) carry that camera as constants in their WGSL.
//! [`scenes::Wormhole`] and [`scenes::ItIsComingForYou`] build their own
//! cameras in `_init` (`Wormhole.js:216-227`,
//! `ItIsComingForYou.js:262-274`), so those two inline their own projection.
//! The two tunnels are fullscreen shaders and ignore the camera entirely.
//!
//! # `SceneContext` and the scene's own GPU resources
//!
//! [`SceneContext`] owns cheap clones of the device/queue plus the two shared
//! helpers: the full-screen-triangle vertex shader and a unit-quad
//! vertex/index buffer. Everything else (pipelines, bind groups, instance
//! buffers, textures) belongs to the scene and is created in `init` — the
//! pipeline cache lives in wgpu, so re-creating a pipeline after a scene
//! switch is cheap.
//!
//! From batch 3 the context also carries an optional `assets_dir`: the engine
//! will pass `--assets-dir` there, and scenes resolve textures through
//! [`shared::assets::AssetLoader`] (`$MODV_ASSETS_DIR`, then `assets_dir`, then
//! the repo's own texture directory).
//!
//! # Not here (later wiring)
//!
//! Instance lifecycle, eviction, the warm cache and presentation are engine
//! concerns (`adapter.js:1-120`, `warm-cache.js`). In Rust there is one device
//! and no OffscreenCanvas blit: the scene draws straight into the caller's
//! target, which removes the adapter's one-frame acquisition lag.

#![forbid(unsafe_code)]

pub mod audio_bridge;
pub mod chromatic;
pub mod chromatic_line;
pub mod postfx;
pub mod scenes;
pub mod shared;

#[cfg(test)]
mod scene_tests;
#[cfg(test)]
mod test_support;

/// HDR scene/postfx target format.
///
/// three's `EffectComposer` uses `HalfFloatType` render targets
/// (`UnrealBloomPass.js:90-125`), so the scene target and every bloom mip are
/// `Rgba16Float`. Scenes are initialised with this format by
/// [`postfx::Composer::scene_format`].
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Full-screen-triangle vertex shader, shared by every full-screen pass.
///
/// The same trick as the legacy pipeline (`ChromaticLine.js` `VERT`: raw NDC
/// from the vertices `[-1,-1, 3,-1, -1,3]`). Use it with a fragment entry that
/// has no vertex inputs, or together with [`fullscreen_uv`] for a
/// `position → uv` mapping.
pub const FULLSCREEN_TRIANGLE_WGSL: &str = "\
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
  var positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(positions[vertex_index], 0.0, 1.0);
}
";

/// Fragment-side `uv` helper matching [`FULLSCREEN_TRIANGLE_WGSL`].
///
/// `resolution` is the target size in pixels; `position` is the
/// `@builtin(position)` value (top-left origin, like `gl_FragCoord` after the
/// WebGL y-flip the legacy shaders get for free). Returns `0..1` with v = 0 at
/// the top row, matching a `DataTexture` upload.
pub const FULLSCREEN_UV_WGSL: &str = "\
fn fullscreen_uv(position: vec2<f32>, resolution: vec2<f32>) -> vec2<f32> {
  return position / resolution;
}
";

/// `timing.js: MAX_FRAME_SECONDS` — the Point10 adapter's frame clock caps a
/// frame at 0.1 s ("a tab-suspend catch-up cannot jump the pattern").
pub const MAX_FRAME_SECONDS: f32 = 0.1;

/// Clamp a frame delta the way the Point10 adapter's frame clock does
/// (`timing.js: createFrameClock`).
///
/// The JS clock also reports **0** on the first frame after a scene is
/// activated; the engine owns that clock, so the Rust scenes only need the cap
/// and the non-finite guard. Batch-2 scenes each carried their own copy of
/// this; batch 3 shares it.
pub fn frame_dt(dt: f32) -> f32 {
    if dt.is_finite() {
        dt.clamp(0.0, MAX_FRAME_SECONDS)
    } else {
        0.0
    }
}

/// One Point10 scene, ported from the `BaseScene` class contract.
pub trait Point10Scene {
    /// Registry name — the wrapper module's `createPoint10Module({ name })`
    /// string (`renderers/2d/modules/Point10*.js`), e.g.
    /// `"Point10 Chromatic Wave"`.
    fn name(&self) -> &'static str;

    /// Build the scene's GPU resources. Called once per activation (and again
    /// after a [`Point10Scene::dispose`]).
    fn init(&mut self, ctx: &mut SceneContext);

    /// Target size changed. Scenes that depend on resolution re-create their
    /// size-dependent resources here.
    fn resize(&mut self, width: u32, height: u32);

    /// Advance one frame. The queue is not passed (the trait keeps the
    /// update/render split of `BaseScene`), so scenes clone the queue in
    /// `init` and upload uniforms/buffers here.
    fn update(&mut self, dt: f32, audio: &audio_bridge::AudioBridge);

    /// Record the scene into `target`, clearing it to transparent black first.
    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView);

    /// Release GPU resources. Idempotent; the scene can be re-initialised.
    fn dispose(&mut self);

    /// The scene's modV prop schema: its `sceneProps.js` entry preceded by
    /// `RESPONSIVENESS_PROPS`, which the adapter adds to every Point10 module
    /// (`adapter.js:868-871`: `props: { ...RESPONSIVENESS_PROPS, ...sceneProps }`).
    ///
    /// Returned by value because the JS spreads two objects; use
    /// [`scene_props_with_levers`] to build it.
    fn props(&self) -> Vec<SceneProp> {
        Vec::new()
    }

    /// The Gallery meta (`adapter.js:857-866`).
    fn meta(&self) -> SceneMeta {
        SceneMeta::DEFAULT
    }
}

/// A modV prop definition, matching the `sceneProps.js` shape exactly
/// (`{ label, type, min, max, step, default }`) so saved presets keep working.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneProp {
    /// Prop key (`props.<name>`).
    pub name: &'static str,
    /// UI label.
    pub label: &'static str,
    /// Prop type.
    pub kind: ScenePropKind,
    /// Default value (booleans use 1.0 / 0.0).
    pub default: f32,
    /// Minimum for numeric props (ignored for booleans).
    pub min: f32,
    /// Maximum for numeric props (ignored for booleans).
    pub max: f32,
    /// Control step for numeric props.
    pub step: f32,
}

/// The prop types the batch-1 scenes declare.
///
/// `sceneProps.js` only uses `float` and `bool` for these four scenes;
/// `readSceneProps` treats every other type as numeric, so the enum grows when
/// a later scene needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenePropKind {
    /// `type: "float"`.
    Float,
    /// `type: "bool"` — read as `value != false`.
    Bool,
}

impl SceneProp {
    /// A `float` prop (`sceneProps.js` helper `float(label, default, min, max, step = 0.01)`).
    pub const fn float(
        name: &'static str,
        label: &'static str,
        default: f32,
        min: f32,
        max: f32,
        step: f32,
    ) -> Self {
        Self {
            name,
            label,
            kind: ScenePropKind::Float,
            default,
            min,
            max,
            step,
        }
    }

    /// A `bool` prop (`props.js` `useAgc` shape).
    pub const fn boolean(name: &'static str, label: &'static str, default: bool) -> Self {
        Self {
            name,
            label,
            kind: ScenePropKind::Bool,
            default: if default { 1.0 } else { 0.0 },
            min: 0.0,
            max: 1.0,
            step: 1.0,
        }
    }

    /// Resolve this prop against an incoming value.
    ///
    /// `sceneProps.js: readSceneProps` (lines 221-241): booleans are
    /// `props[key] !== false`; numbers fall back to the default when the
    /// incoming value is missing/non-finite and clamp into `[min, max]`.
    pub fn read(&self, value: Option<f32>) -> f32 {
        match self.kind {
            ScenePropKind::Bool => {
                if value.unwrap_or(1.0) != 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            ScenePropKind::Float => {
                let value = match value {
                    Some(value) if value.is_finite() => value,
                    _ => self.default,
                };
                value.clamp(self.min, self.max)
            }
        }
    }
}

/// A scene's own schema with the responsiveness levers in front of it — the
/// exact prop list `createPoint10Module` publishes (`adapter.js:868-871`).
pub fn scene_props_with_levers(schema: &[SceneProp]) -> Vec<SceneProp> {
    let mut props = Vec::with_capacity(audio_bridge::RESPONSIVENESS_PROPS.len() + schema.len());
    props.extend_from_slice(audio_bridge::RESPONSIVENESS_PROPS);
    props.extend_from_slice(schema);
    props
}

/// Resolve a whole schema against a prop getter.
///
/// Returns `(key, value)` pairs in schema order — the order the binder in
/// `sceneProps.js: SCENE_PROP_CONFIGS` reads them.
pub fn read_scene_props(
    schema: &'static [SceneProp],
    mut get: impl FnMut(&str) -> Option<f32>,
) -> Vec<(&'static str, f32)> {
    schema
        .iter()
        .map(|prop| (prop.name, prop.read(get(prop.name))))
        .collect()
}

/// The Gallery meta of a Point10 module (`adapter.js:857-866`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneMeta {
    /// Display + registration name, e.g. `"Point10 Particle Field"`.
    pub name: &'static str,
    /// Module author (`"Point10"` for every imported scene).
    pub author: &'static str,
    /// Module version (`"1.0.0"`).
    pub version: &'static str,
    /// Module type — `"2d"` for every Point10 scene.
    pub module_type: &'static str,
    /// Gallery category (`"3d-scenes"` by default; the Chromatic scenes and
    /// fullscreen 2D scenes override it to `"2d-scenes"`).
    pub category: &'static str,
    /// Requested audio features (`["bass", "mid", "treble"]`).
    pub audio_features: &'static [&'static str],
}

impl SceneMeta {
    /// Default meta before a scene overrides it (`adapter.js:857-866`).
    pub const DEFAULT: Self = Self {
        name: "",
        author: "Point10",
        version: "1.0.0",
        module_type: "2d",
        category: "3d-scenes",
        audio_features: &["bass", "mid", "treble"],
    };

    /// Meta for a registered scene name with an explicit Gallery category.
    pub const fn for_scene(name: &'static str, category: &'static str) -> Self {
        Self {
            name,
            category,
            ..Self::DEFAULT
        }
    }
}

/// The shared helper resources a scene may use.
///
/// The device and queue are clones (wgpu handles are reference-counted), so a
/// scene can keep them past `init`. `target_format` is the format of the view
/// [`Point10Scene::render`] receives — [`HDR_FORMAT`] when the scene runs
/// behind [`postfx::Composer`].
pub struct SceneContext {
    /// wgpu device.
    pub device: wgpu::Device,
    /// wgpu queue.
    pub queue: wgpu::Queue,
    /// Format of the render target the scene draws into.
    pub target_format: wgpu::TextureFormat,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Shared unit quad (corner + uv), used by the instanced particle scenes.
    pub quad: UnitQuad,
    /// Directory the engine passes for scene textures (the planned
    /// `--assets-dir`). `None` in the tests and until `modv-app` wires it; the
    /// loader then falls back to `$MODV_ASSETS_DIR` and the repo's texture
    /// directory.
    pub assets_dir: Option<std::path::PathBuf>,
}

impl SceneContext {
    /// Build a context at 1x1; call [`SceneContext::resize`] before the first
    /// frame.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            target_format,
            width: 1,
            height: 1,
            quad: UnitQuad::new(device),
            assets_dir: None,
        }
    }

    /// Set the engine-provided asset directory (builder form, so `new` stays
    /// source-compatible for the engine).
    pub fn with_assets_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.assets_dir = Some(dir.into());
        self
    }

    /// The asset loader for this context (`$MODV_ASSETS_DIR`, then
    /// [`SceneContext::assets_dir`], then the dev fallback).
    pub fn asset_loader(&self) -> shared::assets::AssetLoader {
        shared::assets::AssetLoader::for_assets_dir(self.assets_dir.as_deref())
    }

    /// Update the logical target size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    /// Target aspect ratio (`width / height`), matching the legacy
    /// `PerspectiveCamera` aspect.
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }

    /// Build a full-screen-triangle pipeline from `shader` (which must declare
    /// [`FULLSCREEN_TRIANGLE_WGSL`]'s `vs_main` and its own `fs_main`).
    pub fn fullscreen_pipeline(
        &self,
        label: &str,
        shader: &wgpu::ShaderModule,
        layout: &wgpu::PipelineLayout,
        blend: Option<wgpu::BlendState>,
    ) -> wgpu::RenderPipeline {
        self.fullscreen_pipeline_with_format(label, shader, layout, self.target_format, blend)
    }

    /// [`SceneContext::fullscreen_pipeline`] for an explicit target format.
    ///
    /// The composer-owning scenes draw into the composer's HDR scene target
    /// ([`postfx::Composer::scene_format`]), which is not the engine's
    /// destination format, so they cannot use the context's `target_format`
    /// for their scene pipeline.
    pub fn fullscreen_pipeline_with_format(
        &self,
        label: &str,
        shader: &wgpu::ShaderModule,
        layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
        blend: Option<wgpu::BlendState>,
    ) -> wgpu::RenderPipeline {
        let device = &self.device;
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}

/// A unit quad in clip space with the matching `uv`.
///
/// Corner `(x, y)` is in `[-0.5, 0.5]` with **y up** (clip space), `uv` is
/// `0..1` with **v = 0 at the top**, so a `DataTexture`-style sprite upload
/// (row 0 = top) samples the right way up.
pub struct UnitQuad {
    /// Interleaved `[x, y, u, v]` vertices.
    pub vertex_buffer: wgpu::Buffer,
    /// Two triangles (`0,1,2, 0,2,3`).
    pub index_buffer: wgpu::Buffer,
}

impl UnitQuad {
    /// Vertex count of one quad.
    pub const VERTEX_COUNT: u32 = 6;
    /// Corners, `[x, y, u, v]` — top-left, bottom-left, bottom-right,
    /// top-right (counter-clockwise in y-up clip space).
    pub const VERTICES: [[f32; 4]; 4] = [
        [-0.5, 0.5, 0.0, 0.0],
        [-0.5, -0.5, 0.0, 1.0],
        [0.5, -0.5, 1.0, 1.0],
        [0.5, 0.5, 1.0, 0.0],
    ];
    /// Index order.
    pub const INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

    /// Upload the shared quad buffers.
    pub fn new(device: &wgpu::Device) -> Self {
        use wgpu::util::DeviceExt;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point10-unit-quad-vertices"),
            contents: bytemuck::cast_slice(&Self::VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point10-unit-quad-indices"),
            contents: bytemuck::cast_slice(&Self::INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            vertex_buffer,
            index_buffer,
        }
    }

    /// Vertex layout (`@location(2) corner: vec2<f32>`, `@location(3) uv: vec2<f32>`).
    ///
    /// Locations 0 and 1 are reserved for the per-instance position and
    /// colour of the particle scenes.
    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: 4 * std::mem::size_of::<f32>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 2 * std::mem::size_of::<f32>() as u64,
                    shader_location: 3,
                },
            ],
        }
    }

    /// Bind and draw with an instanced draw call.
    pub fn draw_instanced<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, instances: u32) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..Self::VERTEX_COUNT, 0, 0..instances);
    }
}

/// Every Point10 scene implemented on the shared [`Point10Scene`] contract.
///
/// Order is the WS-7 port order, so the Gallery list is stable:
/// `ChromaticLine, ChromaticWave, ParticleField, SpiralParticles, Waveform`
/// (ChromaticLine was ported ahead of batch 1; the rest are batch 1), then the
/// batch-2 scenes `SquareTunnel, LightTunnel, Wormhole, ItIsComingForYou`, the
/// batch-3 scenes `WaterRipple, GlassLens, Mycelium, AroundPlus` (inventory
/// order 10, 11, 17, 18), the batch-4 scenes `UltraHDR, MirrorCube,
/// ColorCube, Boxy, SoundCube` (inventory order 12, 14, 15, 16, 19) and the
/// batch-5a scenes `HollowCube, AttractorParticles, DvdRoom`. The remaining 4
/// scenes (`GeometricTunnel`, `RollingFire`, `SmokeRoom`, `LatentBackrooms`)
/// are added batch by batch.
pub fn all_scenes() -> Vec<Box<dyn Point10Scene>> {
    vec![
        Box::new(chromatic::line::ChromaticLine::new()),
        Box::new(scenes::chromatic_wave::ChromaticWave::new()),
        Box::new(scenes::particle_field::ParticleField::new()),
        Box::new(scenes::spiral_particles::SpiralParticles::new()),
        Box::new(scenes::waveform::Waveform::new()),
        Box::new(scenes::square_tunnel::SquareTunnel::new()),
        Box::new(scenes::light_tunnel::LightTunnel::new()),
        Box::new(scenes::wormhole::Wormhole::new()),
        Box::new(scenes::it_is_coming_for_you::ItIsComingForYou::new()),
        Box::new(scenes::water_ripple::WaterRipple::new()),
        Box::new(scenes::glass_lens::GlassLens::new()),
        Box::new(scenes::mycelium::Mycelium::new()),
        Box::new(scenes::around_plus::AroundPlus::new()),
        Box::new(scenes::ultra_hdr::UltraHdr::new()),
        Box::new(scenes::mirror_cube::MirrorCube::new()),
        Box::new(scenes::color_cube::ColorCube::new()),
        Box::new(scenes::boxy::Boxy::new()),
        Box::new(scenes::sound_cube::SoundCube::new()),
        Box::new(scenes::hollow_cube::HollowCube::new()),
        Box::new(scenes::attractor_particles::AttractorParticles::new()),
        Box::new(scenes::dvd_room::DvdRoom::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_the_port_order_scenes() {
        let scenes = all_scenes();
        assert_eq!(scenes.len(), 21);

        assert_eq!(scenes[0].name(), "Point10 Chromatic Line");
        assert_eq!(scenes[1].name(), "Point10 Chromatic Wave");
        assert_eq!(scenes[2].name(), "Point10 Particle Field");
        assert_eq!(scenes[3].name(), "Point10 Spiral Particles");
        assert_eq!(scenes[4].name(), "Point10 Waveform");
        // Batch 2, in the WS-7 inventory order.
        assert_eq!(scenes[5].name(), "Point10 Square Tunnel");
        assert_eq!(scenes[6].name(), "Point10 Light Tunnel");
        assert_eq!(scenes[7].name(), "Point10 Wormhole");
        assert_eq!(scenes[8].name(), "Point10 It Is Coming For You");
        // Batch 3, in the WS-7 inventory order (10, 11, 17, 18).
        assert_eq!(scenes[9].name(), "Point10 Water Ripple");
        assert_eq!(scenes[10].name(), "Point10 Glass Lens");
        assert_eq!(scenes[11].name(), "Point10 Mycelium");
        assert_eq!(scenes[12].name(), "Point10 Around Plus");
        // Batch 4, in the WS-7 inventory order (12, 14, 15, 16, 19).
        assert_eq!(scenes[13].name(), "Point10 Ultra HDR");
        assert_eq!(scenes[14].name(), "Point10 Mirror Cube");
        assert_eq!(scenes[15].name(), "Point10 Color Cube");
        assert_eq!(scenes[16].name(), "Point10 Boxy");
        assert_eq!(scenes[17].name(), "Point10 Sound Cube");
        // Batch 5a.
        assert_eq!(scenes[18].name(), "Point10 Hollow Cube");
        assert_eq!(scenes[19].name(), "Point10 Attractor Particles");
        assert_eq!(scenes[20].name(), "Point10 DVD Room");
    }

    #[test]
    fn registry_names_are_unique() {
        let scenes = all_scenes();
        let mut names: Vec<&str> = scenes.iter().map(|scene| scene.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn registry_meta_matches_the_adapter_defaults() {
        for scene in all_scenes() {
            let meta = scene.meta();
            assert_eq!(meta.name, scene.name());
            assert_eq!(meta.author, "Point10");
            assert_eq!(meta.version, "1.0.0");
            assert_eq!(meta.module_type, "2d");
            assert_eq!(meta.audio_features, &["bass", "mid", "treble"]);
        }
    }

    #[test]
    fn every_scene_declares_the_responsiveness_levers() {
        for scene in all_scenes() {
            let props = scene.props();
            for levers in audio_bridge::RESPONSIVENESS_PROPS {
                assert!(
                    props.iter().any(|prop| prop.name == levers.name),
                    "{} is missing {}",
                    scene.name(),
                    levers.name
                );
            }
        }
    }

    #[test]
    fn prop_read_clamps_and_defaults() {
        let distortion = SceneProp::float("distortion", "Chroma", 0.05, 0.0, 0.5, 0.001);
        assert_eq!(distortion.read(None), 0.05);
        assert_eq!(distortion.read(Some(f32::NAN)), 0.05);
        assert_eq!(distortion.read(Some(-2.0)), 0.0);
        assert_eq!(distortion.read(Some(9.0)), 0.5);
        assert_eq!(distortion.read(Some(0.25)), 0.25);

        let use_agc = SceneProp::boolean("useAgc", "Auto Gain (AGC)", true);
        // `props.useAgc !== false`
        assert_eq!(use_agc.read(None), 1.0);
        assert_eq!(use_agc.read(Some(1.0)), 1.0);
        assert_eq!(use_agc.read(Some(0.0)), 0.0);
    }

    #[test]
    fn read_scene_props_keeps_schema_order() {
        const SCHEMA: &[SceneProp] = &[
            SceneProp::float("height", "Height", 1.0, 0.0, 4.0, 0.01),
            SceneProp::float("speed", "Wave Speed", 1.0, 0.0, 4.0, 0.01),
        ];
        let values = read_scene_props(SCHEMA, |key| (key == "speed").then_some(3.0));
        assert_eq!(values, vec![("height", 1.0), ("speed", 3.0)]);
    }

    #[test]
    fn frame_dt_matches_the_adapter_clock() {
        assert_eq!(frame_dt(1.0 / 60.0), 1.0 / 60.0);
        assert_eq!(frame_dt(0.0), 0.0);
        assert_eq!(frame_dt(5.0), MAX_FRAME_SECONDS);
        assert_eq!(frame_dt(-1.0), 0.0);
        assert_eq!(frame_dt(f32::NAN), 0.0);
        assert_eq!(frame_dt(f32::INFINITY), 0.0);
    }

    #[test]
    fn unit_quad_is_wound_counter_clockwise() {
        // wgpu's default front face is CCW in y-up clip space; the quad must
        // not rely on culling being off.
        let vertices = UnitQuad::VERTICES;
        let a = (vertices[0][0], vertices[0][1]);
        let b = (vertices[1][0], vertices[1][1]);
        let c = (vertices[2][0], vertices[2][1]);
        let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
        assert!(cross > 0.0, "expected CCW winding, cross = {cross}");
    }
}
