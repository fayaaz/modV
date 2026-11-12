//! Mycelium Strand — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/Mycelium.js`.
//!
//! A hyphal colony spreads from the screen centre across a dark substrate. The
//! fragment shader is E. T. Carter's `shader.gallery/mycelium` (MIT) wrapped in
//! an audio-reactive shell: growth speed ← bass, wander ← bass + beat, twigs ←
//! mid, runner density ← high, bioluminescence ← mid/high + beat, and the kick
//! gates a narrow light band sweeping out along the strands.
//!
//! # Ported exactly
//!
//! * the whole fragment shader (`Mycelium.js:25-347`): `hash21`, `vnoise`,
//!   `vnoiseX` (the angular-wrap noise across the `atan` branch cut), the
//!   `M2 = (0.80, 0.60, -0.60, 0.80)` fbm, the five-family `colony` walk
//!   (`frontOf`/`offsetOf`/`lifeOf`/`startOf`, the seam-free integer cell count,
//!   the perpendicular distance `|F| / gu`, tip glow and halo), the three-segment
//!   `twigs` layer with its cell-root snapping, the ground mottle/grit/fuzz,
//!   the two warped generations and the pulse band;
//! * the audio mapping (`_audioUniforms`, `:440-456`) — every floor and cap;
//! * the integrated growth clock (`:485-490`): `growth += dt * speed`,
//!   `u_t = growth / 32 + 0.22`, `u_tp = growth`, so a band change cannot make
//!   the clock jump;
//! * the phrase colour: `dropColorPulse` on the bass band, the eased hue
//!   (`min(1, dt * 0.55)`) and `hueRotate` of the four base colours **and** the
//!   fog colour (numeric literals — already in three's linear working space);
//! * the bloom chain: `UnrealBloomPass` at `(0.5, 0.7, 0.25)` with the
//!   per-frame strength `min(0.5 + glow * 0.35 + beat * 0.25, 1.4)`.
//!
//! # `OutputPass`
//!
//! `Mycelium.js:421-423` deliberately omits three's `OutputPass`: the shader is
//! display-referred (the reference's direct-to-screen look) and an `OutputPass`
//! would treat its dark values as linear and gamma-lift the frame to a washed
//! out white. The Rust composer always ends with an output pass, so the scene
//! calls [`Composer::set_encode_srgb(false)`](crate::point10::postfx::Composer::set_encode_srgb)
//! to keep the display-referred values, exactly like the JS.
//!
//! # Deviations
//!
//! * `RawShaderMaterial` → a WGSL pair with the shared fullscreen triangle. The
//!   vertex shader only forwards clip space, so nothing is lost.
//! * `u_pixelRatio` comes from `renderer.getPixelRatio()`; the offscreen
//!   adapter canvas is always 1 device pixel per pixel, so the port uses 1.0.
//! * GLSL's `mod` (floored) is GLSL-only for negative inputs; WGSL's `%` is a
//!   truncated remainder, so [`MYCELIUM_WGSL`] carries `my_mod` instead. The
//!   angular wrap and the `vnoiseX` lattice need the floored version.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::shared::phrase_color::{
    DropColorState, drop_color_pulse, ease_phrase_hue, hue_rotate,
};
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    frame_dt, read_scene_props, scene_props_with_levers,
};

/// `Mycelium.js:414`: `new UnrealBloomPass(size, 0.5, 0.7, 0.25)`.
pub const BLOOM_STRENGTH: f32 = 0.5;
/// `Mycelium.js:414` radius.
pub const BLOOM_RADIUS: f32 = 0.7;
/// `Mycelium.js:414` threshold.
pub const BLOOM_THRESHOLD: f32 = 0.25;

/// The base colours (`Mycelium.js:361-366`) — linear working space.
pub const BASE_COLORS: [[f32; 3]; 4] = [
    [0.231, 0.51, 0.965],
    [0.659, 0.333, 0.969],
    [0.133, 0.827, 0.933],
    [0.957, 0.247, 0.369],
];

/// `this._fogBase = new THREE.Color(0.02, 0.028, 0.05)`.
pub const FOG_BASE: [f32; 3] = [0.02, 0.028, 0.05];

/// `MYCELIUM_PROPS` (`sceneProps.js:104-108`).
const MYCELIUM_PROPS: &[SceneProp] = &[
    SceneProp::float("growthSpeed", "Growth Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("glow", "Glow", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("fogDensity", "Fog Density", 1.0, 0.0, 2.0, 0.01),
];

/// Band-driven uniforms (`_audioUniforms`, before the prop scales).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MyceliumAudio {
    /// Growth speed in waves per second (`this.speed`).
    pub speed: f32,
    /// `u_glow`.
    pub glow: f32,
    /// `u_beat`.
    pub beat: f32,
    /// `u_wander`.
    pub wander: f32,
    /// `u_twigs`.
    pub twigs: f32,
    /// `u_scale`.
    pub scale: f32,
    /// `u_fogDensity`.
    pub fog_density: f32,
    /// `bloomPass.strength`.
    pub bloom_strength: f32,
}

