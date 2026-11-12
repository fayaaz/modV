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
//!   with its own FRAG and texel mapping; deliberately NOT ported here.
//!
//! Line-number references below (e.g. "ChromaticLine.js:171") are anchors,
//! not contracts — they drift with the source file.
#![forbid(unsafe_code)]

// ChromaticLine.js `_init()` (lines 71-83): the construction constants.
/// Samples per texel (~23 ms at 44.1 kHz). `ChromaticLine.js: this.SPX = 1024`.
const SPX: usize = 1024;
/// Texel count of the wave texture. `ChromaticLine.js: this.WAVE_N = 256`.
const WAVE_N: usize = 256;
/// Displayed horizon in samples (SPX * WAVE_N, ~5.9 s). `ChromaticLine.js: this.D`.
const HORIZON: usize = SPX * WAVE_N;
/// Ring length: horizon + one frame's append burst headroom.
/// `ChromaticLine.js: this.R = this.D + 4096` (line 75).
const RING_LEN: usize = HORIZON + 4096;
/// Byte size of the 256x1 RGBA wave texture payload.
const WAVE_BYTES: usize = WAVE_N * 4;
/// Sample count of `P9Audio.wave`. `p10Audio.js: this.wave = new Float32Array(1024)` (line 18).
/// `k = min(round(rate * dt), wave.length)` in ChromaticLine.js:168.
const SAMPLE_CAP: usize = 1024;
/// Sample rate of [`SyntheticSine`].
const SAMPLE_RATE: u32 = 48_000;
/// Fixture sine amplitude. Note the ring stores the raw signed sample, but
/// texel bytes are built from mean |v|, so the sign does not leak through.
const SINE_AMPLITUDE: f32 = 0.9;
/// Tau of the damped band read that approximates P9Audio damping lane 2
/// (`band(-1, 2, n)`, p10Audio.js:71-80). P9Audio lanes 1..=3 are one-pole
/// low-passes; the bridge computes them with tau = 0.02 + smoothing*0.6 s.
/// The fixture uses a fixed lane-2 tau of 0.25 s as specified for this port.
const BAND_TAU_SECONDS: f32 = 0.25;

/// Chromatic Line uniform block, matching `chromatic_line.wgsl` member for
/// member (80 bytes, `#[repr(C)]`, all members at the offsets WGSL computes).
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
    /// note in `chromatic_line.wgsl`).
    pub _pad4: [f32; 3],
}

/// Fixture audio source for the scene: a sine wave at `freq_hz`.
///
/// This stands in for the live analyser path (`AudioBands` → `audioBridge` →
/// `P9Audio.wave`) that the scene consumes in production. Samples are
/// generated on demand from an absolute sample counter, 48 000 Hz, mono
/// (duplicated to both channels in the real graph; the scene reads a single
/// mono waveform), amplitude 0.9.
pub struct SyntheticSine {
    freq_hz: f32,
    sample_index: u64,
}

impl SyntheticSine {
    /// Create a 48 000 Hz sine fixture at `freq_hz`. The absolute sample
    /// counter starts at 0, so the generated sequence is deterministic.
    pub fn new(freq_hz: f32) -> Self {
        Self {
            freq_hz,
            sample_index: 0,
        }
    }

    /// Sample rate of the fixture in Hz (always 48 000).
    pub fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    /// One raw signed sample of the fixture at the absolute sample index.
    fn next_sample(&mut self) -> f32 {
        let t = self.sample_index as f32 / SAMPLE_RATE as f32;
        self.sample_index += 1;
        SINE_AMPLITUDE * (2.0 * std::f32::consts::PI * self.freq_hz * t).sin()
    }

    /// The three band targets the fixture reports through the P9Audio
    /// damping-2 lane, mirroring the bass/mid/treble split the audio bridge
    /// computes for a tone: sub-150 Hz lands in bass, up to 2 kHz in mid,
    /// above that in treble. The envelope is a steady 0.5 (a sine's RMS
    /// times ~1.4, capped) so the band read is deterministic.
    fn band_targets(&self) -> [f32; 3] {
        const A: f32 = 0.5;
        if self.freq_hz <= 150.0 {
            [A, 0.0, 0.0]
        } else if self.freq_hz <= 2000.0 {
            [0.0, A, 0.0]
        } else {
            [0.0, 0.0, A]
        }
    }
}

