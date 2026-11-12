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
//!   the scene reads `band(-1, 2, n)` (damping lane 2, bands 0..=2) through
//!   [`P10Audio`], exactly like the JS.
//! - `src/renderer/src/application/point10/scenes/ChromaticWave.js` — subclass
//!   with its own FRAG and texel mapping; ported in [`super::wave`] on top of
//!   the shared simulation in [`super`].
//!
//! The ring buffer, texel windows and band read live in [`super`] so the wave
//! variant can reuse them; this module keeps the line's scene struct, its
//! uniform block and its WGSL.
//!
//! Production updates are bridge-driven: [`Point10Scene::update`] pulls the
//! genuinely new samples off [`AudioBridge::wave`]'s tail (like ChromaticWave)
//! and reads the bands through [`P10Audio`]. [`SyntheticSine`] exists only for
//! tests and the headless frame harness — it fills a [`BandPayload`] which is
//! then pushed through a real bridge, so the fixture never runs a second
//! implementation of the audio path.
//!
//! Line-number references below (e.g. "ChromaticLine.js:171") are anchors,
//! not contracts — they drift with the source file.
//!
//! [`Point10Scene::update`]: crate::point10::Point10Scene::update
//! [`BandPayload`]: crate::point10::audio_bridge::BandPayload
//! [`SyntheticSine`]: super::SyntheticSine
#![forbid(unsafe_code)]

use super::{
    BandUniforms, CHROMATIC_PROPS, ChromaticLineSim, TexelMapping, WAVE_BYTES, WAVE_N,
    sample_count, upload_wave_texture,
};
use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge, P10Audio};
use crate::point10::{
    SceneContext, SceneMeta, SceneProp, read_scene_props, scene_props_with_levers,
};

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

/// GPU resources built in [`crate::point10::Point10Scene::init`].
struct LineGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    wave_texture: wgpu::Texture,
}

impl LineGpu {
    /// Record the full-screen triangle into `pass`.
    fn draw<'rp>(&'rp self, pass: &mut wgpu::RenderPass<'rp>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Full-screen Chromatic Line scene: pure CPU simulation feeding a 256x1
/// wave texture, drawn as a single full-screen triangle with alpha blending.
///
/// [`Point10Scene::render`] clears the target to transparent black, so the
/// rest of the frame composites underneath. A caller that owns its own pass
/// (the headless harness keeps a near-black backdrop) can call [`Self::draw`]
/// instead.
pub struct ChromaticLine {
    /// CPU-side ring/texel/band state.
    sim: ChromaticLineSim,
    /// P9Audio-compatible reader over the bridge.
    p9: P10Audio,
    /// Drawing-buffer size in pixels. Defaults to `(1, 1)`; `update()` rewrites
    /// the `resolution` uniform from this every frame (`_syncResolution`,
    /// ChromaticLine.js:220-228, folds into the per-frame uniform write).
    size: (u32, u32),
    gpu: Option<LineGpu>,
}

impl Default for ChromaticLine {
    fn default() -> Self {
        Self::new()
    }
}

impl ChromaticLine {
    /// Create the scene. GPU resources are built on
    /// [`crate::point10::Point10Scene::init`].
    pub fn new() -> Self {
        Self {
            sim: ChromaticLineSim::new(),
            p9: P10Audio::new(),
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props.
    ///
    /// `sceneProps.js: SCENE_PROP_CONFIGS["Point10 Chromatic Line"]` (lines
    /// 369-373) — `distortion`, `ampScale = amplitude`, `glowScale = glow`.
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        let values = read_scene_props(CHROMATIC_PROPS, props);
        for (key, value) in values {
            match key {
                "distortion" => self.sim.set_distortion(value),
                "amplitude" => self.sim.set_amp_scale(value),
                "glow" => self.sim.set_glow_scale(value),
                _ => {}
            }
        }
    }

    /// Current line texture payload (for tests and golden-image diffing).
    pub fn wave_bytes(&self) -> [u8; WAVE_BYTES] {
        self.sim.compute_bytes(TexelMapping::Line)
    }

    /// Band-driven uniforms for the current state.
    ///
    /// `ChromaticLine.js: update()` (lines 212-216) reads
    /// `p9.band(-1, 2, n)` — the bridge's damping lane 2 — and
    /// `_audioUniforms` (lines 137-151) maps it.
    fn uniforms(&self) -> ChromaticLineUniforms {
        let bands = [
            self.p9.band(-1, 2, 0, 0.0),
            self.p9.band(-1, 2, 1, 0.0),
            self.p9.band(-1, 2, 2, 0.0),
        ];
        build_uniforms(self.size, &self.sim.band_uniforms(bands))
    }

    /// Draw into a render pass the caller already opened (and cleared).
    ///
    /// A no-op after [`crate::point10::Point10Scene::dispose`].
    pub fn draw<'rp>(&'rp self, pass: &mut wgpu::RenderPass<'rp>) {
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.draw(pass);
        }
    }
}

