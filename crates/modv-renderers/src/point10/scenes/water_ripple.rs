//! Water Ripple — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/WaterRipple.js`.
//!
//! A fullscreen shader over the CPU water height field
//! ([`crate::point10::shared::water_wave::WaveSim`]): the field distorts a
//! swirling concentric-ring pattern, eight expanding impact rings are
//! composited on top, and the whole thing is tinted by a phrase-colour gradient
//! that sweeps and drifts. Only the circular pattern is opaque — everything
//! outside it fades to alpha 0 so the scene composites over the layers below
//! (`CLEAR_ALPHA = 0`).
//!
//! # Ported exactly
//!
//! * the fragment shader (`WaterRipple.js:42-121`) line for line: the water
//!   height read `* (2.4 + uBass * 1.6)` clamped to ±0.65, the eight
//!   `impactRipple` rings (`exp` ring + inner ring * 0.28 + a bead), the
//!   circle mask `1 - smoothstep(circleRadius ± 0.1, length(screenP))`, the
//!   swirl `p *= mat2(cos, -sin, sin, cos)` and the three `tanh` pattern
//!   channels, `body`, `intensity`, the polar gradient sweep/drift and the rim;
//! * `waterRippleDrive` (`:22-32`) and the prop binder (`sceneProps.js:425-430`):
//!   `speedScale`, `distortionScale`, `radiusScale`, `brightnessScale`;
//! * the drop scheduler (`:237-303`): the seeded centre drop, the bass-attack
//!   bursts (2-4 drops on a spiral path, queued with 0.042 s spacing), the
//!   ambient drop every 1.8 s on an elliptical path, the 0.16 s bass-drop
//!   cooldown and the impact ring ageing;
//! * the phrase colour: `dropColorPulse` + the eased hue (`min(1, dt * 0.55)`)
//!   and `equalizedHue` of the three base colours;
//! * `CLEAR_ALPHA = 0` (`:15`) — the render pass clears to transparent black
//!   and the shader writes `inCircle` as alpha.
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `WaveSim` GPU ping-pong (`shared/waterWave.js`) | [`WaveSim`] CPU field + one `Rgba16Float` upload per frame |
//! | `PlaneGeometry(2, 2)` + raw vertex shader | [`crate::point10::FULLSCREEN_TRIANGLE_WGSL`] |
//! | `THREE.Vector4[8]` `uImpacts` | `array<vec4<f32>, 8>` in the uniform block |
//! | `THREE.Color` uniforms | linear RGB, converted from the JS hex colours at construction ([`srgb_hex_to_linear`]) |
//!
//! # Deviations
//!
//! * The JS material is a raw `ShaderMaterial` with no `OutputPass`, so its
//!   linear colours land on the sRGB canvas unconverted. The Rust port follows
//!   the shared scene convention (linear colour, the composer's output pass
//!   applies the sRGB transfer), so the pattern is brighter than the JS at the
//!   same audio level — motion, geometry and alpha are unchanged.
//! * `uDistortion` is declared by the JS shader but never read (the shader
//!   compile drops it). The port keeps the uniform and the prop binder for
//!   preset parity, and the shader leaves it unused as well.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::shared::phrase_color::{
    DropColorState, drop_color_pulse, ease_phrase_hue, equalized_hue, srgb_hex_to_linear,
};
use crate::point10::shared::water_wave::{WAVE_RES, WaveSim, create_wave_texture, wave_sampler};
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt,
    read_scene_props, scene_props_with_levers,
};

