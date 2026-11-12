//! Generic full-screen shader pass: one source texture + one uniform block.
//!
//! This is the Rust stand-in for three's `ShaderPass` — the pass scenes insert
//! between `RenderPass` and `OutputPass` for their own grade/effect shader
//! (`new ShaderPass(SomeShader)` in e.g. `LatentBackrooms.js`). The shader
//! contract is:
//!
//! ```wgsl
//! // Prelude (provided by ShaderPass::new):
//! struct ShaderPassUniforms {
//!   resolution: vec2<f32>, time: f32, _pad: f32, params: vec4<f32>,
//! }
//! @group(0) @binding(0) var<uniform> u_pass: ShaderPassUniforms;
//! @group(0) @binding(1) var pass_source: texture_2d<f32>;
//! @group(0) @binding(2) var pass_sampler: sampler;
//!
//! // The scene supplies only this:
//! @fragment
//! fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> { … }
//! ```
//!
//! The source is sampled with `pass_sampler` (clamp-to-edge, linear) and the uv
//! is `position.xy / u_pass.resolution`; the full-screen vertex shader comes
//! from [`crate::point10::FULLSCREEN_TRIANGLE_WGSL`].
//!
//! `params` is the generic four-lane slider bag a `ShaderPass`'s uniforms
//! object carries; `time` mirrors the JS `time` uniform. A pass that needs more
//! than four values (or several textures) gets its own pipeline, like
//! [`super::UnrealBloomPass`] does.

#![forbid(unsafe_code)]

use super::linear_sampler;
use crate::point10::FULLSCREEN_TRIANGLE_WGSL;

/// The uniform block every [`ShaderPass`] shader sees, 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShaderPassUniforms {
    /// Destination size in pixels (`resolution` in the legacy shaders).
    pub resolution: [f32; 2],
    /// Seconds since the pass was created (`time`).
    pub time: f32,
    /// Standard-layout padding.
    pub _pad: f32,
    /// Four generic sliders (`vec4` uniforms in the legacy shaders).
    pub params: [f32; 4],
}

impl Default for ShaderPassUniforms {
    fn default() -> Self {
        Self {
            resolution: [1.0, 1.0],
            time: 0.0,
            _pad: 0.0,
            params: [0.0; 4],
        }
    }
}

/// The WGSL prelude [`ShaderPass::new`] prepends to the supplied fragment.
pub const SHADER_PASS_PRELUDE: &str = "\
struct ShaderPassUniforms {
  resolution: vec2<f32>, time: f32, _pad: f32, params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u_pass: ShaderPassUniforms;
@group(0) @binding(1) var pass_source: texture_2d<f32>;
@group(0) @binding(2) var pass_sampler: sampler;
";

/// A full-screen textured pass with a user fragment shader.
pub struct ShaderPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    queue: wgpu::Queue,
    uniforms: ShaderPassUniforms,
    bind_group: Option<wgpu::BindGroup>,
    /// Kept so the tests can assert the composed program is the one asked for.
    source: String,
}

impl ShaderPass {
    /// Build the pass from a fragment entry (`fs_main`) that uses
    /// [`SHADER_PASS_PRELUDE`].
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        label: &str,
        fragment_wgsl: &str,
    ) -> Self {
        let source = format!("{FULLSCREEN_TRIANGLE_WGSL}\n{SHADER_PASS_PRELUDE}\n{fragment_wgsl}");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.clone().into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
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
            label: Some(label),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<ShaderPassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pass = Self {
            pipeline,
            bind_group_layout,
            sampler: linear_sampler(device, label),
            uniform_buffer,
            queue: queue.clone(),
            uniforms: ShaderPassUniforms::default(),
            bind_group: None,
            source,
        };
        pass.upload();
        pass
    }

    /// The composed WGSL (vertex + prelude + fragment) — for diagnostics.
    pub fn composed_wgsl(&self) -> &str {
        &self.source
    }

    /// Set the source texture and the destination size.
    pub fn set_source(
        &mut self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.uniforms.resolution = [width.max(1) as f32, height.max(1) as f32];
        self.upload();
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-postfx-shader-pass-bind-group"),
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

    /// Advance the `time` uniform.
    pub fn set_time(&mut self, time: f32) {
        self.uniforms.time = time;
        self.upload();
    }

    /// Set the four generic slider values.
    pub fn set_params(&mut self, params: [f32; 4]) {
        self.uniforms.params = params;
        self.upload();
    }

    /// Record the pass into `target`.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-postfx-shader-pass"),
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

    fn upload(&self) {
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vignette-ish pass exercising the prelude's texture + uniforms.
    const TEST_FRAGMENT: &str = "\
@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u_pass.resolution;
  let texel = textureSample(pass_source, pass_sampler, uv);
  let d = length(uv - vec2<f32>(0.5));
  return vec4<f32>(texel.rgb * u_pass.params.x * (1.0 - d), texel.a);
}
";

    #[test]
    fn composed_program_parses_and_validates() {
        let pass_source =
            format!("{FULLSCREEN_TRIANGLE_WGSL}\n{SHADER_PASS_PRELUDE}\n{TEST_FRAGMENT}");
        let module = match naga::front::wgsl::parse_str(&pass_source) {
            Ok(module) => module,
            Err(error) => panic!("shader pass program failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("shader pass program must validate");
    }

    #[test]
    fn uniforms_are_32_bytes() {
        assert_eq!(std::mem::size_of::<ShaderPassUniforms>(), 32);
    }

    #[test]
    fn prelude_declares_the_documented_bindings() {
        for needle in [
            "@group(0) @binding(0) var<uniform> u_pass",
            "@group(0) @binding(1) var pass_source: texture_2d<f32>",
            "@group(0) @binding(2) var pass_sampler: sampler",
        ] {
            assert!(
                SHADER_PASS_PRELUDE.contains(needle),
                "prelude is missing {needle}"
            );
        }
    }
}
