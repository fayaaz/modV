//! SoundCube — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/SoundCube.js`.
//!
//! The Plane9 "Sound Cube" preset (Joakim Dahl, CC BY-NC-SA 4.0): one rotating
//! unit cube seen through a camera at `z = 5` (fov 45), with `Shader1` drawing
//! a bright ring on each face. `RenderToTexture1` holds the cube plus a
//! blurred, zoomed copy of its previous frame — `StoreTexture1` snapshots the
//! target between frames, `Blur1` blurs it on both axes and `Shader3` adds it
//! back with alpha 0, a classic ping-pong feedback trail. `RenderRect1` then
//! composites `Shader2`: `mix(bg, c, c.a) + c`, where `bg` is the layer beneath
//! (transparent black in standalone playback).
//!
//! # Ported exactly
//!
//! * Expression1 (`:79-99`): `scale = (0.3 + bass * 8) * pulseScale` from
//!   `band(-1, 1, 0, 0)` (the smoothed bass lane);
//! * `Rotator1` (`:101-115`): 1.5/1/0.5 deg/s accumulated in radians, scaled by
//!   `spinScale`, with the 0.1 s frame cap;
//! * the shaders (`:117-216`): the cube's `smoothstep(1, 0.5, |p|)` ring times
//!   `smoothstep(0.4, 0.6, |p|)`, the 9-tap gaussian trail blur
//!   (`width 4`, `sigma = max(0.5, width * 0.5)`, both axes), the trail's
//!   `rgb * 0.8 * trailScale` at zoom 0.95, and the composite
//!   `mix(bg, c, c.a) + c` with the `alpha = max(rgb)` transparency rule;
//! * the feedback materials' blend states (`:295-312`): the trail adds RGB
//!   (`ONE/ONE`) with the destination alpha kept (`ZERO/ONE`);
//! * the targets (`:229-240`, `:342-353`): 256² `HalfFloatType` with no depth
//!   or stencil, linear + clamp-to-edge, `Rgba16Float` here; the composite
//!   target is the canvas size;
//! * the camera (`:34`): `PerspectiveCamera(45, aspect, 0.1, 100)` at
//!   `(0, 0, 5)` looking at the origin;
//! * the prop binder (`sceneProps.js:306-311`): `spinScale = spinSpeed`,
//!   `pulseScale = pulse`, `trailScale = trail`, `brightnessScale =
//!   brightness`.
//!
//! # Replaced three.js machinery
//!
//! | three.js | Rust |
//! |----------|------|
//! | `WebGLRenderTarget` ping-pong (`_readRT`/`_writeRT`) | two `Rgba16Float` targets + [`SoundCube::swap_feedback`] |
//! | `StoreTexture1` snapshot | the ping-pong swap (the store node *is* the snapshot) |
//! | `Blur1` (Dir 2 = H then V) | two full-screen gaussian passes |
//! | `PlaneGeometry(2, 2)` + `OrthographicCamera` | the shared full-screen triangle |
//! | `BoxGeometry` | [`build_box`](crate::point10::scenes::cube_mesh::build_box) |
//! | `RenderRect1` → canvas | the composite writes the composer's scene target, `OutputPass` presents it |
//! | `_finalRT` (canvas-sized `UnsignedByte`) | gone: it existed so `captureTexture` could hand the frame to modV, and the engine's preview reads the presented frame |
//!
//! # Deviations
//!
//! * **The composite target.** The JS composes into an 8-bit canvas-sized
//!   target and then copies it to the screen; the port composites into the
//!   composer's `Rgba16Float` scene target and lets `OutputPass` do the sRGB
//!   step. Same values with more headroom (no 8-bit banding in the additive
//!   trail).
//! * **The depth test.** `cubeMaterial` sets `depthTest: false`, `depthWrite:
//!   false`. With `FrontSide` culling that only ever shows the three
//!   front-facing faces, so the port draws double-sided with a depth test: the
//!   nearest surface wins, i.e. the same three faces, without depending on the
//!   winding convention.
//! * `get captureTexture` returns `_finalRT.texture` in the JS; the engine's
//!   preview reads the presented frame, so there is nothing to expose here.
//! * `P9Audio` is `plane9/P9Audio.js` in the JS; the port reads [`P10Audio`]
//!   over the shared [`AudioBridge`] like every other ported scene.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{Composer, ToneMapping};
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, create_depth, depth_state, euler_xyz, look_at, mat4_mul, perspective,
    scale,
};
use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`SoundCube.js:30-62`)
// ---------------------------------------------------------------------------

