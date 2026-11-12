//! Boxy — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/Boxy.js`.
//!
//! The Plane9 "Boxy" preset (Joakim Dahl, CC BY-NC-SA 4.0): 1000 instanced unit
//! cubes scattered in random directions down a -Z tunnel, textured with the
//! metallic `tinore` material and a Stormy Days reflection. `Shader1` positions
//! every cube from its instance id, `RenderToTexture1` captures that chain,
//! `Bloom1` blurs it and `Shader2` adds bloom + cubes for the screen. There is
//! no `PreviousLayer` node, so no frame feedback.
//!
//! # Ported exactly
//!
//! * the instance placement (`:137-175`): `d = instance / 1000`,
//!   `_rand3(d)` for the direction/scale, `_rand3(instance * 0.825931)` for the
//!   glow lane, `normalize(rnd.xy * 2 - 1) * (1 + rnd2.x * 5)` in xy,
//!   `-mod(d * 20 - in1.z, 20)` in z, and the `0.1 + rnd * 0.2` cube scale;
//! * the material shader (`:177-252`): the tinore albedo/normal/metallic/
//!   roughness/ao reads (the preset packs R=metallic, G=roughness, B=AO; the JS
//!   reconstructs that from the three sibling maps and so does the port), the
//!   derived tangent frame, `_palette` tint, the `smoothstep(0.91, 0.911)`
//!   glowing cubes, `realAlbedo = mix(albedo * ao, 0, metallic)`,
//!   `_lightDirectional(vec3(10), (0,0,-1))`, the `_fresnelRoughness` env term,
//!   `simpleFog(..., 0.3)` and the `smoothstep(0, 1, -wpos.z)` fade;
//! * Expression1 (`:111-131`): `in1 = (bass * 20, mid * 20, perm + PERMRAND *
//!   10)`, `perm += min(2, bass * 20 + 0.2) * dt`, `in2 = (0.3, 0, 0)`,
//!   `PERMRAND = seededRand(23, 8)`;
//! * the camera (`:58`): `PerspectiveCamera(70, aspect, 0.01, 100)` at the
//!   origin looking down -z;
//! * the render chain (`:346-369`): RenderPass → `preserveBloomAlpha`'d
//!   UnrealBloomPass `(1, 0.85, 0)` → OutputPass, `NoToneMapping`, clear
//!   alpha 0;
//! * the prop binder (`sceneProps.js:363-368`): `scrollScale = scrollSpeed`,
//!   `glowScale = glow`, `roughnessFloor = roughness`,
//!   `bloomScale = bloom`.
//!
//! # `preserveBloomAlpha`
//!
//! Boxy *defines* the helper (`:73-86`), and every Point10 scene that blooms
//! with `threshold: 0` imports it (Wormhole, ColorCube). three's additive bloom
//! composite also adds the bloom RGB into the destination alpha; with
//! `threshold: 0` every pixel receives bloom, so the transparent clear would
//! become an opaque veil and the module's layer alpha would be lost. The helper
//! rewrites the bloom blend to
//!
//! ```text
//! colour: Add, src = ONE,       dst = ONE      // bloom RGB is added
//! alpha:  Add, srcAlpha = ZERO, dstAlpha = ONE // destination alpha wins
//! ```
//!
//! and the port is [`Composer::set_preserve_bloom_alpha(true)`](crate::point10::postfx::Composer::set_preserve_bloom_alpha),
//! whose combine entry point is `fs_preserve_alpha` (`base.rgb + bloom.rgb`,
//! `base.a`). `scene_tests::boxy_keeps_the_background_transparent` renders two
//! frames and asserts both an alpha-0 background and opaque cubes, so the
//! module's layer alpha is covered by a lavapipe test.
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `InstancedBufferGeometry` + `aInstance` (1000 floats) | `@builtin(instance_index)` — no attribute buffer |
//! | `BoxGeometry` position/normal/uv | [`build_box`](crate::point10::scenes::cube_mesh::build_box) |
//! | `projectionMatrix * modelViewMatrix` (identity model) | one view-projection uniform |
//! | `TextureLoader` (5 tinore maps) | [`load_texture`] with `Repeat` |
//! | `CubeTexture` (6 Stormy Days faces) | the panorama path the JS already ships: `_texturePanoramicLod` over `stormy/posz.jpg` (`uEnvPanoramaMode = 1`) |
//! | `_rand3`, `_palette`, `_lightDirectional`, `_fresnelRoughness` | [`P9_SHADING_WGSL`] |
//! | `EffectComposer` + `preserveBloomAlpha` | [`Composer`] with `set_preserve_bloom_alpha(true)` |
//!
//! # Deviations
//!
//! * **The environment is the panorama, not the cubemap.** A six-face cube
//!   upload is not something the shared asset loader does, and the JS itself
//!   falls back to `texturePanoramicLod(posz)` when `ImageBitmap` cube faces
//!   cannot be built (`:424-443`). The port takes that supported path —
//!   `envDir` reflection direction, `roughness² * 8` lod, same panorama — so
//!   the metal still reflects the storm rather than a black placeholder.
//! * `texture.anisotropy = maxAniso` is not reproduced (see `shared::assets`).
//! * The `material.bbb` swizzle (`ao, ao, ao`) and the packed `vec3(metallic,
//!   roughness, ao)` rebuild are kept, so the shading maths matches the JS line
//!   for line.
//! * `P9Audio` is `plane9/P9Audio.js` in the JS; the port reads [`P10Audio`]
//!   over the shared [`AudioBridge`] like every other ported scene.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, create_depth, depth_state, look_at, mat4_mul, perspective,
};
use crate::point10::shared::assets::{ColourSpace, Wrap, load_texture};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::p9_shading::P9_SHADING_WGSL;
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`Boxy.js:53-97`)
// ---------------------------------------------------------------------------

