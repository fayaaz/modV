//! Instanced-quad particle renderer shared by ParticleField and
//! SpiralParticles.
//!
//! Both JS scenes draw `THREE.Points` with a `PointsMaterial` whose `map` is
//! the shared circular sprite (`shared/particleSprite.js`), `vertexColors`
//! on, `transparent` on, `sizeAttenuation` on and `alphaTest: 0.01`. wgpu has
//! no point primitives worth using (point size is fixed at 1 px in most
//! backends), so the port draws one billboard quad per particle instead —
//! exactly what `THREE.Points` does under the hood on a tiling GPU.
//!
//! Replaced three.js machinery and what stands in for it:
//!
//! | three.js | Rust |
//! |----------|------|
//! | `PointsMaterial.size` + `sizeAttenuation` projection (`points_vert.glsl.js`) | [`PARTICLE_QUAD_WGSL`] billboard: `size_px = size * (height * 0.5) / -view_z` |
//! | `gl_PointCoord` quad rasterisation | a unit quad (\(2\) triangles) as instances (`mod::UnitQuad`) |
//! | `map` + `alphaTest` + `transparent` + `opacity` | sprite alpha × opacity, `discard` below `ALPHA_TEST`, `SrcAlpha`/`OneMinusSrcAlpha` |
//! | `vertexColors` | per-instance `color` attribute |
//! | `rotation.y` (SpiralParticles) | `ParticleUniforms::spin`, a Y rotation in the vertex stage |
//! | `PerspectiveCamera(60, aspect, .1, 1000)` at `z = 15` (`adapter.js:299-301`) | `fov_scale = 1/tan(30°)`, `camera_z = 15`, `aspect = w/h` |
//!
//! No depth attachment exists in the scene contract, so particles composite in
//! instance order (documented fidelity compromise; the JS depth buffer is only
//! written by ParticleField, whose points are all at similar depths).

#![forbid(unsafe_code)]

use crate::point10::UnitQuad;
use crate::point10::shared::particle_alpha::{
    PARTICLE_SPRITE_SIZE, create_particle_texture, particle_sampler,
};

/// `PointsMaterial.alphaTest: 0.01` (ParticleField.js:66, SpiralParticles.js:63).
pub(crate) const ALPHA_TEST: f32 = 0.01;

/// Camera field of view in degrees (`adapter.js:299`).
pub(crate) const CAMERA_FOV_DEGREES: f32 = 60.0;

/// Camera height above the origin (`adapter.js:300`, `camera.position.set(0, 0, 15)`).
pub(crate) const CAMERA_Z: f32 = 15.0;

/// `1 / tan(fov / 2)` — the reciprocal-tangent factor of three's perspective
/// projection matrix (`PerspectiveCamera.updateProjectionMatrix`), i.e.
/// `projectionMatrix[1][1]`.
pub(crate) fn camera_fov_scale() -> f32 {
    1.0 / (CAMERA_FOV_DEGREES.to_radians() * 0.5).tan()
}

/// Particle uniform block, matching [`PARTICLE_QUAD_WGSL`] (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ParticleUniforms {
    /// Drawing-buffer size in pixels.
    resolution: [f32; 2],
    /// `width / height`.
    aspect: f32,
    /// `1 / tan(fov / 2)`.
    fov_scale: f32,
    /// `PointsMaterial.size` in world units.
    size: f32,
    /// `PointsMaterial.opacity`.
    opacity: f32,
    /// Whole-object Y rotation in radians (SpiralParticles `points.rotation.y`).
    spin: f32,
    /// Camera z position.
    camera_z: f32,
    /// Padding to a 16-byte multiple.
    _pad: [f32; 4],
}

impl ParticleUniforms {
    fn new(width: u32, height: u32) -> Self {
        Self {
            resolution: [width.max(1) as f32, height.max(1) as f32],
            aspect: width.max(1) as f32 / height.max(1) as f32,
            fov_scale: camera_fov_scale(),
            size: 0.1,
            opacity: 1.0,
            spin: 0.0,
            camera_z: CAMERA_Z,
            _pad: [0.0; 4],
        }
    }
}

/// One particle instance: world position + vertex colour.
///
/// Layout matches the instance attributes at `@location(0)`/`@location(1)`
/// (the unit quad keeps `@location(2)`/`@location(3)`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ParticleInstance {
    /// World-space position.
    pub position: [f32; 3],
    /// Linear-space colour (`THREE.Color` working space).
    pub color: [f32; 3],
}

impl ParticleInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const SIZE: u64 = std::mem::size_of::<ParticleInstance>() as u64;
        const FLOAT: u64 = std::mem::size_of::<f32>() as u64;
        wgpu::VertexBufferLayout {
            array_stride: SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 3 * FLOAT,
                    shader_location: 1,
                },
            ],
        }
    }
}