/// `CUBE_SIZE`.
pub const CUBE_SIZE: f32 = 1.0;

/// `CAMERA.fov`.
pub const CAMERA_FOV: f32 = 45.0;
/// `CAMERA.near`.
pub const CAMERA_NEAR: f32 = 0.1;
/// `CAMERA.far`.
pub const CAMERA_FAR: f32 = 100.0;
/// The camera distance (`_applyCamera`: `cam.position.set(0, 0, 5)`).
pub const CAMERA_DISTANCE: f32 = 5.0;

/// `CLEAR_ALPHA` — the cube composites transparently.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `RTT_SIZE` — `RenderToTexture1` WidthCustom/HeightCustom.
pub const RTT_SIZE: u32 = 256;

/// `TRAIL_ZOOM`.
pub const TRAIL_ZOOM: f32 = 0.95;
/// `TRAIL_BRIGHTNESS`.
pub const TRAIL_BRIGHTNESS: f32 = 0.8;

/// `BLUR.width`.
pub const BLUR_WIDTH: f32 = 4.0;
/// The 9-tap kernel radius.
pub const BLUR_RADIUS: i32 = 4;

/// `ROTATOR`, degrees per second.
pub const ROTATOR: [f32; 3] = [1.5, 1.0, 0.5];

/// `SCALE_BASE`.
pub const SCALE_BASE: f32 = 0.3;
/// `SCALE_AUDIO`.
pub const SCALE_AUDIO: f32 = 8.0;

/// `MAX_DT`.
pub const MAX_DT: f32 = 0.1;

fn number01(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        fallback
    }
}

/// One frame of Expression1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundCubeDrive {
    /// The smoothed bass band.
    pub band: f32,
    /// `0.3 + band * 8`.
    pub expression: f32,
    /// `expression * pulseScale`.
    pub scale: f32,
}

/// Pure Expression1: the smoothed bass band drives the cube's uniform scale.
pub fn sound_cube_drive(bass: f32, pulse_scale: f32) -> SoundCubeDrive {
    let band = number01(bass, 0.0);
    let pulse = number01(pulse_scale, 1.0);
    let expression = SCALE_BASE + band * SCALE_AUDIO;

    SoundCubeDrive {
        band,
        expression,
        scale: expression * pulse,
    }
}

/// `soundCubeFeedback(trailScale)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundCubeFeedback {
    /// `TRAIL_ZOOM`.
    pub zoom: f32,
    /// `TRAIL_BRIGHTNESS * trailScale`.
    pub brightness: f32,
}

/// Pure feedback parameters for Shader3.
pub fn sound_cube_feedback(trail_scale: f32) -> SoundCubeFeedback {
    SoundCubeFeedback {
        zoom: TRAIL_ZOOM,
        brightness: TRAIL_BRIGHTNESS * number01(trail_scale, 1.0),
    }
}

/// Pure `Rotator1`: advances euler angles (radians) from the degree-per-second
/// speeds.
pub fn advance_sound_cube_rotation(rotation: [f32; 3], dt: f32, spin_scale: f32) -> [f32; 3] {
    let step = if dt.is_finite() {
        dt.max(0.0).min(MAX_DT)
    } else {
        0.0
    };
    let spin = number01(spin_scale, 1.0);
    let angle = |value: f32, speed: f32| {
        let base = if value.is_finite() { value } else { 0.0 };
        base + speed.to_radians() * step * spin
    };

    [
        angle(rotation[0], ROTATOR[0]),
        angle(rotation[1], ROTATOR[1]),
        angle(rotation[2], ROTATOR[2]),
    ]
}

// ---------------------------------------------------------------------------
// GPU
// ---------------------------------------------------------------------------

