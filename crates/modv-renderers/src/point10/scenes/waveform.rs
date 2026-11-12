//! Waveform — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/Waveform.js`.
//!
//! # What the JS actually draws
//!
//! Despite the name this is **not** a full-screen waveform shader: `Waveform`
//! builds two `THREE.Line` strips of 256 vertices each, rotated
//! `group.rotation.x = -0.3`, seen by the adapter's
//! `PerspectiveCamera(60, aspect, 0.1, 1000)` at `z = 15` (`adapter.js:299-301`).
//! The y value of each vertex is `spectrum[i] / 255` times a sine (cyan line)
//! or cosine (magenta line) envelope, so the "waveform" is a helical FFT
//! ribbon. The port draws the same two line strips with a `LineStrip`
//! pipeline; the file name keeps the JS scene name.
//!
//! # Ported exactly
//!
//! * the geometry (`Waveform.js:20-55`): `points = 256`, line 1 at `z = 0`
//!   with `y = value * 6 * height * sin(i * 0.1 + phase)`, line 2 at `z = 2`
//!   with `y = value * 4 * height * cos(i * 0.15 + phase * 0.7)`, both spanning
//!   `x = (i / 256 - 0.5) * 30`;
//! * `phase += dt * speedScale` (only while audio is present, `:59-60`);
//! * the material opacities `0.8 * opacity` / `0.5 * opacity` (`:47-48`);
//! * the colours `HSL((phase * 0.1) % 1, 1, 0.6)` and
//!   `HSL((hue + 0.5) % 1, 1, 0.4)` (`:82-84`);
//! * `group.rotation.x = -0.3` (`:55`), applied in the vertex stage.
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `THREE.Line` + `LineBasicMaterial` | `PrimitiveTopology::LineStrip`, two draw calls over one 512-vertex buffer |
//! | `PerspectiveCamera` projection | `fov_scale = 1/tan(30°)` in the vertex stage |
//! | `material.color.setHSL` (working space) | [`hsl_to_linear_rgb`] on the CPU |
//! | `geometry.attributes.position.needsUpdate` | one `queue.write_buffer` of 512 vertices per frame |
//!
//! One deviation: WebGL ignores `linewidth` for lines, so the JS draws 1 px
//! lines; wgpu's `LineStrip` is also 1 px. No change.

#![forbid(unsafe_code)]

use super::particle_quad::hsl_to_linear_rgb;
use super::particle_quad::{CAMERA_Z, camera_fov_scale};
use crate::point10::audio_bridge::AudioBridge;
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, read_scene_props, scene_props_with_levers,
};

/// `Waveform.js: this.points = 256`.
const POINTS: usize = 256;
/// Two line strips share one vertex buffer.
const VERTEX_COUNT: usize = POINTS * 2;
/// `Waveform.js: this.group.rotation.x = -0.3` (radians).
const GROUP_TILT: f32 = -0.3;

/// `WAVEFORM_PROPS` (`sceneProps.js:132-136`).
const WAVEFORM_PROPS: &[SceneProp] = &[
    SceneProp::float("height", "Height", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("speed", "Wave Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("opacity", "Opacity", 1.0, 0.0, 1.0, 0.01),
];

/// One line vertex: position + which strip colour to use (0 = line 1, 1 = line 2).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct WaveformVertex {
    position: [f32; 3],
    line: f32,
}

/// Line uniforms, matching [`WAVEFORM_WGSL`] (64 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct WaveformUniforms {
    resolution: [f32; 2],
    aspect: f32,
    fov_scale: f32,
    tilt: f32,
    camera_z: f32,
    _pad0: [f32; 2],
    color_a: [f32; 4],
    color_b: [f32; 4],
}

/// GPU resources built in `init`.
struct WaveformGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
}

