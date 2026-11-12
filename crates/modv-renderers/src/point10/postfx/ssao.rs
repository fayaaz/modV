//! A small, reusable screen-space ambient-occlusion pass.
//!
//! The JS `GeometricTunnel` (and nothing else in the Point10 set) adds
//! three's `SSAOPass` between its `RenderPass` and its bloom mix. three's pass
//! is a full deferred stack: it re-renders the scene into a beauty and a normal
//! target, then runs a hemisphere-kernel AO over the depth texture with the
//! reconstructed normals, and finally multiplies the AO into the beauty.
//!
//! # What this pass is (and is not)
//!
//! This is a **depth-only screen-space occlusion pass** with the same
//! parameter semantics and the same output (a per-pixel ambient-occlusion
//! factor the scene multiplies its colour by):
//!
//! * it reads the scene's depth attachment (`Depth32Float`, sampled with
//!   `textureLoad`, no shadow comparison sampler);
//! * for each pixel it walks a deterministic 12-tap spiral in *screen* space,
//!   sized from the projection so a 0.5-world-unit kernel stays 0.5 world units
//!   across the frame (`radius * projection[1][1] * height * 0.5 / view_z`);
//! * a tap occludes when its window depth is **closer** than the centre pixel
//!   by more than `min_distance` and less than `max_distance` — the same
//!   normalised-near/far units three's `SSAOPass` compares (`GeometricTunnel`
//!   sets 0.00008 / 0.0025 because the scene spans 0.1..1000);
//! * the contribution is rejected when the tap's *linear* depth is more than
//!   `radius` world units away from the centre, so the kernel cannot shadow
//!   surfaces metres apart;
//! * `ao = clamp(1 - strength * occluded_taps / taps, 0, 1)`.
//!
//! **No normal reconstruction.** three's pass renders a normal target; a
//! depth-only port would need the normal from `dpdx`/`dpdy` of the
//! reconstructed view position, which is noisy on the thin extruded hoops and
//! would need a second full-scene pass. The occlusion test above only needs
//! depth, and the behaviour it reproduces (the inside of a hoop band and the
//! braces darken against the metal around them) is the reason the scene asks
//! for AO at all. This is a documented substitution, not a pixel match; see
//! `scenes/geometric_tunnel.rs`.
//!
//! # Usage
//!
//! ```text
//! let mut ssao = SsaoPass::new(&device, &queue, width, height);
//! ssao.set_depth(&device, &depth_view);        // after every depth resize
//! ssao.set_projection(&view_projection);       // every frame
//! ssao.set_params(SsaoParams { radius: 0.5, .. }); // the scene's constants
//! ssao.encode(&mut encoder);                   // writes into output_view()
//! ```
//!
//! The pass owns its output target ([`SsaoPass::output_view`]) because it is
//! inserted between a scene target and the scene's own composite, not inside
//! [`super::Composer`].

#![forbid(unsafe_code)]

use super::create_target;
use crate::point10::FULLSCREEN_TRIANGLE_WGSL;

/// A column-major 4x4 matrix (the same layout as `scenes::cube_mesh::Mat4`).
pub type Mat4 = [[f32; 4]; 4];

/// `GeometricTunnel.TUNNEL_SSAO` — the three.js defaults would only occlude
/// geometry metres apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SsaoParams {
    /// World-space kernel radius (`kernelRadius`).
    pub radius: f32,
    /// Minimum window-depth delta before a tap occludes (`minDistance`).
    pub min_distance: f32,
    /// Maximum window-depth delta (`maxDistance`).
    pub max_distance: f32,
    /// Darkening amount (`1.0` = three's full multiply).
    pub strength: f32,
    /// First tap radius in pixels is `radius_px * sqrt(bias)`; keeps the
    /// innermost taps from re-reading the centre pixel.
    pub bias: f32,
}

impl Default for SsaoParams {
    fn default() -> Self {
        Self {
            radius: 0.5,
            min_distance: 0.000_08,
            max_distance: 0.0025,
            strength: 1.0,
            bias: 0.02,
        }
    }
}