/// `INSTANCE_COUNT` — `MeshInstancer1` NumInstances.
pub const INSTANCE_COUNT: u32 = 1000;
/// `CUBE_SIZE`.
pub const CUBE_SIZE: f32 = 1.0;

/// `CAMERA.fov`.
pub const CAMERA_FOV: f32 = 70.0;
/// `CAMERA.near`.
pub const CAMERA_NEAR: f32 = 0.01;
/// `CAMERA.far`.
pub const CAMERA_FAR: f32 = 100.0;

/// `BLOOM.strength`.
pub const BLOOM_STRENGTH: f32 = 1.0;
/// `BLOOM.radius`.
pub const BLOOM_RADIUS: f32 = 0.85;
/// `BLOOM.threshold`.
pub const BLOOM_THRESHOLD: f32 = 0.0;

/// `CLEAR_ALPHA` — Boxy composites transparently over whatever is behind it.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `ROUGHNESS_FLOOR` (`Expression1`'s `In2 = (0.3, 0, 0)`).
pub const ROUGHNESS_FLOOR: f32 = 0.3;

/// `MAX_DT` — the JS clamps the frame step so the scroll cannot jump.
pub const MAX_DT: f32 = 0.1;

/// The five `tinore` maps.
pub const DIFFUSE_ASSET: &str = "tinore_diffuse.jpg";
/// Normal map.
pub const NORMAL_ASSET: &str = "tinore_normal.jpg";
/// Metallic map (R channel).
pub const METALLIC_ASSET: &str = "tinore_metallic.png";
/// Roughness map (G channel).
pub const ROUGHNESS_ASSET: &str = "tinore_roughness.jpg";
/// Ambient-occlusion map (R channel).
pub const AO_ASSET: &str = "tinore_ao.jpg";

/// The Stormy Days `+Z` face, used as the equirect reflection.
pub const ENV_ASSET: &str = "stormy/posz.jpg";
/// Flat fallback for a packaged asset tree that does not keep the subdirectory.
pub const ENV_ASSET_FLAT: &str = "posz.jpg";

/// `PERMRAND = seededRand(23, 8)`.
pub fn perm_rand() -> f32 {
    seeded_rand(23, 8)
}

/// `makePerm(seed = 1)`.
pub fn make_perm(seed: f32) -> f32 {
    if seed.is_finite() { seed } else { 1.0 }
}

/// One frame of Expression1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxyDrive {
    /// `perm` for the next frame.
    pub perm: f32,
    /// `gIn1` (`x` bass glow, `y` mid glow, `z` tunnel scroll).
    pub in1: [f32; 3],
    /// `gIn2` (`x` roughness floor).
    pub in2: [f32; 3],
}