/// CPU-side simulation state: the ring buffer, the texel windows and the
/// damped band read. GPU-free so the unit tests can drive it directly
/// without a device.
struct ChromaticLineSim {
    /// Ring of the last `RING_LEN` samples. `ChromaticLine.js: this.ring = new Float32Array(this.R)` (line 76).
    ring: Vec<f32>,
    /// Write position into the ring. `ChromaticLine.js: this.head = 0` (line 77).
    head: usize,
    /// Damped band read (`P9Audio._smoo` lanes 6..=8, i.e. damping lane 2 for
    /// bands 0..=2). `p10Audio.js` lane layout: `[damping * 3 + band]` (lines 24-26).
    smoo: [f32; 3],
    /// `ChromaticLine.js: this.ampScale = 1` (line 81).
    amp_scale: f32,
    /// `ChromaticLine.js: this.glowScale = 1` (line 81).
    glow_scale: f32,
    /// `ChromaticLine.js: this.distortion = 0.05` (line 79).
    distortion: f32,
}

/// Output of one simulation advance: the rebuilt wave texture payload and the
/// band-driven uniforms. Kept separate from the GPU write so tests can
/// inspect the values without a device.
struct AdvanceOutput {
    bytes: [u8; WAVE_BYTES],
    band_uniforms: BandUniforms,
}

/// Band-driven uniform values, before `resolution` is merged in.
/// `ChromaticLine.js: _audioUniforms` (lines 137-151).
struct BandUniforms {
    amp: [f32; 3],
    glow: [f32; 3],
    x_scale: [f32; 3],
    distortion: f32,
}

impl ChromaticLineSim {
    /// `ChromaticLine.js: _init()` ring setup (lines 71-83).
    fn new() -> Self {
        Self {
            ring: vec![0.0; RING_LEN],
            head: 0,
            smoo: [0.0; 3],
            amp_scale: 1.0,
            glow_scale: 1.0,
            distortion: 0.05,
        }
    }

    /// Append `samples` to the ring, wrapping at `RING_LEN`.
    ///
    /// `ChromaticLine.js update()` (lines 171-175):
    /// `ring[(this.head + m) % this.R] = w[w.length - k + m]; this.head = (this.head + k) % this.R;`
    /// The fixture generates exactly the new samples, so the samples are
    /// appended directly instead of re-indexed from the analyser tail.
    fn append(&mut self, samples: &[f32]) {
        for (m, &v) in samples.iter().enumerate() {
            self.ring[(self.head + m) % RING_LEN] = v;
        }
        self.head = (self.head + samples.len()) % RING_LEN;
    }

    /// Rebuild the 256x1 RGBA wave texture payload from the ring.
    ///
    /// `ChromaticLine.js update()` texel loop (lines 180-205): each texel is
    /// the mean |v| of its own 23 ms window starting at the texel's ring
    /// position (`m = head - D + i*SPX mod R`), biased up from centre so the
    /// unsigned texture never clips negative values.
    fn compute_bytes(&self) -> [u8; WAVE_BYTES] {
        let span = RING_LEN as i64;
        let mut bytes = [0_u8; WAVE_BYTES];
        for i in 0..WAVE_N {
            // `let m = (this.head - D + i * S) % R; if (m < 0) m += R;` —
            // rem_euclid is the same modulo with the sign handled, and the
            // i64 intermediates cannot overflow for these magnitudes.
            let base = self.head as i64 - HORIZON as i64 + i as i64 * SPX as i64;
            let mut m = base.rem_euclid(span) as usize;
            let mut sum = 0.0_f32;
            for _ in 0..SPX {
                sum += self.ring[m].abs();
                m += 1;
                if m == RING_LEN {
                    m = 0;
                }
            }
            // `const q = sum / S;` — 0..1 mean |v| of this window.
            let e = sum / SPX as f32;
            // Texel byte from the window's mean |v|. `texelValue(e) = 0.5 + 0.5*e`
            // (biased up from centre), `texelByte = 128 + ((texelValue * 127) | 0)`.
            // Ported exactly, warts and all: |0 is truncation toward zero, and
            // the in/out range is [0.5, 1.0] -> t*127 in [63.5, 127] ->
            // truncated [63, 127] -> byte in [191, 255]. Silence therefore
            // yields 191 (not 128): the 0.5 bias exists so single-pole wave
            // textures never clip negative values, and silence is harmless
            // because amp is 0 in silence. Do NOT "fix" this to 128 — the
            // visual baseline is `amp`, not the texel value. `ebar` (the JS
            // running mean across texels) is unused by texelByte and dropped.
            let t = 0.5 + 0.5 * e;
            let byte = (128.0 + (t * 127.0).trunc()) as u8;
            let o = i * 4;
            bytes[o] = byte;
            bytes[o + 1] = byte;
            bytes[o + 2] = byte;
            bytes[o + 3] = 255;
        }
        bytes
    }

