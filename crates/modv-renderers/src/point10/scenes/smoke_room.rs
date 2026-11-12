//! SmokeRoom — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/SmokeRoom.js`.
//!
//! The JS scene is **WebGPU/TSL**: a real `150 x 150 x 300` velocity/dye
//! simulation (seven compute passes per 1/120 s step) rendered by a 16-step
//! `VolumeNodeMaterial` raymarch inside a glass room, with a lava sphere at the
//! centre as the emitter. The music pulses the emission (`uFireSize` from the
//! hi-hat and snare bands), rotates the fire hue on bass phrase crests, and
//! drives the bloom.
//!
//! # The volume is real
//!
//! Unlike the earlier puff approximation, this scene configures
//! [`crate::point10::volume::VolumeSim`] at the JS grid dimensions and runs the
//! same passes in the same order (curl noise once at init; advect velocity,
//! divergence, two Jacobi iterations, project, advect dye, emit per step) and
//! the same 16-step raymarch. See the `volume` module docs for the grid, the
//! ping-pong texture layout and the transfer function.
//!
//! # Ported exactly
//!
//! * the room (`:60-88`, `:130-185`): `VOLUME_WORLD_SIZE 12 x 12 x 24`,
//!   `GLASS_PADDING 0.6`, `GLASS_TOP_MARGIN 0.75`, and the bottom-anchored
//!   `glassFit`/`requiredGlassTop` stretch ([`room`]);
//! * the emitter (`:73`, `:326-338`, `:1022-1073`, `:1332-1349`): a
//!   `SphereGeometry(0.9, 64, 40)` at the grid centre, the CPU Perlin-driven
//!   flame height / sway / flicker / colour noise, and the 3-second fade-in;
//! * the audio mapping (`:340-374`): bass = mean of the first five bins with
//!   `dt * 8` smoothing, hi-hats = mean of the top 15% with `dt * 10`, snares =
//!   bins 2%..13% with `dt * 12`, `uFireSize = 1 + hi * 2.5 + snare * 8`, the
//!   `dropColorPulse` hue flip and its `dt * 1.2` shortest-arc ease;
//! * the simulation clock (`:1351-1392`): the 1/120 s accumulator capped at
//!   eight steps, `uTurbulence = 3.2 / sqrt(speed)`, `uDissipation =
//!   1 / smokeLifespan` (`0` past 100 s), `uCooling = 1 / fireLifespan`;
//! * the render chain (`:1235-1276`, `:1394-1428`): ACES filmic with exposure
//!   2, bloom `min(0.12 + bass * 2.2, 2.8)` at threshold 0.5 / radius 1, the
//!   `scene.max(volume) + volume` combine, and `alpha = max(rgb)`;
//! * the prop binder (`sceneProps.js:18-38`): `smokeDensity`, `fireIntensity`,
//!   the two lifespans, `simulationSpeed`, `boxScale`, `boxXYScale`.
//!
//! # Deviations
//!
//! * **The glass and the lava sphere keep their procedural WGSL materials.**
//!   The JS uses `MeshStandardMaterial` plus a PMREM studio environment and a
//!   shadow-mapped point light; the port keeps the same tint/opacity/fire-light
//!   look without a PMREM pass or shadow maps (see the shaders below).
//! * The volume's shadow-map projection onto the glass/logo (three's
//!   `volumeCastShadow` + the shadow-blended volume mesh) is not drawn: there
//!   is no shadow map in this renderer. The *shadow march inside* the
//!   raymarch's scattering term is ported (that is what shades the smoke).
//! * The lava sphere is the same `64 x 40` UV sphere as the JS, but its
//!   emissive texture (`moon_04_diff_1k.jpg`) is not sampled: the base colour
//!   is black and the emissive is entirely procedural, which is also how the
//!   JS reads at these sizes (the diffuse map is multiplied by a black base).

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::AudioBridge;
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, depth_state, identity, scale, translation,
};
use crate::point10::scenes::fire_common::build_sphere as build_uv_sphere;
use crate::point10::shared::phrase_color::{DropColorState, drop_color_pulse, ease_phrase_hue};
use crate::point10::shared::random::seeded_rand;
use crate::point10::volume::room::{
    BLOOM_BASE, BLOOM_MAX, BLOOM_PER_BASS, CAMERA_FAR, CAMERA_FOV, CAMERA_NEAR, CAMERA_START,
    CLEAR_ALPHA, FIRE_END_HEX, FIRE_MID_HEX, FIRE_START_HEX, FireTemporal, GLASS_HALF_X,
    GLASS_HALF_Y, GLASS_HALF_Z, ImprovedNoise, SPHERE_RADIUS, SimulationClock, emitter_glow,
    fire_colour, fire_size, fire_temporal, glass_fit, room_top, smoke_bass, snare_band,
    treble_band, volume_point_light,
};
use crate::point10::volume::stage::{Material, VolumeStage, volumetric_size};
use crate::point10::volume::{VolumeGrid, VolumeSim, VolumeUniforms};
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