/// Pure Expression1: bass and mid drive `in1.x`/`in1.y`, bass advances the
/// tunnel scroll (capped at 2 units/s) and `in1.z` carries the seeded phase.
pub fn boxy_drive(bass: f32, mid: f32, dt: f32, perm: f32) -> BoxyDrive {
    let bass = if bass.is_finite() { bass.max(0.0) } else { 0.0 };
    let mid = if mid.is_finite() { mid.max(0.0) } else { 0.0 };
    let step = if dt.is_finite() {
        dt.max(0.0).min(MAX_DT)
    } else {
        0.0
    };
    let base = if perm.is_finite() { perm } else { 1.0 };
    let next = base + (bass * 20.0 + 0.2).min(2.0) * step;

    BoxyDrive {
        perm: next,
        in1: [bass * 20.0, mid * 20.0, next + perm_rand() * 10.0],
        in2: [ROUGHNESS_FLOOR, 0.0, 0.0],
    }
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `BOXY_PROPS` (`sceneProps.js:73-78`).
const BOXY_PROPS: &[SceneProp] = &[
    SceneProp::float("scrollSpeed", "Scroll Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("glow", "Glow", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("roughness", "Roughness Floor", 0.3, 0.0, 1.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
];

/// The uniform block (112 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BoxyUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    /// `gIn1`: bass glow, mid glow, tunnel scroll.
    in1: [f32; 4],
    /// `gIn2`: roughness floor.
    in2: [f32; 4],
}

/// GPU resources built in `init`.
struct BoxyGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    mesh: MeshGpu,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    _textures: Vec<wgpu::Texture>,
    composer: Composer,
}

/// Boxy scene.
pub struct Boxy {
    audio: P10Audio,
    /// `this.perm`.
    perm: f32,
    in1: [f32; 4],
    in2: [f32; 4],
    scroll_scale: f32,
    glow_scale: f32,
    roughness_floor: f32,
    bloom_scale: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<BoxyGpu>,
}

impl Default for Boxy {
    fn default() -> Self {
        Self::new()
    }
}