/// `Mycelium.js:440-456` with the three prop scales applied.
pub fn mycelium_audio(
    b0: f32,
    b1: f32,
    b2: f32,
    beat: f32,
    speed_scale: f32,
    glow_scale: f32,
    fog_scale: f32,
) -> MyceliumAudio {
    let clamp01 = |value: f32| {
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let (b0, b1, b2, beat) = (clamp01(b0), clamp01(b1), clamp01(b2), clamp01(beat));

    let speed = (0.25 + b0 * 3.0 + beat * 0.5).min(3.0) * speed_scale;
    let glow = (0.15 + (b1 + b2) * 2.0 + beat * 0.5).min(2.0) * glow_scale;
    let beat_value = beat;
    let wander = (0.6 + b0 * 1.2 + beat * 0.4).min(2.0);
    let twigs = (0.35 + b1 * 1.2 + b2 * 0.8).min(1.0);
    let scale = (0.7 + b2 * 1.2).min(2.0);
    let fog_density = (0.25 + glow * 0.22 + beat * 0.15).min(0.7) * fog_scale;
    let bloom_strength = (0.5 + glow * 0.35 + beat * 0.25).min(1.4);

    MyceliumAudio {
        speed,
        glow,
        beat: beat_value,
        wander,
        twigs,
        scale,
        fog_density,
        bloom_strength,
    }
}

/// Uniform block, matching [`MYCELIUM_WGSL`] (128 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct MyceliumUniforms {
    resolution: [f32; 2],
    t: f32,
    tp: f32,
    scale: f32,
    twigs: f32,
    glow: f32,
    wander: f32,
    beat: f32,
    pixel_ratio: f32,
    fog_density: f32,
    _pad: f32,
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    c3: [f32; 4],
    fog_color: [f32; 4],
}

/// GPU resources built in `init`.
struct MyceliumGpu {
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    composer: Composer,
}

/// Mycelium scene.
pub struct Mycelium {
    audio: P10Audio,
    /// `BaseScene.time`.
    time: f32,
    /// `this.growth` — the integrated wave clock.
    growth: f32,
    /// `this.speed`.
    speed: f32,
    /// `this.speedScale` (`growthSpeed`).
    speed_scale: f32,
    /// `this.glowScale` (`glow`).
    glow_scale: f32,
    /// `this.fogScale` (`fogDensity`).
    fog_scale: f32,
    /// `this._phrase`.
    phrase: DropColorState,
    /// `this._phraseHue`.
    phrase_hue: f32,
    /// `this._baseColors`.
    base_colors: [[f32; 3]; 4],
    /// `this._fogBase`.
    fog_base: [f32; 3],
    /// Rotated colours as last uploaded (`u_c0..u_c3`).
    colors: [[f32; 3]; 4],
    /// Rotated fog colour.
    fog_color: [f32; 3],
    /// The band-driven uniforms as last applied.
    drive: MyceliumAudio,
    /// Frame counter for the deterministic hue pick.
    frame: u32,
    /// `u_t` (wave clock).
    wave_clock: f32,
    /// `u_tp` (pulse clock).
    pulse_clock: f32,
    size: (u32, u32),
    gpu: Option<MyceliumGpu>,
}

impl Default for Mycelium {
    fn default() -> Self {
        Self::new()
    }
}

