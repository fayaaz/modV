//! Glass Lens — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/GlassLens.js`.
//!
//! Sabo Sugi's "3D Glass Photo Lens" as a music visual: a thick glass block
//! with a PCB photo suspended inside it, over the Chromatic Wave shader as the
//! live backdrop. The JS builds the block from `RoundedBoxGeometry` with a
//! `MeshPhysicalMaterial` (transmission 1, ior 1.5, thickness 1.2, clearcoat),
//! captures the wave quad into a `CubeCamera`/PMREM environment for the bevel
//! reflections, and renders it with ACES filmic tone mapping.
//!
//! # Ported exactly
//!
//! * the pure motion/audio helpers (`GlassLens.js:86-155`):
//!   [`glass_lens_energy`], [`glass_lens_kick`], [`glass_lens_pulse`],
//!   [`glass_lens_sway`] with every constant from `ENERGY`/`KICK`/
//!   `KICK_SMOOTH`/`PULSE`/`MOTION`;
//! * the rig: the group's y rotation integrates `dt * spin * spinScale`, x/z are
//!   the sway's absolute rotations, the uniform scale carries the kick swell and
//!   `position.y` floats on the smoothed beat (`:363-374`);
//! * the photo framing (`PHOTO_SCALE = 1.2`, `_updatePhotoScale`): the plane is
//!   1.2 on its longer axis;
//! * the backdrop: an internal Chromatic Wave (its own ring buffer, texel
//!   texture and shader) with the scene's default distortion/glow, updated with
//!   the same frame delta as the glass (`:187-192`, `:330`);
//! * `CLEAR_ALPHA = 0` (`:47`) — the frame is the wave quad, whose own alpha
//!   fades out away from the line, so the module still composites over the
//!   layers below;
//! * `renderer.toneMapping = ACESFilmicToneMapping` (`:175`).
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `RoundedBoxGeometry` + `MeshPhysicalMaterial` (transmission/ior/thickness/clearcoat) | analytic rounded-box SDF ray march in [`GLASS_LENS_WGSL`] with Snell refraction, Schlick fresnel and a two-lobe specular |
//! | `CubeCamera` + `PMREMGenerator` environment capture | the backdrop itself, sampled along the reflected direction |
//! | `EffectComposer`-less direct render + ACES | [`Composer`] with bloom disabled and `ToneMapping::AcesFilmic` |
//! | `ChromaticWave` composition | [`GlassBackdrop`] (the shared ring-buffer sim + `chromatic/wave.wgsl`) |
//! | `THREE.TextureLoader` + Vite `?url` | [`assets`] (`pcb.jpg`, sRGB) |
//!
//! # Deviations
//!
//! * The glass is raymarched rather than tessellated: the box is the same
//!   2.1 x 2.1 x 1.0 block with the same 0.25 corner radius, and refraction
//!   uses the material's `ior`/`thickness`, but the `transmission` pass is a
//!   single-sample approximation (one refraction step, no per-pixel
//!   multi-bounce).
//! * `mountains.png` is deliberately **not** loaded: it was only the fallback
//!   environment before the live PMREM capture existed, and the port reflects
//!   the backdrop directly (the live capture in the JS). A missing
//!   `$MODV_ASSETS_DIR` therefore costs only `pcb.jpg`, which falls back to the
//!   procedural checkerboard.
//! * `MeshPhysicalMaterial.clearcoat` is approximated by the extra specular
//!   lobe; the glass keeps `ior = 1.5` and `thickness = 1.2` for the
//!   refraction and the bevel falloff.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge, P10Audio};
use crate::point10::chromatic::wave::ChromaticWaveUniforms;
use crate::point10::chromatic::{
    ChromaticLineSim, TexelMapping, sample_count, upload_wave_texture,
};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::shared::assets::{ColourSpace, Wrap, load_texture};
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    frame_dt, read_scene_props, scene_props_with_levers,
};

/// `GlassLens.js: export const GLASS_BOX`.
pub const GLASS_BOX: GlassBox = GlassBox {
    size: 2.1,
    depth: 1.0,
    segments: 32,
    radius: 0.25,
};
/// `GlassLens.js: export const PHOTO_SCALE = 1.2`.
pub const PHOTO_SCALE: f32 = 1.2;
/// `GlassLens.js: export const CAMERA = { fov: 45, positionZ: 3 }`.
pub const CAMERA_FOV: f32 = 45.0;
/// Camera z position.
pub const CAMERA_Z: f32 = 3.0;
/// `GlassLens.js: export const CLEAR_ALPHA = 0`.
pub const CLEAR_ALPHA: f64 = 0.0;
/// `GlassLens.js: GLASS_MATERIAL.ior`.
pub const GLASS_IOR: f32 = 1.5;
/// `GlassLens.js: GLASS_MATERIAL.thickness`.
pub const GLASS_THICKNESS: f32 = 1.2;

/// The box the glass material wraps (`GLASS_BOX`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassBox {
    /// Face size (x and y).
    pub size: f32,
    /// Depth (z).
    pub depth: f32,
    /// Rounded-box segments (only the SDF smoothness matters here).
    pub segments: u32,
    /// Corner radius.
    pub radius: f32,
}