impl Boxy {
    /// Create the scene (`Boxy.js:310-383` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            perm: make_perm(1.0),
            in1: [0.0; 4],
            in2: [ROUGHNESS_FLOOR, 0.0, 0.0, 0.0],
            scroll_scale: 1.0,
            glow_scale: 1.0,
            roughness_floor: ROUGHNESS_FLOOR,
            bloom_scale: 1.0,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:363-368`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(BOXY_PROPS, props) {
            match key {
                "scrollSpeed" => self.scroll_scale = value,
                "glow" => self.glow_scale = value,
                "roughness" => self.roughness_floor = value,
                "bloom" => self.bloom_scale = value,
                _ => {}
            }
        }
    }

    /// `this.perm`.
    pub fn perm(&self) -> f32 {
        self.perm
    }

    /// `gIn1` as last uploaded.
    pub fn in1(&self) -> [f32; 4] {
        self.in1
    }

    /// `gIn2` as last uploaded.
    pub fn in2(&self) -> [f32; 4] {
        self.in2
    }

    /// The destination alpha the scene clears to (`CLEAR_ALPHA = 0`).
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            // Screen1: CamPos 0 0 0, lookAt 0 0 -1.
            &look_at([0.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        )
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.uniforms,
            0,
            bytemuck::bytes_of(&BoxyUniforms {
                view_proj: self.view_proj(),
                // The camera sits at the origin (Screen1 CamPos 0 0 0).
                camera_pos: [0.0, 0.0, 0.0, 1.0],
                in1: self.in1,
                in2: self.in2,
            }),
        );
    }
}

impl Point10Scene for Boxy {
    fn name(&self) -> &'static str {
        "Point10 Boxy"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);
        self.camera_aspect = ctx.aspect();

        let loader = ctx.asset_loader();
        let diffuse = load_texture(
            &device,
            &queue,
            &loader,
            DIFFUSE_ASSET,
            ColourSpace::Srgb,
            Wrap::Repeat,
        );
        let normal = load_texture(
            &device,
            &queue,
            &loader,
            NORMAL_ASSET,
            ColourSpace::Linear,
            Wrap::Repeat,
        );
        let metallic = load_texture(
            &device,
            &queue,
            &loader,
            METALLIC_ASSET,
            ColourSpace::Linear,
            Wrap::Repeat,
        );
        let roughness = load_texture(
            &device,
            &queue,
            &loader,
            ROUGHNESS_ASSET,
            ColourSpace::Linear,
            Wrap::Repeat,
        );
        let ao = load_texture(
            &device,
            &queue,
            &loader,
            AO_ASSET,
            ColourSpace::Linear,
            Wrap::Repeat,
        );
        // The panorama path (`uEnvPanorama`), preferring the Stormy Days +Z
        // face and falling back to a flat copy for a packaged asset tree.
        let env_name = if loader.resolve(ENV_ASSET).is_some() {
            ENV_ASSET
        } else {
            ENV_ASSET_FLAT
        };
        let env = load_texture(
            &device,
            &queue,
            &loader,
            env_name,
            ColourSpace::Srgb,
            Wrap::Repeat,
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-boxy-shader"),
            source: wgpu::ShaderSource::Wgsl(boxy_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-boxy-layout"),
            entries: &[
                // 0: uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 1/2: diffuse
                texture_entry(1),
                sampler_entry(2),
                // 3/4: normal
                texture_entry(3),
                sampler_entry(4),
                // 5/6: metallic
                texture_entry(5),
                sampler_entry(6),
                // 7/8: roughness
                texture_entry(7),
                sampler_entry(8),
                // 9/10: ao
                texture_entry(9),
                sampler_entry(10),
                // 11/12: environment panorama
                texture_entry(11),
                sampler_entry(12),
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-boxy-uniforms"),
            size: std::mem::size_of::<BoxyUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-boxy-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&diffuse.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&diffuse.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&normal.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&normal.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&metallic.view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&metallic.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&roughness.view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::Sampler(&roughness.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&ao.view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Sampler(&ao.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: wgpu::BindingResource::TextureView(&env.view),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: wgpu::BindingResource::Sampler(&env.sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-boxy-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-boxy-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(crate::point10::scenes::cube_mesh::Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let (depth, depth_view) =
            create_depth(&device, "point10-boxy-depth", self.size.0, self.size.1);

        // `EffectComposer` + `preserveBloomAlpha` (`:346-369`).
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.enable_bloom(BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD);
        composer.set_preserve_bloom_alpha(true);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        let mesh = MeshGpu::new(
            &device,
            "point10-boxy-mesh",
            &build_box(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
        );

        self.gpu = Some(BoxyGpu {
            device,
            queue,
            pipeline,
            bind_group,
            uniforms,
            mesh,
            depth,
            depth_view,
            _textures: vec![
                diffuse.texture,
                normal.texture,
                metallic.texture,
                roughness.texture,
                ao.texture,
                env.texture,
            ],
            composer,
        });
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.resize(self.size.0, self.size.1);
            let (depth, depth_view) =
                create_depth(&gpu.device, "point10-boxy-depth", self.size.0, self.size.1);
            gpu.depth = depth;
            gpu.depth_view = depth_view;
        }
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.audio.update(dt, audio);

        let drive = boxy_drive(
            self.audio.band(0, 1, 0, 0.0),
            self.audio.band(1, 1, 0, 0.0),
            dt * self.scroll_scale,
            self.perm,
        );
        self.perm = drive.perm;
        self.in1 = [
            drive.in1[0],
            drive.in1[1] * self.glow_scale,
            drive.in1[2],
            0.0,
        ];
        self.in2 = [self.roughness_floor, drive.in2[1], drive.in2[2], 0.0];
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        if let Some(bloom) = gpu.composer.bloom_mut() {
            bloom.set_strength(BLOOM_STRENGTH * self.bloom_scale);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-boxy"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gpu.composer.scene_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            gpu.mesh.draw_instanced(&mut pass, INSTANCE_COUNT);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(BOXY_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10Boxy.js` sets no category.
        SceneMeta::for_scene("Point10 Boxy", "3d-scenes")
    }
}

/// A sampled-texture binding entry.
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A filtering-sampler binding entry.
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// `buildBoxyVertexShader` + `buildBoxyFragmentShader` with the Plane9 helpers
/// in front.
fn boxy_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{P9_SHADING_WGSL}\n{BOXY_WGSL}")
}

const BOXY_WGSL: &str = r#"
struct BoxyUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // x bass glow, y mid glow, z tunnel scroll
  in1: vec4<f32>,
  // x roughness floor
  in2: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: BoxyUniforms;