/// `SOUND_CUBE_PROPS` (`sceneProps.js:165-170`).
const SOUND_CUBE_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("pulse", "Pulse", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("trail", "Trail", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

/// The cube's uniform block (128 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CubeUniforms {
    view_proj: Mat4,
    model: Mat4,
}

/// A full-screen pass's uniform block (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct PassUniforms {
    /// Blur: the width. Trail: the trail scale. Composite: the brightness.
    value: f32,
    _pad: f32,
    /// The target size, for `position.xy / resolution`.
    resolution: [f32; 2],
    /// Blur: the texel offset on the active axis.
    step: [f32; 2],
    _pad2: [f32; 2],
}

/// A render target plus its view.
struct Target {
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
}

impl Target {
    fn new(device: &wgpu::Device, label: &str, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // `RenderToTexture1 Format 5 (RG16F)` keeps the additive feedback
            // in float so the trail does not clamp at 1.0.
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            view,
            _texture: texture,
        }
    }
}

/// GPU resources built in `init`.
struct SoundCubeGpu {
    queue: wgpu::Queue,
    cube_pipeline: wgpu::RenderPipeline,
    cube_bind_group: wgpu::BindGroup,
    cube_uniforms: wgpu::Buffer,
    blur_pipeline: wgpu::RenderPipeline,
    /// `blur_from_a` samples the horizontal result with the vertical uniform.
    blur_from_a: wgpu::BindGroup,
    /// `blur_h[read]` samples read target `read` with the horizontal uniform.
    blur_h: [wgpu::BindGroup; 2],
    /// The two blur uniform blocks: `queue.write_buffer` applies to a whole
    /// submission, so one buffer cannot carry both axes' steps.
    blur_uniforms_h: wgpu::Buffer,
    blur_uniforms_v: wgpu::Buffer,
    blur_a: Target,
    blur_b: Target,
    trail_pipeline: wgpu::RenderPipeline,
    trail_bind_group: wgpu::BindGroup,
    trail_uniforms: wgpu::Buffer,
    composite_pipeline: wgpu::RenderPipeline,
    /// `composite_bind[i]` samples read target `i`.
    composite_bind: [wgpu::BindGroup; 2],
    composite_uniforms: wgpu::Buffer,
    targets: [Target; 2],
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    mesh: MeshGpu,
    composer: Composer,
}

/// SoundCube scene.
pub struct SoundCube {
    audio: P10Audio,
    /// `this.rotation` (radians).
    rotation: [f32; 3],
    scale: f32,
    trail_scale: f32,
    brightness_scale: f32,
    spin_scale: f32,
    pulse_scale: f32,
    /// Which target the next frame reads (`_readRT`).
    read_index: usize,
    /// Which target the next frame writes (`_writeRT`).
    write_index: usize,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<SoundCubeGpu>,
}

