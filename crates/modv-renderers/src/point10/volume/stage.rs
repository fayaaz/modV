//! The render chain the two volume scenes share — port of SmokeRoom's
//! `_initVolume`/`_animate` composer (`SmokeRoom.js:1235-1279`).
//!
//! ```text
//! glass + logo/sphere ──▶ scene_target (HDR, depth)
//!                              │
//! dye/velocity grid ──▶ raymarch (half res) ──▶ gaussian blur (half res)
//!                              │                     │
//!                              └──── combine ────────┘─▶ combined (HDR)
//!                                        │
//!                         UnrealBloomPass(combined) ──▶ final (+ bloom)
//!                                        │              alpha = max(rgb)
//!                                        └──▶ OutputPass (ACES, exposure 2)
//! ```
//!
//! The JS chain is `scenePass → volumetricPass (0.5 resolution scale) →
//! gaussianBlur → max/add combine → bloom(scenePassColor) →
//! outputNode(scenePassColor + bloom)` with `ACESFilmicToneMapping` and
//! `toneMappingExposure = 2`. Every step is kept: the volume light is added on
//! top of the geometry, the bloom threshold (0.5) and radius (1) are the JS's,
//! and the final alpha is `max(rgb)` when the scene sets `_transparentOverlay`
//! (all four volume scenes do) so empty pixels stay transparent for the layer
//! stack; otherwise the stage writes an opaque frame.
//!
//! # Bloom
//!
//! The JS calls three's TSL `bloom()` node (`BloomNode`), not the classic
//! `UnrealBloomPass` this crate ports for the WebGL scenes. They share the
//! bright-pass, the 5 progressively halved gaussian mips and the
//! `lerpBloomFactor(factor, radius)` weights, but `BloomNode` composites
//! `sum(mips) * strength` while the classic pass adds a historical `3 *`
//! multiplier. The stage therefore drives [`UnrealBloomPass`] with
//! [`UnrealBloomPass::set_bloom_scale`] set to `1.0`, so the scenes' own
//! strengths mean the same thing here as in the JS
//! (`min(0.12 + 2.2 * bass, 2.8)` for SmokeRoom/DvdRoom,
//! `min(0.12 + 3.6 * bass, 4)` for RollingFire/BallOfFire).
//!
//! The stage owns the size-dependent targets, the bloom pass and the
//! [`Composer`] that presents the tone-mapped frame, so the scenes only hand it
//! their geometry pass and the volumes' [super::VolumeSim].

#![forbid(unsafe_code)]

use super::VolumeSim;
use crate::point10::postfx::{Composer, ToneMapping, UnrealBloomPass};
use crate::point10::{FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT};

/// `volumetricPass.setResolutionScale(0.5)` (`SmokeRoom.js:1254`).
pub const VOLUMETRIC_RESOLUTION_SCALE: f32 = 0.5;
/// `bloomPass.threshold.value = 0.5` (`SmokeRoom.js:1266`).
pub const BLOOM_THRESHOLD: f32 = 0.5;
/// `bloomPass.radius.value = 1.0` (`SmokeRoom.js:1267`).
pub const BLOOM_RADIUS: f32 = 1.0;

/// A size-dependent HDR target.
struct TextureTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl TextureTarget {
    fn new(device: &wgpu::Device, label: &str, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

/// The half-resolution volumetric target size for a full-resolution frame.
pub fn volumetric_size(width: u32, height: u32) -> (u32, u32) {
    let scale = VOLUMETRIC_RESOLUTION_SCALE;
    (
        ((width as f32 * scale) as u32).max(1),
        ((height as f32 * scale) as u32).max(1),
    )
}

/// Uniform block of the blur pass.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniforms {
    resolution: [f32; 2],
    texel: [f32; 2],
}

/// Uniform block of the combine pass.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CombineUniforms {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// Uniform block of the final alpha pass.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FinalUniforms {
    resolution: [f32; 2],
    /// 1 = `_transparentOverlay` (alpha = max(rgb)); 0 = opaque output.
    overlay: f32,
    _pad: f32,
}