/// `ENERGY = { tau: 3, gain: 1.5 }`.
pub const ENERGY_TAU: f32 = 3.0;
/// Energy gain.
pub const ENERGY_GAIN: f32 = 1.5;
/// `KICK = { jump: 0.08, cooldown: 0.6, gain: 1.2 }`.
pub const KICK_JUMP: f32 = 0.08;
/// Kick cooldown, seconds.
pub const KICK_COOLDOWN: f32 = 0.6;
/// Kick gain.
pub const KICK_GAIN: f32 = 1.2;
/// `KICK_SMOOTH = { rise: 12, fall: 2.5 }`.
pub const KICK_SMOOTH_RISE: f32 = 12.0;
/// Kick fall rate.
pub const KICK_SMOOTH_FALL: f32 = 2.5;
/// `PULSE = { decay: 2.5, attack: 8 }`.
pub const PULSE_DECAY: f32 = 2.5;
/// Pulse attack.
pub const PULSE_ATTACK: f32 = 8.0;

/// `MOTION` — idle spin and the sway/roll/float levers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// Idle spin, rad/s.
    pub spin_base: f32,
    /// Extra spin at full energy.
    pub spin_gain: f32,
    /// Sway frequency base.
    pub sway_time_base: f32,
    /// Sway frequency gain.
    pub sway_time_gain: f32,
    /// Sway amplitude base, radians.
    pub sway_base: f32,
    /// Sway amplitude gain.
    pub sway_gain: f32,
    /// Roll frequency (`0.3 * 0.7`).
    pub roll_time: f32,
    /// Roll amplitude base.
    pub roll_base: f32,
    /// Roll amplitude gain.
    pub roll_gain: f32,
    /// Float frequency.
    pub float_time: f32,
    /// Float amplitude base.
    pub float_base: f32,
    /// Float amplitude gain.
    pub float_gain: f32,
    /// Scale swell at pulse 1.
    pub kick_swell: f32,
}

/// `MOTION` exactly as the JS declares it.
pub const MOTION: Motion = Motion {
    spin_base: 0.5,
    spin_gain: 1.5,
    sway_time_base: 0.4,
    sway_time_gain: 0.5,
    sway_base: 0.2,
    sway_gain: 0.15,
    roll_time: 0.3 * 0.7,
    roll_base: 0.1,
    roll_gain: 0.2,
    float_time: 1.3,
    float_base: 0.08,
    float_gain: 0.15,
    kick_swell: 0.12,
};

/// `GLASS_LENS_PROPS` (`sceneProps.js:199-203`).
const GLASS_LENS_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("pulse", "Kick Pulse", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// `glassLensEnergy(energy, bass, mid, high, dt)` — the one-pole smoothing of
/// the three-band mean that drives the idle spin and sway.
pub fn glass_lens_energy(energy: f32, bass: f32, mid: f32, high: f32, dt: f32) -> f32 {
    let current = clamp01(finite_or(energy, 0.0));
    let level = clamp01((finite_or(bass, 0.0) + finite_or(mid, 0.0) + finite_or(high, 0.0)) / 3.0);
    let step = finite_or(dt, 0.0).max(0.0);
    let beta = 1.0 - (-step * ENERGY_TAU).exp();

    current + (level - current) * (beta * ENERGY_GAIN).min(1.0)
}

/// One frame of the kick detector (`glassLensKick`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassLensKick {
    /// A kick fired this frame.
    pub fire: bool,
    /// `value - previous`.
    pub jump: f32,
    /// The swell target (only when `fire`).
    pub target: Option<f32>,
    /// The raw beat envelope (becomes `prevKick` next frame).
    pub kick: f32,
    /// The asymmetric one-pole smoothed beat.
    pub kick_sm: f32,
}

/// `glassLensKick(kick, prevKick, kickSm, lastPulse, time, dt)`.
pub fn glass_lens_kick(
    kick: f32,
    prev_kick: f32,
    kick_sm: f32,
    last_pulse: f32,
    time: f32,
    dt: f32,
) -> GlassLensKick {
    let value = clamp01(finite_or(kick, 0.0));
    let previous = clamp01(finite_or(prev_kick, 0.0));
    let smoothed = clamp01(finite_or(kick_sm, 0.0));
    let last = finite_or(last_pulse, f32::NEG_INFINITY);
    let now = finite_or(time, 0.0);
    let step = finite_or(dt, 0.0).max(0.0);

    let jump = value - previous;
    let fire = jump > KICK_JUMP && now - last > KICK_COOLDOWN;
    let target = if fire {
        Some((value * KICK_GAIN).min(1.0))
    } else {
        None
    };
    let rate = if value > smoothed {
        KICK_SMOOTH_RISE
    } else {
        KICK_SMOOTH_FALL
    };

    GlassLensKick {
        fire,
        jump,
        target,
        kick: value,
        kick_sm: smoothed + (value - smoothed) * (step * rate).min(1.0),
    }
}

/// One frame of the pulse decay (`glassLensPulse`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassLensPulse {
    /// The decayed target.
    pub target: f32,
    /// The displayed swell.
    pub pulse: f32,
}