/// `WaterRipple.js: const WAVE_RES = 256`.
pub const WATER_RES: u32 = WAVE_RES;
/// `WaterRipple.js: const IMPACT_COUNT = 8`.
pub const IMPACT_COUNT: usize = 8;
/// `WaterRipple.js: export const CLEAR_ALPHA = 0`.
pub const CLEAR_ALPHA: f64 = 0.0;
/// `WaterRipple.js: const clamp01 = …`.
fn clamp01(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// `waterRippleDrive(bass, pulse)` — the audio-driven shader dials.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterRippleDrive {
    /// `uDistortion` (unused by the shader; kept for preset parity).
    pub distortion: f32,
    /// `uSpeed`.
    pub speed: f32,
    /// `uRadius`.
    pub radius: f32,
    /// `uBrightness`.
    pub brightness: f32,
    /// The strength multiplier the drop scheduler puts on its queues.
    pub drop_strength: f32,
}

/// `WaterRipple.js:22-32`, with both inputs clamped to 0..1.
pub fn water_ripple_drive(bass: f32, pulse: f32) -> WaterRippleDrive {
    let b = clamp01(bass);
    let p = clamp01(pulse);
    WaterRippleDrive {
        distortion: 0.55 + b * 1.1 + p * 0.55,
        speed: 1.3 + b * 0.5 + p * 0.2,
        radius: 0.9 + b * 0.1 + p * 0.03,
        brightness: 0.68 + b * 0.5 + p * 0.2,
        drop_strength: 0.012 + b * 0.022 + p * 0.025,
    }
}

/// One `THREE.Vector4(x, y, age, weight)` impact ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    /// U coordinate of the drop.
    pub x: f32,
    /// V coordinate of the drop.
    pub y: f32,
    /// Seconds since the drop (`impact.z`).
    pub age: f32,
    /// Ring weight (`impact.w`).
    pub weight: f32,
}

impl Impact {
    /// The seeded initial ring (`new THREE.Vector4(0.5, 0.5, 10, 0)`): old
    /// enough to be invisible.
    pub const INITIAL: Self = Self {
        x: 0.5,
        y: 0.5,
        age: 10.0,
        weight: 0.0,
    };

    /// The four components as the shader sees them.
    pub fn as_vec4(self) -> [f32; 4] {
        [self.x, self.y, self.age, self.weight]
    }
}

/// A queued drop (`_queueDrop`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct QueuedDrop {
    /// Scene time the drop fires at.
    at: f32,
    x: f32,
    y: f32,
    strength: f32,
    radius: f32,
}

/// `WATER_RIPPLE_PROPS` (`sceneProps.js:127-132`).
const WATER_RIPPLE_PROPS: &[SceneProp] = &[
    SceneProp::float("speed", "Ripple Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("distortion", "Distortion", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("radius", "Radius", 1.0, 0.0, 2.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

/// The JS base colours (`WaterRipple.js:145-149`), linear working space.
const BASE_COLORS_HEX: [u32; 3] = [0x28_7c_ff, 0x55_d7_ff, 0x8c_62_ff];

/// Uniform block, matching [`WATER_RIPPLE_WGSL`] (224 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct WaterRippleUniforms {
    resolution: [f32; 2],
    time: f32,
    bass: f32,
    pulse: f32,
    distortion: f32,
    speed: f32,
    radius: f32,
    brightness: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    impacts: [[f32; 4]; IMPACT_COUNT],
    color_a: [f32; 4],
    color_b: [f32; 4],
    color_c: [f32; 4],
}

/// GPU resources built in `init`.
struct WaterRippleGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    water_texture: wgpu::Texture,
}