/// The shared geometry → volume → bloom → output chain.
pub struct VolumeStage {
    size: (u32, u32),
    /// `_transparentOverlay`: alpha = max(rgb) when true, 1 when false.
    overlay: bool,
    scene: TextureTarget,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    volume: TextureTarget,
    blur: TextureTarget,
    combined: TextureTarget,
    sampler: wgpu::Sampler,
    blur_pipeline: wgpu::RenderPipeline,
    blur_uniforms: wgpu::Buffer,
    blur_layout: wgpu::BindGroupLayout,
    blur_bind_group: wgpu::BindGroup,
    combine_pipeline: wgpu::RenderPipeline,
    combine_uniforms: wgpu::Buffer,
    combine_layout: wgpu::BindGroupLayout,
    combine_bind_group: wgpu::BindGroup,
    final_pipeline: wgpu::RenderPipeline,
    final_uniforms: wgpu::Buffer,
    final_layout: wgpu::BindGroupLayout,
    final_bind_group: wgpu::BindGroup,
    bloom: UnrealBloomPass,
    composer: Composer,
}

impl VolumeStage {
    /// Build the chain at `width x height`, presenting into `target_format`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let size = (width, height);
        let (volume_width, volume_height) = volumetric_size(width, height);

        let scene = TextureTarget::new(device, "point10-volume-scene", width, height);
        let volume = TextureTarget::new(
            device,
            "point10-volume-raymarch-target",
            volume_width,
            volume_height,
        );
        let blur = TextureTarget::new(
            device,
            "point10-volume-blur-target",
            volume_width,
            volume_height,
        );
        let combined = TextureTarget::new(device, "point10-volume-combined", width, height);
        let (depth, depth_view) = crate::point10::scenes::cube_mesh::create_depth(
            device,
            "point10-volume-depth",
            width,
            height,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-volume-stage-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let blur_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-volume-blur-uniforms"),
            size: std::mem::size_of::<BlurUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let combine_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-volume-combine-uniforms"),
            size: std::mem::size_of::<CombineUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let final_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-volume-final-uniforms"),
            size: std::mem::size_of::<FinalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-blur-layout"),
            entries: &[uniform_entry(0), texture_entry(1), sampler_entry(2)],
        });
        let combine_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-combine-layout"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let final_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-final-layout"),
            entries: &[
                uniform_entry(0),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });

        let blur_pipeline = fullscreen(
            device,
            "point10-volume-blur",
            include_str!("passes/blur.wgsl"),
            &blur_layout,
        );
        let combine_pipeline = fullscreen(
            device,
            "point10-volume-combine",
            include_str!("passes/combine.wgsl"),
            &combine_layout,
        );
        let final_pipeline = fullscreen(
            device,
            "point10-volume-final",
            include_str!("passes/final.wgsl"),
            &final_layout,
        );

        let blur_bind_group =
            create_blur_bind_group(device, &blur_layout, &blur_uniforms, &volume.view, &sampler);
        let combine_bind_group = create_combine_bind_group(
            device,
            &combine_layout,
            &combine_uniforms,
            &scene.view,
            &blur.view,
            &sampler,
        );

        let mut bloom = UnrealBloomPass::new(device, queue, width, height);
        bloom.set_strength(super::room::BLOOM_BASE);
        bloom.set_radius(BLOOM_RADIUS);
        bloom.set_threshold(BLOOM_THRESHOLD);
        // SmokeRoom calls the TSL `bloom()` node, whose composite is
        // `sum(mips) * strength`; the classic UnrealBloomPass (this pass's
        // default) multiplies by 3 as well. Without this the volume scene
        // blooms three times as hard as the JS.
        bloom.set_bloom_scale(1.0);
        bloom.set_input(&combined.view);

        let mut composer = Composer::new(device, queue, width, height, target_format);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);
        composer
            .output_mut()
            .set_exposure(super::room::TONE_MAPPING_EXPOSURE);

        let final_bind_group = create_final_bind_group(
            device,
            &final_layout,
            &final_uniforms,
            &combined.view,
            bloom
                .composite_view()
                .expect("the bloom pass always has a composite target"),
            &sampler,
        );

        let stage = Self {
            size,
            overlay: true,
            scene,
            _depth: depth,
            depth_view,
            volume,
            blur,
            combined,
            sampler,
            blur_pipeline,
            blur_uniforms,
            blur_layout,
            blur_bind_group,
            combine_pipeline,
            combine_uniforms,
            combine_layout,
            combine_bind_group,
            final_pipeline,
            final_uniforms,
            final_layout,
            final_bind_group,
            bloom,
            composer,
        };
        stage.upload_uniforms(queue);
        stage
    }

    /// The full-resolution frame size.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The half-resolution raymarch target size (`view.zw` of the volume
    /// uniforms).
    pub fn volume_size(&self) -> (u32, u32) {
        volumetric_size(self.size.0, self.size.1)
    }

    /// Set the output alpha rule (`_transparentOverlay`): `true` writes
    /// `alpha = max(rgb)` so empty pixels stay clear for the layer stack;
    /// `false` forces an opaque frame.
    pub fn set_overlay(&mut self, queue: &wgpu::Queue, overlay: bool) {
        if self.overlay == overlay {
            return;
        }
        self.overlay = overlay;
        self.write_final_uniforms(queue);
    }

    /// The HDR geometry target the scene draws glass/logo/sphere into.
    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene.view
    }

    /// The depth attachment matching [`VolumeStage::scene_view`].
    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }

    /// Recreate the size-dependent targets and rebind the chain.
    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.size = (width, height);
        let (volume_width, volume_height) = volumetric_size(width, height);

        self.scene = TextureTarget::new(device, "point10-volume-scene", width, height);
        self.volume = TextureTarget::new(
            device,
            "point10-volume-raymarch-target",
            volume_width,
            volume_height,
        );
        self.blur = TextureTarget::new(
            device,
            "point10-volume-blur-target",
            volume_width,
            volume_height,
        );
        self.combined = TextureTarget::new(device, "point10-volume-combined", width, height);
        let (depth, depth_view) = crate::point10::scenes::cube_mesh::create_depth(
            device,
            "point10-volume-depth",
            width,
            height,
        );
        self._depth = depth;
        self.depth_view = depth_view;

        self.blur_bind_group = create_blur_bind_group(
            device,
            &self.blur_layout,
            &self.blur_uniforms,
            &self.volume.view,
            &self.sampler,
        );
        self.combine_bind_group = create_combine_bind_group(
            device,
            &self.combine_layout,
            &self.combine_uniforms,
            &self.scene.view,
            &self.blur.view,
            &self.sampler,
        );
        self.bloom.resize(width, height);
        self.bloom.set_input(&self.combined.view);
        self.composer.resize(width, height);
        self.final_bind_group = create_final_bind_group(
            device,
            &self.final_layout,
            &self.final_uniforms,
            &self.combined.view,
            self.bloom
                .composite_view()
                .expect("the bloom pass always has a composite target"),
            &self.sampler,
        );
        self.upload_uniforms(queue);
    }

    /// Begin the geometry pass: clears the HDR scene target to transparent and
    /// the depth buffer to 1.
    pub fn begin_scene<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-volume-geometry"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.scene.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// Record raymarch → blur → combine → bloom → final → tone map into
    /// `target`.
    pub fn encode_post(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        sim: &VolumeSim,
        bloom_strength: f32,
        target: &wgpu::TextureView,
    ) {
        sim.render(encoder, &self.volume.view);

        {
            let mut pass = clear_pass(encoder, &self.blur.view, "point10-volume-blur");
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &self.blur_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = clear_pass(encoder, &self.combined.view, "point10-volume-combine");
            pass.set_pipeline(&self.combine_pipeline);
            pass.set_bind_group(0, &self.combine_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.bloom.set_strength(bloom_strength);
        self.bloom.encode(encoder);

        {
            let mut pass = clear_pass(encoder, self.composer.scene_view(), "point10-volume-final");
            pass.set_pipeline(&self.final_pipeline);
            pass.set_bind_group(0, &self.final_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.composer.render(encoder, target);
    }

    fn upload_uniforms(&self, queue: &wgpu::Queue) {
        let (volume_width, volume_height) = self.volume_size();
        queue.write_buffer(
            &self.blur_uniforms,
            0,
            bytemuck::bytes_of(&BlurUniforms {
                resolution: [volume_width as f32, volume_height as f32],
                texel: [1.0 / volume_width as f32, 1.0 / volume_height as f32],
            }),
        );
        queue.write_buffer(
            &self.combine_uniforms,
            0,
            bytemuck::bytes_of(&CombineUniforms {
                resolution: [self.size.0 as f32, self.size.1 as f32],
                _pad: [0.0; 2],
            }),
        );
        self.write_final_uniforms(queue);
    }

    fn write_final_uniforms(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.final_uniforms,
            0,
            bytemuck::bytes_of(&FinalUniforms {
                resolution: [self.size.0 as f32, self.size.1 as f32],
                overlay: if self.overlay { 1.0 } else { 0.0 },
                _pad: 0.0,
            }),
        );
    }
}

fn clear_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    label: &str,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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
    })
}