@group(0) @binding(1) var map_diffuse: texture_2d<f32>;
@group(0) @binding(2) var map_diffuse_sampler: sampler;
@group(0) @binding(3) var map_normal: texture_2d<f32>;
@group(0) @binding(4) var map_normal_sampler: sampler;
@group(0) @binding(5) var map_metallic: texture_2d<f32>;
@group(0) @binding(6) var map_metallic_sampler: sampler;
@group(0) @binding(7) var map_roughness: texture_2d<f32>;
@group(0) @binding(8) var map_roughness_sampler: sampler;
@group(0) @binding(9) var map_ao: texture_2d<f32>;
@group(0) @binding(10) var map_ao_sampler: sampler;
@group(0) @binding(11) var map_environment: texture_2d<f32>;
@group(0) @binding(12) var map_environment_sampler: sampler;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) tex: vec2<f32>,
  @location(1) w_normal: vec3<f32>,
  @location(2) w_pos: vec3<f32>,
  @location(3) view_dir: vec3<f32>,
  @location(4) pos: vec3<f32>,
  @location(5) rnd: vec2<f32>,
}

@vertex
fn vs_main(
  @builtin(instance_index) instance: u32,
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
) -> VsOut {
  // `float instanceIdFloat = aInstance;` — wgpu's instance index is the same
  // number for free (no 1000-float attribute).
  let instance_id = f32(instance);
  let d = instance_id / 1000.0;

  let rnd = p9_rand3(d);
  let rnd2 = p9_rand3(instance_id * 0.825931);
  var wp = vec3<f32>(0.0);

  // Random direction.
  let r = normalize(rnd.xy * 2.0 - vec2<f32>(1.0)) * (1.0 + rnd2.x * 5.0);
  wp = wp + vec3<f32>(r.x, r.y, 0.0);
  wp.z = wp.z - p9_mod(d * 20.0 - u.in1.z, 20.0);

  let v = position * (vec3<f32>(0.1) + rnd.xyz * 0.2) + wp;

  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(v, 1.0);
  out.tex = uv;
  // `mat3(modelMatrix) * normal`: the instanced mesh has no transform.
  out.w_normal = normalize(normal);
  out.w_pos = v;
  out.view_dir = u.camera_pos.xyz - v;
  out.pos = v;
  out.rnd = rnd2.yz;
  return out;
}