/// `glassLensPulse(pulseTarget, pulse, dt)`.
pub fn glass_lens_pulse(pulse_target: f32, pulse: f32, dt: f32) -> GlassLensPulse {
    let step = finite_or(dt, 0.0).max(0.0);
    let target = clamp01(finite_or(pulse_target, 0.0)) * (-step * PULSE_DECAY).exp();
    let current = clamp01(finite_or(pulse, 0.0));

    GlassLensPulse {
        target,
        pulse: current + (target - current) * (step * PULSE_ATTACK).min(1.0),
    }
}

/// The glass group's pose (`glassLensSway`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassLensSway {
    /// Spin rate, rad/s.
    pub spin: f32,
    /// Absolute x rotation, radians.
    pub rotation_x: f32,
    /// Absolute z rotation, radians.
    pub rotation_z: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Vertical float, world units.
    pub position_y: f32,
}

/// `glassLensSway(time, energy, kickSm, pulse)`.
pub fn glass_lens_sway(time: f32, energy: f32, kick_sm: f32, pulse: f32) -> GlassLensSway {
    let t = finite_or(time, 0.0);
    let e = clamp01(finite_or(energy, 0.0));
    let kick = clamp01(finite_or(kick_sm, 0.0));
    let swell = clamp01(finite_or(pulse, 0.0));

    GlassLensSway {
        spin: MOTION.spin_base + e * MOTION.spin_gain,
        rotation_x: (t * (MOTION.sway_time_base + e * MOTION.sway_time_gain)).cos()
            * (MOTION.sway_base + e * MOTION.sway_gain),
        rotation_z: (t * MOTION.roll_time).sin() * (MOTION.roll_base + kick * MOTION.roll_gain),
        scale: 1.0 + swell * MOTION.kick_swell,
        position_y: (t * MOTION.float_time).sin() * (MOTION.float_base + kick * MOTION.float_gain),
    }
}

/// Uniform block, matching [`GLASS_LENS_WGSL`] (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GlassLensUniforms {
    resolution: [f32; 2],
    time: f32,
    fov_scale: f32,
    aspect: f32,
    camera_z: f32,
    _pad0: [f32; 2],
    rotation: [f32; 3],
    scale: f32,
    position_y: f32,
    _pad1: f32,
    photo_scale: [f32; 2],
    brightness: f32,
    ior: f32,
    thickness: f32,
    energy: f32,
    pulse: f32,
    _pad2: [f32; 3],
}

/// The internal Chromatic Wave backdrop.
struct GlassBackdrop {
    sim: ChromaticLineSim,
    p9: P10Audio,
    time: f32,
    size: (u32, u32),
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    wave_texture: wgpu::Texture,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
}

impl GlassBackdrop {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let size = (width.max(1), height.max(1));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-glass-lens-backdrop-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
                    include_str!("../chromatic/wave.wgsl")
                )
                .into(),
            ),
        });

        let wave_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point10-glass-lens-wave-texels"),
            size: wgpu::Extent3d {
                width: crate::point10::chromatic::WAVE_N as u32,
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
        let wave_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-glass-lens-wave-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-glass-lens-backdrop-layout"),
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
            label: Some("point10-glass-lens-backdrop-uniforms"),
            size: std::mem::size_of::<ChromaticWaveUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-glass-lens-backdrop-bind-group"),
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
                    resource: wgpu::BindingResource::Sampler(&wave_sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-glass-lens-backdrop-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-glass-lens-backdrop-pipeline"),
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
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point10-glass-lens-backdrop"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let backdrop = Self {
            sim: ChromaticLineSim::new(),
            p9: P10Audio::new(),
            time: 0.0,
            size,
            texture,
            view,
            wave_texture,
            uniform_buffer,
            bind_group,
            pipeline,
        };
        // The wave shader's `_syncResolution` runs per frame; seed the texture
        // at silence so the first composite has something to sample.
        upload_wave_texture(
            queue,
            &backdrop.wave_texture,
            &backdrop.sim.compute_bytes(TexelMapping::Energy),
        );
        backdrop.upload_uniforms(queue);
        backdrop
    }

    fn uniforms(&self) -> ChromaticWaveUniforms {
        // The backdrop is a default ChromaticWave: distortion 0.05, amplitude
        // and glow at their schema defaults.
        let bands = [
            self.p9.band(-1, 2, 0, 0.0),
            self.p9.band(-1, 2, 1, 0.0),
            self.p9.band(-1, 2, 2, 0.0),
        ];
        ChromaticWaveUniforms {
            resolution: [self.size.0 as f32, self.size.1 as f32],
            _pad0: [0.0; 2],
            x_scale: [
                1.0 + bands[0] * 0.05,
                1.0 + bands[1] * 0.05,
                1.0 + bands[2] * 0.05,
            ],
            _pad1: 0.0,
            glow: [
                (0.05 + bands[0] * 0.3),
                (0.05 + bands[1] * 0.3),
                (0.05 + bands[2] * 0.3),
            ],
            _pad2: 0.0,
            distortion: 0.05,
            time: self.time,
            _pad3: [0.0; 2],
        }
    }

    fn upload_uniforms(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&self.uniforms()),
        );
    }

    fn update(&mut self, queue: &wgpu::Queue, dt: f32, audio: &AudioBridge) {
        if dt > 0.0 {
            self.time += dt;
        }
        self.p9.update(dt, audio);
        if self.p9.has_audio() {
            let k = sample_count(audio.sample_rate, dt);
            if k > 0 {
                self.sim.append_wave_tail(&audio.wave[AUDIO_SAMPLES - k..]);
            }
        }
        let bytes = self.sim.compute_bytes(TexelMapping::Energy);
        upload_wave_texture(queue, &self.wave_texture, &bytes);
        self.upload_uniforms(queue);
    }

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let size = (width.max(1), height.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        self.texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point10-glass-lens-backdrop"),
            size: wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-glass-lens-backdrop"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
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
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// GPU resources built in `init`.
struct GlassLensGpu {
    queue: wgpu::Queue,
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    backdrop: GlassBackdrop,
    backdrop_sampler: wgpu::Sampler,
    pcb_view: wgpu::TextureView,
    pcb_sampler: wgpu::Sampler,
    _pcb_texture: wgpu::Texture,
    composer: Composer,
}

/// The composite bind group (rebuilt whenever the backdrop view is replaced).
fn composite_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    backdrop: &wgpu::TextureView,
    backdrop_sampler: &wgpu::Sampler,
    pcb: &wgpu::TextureView,
    pcb_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-glass-lens-bind-group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(backdrop),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(backdrop_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(pcb),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(pcb_sampler),
            },
        ],
    })
}