fn fullscreen(
    device: &wgpu::Device,
    label: &str,
    body: &str,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(format!("{FULLSCREEN_TRIANGLE_WGSL}\n{body}").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
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
            module: &module,
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
    })
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

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

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn create_blur_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    source: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-volume-blur-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn create_combine_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    scene: &wgpu::TextureView,
    volume: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-volume-combine-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(scene),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(volume),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn create_final_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    combined: &wgpu::TextureView,
    bloom: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-volume-final-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(combined),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(bloom),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// A single-uniform material: one pipeline, its uniform buffer and bind group.
///
/// Both volume scenes draw a handful of meshes with a shader that reads one
/// uniform block and nothing else; this wraps the four wgpu objects that
/// boilerplate needs.
pub struct Material {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Material {
    /// Build a pipeline from `shader_source` (which must declare `vs_main` and
    /// `fs_main`, with all of `uniform_size` bytes bound at `group(0)
    /// binding(0)`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        label: &str,
        shader_source: &str,
        uniform_size: u64,
        vertex_layout: Option<wgpu::VertexBufferLayout<'static>>,
        primitive: wgpu::PrimitiveState,
        depth_stencil: Option<wgpu::DepthStencilState>,
        blend: Option<wgpu::BlendState>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let buffers: Vec<Option<wgpu::VertexBufferLayout<'static>>> =
            vertex_layout.map(Some).into_iter().collect();
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: buffers.as_slice(),
            },
            primitive,
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            uniforms,
            bind_group,
        }
    }

    /// Upload this frame's uniform block.
    pub fn write(&self, queue: &wgpu::Queue, bytes: &[u8]) {
        queue.write_buffer(&self.uniforms, 0, bytes);
    }

    /// Draw `mesh` with this material.
    pub fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        mesh: &'pass crate::point10::scenes::cube_mesh::MeshGpu,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        mesh.draw(pass);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volumetric_pass_runs_at_half_resolution() {
        assert_eq!(VOLUMETRIC_RESOLUTION_SCALE, 0.5);
        assert_eq!(volumetric_size(256, 256), (128, 128));
        assert_eq!(volumetric_size(1920, 1080), (960, 540));
        // Never zero, even for a 1x1 frame.
        assert_eq!(volumetric_size(1, 1), (1, 1));
        assert_eq!(volumetric_size(0, 0), (1, 1));
    }

    #[test]
    fn the_bloom_and_final_uniforms_are_16_bytes() {
        assert_eq!(std::mem::size_of::<BlurUniforms>(), 16);
        assert_eq!(std::mem::size_of::<CombineUniforms>(), 16);
        assert_eq!(std::mem::size_of::<FinalUniforms>(), 16);
        assert_eq!(BLOOM_THRESHOLD, 0.5);
        assert_eq!(BLOOM_RADIUS, 1.0);
    }

    #[test]
    fn the_final_pass_applies_the_alpha_rule() {
        let final_shader = include_str!("passes/final.wgsl");
        assert!(
            final_shader.contains("clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0)"),
            "the final pass must write alpha = max(rgb)"
        );
        assert!(final_shader.contains("select(1.0, overlay_alpha, u.overlay > 0.5)"));
        assert!(final_shader.contains("scene.rgb + bloom.rgb"));
    }
}