/// Waveform scene.
pub struct Waveform {
    /// `this._phase`.
    phase: f32,
    /// `this.heightScale`.
    height_scale: f32,
    /// `this.speedScale`.
    speed_scale: f32,
    /// `this.opacity`.
    opacity: f32,
    size: (u32, u32),
    gpu: Option<WaveformGpu>,
}

impl Default for Waveform {
    fn default() -> Self {
        Self::new()
    }
}

impl Waveform {
    /// Create the scene (`Waveform.js:17-22` defaults).
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            height_scale: 1.0,
            speed_scale: 1.0,
            opacity: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:431-435`):
    /// `heightScale = height`, `speedScale = speed`, `opacity`.
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(WAVEFORM_PROPS, props) {
            match key {
                "height" => self.height_scale = value,
                "speed" => self.speed_scale = value,
                "opacity" => self.opacity = value,
                _ => {}
            }
        }
    }

    /// Current phase (`this._phase`).
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// The 512 vertices the next draw would upload (for tests).
    fn vertices(&self, spectrum: &[u8]) -> [WaveformVertex; VERTEX_COUNT] {
        let mut vertices = [WaveformVertex {
            position: [0.0; 3],
            line: 0.0,
        }; VERTEX_COUNT];

        if spectrum.is_empty() {
            return vertices;
        }

        let phase = self.phase;
        let height = self.height_scale;

        for index in 0..POINTS {
            // `const x = (i / this.points - 0.5) * 30;`
            let x = (index as f32 / POINTS as f32 - 0.5) * 30.0;
            let value = spectrum[index % spectrum.len()] as f32 / 255.0;

            // Line 1: y = value * 6 * height * sin(i * 0.1 + phase), z = 0.
            vertices[index] = WaveformVertex {
                position: [
                    x,
                    value * 6.0 * height * (index as f32 * 0.1 + phase).sin(),
                    0.0,
                ],
                line: 0.0,
            };

            // Line 2: y = value * 4 * height * cos(i * 0.15 + phase * 0.7), z = 2.
            vertices[POINTS + index] = WaveformVertex {
                position: [
                    x,
                    value * 4.0 * height * (index as f32 * 0.15 + phase * 0.7).cos(),
                    2.0,
                ],
                line: 1.0,
            };
        }

        vertices
    }

    /// Uniform block for the current phase/opacity.
    fn uniforms(&self) -> WaveformUniforms {
        let hue = (self.phase * 0.1).rem_euclid(1.0);
        // `line.material.color.setHSL(hue, 1, 0.6)` / `line2 ... +0.5, 1, 0.4`
        let color_a = hsl_to_linear_rgb(hue, 1.0, 0.6);
        let color_b = hsl_to_linear_rgb((hue + 0.5).rem_euclid(1.0), 1.0, 0.4);

        WaveformUniforms {
            resolution: [self.size.0 as f32, self.size.1 as f32],
            aspect: self.size.0 as f32 / self.size.1 as f32,
            fov_scale: camera_fov_scale(),
            tilt: GROUP_TILT,
            camera_z: CAMERA_Z,
            _pad0: [0.0, 0.0],
            // `line.material.opacity = 0.8 * this.opacity` (`:47`)
            color_a: [color_a[0], color_a[1], color_a[2], 0.8 * self.opacity],
            // `line2.material.opacity = 0.5 * this.opacity` (`:48`)
            color_b: [color_b[0], color_b[1], color_b[2], 0.5 * self.opacity],
        }
    }

    fn upload(&self, gpu: &WaveformGpu, vertices: &[WaveformVertex; VERTEX_COUNT]) {
        gpu.queue.write_buffer(
            &gpu.vertex_buffer,
            0,
            bytemuck::cast_slice(vertices.as_slice()),
        );
        gpu.queue
            .write_buffer(&gpu.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms()));
    }
}

impl Point10Scene for Waveform {
    fn name(&self) -> &'static str {
        "Point10 Waveform"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = &ctx.device;
        let queue = &ctx.queue;
        self.size = (ctx.width, ctx.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-waveform-shader"),
            source: wgpu::ShaderSource::Wgsl(WAVEFORM_WGSL.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-waveform-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-waveform-uniforms"),
            size: std::mem::size_of::<WaveformUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-waveform-vertices"),
            size: std::mem::size_of::<WaveformVertex>() as u64 * VERTEX_COUNT as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-waveform-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-waveform-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-waveform-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<WaveformVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 3 * std::mem::size_of::<f32>() as u64,
                            shader_location: 1,
                        },
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let gpu = WaveformGpu {
            queue: queue.clone(),
            pipeline,
            bind_group,
            uniform_buffer,
            vertex_buffer,
        };
        // Flat lines until the first audio frame (`_init` leaves the position
        // array zeroed).
        let vertices = self.vertices(&[]);
        self.upload(&gpu, &vertices);
        self.gpu = Some(gpu);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        // `if (!audioData) return;` — the clock only drives the phase, which
        // also only advances with audio (`:57-60`).
        if audio.spectrum.is_empty() {
            return;
        }

        if dt.is_finite() && dt > 0.0 {
            self.phase += dt * self.speed_scale;
        }

        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let vertices = self.vertices(&audio.spectrum);
        self.upload(gpu, &vertices);
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-waveform"),
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
        pass.set_vertex_buffer(0, gpu.vertex_buffer.slice(..));
        // Two strips, line 1 then line 2 (`this.group.add(this.line)` then
        // `this.line2`, so line 2 draws last / on top).
        pass.draw(0..POINTS as u32, 0..1);
        pass.draw(POINTS as u32..VERTEX_COUNT as u32, 0..1);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(WAVEFORM_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10Waveform.js` sets no category, so the adapter default wins.
        SceneMeta::for_scene("Point10 Waveform", "3d-scenes")
    }
}