/// WaterRipple scene.
pub struct WaterRipple {
    audio: P10Audio,
    /// `BaseScene.time`.
    time: f32,
    /// `this._wave` — always present here (the JS allows a renderer-less
    /// instance; the Rust contract always has a device).
    sim: WaveSim,
    /// `this.impacts`.
    impacts: [Impact; IMPACT_COUNT],
    /// `this._impactCursor`.
    impact_cursor: usize,
    /// `this._dropQueue`.
    drop_queue: Vec<QueuedDrop>,
    /// `this._seeded`.
    seeded: bool,
    /// `this._nextAmbient`.
    next_ambient: f32,
    /// `this._lastBass`.
    last_bass: f32,
    /// `this._motionBass`.
    motion_bass: f32,
    /// `this._lastBassDrop`.
    last_bass_drop: f32,
    /// `this._pulse`.
    pulse: f32,
    /// `this._phrase`.
    phrase: DropColorState,
    /// `this._phraseHue`.
    phrase_hue: f32,
    /// `this._phraseColors` (linear).
    colors: [[f32; 3]; 3],
    /// Frame counter for the deterministic hue pick.
    frame: u32,
    /// `speedScale`.
    speed_scale: f32,
    /// `distortionScale`.
    distortion_scale: f32,
    /// `radiusScale`.
    radius_scale: f32,
    /// `brightnessScale`.
    brightness_scale: f32,
    /// Render-target size.
    size: (u32, u32),
    gpu: Option<WaterRippleGpu>,
}

impl Default for WaterRipple {
    fn default() -> Self {
        Self::new()
    }
}

impl WaterRipple {
    /// Create the scene (`WaterRipple.js:128-196` defaults).
    pub fn new() -> Self {
        Self::with_wave_res(WATER_RES)
    }

