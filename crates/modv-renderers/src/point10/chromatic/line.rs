//! Chromatic Line — Rust/wgpu port of the Point10 scene of the same name.
//!
//! Sources of truth (all line anchors point at this repository):
//!
//! - `src/renderer/src/application/point10/scenes/ChromaticLine.js` — the
//!   scene. The header comment there explains the audio/visual design: one
//!   full-screen quad, a fragment shader that evaluates the line three times
//!   with the x input radially offset per channel, fed by a ring buffer of
//!   ~5.9 s of audio. Each texel of the 256x1 wave texture is the mean |v| of
//!   its own ~23 ms window (`SPX = 1024` samples). Red reads the bass band,
//!   green the mids, blue the highs, each through P9Audio's damping-2 lane.
//! - `src/renderer/src/application/point10/p10Audio.js` — `P9Audio.band()`;
//!   this port implements only the `band(-1, 2, n)` semantics (damping lane 2,
//!   bands 0..=2), approximated by a single one-pole low-pass (tau = 0.25 s).
//! - `src/renderer/src/application/point10/scenes/ChromaticWave.js` — subclass
//!   with its own FRAG and texel mapping; ported in [`super::wave`] on top of
//!   the shared simulation in [`super`].
//!
//! The ring buffer, texel windows and band read live in [`super`] so the wave
//! variant can reuse them; this module keeps the line's scene struct, its
//! uniform block and its WGSL.
//!
//! Line-number references below (e.g. "ChromaticLine.js:171") are anchors,
//! not contracts — they drift with the source file.
#![forbid(unsafe_code)]

use super::{AdvanceOutput, BandUniforms, ChromaticLineSim, SyntheticSine, WAVE_BYTES, WAVE_N};

/// Chromatic Line uniform block, matching `line.wgsl` member for member
/// (80 bytes, `#[repr(C)]`, all members at the offsets WGSL computes).
///
/// The `_pad*` members keep vec3 scalars 16-byte aligned in Rust's layout;
/// WGSL's own uniform layout for the same struct is verified byte-identical
/// by the `wgsl_parses` test (vec3<f32> spans are flattened to scalars there
/// because WGSL vec3 has 16-byte alignment — see the note in the .wgsl file).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChromaticLineUniforms {
    /// Drawing-buffer size in pixels. Written as `[width, height]`.
    pub resolution: [f32; 2],
    /// Padding: keeps `x_scale` on a vec4 boundary in the WGSL layout.
    pub _pad0: [f32; 2],
    /// Per-channel horizontal frequency scale. `ChromaticLine.js: xScale`.
    pub x_scale: [f32; 3],
    /// Padding: keeps `amp` on a vec4 boundary in the WGSL layout.
    pub _pad1: f32,
    /// Per-channel wave displacement. `ChromaticLine.js: amp` (min(band*10, 0.8) * ampScale).
    pub amp: [f32; 3],
    /// Padding: keeps `glow` on a vec4 boundary in the WGSL layout.
    pub _pad2: f32,
    /// Per-channel glow intensity. `ChromaticLine.js: glow` ((0.05 + band*0.08) * glowScale).
    pub glow: [f32; 3],
    /// Padding: aligns `distortion` with the WGSL layout.
    pub _pad3: f32,
    /// Radial fringing factor. `ChromaticLine.js: this.distortion = 0.05`.
    pub distortion: f32,
    /// Padding: trailing vec3 tail, flattened to scalars in the WGSL struct
    /// so the span stays 80 bytes (a vec3 tail would span to 96 — see the
    /// note in `line.wgsl`).
    pub _pad4: [f32; 3],
}

/// Build the line's uniform block from the target size and band uniforms.
pub(crate) fn build_uniforms(size: (u32, u32), band: &BandUniforms) -> ChromaticLineUniforms {
    ChromaticLineUniforms {
        resolution: [size.0 as f32, size.1 as f32],
        _pad0: [0.0, 0.0],
        x_scale: band.x_scale,
        _pad1: 0.0,
        amp: band.amp,
        _pad2: 0.0,
        glow: band.glow,
        _pad3: 0.0,
        distortion: band.distortion,
        _pad4: [0.0, 0.0, 0.0],
    }
}