impl Default for SoundCube {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundCube {
    /// Create the scene (`SoundCube.js:242-372` defaults).
    pub fn new() -> Self {
        Self {
            audio: P10Audio::new(),
            rotation: [0.0; 3],
            scale: SCALE_BASE,
            trail_scale: 1.0,
            brightness_scale: 1.0,
            spin_scale: 1.0,
            pulse_scale: 1.0,
            read_index: 0,
            write_index: 1,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:306-311`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(SOUND_CUBE_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_scale = value,
                "pulse" => self.pulse_scale = value,
                "trail" => self.trail_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// `this.rotation` in radians.
    pub fn rotation(&self) -> [f32; 3] {
        self.rotation
    }

    /// The cube's uniform scale.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The trail brightness as last uploaded.
    pub fn trail_brightness(&self) -> f32 {
        sound_cube_feedback(self.trail_scale).brightness
    }

    /// The feedback target pair `(read, write)` for this frame.
    pub fn feedback_order(&self) -> (usize, usize) {
        (self.read_index, self.write_index)
    }

    /// `_renderFeedback`'s trailing swap: the frame just written becomes the
    /// next source.
    pub fn swap_feedback(&mut self) {
        std::mem::swap(&mut self.read_index, &mut self.write_index);
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &look_at(
                [0.0, 0.0, CAMERA_DISTANCE],
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ),
        )
    }

    fn model(&self) -> Mat4 {
        mat4_mul(
            &euler_xyz(self.rotation[0], self.rotation[1], self.rotation[2]),
            &scale(self.scale, self.scale, self.scale),
        )
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        gpu.queue.write_buffer(
            &gpu.cube_uniforms,
            0,
            bytemuck::bytes_of(&CubeUniforms {
                view_proj: self.view_proj(),
                model: self.model(),
            }),
        );
        gpu.queue.write_buffer(
            &gpu.trail_uniforms,
            0,
            bytemuck::bytes_of(&PassUniforms {
                value: sound_cube_feedback(self.trail_scale).brightness,
                _pad: 0.0,
                resolution: [RTT_SIZE as f32, RTT_SIZE as f32],
                step: [0.0; 2],
                _pad2: [0.0; 2],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.composite_uniforms,
            0,
            bytemuck::bytes_of(&PassUniforms {
                value: if self.brightness_scale.is_finite() {
                    self.brightness_scale.max(0.0)
                } else {
                    1.0
                },
                _pad: 0.0,
                resolution: [self.size.0 as f32, self.size.1 as f32],
                step: [0.0; 2],
                _pad2: [0.0; 2],
            }),
        );
    }

    /// Upload the blur's per-axis step.
    ///
    /// The two axes get one uniform buffer each: `queue.write_buffer` applies
    /// to the whole submission, so a single buffer cannot carry two different
    /// steps for the two passes recorded in one encoder.
    fn upload_blur(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let texel = 1.0 / RTT_SIZE as f32;
        for (buffer, step) in [
            (&gpu.blur_uniforms_h, [texel, 0.0]),
            (&gpu.blur_uniforms_v, [0.0, texel]),
        ] {
            gpu.queue.write_buffer(
                buffer,
                0,
                bytemuck::bytes_of(&PassUniforms {
                    value: BLUR_WIDTH,
                    _pad: 0.0,
                    resolution: [RTT_SIZE as f32, RTT_SIZE as f32],
                    step,
                    _pad2: [0.0; 2],
                }),
            );
        }
    }
}

impl Point10Scene for SoundCube {
    fn name(&self) -> &'static str {
        "Point10 Sound Cube"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width, ctx.height);
        self.camera_aspect = ctx.aspect();
        self.read_index = 0;
        self.write_index = 1;

        let blur_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-sound-cube-rtt-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // --- cube ----------------------------------------------------------
        let cube_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-sound-cube-cube-shader"),
            source: wgpu::ShaderSource::Wgsl(sound_cube_cube_wgsl().into()),
        });
        let cube_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-sound-cube-cube-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let cube_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-sound-cube-cube-uniforms"),
            size: std::mem::size_of::<CubeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cube_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-sound-cube-cube-bind-group"),
            layout: &cube_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cube_uniforms.as_entire_binding(),
            }],
        });
        let cube_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-sound-cube-cube-pipeline-layout"),
            bind_group_layouts: &[Some(&cube_layout)],
            immediate_size: 0,
        });
        let cube_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-sound-cube-cube-pipeline"),
            layout: Some(&cube_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &cube_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(crate::point10::scenes::cube_mesh::Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &cube_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    // `blending: THREE.NoBlending` — overwrite the target.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // --- full-screen passes -------------------------------------------
        let pass_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-sound-cube-pass-shader"),
            source: wgpu::ShaderSource::Wgsl(sound_cube_pass_wgsl().into()),
        });
        let pass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-sound-cube-pass-layout"),
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
        let blur_uniforms_h = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-sound-cube-blur-uniforms-h"),
            size: std::mem::size_of::<PassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blur_uniforms_v = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-sound-cube-blur-uniforms-v"),
            size: std::mem::size_of::<PassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let trail_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-sound-cube-trail-uniforms"),
            size: std::mem::size_of::<PassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let composite_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-sound-cube-composite-uniforms"),
            size: std::mem::size_of::<PassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let targets = [
            Target::new(&device, "point10-sound-cube-rtt-a", RTT_SIZE, RTT_SIZE),
            Target::new(&device, "point10-sound-cube-rtt-b", RTT_SIZE, RTT_SIZE),
        ];
        let blur_a = Target::new(&device, "point10-sound-cube-blur-a", RTT_SIZE, RTT_SIZE);
        let blur_b = Target::new(&device, "point10-sound-cube-blur-b", RTT_SIZE, RTT_SIZE);

        let make_bind = |uniforms: &wgpu::Buffer, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("point10-sound-cube-pass-bind-group"),
                layout: &pass_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&blur_sampler),
                    },
                ],
            })
        };

        let blur_h = [
            make_bind(&blur_uniforms_h, &targets[0].view),
            make_bind(&blur_uniforms_h, &targets[1].view),
        ];
        let blur_from_a = make_bind(&blur_uniforms_v, &blur_a.view);
        let trail_bind_group = make_bind(&trail_uniforms, &blur_b.view);
        // The JS's PreviousLayer is the layer beneath the scene; in standalone
        // playback it is a 1x1 transparent placeholder, so the composite only
        // needs the feedback target.
        let composite_bind = [
            make_bind(&composite_uniforms, &targets[0].view),
            make_bind(&composite_uniforms, &targets[1].view),
        ];

        let pass_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-sound-cube-pass-pipeline-layout"),
            bind_group_layouts: &[Some(&pass_layout)],
            immediate_size: 0,
        });
        let build_pass_pipeline = |label: &str, entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pass_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &pass_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pass_shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let blur_pipeline =
            build_pass_pipeline("point10-sound-cube-blur-pipeline", "fs_blur", None);
        // `trailMaterial`: `CustomBlending`, `AddEquation`, `OneFactor`/`OneFactor`,
        // alpha `ZeroFactor`/`OneFactor` (`:302-312`).
        let additive_preserve_alpha = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let trail_pipeline = build_pass_pipeline(
            "point10-sound-cube-trail-pipeline",
            "fs_trail",
            Some(additive_preserve_alpha),
        );
        let composite_pipeline = build_pass_pipeline(
            "point10-sound-cube-composite-pipeline",
            "fs_composite",
            None,
        );

        let (depth, depth_view) =
            create_depth(&device, "point10-sound-cube-depth", RTT_SIZE, RTT_SIZE);

        // The JS renders straight to the canvas with `NoToneMapping`; the
        // composer only supplies the sRGB presentation step (and keeps the
        // composite's alpha).
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        let mesh = MeshGpu::new(
            &device,
            "point10-sound-cube-mesh",
            &build_box(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
        );

        self.gpu = Some(SoundCubeGpu {
            queue,
            cube_pipeline,
            cube_bind_group,
            cube_uniforms,
            blur_pipeline,
            blur_from_a,
            blur_h,
            blur_uniforms_h,
            blur_uniforms_v,
            blur_a,
            blur_b,
            trail_pipeline,
            trail_bind_group,
            trail_uniforms,
            composite_pipeline,
            composite_bind,
            composite_uniforms,
            targets,
            _depth: depth,
            depth_view,
            mesh,
            composer,
        });
        self.upload_uniforms();
        self.upload_blur();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.resize(self.size.0, self.size.1);
        }
        self.upload_uniforms();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.audio.update(dt, audio);

        // Expression1: smoothed bass drives the cube's uniform scale.
        let drive = sound_cube_drive(self.audio.band(-1, 1, 0, 0.0), self.pulse_scale);
        self.scale = drive.scale;

        // Rotator1: degrees per second around each axis.
        self.rotation = advance_sound_cube_rotation(self.rotation, dt, self.spin_scale);
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let (read, write) = self.feedback_order();

        // `_renderFeedback()`: blur the previous frame (H then V), draw the cube
        // into `write`, then add the blurred, zoomed trail back.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-sound-cube-blur-h"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.blur_a.view,
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
            pass.set_pipeline(&gpu.blur_pipeline);
            pass.set_bind_group(0, &gpu.blur_h[read], &[]);
            pass.draw(0..3, 0..1);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-sound-cube-blur-v"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.blur_b.view,
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
            pass.set_pipeline(&gpu.blur_pipeline);
            pass.set_bind_group(0, &gpu.blur_from_a, &[]);
            pass.draw(0..3, 0..1);
        }

        // `renderer.setRenderTarget(_writeRT)` + clear + `render(cubeGroup)`.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-sound-cube-cube"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.targets[write].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.cube_pipeline);
            pass.set_bind_group(0, &gpu.cube_bind_group, &[]);
            gpu.mesh.draw(&mut pass);
        }

        // Shader3: add the blurred, zoomed previous frame with alpha 0.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-sound-cube-trail"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.targets[write].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.trail_pipeline);
            pass.set_bind_group(0, &gpu.trail_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Swap: the target just written is the next frame's source.
        let presented = write;

        // `_renderComposite()`: `mix(bg, c, c.a) + c` into the presented frame.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-sound-cube-composite"),
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
            pass.set_pipeline(&gpu.composite_pipeline);
            pass.set_bind_group(0, &gpu.composite_bind[presented], &[]);
            pass.draw(0..3, 0..1);
        }

        // The frame's read/write usage is over: the target just written becomes
        // the next source (`_swapFeedback` at the end of the JS `render`).
        self.swap_feedback();

        if let Some(gpu) = self.gpu.as_mut() {
            gpu.composer.render(encoder, target);
        }
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(SOUND_CUBE_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10SoundCube.js` sets no category.
        SceneMeta::for_scene("Point10 Sound Cube", "3d-scenes")
    }
}

/// The cube shader (`buildSoundCubeVertexShader` + fragment).
fn sound_cube_cube_wgsl() -> String {
    format!("{P9_COMMON_WGSL}\n{SOUND_CUBE_CUBE_WGSL}")
}

/// The blur / trail / composite passes (`buildSoundCubeBlurFragment`,
/// `buildSoundCubeTrailFragment`, `buildSoundCubeCompositeFragment`).
fn sound_cube_pass_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{SOUND_CUBE_PASS_WGSL}")
}

const SOUND_CUBE_CUBE_WGSL: &str = r#"
struct CubeUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> u: CubeUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) tex: vec2<f32>,
  @location(1) diffuse: vec4<f32>,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
) -> VsOut {
  var out: VsOut;
  out.tex = uv;
  // `vDiffuse = gColor * gColor1`, both (1, 1, 1, 1) in this preset.
  out.diffuse = vec4<f32>(1.0);
  out.position = u.view_proj * (u.model * vec4<f32>(position, 1.0));
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let p = in.tex * 2.0 - 1.0;
  let v = p9_smoothstep(1.0, 0.5, abs(p.x)) * p9_smoothstep(1.0, 0.5, abs(p.y));
  let inner = p9_smoothstep(0.4, 0.6, length(p));
  return vec4<f32>(vec3<f32>(v * inner) * in.diffuse.rgb, 1.0);
}
"#;

