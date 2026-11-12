//! The Point10 `EffectComposer`.
//!
//! Owns the render-target chain and drives it in the order the JS composer
//! does (`EffectComposer.js:render`):
//!
//! ```text
//! scene ──▶ scene_target          (scenes draw here, HDR)
//!              │ RenderPass (blit)
//!              ▼
//!            base_target ──┐
//!                          ├─ combine ──▶ hdr_target ──▶ OutputPass ──▶ destination
//!   bright ─▶ blur×5 ─▶ bloom composite ─┘
//! ```
//!
//! `RenderPass` is three's RenderPass, reduced to a copy: the Rust scene
//! already rendered into `scene_target` by the time the composer runs (see
//! [`super::render_pass`]). The `base` copy exists because three's bloom
//! blends its composite back into the texture it sampled; wgpu forbids that, so
//! the combine pass reads `base` + `bloom` and writes a third target with the
//! same `dst = src + bloom` arithmetic.
//!
//! # Sizing
//!
//! [`Composer::resize`] re-creates every target and re-binds the passes. The
//! scene must be told separately (its
//! [`crate::point10::Point10Scene::resize`]) — the composer does not own scene
//! state.
//!
//! # Preserving the frame alpha (`preserveBloomAlpha`)
//!
//! The default combine is three's additive bloom (`dst = base + bloom`, alpha
//! included). Scenes that clear to transparent and bloom with `threshold: 0`
//! (Wormhole, via `Boxy.js:preserveBloomAlpha`) need the bloom **colour**
//! without the bloom **alpha**, otherwise the transparent background turns
//! into an opaque veil. [`Composer::set_preserve_bloom_alpha`] switches the
//! combine pass to `fs_preserve_alpha`, which returns `base.rgb + bloom.rgb`
//! with `base.a` — the same arithmetic as `blendSrcAlpha = ZERO`,
//! `blendDstAlpha = ONE`.
//!
//! # Not here
//!
//! No texture pool from `modv-render`: `modv-renderers` does not depend on the
//! render crate (the engine owns the shared pool and can hand a `Composer` any
//! destination view). The composer's own targets are short-lived, per-instance
//! and small.

#![forbid(unsafe_code)]

use super::output_pass::{OutputPass, ToneMapping};
use super::render_pass::RenderPass;
use super::unreal_bloom_pass::UnrealBloomPass;
use super::{create_target, linear_sampler};
use crate::point10::{FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT};

/// Combines the base copy and the bloom composite into the HDR target.
///
/// Private because it is three's `blendMaterial` draw, not a pass a scene
/// configures.
struct CombinePass {
    pipeline: wgpu::RenderPipeline,
    /// `preserveBloomAlpha` variant (additive RGB, destination alpha kept).
    preserve_pipeline: wgpu::RenderPipeline,
    /// Whether [`CombinePass::preserve_pipeline`] is the active one.
    preserve_alpha: bool,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
}

impl CombinePass {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-postfx-combine-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
                    include_str!("bloom_combine.wgsl")
                )
                .into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-postfx-combine-layout"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-postfx-combine-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let build = |label: &str, entry_point: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry_point),
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
        };

        Self {
            pipeline: build("point10-postfx-combine-pipeline", "fs_main"),
            preserve_pipeline: build(
                "point10-postfx-combine-preserve-pipeline",
                "fs_preserve_alpha",
            ),
            preserve_alpha: false,
            bind_group_layout,
            sampler: linear_sampler(device, "point10-postfx-combine-sampler"),
            bind_group: None,
        }
    }

    /// Select the `preserveBloomAlpha` arithmetic.
    fn set_preserve_alpha(&mut self, preserve: bool) {
        self.preserve_alpha = preserve;
    }

    fn preserve_alpha(&self) -> bool {
        self.preserve_alpha
    }

    fn set_sources(
        &mut self,
        device: &wgpu::Device,
        base: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
    ) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-postfx-combine-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(base),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        let pipeline = if self.preserve_alpha {
            &self.preserve_pipeline
        } else {
            &self.pipeline
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-postfx-combine"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The post-processing chain for a Point10 scene.
pub struct Composer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
    output_format: wgpu::TextureFormat,

    // The texture handles are kept alongside their views so the composer
    // unambiguously owns its targets. wgpu views already keep their texture
    // alive, so the underscore names only document "owned, not read here".
    _scene_texture: wgpu::Texture,
    scene_view: wgpu::TextureView,
    _base_texture: wgpu::Texture,
    base_view: wgpu::TextureView,
    _hdr_texture: wgpu::Texture,
    hdr_view: wgpu::TextureView,
    blit: RenderPass,
    bloom: Option<UnrealBloomPass>,
    combine: CombinePass,
    output: OutputPass,
}

impl Composer {
    /// Build the chain for a `width` x `height` destination in `output_format`.
    ///
    /// Starts with bloom **disabled** ([`Composer::enable_bloom`] turns it on):
    /// only scenes that use an `EffectComposer` in the JS pay for it.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let (scene_texture, scene_view) =
            create_target(device, "point10-composer-scene", width, height);
        let (base_texture, base_view) =
            create_target(device, "point10-composer-base", width, height);
        let (hdr_texture, hdr_view) = create_target(device, "point10-composer-hdr", width, height);

        let mut blit = RenderPass::new(device, HDR_FORMAT);
        blit.set_source(device, &scene_view);

        // `WebGLRenderer` writes to an sRGB canvas, so the shader-side encode is
        // only needed when the destination is a plain UNORM format.
        let encode_srgb = !output_format.is_srgb();
        let mut output = OutputPass::new(device, queue, output_format, encode_srgb);
        output.set_source(device, &hdr_view);