/// Two line strips driven by the byte spectrum.
const WAVEFORM_WGSL: &str = "\
struct WaveformUniforms {
  resolution: vec2<f32>, aspect: f32, fov_scale: f32,
  tilt: f32, camera_z: f32, _pad0: vec2<f32>,
  color_a: vec4<f32>, color_b: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: WaveformUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
}

// group.rotation.x = -0.3 (Waveform.js:55) then the adapter's perspective
// camera at (0, 0, 15) looking at the origin (adapter.js:299-301).
@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) line: f32) -> VsOut {
  let c = cos(u.tilt);
  let s = sin(u.tilt);
  let world = vec3<f32>(
    position.x,
    position.y * c - position.z * s,
    position.y * s + position.z * c,
  );
  let view = world - vec3<f32>(0.0, 0.0, u.camera_z);
  let dist = max(-view.z, 0.0001);
  let ndc = vec2<f32>(view.x * u.fov_scale / u.aspect, view.y * u.fov_scale) / dist;

  var out: VsOut;
  out.position = vec4<f32>(ndc, 0.0, 1.0);
  out.color = mix(u.color_a, u.color_b, line);
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  return in.color;
}
";

#[cfg(test)]
mod tests {
    use super::*;

    fn spectrum(len: usize, value: u8) -> Vec<u8> {
        vec![value; len]
    }

    #[test]
    fn geometry_matches_the_js_loops() {
        let wave = Waveform::new();
        let vertices = wave.vertices(&spectrum(1024, 255));

        for index in 0..POINTS {
            let x = (index as f32 / 256.0 - 0.5) * 30.0;
            // value = 1, height = 1, phase = 0
            assert!((vertices[index].position[0] - x).abs() < 1e-5);
            assert!((vertices[index].position[1] - 6.0 * (index as f32 * 0.1).sin()).abs() < 1e-4);
            assert_eq!(vertices[index].position[2], 0.0);
            assert_eq!(vertices[index].line, 0.0);

            assert!((vertices[POINTS + index].position[0] - x).abs() < 1e-5);
            assert!(
                (vertices[POINTS + index].position[1] - 4.0 * (index as f32 * 0.15).cos()).abs()
                    < 1e-4
            );
            assert_eq!(vertices[POINTS + index].position[2], 2.0);
            assert_eq!(vertices[POINTS + index].line, 1.0);
        }
    }