    /// Create the scene with a smaller height field.
    ///
    /// `WAVE_RES = 256` is what the JS renders; the unit tests drive hundreds of
    /// frames and a 256² step costs 65k texels each, so they use this to keep
    /// the propagation stepping real but cheap. Nothing on the production path
    /// calls it.
    pub fn with_wave_res(res: u32) -> Self {
        let base: [[f32; 3]; 3] = BASE_COLORS_HEX.map(srgb_hex_to_linear);
        Self {
            audio: P10Audio::new(),
            time: 0.0,
            sim: WaveSim::new(res.max(1)),
            impacts: [Impact::INITIAL; IMPACT_COUNT],
            impact_cursor: 0,
            drop_queue: Vec::new(),
            seeded: false,
            next_ambient: 0.9,
            last_bass: 0.0,
            motion_bass: 0.0,
            last_bass_drop: -10.0,
            pulse: 0.0,
            phrase: DropColorState::new(),
            phrase_hue: 0.0,
            colors: base.map(|color| equalized_hue(color, 0.0)),
            frame: 0,
            speed_scale: 1.0,
            distortion_scale: 1.0,
            radius_scale: 1.0,
            brightness_scale: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:425-430`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(WATER_RIPPLE_PROPS, props) {
            match key {
                "speed" => self.speed_scale = value,
                "distortion" => self.distortion_scale = value,
                "radius" => self.radius_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// The destination alpha the scene clears to ([`CLEAR_ALPHA`]).
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// The water field (for tests and golden frames).
    pub fn wave(&self) -> &WaveSim {
        &self.sim
    }

    /// The impact rings as last uploaded.
    pub fn impacts(&self) -> [Impact; IMPACT_COUNT] {
        self.impacts
    }

    /// The phrase colour state.
    pub fn phrase(&self) -> DropColorState {
        self.phrase
    }

    /// The eased displayed hue (radians).
    pub fn phrase_hue(&self) -> f32 {
        self.phrase_hue
    }

    /// `_spawnImpact(x, y, strength)` (`:237-241`).
    fn spawn_impact(&mut self, x: f32, y: f32, strength: f32) {
        let impact = &mut self.impacts[self.impact_cursor];
        impact.x = x;
        impact.y = y;
        impact.age = 0.0;
        impact.weight = clamp01(strength);
        self.impact_cursor = (self.impact_cursor + 1) % self.impacts.len();
    }

    /// `_queueDrop(x, y, strength, radius, delay)`.
    fn queue_drop(&mut self, x: f32, y: f32, strength: f32, radius: f32, delay: f32) {
        self.drop_queue.push(QueuedDrop {
            at: self.time + delay,
            x,
            y,
            strength,
            radius,
        });
    }

    /// `_processDropQueue` (`:247-260`).
    fn process_drop_queue(&mut self) {
        let mut index = 0;
        while index < self.drop_queue.len() {
            if self.drop_queue[index].at > self.time {
                index += 1;
                continue;
            }
            let drop = self.drop_queue.remove(index);
            self.sim.drop(drop.x, drop.y, drop.strength, drop.radius);
            self.spawn_impact(drop.x, drop.y, (drop.strength * 6.0).min(1.0));
        }
    }

    /// `_addBassRipple(bass, jump, pulse, beat)` (`:262-292`).
    fn add_bass_ripple(&mut self, jump: f32, pulse: f32, beat: f32) {
        if self.time - self.last_bass_drop < 0.16 || (jump <= 0.02 && beat <= 0.08) {
            return;
        }

        let level = self.motion_bass.max(beat * 0.45);
        let cursor = self.impact_cursor;
        let angle = cursor as f32 * 2.399_963 + self.time * 0.11;
        let base_radius = 0.08 + ((cursor as f32 * 0.17) % 0.3);
        let count = if level > 0.65 {
            4
        } else if level > 0.3 {
            3
        } else {
            2
        };
        let drive = water_ripple_drive(self.motion_bass, pulse);

        for i in 0..count {
            let along = if count == 1 {
                0.0
            } else {
                i as f32 / (count - 1) as f32 - 0.5
            };
            let path_angle = angle + along * (0.22 + level * 0.18);
            let radius = base_radius + (i as f32 * 1.7 + cursor as f32).sin() * 0.018;
            let x = 0.5 + path_angle.cos() * radius;
            let y = 0.5 + path_angle.sin() * radius;
            let falloff = 1.0 - along.abs() * 0.55;
            let strength = (0.2 + level * 0.45 + pulse * 0.08) * falloff;
            self.queue_drop(
                x,
                y,
                drive.drop_strength * (0.7 + strength),
                0.022 + level * 0.018 + i as f32 * 0.002,
                i as f32 * 0.042,
            );
        }

        self.last_bass_drop = self.time;
    }

    /// `_addAmbientRipple` (`:294-303`).
    fn add_ambient_ripple(&mut self) {
        let angle = self.time * 0.41;
        self.sim.drop(
            0.5 + angle.cos() * 0.2,
            0.5 + (angle * 1.31).sin() * 0.2,
            0.008,
            0.065,
        );
    }

    /// The band values the update reads (`WaterRipple.js:308-311`).
    fn bands(&self) -> (f32, f32) {
        if self.audio.has_audio() {
            (
                clamp01(self.audio.band(-1, 1, 0, 0.0)),
                self.audio.beat(0.0),
            )
        } else {
            (0.0, 0.0)
        }
    }

    /// `this._applyPhraseColor()` (`:198-204`).
    fn apply_phrase_colour(&mut self) {
        let base: [[f32; 3]; 3] = BASE_COLORS_HEX.map(srgb_hex_to_linear);
        for (index, color) in base.iter().enumerate() {
            self.colors[index] = equalized_hue(*color, self.phrase_hue);
        }
    }
}

impl Point10Scene for WaterRipple {
    fn name(&self) -> &'static str {
        "Point10 Water Ripple"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-water-ripple-shader"),
            source: wgpu::ShaderSource::Wgsl(water_ripple_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-water-ripple-layout"),
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

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-water-ripple-uniforms"),
            size: std::mem::size_of::<WaterRippleUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let water_texture = create_wave_texture(&device, self.sim.res());
        let water_view = water_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = wave_sampler(&device);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-water-ripple-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&water_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-water-ripple-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = ctx.fullscreen_pipeline(
            "point10-water-ripple",
            &shader,
            &pipeline_layout,
            // `transparent: true` -> three's normal blending, with the alpha
            // channel accumulating coverage.
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        self.sim.upload(&queue, &water_texture);
        self.gpu = Some(WaterRippleGpu {
            queue,
            pipeline,
            bind_group,
            uniform_buffer,
            water_texture,
        });
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        // The adapter's frame clock caps a frame at 0.1 s.
        let dt = frame_dt(dt);
        self.time += dt;
        self.frame = self.frame.wrapping_add(1);
        self.audio.update(dt, audio);

        let (bass, beat) = self.bands();

        self.motion_bass += (bass - self.motion_bass) * (dt * 7.0).min(1.0);
        let jump = (bass - self.last_bass).max(0.0);
        let attack = clamp01(jump * 8.0);
        self.pulse = (attack * 0.65).max(self.pulse * (-dt * 3.5).exp());

        let rand = crate::point10::shared::random::seeded_rand(self.frame as i32, 41);
        drop_color_pulse(&mut self.phrase, bass, dt, rand);
        self.phrase_hue = ease_phrase_hue(self.phrase_hue, self.phrase.hue, dt, 0.55);
        self.apply_phrase_colour();

        for impact in &mut self.impacts {
            impact.age += dt;
            if impact.age > 5.0 {
                impact.weight = 0.0;
            }
        }

        if !self.seeded {
            self.sim.drop(0.5, 0.5, 0.04, 0.12);
            self.spawn_impact(0.5, 0.5, 0.24);
            self.seeded = true;
        }
        self.add_bass_ripple(jump, self.pulse, beat);
        self.process_drop_queue();
        if self.time >= self.next_ambient {
            self.add_ambient_ripple();
            self.next_ambient = self.time + 1.8;
        }
        self.sim.update(dt);

        self.last_bass = bass;

        if let Some(gpu) = self.gpu.as_ref() {
            self.sim.upload(&gpu.queue, &gpu.water_texture);
        }
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-water-ripple"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // `setClearColor(0x000000, 0)` — the circle is the artwork.
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
        scene_props_with_levers(WATER_RIPPLE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10WaterRipple.js`: `meta: { category: "2d-scenes" }`.
        SceneMeta::for_scene("Point10 Water Ripple", "2d-scenes")
    }
}

impl WaterRipple {
    /// Build and upload the uniform block from the current state.
    fn uniforms(&self) -> WaterRippleUniforms {
        let drive = water_ripple_drive(self.motion_bass, self.pulse);
        WaterRippleUniforms {
            resolution: [self.size.0 as f32, self.size.1 as f32],
            time: self.time,
            bass: self.motion_bass,
            pulse: self.pulse,
            distortion: drive.distortion * self.distortion_scale,
            speed: drive.speed * self.speed_scale,
            radius: drive.radius * self.radius_scale,
            brightness: drive.brightness * self.brightness_scale,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
            impacts: self.impacts.map(Impact::as_vec4),
            color_a: [self.colors[0][0], self.colors[0][1], self.colors[0][2], 1.0],
            color_b: [self.colors[1][0], self.colors[1][1], self.colors[1][2], 1.0],
            color_c: [self.colors[2][0], self.colors[2][1], self.colors[2][2], 1.0],
        }
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue
            .write_buffer(&gpu.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms()));
    }
}