/// GPU resources of an instanced-quad particle scene.
pub(crate) struct ParticleQuadRenderer {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    quad: UnitQuad,
    /// Instance capacity of `instance_buffer`.
    capacity: usize,
    uniforms: ParticleUniforms,
}

impl ParticleQuadRenderer {
    /// Build the pipeline, the sprite texture and an instance buffer sized for
    /// `capacity` particles.
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        capacity: usize,
        sprite_size: u32,
    ) -> Self {
        let sprite_size = sprite_size.max(1);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-particle-quad-shader"),
            source: wgpu::ShaderSource::Wgsl(particle_quad_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-particle-quad-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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

        let uniforms = ParticleUniforms::new(1, 1);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-particle-quad-uniforms"),
            size: std::mem::size_of::<ParticleUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Only the view (and the bind group) are kept: wgpu handles are
        // reference counted, so the texture outlives this local.
        let sprite = create_particle_texture(device, queue, sprite_size);
        let sprite_view = sprite.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = particle_sampler(device);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-particle-quad-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sprite_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-particle-quad-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let quad = UnitQuad::new(device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-particle-quad-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                // Slot 0 is the unit quad, slot 1 the per-instance data (the
                // instance attributes live at @location(0)/@location(1), which
                // is independent of the buffer slot).
                buffers: &[
                    Some(UnitQuad::vertex_layout()),
                    Some(ParticleInstance::layout()),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Point sprites have no defined winding, and a negative spin
                // mirrors the quad; disabling culling keeps both sides drawn.
                cull_mode: None,
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
                    // `transparent: true` + `premultipliedAlpha: false`
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let capacity = capacity.max(1);
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-particle-quad-instances"),
            size: (capacity * std::mem::size_of::<ParticleInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            queue: queue.clone(),
            pipeline,
            bind_group,
            uniform_buffer,
            instance_buffer,
            quad,
            capacity,
            uniforms,
        }
    }

    /// Update the size/opacity/spin levers and the resolution.
    pub(crate) fn set_state(
        &mut self,
        width: u32,
        height: u32,
        size: f32,
        opacity: f32,
        spin: f32,
    ) {
        self.uniforms = ParticleUniforms {
            resolution: [width.max(1) as f32, height.max(1) as f32],
            aspect: width.max(1) as f32 / height.max(1) as f32,
            fov_scale: camera_fov_scale(),
            size,
            opacity,
            spin,
            camera_z: CAMERA_Z,
            _pad: [0.0; 4],
        };
    }

    /// Upload `instances` and the uniform block.
    pub(crate) fn upload(&self, instances: &[ParticleInstance]) {
        let count = instances.len().min(self.capacity);
        if count > 0 {
            self.queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&instances[..count]),
            );
        }
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }

    /// Record the instanced draw for the first `instances` particles.
    pub(crate) fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, instances: u32) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        self.quad
            .draw_instanced(pass, instances.min(self.capacity as u32));
    }
}

/// Instanced billboard WGSL: three's `points_vert`/`points_frag` equivalent.
///
/// Uniforms at `@group(0) @binding(0)` (`ParticleUniforms`), sprite texture at
/// binding 1 and its linear sampler at binding 2.
pub(crate) fn particle_quad_wgsl() -> String {
    PARTICLE_QUAD_WGSL.replace("$ALPHA_TEST", &format!("{ALPHA_TEST}"))
}

pub(crate) const PARTICLE_QUAD_WGSL: &str = "\
struct ParticleUniforms {
  resolution: vec2<f32>, aspect: f32, fov_scale: f32,
  size: f32, opacity: f32, spin: f32, camera_z: f32,
  _pad0: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: ParticleUniforms;
@group(0) @binding(1) var sprite: texture_2d<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec3<f32>,
  @location(1) uv: vec2<f32>,
}

// Camera: PerspectiveCamera(60, aspect, 0.1, 1000) at z = 15 looking at the
// origin (`adapter.js:299-301`). view = world - (0, 0, camera_z).
@vertex
fn vs_main(
  @location(0) instance_position: vec3<f32>,
  @location(1) instance_color: vec3<f32>,
  @location(2) corner: vec2<f32>,
  @location(3) uv: vec2<f32>,
) -> VsOut {
  // points.rotation.y (SpiralParticles.js:79): x' = x cos + z sin,
  // z' = -x sin + z cos.
  let c = cos(u.spin);
  let s = sin(u.spin);
  let world = vec3<f32>(
    instance_position.x * c + instance_position.z * s,
    instance_position.y,
    -instance_position.x * s + instance_position.z * c,
  );
  let view = world - vec3<f32>(0.0, 0.0, u.camera_z);
  let dist = max(-view.z, 0.0001);
  let ndc = vec2<f32>(view.x * u.fov_scale / u.aspect, view.y * u.fov_scale) / dist;

  // sizeAttenuation: gl_PointSize = size * (height * 0.5) / -mvPosition.z
  let size_px = u.size * (u.resolution.y * 0.5) / dist;
  let offset = corner * size_px * 2.0 / u.resolution;

  var out: VsOut;
  out.position = vec4<f32>(ndc + offset, 0.0, 1.0);
  out.color = instance_color;
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let texel = textureSample(sprite, sprite_sampler, in.uv);
  let alpha = texel.a * u.opacity;
  // PointsMaterial.alphaTest (sprite corners are fully clear)
  if (alpha < $ALPHA_TEST) {
    discard;
  }
  return vec4<f32>(in.color * texel.rgb, alpha);
}
";