fn simple_fog(org_col: vec3<f32>, dist_to_cam: f32, density: f32) -> vec3<f32> {
  let fog_amount = 1.0 - exp(-dist_to_cam * density);
  return mix(org_col, vec3<f32>(0.0), fog_amount);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // three's BoxGeometry uvs put v = 1 at the top of a face and three uploads
  // textures with `flipY = true`; the shared loader uploads unflipped (row 0 =
  // top = v 0), so sample with the v axis inverted to match the JS look.
  let tex = vec2<f32>(in.tex.x, 1.0 - in.tex.y);
  let viewdir = normalize(in.view_dir);
  let normal_surf = normalize(in.w_normal);
  // BoxGeometry has no tangents, so build a stable tangent frame around the
  // interpolated world normal (the axis pick avoids the degenerate Y case).
  let axis = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(normal_surf.y) < 0.99);
  let tangent = normalize(cross(normal_surf, axis));
  let bitangent = cross(normal_surf, tangent);

  let albedo_tex = textureSample(map_diffuse, map_diffuse_sampler, tex).rgb;
  let normal_tex = textureSample(map_normal, map_normal_sampler, tex).rgb * 2.0 - vec3<f32>(1.0);
  // Packed material: R = metallic, G = roughness, B = AO (three sibling maps).
  let material = vec3<f32>(
    textureSample(map_metallic, map_metallic_sampler, tex).r,
    textureSample(map_roughness, map_roughness_sampler, tex).g,
    textureSample(map_ao, map_ao_sampler, tex).r,
  );
  let roughness = max(u.in2.x, material.g);
  let normal = normalize(normal_tex.x * tangent + normal_tex.y * bitangent + normal_tex.z * normal_surf);
  let tint_col = p9_palette(
    0.75 + in.rnd.r * 0.25,
    vec3<f32>(0.5),
    vec3<f32>(0.1),
    vec3<f32>(1.0, 1.0, 1.0),
    vec3<f32>(0.2, 0.1, 0.7),
  );
  let albedo = albedo_tex * tint_col;
  let glowing_object = p9_smoothstep(0.91, 0.911, in.rnd.r);

  let real_albedo = mix(albedo * material.bbb, vec3<f32>(0.0), vec3<f32>(material.r));
  let real_specular = mix(vec3<f32>(0.04), albedo, vec3<f32>(material.r));

  var c = p9_light_directional(
    real_albedo,
    real_specular,
    normal,
    roughness,
    vec3<f32>(1.0) * 10.0,
    vec3<f32>(0.0, 0.0, -1.0),
    viewdir,
  );

  // Environment: the Stormy Days panorama through the same
  // `_texturePanoramicLod` the JS falls back to (`uEnvPanoramaMode = 1`).
  let env_dir = reflect(-viewdir, normal);
  let env_lod = roughness * roughness * 8.0;
  let env_color = p9_texture_panoramic_lod(map_environment, map_environment_sampler, env_dir, env_lod).rgb;
  let env_fresnel = p9_fresnel_roughness(real_specular, roughness, normal, viewdir);
  c = c + env_color * env_fresnel * vec3<f32>(0.5);

  if (in.rnd.r > 0.9) {
    c = c + p9_palette(
      in.rnd.g * 0.25,
      vec3<f32>(0.5),
      vec3<f32>(0.5),
      vec3<f32>(1.0, 1.0, 1.0),
      vec3<f32>(0.2, 0.1, 0.7),
    ) * glowing_object * (1.0 + u.in1.y);
  }
  c = simple_fog(c, length(in.view_dir), 0.3);
  c = c * p9_smoothstep(0.0, 1.0, -in.w_pos.z);
  // `vec4(c, 1.0)` in the JS; the clear alpha 0 is what keeps the background
  // transparent, and `preserveBloomAlpha` keeps it that way through the bloom.
  return vec4<f32>(c, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-boxy.test.js.

    #[test]
    fn perm_starts_at_the_preset_seed_and_falls_back() {
        assert_eq!(make_perm(1.0), 1.0);
        assert_eq!(make_perm(4.0), 4.0);
        assert_eq!(make_perm(f32::NAN), 1.0);
    }

    #[test]
    fn drive_maps_bass_to_in1_x_and_mid_to_in1_y() {
        let drive = boxy_drive(0.05, 0.02, 0.5, 1.0);

        assert!((drive.in1[0] - 1.0).abs() < 1e-5);
        assert!((drive.in1[1] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn drive_advances_perm_from_the_bass_capped_at_two_units_per_second() {
        let quiet = boxy_drive(0.05, 0.0, 0.1, 1.0);
        let loud = boxy_drive(1.0, 0.0, 0.1, 0.0);
        let louder = boxy_drive(10.0, 0.0, 0.1, 0.0);

        assert!((quiet.perm - (1.0 + (0.05_f32 * 20.0 + 0.2).min(2.0) * 0.1)).abs() < 1e-6);
        assert!((loud.perm - 0.2).abs() < 1e-6);
        assert!((louder.perm - loud.perm).abs() < 1e-6);
    }

    #[test]
    fn drive_creeps_in_silence_and_sanitises_negative_or_bad_input() {
        let silent = boxy_drive(0.0, 0.0, 0.1, 0.0);
        let negative = boxy_drive(-1.0, -2.0, -1.0, f32::NAN);
        let bad = boxy_drive(f32::NAN, f32::NAN, f32::NAN, f32::NAN);

        assert!((silent.perm - 0.02).abs() < 1e-6);
        assert!((negative.perm - 1.0).abs() < 1e-6);
        assert_eq!(negative.in1[0], 0.0);
        assert_eq!(negative.in1[1], 0.0);
        assert!((bad.perm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn drive_exposes_the_seeded_phase_and_the_roughness_floor() {
        let drive = boxy_drive(0.0, 0.0, 0.0, 3.0);

        assert!((drive.in1[2] - (3.0 + perm_rand() * 10.0)).abs() < 1e-5);
        assert_eq!(drive.in2, [0.3, 0.0, 0.0]);
        assert_eq!(drive.in2[0], ROUGHNESS_FLOOR);
        assert!((0.0..1.0).contains(&perm_rand()));
    }

    #[test]
    fn clears_to_a_fully_transparent_background() {
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(Boxy::new().clear_alpha(), 0.0);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(INSTANCE_COUNT, 1000);
        assert_eq!(CUBE_SIZE, 1.0);
        assert_eq!((CAMERA_FOV, CAMERA_NEAR, CAMERA_FAR), (70.0, 0.01, 100.0));
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (1.0, 0.85, 0.0)
        );
        assert_eq!(ROUGHNESS_FLOOR, 0.3);
        assert_eq!(MAX_DT, 0.1);
    }

    #[test]
    fn update_consumes_the_props_and_the_bands() {
        let mut scene = Boxy::new();
        scene.apply_props(|key| match key {
            "scrollSpeed" => Some(2.0),
            "glow" => Some(0.5),
            "roughness" => Some(0.6),
            _ => None,
        });

        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[3] = 0.1; // damping lane 1, bass (`band(0, 1, 0, 0)`)
        // `P9Audio.band(channel, damping, bandnr)`: the channel is ignored
        // (WebAudio FFT data is mono) and `bandnr` defaults to 0, so Boxy's
        // second call `band(1, 1, 0, 0)` reads the *bass* lane too. Set the
        // would-be mid lane to a different value and prove it is unused.
        payload.lanes[4] = 0.2;
        payload.byte_frequency_data = vec![0; 1024];
        payload.freq_mag = [0.5; crate::point10::audio_bridge::AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.1, &bridge);
        // `in1.x = bass * 20`, `in1.y = band(1,1,0,0) * 20 * glowScale` where
        // `bandnr` is 0, i.e. bass again, then scaled by the `glow` prop.
        assert!((scene.in1()[0] - 2.0).abs() < 1e-5);
        assert!((scene.in1()[1] - 2.0 * 0.5).abs() < 1e-5);
        // `dt * scrollScale` feeds the drive's step, capped at MAX_DT (0.1 s):
        // 0.1 s * 2.0 scrollSpeed still advances 0.1 s worth of tunnel.
        assert!((scene.perm() - (1.0 + (0.1_f32 * 20.0 + 0.2).min(2.0) * 0.1)).abs() < 1e-5);
        assert_eq!(scene.in2()[0], 0.6);
    }

    #[test]
    fn time_is_capped_where_the_js_caps_it() {
        let mut scene = Boxy::new();
        scene.update(4.0, &AudioBridge::new());
        // `boxyDrive` caps the step at MAX_DT independently of the frame clock.
        assert!((scene.perm() - (1.0 + 0.2 * MAX_DT)).abs() < 1e-6);
    }

    #[test]
    fn uniforms_are_112_bytes_and_the_mesh_is_24_vertices() {
        assert_eq!(std::mem::size_of::<BoxyUniforms>(), 112);
        let mesh = build_box(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE);
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = boxy_wgsl();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("BOXY_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("BOXY_WGSL must validate");
    }

    #[test]
    fn shader_places_the_cubes_from_the_instance_index() {
        let program = boxy_wgsl();
        for needle in [
            "@builtin(instance_index) instance: u32",
            "let d = instance_id / 1000.0;",
            "p9_rand3(d)",
            "p9_rand3(instance_id * 0.825931)",
            "p9_mod(d * 20.0 - u.in1.z, 20.0)",
            "vec3<f32>(0.1) + rnd.xyz * 0.2",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
        assert!(!program.contains("gl_InstanceID"));
    }

    #[test]
    fn shader_keeps_the_lighting_environment_and_fog_maths() {
        let program = boxy_wgsl();
        for needle in [
            "p9_palette(",
            "p9_light_directional(",
            "p9_fresnel_roughness(",
            "p9_texture_panoramic_lod(map_environment",
            "roughness * roughness * 8.0",
            "p9_smoothstep(0.91, 0.911, in.rnd.r)",
            "let tex = vec2<f32>(in.tex.x, 1.0 - in.tex.y);",
            "simple_fog(c, length(in.view_dir), 0.3)",
            "u.in2.x",
            "mix(albedo * material.bbb, vec3<f32>(0.0), vec3<f32>(material.r))",
            "textureSample(map_metallic, map_metallic_sampler, tex).r",
            "textureSample(map_roughness, map_roughness_sampler, tex).g",
            "textureSample(map_ao, map_ao_sampler, tex).r",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
    }

    #[test]
    fn shader_builds_a_tangent_frame_for_the_normal_map() {
        let program = boxy_wgsl();
        for needle in [
            "let tangent = normalize(cross(normal_surf, axis));",
            "normal_tex.x * tangent",
            "normal_tex.y * bitangent",
            "normal_tex.z * normal_surf",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = Boxy::new();
        assert_eq!(scene.meta().name, "Point10 Boxy");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 4..],
            &["scrollSpeed", "glow", "roughness", "bloom"]
        );
        assert_eq!(names[0], "gain");
    }
}