/// The uniform block (176 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SsaoUniforms {
    /// The scene's view-projection (`cameraProjectionMatrix` in the JS pass).
    pub projection: Mat4,
    /// Its inverse (kept for parity/diagnostics; the pass works in depth).
    pub inverse_projection: Mat4,
    /// Destination size in pixels.
    pub resolution: [f32; 2],
    /// Camera clip planes — the units `min_distance`/`max_distance` are in.
    pub near: f32,
    pub far: f32,
    /// World-space kernel radius.
    pub radius: f32,
    /// Minimum window-depth delta.
    pub min_distance: f32,
    /// Maximum window-depth delta.
    pub max_distance: f32,
    /// Darkening strength.
    pub strength: f32,
    /// Screen-space kernel bias.
    pub bias: f32,
    /// Scalar padding so the block is exactly 192 bytes: WGSL rounds the
    /// struct up to its 16-byte alignment (the last real field, `bias`, ends at
    /// 164 and the struct needs 192), while `repr(C)` would report 164.
    pub _pad: [f32; 7],
}

impl Default for SsaoUniforms {
    fn default() -> Self {
        Self {
            projection: identity(),
            inverse_projection: identity(),
            resolution: [1.0, 1.0],
            near: 0.1,
            far: 100.0,
            radius: 0.5,
            min_distance: 0.000_08,
            max_distance: 0.0025,
            strength: 1.0,
            bias: 0.02,
            _pad: [0.0; 7],
        }
    }
}

/// The depth-only ambient-occlusion pass.
pub struct SsaoPass {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    uniforms_value: SsaoUniforms,
    _output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,
    width: u32,
    height: u32,
    bind_group: Option<wgpu::BindGroup>,
}

impl SsaoPass {
    /// Build the pass for a `width` x `height` destination.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-postfx-ssao-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{FULLSCREEN_TRIANGLE_WGSL}\n{}", include_str!("ssao.wgsl")).into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-postfx-ssao-layout"),
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
                        // `texture_depth_2d` is a non-filtering sample type.
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-postfx-ssao-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-postfx-ssao-pipeline-layout"),
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
            label: Some("point10-postfx-ssao-uniforms"),
            size: std::mem::size_of::<SsaoUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (output_texture, output_view) =
            create_target(device, "point10-postfx-ssao-output", width, height);

        let pass = Self {
            queue: queue.clone(),
            pipeline,
            bind_group_layout,
            uniforms,
            uniforms_value: SsaoUniforms {
                resolution: [width as f32, height as f32],
                ..SsaoUniforms::default()
            },
            _output_texture: output_texture,
            output_view,
            width,
            height,
            bind_group: None,
        };
        pass.upload();
        pass
    }

    /// Bind the scene's depth attachment (must have `TEXTURE_BINDING`).
    pub fn set_depth(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-postfx-ssao-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        }));
    }

    /// Set the camera's view-projection (and its inverse) for this frame.
    pub fn set_projection(&mut self, projection: &Mat4) {
        self.uniforms_value.projection = *projection;
        self.uniforms_value.inverse_projection = invert(projection);
        self.upload();
    }

    /// Set the clip planes the depth range is measured against.
    pub fn set_clip_planes(&mut self, near: f32, far: f32) {
        self.uniforms_value.near = near;
        self.uniforms_value.far = far;
        self.upload();
    }

    /// Apply the scene's AO parameters.
    pub fn set_params(&mut self, params: SsaoParams) {
        self.uniforms_value.radius = params.radius;
        self.uniforms_value.min_distance = params.min_distance;
        self.uniforms_value.max_distance = params.max_distance;
        self.uniforms_value.strength = params.strength;
        self.uniforms_value.bias = params.bias;
        self.upload();
    }

    /// The AO target (RGB = factor, alpha = 1).
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.output_view
    }

    /// Re-create the AO target for a new size.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if (width, height) == (self.width, self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        let (texture, view) = create_target(device, "point10-postfx-ssao-output", width, height);
        self._output_texture = texture;
        self.output_view = view;
        self.uniforms_value.resolution = [width as f32, height as f32];
        self.upload();
    }

    /// Record the AO pass. A no-op until a depth view has been bound.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-postfx-ssao"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.output_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
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
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&self.uniforms_value));
    }
}

/// Column-major identity.
pub fn identity() -> Mat4 {
    let mut matrix = [[0.0_f32; 4]; 4];
    for (index, column) in matrix.iter_mut().enumerate() {
        column[index] = 1.0;
    }
    matrix
}