impl Mycelium {
    /// Create the scene (`Mycelium.js:350-425` defaults).
    pub fn new() -> Self {
        let mut colors = [[0.0_f32; 3]; 4];
        for (index, color) in BASE_COLORS.iter().enumerate() {
            colors[index] = hue_rotate(*color, 0.0);
        }
        Self {
            audio: P10Audio::new(),
            time: 0.0,
            growth: 0.0,
            speed: 0.25,
            speed_scale: 1.0,
            glow_scale: 1.0,
            fog_scale: 1.0,
            phrase: DropColorState::new(),
            phrase_hue: 0.0,
            base_colors: BASE_COLORS,
            fog_base: FOG_BASE,
            colors,
            fog_color: hue_rotate(FOG_BASE, 0.0),
            drive: mycelium_audio(0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0),
            frame: 0,
            wave_clock: 0.22,
            pulse_clock: 0.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:401-405`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(MYCELIUM_PROPS, props) {
            match key {
                "growthSpeed" => self.speed_scale = value,
                "glow" => self.glow_scale = value,
                "fogDensity" => self.fog_scale = value,
                _ => {}
            }
        }
    }

    /// The integrated growth clock (`this.growth`).
    pub fn growth(&self) -> f32 {
        self.growth
    }

    /// The band-driven uniforms as last applied.
    pub fn drive(&self) -> MyceliumAudio {
        self.drive
    }

    /// The rotated palette as last uploaded.
    pub fn colors(&self) -> [[f32; 3]; 4] {
        self.colors
    }

    /// The rotated fog colour.
    pub fn fog_color(&self) -> [f32; 3] {
        self.fog_color
    }

    /// Uniforms from the current state.
    fn uniforms(&self) -> MyceliumUniforms {
        MyceliumUniforms {
            resolution: [self.size.0 as f32, self.size.1 as f32],
            t: self.wave_clock,
            tp: self.pulse_clock,
            scale: self.drive.scale,
            twigs: self.drive.twigs,
            glow: self.drive.glow,
            wander: self.drive.wander,
            beat: self.drive.beat,
            pixel_ratio: 1.0,
            fog_density: self.drive.fog_density,
            _pad: 0.0,
            c0: [self.colors[0][0], self.colors[0][1], self.colors[0][2], 1.0],
            c1: [self.colors[1][0], self.colors[1][1], self.colors[1][2], 1.0],
            c2: [self.colors[2][0], self.colors[2][1], self.colors[2][2], 1.0],
            c3: [self.colors[3][0], self.colors[3][1], self.colors[3][2], 1.0],
            fog_color: [self.fog_color[0], self.fog_color[1], self.fog_color[2], 1.0],
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

impl Point10Scene for Mycelium {
    fn name(&self) -> &'static str {
        "Point10 Mycelium"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-mycelium-shader"),
            source: wgpu::ShaderSource::Wgsl(mycelium_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-mycelium-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-mycelium-uniforms"),
            size: std::mem::size_of::<MyceliumUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-mycelium-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-mycelium-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // `RawShaderMaterial` with `side: DoubleSide` and no blending: a single
        // fullscreen triangle writing alpha 1.
        // Renders into the composer's HDR scene target, not the destination.
        let pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-mycelium",
            &shader,
            &pipeline_layout,
            HDR_FORMAT,
            None,
        );

        // `EffectComposer`: RenderPass -> UnrealBloomPass, and deliberately no
        // OutputPass (the scene is display-referred).
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.enable_bloom(BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD);
        composer.output_mut().set_tone_mapping(ToneMapping::None);
        composer.set_encode_srgb(false);

        self.gpu = Some(MyceliumGpu {
            queue,
            pipeline,
            bind_group,
            uniform_buffer,
            composer,
        });
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.resize(self.size.0, self.size.1);
        }
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.time += dt;
        self.frame = self.frame.wrapping_add(1);
        self.audio.update(dt, audio);

        let b0 = self.audio.band(-1, 1, 0, 0.0);
        let b1 = self.audio.band(-1, 1, 1, 0.0);
        let b2 = self.audio.band(-1, 1, 2, 0.0);
        let beat = self.audio.beat(0.0);

        self.drive = mycelium_audio(
            b0,
            b1,
            b2,
            beat,
            self.speed_scale,
            self.glow_scale,
            self.fog_scale,
        );
        self.speed = self.drive.speed;

        let rand = seeded_rand(self.frame as i32, 17);
        drop_color_pulse(&mut self.phrase, b0, dt, rand);
        self.phrase_hue = ease_phrase_hue(self.phrase_hue, self.phrase.hue, dt, 0.55);
        for index in 0..4 {
            self.colors[index] = hue_rotate(self.base_colors[index], self.phrase_hue);
        }
        self.fog_color = hue_rotate(self.fog_base, self.phrase_hue);

        // The integrated clock keeps `u_t`/`u_tp` monotonic as the band moves.
        self.growth += dt * self.speed;
        self.wave_clock = self.growth / 32.0 + 0.22;
        self.pulse_clock = self.growth;

        self.upload_uniforms();

        if let Some(gpu) = self.gpu.as_mut() {
            if let Some(bloom) = gpu.composer.bloom_mut() {
                bloom.set_strength(self.drive.bloom_strength);
            }
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-mycelium"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gpu.composer.scene_view(),
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

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(MYCELIUM_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10Mycelium.js`: `meta: { category: "2d-scenes" }`.
        SceneMeta::for_scene("Point10 Mycelium", "2d-scenes")
    }
}

/// The fragment shader with the shared vertex entry in front.
fn mycelium_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{P9_COMMON_WGSL}\n{MYCELIUM_WGSL}")
}

const MYCELIUM_WGSL: &str = r#"
struct MyceliumUniforms {
  resolution: vec2<f32>,
  t: f32,
  tp: f32,
  scale: f32,
  twigs: f32,
  glow: f32,
  wander: f32,
  beat: f32,
  pixel_ratio: f32,
  fog_density: f32,
  _pad: f32,
  c0: vec4<f32>,
  c1: vec4<f32>,
  c2: vec4<f32>,
  c3: vec4<f32>,
  fog_color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: MyceliumUniforms;

const TAU: f32 = 6.28318530718;
const ORIGIN: vec2<f32> = vec2<f32>(0.0, 0.0);
const RMIN: f32 = 0.05;
const RMAX: f32 = 1.25;

// GLSL `mod` (floored); WGSL's `%` truncates toward zero.
fn my_mod(x: f32, y: f32) -> f32 {
  return x - y * floor(x / y);
}

fn hash21(p_in: vec2<f32>) -> f32 {
  var p = fract(p_in * vec2<f32>(234.34, 435.345));
  p = p + vec2<f32>(dot(p, p + vec2<f32>(34.23)));
  return fract(p.x * p.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u2 = f * f * (3.0 - 2.0 * f);
  let a = hash21(i);
  let b = hash21(i + vec2<f32>(1.0, 0.0));
  let c = hash21(i + vec2<f32>(0.0, 1.0));
  let d = hash21(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u2.x), mix(c, d, u2.x), u2.y);
}

// value noise whose x lattice wraps every px cells, so an angular coordinate
// can span the atan branch cut with matching tile edges
fn vnoise_x(p: vec2<f32>, px: f32) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u2 = f * f * (3.0 - 2.0 * f);
  let x0 = my_mod(i.x, px);
  let x1 = my_mod(i.x + 1.0, px);
  let a = hash21(vec2<f32>(x0, i.y));
  let b = hash21(vec2<f32>(x1, i.y));
  let c = hash21(vec2<f32>(x0, i.y + 1.0));
  let d = hash21(vec2<f32>(x1, i.y + 1.0));
  return mix(mix(a, b, u2.x), mix(c, d, u2.x), u2.y);
}