impl crate::point10::Point10Scene for ChromaticLine {
    fn name(&self) -> &'static str {
        "Point10 Chromatic Line"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = &ctx.device;
        let queue = &ctx.queue;

        // `line.wgsl` carries its own `vs_main` (the raw NDC triangle from
        // ChromaticLine.js VERT), so it is compiled as-is rather than on top of
        // `FULLSCREEN_TRIANGLE_WGSL`.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-chromatic-line-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("line.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-chromatic-line-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        // Shader requires 80 bytes; the buffer is exactly 80.
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

        // `ChromaticLine.js: this.waveTex = new THREE.DataTexture(...)` (lines
        // 85-94): 256x1 RGBA, Linear filters, Repeat/REPEAT on S.
        let wave_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point10-chromatic-line-texels"),
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
            label: Some("point10-chromatic-line-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-chromatic-line-uniforms"),
            size: std::mem::size_of::<ChromaticLineUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-chromatic-line-bind-group"),
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
            label: Some("point10-chromatic-line-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = ctx.fullscreen_pipeline(
            "point10-chromatic-line-pipeline",
            &shader,
            &pipeline_layout,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        // Seed the texture with the line's silence texel (the 191 bias the
        // first update() writes anyway). The JS `waveData.fill(255)` before the
        // first frame is a pre-upload placeholder; 255 and 191 both put the
        // flat line on the same midline because `amp` is 0 in silence.
        let silence = self.sim.compute_bytes(TexelMapping::Line);
        upload_wave_texture(queue, &wave_texture, &silence);

        self.size = (ctx.width, ctx.height);
        self.gpu = Some(LineGpu {
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
        self.p9.update(dt, audio);

        // Append the genuinely new samples (the analyser's tail) so the
        // waveform scrolls in real time (`ChromaticLine.js:167-176`).
        if self.p9.has_audio() {
            let k = sample_count(audio.sample_rate, dt);
            if k > 0 {
                self.sim.append_wave_tail(&audio.wave[AUDIO_SAMPLES - k..]);
            }
        }

        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let bytes = self.wave_bytes();
        upload_wave_texture(&gpu.queue, &gpu.wave_texture, &bytes);
        let uniforms = self.uniforms();
        gpu.queue
            .write_buffer(&gpu.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        // BaseScene renders through the adapter's renderer, which clears to
        // transparent black (`adapter.js:297`), so an empty frame composites
        // over the layers below.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-chromatic-line"),
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
        if let Some(gpu) = self.gpu.as_ref() {
            gpu.draw(&mut pass);
        }
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(CHROMATIC_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10ChromaticLine.js`: `meta: { category: "2d-scenes" }` — the
        // fullscreen 2D scenes override the adapter's "3d-scenes" default.
        SceneMeta::for_scene("Point10 Chromatic Line", "2d-scenes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::Point10Scene;
    use crate::point10::audio_bridge::{BandPayload, RESPONSIVENESS_PROPS};
    use crate::point10::chromatic::{RING_LEN, SyntheticSine};

    /// Bridge carrying `payload` for one frame.
    fn bridge_with(payload: &BandPayload) -> AudioBridge {
        let mut bridge = AudioBridge::new();
        bridge.update(1.0 / 60.0, Some(payload), &Default::default());
        bridge
    }

    /// Without any audio the ring stays silent, which maps to the biased texel
    /// value (0.5+0.5*0=0.5) truncated to the byte 128+63 = 191. Ported exactly
    /// from the JS (`texelByte`, ChromaticLine.js:131-133): the 0.5 bias exists
    /// so the unsigned texture never clips negative wave values. Silence
    /// therefore reads as 191, NOT 128 — this test documents the wart instead
    /// of "fixing" it.
    #[test]
    fn silence_produces_bias_byte_191() {
        let mut line = ChromaticLine::new();
        let mut bytes = [0_u8; WAVE_BYTES];
        for _ in 0..10 {
            // Several ~60 fps updates of a bridge that never saw a payload.
            line.update(1.0 / 60.0, &AudioBridge::new());
            bytes = line.wave_bytes();
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

    /// A bridge with a silent payload (audio present, zero level) must keep the
    /// line flat on the midline: every texel at the 191 bias, `amp` 0 and only
    /// the idle 0.05 glow — the "flat glowing line in silence" the JS header
    /// describes.
    #[test]
    fn audio_driven_silence_keeps_the_flat_midline() {
        let mut line = ChromaticLine::new();
        let payload = BandPayload::silent(44_100.0);
        let mut bridge = AudioBridge::new();
        for _ in 0..10 {
            bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
            line.update(1.0 / 60.0, &bridge);
        }

        let bytes = line.wave_bytes();
        for i in 0..WAVE_N {
            let o = i * 4;
            assert_eq!(&bytes[o..o + 3], &[191, 191, 191], "texel {i} not the bias");
            assert_eq!(bytes[o + 3], 255, "alpha texel {i} must stay 255");
        }

        let uniforms = line.uniforms();
        assert_eq!(uniforms.amp, [0.0; 3], "silence must not displace the line");
        for i in 0..3 {
            assert!((uniforms.glow[i] - 0.05).abs() < 1e-6, "glow[{i}]");
            assert!((uniforms.x_scale[i] - 2.5).abs() < 1e-6, "x_scale[{i}]");
        }
        assert_eq!(uniforms.distortion, 0.05);
    }

    /// Ring append must wrap at RING_LEN without panicking and leave `head`
    /// pointing at the next write position.
    #[test]
    fn ring_advances_and_wraps() {
        let mut sim = ChromaticLineSim::new();
        sim.append_wave_tail(&vec![0.5; 5000]);
        assert_eq!(sim.head, 5000 % RING_LEN);
        sim.append_wave_tail(&vec![0.5; RING_LEN]);
        assert_eq!(sim.head, 5000 % RING_LEN);
    }

    /// A 110 Hz sine pushed through the *real* audio path for 400 frames
    /// (~6.6 s) must lift texel bytes above the silence bias: the windows
    /// contain the near-full-scale signal.
    ///
    /// The fixture only fills a [`BandPayload`]; the bridge and the scene then
    /// run their production code, so this covers the tail append and the texel
    /// windows exactly as live audio would.
    #[test]
    fn full_scale_sine_raises_bytes() {
        let mut line = ChromaticLine::new();
        let mut sine = SyntheticSine::new(110.0);
        let mut payload = BandPayload::silent(sine.sample_rate() as f32);
        let mut bridge = AudioBridge::new();
        let mut bytes = [0_u8; WAVE_BYTES];
        for _ in 0..400 {
            sine.fill_payload(&mut payload, 1.0 / 60.0);
            bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
            line.update(1.0 / 60.0, &bridge);
            bytes = line.wave_bytes();
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
        // The 110 Hz tone lands in bass only, so red reads a level and the
        // other two lanes stay silent (no displacement, idle glow).
        let uniforms = line.uniforms();
        assert!(uniforms.amp[0] > 0.0, "bass must displace the red channel");
        assert_eq!(uniforms.amp[1], 0.0);
        assert_eq!(uniforms.amp[2], 0.0);
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

    /// The scene reads the bridge's damping-2 lane, not the raw band: the
    /// uniforms must follow lanes 6..=8 (`p10Audio.js: band(-1, 2, n)`).
    #[test]
    fn damping_lane_two_drives_the_uniforms() {
        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[0] = 1.0;
        payload.lanes[6] = 0.05;
        payload.lanes[8] = 1.0;

        let mut line = ChromaticLine::new();
        line.update(1.0 / 60.0, &bridge_with(&payload));

        let uniforms = line.uniforms();
        assert!((uniforms.amp[0] - 0.5).abs() < 1e-6, "amp[0]");
        assert_eq!(uniforms.amp[1], 0.0);
        assert!(
            (uniforms.amp[2] - 0.8).abs() < 1e-6,
            "amp[2] must cap at 0.8"
        );
        assert!((uniforms.glow[0] - (0.05 + 0.05 * 0.08)).abs() < 1e-6);
        assert!((uniforms.x_scale[2] - (2.5 + 0.8)).abs() < 1e-6);
    }

    /// `sceneProps.js` binder: distortion / amplitude / glow scale the schema
    /// values and default to 1.0 (distortion 0.05).
    #[test]
    fn props_bind_distortion_amplitude_and_glow() {
        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[6] = 0.05;
        let bridge = bridge_with(&payload);

        let mut defaults = ChromaticLine::new();
        defaults.apply_props(|_| None);
        defaults.update(1.0 / 60.0, &bridge);
        let u = defaults.uniforms();
        assert_eq!(u.distortion, 0.05);
        assert!((u.amp[0] - 0.5).abs() < 1e-6);
        assert!((u.glow[0] - 0.054).abs() < 1e-6);

        let mut tuned = ChromaticLine::new();
        tuned.apply_props(|key| match key {
            "distortion" => Some(0.2),
            "amplitude" => Some(2.0),
            "glow" => Some(1.5),
            _ => None,
        });
        tuned.update(1.0 / 60.0, &bridge);
        let u = tuned.uniforms();
        assert_eq!(u.distortion, 0.2);
        // amp = min(0.05 * 10, 0.8) * 2.0; glow = (0.05 + 0.05 * 0.08) * 1.5.
        assert!((u.amp[0] - 1.0).abs() < 1e-6, "amp[0] = {}", u.amp[0]);
        assert!((u.glow[0] - 0.081).abs() < 1e-6, "glow[0] = {}", u.glow[0]);
    }

    /// The prop schema is `CHROMATIC_LINE_PROPS` (distortion/amplitude/glow)
    /// behind the shared responsiveness levers.
    #[test]
    fn props_include_the_chromatic_schema_and_the_levers() {
        let props = ChromaticLine::new().props();
        assert_eq!(props.len(), RESPONSIVENESS_PROPS.len() + 3);
        for levers in RESPONSIVENESS_PROPS {
            assert!(props.iter().any(|prop| prop.name == levers.name));
        }

        let own: Vec<&str> = props
            .iter()
            .skip(RESPONSIVENESS_PROPS.len())
            .map(|prop| prop.name)
            .collect();
        assert_eq!(own, vec!["distortion", "amplitude", "glow"]);

        let distortion = props
            .iter()
            .find(|prop| prop.name == "distortion")
            .expect("distortion prop");
        assert_eq!(distortion.default, 0.05);
        assert_eq!(distortion.min, 0.0);
        assert_eq!(distortion.max, 0.5);
        assert_eq!(distortion.step, 0.001);
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

    /// The registry name/category must match the wrapper module
    /// (`renderers/2d/modules/Point10ChromaticLine.js`).
    #[test]
    fn meta_matches_the_wrapper_module() {
        let meta = ChromaticLine::new().meta();
        assert_eq!(meta.name, "Point10 Chromatic Line");
        assert_eq!(meta.category, "2d-scenes");
        assert_eq!(meta.module_type, "2d");
        assert_eq!(meta.author, "Point10");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.audio_features, &["bass", "mid", "treble"]);
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