/// A 4x4 inverse (cofactor expansion; the matrices here are invertible).
pub fn invert(matrix: &Mat4) -> Mat4 {
    let m = matrix;
    let mut inv = [[0.0_f32; 4]; 4];

    inv[0][0] =
        m[1][1] * m[2][2] * m[3][3] - m[1][1] * m[2][3] * m[3][2] - m[2][1] * m[1][2] * m[3][3]
            + m[2][1] * m[1][3] * m[3][2]
            + m[3][1] * m[1][2] * m[2][3]
            - m[3][1] * m[1][3] * m[2][2];
    inv[1][0] =
        -m[1][0] * m[2][2] * m[3][3] + m[1][0] * m[2][3] * m[3][2] + m[2][0] * m[1][2] * m[3][3]
            - m[2][0] * m[1][3] * m[3][2]
            - m[3][0] * m[1][2] * m[2][3]
            + m[3][0] * m[1][3] * m[2][2];
    inv[2][0] =
        m[1][0] * m[2][1] * m[3][3] - m[1][0] * m[2][3] * m[3][1] - m[2][0] * m[1][1] * m[3][3]
            + m[2][0] * m[1][3] * m[3][1]
            + m[3][0] * m[1][1] * m[2][3]
            - m[3][0] * m[1][3] * m[2][1];
    inv[3][0] =
        -m[1][0] * m[2][1] * m[3][2] + m[1][0] * m[2][2] * m[3][1] + m[2][0] * m[1][1] * m[3][2]
            - m[2][0] * m[1][2] * m[3][1]
            - m[3][0] * m[1][1] * m[2][2]
            + m[3][0] * m[1][2] * m[2][1];
    inv[0][1] =
        -m[0][1] * m[2][2] * m[3][3] + m[0][1] * m[2][3] * m[3][2] + m[2][1] * m[0][2] * m[3][3]
            - m[2][1] * m[0][3] * m[3][2]
            - m[3][1] * m[0][2] * m[2][3]
            + m[3][1] * m[0][3] * m[2][2];
    inv[1][1] =
        m[0][0] * m[2][2] * m[3][3] - m[0][0] * m[2][3] * m[3][2] - m[2][0] * m[0][2] * m[3][3]
            + m[2][0] * m[0][3] * m[3][2]
            + m[3][0] * m[0][2] * m[2][3]
            - m[3][0] * m[0][3] * m[2][2];
    inv[2][1] =
        -m[0][0] * m[2][1] * m[3][3] + m[0][0] * m[2][3] * m[3][1] + m[2][0] * m[0][1] * m[3][3]
            - m[2][0] * m[0][3] * m[3][1]
            - m[3][0] * m[0][1] * m[2][3]
            + m[3][0] * m[0][3] * m[2][1];
    inv[3][1] =
        m[0][0] * m[2][1] * m[3][2] - m[0][0] * m[2][2] * m[3][1] - m[2][0] * m[0][1] * m[3][2]
            + m[2][0] * m[0][2] * m[3][1]
            + m[3][0] * m[0][1] * m[2][2]
            - m[3][0] * m[0][2] * m[2][1];
    inv[0][2] =
        m[0][1] * m[1][2] * m[3][3] - m[0][1] * m[1][3] * m[3][2] - m[1][1] * m[0][2] * m[3][3]
            + m[1][1] * m[0][3] * m[3][2]
            + m[3][1] * m[0][2] * m[1][3]
            - m[3][1] * m[0][3] * m[1][2];
    inv[1][2] =
        -m[0][0] * m[1][2] * m[3][3] + m[0][0] * m[1][3] * m[3][2] + m[1][0] * m[0][2] * m[3][3]
            - m[1][0] * m[0][3] * m[3][2]
            - m[3][0] * m[0][2] * m[1][3]
            + m[3][0] * m[0][3] * m[1][2];
    inv[2][2] =
        m[0][0] * m[1][1] * m[3][3] - m[0][0] * m[1][3] * m[3][1] - m[1][0] * m[0][1] * m[3][3]
            + m[1][0] * m[0][3] * m[3][1]
            + m[3][0] * m[0][1] * m[1][3]
            - m[3][0] * m[0][3] * m[1][1];
    inv[3][2] =
        -m[0][0] * m[1][1] * m[3][2] + m[0][0] * m[1][2] * m[3][1] + m[1][0] * m[0][1] * m[3][2]
            - m[1][0] * m[0][2] * m[3][1]
            - m[3][0] * m[0][1] * m[1][2]
            + m[3][0] * m[0][2] * m[1][1];
    inv[0][3] =
        -m[0][1] * m[1][2] * m[2][3] + m[0][1] * m[1][3] * m[2][2] + m[1][1] * m[0][2] * m[2][3]
            - m[1][1] * m[0][3] * m[2][2]
            - m[2][1] * m[0][2] * m[1][3]
            + m[2][1] * m[0][3] * m[1][2];
    inv[1][3] =
        m[0][0] * m[1][2] * m[2][3] - m[0][0] * m[1][3] * m[2][2] - m[1][0] * m[0][2] * m[2][3]
            + m[1][0] * m[0][3] * m[2][2]
            + m[2][0] * m[0][2] * m[1][3]
            - m[2][0] * m[0][3] * m[1][2];
    inv[2][3] =
        -m[0][0] * m[1][1] * m[2][3] + m[0][0] * m[1][3] * m[2][1] + m[1][0] * m[0][1] * m[2][3]
            - m[1][0] * m[0][3] * m[2][1]
            - m[2][0] * m[0][1] * m[1][3]
            + m[2][0] * m[0][3] * m[1][1];
    inv[3][3] =
        m[0][0] * m[1][1] * m[2][2] - m[0][0] * m[1][2] * m[2][1] - m[1][0] * m[0][1] * m[2][2]
            + m[1][0] * m[0][2] * m[2][1]
            + m[2][0] * m[0][1] * m[1][2]
            - m[2][0] * m[0][2] * m[1][1];

    let det = m[0][0] * inv[0][0] + m[0][1] * inv[1][0] + m[0][2] * inv[2][0] + m[0][3] * inv[3][0];
    if det.abs() < 1e-12 {
        return identity();
    }
    let inv_det = 1.0 / det;

    for column in inv.iter_mut() {
        for value in column.iter_mut() {
            *value *= inv_det;
        }
    }
    inv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_are_192_bytes() {
        assert_eq!(std::mem::size_of::<SsaoUniforms>(), 192);
    }

    #[test]
    fn the_js_tunnel_parameters_are_the_defaults() {
        let params = SsaoParams::default();
        assert_eq!(params.radius, 0.5);
        assert_eq!(params.min_distance, 0.000_08);
        assert_eq!(params.max_distance, 0.0025);
        assert!(params.max_distance > params.min_distance);
    }

    fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
        let mut out = [[0.0_f32; 4]; 4];
        for column in 0..4 {
            for row in 0..4 {
                out[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
            }
        }
        out
    }

    #[test]
    fn invert_round_trips_a_projection() {
        let projection = [
            [1.5, 0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.0, 0.0, -1.002, -1.0],
            [0.0, 0.0, -0.2002, 0.0],
        ];
        let inverse = invert(&projection);
        let identity = multiply(&projection, &inverse);
        for (column, values) in identity.iter().enumerate() {
            for (row, value) in values.iter().enumerate() {
                let expected = if column == row { 1.0 } else { 0.0 };
                assert!(
                    (value - expected).abs() < 1e-5,
                    "identity[{column}][{row}] = {value}"
                );
            }
        }
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = format!("{FULLSCREEN_TRIANGLE_WGSL}\n{}", include_str!("ssao.wgsl"));
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("ssao WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("ssao WGSL must validate");
    }

    #[test]
    fn shader_keeps_the_documented_occlusion_rule() {
        let source = include_str!("ssao.wgsl");
        for needle in [
            "textureLoad(depth_map, pixel, 0)",
            "let delta = centre_depth - sample_depth;",
            "delta > u.min_distance && delta < u.max_distance",
            "abs(linear_depth(sample_depth) - centre_linear) < u.radius",
            "clamp(1.0 - u.strength * occlusion / f32(TAP_COUNT), 0.0, 1.0)",
        ] {
            assert!(source.contains(needle), "ssao lost {needle}");
        }
    }
}