fn fbm(p_in: vec2<f32>) -> f32 {
  var p = p_in;
  var a = 0.5;
  var s = 0.0;
  for (var i = 0; i < 3; i = i + 1) {
    s = s + a * vnoise(p);
    // const mat2 M2 = mat2(0.80, 0.60, -0.60, 0.80); column-major.
    p = vec2<f32>(
      0.80 * p.x + -0.60 * p.y,
      0.60 * p.x + 0.80 * p.y,
    ) * 2.03 + vec2<f32>(11.7, 5.3);
    a = a * 0.5;
  }
  return s * 1.14;
}

fn dir_of(a: f32) -> vec2<f32> {
  return vec2<f32>(cos(a), sin(a));
}

// where the growth front of generation g sits, in radius, for the line n of
// family f: a ragged margin, each hypha a little ahead or behind its fellows
fn front_of(tg: f32, n: f32, f: f32, g: f32) -> f32 {
  let j = hash21(vec2<f32>(n * 1.7 + f * 13.1, g * 7.3)) - 0.5;
  return RMIN + (RMAX - RMIN + 0.45) * p9_smoothstep(0.0, 0.85, tg) + j * 0.35;
}

// per-line angular offset in spacing units: the bend that peels a child line
// off its parent near its start (half a spacing, decaying outward) plus a
// small fixed jitter that only takes hold once the bend has relaxed, so no
// line ever crosses its own index boundary
fn offset_of(r: f32, rs: f32, sgn: f32, jit: f32) -> f32 {
  let b = sgn * 0.49 * exp(-max(r - rs, 0.0) / 0.14);
  return b + jit * (1.0 - abs(b) / 0.49);
}

// radius where child line n of family f gives up: many hyphae simply stop
fn life_of(rs: f32, n: f32, f: f32, g: f32) -> f32 {
  let h = hash21(vec2<f32>(n * 3.1 + f * 7.7, g * 5.9 + 2.0));
  return rs + 0.25 + 2.4 * h * h;
}

// start radius of family f in generation g: where its lines branch off
fn start_of(f: f32, g: f32) -> f32 {
  if (f < 0.5) {
    return 0.0;
  }
  return RMIN + 0.08 + f * 0.18 + (hash21(vec2<f32>(g, f)) - 0.5) * 0.12;
}

struct Colony {
  core: f32,
  bright: f32,
  halo: f32,
  tip: f32,
};

