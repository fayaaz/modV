//! Chromatic Wave — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/ChromaticWave.js`.
//!
//! The JS class extends ChromaticLine: same ring buffer, same 256x1 texel
//! texture, same three-channel read — but it restores the CodePen
//! "GLSL Sample027" look: three chromatically-fringed **sine** waves (the
//! pen's `sin((u + time) * xScale) * yScale`), and the ring buffer's energy
//! multiplies the sine subtly in shader (`mul = 0.75 + 0.5 * energy`), so a
//! loud passage swells the wave by ~2/3 and the energy scrolls leftward in
//! real time.
//!
//! Differences from [`super::line`] (ported literally):
//!
//! * the texel mapping is plain energy, `texelByte(e) = (e * 255) | 0`
//!   (`ChromaticWave.js:59-61`) instead of the line's centre bias;
//! * there is no `amp` uniform — `_audioUniforms` deletes it and adds `time`
//!   (`ChromaticWave.js:63-68`);
//! * the wave is sampled edge to edge (`gl_FragCoord.x / resolution.x`)
//!   instead of tiled with `fract(u)`;
//! * bloom is the music driver: `glow = (0.05 + band * 0.3) * glowScale`,
//!   while `xScale` only nudges the pitch (`1.0 + band * 0.05`) so audio no
//!   longer visibly speeds the scroll (`ChromaticWave.js:86-100`).
//!
//! The ring/texel/band simulation is shared with the line through
//! [`super::ChromaticLineSim`]; this module owns the wave's uniform block and
//! WGSL.
#![forbid(unsafe_code)]

use super::{ChromaticLineSim, TexelMapping, WAVE_BYTES, WAVE_N};
use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge, P10Audio};
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, SceneContext, SceneMeta, SceneProp, read_scene_props,
    scene_props_with_levers,
};

/// Base uniforms for ChromaticWave, matching `wave.wgsl` (64 bytes).
///
/// The JS `uniforms` object is the line's minus `amp` plus `time`
/// (`ChromaticWave.js:66-68`); the Rust block keeps the same member order so a
/// future golden-image diff can line the bytes up.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChromaticWaveUniforms {
    /// Drawing-buffer size in pixels.
    pub resolution: [f32; 2],
    /// Padding: keeps `x_scale` on a vec4 boundary.
    pub _pad0: [f32; 2],
    /// Per-channel horizontal frequency scale (`1.0 + band * 0.05`).
    pub x_scale: [f32; 3],
    /// Padding: keeps `glow` on a vec4 boundary.
    pub _pad1: f32,
    /// Per-channel glow (`(0.05 + band * 0.3) * glowScale`).
    pub glow: [f32; 3],
    /// Padding: keeps `distortion`/`time` aligned.
    pub _pad2: f32,
    /// Radial fringing factor.
    pub distortion: f32,
    /// Scene clock in seconds (`this.time`).
    pub time: f32,
    /// Trailing padding (keeps the span at 64 bytes).
    pub _pad3: [f32; 2],
}

/// `CHROMATIC_LINE_PROPS`, shared by the line and the wave (`sceneProps.js:84-88`).
///
/// The wave carries `amplitude` even though its shader has no `amp` uniform:
/// the JS registry applies the same schema to both scenes and sets
/// `scene.ampScale`, which ChromaticWave's `_audioUniforms` simply never reads.
const CHROMATIC_PROPS: &[SceneProp] = &[
    SceneProp::float("distortion", "Chroma", 0.05, 0.0, 0.5, 0.001),
    SceneProp::float("amplitude", "Wave Amplitude", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("glow", "Glow", 1.0, 0.0, 3.0, 0.01),
];

/// GPU resources built in [`crate::point10::Point10Scene::init`].
struct WaveGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    wave_texture: wgpu::Texture,
}

/// Chromatic Wave scene.
pub struct ChromaticWave {
    /// Ring buffer, texel windows and damping state.
    sim: ChromaticLineSim,
    /// P9Audio-compatible reader over the bridge.
    p9: P10Audio,
    /// Scene clock (`BaseScene.time`), seconds.
    time: f32,
    /// `this.distortion = 0.05` (`ChromaticLine.js:79`).
    distortion: f32,
    /// `this.ampScale` — set by the shared prop binder, unused by the wave
    /// shader (see [`CHROMATIC_PROPS`]).
    amp_scale: f32,
    /// `this.glowScale = 1`.
    glow_scale: f32,
    /// Drawing-buffer size in pixels.
    size: (u32, u32),
    gpu: Option<WaveGpu>,
}

impl Default for ChromaticWave {
    fn default() -> Self {
        Self::new()
    }
}