    /// Advance the one-pole damped band read toward `target`.
    ///
    /// `P9Audio.band(-1, 2, n)` (p10Audio.js:71-80) reads `_smoo[2*3+n]`;
    /// lane 2 is a low-pass lane. The audio bridge computes the lanes with a
    /// per-lane one-pole; this fixture uses a fixed lane-2 time constant of
    /// `BAND_TAU_SECONDS` (0.25 s). Returns the updated damped values.
    fn update_bands(&mut self, dt_seconds: f32, target: [f32; 3]) -> [f32; 3] {
        let k = 1.0 - (-dt_seconds / BAND_TAU_SECONDS).exp();
        for i in 0..3 {
            self.smoo[i] += (target[i] - self.smoo[i]) * k;
        }
        self.smoo
    }

    /// Map damped bands to the band-driven uniforms.
    ///
    /// `ChromaticLine.js _audioUniforms` (lines 137-151):
    /// `amp[i] = min(b*10, 0.8) * ampScale`, `glow[i] = (0.05 + b*0.08) * glowScale`,
    /// `xScale[i] = 2.5 + b*0.8`, `distortion` never changes.
    fn band_uniforms(&self, bands: [f32; 3]) -> BandUniforms {
        let mut amp = [0.0_f32; 3];
        let mut glow = [0.0_f32; 3];
        let mut x_scale = [0.0_f32; 3];
        for i in 0..3 {
            amp[i] = (bands[i] * 10.0).min(0.8) * self.amp_scale;
            glow[i] = (0.05 + bands[i] * 0.08) * self.glow_scale;
            x_scale[i] = 2.5 + bands[i] * 0.8;
        }
        BandUniforms {
            amp,
            glow,
            x_scale,
            distortion: self.distortion,
        }
    }

    /// Advance the simulation by `dt_seconds`: pull the genuinely new samples
    /// from the fixture (if any), rebuild the texel bytes and return them
    /// with the band-driven uniforms.
    ///
    /// `ChromaticLine.js update()` (lines 159-217), audio/ring/texel parts:
    /// `const k = Math.min(Math.round(this.rate * dt), this.p9.wave.length);`
    /// with `this.rate = audio.ctx?.sampleRate || 44100` (line 156) — here the
    /// fixture's rate (48 000) is used when audio is present.
    fn advance(&mut self, dt_seconds: f32, audio: &mut Option<SyntheticSine>) -> AdvanceOutput {
        let mut bands = [0.0_f32; 3];
        if let Some(sine) = audio.as_mut() {
            let rate = sine.sample_rate() as f32;
            // Guard pathological dt (NaN/negative) so the `as usize` cast
            // can never panic; a zero/negative k appends nothing, matching
            // the JS `if (k > 0)` guard (ChromaticLine.js:169).
            let k = ((rate * dt_seconds).round().max(0.0) as usize).min(SAMPLE_CAP);
            let mut samples = Vec::with_capacity(k);
            for _ in 0..k {
                samples.push(sine.next_sample());
            }
            self.append(&samples);
            bands = self.update_bands(dt_seconds, sine.band_targets());
        }
        // No audio (p9 undefined): `band()` returns 0 for every channel
        // (p10Audio.js:71-80), so amp falls to 0 and glow to 0.05 — the idle
        // glow line. The damped lanes are left untouched, matching the JS.
        let bytes = self.compute_bytes();
        let band_uniforms = self.band_uniforms(bands);
        AdvanceOutput {
            bytes,
            band_uniforms,
        }
    }
}

fn build_uniforms(size: (u32, u32), band: &BandUniforms) -> ChromaticLineUniforms {
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
            source: wgpu::ShaderSource::Wgsl(include_str!("chromatic_line.wgsl").into()),
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

    /// The naga dev-dependency (wgsl-in) must accept the shader as written.
    #[test]
    fn wgsl_parses() {
        let result = naga::front::wgsl::parse_str(include_str!("chromatic_line.wgsl"));
        if let Err(error) = &result {
            panic!("chromatic_line.wgsl failed to parse: {error}");
        }
    }
}