// one generation of the colony at warped point pw. Accumulates core coverage
// of the nearest line, its brightness, the halo and the tip glow.
fn colony(pw: vec2<f32>, g: f32, tg: f32, s0: f32, px: f32) -> Colony {
  var out = Colony(0.0, 0.0, 0.0, 0.0);
  let d = pw - ORIGIN;
  let r = max(length(d), 1e-4);
  // wrap the angle so the radial tiling is periodic: an exact cell count means
  // no partial tile and no line cut along the atan branch (the seam line)
  let th = my_mod(atan2(d.y, d.x), TAU);
  let alpha = 1.0 - p9_smoothstep(1.0, 1.85, tg);
  if (alpha <= 0.0) {
    return out;
  }

  var s = s0;
  for (var f = 0; f < 5; f = f + 1) {
    let ff = f32(f);
    let rsf = start_of(ff, g);
    // integer cell count around the circle, then an exact spacing: the first
    // and last tiles keep full width and meet seamlessly
    let n_cells = max(1.0, floor(TAU / s + 0.5));
    let ss = TAU / n_cells;
    let u0 = th / ss;
    let n = floor(u0);
    let hn = hash21(vec2<f32>(n, ff + g * 3.0));
    let sgn = select(select(f32(-1.0), f32(1.0), hn < 0.5), f32(0.0), f == 0);
    let jit = (hash21(vec2<f32>(n * 1.3, ff * 2.1 + g)) - 0.5) * 0.30;
    let big_f = fract(u0 + offset_of(r, rsf, sgn, jit)) - 0.5;
    // perpendicular distance: |F| over the gradient magnitude of u
    let db = -sgn * 0.49 / 0.14 * exp(-max(r - rsf, 0.0) / 0.14);
    let gu = sqrt(1.0 / (ss * ss * r * r) + db * db);
    let dist = abs(big_f) / gu;
    // visibility: born at its start radius, alive up to the growth front or
    // until the hypha gives up on its own
    let front = min(front_of(tg, n, ff, g), life_of(rsf, n, ff, g));
    let born = p9_smoothstep(rsf - 0.02, rsf + 0.02, r);
    let alive = p9_smoothstep(front + 0.008, front - 0.008, r);
    let vis = born * alive * alpha;
    // width: leaders thickest, each family finer, tapering to the tip
    let w = 0.0021 * pow(0.80, ff) * (1.0 - 0.4 * p9_smoothstep(front - 0.5, front, r));
    let c = p9_smoothstep(w + px, w - px * 0.6, dist) * vis;
    // the body brightens toward its tip where the cytoplasm is busiest
    let age = exp(-(front - r) * 1.1);
    let bb = 0.55 + 0.45 * age;
    let is_core = step(out.core, c) * step(0.001, c);
    out.bright = mix(out.bright, bb, is_core);
    out.core = max(out.core, c);
    out.halo = out.halo + vis * exp(-dist * dist * 3500.0) * (0.3 + 0.7 * age) * pow(0.85, ff);
    // apex: swollen bright tip at the end of every living line
    let tipv = vec2<f32>(dist * 1.6, (r - front) * 0.9);
    let td = dot(tipv, tipv);
    out.tip = out.tip + alpha * born * exp(-td * 30000.0) * pow(0.9, ff);
    s = s * 0.5;
  }
  return out;
}

struct Twigs {
  min_d: f32,
  min_w: f32,
  min_a: f32,
  halo: f32,
  tip: f32,
};