const SOUND_CUBE_PASS_WGSL: &str = r#"
struct PassUniforms {
  // Blur: the width. Trail: the trail scale. Composite: the brightness.
  value: f32,
  _pad: f32,
  // The target size, for `position.xy / resolution`.
  resolution: vec2<f32>,
  // Blur: the texel offset on the active axis.
  step: vec2<f32>,
  _pad2: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: PassUniforms;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

// `buildSoundCubeBlurFragment`: a 9-tap gaussian, one axis per pass.
@fragment
fn fs_blur(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let sigma = max(0.5, u.value * 0.5);
  var sum = vec3<f32>(0.0);
  var total = 0.0;
  for (var i = -4; i <= 4; i = i + 1) {
    let fi = f32(i);
    let weight = exp(-0.5 * fi * fi / (sigma * sigma));
    sum = sum + textureSample(source, source_sampler, uv + u.step * fi).rgb * weight;
    total = total + weight;
  }
  return vec4<f32>(sum / total, 1.0);
}

// `buildSoundCubeTrailFragment`: the zoomed previous frame, dimmed, alpha 0.
@fragment
fn fs_trail(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let p = uv * 2.0 - 1.0;
  let c = textureSample(source, source_sampler, (p * 0.95) * 0.5 + 0.5);
  return vec4<f32>(c.rgb * 0.8 * u.value, 0.0);
}

// `buildSoundCubeCompositeFragment`: `mix(bg, c, c.a) + c` with the visible
// glow as the alpha, so empty space stays transparent.
@fragment
fn fs_composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  // PreviousLayer is transparent black in standalone playback.
  let bg = vec4<f32>(0.0);
  let c = textureSample(source, source_sampler, uv);
  let col = mix(bg, c, c.a) + c;
  let rgb = col.rgb * u.value;
  let alpha = clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0);
  return vec4<f32>(rgb, alpha);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::BandPayload;

    // Port of tests/unit/point10-sound-cube.test.js.

    #[test]
    fn drive_scales_the_cube_from_the_smoothed_bass() {
        let silent = sound_cube_drive(0.0, 1.0);
        assert_eq!(silent.band, 0.0);
        assert!((silent.expression - SCALE_BASE).abs() < 1e-6);
        assert!((silent.scale - SCALE_BASE).abs() < 1e-6);

        let loud = sound_cube_drive(0.5, 1.0);
        assert!((loud.expression - (0.3 + 4.0)).abs() < 1e-6);
        assert!((loud.scale - 4.3).abs() < 1e-6);
    }

    #[test]
    fn drive_multiplies_by_the_pulse_prop_and_sanitises_bad_input() {
        let pulsed = sound_cube_drive(0.25, 2.0);
        assert!((pulsed.scale - (0.3 + 2.0) * 2.0).abs() < 1e-6);

        let negative = sound_cube_drive(-1.0, -1.0);
        assert_eq!(negative.band, 0.0);
        // `number01(pulseScale, 1)` clamps a negative pulse to zero.
        assert_eq!(negative.scale, 0.0);

        let bad = sound_cube_drive(f32::NAN, f32::NAN);
        assert_eq!(bad.band, 0.0);
        assert!((bad.scale - SCALE_BASE).abs() < 1e-6);
    }

    #[test]
    fn feedback_keeps_the_preset_zoom_and_scales_the_brightness() {
        let unity = sound_cube_feedback(1.0);
        assert_eq!(unity.zoom, TRAIL_ZOOM);
        assert!((unity.brightness - TRAIL_BRIGHTNESS).abs() < 1e-6);

        let off = sound_cube_feedback(0.0);
        assert_eq!(off.brightness, 0.0);

        let bad = sound_cube_feedback(f32::NAN);
        assert!((bad.brightness - TRAIL_BRIGHTNESS).abs() < 1e-6);
    }

    #[test]
    fn rotator_advances_degrees_per_second_in_radians() {
        let start = [0.0, 0.0, 0.0];
        // One frame at the 0.1 s cap: 1.5°/1°/0.5° per second.
        let next = advance_sound_cube_rotation(start, MAX_DT, 1.0);

        assert!((next[0] - 1.5_f32.to_radians() * MAX_DT).abs() < 1e-6);
        assert!((next[1] - 1.0_f32.to_radians() * MAX_DT).abs() < 1e-6);
        assert!((next[2] - 0.5_f32.to_radians() * MAX_DT).abs() < 1e-6);
    }

    #[test]
    fn rotator_caps_the_step_and_scales_with_spin_speed() {
        let capped = advance_sound_cube_rotation([0.0; 3], 5.0, 1.0);
        assert!((capped[0] - (1.5_f32 * MAX_DT).to_radians()).abs() < 1e-6);

        let spun = advance_sound_cube_rotation([0.0; 3], 0.1, 4.0);
        assert!((spun[0] - (1.5_f32 * 0.1 * 4.0).to_radians()).abs() < 1e-6);

        let bad = advance_sound_cube_rotation([f32::NAN; 3], f32::NAN, f32::NAN);
        assert_eq!(bad, [0.0; 3]);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(CUBE_SIZE, 1.0);
        assert_eq!((CAMERA_FOV, CAMERA_NEAR, CAMERA_FAR), (45.0, 0.1, 100.0));
        assert_eq!(CAMERA_DISTANCE, 5.0);
        assert_eq!(RTT_SIZE, 256);
        assert_eq!(TRAIL_ZOOM, 0.95);
        assert_eq!(TRAIL_BRIGHTNESS, 0.8);
        assert_eq!(BLUR_WIDTH, 4.0);
        assert_eq!(BLUR_RADIUS, 4);
        assert_eq!(ROTATOR, [1.5, 1.0, 0.5]);
        assert_eq!(SCALE_BASE, 0.3);
        assert_eq!(SCALE_AUDIO, 8.0);
        assert_eq!(CLEAR_ALPHA, 0.0);
    }

    #[test]
    fn feedback_starts_with_read_a_and_flips() {
        let mut scene = SoundCube::new();
        assert_eq!(scene.feedback_order(), (0, 1));
        scene.swap_feedback();
        assert_eq!(scene.feedback_order(), (1, 0));
    }

    #[test]
    fn update_scales_and_rotates_from_the_bass_lane() {
        let mut scene = SoundCube::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.lanes[3] = 0.05; // damping lane 1 of the bass band
        payload.byte_frequency_data = vec![0; 1024];
        payload.freq_mag = [0.5; crate::point10::audio_bridge::AUDIO_SAMPLES];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        scene.update(0.1, &bridge);
        assert!((scene.scale() - (0.3 + 0.05 * 8.0)).abs() < 1e-5);
        assert!((scene.rotation()[0] - (1.5_f32 * 0.1).to_radians()).abs() < 1e-6);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = SoundCube::new();
        scene.apply_props(|key| match key {
            "spinSpeed" => Some(2.0),
            "pulse" => Some(9.0),
            "trail" => Some(0.5),
            "brightness" => Some(-1.0),
            _ => None,
        });
        assert_eq!(scene.spin_scale, 2.0);
        assert_eq!(scene.pulse_scale, 4.0);
        assert_eq!(scene.trail_scale, 0.5);
        assert_eq!(scene.brightness_scale, 0.0);
        assert!((scene.trail_brightness() - TRAIL_BRIGHTNESS * 0.5).abs() < 1e-6);
    }

    #[test]
    fn uniforms_are_the_documented_size() {
        assert_eq!(std::mem::size_of::<CubeUniforms>(), 128);
        assert_eq!(std::mem::size_of::<PassUniforms>(), 32);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("cube", sound_cube_cube_wgsl()),
            ("pass", sound_cube_pass_wgsl()),
        ] {
            let module = match naga::front::wgsl::parse_str(&program) {
                Ok(module) => module,
                Err(error) => panic!("{label} WGSL failed to parse: {error}"),
            };
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{label} WGSL must validate: {error}"));
        }
    }

    #[test]
    fn shaders_keep_the_preset_maths() {
        let cube = sound_cube_cube_wgsl();
        for needle in [
            "p9_smoothstep(1.0, 0.5, abs(p.x))",
            "p9_smoothstep(1.0, 0.5, abs(p.y))",
            "p9_smoothstep(0.4, 0.6, length(p))",
            "vec3<f32>(v * inner) * in.diffuse.rgb",
        ] {
            assert!(cube.contains(needle), "cube lost {needle}");
        }

        let pass = sound_cube_pass_wgsl();
        for needle in [
            "let sigma = max(0.5, u.value * 0.5);",
            "exp(-0.5 * fi * fi / (sigma * sigma))",
            "for (var i = -4; i <= 4; i = i + 1)",
            "(p * 0.95) * 0.5 + 0.5",
            "c.rgb * 0.8 * u.value",
            "mix(bg, c, c.a) + c",
            "let alpha = clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0);",
        ] {
            assert!(pass.contains(needle), "pass lost {needle}");
        }
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = SoundCube::new();
        assert_eq!(scene.meta().name, "Point10 Sound Cube");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 4..],
            &["spinSpeed", "pulse", "trail", "brightness"]
        );
        assert_eq!(names[0], "gain");
    }
}