impl GlassLensGpu {
    /// Re-create the composite bind group after a backdrop resize.
    fn rebind(&mut self) {
        self.bind_group = composite_bind_group(
            &self.device,
            &self.layout,
            &self.uniform_buffer,
            &self.backdrop.view,
            &self.backdrop_sampler,
            &self.pcb_view,
            &self.pcb_sampler,
        );
    }
}

/// GlassLens scene.
pub struct GlassLens {
    audio: P10Audio,
    /// `BaseScene.time`.
    time: f32,
    /// `this._smEnergy`.
    sm_energy: f32,
    /// `this._kick`.
    kick: f32,
    /// `this._kickSm`.
    kick_sm: f32,
    /// `this._pulseTarget`.
    pulse_target: f32,
    /// `this._pulse`.
    pulse: f32,
    /// `this._lastPulse`.
    last_pulse: f32,
    /// `this.group.rotation.y` — integrated.
    rotation_y: f32,
    /// `this.group.rotation.x/z` from the sway.
    rotation_x: f32,
    rotation_z: f32,
    /// `this.group.scale`.
    scale: f32,
    /// `this.group.position.y`.
    position_y: f32,
    /// `this.spinScale` (`spinSpeed`).
    spin_scale: f32,
    /// `this.pulseScale` (`pulse`).
    pulse_scale: f32,
    /// `this.brightnessScale` (`brightness`).
    brightness_scale: f32,
    /// The PCB aspect (`_photoAspect`), 1 until the asset decodes.
    photo_aspect: f32,
    size: (u32, u32),
    gpu: Option<GlassLensGpu>,
}

impl Default for GlassLens {
    fn default() -> Self {
        Self::new()
    }
}