// one twig segment; see the twig layer below
fn seg(
  q: vec2<f32>,
  a: vec2<f32>,
  b: vec2<f32>,
  s0: f32,
  len: f32,
  w: f32,
  reach: f32,
  alpha: f32,
  acc: ptr<function, Twigs>,
) {
  let frac = clamp((reach - s0) / len, 0.0, 1.0);
  if (frac <= 0.0) {
    return;
  }
  let ab = (b - a) * frac;
  let aq = q - a;
  let h = clamp(dot(aq, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
  let dd = length(aq - ab * h);
  let ww = w * (1.0 - 0.45 * (s0 + len * frac * h));
  if (dd < (*acc).min_d) {
    (*acc).min_d = dd;
    (*acc).min_w = ww;
    (*acc).min_a = alpha;
  }
  (*acc).halo = (*acc).halo + alpha * exp(-dd * dd * 500.0);
  if (frac < 1.0) {
    let tp = q - (a + ab);
    (*acc).tip = (*acc).tip + alpha * exp(-dot(tp, tp) * 2500.0);
  }
}

// lateral twigs of generation g: short branching polylines rooted on the
// finest runner line of their cell and growing outward behind the front
fn twigs(
  pw: vec2<f32>,
  g: f32,
  tg: f32,
  s0: f32,
  cell: f32,
  density: f32,
  acc: ptr<function, Twigs>,
) {
  let alpha = 1.0 - p9_smoothstep(1.0, 1.85, tg);
  if (alpha <= 0.0) {
    return;
  }
  let q = pw / cell;
  let ci = floor(q);
  for (var y = -1; y <= 1; y = y + 1) {
    for (var x = -1; x <= 1; x = x + 1) {
      let id = ci + vec2<f32>(f32(x), f32(y));
      let gs = id + vec2<f32>(g * 5.3, g * 2.1);
      let h0 = hash21(gs);
      let h1 = hash21(gs + 11.1);
      let h2 = hash21(gs + 23.7);
      let h3 = hash21(gs + 37.3);
      if (h3 > density) {
        continue;
      }
      // find the finest family born at this cell and its nearest line, then
      // put the root exactly on that line, bend and jitter included
      let pc = (id + 0.5) * cell;
      let d = pc - ORIGIN;
      let r = max(length(d) + (h0 - 0.5) * 0.6 * cell, 1e-4);
      let th = my_mod(atan2(d.y, d.x), TAU);
      var s = s0;
      var ff = 0.0;
      var rsf = start_of(0.0, g);
      for (var f = 1; f < 5; f = f + 1) {
        let rt = start_of(f32(f), g);
        if (r > rt + 0.05) {
          s = s * 0.5;
          ff = f32(f);
          rsf = rt;
        }
      }
      let n_cells = max(1.0, floor(TAU / s + 0.5));
      let ss = TAU / n_cells;
      let n = floor(th / ss);
      let hn = hash21(vec2<f32>(n, ff + g * 3.0));
      let sgn = select(select(f32(-1.0), f32(1.0), hn < 0.5), f32(0.0), ff < 0.5);
      let jit = (hash21(vec2<f32>(n * 1.3, ff * 2.1 + g)) - 0.5) * 0.30;
      let line_th = (n + 0.5 - offset_of(r, rsf, sgn, jit)) * ss;
      var root = ORIGIN + dir_of(line_th) * r;
      // keep the root within reach of the cell so nothing clips at borders
      root = clamp(root / cell, id - vec2<f32>(0.35), id + vec2<f32>(1.35));
      let front = min(front_of(tg, n, ff, g), life_of(rsf, n, ff, g));
      // grows once the front has passed its root; none past a dead end
      let reach = p9_smoothstep(0.0, 0.45, front - r) * 0.8;
      // outward lateral: off the runner at an acute angle, then wavering on
      let sb = select(f32(-1.0), f32(1.0), h1 < 0.5);
      let a0 = line_th + sb * (0.45 + 0.5 * h2);
      let a1 = a0 - sb * (0.2 + 0.5 * h0);
      let p1 = root + dir_of(a0) * 0.36;
      let p2 = p1 + dir_of(a1) * 0.30;
      let b1 = p1 + dir_of(a0 + sb * (0.5 + 0.4 * h3)) * 0.20;
      let w = 0.012 * (0.8 + 0.4 * h2);
      seg(q, root, p1, 0.00, 0.34, w, reach, alpha, acc);
      seg(q, p1, p2, 0.34, 0.28, w * 0.75, reach, alpha, acc);
      seg(q, p1, b1, 0.34, 0.18, w * 0.55, reach, alpha, acc);
    }
  }
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let resolution = u.resolution;
  // gl_FragCoord is bottom-left; the fragment position is top-left.
  let frag = vec2<f32>(position.x, resolution.y - position.y);
  let uv = frag / resolution.xy;
  let aspect = resolution.x / resolution.y;
  let p = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
  let t = u.t; // music-driven wave clock
  let tp = u.tp;
  let s0 = 0.17 / max(clamp(u.scale, 0.3, 2.0), 0.3);
  let glow = clamp(u.glow, 0.0, 2.0);
  let wander = clamp(u.wander, 0.0, 2.0);
  let dens = clamp(u.twigs, 0.0, 1.0);
  let px = 1.0 / resolution.y; // frame units per device px

  let gen = floor(t);
  let tg_a = t - gen; // the growing wave
  let tg_b = tg_a + 1.0; // the previous wave, dimming

  // ---- ground: dark damp substrate with pulp mottle and grit
  // (vec2 offset with a non-integer y so the value-noise lattice never aligns
  // with device rows - an integer +4.0 offset laid a 1px crease at p.y = 1/3)
  let mott = fbm(p * 3.0 + vec2<f32>(4.0, 4.173));
  let grit = hash21(floor(frag / max(u.pixel_ratio, 1.0) * 0.5));
  var soil = mix(
    vec3<f32>(0.030, 0.027, 0.025),
    u.c3.rgb * 0.11 + u.c0.rgb * 0.03,
    vec3<f32>(0.75),
  );
  soil = soil * (0.70 + 0.60 * mott);
  soil = soil + vec3<f32>((grit - 0.5) * 0.012);
  let colony_f = fbm(p * 0.9 + vec2<f32>(1.0, 5.0) + tp * 0.006);
  let haze_col = mix(u.c2.rgb, u.c0.rgb, p9_smoothstep(0.35, 0.65, fbm(p * 0.6 + 13.0)));
  soil = soil + haze_col * 0.04 * glow * p9_smoothstep(0.35, 0.8, colony_f);

  // ---- fine fuzz: the felt of finest hyphae the colony lays down, streaks
  // aligned with the radial flow, very faint
  let dp = p - ORIGIN;
  let rp = max(length(dp), 1e-4);
  let thp = atan2(dp.y, dp.x);
  let fz1 = vnoise_x(vec2<f32>((thp / TAU + 0.5) * 90.0, rp * 5.0) + vec2<f32>(7.0), 90.0);
  let fz2 = vnoise_x(vec2<f32>((thp / TAU + 0.5) * 170.0, rp * 9.0) + vec2<f32>(19.0), 170.0);
  var fmat = p9_smoothstep(0.60, 0.92, fz1) * 0.6 + p9_smoothstep(0.66, 0.94, fz2) * 0.4;
  fmat = fmat * (0.4 + 0.6 * p9_smoothstep(0.30, 0.75, colony_f));
  let mat_col = mix(u.c2.rgb, u.c1.rgb, vec3<f32>(0.4)) * 0.14 * glow + vec3<f32>(0.04);
  var col = soil + mat_col * fmat;

  // ---- the two waves, each in its own warped domain
  let warp_a = vec2<f32>(
    fbm(p * 1.4 + vec2<f32>(3.1, 7.7) + gen * 4.7),
    fbm(p * 1.4 + vec2<f32>(9.2, 1.4) + gen * 2.9),
  ) - vec2<f32>(0.5);
  let warp_b = vec2<f32>(
    fbm(p * 1.4 + vec2<f32>(3.1, 7.7) + (gen - 1.0) * 4.7),
    fbm(p * 1.4 + vec2<f32>(9.2, 1.4) + (gen - 1.0) * 2.9),
  ) - vec2<f32>(0.5);
  // a fine wobble so the threads are never ruler-straight
  let wob = vec2<f32>(vnoise(p * 9.0 + 41.0), vnoise(p * 9.0 + 77.0)) - vec2<f32>(0.5);
  let pw_a = p + (warp_a * 0.30 + wob * 0.014) * wander;
  let pw_b = p + (warp_b * 0.30 + wob * 0.014) * wander;

  // previous wave first: it sits beneath
  let colony_b = colony(pw_b, gen - 1.0, tg_b, s0, px);
  let colony_a = colony(pw_a, gen, tg_a, s0, px);

  // twigs on both waves
  let cell = 0.13;
  var twig = Twigs(10.0, 0.02, 0.0, 0.0, 0.0);
  twigs(pw_b, gen - 1.0, tg_b, s0, cell, dens, &twig);
  twigs(pw_a, gen, tg_a, s0, cell, dens, &twig);
  let px_c = px / cell;
  let twig_core = p9_smoothstep(twig.min_w + px_c, twig.min_w - px_c * 0.5, twig.min_d) * twig.min_a;

  // a narrow band of light sweeps outward along the threads; the kick gates it
  let r_a = max(length(pw_a - ORIGIN), 1e-4);
  let pulse_speed = 1.6 + u.beat * 1.2;
  let pulse_pos = my_mod(tp * pulse_speed, RMAX);
  let pulse_delta = r_a - pulse_pos;
  let pulse = exp(-pulse_delta * pulse_delta * 1200.0) * u.beat;

  // ---- colour. Halo of bioluminescence, then the pale hyphal bodies
  let halo_col = mix(u.c2.rgb, u.c1.rgb, p9_smoothstep(0.3, 0.7, colony_f));
  let halo_all = colony_b.halo * 0.5 + colony_a.halo + twig.halo * 0.6;
  col = col + halo_col * 0.22 * glow * min(halo_all, 2.5);

  // old wave: dimmer, cooler, sunk into the ground
  let body_b = mix(vec3<f32>(0.60, 0.60, 0.57), u.c2.rgb, vec3<f32>(0.45))
    * (0.6 + 0.3 * colony_b.bright);
  col = mix(col, body_b, colony_b.core * 0.9);
  col = mix(col, body_b * 0.9, twig_core * 0.8);
  // new wave: pale translucent body lit from within by the pulse
  var body_a = mix(vec3<f32>(0.90, 0.88, 0.82), u.c2.rgb, vec3<f32>(0.28))
    * (0.55 + 0.45 * colony_a.bright);
  body_a = body_a * (0.70 + 0.25 * pulse * glow);
  body_a = body_a + u.c0.rgb * 0.12 * pulse * glow;
  col = mix(col, body_a, colony_a.core);
  col = mix(col, body_a, twig_core * step(0.001, twig.min_a) * 0.9);

  // apex glow: swollen bright tips along the advancing margin, flaring on beat
  let tip_col = mix(u.c1.rgb, vec3<f32>(1.0), vec3<f32>(0.45));
  col = col + tip_col
    * min(colony_a.tip + colony_b.tip * 0.3 + twig.tip * 0.5, 1.4)
    * (0.35 + 0.45 * glow + u.beat * 0.2);

  // depth fog sinks the far strands into the dark; a kick lights the fog it
  // passes through - wider than the strand band, like light scattered by haze
  let fog_depth = max(rp - 0.15, 0.0) + fbm(p * 2.0 + 17.0) * 0.12;
  let fog_amount = 1.0 - exp(-fog_depth * u.fog_density);
  col = mix(col, u.fog_color.rgb, fog_amount);
  let fog_pulse = exp(-pulse_delta * pulse_delta * 280.0) * u.beat;
  col = col + mix(u.c1.rgb, vec3<f32>(1.0), vec3<f32>(0.35)) * fog_pulse * fog_amount * 1.8;

  return vec4<f32>(col, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    #[test]
    fn audio_mapping_matches_the_js_floors_and_caps() {
        let silent = mycelium_audio(0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        assert_eq!(silent.speed, 0.25);
        assert_eq!(silent.glow, 0.15);
        assert_eq!(silent.wander, 0.6);
        assert_eq!(silent.twigs, 0.35);
        assert_eq!(silent.scale, 0.7);
        assert_eq!(silent.fog_density, 0.25 + 0.15 * 0.22);
        assert_eq!(silent.bloom_strength, 0.5 + 0.15 * 0.35);

        let loud = mycelium_audio(1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0);
        assert_eq!(loud.speed, 3.0); // min(0.25 + 3 + 0.5, 3)
        assert_eq!(loud.glow, 2.0); // min(0.15 + 4 + 0.5, 2)
        assert_eq!(loud.wander, 2.0);
        assert_eq!(loud.twigs, 1.0);
        assert!(
            (loud.scale - 1.9).abs() < 1e-5,
            "loud.scale = {}",
            loud.scale
        ); // min(0.7 + 1.2, 2)
        assert_eq!(loud.fog_density, 0.7); // min(0.25 + 0.44 + 0.15, 0.7)
        assert_eq!(loud.bloom_strength, 1.4); // min(0.5 + 0.7 + 0.25, 1.4)
    }

    #[test]
    fn audio_mapping_applies_the_prop_scales() {
        let drive = mycelium_audio(0.0, 0.0, 0.0, 0.0, 2.0, 0.5, 0.0);
        assert_eq!(drive.speed, 0.5);
        assert_eq!(drive.glow, 0.075);
        assert_eq!(drive.fog_density, 0.0);
    }

    #[test]
    fn audio_mapping_ignores_non_finite_bands() {
        let nan = mycelium_audio(f32::NAN, f32::NAN, f32::NAN, f32::NAN, 1.0, 1.0, 1.0);
        assert_eq!(nan, mycelium_audio(0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn growth_clock_is_integrated_and_smooth() {
        let mut scene = Mycelium::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[3] = 0.0;
        let mut bridge = AudioBridge::new();
        for _ in 0..10 {
            bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
            scene.update(1.0 / 60.0, &bridge);
        }
        // Silence: speed = 0.25, so growth advances 10/60 * 0.25.
        assert!((scene.growth() - (10.0 / 60.0) * 0.25).abs() < 1e-5);
        assert!((scene.pulse_clock - scene.growth()).abs() < 1e-6);
        assert!((scene.wave_clock - (scene.growth() / 32.0 + 0.22)).abs() < 1e-6);
    }

    #[test]
    fn growth_never_jumps_when_the_band_changes() {
        let mut scene = Mycelium::new();
        let mut bridge = AudioBridge::new();
        let mut payload = BandPayload::silent(44_100.0);

        payload.lanes[3] = 0.0;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);
        let before = scene.growth();

        // A loud frame changes the *rate*, not the clock.
        payload.lanes[3] = 1.0;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);
        let after = scene.growth();
        assert!(after > before);
        assert!((after - before) <= 3.0 / 60.0 + 1e-5);
    }

    #[test]
    fn phrase_colour_rotates_the_palette() {
        let mut scene = Mycelium::new();
        let mut bridge = AudioBridge::new();
        let mut payload = BandPayload::silent(44_100.0);

        // The scene starts at hue 0, where hueRotate is the identity.
        assert_eq!(scene.colors()[0], hue_rotate(BASE_COLORS[0], 0.0));

        for _ in 0..620 {
            payload.lanes[3] = 0.3;
            bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
            scene.update(1.0 / 60.0, &bridge);
        }
        payload.lanes[3] = 0.9;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);
        payload.lanes[3] = 0.87;
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);

        assert!(scene.phrase.hue != 0.0);
        assert_ne!(scene.colors()[0], hue_rotate(BASE_COLORS[0], 0.0));
        assert_ne!(scene.fog_color(), hue_rotate(FOG_BASE, 0.0));
    }

    #[test]
    fn time_is_capped_at_a_tenth_of_a_second() {
        let mut scene = Mycelium::new();
        let bridge = AudioBridge::new();
        scene.update(5.0, &bridge);
        assert!((scene.time - 0.1).abs() < 1e-6);
        assert!((scene.growth() - 0.1 * 0.25).abs() < 1e-5);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = Mycelium::new();
        scene.apply_props(|key| match key {
            "growthSpeed" => Some(9.0),
            "glow" => Some(-1.0),
            "fogDensity" => Some(1.5),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.glow_scale, 0.0);
        assert_eq!(scene.fog_scale, 1.5);
    }

    #[test]
    fn uniforms_are_128_bytes() {
        assert_eq!(std::mem::size_of::<MyceliumUniforms>(), 128);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = mycelium_wgsl();
        for needle in [
            "fn my_mod(x: f32, y: f32) -> f32",
            "fn vnoise_x(p: vec2<f32>, px: f32) -> f32",
            "0.80 * p.x + -0.60 * p.y",
            "let sgn = select(select(f32(-1.0), f32(1.0), hn < 0.5), f32(0.0), f == 0);",
            "0.0021 * pow(0.80, ff)",
            "seg(q, p1, b1, 0.34, 0.18, w * 0.55, reach, alpha, acc);",
            "exp(-pulse_delta * pulse_delta * 1200.0) * u.beat",
            "1.0 - exp(-fog_depth * u.fog_density)",
            "return vec4<f32>(col, 1.0);",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }

        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("MYCELIUM_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("MYCELIUM_WGSL must validate");
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = Mycelium::new();
        assert_eq!(scene.meta().name, "Point10 Mycelium");
        assert_eq!(scene.meta().category, "2d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 3..],
            &["growthSpeed", "glow", "fogDensity"]
        );
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn bloom_matches_the_js_pass() {
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (0.5, 0.7, 0.25)
        );
    }

    #[test]
    fn the_scene_is_opaque() {
        // The shader writes alpha 1 everywhere, like the JS `gl_FragColor =
        // vec4(col, 1.0)`.
        assert!(MYCELIUM_WGSL.contains("vec4<f32>(col, 1.0)"));
    }
}