/// `uEmitDensity` default (`SmokeRoom.js:490`).
pub const EMIT_DENSITY: f32 = 28.0;
/// `uEmitTemperature` default (`SmokeRoom.js:491`).
pub const EMIT_TEMPERATURE: f32 = 22.0;
/// `uFireIntensity` default (`SmokeRoom.js:498`).
pub const FIRE_INTENSITY: f32 = 40.0;
/// `uFireGlowSpread` (`SmokeRoom.js:499`).
pub const FIRE_GLOW_SPREAD: f32 = 5.0;
/// `uBuoyancy` (`SmokeRoom.js:483`).
pub const BUOYANCY: f32 = 3.0;
/// `uWeight` (`SmokeRoom.js:484`).
pub const WEIGHT: f32 = 0.15;
/// `uTurbulenceDecay` (`SmokeRoom.js:485`).
pub const TURBULENCE_DECAY: f32 = 0.1;
/// `uTurbFrequency` (`SmokeRoom.js:486`).
pub const TURB_FREQUENCY: f32 = 10.0;
/// `uVelDamping` (`SmokeRoom.js:487`).
pub const VEL_DAMPING: f32 = 0.25;
/// `uWindStrength` (`SmokeRoom.js:496`).
pub const WIND_STRENGTH: f32 = 6.5;
/// `uShadowAbsorption` (`SmokeRoom.js:500`).
pub const SHADOW_ABSORPTION: f32 = 2.0;
/// `uShadowAmbient` (`SmokeRoom.js:501`).
pub const SHADOW_AMBIENT: f32 = 0.5;
/// `uMultiScattering` (`SmokeRoom.js:507`).
pub const MULTI_SCATTERING: f32 = 1.0;
/// `uPowderStrength` (`SmokeRoom.js:507`).
pub const POWDER_STRENGTH: f32 = 0.59;

/// `this._simSpeed` default (`SmokeRoom.js:1324`).
pub const SIMULATION_SPEED: f32 = 1.2;
/// `this._smokeLifespan` (`SmokeRoom.js:1325`).
pub const SMOKE_LIFESPAN: f32 = 3.5;
/// `this._fireLifespan` (`SmokeRoom.js:1326`).
pub const FIRE_LIFESPAN: f32 = 1.3;

/// `SMOKE_ROOM_PROPS` (`sceneProps.js:18-38`).
const SMOKE_ROOM_PROPS: &[SceneProp] = &[
    SceneProp::float("smokeDensity", "Smoke Density", 28.0, 0.0, 100.0, 0.1),
    SceneProp::float("fireIntensity", "Fire Intensity", 40.0, 0.0, 100.0, 0.1),
    SceneProp::float("smokeLifespan", "Smoke Lifespan", 3.5, 0.1, 20.0, 0.1),
    SceneProp::float("fireLifespan", "Fire Lifespan", 1.3, 0.1, 5.0, 0.1),
    SceneProp::float("simulationSpeed", "Simulation Speed", 1.2, 0.1, 4.0, 0.01),
    SceneProp::float("boxScale", "Box Scale", 1.0, 0.5, 5.0, 0.05),
    SceneProp::float("boxXYScale", "Box Width & Height", 1.0, 0.5, 5.0, 0.05),
];

/// The glass shader's uniform block (176 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GlassUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// rgb = fire colour, w = fire intensity.
    fire: [f32; 4],
    /// x opacity, y time, z bass, w unused.
    params: [f32; 4],
}

/// The emitter shader's uniform block (176 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct EmitterUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// rgb = fire colour, w = glow scale.
    glow: [f32; 4],
    /// x time, y flame height, z flicker, w unused.
    params: [f32; 4],
}

/// GPU resources built in `init`.
struct SmokeRoomGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    sim: VolumeSim,
    stage: VolumeStage,
    glass: Material,
    emitter: Material,
    glass_mesh: MeshGpu,
    emitter_mesh: MeshGpu,
}

