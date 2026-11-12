//! three's `AfterimagePass` as a reusable wgpu pass.
//!
//! Port of `three/addons/postprocessing/AfterimagePass.js` +
//! `AfterimageShader.js`, the pass `LatentBackrooms.js:1605-1606` inserts
//! between `RenderPass` and `UnrealBloomPass`.
//!
//! # three's semantics
//!
//! `AfterimagePass` owns two full-size render targets and ping-pongs them:
//!
//! ```text
//! newFrame -- tNew ┐
//!                  ├─ max(tNew, tOld * damp * step(0.1, tOld)) ──▶ write target
//! history  -- tOld ┘                                              (swap)
//! ```
//!
//! i.e. a damped feedback trail: every frame the previous frame is multiplied
//! by `damp` (only where it is brighter than 0.1, so the black background does
//! not smear), then the current frame takes the component-wise maximum. `damp`
//! is the JS's `uniforms.damp` (`LatentBackrooms.js` sets it from
//! `0.48 + smHigh * 0.08 + flash * 0.04`, scaled by `trailScale`).
//!
//! The JS clears both history targets on activation (`LatentBackrooms.js:
//! 458-470`); [`AfterimagePass::reset`] is the equivalent, and the pass also
//! clears its targets on construction/resize because a fresh wgpu texture
//! holds undefined contents.
//!
//! # Why the ping-pong is explicit
//!
//! wgpu forbids binding a texture as both a sampled source and a render
//! attachment, so the pass can never do three's "render into the target it
//! samples" trick. It writes into the *other* history target and swaps, which
//! is the same double-buffer three's `AfterimagePass` keeps internally
//! (`textureOld` / `textureComp`).

#![forbid(unsafe_code)]

use super::{create_target, linear_sampler};
use crate::point10::FULLSCREEN_TRIANGLE_WGSL;

/// `AfterimageShader`'s default `damp` (`AfterimagePass` constructor default is
/// 0.96; LatentBackrooms passes 0.78).
pub const DEFAULT_DAMP: f32 = 0.96;

/// The feedback uniform block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AfterimageUniforms {
    /// `x` = damp; the rest is standard-layout padding.
    pub params: [f32; 4],
}

impl Default for AfterimageUniforms {
    fn default() -> Self {
        Self {
            params: [DEFAULT_DAMP, 0.0, 0.0, 0.0],
        }
    }
}

/// One ping-pong target.
struct History {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// A damped afterimage feedback pass.
pub struct AfterimagePass {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    uniforms_value: AfterimageUniforms,
    history: [History; 2],
    /// Index of the target holding the latest frame.
    read: usize,
    width: u32,
    height: u32,
    needs_clear: bool,
}

impl AfterimagePass {
    /// Build the pass at `width` x `height` with `damp` (clamped 0..1).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        damp: f32,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-postfx-afterimage-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
                    include_str!("afterimage.wgsl")
                )
                .into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-postfx-afterimage-layout"),
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
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-postfx-afterimage-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-postfx-afterimage-pipeline-layout"),
                    bind_group_layouts: &[Some(&bind_group_layout)],
                    immediate_size: 0,
                }),
            ),
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
                    format: crate::point10::HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-postfx-afterimage-uniforms"),
            size: std::mem::size_of::<AfterimageUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pass = Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            bind_group_layout,
            sampler: linear_sampler(device, "point10-postfx-afterimage-sampler"),
            uniforms,
            uniforms_value: AfterimageUniforms {
                params: [clamp_damp(damp), 0.0, 0.0, 0.0],
            },
            history: [
                make_history(device, width, height, 0),
                make_history(device, width, height, 1),
            ],
            read: 0,
            width,
            height,
            needs_clear: true,
        };
        pass.upload();
        pass
    }

    /// `uniforms.damp.value = …` — clamped to three's sensible 0..1 range.
    pub fn set_damp(&mut self, damp: f32) {
        self.uniforms_value.params[0] = clamp_damp(damp);
        self.upload();
    }

    /// The current damp.
    pub fn damp(&self) -> f32 {
        self.uniforms_value.params[0]
    }

    /// Clear the feedback history (`LatentBackrooms.onActivate`).
    pub fn reset(&mut self) {
        self.needs_clear = true;
    }

    /// Re-create both history targets for a new size.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.history = [
            make_history(&self.device, width, height, 0),
            make_history(&self.device, width, height, 1),
        ];
        self.read = 0;
        self.needs_clear = true;
    }

    /// The view of the latest feedback frame.
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.history[self.read].view
    }

    /// Run one feedback step: `source` is the frame that was just rendered.
    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::TextureView) {
        if self.needs_clear {
            // A fresh texture is undefined in wgpu; three explicitly clears both
            // targets on activation for the same reason.
            for index in 0..2 {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("point10-postfx-afterimage-clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.history[index].view,
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
            self.needs_clear = false;
        }

        let write = 1 - self.read;
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-postfx-afterimage-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.history[self.read].view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-postfx-afterimage"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.history[write].view,
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.read = write;
    }

    fn upload(&self) {
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&self.uniforms_value));
    }
}

/// three's `AfterimagePass` leaves `damp` unbounded in the shader; the scene's
/// own clamp (`LatentBackrooms.js:1883-1887`) is 0..0.95, and anything above 1
/// makes the feedback grow instead of decay, so the pass clamps to 0..1.
fn clamp_damp(damp: f32) -> f32 {
    if damp.is_finite() {
        damp.clamp(0.0, 1.0)
    } else {
        DEFAULT_DAMP
    }
}

fn make_history(device: &wgpu::Device, width: u32, height: u32, index: usize) -> History {
    let (texture, view) = create_target(
        device,
        &format!("point10-postfx-afterimage-{index}"),
        width,
        height,
    );
    History {
        _texture: texture,
        view,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_are_16_bytes() {
        assert_eq!(std::mem::size_of::<AfterimageUniforms>(), 16);
    }

    #[test]
    fn damp_is_clamped_and_sanitised() {
        assert_eq!(clamp_damp(0.78), 0.78);
        assert_eq!(clamp_damp(-1.0), 0.0);
        assert_eq!(clamp_damp(4.0), 1.0);
        assert_eq!(clamp_damp(f32::NAN), DEFAULT_DAMP);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = format!(
            "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
            include_str!("afterimage.wgsl")
        );
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("afterimage WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("afterimage WGSL must validate");
    }

    #[test]
    fn shader_keeps_the_three_semantics() {
        let source = include_str!("afterimage.wgsl");
        for needle in [
            "let damped = old * u.params.x * select(vec4<f32>(0.0), vec4<f32>(1.0), old > vec4<f32>(0.1));",
            "return max(new_sample, damped);",
        ] {
            assert!(source.contains(needle), "afterimage lost {needle}");
        }
    }
}
