//! GPU volumetric fire/smoke simulation — port of the TSL compute + raymarch
//! machinery in `src/renderer/src/application/point10/scenes/SmokeRoom.js`.
//!
//! SmokeRoom and DvdRoom are the same scene with a different emitter (a lava
//! sphere vs. a bouncing extruded DVD logo). The JS builds the *same* grid,
//! the same seven compute passes and the same 16-step `VolumeNodeMaterial`
//! raymarch in both; only the emitter geometry, its world matrix and a few
//! light/tuning values differ. This module is that shared engine, so the two
//! Rust scenes configure it instead of approximating the field.
//!
//! # Grid
//!
//! The JS grid is `GRID_SIZE_X x GRID_SIZE_Y x GRID_SIZE_Z =
//! 150 x 150 x 300 = 6 750 000` cells (`SmokeRoom.js:60-63`), i.e. 12.5 voxels
//! per world unit across the `12 x 12 x 24` volume. Every state texture is
//! `Rgba16Float` (three's `HalfFloatType`), 54 MB per texture; the simulation
//! keeps velocity/pressure/dye ping-pong pairs plus divergence and a curl-noise
//! field, 432 MB total, which is what the JS allocates too.
//!
//! [`VolumeGrid`] can be constructed at a smaller size for tests and the
//! offline reference renderer ([`VolumeGrid::new`]); the scene defaults are
//! [`VolumeGrid::JS`]. Everything (UVW mapping, texel offsets, dispatch) is
//! derived from the grid, so a scaled grid simulates the same world-space
//! behaviour at lower detail.
//!
//! # Passes (one per `uDt` = 1/120 * simulationSpeed step)
//!
//! 1. **advect velocity** — semi-Lagrangian self-advection, thermal + ambient
//!    curl turbulence, radial buoyancy away from the emitter, the emitter's
//!    wind, and the boundary damping that seals the room.
//! 2. **divergence** — central differences of the advected velocity.
//! 3. **jacobi AB / jacobi BA** — `PRESSURE_ITERATIONS` (2) pressure
//!    iterations.
//! 4. **project** — subtracts the pressure gradient.
//! 5. **advect dye** — density/temperature/age advection with the smoke and
//!    fire dissipation rates.
//! 6. **emit** — one invocation per emitter vertex: adds density + temperature
//!    at the vertex's voxel with the noise flicker and the hi-hat/snare pulse.
//!
//! The curl-noise field is computed once at [`VolumeSim::new`], like the JS's
//! init-time `computeAsync(computeCurlNoisePass)`.
//!
//! # Ping-pong without read-write storage textures
//!
//! WebGPU (and wgpu's default limits) allow `Rgba16Float` **write-only**
//! storage textures without extra features, but not read-write storage
//! textures. The JS's `Storage3DTexture` swaps are therefore expressed as
//! ping-pong pairs with one sampled view and one write-only view per pass —
//! exactly the hardware the TSL graph compiles to. No device feature is
//! required; see [`VolumeSim::new`].
//!
//! # Raymarch
//!
//! [`VolumeSim::render`] records the 16-step analytic box raymarch
//! (`passes/raymarch.wgsl`) into a half-resolution HDR target, matching
//! `volumetricPass.setResolutionScale(0.5)` and its `gaussianBlur`/combine
//! chain in [`stage::VolumeStage`].

#![forbid(unsafe_code)]

pub mod room;
pub mod stage;

use crate::point10::scenes::cube_mesh::{Mat4, identity};

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

/// `GRID_SIZE_X` (`SmokeRoom.js:60`).
pub const GRID_SIZE_X: u32 = 150;
/// `GRID_SIZE_Y` (`SmokeRoom.js:61`).
pub const GRID_SIZE_Y: u32 = 150;
/// `GRID_SIZE_Z` (`SmokeRoom.js:62`).
pub const GRID_SIZE_Z: u32 = 300;

/// `CELL_COUNT` (`SmokeRoom.js:63`).
pub const CELL_COUNT: u32 = GRID_SIZE_X * GRID_SIZE_Y * GRID_SIZE_Z;

/// `PRESSURE_ITERATIONS` (`SmokeRoom.js:64`). Must be even: the iteration pair
/// leaves the final pressure in texture A, which `project` samples.
pub const PRESSURE_ITERATIONS: u32 = 2;

/// `volumetricMaterial.steps = 16` (`SmokeRoom.js:840`).
pub const RAYMARCH_STEPS: u32 = 16;
/// Shadow-march steps inside `scatteringNode` (`SmokeRoom.js:900`).
pub const SHADOW_STEPS: u32 = 2;
/// `shadowStepSize` (`SmokeRoom.js:898`).
pub const SHADOW_STEP_SIZE: f32 = 0.35;

/// The grid's world footprint (`VOLUME_WORLD_SIZE_*`, `SmokeRoom.js:65-67`).
pub const VOLUME_WORLD_SIZE_X: f32 = 12.0;
/// See [`VOLUME_WORLD_SIZE_X`].
pub const VOLUME_WORLD_SIZE_Y: f32 = 12.0;
/// See [`VOLUME_WORLD_SIZE_X`].
pub const VOLUME_WORLD_SIZE_Z: f32 = 24.0;