impl GlassLens {
    /// Create the scene (`GlassLens.js:157-273` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            time: 0.0,
            sm_energy: 0.0,
            kick: 0.0,
            kick_sm: 0.0,
            pulse_target: 0.0,
            pulse: 0.0,
            last_pulse: -1.0,
            rotation_y: 0.0,
            rotation_x: 0.0,
            rotation_z: 0.0,
            scale: 1.0,
            position_y: 0.0,
            spin_scale: 1.0,
            pulse_scale: 1.0,
            brightness_scale: 1.0,
            photo_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:338-342`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(GLASS_LENS_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_scale = value,
                "pulse" => self.pulse_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// The destination alpha the scene clears to ([`CLEAR_ALPHA`]).
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// The smoothed energy.
    pub fn energy(&self) -> f32 {
        self.sm_energy
    }

    /// The displayed kick swell.
    pub fn pulse(&self) -> f32 {
        self.pulse
    }

    /// The glass group's accumulated y rotation.
    pub fn rotation(&self) -> [f32; 3] {
        [self.rotation_x, self.rotation_y, self.rotation_z]
    }

    /// The PCB aspect ratio (width / height).
    pub fn photo_aspect(&self) -> f32 {
        self.photo_aspect
    }

    /// `_updatePhotoScale` — the photo plane's local half extents.
    fn photo_half_extents(&self) -> [f32; 2] {
        let s = PHOTO_SCALE;
        if self.photo_aspect > 1.0 {
            [s * 0.5, (s / self.photo_aspect) * 0.5]
        } else {
            [(s * self.photo_aspect) * 0.5, s * 0.5]
        }
    }

    fn uniforms(&self) -> GlassLensUniforms {
        let photo = self.photo_half_extents();
        GlassLensUniforms {
            resolution: [self.size.0 as f32, self.size.1 as f32],
            time: self.time,
            fov_scale: (CAMERA_FOV.to_radians() * 0.5).tan(),
            aspect: self.size.0 as f32 / self.size.1 as f32,
            camera_z: CAMERA_Z,
            _pad0: [0.0; 2],
            rotation: [self.rotation_x, self.rotation_y, self.rotation_z],
            scale: self.scale.max(0.0001),
            position_y: self.position_y,
            _pad1: 0.0,
            photo_scale: [photo[0] * 2.0, photo[1] * 2.0],
            brightness: self.brightness_scale,
            ior: GLASS_IOR,
            thickness: GLASS_THICKNESS,
            energy: self.sm_energy,
            pulse: self.pulse,
            _pad2: [0.0; 3],
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

impl Point10Scene for GlassLens {
    fn name(&self) -> &'static str {
        "Point10 Glass Lens"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);

        let loader = ctx.asset_loader();
        let pcb = load_texture(
            &device,
            &queue,
            &loader,
            "pcb.jpg",
            ColourSpace::Srgb,
            Wrap::Clamp,
        );
        if pcb.width > 1 && pcb.height > 1 {
            self.photo_aspect = pcb.width as f32 / pcb.height as f32;
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-glass-lens-shader"),
            source: wgpu::ShaderSource::Wgsl(glass_lens_wgsl().into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-glass-lens-layout"),
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
                // The rendered wave backdrop.
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
                // The PCB photo.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-glass-lens-uniforms"),
            size: std::mem::size_of::<GlassLensUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let backdrop = GlassBackdrop::new(&device, &queue, self.size.0, self.size.1);
        let backdrop_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-glass-lens-backdrop-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bind_group = composite_bind_group(
            &device,
            &layout,
            &uniform_buffer,
            &backdrop.view,
            &backdrop_sampler,
            &pcb.view,
            &pcb.sampler,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-glass-lens-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Renders into the composer's HDR scene target.
        let pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-glass-lens",
            &shader,
            &pipeline_layout,
            HDR_FORMAT,
            None,
        );

        // `renderer.toneMapping = ACESFilmicToneMapping`, no composer in the JS;
        // the Rust port presents through the shared chain with bloom off.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);

        self.gpu = Some(GlassLensGpu {
            queue,
            device,
            layout,
            pipeline,
            bind_group,
            uniform_buffer,
            backdrop,
            backdrop_sampler,
            pcb_view: pcb.view,
            pcb_sampler: pcb.sampler,
            _pcb_texture: pcb.texture,
            composer,
        });
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.backdrop.resize(&gpu.device, self.size.0, self.size.1);
            gpu.composer.resize(self.size.0, self.size.1);
            gpu.rebind();
        }
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.time += dt;
        self.audio.update(dt, audio);

        if let Some(gpu) = self.gpu.as_mut() {
            gpu.backdrop.update(&gpu.queue, dt, audio);
        }

        let bands = self.audio.bands();
        let energy = glass_lens_energy(self.sm_energy, bands[0], bands[1], bands[2], dt);
        self.sm_energy = energy;

        let kick = glass_lens_kick(
            self.audio.beat(0.0),
            self.kick,
            self.kick_sm,
            self.last_pulse,
            self.time,
            dt,
        );
        if kick.fire {
            self.last_pulse = self.time;
            self.pulse_target = kick.target.unwrap_or(0.0);
        }
        self.kick = kick.kick;
        self.kick_sm = kick.kick_sm;

        let pulse = glass_lens_pulse(self.pulse_target, self.pulse, dt);
        self.pulse_target = pulse.target;
        self.pulse = pulse.pulse;

        let sway = glass_lens_sway(
            self.time * self.spin_scale,
            energy,
            self.kick_sm,
            self.pulse,
        );
        self.rotation_y += dt * sway.spin * self.spin_scale;
        self.rotation_x = sway.rotation_x;
        self.rotation_z = sway.rotation_z;
        self.scale = 1.0 + (sway.scale - 1.0) * self.pulse_scale;
        self.position_y = sway.position_y * self.pulse_scale;

        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };

        gpu.backdrop.encode(encoder);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-glass-lens"),
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
        scene_props_with_levers(GLASS_LENS_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10GlassLens.js` sets no category, so the adapter default wins.
        SceneMeta::for_scene("Point10 Glass Lens", "3d-scenes")
    }
}

/// The composite shader with the shared vertex entry in front.
fn glass_lens_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{GLASS_LENS_WGSL}")
}

const GLASS_LENS_WGSL: &str = r#"
struct GlassLensUniforms {
  resolution: vec2<f32>,
  time: f32,
  fov_scale: f32,
  aspect: f32,
  camera_z: f32,
  _pad0: vec2<f32>,
  rotation: vec3<f32>,
  scale: f32,
  position_y: f32,
  _pad1: f32,
  photo_scale: vec2<f32>,
  brightness: f32,
  ior: f32,
  thickness: f32,
  energy: f32,
  pulse: f32,
  _pad2a: f32,
  _pad2b: f32,
  _pad2c: f32,
}
@group(0) @binding(0) var<uniform> u: GlassLensUniforms;
@group(0) @binding(1) var backdrop: texture_2d<f32>;
@group(0) @binding(2) var backdrop_sampler: sampler;
@group(0) @binding(3) var pcb: texture_2d<f32>;
@group(0) @binding(4) var pcb_sampler: sampler;