        Self {
            device: device.clone(),
            queue: queue.clone(),
            width,
            height,
            output_format,
            _scene_texture: scene_texture,
            scene_view,
            _base_texture: base_texture,
            base_view,
            _hdr_texture: hdr_texture,
            hdr_view,
            blit,
            bloom: None,
            combine: CombinePass::new(device),
            output,
        }
    }

    /// The scene target's format — scenes must be initialised with this.
    pub fn scene_format() -> wgpu::TextureFormat {
        HDR_FORMAT
    }

    /// The view a scene renders into.
    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    /// Enable bloom with three's parameters (`UnrealBloomPass(size, strength,
    /// radius, threshold)` — e.g. `Mycelium.js:414`, `GeometricTunnel.js:510`).
    pub fn enable_bloom(&mut self, strength: f32, radius: f32, threshold: f32) {
        let mut bloom = UnrealBloomPass::new(&self.device, &self.queue, self.width, self.height);
        bloom.set_strength(strength);
        bloom.set_radius(radius);
        bloom.set_threshold(threshold);
        bloom.set_input(&self.base_view);
        self.bloom = Some(bloom);
        self.rebind();
    }

    /// Use `preserveBloomAlpha`'s arithmetic for the bloom combine
    /// (`Boxy.js:73-86`; Wormhole is the batch-2 scene that needs it).
    ///
    /// three's additive bloom composite also adds the bloom RGB into the
    /// destination **alpha**. With `threshold: 0` every pixel receives bloom,
    /// so a scene that clears to transparent black would get an opaque veil
    /// laid over it and lose its layer alpha. `preserveBloomAlpha` keeps the
    /// destination alpha while still adding the bloom colour; the Rust
    /// equivalent is [`CombinePass`]'s `fs_preserve_alpha` entry.
    pub fn set_preserve_bloom_alpha(&mut self, preserve: bool) {
        self.combine.set_preserve_alpha(preserve);
    }

    /// Whether the combine pass preserves the destination alpha.
    pub fn preserve_bloom_alpha(&self) -> bool {
        self.combine.preserve_alpha()
    }

    /// Turn bloom off again (the chain becomes blit → output).
    pub fn disable_bloom(&mut self) {
        self.bloom = None;
        self.rebind();
    }

    /// Mutable access to the bloom pass (setting strength/radius/threshold).
    pub fn bloom_mut(&mut self) -> Option<&mut UnrealBloomPass> {
        self.bloom.as_mut()
    }

    /// Read-only access to the bloom pass.
    pub fn bloom(&self) -> Option<&UnrealBloomPass> {
        self.bloom.as_ref()
    }

    /// Mutable access to the output pass (tone mapping, exposure).
    pub fn output_mut(&mut self) -> &mut OutputPass {
        &mut self.output
    }

    /// Override the final sRGB transfer (see
    /// [`OutputPass::set_encode_srgb`]).
    pub fn set_encode_srgb(&mut self, encode_srgb: bool) {
        self.output.set_encode_srgb(encode_srgb);
    }

    /// The active tone mapping operator.
    pub fn tone_mapping(&self) -> ToneMapping {
        self.output.tone_mapping()
    }

    /// Destination size.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Destination format.
    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    /// Re-create every target for a new size and re-bind the passes.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;

        let (scene_texture, scene_view) =
            create_target(&self.device, "point10-composer-scene", width, height);
        let (base_texture, base_view) =
            create_target(&self.device, "point10-composer-base", width, height);
        let (hdr_texture, hdr_view) =
            create_target(&self.device, "point10-composer-hdr", width, height);
        self._scene_texture = scene_texture;
        self.scene_view = scene_view;
        self._base_texture = base_texture;
        self.base_view = base_view;
        self._hdr_texture = hdr_texture;
        self.hdr_view = hdr_view;

        if let Some(bloom) = self.bloom.as_mut() {
            bloom.resize(width, height);
        }
        self.rebind();
    }

    /// Render the chain: the caller has already drawn the scene into
    /// [`Composer::scene_view`].
    pub fn render(&mut self, encoder: &mut wgpu::CommandEncoder, destination: &wgpu::TextureView) {
        if let Some(bloom) = self.bloom.as_mut() {
            // Scene -> base copy (three's RenderPass), then the bloom chain
            // (its bind groups are refreshed by `rebind`), then
            // `base + bloom` into the HDR target.
            self.blit.encode(encoder, &self.base_view);
            bloom.encode(encoder);
            if bloom.composite_view().is_some() {
                self.combine.encode(encoder, &self.hdr_view);
            }
        } else {
            // No bloom: blit straight into the HDR target.
            self.blit.encode(encoder, &self.hdr_view);
        }
        self.output.encode(encoder, destination);
    }

    /// Re-point every pass at the current target views.
    fn rebind(&mut self) {
        self.blit.set_source(&self.device, &self.scene_view);
        self.output.set_source(&self.device, &self.hdr_view);
        if let Some(bloom) = self.bloom.as_mut() {
            bloom.set_input(&self.base_view);
            if let Some(composite) = bloom.composite_view() {
                self.combine
                    .set_sources(&self.device, &self.base_view, composite);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn combine_wgsl_parses() {
        let source = format!(
            "{}\n{}",
            crate::point10::FULLSCREEN_TRIANGLE_WGSL,
            include_str!("bloom_combine.wgsl")
        );
        if let Err(error) = naga::front::wgsl::parse_str(&source) {
            panic!("bloom_combine.wgsl failed to parse: {error}");
        }
    }
}