/// The compute workgroup is `4 x 4 x 4` cells (64 invocations, the WebGPU
/// default limit per workgroup is 256).
pub const WORKGROUP_SIZE: u32 = 4;

/// Storage/sample format of every volume texture: three's `HalfFloatType`.
pub const VOLUME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// A voxel grid size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeGrid {
    /// Width (X).
    pub x: u32,
    /// Height (Y).
    pub y: u32,
    /// Depth (Z).
    pub z: u32,
}

impl VolumeGrid {
    /// The JS grid (`150 x 150 x 300`).
    pub const JS: Self = Self::new(GRID_SIZE_X, GRID_SIZE_Y, GRID_SIZE_Z);

    /// A grid with every axis clamped to at least one voxel.
    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        Self {
            x: if x > 0 { x } else { 1 },
            y: if y > 0 { y } else { 1 },
            z: if z > 0 { z } else { 1 },
        }
    }

    /// `CELL_COUNT` for this grid.
    pub const fn cell_count(self) -> u32 {
        self.x * self.y * self.z
    }

    /// `TEXEL_X/Y/Z` (`SmokeRoom.js:70-72`).
    pub fn texel(self) -> [f32; 3] {
        [
            1.0 / self.x as f32,
            1.0 / self.y as f32,
            1.0 / self.z as f32,
        ]
    }

    /// Workgroup counts for the 3D passes (`ceil(dim / 4)`).
    pub fn dispatch(self) -> [u32; 3] {
        [
            self.x.div_ceil(WORKGROUP_SIZE),
            self.y.div_ceil(WORKGROUP_SIZE),
            self.z.div_ceil(WORKGROUP_SIZE),
        ]
    }

    /// The grid as the shader's `u.grid.xyz`.
    pub fn dims(self) -> [f32; 3] {
        [self.x as f32, self.y as f32, self.z as f32]
    }
}

// ---------------------------------------------------------------------------
// Uniforms
// ---------------------------------------------------------------------------

/// The uniform block every pass binds at `group(0) binding(0)`.
///
/// Mirrors `VolumeUniforms` in `volume/common.wgsl` field for field; every row
/// is a `vec4` (or the trailing matrix) so there is no padding to get wrong.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VolumeUniforms {
    /// xyz grid dims, w simulation dt (`uDt`).
    pub grid: [f32; 4],
    /// xyz `uVolumeWorldSize`, w `uTime`.
    pub world: [f32; 4],
    /// xyz `uGridOrigin`, w `uBoxScale`.
    pub origin: [f32; 4],
    /// xyz `uSpherePosition`, w `uSphereSpeed`.
    pub sphere_pos: [f32; 4],
    /// xyz `uSphereVelocity`, w `uMotionBoost`.
    pub sphere_vel: [f32; 4],
    /// xyz camera position, w frame id (raymarch jitter).
    pub camera: [f32; 4],
    /// Camera right axis.
    pub cam_r: [f32; 4],
    /// Camera up axis.
    pub cam_u: [f32; 4],
    /// Camera forward axis.
    pub cam_f: [f32; 4],
    /// rgb `uFireStartColor`, w `uFireIntensity`.
    pub fire_start: [f32; 4],
    /// rgb `uFireMidColor`, w `uFireGlowSpread`.
    pub fire_mid: [f32; 4],
    /// rgb `uFireEndColor`, w `uFireHue`.
    pub fire_end: [f32; 4],
    /// x `uBuoyancy`, y `uWeight`, z `uTurbulence`, w `uTurbulenceDecay`.
    pub turb: [f32; 4],
    /// x `uTurbFrequency`, y `uVelDamping`, z `uCooling`, w `uDissipation`.
    pub turb2: [f32; 4],
    /// x `uEmitDensity`, y `uEmitTemperature`, z `uFireSize`, w `uEmitGain`.
    pub emit: [f32; 4],
    /// x `uWindStrength`, y `uFlicker`, z `uFlameHeight`, w unused.
    pub emit2: [f32; 4],
    /// x `uShadowAbsorption`, y `uShadowAmbient`, z `uMultiScattering`,
    /// w `uPowderStrength`.
    pub light: [f32; 4],
    /// x `uAsymmetry`, yzw unused.
    pub light2: [f32; 4],
    /// rgb the volume point-light colour (hue-equalised, flickered and
    /// fade-in scaled on the CPU), w unused.
    pub point_light: [f32; 4],
    /// x tan(fov/2), y aspect, z raymarch width, w raymarch height.
    pub view: [f32; 4],
    /// x emitter vertex count, yzw unused.
    pub misc: [f32; 4],
    /// `uSphereMatrix`: the emitter's world matrix.
    pub emitter_model: Mat4,
}

fn fire_stop(hex: u32, intensity: f32) -> [f32; 4] {
    let colour = crate::point10::shared::phrase_color::srgb_hex_to_linear(hex);
    [colour[0], colour[1], colour[2], intensity]
}