/// SmokeRoom scene.
pub struct SmokeRoom {
    /// `this._bass`.
    bass: f32,
    /// `this._hi`.
    hi: f32,
    /// `this._snare`.
    snare: f32,
    /// `uFireSize`.
    fire_size: f32,
    /// `this._uFireHue`.
    fire_hue: f32,
    /// `this._pulse`.
    pulse: DropColorState,
    /// The seed stream behind the drop detector.
    seed: u32,
    /// `this.time` (`BaseSceneWebGPU.update` accumulates the frame delta).
    time: f32,
    /// The frame delta handed to `update`.
    dt: f32,
    /// `this._simAccumulator` / `this._simulationTime`.
    clock: SimulationClock,
    /// `this._cpuNoise`.
    noise: ImprovedNoise,
    /// The raymarch dither's `frameId`.
    frame: u32,
    /// Props.
    smoke_density: f32,
    fire_intensity: f32,
    smoke_lifespan: f32,
    fire_lifespan: f32,
    simulation_speed: f32,
    box_scale: f32,
    box_xy_scale: f32,
    /// The glass fit (`glass.scale.y` / `glass.position.y`).
    glass_height: f32,
    glass_centre_y: f32,
    camera_aspect: f32,
    size: (u32, u32),
    /// The simulation grid (`VolumeGrid::JS` unless a test scales it).
    grid: VolumeGrid,
    gpu: Option<SmokeRoomGpu>,
}

impl Default for SmokeRoom {
    fn default() -> Self {
        Self::new()
    }
}

impl SmokeRoom {
    /// A scene with a different simulation grid (tests and the offline
    /// reference renderer use a smaller grid; the scene defaults to the JS
    /// `150 x 150 x 300`).
    pub fn with_grid(grid: VolumeGrid) -> Self {
        let mut scene = Self::new();
        scene.grid = grid;
        scene
    }