/// HSL→sRGB→linear, i.e. `new THREE.Color().setHSL(h, s, l)` under three's
/// default colour management (`THREE.ColorManagement.enabled = true`, working
/// space linear-sRGB).
///
/// The HSL→RGB step is the CSS/three `hue2rgb` formula; the second step is the
/// sRGB EOTF (`colorspace_pars_fragment: sRGBTransferEOTF`), so a pixel that
/// survives the composer's sRGB output transform reads back as the HSL colour.
pub(crate) fn hsl_to_linear_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let hue = hue.rem_euclid(1.0);
    let s = saturation.clamp(0.0, 1.0);
    let l = lightness.clamp(0.0, 1.0);

    if s == 0.0 {
        let value = srgb_to_linear(l);
        return [value, value, value];
    }

    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;

    let hue_to_rgb = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };

    [
        srgb_to_linear(hue_to_rgb(hue + 1.0 / 3.0)),
        srgb_to_linear(hue_to_rgb(hue)),
        srgb_to_linear(hue_to_rgb(hue - 1.0 / 3.0)),
    ]
}

/// sRGB EOTF (`c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055)^2.4`).
pub(crate) fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The sprite resolution ParticleField uses (`createParticleTexture()` — default 64).
pub(crate) const PARTICLE_FIELD_SPRITE: u32 = PARTICLE_SPRITE_SIZE;

/// The sprite resolution SpiralParticles uses (`createParticleTexture(32)`).
pub(crate) const SPIRAL_SPRITE: u32 = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_matches_the_srgb_reference_colours() {
        // setHSL(0, 1, 0.5) is pure red in sRGB -> linear 1, 0, 0.
        let red = hsl_to_linear_rgb(0.0, 1.0, 0.5);
        assert!((red[0] - 1.0).abs() < 1e-5);
        assert!(red[1].abs() < 1e-5 && red[2].abs() < 1e-5);

        // setHSL(1/3, 1, 0.5) is pure green.
        let green = hsl_to_linear_rgb(1.0 / 3.0, 1.0, 0.5);
        assert!(green[0].abs() < 1e-5 && green[2].abs() < 1e-5);
        assert!((green[1] - 1.0).abs() < 1e-5);

        // Saturation 0 is grey, and 0.5 sRGB -> ~0.2140 linear.
        let grey = hsl_to_linear_rgb(0.42, 0.0, 0.5);
        assert!((grey[0] - grey[1]).abs() < 1e-6 && (grey[1] - grey[2]).abs() < 1e-6);
        assert!((grey[0] - 0.214_041_14).abs() < 1e-5);
    }

    #[test]
    fn hsl_wraps_negative_and_large_hues() {
        let a = hsl_to_linear_rgb(0.25, 0.8, 0.6);
        let b = hsl_to_linear_rgb(1.25, 0.8, 0.6);
        let c = hsl_to_linear_rgb(-0.75, 0.8, 0.6);
        for channel in 0..3 {
            assert!((a[channel] - b[channel]).abs() < 1e-5);
            assert!((a[channel] - c[channel]).abs() < 1e-5);
        }
    }

    #[test]
    fn srgb_eotf_matches_the_linear_segment() {
        assert!((srgb_to_linear(0.0) - 0.0).abs() < 1e-8);
        assert!((srgb_to_linear(0.04) - 0.04 / 12.92).abs() < 1e-8);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn uniform_block_is_48_bytes() {
        assert_eq!(std::mem::size_of::<ParticleUniforms>(), 48);
        assert_eq!(std::mem::size_of::<ParticleInstance>(), 24);
    }

    #[test]
    fn fov_scale_matches_the_js_camera() {
        // 1 / tan(30°) = 1.7320508
        assert!((camera_fov_scale() - 1.732_050_8).abs() < 1e-6);
        assert_eq!(CAMERA_Z, 15.0);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        // naga validation catches missing bindings/attributes that parse fine.
        let program = particle_quad_wgsl();
        assert!(
            program.contains(&format!("alpha < {ALPHA_TEST}")),
            "the alpha-test constant must reach the shader"
        );
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("PARTICLE_QUAD_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("PARTICLE_QUAD_WGSL must validate");
    }
}