impl Default for VolumeUniforms {
    /// The JS defaults (`SmokeRoom.js:479-526`).
    fn default() -> Self {
        Self {
            grid: [
                GRID_SIZE_X as f32,
                GRID_SIZE_Y as f32,
                GRID_SIZE_Z as f32,
                0.01,
            ],
            world: [
                VOLUME_WORLD_SIZE_X,
                VOLUME_WORLD_SIZE_Y,
                VOLUME_WORLD_SIZE_Z,
                0.0,
            ],
            origin: [0.0, -VOLUME_WORLD_SIZE_Y / 2.0, 0.0, 1.0],
            sphere_pos: [0.0, 0.0, 0.0, 0.0],
            sphere_vel: [0.0, 0.0, 0.0, 0.25],
            camera: [0.0, 0.0, 0.0, 0.0],
            cam_r: [1.0, 0.0, 0.0, 0.0],
            cam_u: [0.0, 1.0, 0.0, 0.0],
            cam_f: [0.0, 0.0, -1.0, 0.0],
            // `0xffe68c` / `0xff7305` / `0xff0000` in linear-sRGB, times the
            // `uFireIntensity`/`uFireGlowSpread` defaults. three's
            // `THREE.Color(hex)` converts sRGB -> linear on the CPU, so the
            // shader's ramp stops are linear.
            fire_start: fire_stop(0x00ff_e68c, 40.0),
            fire_mid: fire_stop(0x00ff_7305, 5.0),
            fire_end: fire_stop(0x00ff_0000, 0.0),
            turb: [3.0, 0.15, 3.2, 0.1],
            turb2: [10.0, 0.25, 0.769_23, 0.285_71],
            emit: [28.0, 22.0, 1.0, 1.0],
            emit2: [6.5, 1.0, 3.5, 0.0],
            light: [2.0, 0.5, 1.0, 0.59],
            light2: [0.0, 0.0, 0.0, 0.0],
            point_light: [0.0, 0.0, 0.0, 0.0],
            view: [0.577_35, 1.0, 1.0, 1.0],
            misc: [0.0, 0.0, 0.0, 0.0],
            emitter_model: identity(),
        }
    }
}

impl VolumeUniforms {
    /// Defaults re-targeted at `grid`.
    pub fn for_grid(grid: VolumeGrid) -> Self {
        Self {
            grid: [grid.x as f32, grid.y as f32, grid.z as f32, 0.01],
            ..Self::default()
        }
    }

    /// Write the grid dims (`u.grid.xyz`).
    pub fn set_grid(&mut self, grid: VolumeGrid) {
        self.grid[0] = grid.x as f32;
        self.grid[1] = grid.y as f32;
        self.grid[2] = grid.z as f32;
    }

    /// Set the camera basis + position from an eye/target/up triple (the JS
    /// `camera.updateMatrixWorld()` values the raymarch needs).
    pub fn set_camera(&mut self, eye: [f32; 3], target: [f32; 3], up: [f32; 3]) {
        let forward = crate::point10::scenes::cube_mesh::normalise([
            target[0] - eye[0],
            target[1] - eye[1],
            target[2] - eye[2],
        ]);
        let right = crate::point10::scenes::cube_mesh::normalise([
            forward[1] * up[2] - forward[2] * up[1],
            forward[2] * up[0] - forward[0] * up[2],
            forward[0] * up[1] - forward[1] * up[0],
        ]);
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        self.camera = [eye[0], eye[1], eye[2], self.camera[3]];
        self.cam_r = [right[0], right[1], right[2], 0.0];
        self.cam_u = [up[0], up[1], up[2], 0.0];
        self.cam_f = [forward[0], forward[1], forward[2], 0.0];
    }

    /// `uSphereMatrix` from an emitter transform.
    pub fn set_emitter_model(&mut self, model: Mat4) {
        self.emitter_model = model;
    }
}

// ---------------------------------------------------------------------------
// GPU resources
// ---------------------------------------------------------------------------

/// One `Rgba16Float` 3D texture with its sampled and write-only storage views.
struct VolumeTexture {
    _texture: wgpu::Texture,
    sampled: wgpu::TextureView,
    storage: wgpu::TextureView,
}

impl VolumeTexture {
    fn new(device: &wgpu::Device, label: &str, grid: VolumeGrid) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: grid.x,
                height: grid.y,
                depth_or_array_layers: grid.z,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: VOLUME_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let sampled = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let storage = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            sampled,
            storage,
        }
    }
}

/// The volume textures, kept alive for the test-only readback (the bind groups
/// hold their own references, so only the dye field is ever read, and only by
/// the debug harness).
#[allow(dead_code)]
struct VolumeTextures {
    velocity: [VolumeTexture; 2],
    dye: [VolumeTexture; 2],
    pressure: [VolumeTexture; 2],
    divergence: VolumeTexture,
    curl: VolumeTexture,
}

/// The compute + raymarch pipelines.
struct Pipelines {
    curl: wgpu::ComputePipeline,
    advect_velocity: wgpu::ComputePipeline,
    divergence: wgpu::ComputePipeline,
    jacobi: wgpu::ComputePipeline,
    project: wgpu::ComputePipeline,
    advect_dye: wgpu::ComputePipeline,
    emit: wgpu::ComputePipeline,
    raymarch: wgpu::RenderPipeline,
}

/// The bind groups for both dye parities.
struct BindGroups {
    curl: wgpu::BindGroup,
    advect_velocity: wgpu::BindGroup,
    divergence: wgpu::BindGroup,
    jacobi_ab: wgpu::BindGroup,
    jacobi_ba: wgpu::BindGroup,
    project: wgpu::BindGroup,
    advect_dye: [wgpu::BindGroup; 2],
    emit: [wgpu::BindGroup; 2],
    raymarch: [wgpu::BindGroup; 2],
}