/// The fullscreen shader with the shared vertex entry in front.
fn water_ripple_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{WATER_RIPPLE_WGSL}")
}

const WATER_RIPPLE_WGSL: &str = r#"
struct WaterRippleUniforms {
  resolution: vec2<f32>,
  time: f32,
  bass: f32,
  pulse: f32,
  distortion: f32,
  speed: f32,
  radius: f32,
  brightness: f32,
  _pad0: f32,
  _pad1: f32,
  _pad2: f32,
  impacts: array<vec4<f32>, 8>,
  color_a: vec4<f32>,
  color_b: vec4<f32>,
  color_c: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: WaterRippleUniforms;
@group(0) @binding(1) var water: texture_2d<f32>;
@group(0) @binding(2) var water_sampler: sampler;

fn impact_ripple(uv: vec2<f32>, impact: vec4<f32>) -> f32 {
  let age = impact.z;
  let delta = uv - impact.xy;
  let distance_to_drop = length(delta);
  let front = age * 0.28;
  let outer_distance = (distance_to_drop - front) / 0.014;
  let inner_distance = (distance_to_drop - front * 0.7) / 0.024;
  let ring = exp(-outer_distance * outer_distance)
    + exp(-inner_distance * inner_distance) * 0.28;
  let bead = exp(-distance_to_drop * distance_to_drop / 0.00008) * exp(-age * 10.0) * 0.2;
  return (ring * 0.55 + bead) * exp(-age * 0.8) * impact.w;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let r = u.resolution;
  // GL's gl_FragCoord is bottom-left; the fragment position is top-left. The
  // PlaneGeometry `vUv` the JS reads is v = 0 at the bottom, so this is also
  // the JS `vUv`.
  let fragment = vec2<f32>(position.x, r.y - position.y);
  let water_uv = vec2<f32>(fragment.x / r.x, fragment.y / r.y);
  let screen_p = (fragment * 2.0 - r) / r.y;

  let water_height = textureSample(water, water_sampler, water_uv).r * (2.4 + u.bass * 1.6);
  var water_influence = clamp(water_height, -0.65, 0.65);

  var impact_influence = 0.0;
  for (var i = 0u; i < 8u; i = i + 1u) {
    impact_influence = impact_influence + impact_ripple(water_uv, u.impacts[i]);
  }
  water_influence = clamp(water_influence + impact_influence * 0.025, -0.8, 0.8);

  let audio_influence = u.bass * 0.4 + u.pulse * 0.2;
  let circle_radius = u.radius + audio_influence * 0.08 + abs(water_influence) * 0.1;
  let in_circle = 1.0 - smoothstep(
    circle_radius - 0.1,
    circle_radius + 0.1,
    length(screen_p),
  );

  var p = screen_p * 1.1;
  let angle = length(p) * 4.0 + audio_influence * 2.0 + water_influence * 0.25;
  // p *= mat2(cos(angle), -sin(angle), sin(angle), cos(angle)); column-major.
  let swirl_cos = cos(angle);
  let swirl_sin = sin(angle);
  p = vec2<f32>(
    swirl_cos * p.x + swirl_sin * p.y,
    -swirl_sin * p.x + swirl_cos * p.y,
  );

  let l = length(p) - 0.7 + water_influence * 0.5 + audio_influence * 0.12;
  let t = u.time * u.speed + water_influence * 2.0 + audio_influence * 1.5;
  let enhanced_y = p.y + water_influence * 0.3 + audio_influence * 0.15;
  let edge = max(max(l / 0.1, -l), 0.0001);
  let wave = l + enhanced_y * max(1.0, -l / 0.1) + t;
  let pattern1 = 0.5 + 0.5 * tanh(0.1 / edge - sin(wave));
  let pattern2 = 0.5 + 0.5 * tanh(0.1 / edge - sin(wave + 1.0));
  let pattern3 = 0.5 + 0.5 * tanh(0.1 / edge - sin(wave + 2.0));
  let body = 0.24 + 0.76 * smoothstep(-0.65, 0.0, l);
  let intensity = (0.78 + water_influence * 0.25 + audio_influence * 0.22)
    * (0.9 + u.brightness * 0.12);

  let centered = water_uv - vec2<f32>(0.5);
  let polar = atan2(centered.y, centered.x);
  let gradient_sweep = 0.5 + 0.5 * sin(
    polar * 2.0 - u.time * 0.22 + length(centered) * 4.0 + water_influence * 2.4,
  );
  let gradient_drift = 0.5 + 0.5 * sin(
    (water_uv.x + water_uv.y) * 5.0 + u.time * 0.16 - water_influence * 1.5,
  );
  var gradient_color = mix(u.color_a.rgb, u.color_b.rgb, gradient_sweep);
  gradient_color = mix(gradient_color, u.color_c.rgb, gradient_drift * 0.28);

  let pattern_color = vec3<f32>(
    pattern1 * gradient_color.r,
    pattern2 * gradient_color.g,
    pattern3 * gradient_color.b,
  );
  var color = pattern_color * body * intensity;
  color = color + gradient_color * body * (0.08 + gradient_sweep * 0.12);
  let outer = smoothstep(0.02, 0.28, l);
  color = mix(color, gradient_color * 0.42, outer * 0.72);
  let rim = exp(-l * l / 0.0024);
  let rim_color = mix(vec3<f32>(0.78, 0.96, 1.0), gradient_color, 0.2);
  color = color + rim * rim_color * (1.05 + audio_influence * 0.3);
  color = color + impact_influence * gradient_color * (0.018 + u.bass * 0.025);

  // The circular pattern is the artwork; outside it the alpha fades to zero
  // (inCircle is already a soft edge) so the scene composites transparently.
  return vec4<f32>(color, in_circle);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-water-ripple.test.js plus the batch-3 state
    // machine checks.

    #[test]
    fn drive_uses_the_seeded_silence_defaults() {
        let drive = water_ripple_drive(0.0, 0.0);
        assert_eq!(drive.distortion, 0.55);
        assert_eq!(drive.speed, 1.3);
        assert_eq!(drive.radius, 0.9);
        assert_eq!(drive.brightness, 0.68);
        assert_eq!(drive.drop_strength, 0.012);
    }

    #[test]
    fn drive_lifts_the_pattern_with_bass_and_pulse() {
        let drive = water_ripple_drive(0.5, 0.25);
        assert!((drive.distortion - (0.55 + 0.5 * 1.1 + 0.25 * 0.55)).abs() < 1e-6);
        assert!((drive.brightness - (0.68 + 0.5 * 0.5 + 0.25 * 0.2)).abs() < 1e-6);
    }

    #[test]
    fn drive_clamps_non_finite_and_out_of_range_input() {
        assert_eq!(water_ripple_drive(-1.0, -1.0), water_ripple_drive(0.0, 0.0));
        assert_eq!(
            water_ripple_drive(f32::NAN, f32::NAN),
            water_ripple_drive(0.0, 0.0)
        );
        assert_eq!(water_ripple_drive(10.0, 10.0), water_ripple_drive(1.0, 1.0));
    }

    #[test]
    fn clears_to_a_fully_transparent_background() {
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(WaterRipple::new().clear_alpha(), 0.0);
    }

    #[test]
    fn shader_keeps_the_ported_maths_and_alpha() {
        let program = water_ripple_wgsl();
        for needle in [
            "impact_ripple(water_uv, u.impacts[i])",
            "exp(-outer_distance * outer_distance)",
            "exp(-age * 10.0) * 0.2",
            "1.0 - smoothstep(",
            "tanh(0.1 / edge - sin(wave))",
            "0.24 + 0.76 * smoothstep(-0.65, 0.0, l)",
            "polar * 2.0 - u.time * 0.22",
            "exp(-l * l / 0.0024)",
            "return vec4<f32>(color, in_circle);",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }
        assert!(
            !program.contains("0.02, 0.02, 0.05"),
            "the old near-black backdrop must not come back"
        );
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = water_ripple_wgsl();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("WATER_RIPPLE_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("WATER_RIPPLE_WGSL must validate");
    }

    #[test]
    fn uniforms_are_224_bytes() {
        assert_eq!(std::mem::size_of::<WaterRippleUniforms>(), 224);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = WaterRipple::new();
        scene.apply_props(|key| match key {
            "speed" => Some(99.0),
            "distortion" => Some(-1.0),
            "radius" => Some(9.0),
            "brightness" => Some(0.5),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.distortion_scale, 0.0);
        assert_eq!(scene.radius_scale, 2.0);
        assert_eq!(scene.brightness_scale, 0.5);
    }

    #[test]
    fn the_first_frame_seeds_the_centre_and_steps_the_field() {
        let mut scene = WaterRipple::new();
        let bridge = AudioBridge::new();
        scene.update(0.0, &bridge);

        // `_seeded` the centre drop; the impact cursor advanced past it.
        assert_eq!(scene.impact_cursor, 1);
        assert_eq!(scene.impacts[0].x, 0.5);
        assert!((scene.impacts[0].weight - 0.24).abs() < 1e-6);
        // The step ran once, so the field is no longer at rest.
        assert!(scene.wave().data().iter().any(|value| *value != 0.0));
        assert_eq!(scene.time, 0.0);
    }

    #[test]
    fn bass_attacks_queue_and_fire_drops() {
        let mut scene = WaterRipple::with_wave_res(32);
        let mut bridge = AudioBridge::new();
        scene.update(0.0, &bridge);

        let mut payload = BandPayload::silent(44_100.0);
        let mut bass = 0.0_f32;
        // A rising bass ramps `_motionBass`; once the attack gate opens the
        // scheduler queues two drops per burst.
        for _ in 0..40 {
            bass = (bass + 0.05).min(0.8);
            payload.lanes[3] = bass; // damping lane 1 of band 0
            bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
            scene.update(1.0 / 60.0, &bridge);
        }

        let live = scene.impacts().iter().filter(|i| i.weight > 0.0).count();
        assert!(live > 1, "expected bass ripples, got {live} live impacts");
        // The impact rings age with the frame clock.
        assert!(scene.impacts().iter().any(|impact| impact.age > 0.0));
        assert!(scene.wave().data().iter().any(|value| *value != 0.0));
    }

    #[test]
    fn impacts_fade_out_after_five_seconds() {
        let mut scene = WaterRipple::with_wave_res(16);
        let bridge = AudioBridge::new();
        scene.spawn_impact(0.25, 0.75, 1.0);
        for _ in 0..400 {
            scene.update(0.1, &bridge);
        }
        // Everything spawned at the seeded frame is older than 5 s.
        assert!(scene.impacts().iter().all(|impact| impact.weight == 0.0));
    }

    #[test]
    fn phrase_hue_follows_the_drop_detector() {
        let mut scene = WaterRipple::with_wave_res(16);
        let mut bridge = AudioBridge::new();
        let mut payload = BandPayload::silent(44_100.0);

        // 10 s at a resting bass, then a sharp attack and a shallow crest.
        for _ in 0..620 {
            payload.lanes[3] = 0.3;
            bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
            scene.update(1.0 / 60.0, &bridge);
        }
        assert_eq!(scene.phrase().hue, 0.0);

        payload.lanes[3] = 0.9;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);
        payload.lanes[3] = 0.87;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);

        let target = scene.phrase().hue;
        assert!(target != 0.0, "the crest must flip the phrase hue");
        // The displayed hue eases toward the target instead of snapping.
        assert!(scene.phrase_hue() != 0.0);
        assert!((scene.phrase_hue() - target).abs() > 1e-6);
    }

    #[test]
    fn time_is_capped_at_a_tenth_of_a_second() {
        let mut scene = WaterRipple::new();
        let bridge = AudioBridge::new();
        scene.update(10.0, &bridge);
        assert!((scene.time - 0.1).abs() < 1e-6);
        scene.update(f32::NAN, &bridge);
        assert!((scene.time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = WaterRipple::new();
        assert_eq!(scene.meta().name, "Point10 Water Ripple");
        assert_eq!(scene.meta().category, "2d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 4..],
            &["speed", "distortion", "radius", "brightness"]
        );
        assert_eq!(names[0], "gain");
    }
}