impl ChromaticWave {
    /// Create the scene. GPU resources are built on
    /// [`crate::point10::Point10Scene::init`].
    pub fn new() -> Self {
        Self {
            sim: ChromaticLineSim::new(),
            p9: P10Audio::new(),
            time: 0.0,
            distortion: 0.05,
            amp_scale: 1.0,
            glow_scale: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props.
    ///
    /// `sceneProps.js: SCENE_PROP_CONFIGS["Point10 Chromatic Wave"]` —
    /// `distortion`, `ampScale = amplitude`, `glowScale = glow`.
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        let values = read_scene_props(CHROMATIC_PROPS, props);
        for (key, value) in values {
            match key {
                "distortion" => self.distortion = value,
                "amplitude" => self.amp_scale = value,
                "glow" => self.glow_scale = value,
                _ => {}
            }
        }
    }

    /// Current wave texture payload (for tests and golden-image diffing).
    pub fn wave_bytes(&self) -> [u8; WAVE_BYTES] {
        self.sim.compute_bytes(TexelMapping::Energy)
    }

    /// Band-driven uniforms for the current state.
    ///
    /// `ChromaticWave.js: _audioUniforms` (lines 86-100), fed from
    /// `P9Audio.band(-1, 2, n)` — the bridge's damping-2 lane — as
    /// `ChromaticLine.js:update` does at lines 210-214.
    fn uniforms(&self) -> ChromaticWaveUniforms {
        let bands = [
            self.p9.band(-1, 2, 0, 0.0),
            self.p9.band(-1, 2, 1, 0.0),
            self.p9.band(-1, 2, 2, 0.0),
        ];

        ChromaticWaveUniforms {
            resolution: [self.size.0 as f32, self.size.1 as f32],
            _pad0: [0.0, 0.0],
            x_scale: [
                1.0 + bands[0] * 0.05,
                1.0 + bands[1] * 0.05,
                1.0 + bands[2] * 0.05,
            ],
            _pad1: 0.0,
            glow: [
                (0.05 + bands[0] * 0.3) * self.glow_scale,
                (0.05 + bands[1] * 0.3) * self.glow_scale,
                (0.05 + bands[2] * 0.3) * self.glow_scale,
            ],
            _pad2: 0.0,
            distortion: self.distortion,
            time: self.time,
            _pad3: [0.0, 0.0],
        }
    }

    /// Number of new samples to append this frame.
    ///
    /// `ChromaticLine.js:168`: `Math.min(Math.round(this.rate * dt), this.p9.wave.length)`
    /// with `this.rate = audio.ctx?.sampleRate || 44100`. A frame longer than
    /// the 1024-sample (~23 ms) analyser window drops samples instead of
    /// re-reading them.
    fn sample_count(sample_rate: f32, dt: f32) -> usize {
        if !dt.is_finite() || !sample_rate.is_finite() || dt <= 0.0 || sample_rate <= 0.0 {
            return 0;
        }
        ((sample_rate * dt).round() as usize).min(AUDIO_SAMPLES)
    }
}

impl crate::point10::Point10Scene for ChromaticWave {
    fn name(&self) -> &'static str {
        "Point10 Chromatic Wave"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = &ctx.device;
        let queue = &ctx.queue;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-chromatic-wave-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{FULLSCREEN_TRIANGLE_WGSL}\n{}", include_str!("wave.wgsl")).into(),
            ),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-chromatic-wave-layout"),
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