/// The GPU simulation: textures, pipelines and bind groups for one scene.
pub struct VolumeSim {
    grid: VolumeGrid,
    /// Index of the dye texture that currently holds the state.
    parity: usize,
    uniforms: wgpu::Buffer,
    pipelines: Pipelines,
    binds: BindGroups,
    emitter_count: u32,
    #[cfg_attr(not(test), allow(dead_code))]
    textures: VolumeTextures,
}

impl VolumeSim {
    /// Build the simulation and precompute the curl-noise field.
    ///
    /// `emitter` is the emitter mesh's local vertex positions — the same list
    /// the JS passes to `storage(geometry.attributes.position, "vec3", n)`.
    /// The emitter's *world* transform comes from
    /// [`VolumeUniforms::set_emitter_model`] per frame.
    ///
    /// No `wgpu::Features` are required: `Rgba16Float` supports write-only
    /// storage texture access (`texture_storage_3d<rgba16float, write>`) on the
    /// WebGPU baseline, and the shaders never read a texture they write in the
    /// same pass (see the module docs).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        grid: VolumeGrid,
        emitter: &[[f32; 3]],
    ) -> Self {
        let velocity = [
            VolumeTexture::new(device, "point10-volume-velocity-a", grid),
            VolumeTexture::new(device, "point10-volume-velocity-b", grid),
        ];
        let dye = [
            VolumeTexture::new(device, "point10-volume-dye-a", grid),
            VolumeTexture::new(device, "point10-volume-dye-b", grid),
        ];
        let pressure = [
            VolumeTexture::new(device, "point10-volume-pressure-a", grid),
            VolumeTexture::new(device, "point10-volume-pressure-b", grid),
        ];
        let divergence = VolumeTexture::new(device, "point10-volume-divergence", grid);
        let curl = VolumeTexture::new(device, "point10-volume-curl", grid);

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-volume-uniforms"),
            size: std::mem::size_of::<VolumeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-volume-linear-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        // `curlNoiseTex.wrapS/T/R = RepeatWrapping` (`SmokeRoom.js:463-465`).
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-volume-repeat-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let emitter_buffer = {
            use wgpu::util::DeviceExt;
            let mut packed: Vec<[f32; 4]> = emitter
                .iter()
                .map(|position| [position[0], position[1], position[2], 1.0])
                .collect();
            if packed.is_empty() {
                packed.push([0.0, 0.0, 0.0, 1.0]);
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-volume-emitter"),
                contents: bytemuck::cast_slice(&packed),
                usage: wgpu::BufferUsages::STORAGE,
            })
        };
        let emitter_count = emitter.len().max(1) as u32;

        let uniform_entry =
            |binding: u32, visibility: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
        let sampled_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let storage_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: VOLUME_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D3,
            },
            count: None,
        };
        let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;

        let curl_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-curl-layout"),
            entries: &[uniform_entry(0, compute), storage_entry(1)],
        });
        let advect_velocity_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("point10-volume-advect-velocity-layout"),
                entries: &[
                    uniform_entry(0, compute),
                    sampled_entry(1),
                    sampled_entry(2),
                    sampled_entry(3),
                    sampler_entry(4),
                    sampler_entry(5),
                    storage_entry(6),
                ],
            });
        let divergence_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-divergence-layout"),
            entries: &[
                uniform_entry(0, compute),
                sampled_entry(1),
                sampler_entry(2),
                storage_entry(3),
            ],
        });
        // The Jacobi, projection and dye-advection passes share one shape:
        // two sampled volumes, a sampler and one write-only target.
        let pair_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-pair-layout"),
            entries: &[
                uniform_entry(0, compute),
                sampled_entry(1),
                sampled_entry(2),
                sampler_entry(3),
                storage_entry(4),
            ],
        });
        let emit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-emit-layout"),
            entries: &[
                uniform_entry(0, compute),
                sampled_entry(1),
                sampler_entry(2),
                storage_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let raymarch_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-volume-raymarch-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                sampled_entry(1),
                sampled_entry(2),
                sampler_entry(3),
            ],
        });

        let compute_pipeline = |label: &str, layout: &wgpu::BindGroupLayout, body: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(program(body).into()),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };

        let pipelines = Pipelines {
            curl: compute_pipeline(
                "point10-volume-curl-pipeline",
                &curl_layout,
                include_str!("passes/curl.wgsl"),
            ),
            advect_velocity: compute_pipeline(
                "point10-volume-advect-velocity-pipeline",
                &advect_velocity_layout,
                include_str!("passes/advect_velocity.wgsl"),
            ),
            divergence: compute_pipeline(
                "point10-volume-divergence-pipeline",
                &divergence_layout,
                include_str!("passes/divergence.wgsl"),
            ),
            jacobi: compute_pipeline(
                "point10-volume-jacobi-pipeline",
                &pair_layout,
                include_str!("passes/jacobi.wgsl"),
            ),
            project: compute_pipeline(
                "point10-volume-project-pipeline",
                &pair_layout,
                include_str!("passes/project.wgsl"),
            ),
            advect_dye: compute_pipeline(
                "point10-volume-advect-dye-pipeline",
                &pair_layout,
                include_str!("passes/advect_dye.wgsl"),
            ),
            emit: compute_pipeline(
                "point10-volume-emit-pipeline",
                &emit_layout,
                include_str!("passes/emit.wgsl"),
            ),
            raymarch: {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("point10-volume-raymarch-shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        format!(
                            "{}\n{}",
                            crate::point10::FULLSCREEN_TRIANGLE_WGSL,
                            program(include_str!("passes/raymarch.wgsl"))
                        )
                        .into(),
                    ),
                });
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("point10-volume-raymarch-pipeline-layout"),
                        bind_group_layouts: &[Some(&raymarch_layout)],
                        immediate_size: 0,
                    });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("point10-volume-raymarch-pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
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
                        module: &module,
                        entry_point: Some("fs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: VOLUME_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            },
        };

        let binds = BindGroups {
            curl: create_bind_group(
                device,
                "point10-volume-curl-bind-group",
                &curl_layout,
                &[
                    uniform_binding(&uniforms),
                    texture_binding(1, &curl.storage),
                ],
            ),
            advect_velocity: create_bind_group(
                device,
                "point10-volume-advect-velocity-bind-group",
                &advect_velocity_layout,
                &[
                    uniform_binding(&uniforms),
                    texture_binding(1, &velocity[0].sampled),
                    texture_binding(2, &dye[0].sampled),
                    texture_binding(3, &curl.sampled),
                    sampler_binding(4, &linear_sampler),
                    sampler_binding(5, &repeat_sampler),
                    texture_binding(6, &velocity[1].storage),
                ],
            ),
            divergence: create_bind_group(
                device,
                "point10-volume-divergence-bind-group",
                &divergence_layout,
                &[
                    uniform_binding(&uniforms),
                    texture_binding(1, &velocity[1].sampled),
                    sampler_binding(2, &linear_sampler),
                    texture_binding(3, &divergence.storage),
                ],
            ),
            jacobi_ab: create_bind_group(
                device,
                "point10-volume-jacobi-ab-bind-group",
                &pair_layout,
                &[
                    uniform_binding(&uniforms),
                    texture_binding(1, &pressure[0].sampled),
                    texture_binding(2, &divergence.sampled),
                    sampler_binding(3, &linear_sampler),
                    texture_binding(4, &pressure[1].storage),
                ],
            ),
            jacobi_ba: create_bind_group(
                device,
                "point10-volume-jacobi-ba-bind-group",
                &pair_layout,
                &[
                    uniform_binding(&uniforms),
                    texture_binding(1, &pressure[1].sampled),
                    texture_binding(2, &divergence.sampled),
                    sampler_binding(3, &linear_sampler),
                    texture_binding(4, &pressure[0].storage),
                ],
            ),
            project: create_bind_group(
                device,
                "point10-volume-project-bind-group",
                &pair_layout,
                &[
                    uniform_binding(&uniforms),
                    texture_binding(1, &pressure[0].sampled),
                    texture_binding(2, &velocity[1].sampled),
                    sampler_binding(3, &linear_sampler),
                    texture_binding(4, &velocity[0].storage),
                ],
            ),
            advect_dye: [
                create_bind_group(
                    device,
                    "point10-volume-advect-dye-bind-group-0",
                    &pair_layout,
                    &[
                        uniform_binding(&uniforms),
                        texture_binding(1, &velocity[0].sampled),
                        texture_binding(2, &dye[0].sampled),
                        sampler_binding(3, &linear_sampler),
                        texture_binding(4, &dye[1].storage),
                    ],
                ),
                create_bind_group(
                    device,
                    "point10-volume-advect-dye-bind-group-1",
                    &pair_layout,
                    &[
                        uniform_binding(&uniforms),
                        texture_binding(1, &velocity[0].sampled),
                        texture_binding(2, &dye[1].sampled),
                        sampler_binding(3, &linear_sampler),
                        texture_binding(4, &dye[0].storage),
                    ],
                ),
            ],
            emit: [
                create_bind_group(
                    device,
                    "point10-volume-emit-bind-group-0",
                    &emit_layout,
                    &[
                        uniform_binding(&uniforms),
                        texture_binding(1, &dye[0].sampled),
                        sampler_binding(2, &linear_sampler),
                        texture_binding(3, &dye[1].storage),
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: emitter_buffer.as_entire_binding(),
                        },
                    ],
                ),
                create_bind_group(
                    device,
                    "point10-volume-emit-bind-group-1",
                    &emit_layout,
                    &[
                        uniform_binding(&uniforms),
                        texture_binding(1, &dye[1].sampled),
                        sampler_binding(2, &linear_sampler),
                        texture_binding(3, &dye[0].storage),
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: emitter_buffer.as_entire_binding(),
                        },
                    ],
                ),
            ],
            raymarch: [
                create_bind_group(
                    device,
                    "point10-volume-raymarch-bind-group-0",
                    &raymarch_layout,
                    &[
                        uniform_binding(&uniforms),
                        texture_binding(1, &dye[0].sampled),
                        texture_binding(2, &velocity[0].sampled),
                        sampler_binding(3, &linear_sampler),
                    ],
                ),
                create_bind_group(
                    device,
                    "point10-volume-raymarch-bind-group-1",
                    &raymarch_layout,
                    &[
                        uniform_binding(&uniforms),
                        texture_binding(1, &dye[1].sampled),
                        texture_binding(2, &velocity[0].sampled),
                        sampler_binding(3, &linear_sampler),
                    ],
                ),
            ],
        };

        let textures = VolumeTextures {
            velocity,
            dye,
            pressure,
            divergence,
            curl,
        };
        let sim = Self {
            grid,
            parity: 0,
            uniforms,
            pipelines,
            binds,
            emitter_count,
            textures,
        };

        // `renderer.computeAsync(computeCurlNoisePass)` (`SmokeRoom.js:836`).
        sim.precompute_curl(queue, device);
        sim
    }

    /// The grid this simulation runs at.
    pub fn grid(&self) -> VolumeGrid {
        self.grid
    }

    /// How many emitter vertices seed the field each step.
    pub fn emitter_count(&self) -> u32 {
        self.emitter_count
    }

    /// Write the per-step uniforms (and stamp the emitter vertex count the
    /// emit pass dispatches over).
    pub fn write(&self, queue: &wgpu::Queue, uniforms: &VolumeUniforms) {
        let mut uniforms = *uniforms;
        uniforms.misc[0] = self.emitter_count as f32;
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Record one simulation step (the seven passes of `_animate`'s inner
    /// loop, `SmokeRoom.js:1374-1389`) and swap the dye ping-pong pair.
    ///
    /// `uniforms` must already be written with [`VolumeSim::write`] for this
    /// step (the JS updates `uTime`, `uFlicker`, … per step).
    pub fn step(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let groups = self.grid.dispatch();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("point10-volume-step"),
            timestamp_writes: None,
        });

        pass.set_pipeline(&self.pipelines.advect_velocity);
        pass.set_bind_group(0, &self.binds.advect_velocity, &[]);
        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);

        pass.set_pipeline(&self.pipelines.divergence);
        pass.set_bind_group(0, &self.binds.divergence, &[]);
        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);

        for iteration in 0..PRESSURE_ITERATIONS {
            let bind = if iteration % 2 == 0 {
                &self.binds.jacobi_ab
            } else {
                &self.binds.jacobi_ba
            };
            pass.set_pipeline(&self.pipelines.jacobi);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
        }

        pass.set_pipeline(&self.pipelines.project);
        pass.set_bind_group(0, &self.binds.project, &[]);
        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);

        pass.set_pipeline(&self.pipelines.advect_dye);
        pass.set_bind_group(0, &self.binds.advect_dye[self.parity], &[]);
        pass.dispatch_workgroups(groups[0], groups[1], groups[2]);

        pass.set_pipeline(&self.pipelines.emit);
        pass.set_bind_group(0, &self.binds.emit[self.parity], &[]);
        pass.dispatch_workgroups(self.emitter_count.div_ceil(64), 1, 1);

        drop(pass);
        self.parity ^= 1;
    }

    /// Record the raymarch into `target` (the half-resolution volumetric
    /// pass).
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-volume-raymarch"),
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
        pass.set_pipeline(&self.pipelines.raymarch);
        pass.set_bind_group(0, &self.binds.raymarch[self.parity], &[]);
        pass.draw(0..3, 0..1);
    }

    /// Read the current dye field back as `[density, temperature, age, _]`
    /// voxels (test/debug only; x fastest, then y, then z).
    #[cfg(test)]
    pub(crate) fn read_dye(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<[f32; 4]> {
        let grid = self.grid;
        let unpadded = grid.x as u64 * 8;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
        let padded = unpadded.div_ceil(align) * align;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-volume-dye-readback"),
            size: padded * grid.y as u64 * grid.z as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("point10-volume-dye-readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.textures.dye[self.parity]._texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(grid.y),
                },
            },
            wgpu::Extent3d {
                width: grid.x,
                height: grid.y,
                depth_or_array_layers: grid.z,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll must succeed");
        receiver
            .recv()
            .expect("map callback must run")
            .expect("mapping must succeed");

        let mapped = slice.get_mapped_range().expect("buffer must be mapped");
        let mut out = Vec::with_capacity(grid.cell_count() as usize);
        for z in 0..grid.z as u64 {
            for y in 0..grid.y as u64 {
                let row = (z * grid.y as u64 + y) * padded;
                for x in 0..grid.x as u64 {
                    let offset = (row + x * 8) as usize;
                    let mut lanes = [0.0_f32; 4];
                    for (lane, value) in lanes.iter_mut().enumerate() {
                        let half = u16::from_le_bytes([
                            mapped[offset + lane * 2],
                            mapped[offset + lane * 2 + 1],
                        ]);
                        *value = half_to_f32(half);
                    }
                    out.push(lanes);
                }
            }
        }
        drop(mapped);
        buffer.unmap();
        out
    }

    fn precompute_curl(&self, queue: &wgpu::Queue, device: &wgpu::Device) {
        let groups = self.grid.dispatch();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("point10-volume-curl"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("point10-volume-curl-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines.curl);
            pass.set_bind_group(0, &self.binds.curl, &[]);
            pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
        }
        queue.submit(Some(encoder.finish()));
    }
}

fn create_bind_group<'a>(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    entries: &[wgpu::BindGroupEntry<'a>],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries,
    })
}