    /// Create the scene (`SmokeRoom.js:99-127` defaults).
    pub fn new() -> Self {
        Self {
            bass: 0.0,
            hi: 0.0,
            snare: 0.0,
            fire_size: 1.0,
            fire_hue: 0.0,
            pulse: DropColorState::new(),
            seed: 0,
            time: 0.0,
            dt: 0.0,
            clock: SimulationClock::default(),
            noise: ImprovedNoise,
            frame: 0,
            smoke_density: EMIT_DENSITY,
            fire_intensity: FIRE_INTENSITY,
            smoke_lifespan: SMOKE_LIFESPAN,
            fire_lifespan: FIRE_LIFESPAN,
            simulation_speed: SIMULATION_SPEED,
            box_scale: 1.0,
            box_xy_scale: 1.0,
            glass_height: GLASS_HALF_Y * 2.0,
            glass_centre_y: 0.0,
            camera_aspect: 1.0,
            size: (1, 1),
            grid: VolumeGrid::JS,
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:18-38`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(SMOKE_ROOM_PROPS, props) {
            match key {
                "smokeDensity" => self.smoke_density = value,
                "fireIntensity" => self.fire_intensity = value,
                "smokeLifespan" => self.smoke_lifespan = value,
                "fireLifespan" => self.fire_lifespan = value,
                "simulationSpeed" => self.simulation_speed = value,
                "boxScale" => self.box_scale = value,
                "boxXYScale" => self.box_xy_scale = value,
                _ => {}
            }
        }
        self.refresh_glass_fit();
    }

    /// `uFireSize` (`:364-365`).
    pub fn fire_size(&self) -> f32 {
        self.fire_size
    }

    /// `uFireHue`.
    pub fn fire_hue(&self) -> f32 {
        self.fire_hue
    }

    /// `this._bass`.
    pub fn bass(&self) -> f32 {
        self.bass
    }

    /// The simulation clock's accumulated time.
    pub fn simulation_time(&self) -> f32 {
        self.clock.time()
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// The smoke-room bloom strength (`:1394-1400`).
    pub fn bloom_strength(&self) -> f32 {
        (BLOOM_BASE + self.bass * BLOOM_PER_BASS).min(BLOOM_MAX)
    }

    /// The volume grid the scene runs (`VolumeGrid::JS` by default).
    pub fn grid(&self) -> VolumeGrid {
        self.grid
    }

    /// The fire light the glass reflects (`:499`, `:953`).
    pub fn fire_light(&self) -> [f32; 4] {
        let colour = fire_colour(0.72, self.fire_hue);
        let intensity = (self.fire_intensity / 40.0).max(0.0) * 2.0;
        [
            colour[0] * intensity * 0.15,
            colour[1] * intensity * 0.15,
            colour[2] * intensity * 0.15,
            intensity * 0.15,
        ]
    }

    /// The lava emitter's emissive scale for a temporal state.
    pub fn emitter_glow(&self, temporal: &FireTemporal) -> f32 {
        emitter_glow(
            EMIT_TEMPERATURE,
            self.smoke_density,
            self.fire_intensity,
            temporal,
        )
    }

    fn view_proj(&self) -> Mat4 {
        crate::point10::scenes::cube_mesh::mat4_mul(
            &crate::point10::scenes::cube_mesh::perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &crate::point10::scenes::cube_mesh::look_at(
                CAMERA_START,
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ),
        )
    }

    fn glass_model(&self) -> Mat4 {
        let model = translation(0.0, self.glass_centre_y, 0.0);
        crate::point10::scenes::cube_mesh::mat4_mul(
            &model,
            &scale(
                GLASS_HALF_X * 2.0 * self.box_scale * self.box_xy_scale,
                self.glass_height,
                GLASS_HALF_Z * 2.0 * self.box_scale,
            ),
        )
    }

    fn refresh_glass_fit(&mut self) {
        let top = room_top(CAMERA_START, self.box_scale, self.box_xy_scale);
        let (height, centre_y) = glass_fit(top, self.box_scale, self.box_xy_scale);
        self.glass_height = height;
        self.glass_centre_y = centre_y;
    }

    fn glass_uniforms(&self) -> GlassUniforms {
        let fire = self.fire_light();
        GlassUniforms {
            view_proj: self.view_proj(),
            model: self.glass_model(),
            camera_pos: [CAMERA_START[0], CAMERA_START[1], CAMERA_START[2], 1.0],
            fire,
            params: [0.35, self.time, self.bass, 0.0],
        }
    }

    fn emitter_uniforms(&self, temporal: &FireTemporal) -> EmitterUniforms {
        let colour = fire_colour(0.55, self.fire_hue);
        EmitterUniforms {
            view_proj: self.view_proj(),
            model: identity(),
            camera_pos: [CAMERA_START[0], CAMERA_START[1], CAMERA_START[2], 1.0],
            glow: [colour[0], colour[1], colour[2], self.emitter_glow(temporal)],
            params: [temporal.time, temporal.flame_height, temporal.flicker, 0.0],
        }
    }

    /// The volume uniforms for one simulation step.
    ///
    /// `boxScale`/`boxXYScale` scale the world size and the grid origin (the
    /// JS's `_applyBoxScale`), while the emitter radius follows `boxScale`
    /// alone (`SmokeRoom.js:632`).
    fn volume_uniforms(
        &self,
        temporal: &FireTemporal,
        tick: &crate::point10::volume::room::SimTick,
    ) -> VolumeUniforms {
        let scale = self.box_scale;
        let xy = self.box_xy_scale;
        let world = [12.0 * scale * xy, 12.0 * scale * xy, 24.0 * scale];
        let mut uniforms = VolumeUniforms::for_grid(self.grid());
        uniforms.grid[3] = tick.step;
        uniforms.world = [world[0], world[1], world[2], temporal.time];
        uniforms.origin = [0.0, -world[1] * 0.5, 0.0, scale];
        uniforms.sphere_pos = [0.0, 0.0, 0.0, 0.0];
        uniforms.sphere_vel = [0.0, 0.0, 0.0, 0.25];
        uniforms.camera = [
            CAMERA_START[0],
            CAMERA_START[1],
            CAMERA_START[2],
            self.frame as f32,
        ];
        uniforms.set_camera(CAMERA_START, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        uniform_colours(&mut uniforms);
        uniforms.fire_start[3] = self.fire_intensity;
        uniforms.fire_mid[3] = FIRE_GLOW_SPREAD;
        uniforms.fire_end[3] = self.fire_hue;
        uniforms.turb = [BUOYANCY, WEIGHT, tick.turbulence, TURBULENCE_DECAY];
        uniforms.turb2 = [TURB_FREQUENCY, VEL_DAMPING, tick.cooling, tick.dissipation];
        uniforms.emit = [self.smoke_density, EMIT_TEMPERATURE, self.fire_size, 1.0];
        uniforms.emit2 = [WIND_STRENGTH, temporal.flicker, temporal.flame_height, 0.0];
        uniforms.light = [
            SHADOW_ABSORPTION,
            SHADOW_AMBIENT,
            MULTI_SCATTERING,
            POWDER_STRENGTH,
        ];
        uniforms.point_light = {
            let colour = volume_point_light(
                temporal,
                EMIT_TEMPERATURE,
                self.smoke_density,
                self.fire_intensity,
                self.fire_hue,
            );
            [colour[0], colour[1], colour[2], 0.0]
        };
        let (volume_width, volume_height) = volumetric_size(self.size.0, self.size.1);
        uniforms.view = [
            (CAMERA_FOV.to_radians() * 0.5).tan(),
            self.camera_aspect,
            volume_width as f32,
            volume_height as f32,
        ];
        uniforms.set_emitter_model(identity());
        uniforms
    }
}

/// Stamp the fire ramp stops (linear-sRGB, as `THREE.Color(hex)` converts them)
/// onto the volume uniforms. The intensity slots are filled by the caller.
fn uniform_colours(uniforms: &mut VolumeUniforms) {
    let start = crate::point10::shared::phrase_color::srgb_hex_to_linear(FIRE_START_HEX);
    let mid = crate::point10::shared::phrase_color::srgb_hex_to_linear(FIRE_MID_HEX);
    let end = crate::point10::shared::phrase_color::srgb_hex_to_linear(FIRE_END_HEX);
    uniforms.fire_start = [start[0], start[1], start[2], 40.0];
    uniforms.fire_mid = [mid[0], mid[1], mid[2], 5.0];
    uniforms.fire_end = [end[0], end[1], end[2], 0.0];
}

#[cfg(test)]
impl SmokeRoom {
    /// The live simulation, for the debug readback in the reference renderer.
    pub(crate) fn debug_sim(&self) -> Option<&crate::point10::volume::VolumeSim> {
        self.gpu.as_ref().map(|gpu| &gpu.sim)
    }
}

impl Point10Scene for SmokeRoom {
    fn name(&self) -> &'static str {
        "Point10 Smoke Room"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width.max(1), ctx.height.max(1));
        self.camera_aspect = ctx.aspect();
        self.refresh_glass_fit();

        // The emitter mesh (also the simulation's seed geometry): the JS's
        // `SphereGeometry(SPHERE_RADIUS, 64, 40)`.
        let sphere = build_uv_sphere(SPHERE_RADIUS, 64, 40);
        let emitter_positions: Vec<[f32; 3]> = sphere
            .vertices
            .iter()
            .map(|vertex| vertex.position)
            .collect();

        let sim = VolumeSim::new(&device, &queue, self.grid(), &emitter_positions);
        let stage = VolumeStage::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);

        let glass = Material::new(
            &device,
            "point10-smoke-room-glass",
            GLASS_WGSL,
            std::mem::size_of::<GlassUniforms>() as u64,
            Some(crate::point10::scenes::cube_mesh::Vertex::layout()),
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            Some(depth_state()),
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        let emitter = Material::new(
            &device,
            "point10-smoke-room-emitter",
            &emitter_program(),
            std::mem::size_of::<EmitterUniforms>() as u64,
            Some(crate::point10::scenes::cube_mesh::Vertex::layout()),
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            Some(depth_state()),
            None,
        );

        let glass_mesh = MeshGpu::new(
            &device,
            "point10-smoke-room-glass-mesh",
            &build_box(1.0, 1.0, 1.0),
        );
        let emitter_mesh = MeshGpu::new(&device, "point10-smoke-room-emitter-mesh", &sphere);

        self.gpu = Some(SmokeRoomGpu {
            device,
            queue,
            sim,
            stage,
            glass,
            emitter,
            glass_mesh,
            emitter_mesh,
        });
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        self.refresh_glass_fit();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let device = gpu.device.clone();
        let queue = gpu.queue.clone();
        let (width, height) = self.size;
        gpu.stage.resize(&device, &queue, width, height);
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.dt = dt;
        self.time += dt;

        if audio.is_available() {
            let freq_mag = &audio.freq_mag;
            let bass = smoke_bass(freq_mag);
            self.bass += (bass - self.bass) * (dt * 8.0).min(1.0);

            let hi = treble_band(freq_mag);
            let snare = snare_band(freq_mag);
            self.hi += (hi - self.hi) * (dt * 10.0).min(1.0);
            self.snare += (snare - self.snare) * (dt * 12.0).min(1.0);
            self.fire_size = fire_size(self.hi, self.snare);

            // The drop detector + shortest-arc hue ease (`:370-373`).
            let sample = seeded_rand(0, self.seed as i32);
            if drop_color_pulse(&mut self.pulse, self.bass, dt, sample) {
                self.seed = self.seed.wrapping_add(1);
            }
            self.fire_hue = ease_phrase_hue(self.fire_hue, self.pulse.hue, dt, 1.2);
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let tick = self.clock.pending(
            frame_dt(self.dt),
            self.simulation_speed,
            self.smoke_lifespan,
            self.fire_lifespan,
        );

        // The JS refreshes the temporal uniforms *per step*; one submission
        // per step is what makes that possible without a uniform ring buffer.
        let mut steps: Vec<VolumeUniforms> = Vec::with_capacity(tick.steps as usize);
        let mut temporal = fire_temporal(&self.noise, self.clock.time());
        for index in 0..tick.steps {
            let step_temporal = fire_temporal(&self.noise, tick.time_of(index));
            steps.push(self.volume_uniforms(&step_temporal, &tick));
            temporal = step_temporal;
        }
        // With no step this frame, still refresh the uniforms so the emitter
        // and glass keep animating.
        let idle_uniforms = if steps.is_empty() {
            Some(self.volume_uniforms(&temporal, &tick))
        } else {
            None
        };
        let glass_uniforms = self.glass_uniforms();
        let emitter_uniforms = self.emitter_uniforms(&temporal);
        let bloom_strength = self.bloom_strength();
        self.frame = self.frame.wrapping_add(1);

        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let device = gpu.device.clone();
        let queue = gpu.queue.clone();

        if let Some(uniforms) = idle_uniforms {
            gpu.sim.write(&queue, &uniforms);
        }
        for uniforms in &steps {
            let mut sim_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("point10-smoke-room-sim"),
            });
            gpu.sim.write(&queue, uniforms);
            gpu.sim.step(&mut sim_encoder);
            queue.submit(Some(sim_encoder.finish()));
        }

        gpu.glass.write(&queue, bytemuck::bytes_of(&glass_uniforms));
        gpu.emitter
            .write(&queue, bytemuck::bytes_of(&emitter_uniforms));

        {
            let mut pass = gpu.stage.begin_scene(encoder);
            // Opaque emitter first, then the transparent glass over it (the
            // order three's renderer uses).
            gpu.emitter.draw(&mut pass, &gpu.emitter_mesh);
            gpu.glass.draw(&mut pass, &gpu.glass_mesh);
        }

        gpu.stage
            .encode_post(encoder, &gpu.sim, bloom_strength, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(SMOKE_ROOM_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 Smoke Room", "3d-scenes")
    }
}

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

const GLASS_WGSL: &str = r#"
struct GlassUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  fire: vec4<f32>,
  // x opacity, y time, z bass
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: GlassUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) world: vec3<f32>,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
) -> VsOut {
  let world = u.model * vec4<f32>(position, 1.0);
  var out: VsOut;
  out.position = u.view_proj * world;
  out.normal = normalize((u.model * vec4<f32>(normal, 0.0)).xyz);
  out.world = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.world);
  let fresnel = pow(1.0 - clamp(abs(dot(normal, view_dir)), 0.0, 1.0), 3.0);

  // `MeshStandardMaterial({ color: 0x9fc4e8, metalness: 0.1, roughness: 0.15,
  // envMapIntensity: 2.5 })` against the JS's dark studio (a few bright bars on
  // black, PMREM'd). A standard material reads that as almost-black glass with
  // a specular band and a fresnel rim — not a lit sheet — so the env term is
  // kept very dark and the highlight comes from the overhead bar.
  let tint = vec3<f32>(0.62, 0.77, 0.91);
  let env = mix(vec3<f32>(0.0015, 0.002, 0.004), vec3<f32>(0.02, 0.028, 0.045), normal.y * 0.5 + 0.5);
  let key_dir = normalize(vec3<f32>(0.0, 1.0, 0.35));
  let highlight = pow(max(dot(normal, key_dir), 0.0), 6.0) * 0.55;

  let to_fire = -in.world;
  let fire_distance = length(to_fire);
  let fire_dir = to_fire / max(fire_distance, 0.001);
  let fire_term = max(dot(normal, fire_dir), 0.0) * u.fire.a / (1.0 + fire_distance * fire_distance);

  let colour = tint * (env * 2.5 + highlight) + u.fire.rgb * fire_term + tint * fresnel * 0.35;
  return vec4<f32>(colour, u.params.x + fresnel * 0.15);
}
"#;

const EMITTER_WGSL: &str = r#"
struct EmitterUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // rgb = fire colour, w = glow scale
  glow: vec4<f32>,
  // x time, y flame height, z flicker
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: EmitterUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) local: vec3<f32>,
  @location(2) world: vec3<f32>,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
) -> VsOut {
  let world = u.model * vec4<f32>(position, 1.0);
  var out: VsOut;
  out.position = u.view_proj * world;
  out.normal = normalize(normal);
  out.local = position;
  out.world = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // `positionLocal.mul(0.5)` + the three-octave mx_noise flow (`:1036-1044`).
  let p = in.local * 0.5;
  let flow = vec3<f32>(0.0, -u.params.x, 0.0);
  let n1 = p9_noise(p + flow) * 0.5 + 0.5;
  let p2 = p * 2.0 - flow * 1.5;
  let n2 = p9_noise(p2 + vec3<f32>(n1 * 0.4)) * 0.5 + 0.5;
  let p3 = p * 4.0 + flow * 2.5;
  let n3 = p9_noise(p3) * 0.5 + 0.5;
  let noise = n1 * 0.5 + n2 * 0.35 + n3 * 0.15;
  let lava_t = clamp(pow(noise, 2.5), 0.0, 1.0);
  // The CPU already applied the fire ramp and the hue at `lava_t + 0.1`; the
  // noise keeps the lava's mottling, the glow scale carries the brightness.
  let emissive = u.glow.rgb * u.glow.w * (0.55 + lava_t * 0.9) * u.params.z;
  // A dark base under the emissive (`color: 0x000000`, roughness 1).
  return vec4<f32>(emissive, 1.0);
}
"#;

fn emitter_program() -> String {
    format!(
        "{}\n{EMITTER_WGSL}",
        crate::point10::shared::p9_glsl::P9_COMMON_WGSL
    )
}

/// The glass/emitter/voxel shader sources, for the naga validation test.
#[cfg(test)]
fn shader_programs() -> Vec<(&'static str, String)> {
    vec![
        ("glass", GLASS_WGSL.to_string()),
        ("emitter", emitter_program()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::{AUDIO_SAMPLES, BandPayload};
    use crate::point10::volume::room::{BASS_BINS, FIRE_HAT_K, FIRE_SNARE_K, box_half_extents};

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn room_extents_and_emitter_follow_the_box_scale() {
        assert_eq!((GLASS_HALF_X, GLASS_HALF_Y, GLASS_HALF_Z), (6.3, 6.3, 12.3));
        assert_eq!(box_half_extents(1.0, 1.0), [6.3, 6.3, 12.3]);
        let scaled = box_half_extents(2.5, 1.6);
        close(scaled[0], 6.3 * 2.5 * 1.6, 1e-5);
        close(scaled[2], 12.3 * 2.5, 1e-5);
        assert_eq!(SPHERE_RADIUS, 0.9);
    }

    #[test]
    fn the_fire_size_ramps_on_hats_and_snares() {
        assert_eq!(fire_size(0.0, 0.0), 1.0);
        close(fire_size(0.1, 0.0), 1.0 + 0.1 * FIRE_HAT_K, 1e-6);
        close(fire_size(0.0, 0.1), 1.0 + 0.1 * FIRE_SNARE_K, 1e-6);
        assert!(fire_size(0.1, 0.1) > 1.8);
    }

    #[test]
    fn bands_use_the_js_bin_ranges() {
        let mut freq = [0.0_f32; AUDIO_SAMPLES];
        for (index, value) in freq.iter_mut().enumerate().take(BASS_BINS) {
            *value = 0.5 + index as f32 * 0.1;
        }
        close(
            smoke_bass(&freq),
            (0..BASS_BINS).map(|i| 0.5 + i as f32 * 0.1).sum::<f32>() / BASS_BINS as f32,
            1e-6,
        );
        let snare_lo = (AUDIO_SAMPLES as f32 * 0.02) as usize;
        freq[snare_lo] = 1.0;
        assert!(snare_band(&freq) > 0.0);
        freq[AUDIO_SAMPLES - 1] = 1.0;
        assert!(treble_band(&freq) > 0.0);
    }

    #[test]
    fn emitter_glow_scales_with_temperature_density_and_fire_intensity() {
        let temporal = fire_temporal(&ImprovedNoise, 10.0);
        let base = emitter_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, &temporal);
        assert!(base > 0.0 && base < 1.5, "glow {base}");
        // The fade-in is smoothstep(t / 3), so t = 0 is black.
        let early = fire_temporal(&ImprovedNoise, 0.0);
        assert_eq!(
            emitter_glow(EMIT_TEMPERATURE, EMIT_DENSITY, FIRE_INTENSITY, &early),
            0.0
        );
        assert!(
            emitter_glow(
                EMIT_TEMPERATURE,
                EMIT_DENSITY,
                FIRE_INTENSITY * 2.0,
                &temporal
            ) > base
        );
        assert!(
            emitter_glow(
                EMIT_TEMPERATURE,
                EMIT_DENSITY * 2.0,
                FIRE_INTENSITY,
                &temporal
            ) > base
        );
    }

    #[test]
    fn the_scene_runs_the_js_grid_and_simulation_clock() {
        let scene = SmokeRoom::new();
        assert_eq!(scene.grid(), VolumeGrid::JS);
        assert_eq!(
            (scene.grid().x, scene.grid().y, scene.grid().z),
            (150, 150, 300)
        );
        assert_eq!(scene.grid().cell_count(), 6_750_000);
        assert_eq!((SMOKE_LIFESPAN, FIRE_LIFESPAN), (3.5, 1.3));
        assert_eq!(SIMULATION_SPEED, 1.2);
        assert_eq!(scene.clear_alpha(), 0.0);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!((BLOOM_BASE, BLOOM_PER_BASS, BLOOM_MAX), (0.12, 2.2, 2.8));
        assert_eq!(
            (EMIT_DENSITY, EMIT_TEMPERATURE, FIRE_INTENSITY),
            (28.0, 22.0, 40.0)
        );
        assert_eq!((FIRE_HAT_K, FIRE_SNARE_K), (2.5, 8.0));
        assert_eq!((BUOYANCY, WEIGHT), (3.0, 0.15));
        assert_eq!(
            (TURBULENCE_DECAY, TURB_FREQUENCY, VEL_DAMPING),
            (0.1, 10.0, 0.25)
        );
        assert_eq!(
            (
                SHADOW_ABSORPTION,
                SHADOW_AMBIENT,
                MULTI_SCATTERING,
                POWDER_STRENGTH
            ),
            (2.0, 0.5, 1.0, 0.59)
        );
        assert_eq!((FIRE_GLOW_SPREAD, WIND_STRENGTH), (5.0, 6.5));
        assert_eq!(CAMERA_START, [14.0, 5.5, 4.4]);
        assert_eq!(FIRE_START_HEX, 0xffe68c);
        assert_eq!(FIRE_MID_HEX, 0xff7305);
        assert_eq!(FIRE_END_HEX, 0xff0000);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<GlassUniforms>(), 176);
        assert_eq!(std::mem::size_of::<EmitterUniforms>(), 176);
    }

    #[test]
    fn the_volume_uniforms_carry_the_scene_state() {
        let mut scene = SmokeRoom::new();
        scene.box_scale = 2.0;
        scene.box_xy_scale = 1.5;
        scene.fire_size = 2.0;
        scene.camera_aspect = 16.0 / 9.0;
        scene.size = (256, 256);
        let temporal = fire_temporal(&ImprovedNoise, 10.0);
        let tick = crate::point10::volume::room::SimTick {
            steps: 2,
            step: 0.01,
            start_time: 0.0,
            turbulence: 3.2 / 1.2_f32.sqrt(),
            dissipation: 1.0 / 3.5,
            cooling: 1.0 / 1.3,
        };
        let uniforms = scene.volume_uniforms(&temporal, &tick);

        assert_eq!(uniforms.grid[0..3], [150.0, 150.0, 300.0]);
        close(uniforms.grid[3], 0.01, 1e-6);
        close(uniforms.world[0], 12.0 * 2.0 * 1.5, 1e-5);
        close(uniforms.world[1], 12.0 * 2.0 * 1.5, 1e-5);
        close(uniforms.world[2], 24.0 * 2.0, 1e-5);
        close(uniforms.origin[1], -uniforms.world[1] * 0.5, 1e-4);
        close(uniforms.origin[3], 2.0, 1e-6);
        assert!((uniforms.camera[0] - CAMERA_START[0]).abs() < 1e-6);
        close(uniforms.emit[0], 28.0, 1e-6);
        close(uniforms.emit[2], 2.0, 1e-6);
        close(uniforms.turb[2], tick.turbulence, 1e-6);
        close(uniforms.turb2[2], tick.cooling, 1e-6);
        close(uniforms.turb2[3], tick.dissipation, 1e-6);
        // Half-resolution raymarch target.
        assert_eq!(uniforms.view[2], 128.0);
        assert_eq!(uniforms.view[3], 128.0);
        close(uniforms.view[0], (60.0_f32.to_radians() * 0.5).tan(), 1e-6);
        close(uniforms.view[1], 16.0 / 9.0, 1e-6);
        // The emitter model is the identity (the sphere sits at the origin).
        assert_eq!(uniforms.emitter_model, identity());
        // Point light: dark at t = 0? No — the temporal is t = 10, so lit.
        assert!(uniforms.point_light[0] > 0.0);
    }

    #[test]
    fn props_and_meta_match_the_js_registry() {
        let scene = SmokeRoom::new();
        assert_eq!(scene.meta().name, "Point10 Smoke Room");
        assert_eq!(scene.meta().category, "3d-scenes");
        assert_eq!(scene.clear_alpha(), 0.0);
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 7..],
            &[
                "smokeDensity",
                "fireIntensity",
                "smokeLifespan",
                "fireLifespan",
                "simulationSpeed",
                "boxScale",
                "boxXYScale",
            ]
        );
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn update_maps_the_audio_and_props() {
        let mut scene = SmokeRoom::new();
        scene.apply_props(|key| match key {
            "fireIntensity" => Some(80.0),
            "boxScale" => Some(2.0),
            "boxXYScale" => Some(1.5),
            "smokeDensity" => Some(56.0),
            _ => None,
        });
        assert_eq!(scene.smoke_density, 56.0);
        assert_eq!(scene.fire_intensity, 80.0);
        assert_eq!(scene.box_scale, 2.0);
        assert_eq!(scene.box_xy_scale, 1.5);

        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag = [0.5; AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());

        scene.update(1.0 / 60.0, &bridge);
        assert!(scene.bass() > 0.0);
        assert!(scene.fire_size() > 1.0);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in shader_programs() {
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
    fn shaders_keep_the_ported_maths() {
        assert!(GLASS_WGSL.contains("u.params.x + fresnel * 0.15"));
        let emitter = emitter_program();
        for needle in [
            "p9_noise(p + flow)",
            "p9_noise(p2 + vec3<f32>(n1 * 0.4))",
            "pow(noise, 2.5)",
        ] {
            assert!(emitter.contains(needle), "emitter lost {needle}");
        }
    }
}