        // `ChromaticLine.js: this.waveTex = new THREE.DataTexture(...)` — the
        // wave inherits the same 256x1 RGBA texture; 128 starts are irrelevant
        // because the wave overwrites every texel each frame.
        let wave_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point10-chromatic-wave-texels"),
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
            label: Some("point10-chromatic-wave-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-chromatic-wave-uniforms"),
            size: std::mem::size_of::<ChromaticWaveUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-chromatic-wave-bind-group"),
            layout: &layout,
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
            label: Some("point10-chromatic-wave-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = ctx.fullscreen_pipeline(
            "point10-chromatic-wave-pipeline",
            &shader,
            &pipeline_layout,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        // Seed the texture with the wave's silence texel (0), then the first
        // frame overwrites it.
        let silence = self.sim.compute_bytes(TexelMapping::Energy);
        write_wave(queue, &wave_texture, &silence);

        self.size = (ctx.width, ctx.height);
        self.gpu = Some(WaveGpu {
            queue: queue.clone(),
            pipeline,
            bind_group,
            uniform_buffer,
            wave_texture,
        });

        let uniforms = self.uniforms();
        queue.write_buffer(
            &self.gpu.as_ref().unwrap().uniform_buffer,
            0,
            bytemuck::bytes_of(&uniforms),
        );
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        // `BaseScene.update`: `this.time += dt`.
        if dt.is_finite() && dt > 0.0 {
            self.time += dt;
        }
        self.p9.update(dt, audio);

        // Append the genuinely new samples (the analyser's tail) so the
        // waveform scrolls in real time (`ChromaticLine.js:167-176`).
        if self.p9.has_audio() {
            let k = Self::sample_count(audio.sample_rate, dt);
            if k > 0 {
                self.sim.append_wave_tail(&audio.wave[AUDIO_SAMPLES - k..]);
            }
        }

        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let bytes = self.wave_bytes();
        write_wave(&gpu.queue, &gpu.wave_texture, &bytes);
        let uniforms = self.uniforms();
        gpu.queue
            .write_buffer(&gpu.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };

        // BaseScene renders through the adapter's renderer, which clears to
        // transparent black (`adapter.js:297`), so an empty frame composites
        // over the layers below.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-chromatic-wave"),
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
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &gpu.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(CHROMATIC_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10ChromaticWave.js`: `meta: { category: "2d-scenes" }` — the
        // fullscreen 2D scenes override the adapter's "3d-scenes" default.
        SceneMeta::for_scene("Point10 Chromatic Wave", "2d-scenes")
    }
}

fn write_wave(queue: &wgpu::Queue, texture: &wgpu::Texture, bytes: &[u8; WAVE_BYTES]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::Point10Scene;
    use crate::point10::audio_bridge::BandPayload;

    #[test]
    fn wave_texels_are_plain_energy() {
        let wave = ChromaticWave::new();
        // Silence -> 0, not the line's 191 bias (`ChromaticWave.js:59-61`).
        let bytes = wave.wave_bytes();
        for texel in bytes.chunks_exact(4) {
            assert_eq!(&texel[0..3], &[0, 0, 0], "silence must read as 0 energy");
            assert_eq!(texel[3], 255, "alpha stays opaque");
        }
    }

    #[test]
    fn glow_follows_the_bloom_curve_and_x_scale_barely_moves() {
        let mut wave = ChromaticWave::new();
        let mut payload = BandPayload::silent(44_100.0);
        // Damping lane 2 of each band is lanes[6], lanes[7], lanes[8].
        let mut bridge = AudioBridge::new();
        payload.lanes[6] = 0.5;
        payload.lanes[7] = 0.0;
        payload.lanes[8] = 1.0;
        bridge.update(0.1, Some(&payload), &Default::default());
        wave.p9.update(0.1, &bridge);

        let uniforms = wave.uniforms();
        // glow = (0.05 + band * 0.3) * glowScale
        assert!((uniforms.glow[0] - 0.2).abs() < 1e-6);
        assert!((uniforms.glow[1] - 0.05).abs() < 1e-6);
        assert!((uniforms.glow[2] - 0.35).abs() < 1e-6);
        // xScale = 1.0 + band * 0.05
        assert!((uniforms.x_scale[0] - 1.025).abs() < 1e-6);
        assert!((uniforms.x_scale[2] - 1.05).abs() < 1e-6);
        assert_eq!(uniforms.distortion, 0.05);
    }

    #[test]
    fn props_bind_distortion_amplitude_and_glow() {
        let mut wave = ChromaticWave::new();
        wave.apply_props(|key| match key {
            "distortion" => Some(0.2),
            "amplitude" => Some(2.0),
            "glow" => Some(1.5),
            _ => None,
        });

        assert_eq!(wave.distortion, 0.2);
        assert_eq!(wave.amp_scale, 2.0);
        assert_eq!(wave.glow_scale, 1.5);

        // Missing props fall back to the schema defaults.
        let mut defaults = ChromaticWave::new();
        defaults.apply_props(|_| None);
        assert_eq!(defaults.distortion, 0.05);
        assert_eq!(defaults.amp_scale, 1.0);
        assert_eq!(defaults.glow_scale, 1.0);
    }

    #[test]
    fn sample_count_matches_the_js_cap() {
        // 60 fps at 44.1 kHz -> 735 samples; a 100 ms frame caps at the
        // 1024-sample analyser window.
        assert_eq!(ChromaticWave::sample_count(44_100.0, 1.0 / 60.0), 735);
        assert_eq!(ChromaticWave::sample_count(44_100.0, 0.1), 1024);
        assert_eq!(ChromaticWave::sample_count(44_100.0, 0.0), 0);
        assert_eq!(ChromaticWave::sample_count(44_100.0, f32::NAN), 0);
        assert_eq!(ChromaticWave::sample_count(0.0, 0.016), 0);
    }

    #[test]
    fn wave_texture_rises_with_a_loud_signal() {
        let mut wave = ChromaticWave::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.wave = [0.75; AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();

        // ~6.6 s of signal so the whole ring horizon is filled.
        for _ in 0..400 {
            bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
            wave.update(1.0 / 60.0, &bridge);
        }

        let bytes = wave.wave_bytes();
        let red: Vec<u8> = (0..WAVE_N).map(|i| bytes[i * 4]).collect();
        // mean |0.75| -> byte ~191 (0.75 * 255 = 191.25, truncated).
        assert!(
            red.iter().all(|&byte| (190..=192).contains(&byte)),
            "unexpected texel range {:?}",
            (red.iter().min(), red.iter().max())
        );
    }

    #[test]
    fn meta_matches_the_wrapper_module() {
        let meta = ChromaticWave::new().meta();
        assert_eq!(meta.name, "Point10 Chromatic Wave");
        assert_eq!(meta.category, "2d-scenes");
        assert_eq!(meta.module_type, "2d");
    }

    #[test]
    fn wgsl_parses() {
        let source = format!("{FULLSCREEN_TRIANGLE_WGSL}\n{}", include_str!("wave.wgsl"));
        if let Err(error) = naga::front::wgsl::parse_str(&source) {
            panic!("wave.wgsl failed to parse: {error}");
        }
    }
}