fn uniform_binding(buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding: 0,
        resource: buffer.as_entire_binding(),
    }
}

fn texture_binding(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn sampler_binding(binding: u32, sampler: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::Sampler(sampler),
    }
}

/// IEEE 754 binary16 -> f32 (the volume textures are `Rgba16Float`).
#[cfg(test)]
fn half_to_f32(half: u16) -> f32 {
    let sign = (half & 0x8000) as u32;
    let exponent = ((half >> 10) & 0x1f) as u32;
    let mantissa = (half & 0x03ff) as u32;
    let bits = if exponent == 0 {
        if mantissa == 0 {
            sign << 16
        } else {
            // Subnormal.
            let mut exponent = -1_i32;
            let mut mantissa = mantissa as u16;
            while mantissa & 0x0400 == 0 {
                mantissa <<= 1;
                exponent -= 1;
            }
            let mantissa = (mantissa as u32 & 0x03ff) << 13;
            let exponent = ((exponent + 127 - 15) as u32) << 23;
            (sign << 16) | exponent | mantissa
        }
    } else if exponent == 0x1f {
        (sign << 16) | 0x7f80_0000 | (mantissa << 13)
    } else {
        (sign << 16) | ((exponent + 127 - 15) << 23) | (mantissa << 13)
    };
    f32::from_bits(bits)
}