const BOX_HALF: vec3<f32> = vec3<f32>(1.05, 1.05, 0.5);
const BOX_RADIUS: f32 = 0.25;

fn rotate_x(p_in: vec3<f32>, a: f32) -> vec3<f32> {
  let c = cos(a);
  let s = sin(a);
  return vec3<f32>(p_in.x, p_in.y * c - p_in.z * s, p_in.y * s + p_in.z * c);
}
fn rotate_y(p_in: vec3<f32>, a: f32) -> vec3<f32> {
  let c = cos(a);
  let s = sin(a);
  return vec3<f32>(p_in.x * c + p_in.z * s, p_in.y, -p_in.x * s + p_in.z * c);
}
fn rotate_z(p_in: vec3<f32>, a: f32) -> vec3<f32> {
  let c = cos(a);
  let s = sin(a);
  return vec3<f32>(p_in.x * c - p_in.y * s, p_in.x * s + p_in.y * c, p_in.z);
}

// World -> glass group space: undo the float, the rotations (in reverse) and
// the kick swell.
fn to_local(p_in: vec3<f32>) -> vec3<f32> {
  var p = p_in - vec3<f32>(0.0, u.position_y, 0.0);
  p = rotate_y(p, -u.rotation.y);
  p = rotate_x(p, -u.rotation.x);
  p = rotate_z(p, -u.rotation.z);
  return p / u.scale;
}

// Group direction -> world direction (rotations only, then renormalise).
fn to_world(d_in: vec3<f32>) -> vec3<f32> {
  var d = rotate_z(d_in, u.rotation.z);
  d = rotate_x(d, u.rotation.x);
  d = rotate_y(d, u.rotation.y);
  return normalize(d);
}

fn sd_rounded_box(p: vec3<f32>) -> f32 {
  let q = abs(p) - (BOX_HALF - vec3<f32>(BOX_RADIUS));
  return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0) - BOX_RADIUS;
}

fn sdf_normal(p: vec3<f32>) -> vec3<f32> {
  let eps = 0.0015;
  let dx = sd_rounded_box(p + vec3<f32>(eps, 0.0, 0.0)) - sd_rounded_box(p - vec3<f32>(eps, 0.0, 0.0));
  let dy = sd_rounded_box(p + vec3<f32>(0.0, eps, 0.0)) - sd_rounded_box(p - vec3<f32>(0.0, eps, 0.0));
  let dz = sd_rounded_box(p + vec3<f32>(0.0, 0.0, eps)) - sd_rounded_box(p - vec3<f32>(0.0, 0.0, eps));
  return normalize(vec3<f32>(dx, dy, dz) + vec3<f32>(1e-6));
}

// The PCB photo plane at the box centre: returns (hit, uv).
fn photo_uv(p_local: vec3<f32>, dir_local: vec3<f32>) -> vec2<f32> {
  if (abs(dir_local.z) < 1e-6) {
    return vec2<f32>(-1.0);
  }
  let t = -p_local.z / dir_local.z;
  if (t < 0.0) {
    return vec2<f32>(-1.0);
  }
  let hit = p_local + dir_local * t;
  let photo_half = u.photo_scale * 0.5;
  if (abs(hit.x) > photo_half.x || abs(hit.y) > photo_half.y) {
    return vec2<f32>(-1.0);
  }
  // PlaneGeometry's uv has v = 0 at the bottom; the sampled texture's row 0
  // is the top.
  return vec2<f32>(hit.x / photo_half.x * 0.5 + 0.5, 0.5 - hit.y / photo_half.y * 0.5);
}