    #[test]
    fn silence_is_a_flat_line_and_spectrum_wraps() {
        let wave = Waveform::new();
        let flat = wave.vertices(&spectrum(1024, 0));
        for vertex in flat {
            assert_eq!(vertex.position[1], 0.0);
        }

        // `wave[i % wave.length]`: a 5-bin spectrum wraps around the 256 points.
        let short = wave.vertices(&[0, 255, 0, 0, 0]);
        assert!(short[0].position[1].abs() < 1e-6); // index 0 -> bin 0
        assert!((short[1].position[1] - 6.0 * (0.1_f32).sin()).abs() < 1e-4); // bin 1
        assert!(short[2].position[1].abs() < 1e-6); // bin 2
        // index 5 -> 5 % 5 = bin 0 (silent), index 6 -> bin 1 (loud).
        assert!(short[5].position[1].abs() < 1e-6);
        assert!((short[6].position[1] - 6.0 * (0.6_f32).sin()).abs() < 1e-4);
    }

    #[test]
    fn phase_advances_only_with_audio() {
        let mut wave = Waveform::new();
        wave.apply_props(|key| (key == "speed").then_some(2.0));
        let bridge = AudioBridge::new();
        wave.update(0.5, &bridge);
        assert_eq!(wave.phase(), 0.0);

        let mut payload = crate::point10::audio_bridge::BandPayload::silent(44_100.0);
        payload.byte_frequency_data = spectrum(1024, 200);
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        wave.update(0.5, &bridge);
        assert!((wave.phase() - 1.0).abs() < 1e-6, "0.5 * 2.0");
    }

    #[test]
    fn colours_follow_the_phase_and_opacity_prop() {
        let mut wave = Waveform::new();
        let start = wave.uniforms();
        // hue = 0 -> red-ish at lightness 0.6; line 2 is hue + 0.5.
        assert!(start.color_a[0] > start.color_a[1]);
        assert!((start.color_a[3] - 0.8).abs() < 1e-6);
        assert!((start.color_b[3] - 0.5).abs() < 1e-6);

        wave.apply_props(|key| (key == "opacity").then_some(0.5));
        wave.phase = 1.0; // hue = 0.1
        let later = wave.uniforms();
        assert!((later.color_a[3] - 0.4).abs() < 1e-6);
        assert!((later.color_b[3] - 0.25).abs() < 1e-6);
        assert_ne!(start.color_a, later.color_a);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut wave = Waveform::new();
        wave.apply_props(|key| match key {
            "height" => Some(9.0),
            "speed" => Some(9.0),
            "opacity" => Some(-1.0),
            _ => None,
        });
        assert_eq!(wave.height_scale, 4.0);
        assert_eq!(wave.speed_scale, 4.0);
        assert_eq!(wave.opacity, 0.0);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let wave = Waveform::new();
        assert_eq!(wave.meta().name, "Point10 Waveform");
        let names: Vec<&str> = wave.props().iter().map(|prop| prop.name).collect();
        assert_eq!(&names[names.len() - 3..], &["height", "speed", "opacity"]);
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn uniform_and_vertex_layouts_are_pod_sized() {
        assert_eq!(std::mem::size_of::<WaveformUniforms>(), 64);
        assert_eq!(std::mem::size_of::<WaveformVertex>(), 16);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let module = match naga::front::wgsl::parse_str(WAVEFORM_WGSL) {
            Ok(module) => module,
            Err(error) => panic!("WAVEFORM_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("WAVEFORM_WGSL must validate");
    }
}
