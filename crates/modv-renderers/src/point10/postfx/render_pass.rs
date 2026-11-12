//! RenderPass — render a scene texture into an HDR target.
//!
//! # Why this is a blit and not a scene render
//!
//! In three, `RenderPass` is where the scene is drawn into the composer's read
//! buffer (`RenderPass.js:53-110`). The Rust scene contract already renders
//! into a target ([`crate::point10::Point10Scene::render`]), so by the time the
//! composer runs, the scene has been drawn. What remains is the copy three's
//! chain needs because it writes bloom back into the buffer it samples — in
//! wgpu that is illegal, so [`super::Composer`] keeps a `base` copy and the
//! combine pass writes `base + bloom` elsewhere.
//!
//! The pass is therefore a documented placeholder **and** a real, tested blit:
//! it copies the scene target into the composer's HDR base target, alpha and
//! all (the point of keeping alpha intact is that Point10 scenes clear to
//! transparent black and composite over the layers below).

#![forbid(unsafe_code)]

use super::linear_sampler;
use crate::point10::FULLSCREEN_TRIANGLE_WGSL;

/// Full-screen texture copy pass.
pub struct RenderPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
}

impl RenderPass {
    /// Build the pipeline for `target_format`.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-postfx-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{FULLSCREEN_TRIANGLE_WGSL}\n{}", include_str!("blit.wgsl")).into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-postfx-blit-layout"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-postfx-blit-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-postfx-blit-pipeline"),
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
                    format: target_format,
                    // A straight copy: keep the alpha exactly as the scene
                    // wrote it.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler: linear_sampler(device, "point10-postfx-blit-sampler"),
            bind_group: None,
        }
    }

    /// Point the pass at `source` (the composer's scene target). Call on build
    /// and after every resize, since the target views are re-created.
    pub fn set_source(&mut self, device: &wgpu::Device, source: &wgpu::TextureView) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-postfx-blit-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    /// Record the copy into `target` (no clear: the pass overwrites every
    /// pixel).
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-postfx-blit"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wgsl_parses() {
        let source = format!(
            "{}\n{}",
            crate::point10::FULLSCREEN_TRIANGLE_WGSL,
            include_str!("blit.wgsl")
        );
        if let Err(error) = naga::front::wgsl::parse_str(&source) {
            panic!("blit.wgsl failed to parse: {error}");
        }
    }
}