fn sample_backdrop(uv: vec2<f32>) -> vec4<f32> {
  return textureSample(backdrop, backdrop_sampler, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let resolution = u.resolution;
  let uv = position.xy / resolution;
  let base = sample_backdrop(uv);

  // Camera at (0, 0, camera_z) looking down -z, fov 45.
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
  let ray_world = normalize(vec3<f32>(
    ndc.x * u.fov_scale * u.aspect,
    ndc.y * u.fov_scale,
    -1.0,
  ));
  let origin_world = vec3<f32>(0.0, 0.0, u.camera_z);

  let ro = to_local(origin_world);
  let rd = normalize(to_local(origin_world + ray_world) - ro);

  // Raymarch the rounded box (no texture reads in here, so this loop may
  // diverge freely).
  var t = 0.0;
  var inside = false;
  for (var i = 0; i < 96; i = i + 1) {
    let p = ro + rd * t;
    let d = sd_rounded_box(p);
    if (d < 0.001) {
      inside = true;
      break;
    }
    t = t + max(d * 0.9, 0.002);
    if (t > 40.0) {
      break;
    }
  }

  let p_local = ro + rd * t;
  let n_local = sdf_normal(p_local);
  let n_world = to_world(n_local);
  let rd_world = to_world(rd);

  let view_dir = -rd_world;
  let cos_theta = clamp(dot(n_world, view_dir), 0.0, 1.0);
  // Schlick fresnel for ior 1.5 (F0 = 0.04).
  let fresnel = 0.04 + 0.96 * pow(1.0 - cos_theta, 5.0);

  // Reflection of the backdrop along the mirror direction (the JS captures the
  // wave quad into a cube map for exactly this).
  let reflected = reflect(rd_world, n_world);
  let reflect_uv = vec2<f32>(
    uv.x + reflected.x * 0.18,
    uv.y - reflected.y * 0.18,
  );
  let reflection = sample_backdrop(reflect_uv);

  // Refraction into the glass, then through the block to the photo or the
  // backdrop behind it.
  var refracted = refract(rd_world, n_world, 1.0 / u.ior);
  if (dot(refracted, refracted) < 1e-6) {
    refracted = reflected;
  }
  let refracted_local = to_local(origin_world + refracted);
  let photo = photo_uv(p_local, normalize(refracted_local - ro));
  // The photo sample is taken unconditionally and selected afterwards: WGSL
  // requires `textureSample` in uniform control flow.
  let photo_sample = textureSample(pcb, pcb_sampler, clamp(photo, vec2<f32>(0.0), vec2<f32>(1.0)));
  // The exit point shifts by the thickness along the refracted ray, which is
  // what bends the backdrop behind the glass.
  let shift = refracted.xy * (u.thickness * 0.12);
  let behind = sample_backdrop(vec2<f32>(uv.x + shift.x, uv.y - shift.y));
  let has_photo = photo.x >= 0.0 && photo.x <= 1.0 && photo.y >= 0.0 && photo.y <= 1.0;
  var through = select(behind, photo_sample, has_photo);
  through = vec4<f32>(through.rgb * u.brightness, through.a);

  // Two specular lobes for the three DirectionalLights (back, top, front) and
  // the clearcoat highlight.
  let light_back = normalize(vec3<f32>(-5.0, 2.0, -10.0) - origin_world);
  let light_top = normalize(vec3<f32>(0.0, 10.0, 0.0) - origin_world);
  let light_front = normalize(vec3<f32>(0.0, 2.0, 10.0) - origin_world);
  let h_back = normalize(light_back + view_dir);
  let h_top = normalize(light_top + view_dir);
  let h_front = normalize(light_front + view_dir);
  let spec = pow(max(dot(n_world, h_back), 0.0), 120.0) * 3.0
    + pow(max(dot(n_world, h_top), 0.0), 120.0) * 2.0
    + pow(max(dot(n_world, h_front), 0.0), 80.0) * 1.0;

  // A thin bevel rim: light grazing the rounded corners.
  let bevel = pow(1.0 - cos_theta, 3.0) * 0.6;

  let glass = mix(through.rgb, reflection.rgb, fresnel) * u.brightness;
  let glass_color = glass + vec3<f32>(spec + bevel) * (0.6 + u.energy * 0.6 + u.pulse * 0.8);

  // Outside the block the backdrop quad is the artwork, alpha included, so the
  // module composites over the layers below.
  let color = select(base.rgb, glass_color, inside);
  let alpha = select(base.a, f32(1.0), inside);
  return vec4<f32>(color, alpha);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    #[test]
    fn energy_smooths_the_band_mean() {
        // beta = 1 - exp(-dt * 3); gain 1.5.
        let level = (0.6 + 0.3 + 0.0) / 3.0;
        let step = 1.0 / 60.0;
        let beta = 1.0 - (-step * 3.0_f32).exp();
        let expected = 0.0 + (level - 0.0) * (beta * 1.5).min(1.0);
        assert!((glass_lens_energy(0.0, 0.6, 0.3, 0.0, step) - expected).abs() < 1e-6);

        // A long frame converges to the level.
        assert!((glass_lens_energy(0.0, 1.0, 1.0, 1.0, 100.0) - 1.0).abs() < 1e-6);
        // Non-finite inputs fall back to zero without poisoning the state.
        assert_eq!(glass_lens_energy(f32::NAN, f32::NAN, 0.0, 0.0, 0.0), 0.0);
        // Negative dt is treated as no time passing.
        assert_eq!(glass_lens_energy(0.4, 1.0, 1.0, 1.0, -1.0), 0.4);
    }

    #[test]
    fn kick_fires_on_a_rise_after_the_cooldown() {
        let fired = glass_lens_kick(0.5, 0.1, 0.2, 0.0, 5.0, 1.0 / 60.0);
        assert!(fired.fire);
        assert!((fired.jump - 0.4).abs() < 1e-6);
        assert_eq!(fired.target, Some(0.6));
        // Rise rate while the value is above the smoothed level.
        assert!((fired.kick_sm - (0.2 + (0.5 - 0.2) * (1.0 / 60.0 * 12.0))).abs() < 1e-6);

        // Inside the cooldown the detector stays quiet.
        let cooling = glass_lens_kick(0.5, 0.1, 0.2, 4.9, 5.0, 1.0 / 60.0);
        assert!(!cooling.fire);
        assert_eq!(cooling.target, None);

        // A small rise is not a kick; the fall rate applies when the value
        // drops below the smoothed level.
        let quiet = glass_lens_kick(0.12, 0.1, 0.5, 0.0, 20.0, 1.0 / 60.0);
        assert!(!quiet.fire);
        assert!((quiet.kick_sm - (0.5 + (0.12 - 0.5) * (1.0 / 60.0 * 2.5))).abs() < 1e-6);
    }

    #[test]
    fn pulse_decays_and_eases() {
        let step = 1.0 / 60.0;
        let next = glass_lens_pulse(1.0, 0.0, step);
        assert!((next.target - (-step * 2.5).exp()).abs() < 1e-6);
        assert!((next.pulse - next.target * (step * 8.0)).abs() < 1e-5);

        // Long frame: the target decays to nothing and the swell follows.
        let dead = glass_lens_pulse(1.0, 1.0, 100.0);
        assert!(dead.target < 1e-6 && dead.pulse < 1e-6);
    }

    #[test]
    fn sway_matches_the_motion_curves() {
        let sway = glass_lens_sway(2.0, 0.5, 0.25, 0.5);
        let t = 2.0_f32;
        let e = 0.5_f32;
        assert!((sway.spin - (0.5 + e * 1.5)).abs() < 1e-6);
        assert!(
            (sway.rotation_x - (t * (0.4 + 0.5 * 0.5)).cos() * (0.2 + 0.5 * 0.15)).abs() < 1e-6
        );
        assert!((sway.rotation_z - (t * (0.3 * 0.7)).sin() * (0.1 + 0.25 * 0.2)).abs() < 1e-6);
        assert!((sway.scale - (1.0 + 0.5 * 0.12)).abs() < 1e-6);
        assert!((sway.position_y - (t * 1.3).sin() * (0.08 + 0.25 * 0.15)).abs() < 1e-6);
    }

    #[test]
    fn silence_keeps_the_idle_rig() {
        let mut scene = GlassLens::new();
        let bridge = AudioBridge::new();
        scene.update(0.0, &bridge);
        // Idle spin 0.5 rad/s and the sway at t = 0: cos(0) = 1.
        assert_eq!(scene.rotation()[1], 0.0);
        assert!((scene.rotation()[0] - 0.2).abs() < 1e-6);
        assert_eq!(scene.rotation()[2], 0.0);
        assert_eq!(scene.scale, 1.0);
        // A long frame is capped at 0.1 s by the adapter clock.
        scene.update(0.5, &bridge);
        assert!((scene.rotation()[1] - 0.05).abs() < 1e-5);
    }

    #[test]
    fn time_is_capped_at_a_tenth_of_a_second() {
        let mut scene = GlassLens::new();
        let bridge = AudioBridge::new();
        scene.update(9.0, &bridge);
        assert!((scene.time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn photo_scale_follows_the_asset_aspect() {
        let mut scene = GlassLens::new();
        // Square until the asset loads.
        assert_eq!(scene.photo_half_extents(), [0.6, 0.6]);

        scene.photo_aspect = 2.0;
        assert_eq!(scene.photo_half_extents(), [0.6, 0.3]);
        scene.photo_aspect = 0.5;
        assert_eq!(scene.photo_half_extents(), [0.3, 0.6]);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = GlassLens::new();
        scene.apply_props(|key| match key {
            "spinSpeed" => Some(9.0),
            "pulse" => Some(2.0),
            "brightness" => Some(-1.0),
            _ => None,
        });
        assert_eq!(scene.spin_scale, 4.0);
        assert_eq!(scene.pulse_scale, 2.0);
        assert_eq!(scene.brightness_scale, 0.0);
    }

    #[test]
    fn kick_drives_the_scale_swell() {
        let mut scene = GlassLens::new();
        let mut bridge = AudioBridge::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.beat = 1.0;
        bridge.update(0.1, Some(&payload), &Default::default());

        // A beat attack from silence fires the kick once the cooldown passes.
        scene.update(0.5, &bridge);
        assert!(scene.pulse() > 0.0);
        assert!(scene.scale > 1.0);
    }

    #[test]
    fn uniforms_are_96_bytes() {
        assert_eq!(std::mem::size_of::<GlassLensUniforms>(), 96);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        let program = glass_lens_wgsl();
        for needle in [
            "fn sd_rounded_box(p: vec3<f32>) -> f32",
            "vec3<f32>(1.05, 1.05, 0.5)",
            "const BOX_RADIUS: f32 = 0.25;",
            "refract(rd_world, n_world, 1.0 / u.ior)",
            "0.04 + 0.96 * pow(1.0 - cos_theta, 5.0)",
            "textureSample(pcb, pcb_sampler, clamp(photo, vec2<f32>(0.0), vec2<f32>(1.0)))",
            "return vec4<f32>(color, alpha);",
        ] {
            assert!(program.contains(needle), "shader lost {needle}");
        }

        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("GLASS_LENS_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("GLASS_LENS_WGSL must validate");
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = GlassLens::new();
        assert_eq!(scene.meta().name, "Point10 Glass Lens");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 3..],
            &["spinSpeed", "pulse", "brightness"]
        );
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn material_constants_match_the_js() {
        assert_eq!(GLASS_IOR, 1.5);
        assert_eq!(GLASS_THICKNESS, 1.2);
        assert_eq!(PHOTO_SCALE, 1.2);
        assert_eq!(CAMERA_FOV, 45.0);
        assert_eq!(CAMERA_Z, 3.0);
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(GLASS_BOX.size, 2.1);
        assert_eq!(GLASS_BOX.depth, 1.0);
        assert_eq!(GLASS_BOX.radius, 0.25);
    }
}