/// Assemble a shader as `common.wgsl` + the pass body.
fn program(body: &str) -> String {
    format!("{}\n{}", include_str!("common.wgsl"), body)
}

/// Every shader program the volume owns (for the naga validation test).
#[cfg(test)]
pub(crate) fn shader_programs() -> Vec<(&'static str, String)> {
    vec![
        ("curl", program(include_str!("passes/curl.wgsl"))),
        (
            "advect_velocity",
            program(include_str!("passes/advect_velocity.wgsl")),
        ),
        (
            "divergence",
            program(include_str!("passes/divergence.wgsl")),
        ),
        ("jacobi", program(include_str!("passes/jacobi.wgsl"))),
        ("project", program(include_str!("passes/project.wgsl"))),
        (
            "advect_dye",
            program(include_str!("passes/advect_dye.wgsl")),
        ),
        ("emit", program(include_str!("passes/emit.wgsl"))),
        (
            "raymarch",
            format!(
                "{}\n{}",
                crate::point10::FULLSCREEN_TRIANGLE_WGSL,
                program(include_str!("passes/raymarch.wgsl"))
            ),
        ),
        (
            "blur",
            format!(
                "{}\n{}",
                crate::point10::FULLSCREEN_TRIANGLE_WGSL,
                include_str!("passes/blur.wgsl")
            ),
        ),
        (
            "combine",
            format!(
                "{}\n{}",
                crate::point10::FULLSCREEN_TRIANGLE_WGSL,
                include_str!("passes/combine.wgsl")
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn the_js_grid_is_150_by_150_by_300() {
        assert_eq!(VolumeGrid::JS, VolumeGrid::new(150, 150, 300));
        assert_eq!(VolumeGrid::JS.cell_count(), 6_750_000);
        assert_eq!(CELL_COUNT, 6_750_000);
        // The world footprint gives 12.5 voxels per world unit on every axis,
        // i.e. one texel per 0.08 world units.
        let texel = VolumeGrid::JS.texel();
        close(texel[0] * VOLUME_WORLD_SIZE_X, 0.08, 1e-6);
        close(texel[1] * VOLUME_WORLD_SIZE_Y, 0.08, 1e-6);
        close(texel[2] * VOLUME_WORLD_SIZE_Z, 0.08, 1e-6);
        close(texel[0], 1.0 / 150.0, 1e-9);
        close(texel[2], 1.0 / 300.0, 1e-9);
    }

    #[test]
    fn dispatch_covers_every_voxel() {
        assert_eq!(VolumeGrid::JS.dispatch(), [38, 38, 75]);
        // 38 * 4 = 152 >= 150, 75 * 4 = 300 exactly.
        let groups = VolumeGrid::JS.dispatch();
        assert!(groups[0] * WORKGROUP_SIZE >= VolumeGrid::JS.x);
        assert!(groups[1] * WORKGROUP_SIZE >= VolumeGrid::JS.y);
        assert_eq!(groups[2] * WORKGROUP_SIZE, VolumeGrid::JS.z);
        assert_eq!(VolumeGrid::new(8, 4, 1).dispatch(), [2, 1, 1]);
        assert_eq!(VolumeGrid::new(0, 0, 0), VolumeGrid::new(1, 1, 1));
    }

    #[test]
    fn the_pressure_pair_is_even_so_project_reads_a() {
        assert_eq!(PRESSURE_ITERATIONS % 2, 0);
        assert_eq!(RAYMARCH_STEPS, 16);
        assert_eq!(SHADOW_STEPS, 2);
        close(SHADOW_STEP_SIZE, 0.35, 1e-9);
    }

    #[test]
    fn uniforms_match_the_wgsl_layout() {
        // 21 rows of vec4 + the emitter matrix.
        assert_eq!(std::mem::size_of::<VolumeUniforms>(), 21 * 16 + 64);
        assert_eq!(std::mem::size_of::<VolumeUniforms>() % 16, 0);
        assert_eq!(VOLUME_FORMAT, wgpu::TextureFormat::Rgba16Float);

        let defaults = VolumeUniforms::for_grid(VolumeGrid::new(4, 5, 6));
        assert_eq!(defaults.grid, [4.0, 5.0, 6.0, defaults.grid[3]]);
        assert_eq!(defaults.world[0..3], [12.0, 12.0, 24.0]);
        assert_eq!(defaults.origin[1], -6.0);
        assert_eq!(
            (
                defaults.turb[0],
                defaults.turb[1],
                defaults.turb[2],
                defaults.turb[3]
            ),
            (3.0, 0.15, 3.2, 0.1)
        );
        assert_eq!((defaults.turb2[0], defaults.turb2[1]), (10.0, 0.25));
        assert_eq!(
            (defaults.emit[0], defaults.emit[1], defaults.emit[3]),
            (28.0, 22.0, 1.0)
        );
        assert_eq!(
            (
                defaults.light[0],
                defaults.light[1],
                defaults.light[2],
                defaults.light[3]
            ),
            (2.0, 0.5, 1.0, 0.59)
        );
        assert_eq!(defaults.emit2[0], 6.5);
        assert_eq!(defaults.fire_mid[3], 5.0);
        assert_eq!(defaults.fire_start[3], 40.0);
    }

    #[test]
    fn the_camera_basis_orthonormalises_like_three() {
        let mut uniforms = VolumeUniforms {
            camera: [14.0, 5.5, 4.4, 7.0],
            ..VolumeUniforms::default()
        };
        uniforms.set_camera([14.0, 5.5, 4.4], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(uniforms.camera, [14.0, 5.5, 4.4, 7.0]);
        let forward = uniforms.cam_f;
        let right = uniforms.cam_r;
        let up = uniforms.cam_u;
        let dot = |a: [f32; 4], b: [f32; 4]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        close(dot(forward, right), 0.0, 1e-6);
        close(dot(forward, up), 0.0, 1e-6);
        close(dot(right, up), 0.0, 1e-6);
        close(dot(forward, forward), 1.0, 1e-6);
        close(dot(right, right), 1.0, 1e-6);
        close(dot(up, up), 1.0, 1e-6);
        // Forward points from the camera at the origin it looks at.
        assert!(forward[0] < 0.0 && forward[1] < 0.0 && forward[2] < 0.0);
        // Up is roughly +Y (the JS camera up).
        assert!(up[1] > 0.5);
    }

    #[test]
    fn every_shader_parses_and_validates() {
        for (label, source) in shader_programs() {
            let module = match naga::front::wgsl::parse_str(&source) {
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
    fn shaders_keep_the_ported_maths() {
        let common = include_str!("common.wgsl");
        for needle in [
            "fn fire_ramp(",
            "fn fire_hue(",
            "fn snoise(",
            "fn snoise_vec3(",
            "fn perlin_noise(",
            "const ROYGBIV_GREEN: f32 = 2.0943951;",
            "smoothstep(0.05, 0.35, t)",
            "const SPHERE_RADIUS: f32 = 0.9;",
            "const KEY_LIGHT: vec3<f32> = vec3<f32>(0.0, 80.0, 0.0);",
        ] {
            assert!(common.contains(needle), "common.wgsl lost {needle}");
        }

        let advect = include_str!("passes/advect_velocity.wgsl");
        for needle in [
            "smoothstep(0.0, 0.08, min(edge.x, min(edge.y, edge.z)))",
            "max(1.0 - u.turb2.y * dt, 0.0)",
            "u.sphere_vel.xyz * u.emit2.x + wind_turbulence",
            "(temperature * u.turb.x - density * u.turb.y) * u.world.y",
        ] {
            assert!(advect.contains(needle), "advect_velocity lost {needle}");
        }

        let dye = include_str!("passes/advect_dye.wgsl");
        for needle in [
            "max(1.0 - u.turb2.w * dt, 0.0)",
            "max(1.0 - u.turb2.z * dt, 0.0)",
            "clamp(temperature, 0.0, 12.0)",
            "if (density <= 0.01)",
        ] {
            assert!(dye.contains(needle), "advect_dye lost {needle}");
        }

        let emit = include_str!("passes/emit.wgsl");
        for needle in [
            "u.emit.x * (1.0 / 120.0)",
            "u.emit.y * (1.0 / 120.0)",
            "flicker * 0.85 + 0.15",
        ] {
            assert!(emit.contains(needle), "emit lost {needle}");
        }

        let raymarch = include_str!("passes/raymarch.wgsl");
        for needle in [
            "const RAYMARCH_STEPS: u32 = 16u;",
            "const SHADOW_STEP_SIZE: f32 = 0.35;",
            "exp(-tau * 0.25) * 0.5",
            "1.0 - exp(-tau * 2.0)",
            "0.618033988749895",
            "400.0 / max(dot(light_delta, light_delta), 1e-3)",
            "let fire_power = 6.0 - u.fire_mid.w;",
        ] {
            assert!(raymarch.contains(needle), "raymarch lost {needle}");
        }

        let combine = include_str!("passes/combine.wgsl");
        assert!(combine.contains("max(scene, adjusted) + adjusted"));
        let blur = include_str!("passes/blur.wgsl");
        assert!(blur.contains("const AXIAL: f32 = 0.1353352832366127;"));
    }
}