/// Full-screen Chromatic Line scene: pure CPU simulation feeding a 256x1
/// wave texture, drawn as a single full-screen triangle with alpha blending.
///
/// The scene does not clear the pass; it alpha-blends
/// (`SrcAlpha` / `OneMinusSrcAlpha`) over whatever is already in the colour
/// attachment, so the rest of the frame composites normally underneath.
/// `render()` expects a render pass with an existing colour attachment
/// (e.g. the engine's target, or the caller's clear).
pub struct ChromaticLine {
    /// CPU-side ring/texel/band state.
    sim: ChromaticLineSim,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    wave_texture: wgpu::Texture,
    /// Drawing-buffer size in pixels. Defaults to `(1, 1)`; update() rewrites
    /// the `resolution` uniform from this every frame (`_syncResolution`,
    /// ChromaticLine.js:220-228, folds into the per-frame uniform write).
    size: (u32, u32),
}

impl ChromaticLine {
    /// Build pipelines, buffers and the 256x1 wave texture. `target_format`
    /// is the swapchain/headless-target format the scene draws into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("chromatic_line-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("line.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("chromatic_line-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None, // shader requires 80 bytes; the buffer is exactly 80
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

        // `ChromaticLine.js: this.waveTex = new THREE.DataTexture(...)` (lines 85-94):
        // 256x1 RGBA, Linear filters, Repeat/REPEAT on S.
        let wave_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("chromatic_line-wave"),
            size: wgpu::Extent3d {
                width: WAVE_N as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let wave_view = wave_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("chromatic_line-wave-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chromatic_line-uniforms"),
            size: std::mem::size_of::<ChromaticLineUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // The view/sampler are only needed at bind-group creation; wgpu
        // retains its own references, so they are not stored on the scene.
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("chromatic_line-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&wave_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("chromatic_line-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("chromatic_line-pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // `ChromaticLine.js: this.waveData.fill(255)` / `waveTex.needsUpdate = true`
        // (lines 83-94): upload an all-255 payload so the texture is defined
        // before the first update() call. (The `rgb starts 128` comment in the
        // JS is stale — fill(255) covers alpha too.)
        let initial = [255_u8; WAVE_BYTES];
        let scene = Self {
            sim: ChromaticLineSim::new(),
            pipeline,
            bind_group,
            uniform_buffer,
            wave_texture,
            size: (1, 1),
        };
        scene.upload_wave(queue, &initial);
        // Upload the initial uniforms (silence at 1x1, the JS defaults:
        // xScale (2.5,2.5,2.5), amp (0,0,0), glow (0.05,0.05,0.05)).
        let silence = scene.sim.band_uniforms([0.0, 0.0, 0.0]);
        let uniforms = build_uniforms(scene.size, &silence);
        queue.write_buffer(&scene.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        scene
    }

    /// Drawing-buffer size in pixels. Called on window resize; `update()`
    /// rewrites the `resolution` uniform from the stored size every frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
    }

    /// Advance the simulation by `dt_seconds`, upload the wave texture and
    /// uniforms. `audio` is the sine fixture source; None = silence.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        dt_seconds: f32,
        audio: &mut Option<SyntheticSine>,
    ) {
        let AdvanceOutput {
            bytes,
            band_uniforms,
            ..
        } = self.sim.advance(dt_seconds, audio);
        self.upload_wave(queue, &bytes);
        let uniforms = build_uniforms(self.size, &band_uniforms);
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Record the full-screen triangle into the caller's render pass.
    /// Expects the pass to already have a clear colour attachment (the scene
    /// does not clear). Alpha-blends (SrcAlpha, OneMinusSrcAlpha).
    pub fn render<'rp>(&self, render_pass: &mut wgpu::RenderPass<'rp>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Upload the 256x1 RGBA wave payload, reusing the stable texture.
    fn upload_wave(&self, queue: &wgpu::Queue, bytes: &[u8; WAVE_BYTES]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.wave_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WAVE_BYTES as u32),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: WAVE_N as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::chromatic::{RING_LEN, TexelMapping};

    /// `update()` with no audio must leave the ring silent, which maps to the
    /// biased texel value (0.5+0.5*0=0.5) truncated to the byte 128+63 = 191.
    /// Ported exactly from the JS (`texelByte`, ChromaticLine.js:131-133): the
    /// 0.5 bias exists so the unsigned texture never clips negative wave
    /// values. Silence therefore reads as 191, NOT 128 — this test documents
    /// the wart instead of "fixing" it.
    #[test]
    fn silence_produces_bias_byte_191() {
        let mut sim = ChromaticLineSim::new();
        let mut audio = None;
        let mut bytes = [0_u8; WAVE_BYTES];
        for _ in 0..10 {
            // Simulate several ~60 fps updates of silence.
            bytes = sim.advance(1.0 / 60.0, &mut audio).bytes;
        }
        for i in 0..WAVE_N {
            assert_eq!(bytes[i * 4], 191, "red texel {i} not the silence bias");
            assert_eq!(
                bytes[i * 4 + 1],
                191,
                "green texel {i} not the silence bias"
            );
            assert_eq!(bytes[i * 4 + 2], 191, "blue texel {i} not the silence bias");
            assert_eq!(bytes[i * 4 + 3], 255, "alpha texel {i} must stay 255");
        }
    }

    /// Ring append must wrap at RING_LEN without panicking and leave `head`
    /// pointing at the next write position.
    #[test]
    fn ring_advances_and_wraps() {
        let mut sim = ChromaticLineSim::new();
        sim.append(&vec![0.5; 5000]);
        assert_eq!(sim.head, 5000 % RING_LEN);
        sim.append(&vec![0.5; RING_LEN]);
        assert_eq!(sim.head, 5000 % RING_LEN);
    }

    /// A 110 Hz sine pushed through `advance` for 400 frames (~6.6 s) must
    /// lift texel bytes above the silence bias: the windows contain the
    /// near-full-scale signal.
    #[test]
    fn full_scale_sine_raises_bytes() {
        let mut sim = ChromaticLineSim::new();
        let mut audio = Some(SyntheticSine::new(110.0));
        let mut bytes = [0_u8; WAVE_BYTES];
        for _ in 0..400 {
            bytes = sim.advance(1.0 / 60.0, &mut audio).bytes;
        }
        let texels: Vec<u8> = (0..WAVE_N).map(|i| bytes[i * 4]).collect();
        assert!(
            texels.iter().any(|&b| b > 200),
            "expected at least one texel byte > 200, got max {}",
            texels.iter().max().copied().unwrap_or(0)
        );
        assert!(
            !texels.iter().all(|&b| b == 191),
            "the flat silence case must be gone after 6.6 s of signal"
        );
    }

    /// Band -> uniform mapping must follow the wire curve: `amp = min(b*10, 0.8)`,
    /// `glow = 0.05 + b*0.08`, `x_scale = 2.5 + b*0.8`, `distortion = 0.05`.
    #[test]
    fn uniforms_follow_band_curve() {
        let sim = ChromaticLineSim::new();
        let u = sim.band_uniforms([0.05, 0.0, 0.0]);
        assert!(
            (u.amp[0] - 0.5).abs() < f32::EPSILON,
            "amp[0] = {}",
            u.amp[0]
        );
        assert!((u.glow[0] - (0.05 + 0.05 * 0.08)).abs() < f32::EPSILON);
        assert!((u.x_scale[0] - (2.5 + 0.05 * 0.8)).abs() < f32::EPSILON);
        assert_eq!(u.distortion, 0.05);

        let capped = sim.band_uniforms([0.2, 0.0, 0.0]);
        assert!(
            (capped.amp[0] - 0.8).abs() < f32::EPSILON,
            "amp must cap at 0.8, got {}",
            capped.amp[0]
        );
    }

    /// The line mapping and the wave mapping must stay distinct: the line sits
    /// at the biased 191 in silence, the wave at 0.
    #[test]
    fn texel_mappings_differ() {
        assert_eq!(TexelMapping::Line.texel_byte(0.0), 191);
        assert_eq!(TexelMapping::Energy.texel_byte(0.0), 0);
        assert_eq!(TexelMapping::Line.texel_byte(1.0), 255);
        assert_eq!(TexelMapping::Energy.texel_byte(1.0), 255);
    }

    /// The naga dev-dependency (wgsl-in) must accept the shader as written.
    #[test]
    fn wgsl_parses() {
        let result = naga::front::wgsl::parse_str(include_str!("line.wgsl"));
        if let Err(error) = &result {
            panic!("line.wgsl failed to parse: {error}");
        }
    }
}
