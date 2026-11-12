//! OutputPass — tone mapping plus the sRGB transfer on the final blit.
//!
//! Port of `examples/jsm/postprocessing/OutputPass.js` +
//! `examples/jsm/shaders/OutputShader.js`. OutputPass copies the composer's
//! read buffer, applies `renderer.toneMapping` and then
//! `renderer.outputColorSpace` (`OutputPass.js:106-123`,
//! `OutputShader.js:60-105`).
//!
//! # Which tone mapping the scenes see
//!
//! `WebGLRenderer`'s default is `NoToneMapping`, and the Point10 adapter never
//! changes it (`adapter.js:287-297`) — so [`ToneMapping::None`] is the default
//! here too and batch-1 scenes are pixel-faithful. The composer-using scenes
//! opt into `ACESFilmicToneMapping` explicitly, exactly like their JS
//! counterparts (`Earth.js:530`, `LightTunnel.js:134`, `SmokeRoom.js:417`, …)
//! and can set [`ToneMapping::AcesFilmic`] on the [`OutputPass`].
//!
//! # sRGB
//!
//! three's renderer writes to a canvas that WebGL marks as sRGB, so
//! `OutputPass` applies `sRGBTransferOETF`. In wgpu the equivalent depends on
//! the destination format:
//!
//! * an `*Srgb` format does the transfer in hardware, so the shader must emit
//!   **linear** values ([`OutputPass::encode_srgb`] off);
//! * a plain `*Unorm` format needs the transfer in the shader
//!   ([`OutputPass::encode_srgb`] on).
//!
//! [`super::Composer`] picks automatically from the destination format.

#![forbid(unsafe_code)]

use super::linear_sampler;
use crate::point10::FULLSCREEN_TRIANGLE_WGSL;

/// Tone mapping operators, matching three's `ToneMapping` constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneMapping {
    /// `NoToneMapping` — pass the colour through.
    None,
    /// `LinearToneMapping` — `saturate(exposure * color)`.
    Linear,
    /// `ReinhardToneMapping`.
    Reinhard,
    /// `CineonToneMapping` (Hejl / Burgess-Dawson filmic).
    Cineon,
    /// `ACESFilmicToneMapping` (RRT+ODT fit, 1/0.6 viewing scale).
    AcesFilmic,
}

impl ToneMapping {
    /// The uniform value `output.wgsl` switches on.
    pub fn mode(self) -> u32 {
        match self {
            ToneMapping::None => 0,
            ToneMapping::Linear => 1,
            ToneMapping::Reinhard => 2,
            ToneMapping::Cineon => 3,
            ToneMapping::AcesFilmic => 4,
        }
    }
}

/// Output uniforms (`output.wgsl`), 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct OutputUniforms {
    exposure: f32,
    mode: u32,
    srgb_encode: u32,
    _pad: f32,
}

/// Final blit with tone mapping and the sRGB transfer.
pub struct OutputPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    queue: wgpu::Queue,
    bind_group: Option<wgpu::BindGroup>,
    tone_mapping: ToneMapping,
    exposure: f32,
    encode_srgb: bool,
}

impl OutputPass {
    /// Build the pass.
    ///
    /// `output_format` is the destination view's format. `encode_srgb` selects
    /// the shader-side sRGB transfer — pass `false` when `output_format` is an
    /// `*Srgb` format.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
        encode_srgb: bool,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-postfx-output-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
                    include_str!("output.wgsl")
                )
                .into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-postfx-output-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
            label: Some("point10-postfx-output-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-postfx-output-pipeline"),
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
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    // The destination is the output window: replace, never
                    // blend (three's OutputPass draws with blending off).
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-postfx-output-uniforms"),
            size: std::mem::size_of::<OutputUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pass = Self {
            pipeline,
            bind_group_layout,
            sampler: linear_sampler(device, "point10-postfx-output-sampler"),
            uniform_buffer,
            queue: queue.clone(),
            bind_group: None,
            tone_mapping: ToneMapping::None,
            exposure: 1.0,
            encode_srgb,
        };
        pass.upload_uniforms();
        pass
    }

    /// Build the pass with ACES filmic tone mapping — the operator the
    /// composer-using scenes set (`Earth.js:530`).
    pub fn with_aces(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
        encode_srgb: bool,
    ) -> Self {
        let mut pass = Self::new(device, queue, output_format, encode_srgb);
        pass.set_tone_mapping(ToneMapping::AcesFilmic);
        pass
    }

    /// The active tone mapping operator.
    pub fn tone_mapping(&self) -> ToneMapping {
        self.tone_mapping
    }

    /// `renderer.toneMapping` / `renderer.toneMappingExposure = 1`.
    pub fn set_tone_mapping(&mut self, tone_mapping: ToneMapping) {
        self.tone_mapping = tone_mapping;
        self.upload_uniforms();
    }

    /// `renderer.toneMappingExposure`.
    pub fn exposure(&self) -> f32 {
        self.exposure
    }

    /// Set `renderer.toneMappingExposure`.
    pub fn set_exposure(&mut self, exposure: f32) {
        self.exposure = exposure;
        self.upload_uniforms();
    }

    /// Whether the shader applies the sRGB transfer.
    pub fn encode_srgb(&self) -> bool {
        self.encode_srgb
    }

    /// Override the shader-side sRGB transfer.
    ///
    /// [`Composer`](super::Composer) picks it from the destination format; a
    /// scene whose JS counterpart has no `OutputPass` (Mycelium) turns it off
    /// so its display-referred values are not gamma-lifted a second time.
    pub fn set_encode_srgb(&mut self, encode_srgb: bool) {
        self.encode_srgb = encode_srgb;
        self.upload_uniforms();
    }

    /// Point the pass at the composer's final HDR target.
    pub fn set_source(&mut self, device: &wgpu::Device, source: &wgpu::TextureView) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-postfx-output-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    /// Record the final blit into `destination`.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, destination: &wgpu::TextureView) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-postfx-output"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn upload_uniforms(&self) {
        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&OutputUniforms {
                exposure: self.exposure,
                mode: self.tone_mapping.mode(),
                srgb_encode: u32::from(self.encode_srgb),
                _pad: 0.0,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_mapping_modes_match_the_shader_switch() {
        assert_eq!(ToneMapping::None.mode(), 0);
        assert_eq!(ToneMapping::Linear.mode(), 1);
        assert_eq!(ToneMapping::Reinhard.mode(), 2);
        assert_eq!(ToneMapping::Cineon.mode(), 3);
        assert_eq!(ToneMapping::AcesFilmic.mode(), 4);
    }

    #[test]
    fn uniform_block_is_16_bytes() {
        assert_eq!(std::mem::size_of::<OutputUniforms>(), 16);
    }

    #[test]
    fn wgsl_parses() {
        let source = format!(
            "{}\n{}",
            crate::point10::FULLSCREEN_TRIANGLE_WGSL,
            include_str!("output.wgsl")
        );
        if let Err(error) = naga::front::wgsl::parse_str(&source) {
            panic!("output.wgsl failed to parse: {error}");
        }
    }
}
