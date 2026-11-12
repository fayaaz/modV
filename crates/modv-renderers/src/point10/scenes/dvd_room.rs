//! DvdRoom — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/DvdRoom.js`.
//!
//! The JS scene inherits `SmokeRoom` and swaps the lava sphere for a bouncing
//! extruded DVD logo that *is* the smoke emitter. The room, the simulation
//! grid, the compute passes and the raymarch are identical, so this scene
//! configures the same [`VolumeSim`] and [`VolumeStage`] and only changes the
//! emitter, the lights and the logo behaviour.
//!
//! # Ported exactly
//!
//! * the logo geometry (`:251-275`): the SVG path from
//!   `textures/dvd_logo.svg`, inlined and extruded by
//!   [`crate::point10::scenes::dvd_logo`] with the JS's `LOGO_WIDTH` /
//!   `LOGO_DEPTH` normalisation (the bounce box uses the measured extents);
//! * the logo motion (`:350-374`): `pos += vel * dt * speedMul` with
//!   `speedMul = 1 + min(smBass, 0.6) + burst * 0.5`, the per-axis room bounds
//!   `VOLUME_WORLD_SIZE / 2 - logoHalf`, the reflection, one palette step plus
//!   `pop = 1` per bounce, and `cornerFlash = 1` on a 2-axis hit;
//! * the palette (`:31-33`), the initial position `(-2, 1, -4)` and the
//!   `normalize(1, 0.6, 0.8) * 3.5` velocity;
//! * the logo transform (`:410-425`): `rotation = (sin(time * 0.3) * 0.25,
//!   faceYaw + sin(time * 0.4) * 0.5, 0)` with
//!   `faceYaw = atan2(cam.x - pos.x, cam.z - pos.z)`, and the `1 + pop * 0.12`
//!   scale;
//! * the audio mapping (`:376-408`): bass mean of bins `[1, 9)`, treble mean of
//!   the top 15%, the asymmetric `dt*16` / `dt*5` smoothing, the kick attack
//!   (`bass > 0.07`, `bass > bassAvg * 1.4`, `time - lastBeat > 0.12`, with
//!   `pop = 1` and `burst = min(1.5, burst + 0.8)`), the `exp(-dt*2/6/3)`
//!   decays and the treble/corner emissive;
//! * the camera orbit (`:427-438`): `orbitYaw += (smBass * 0.35 - orbitYaw) *
//!   min(1, dt * 0.4)`, then the `k = min(1, dt * 1.2)` lerp toward
//!   `(cos(orbitYaw) * 17, 6.5 + sin(time * 0.23) * 1.2, sin(orbitYaw) * 17)`
//!   with `lookAt(0, 0, 0)`;
//! * the emission gain (`:277-300`): `(sphereVerts / logoVerts) * 6`, from the
//!   ported logo mesh's vertex count;
//! * the room lighting (`:319-336`): the dimmed `0.35` fire light, the
//!   HemisphereLight fill and the `22` fire-intensity default that keeps the
//!   bounce colour readable;
//! * the glass pane opacity (`:445`): `0.34 + smBass * 0.06` (SmokeRoom's
//!   `_band(1, 9)` bass, not the blooms' first-five-bin bass);
//! * the prop binder (`sceneProps.js:40-44`, `:205-218`).
//!
//! # Deviations
//!
//! * **No bevel on the logo.** `ExtrudeGeometry` bevels 3 SVG units (0.012
//!   world units at this scale) — sub-pixel, so the caps and walls are flat.
//! * The extruded caps are triangulated with a trapezoidal decomposition rather
//!   than Earcut (see `dvd_logo`).
//! * The logo and the glass keep their procedural WGSL materials instead of
//!   `MeshStandardMaterial` + PMREM (the same substitution SmokeRoom
//!   documents).
//! * The volume's shadow projection onto the glass/logo is not drawn (no
//!   shadow maps); the raymarch's shadow march is.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::AudioBridge;
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, build_box, depth_state, look_at, mat4_mul, normalise, perspective, scale,
    translation, trs,
};
use crate::point10::scenes::dvd_logo::{LogoMesh, build_logo};
use crate::point10::shared::phrase_color::{
    DropColorState, drop_color_pulse, ease_phrase_hue, srgb_hex_to_linear,
};
use crate::point10::shared::random::seeded_rand;
use crate::point10::volume::room::{
    BAND_TWO_END, BAND_TWO_START, BLOOM_BASE, BLOOM_MAX, BLOOM_PER_BASS, CAMERA_FAR, CAMERA_FOV,
    CAMERA_NEAR, CAMERA_START, CLEAR_ALPHA, FireTemporal, GLASS_HALF_X, GLASS_HALF_Y, GLASS_HALF_Z,
    ImprovedNoise, SimulationClock, fire_colour, fire_size, fire_temporal, glass_fit, room_top,
    smoke_bass, treble_band, volume_point_light,
};
use crate::point10::volume::stage::{Material, VolumeStage, volumetric_size};
use crate::point10::volume::{VolumeGrid, VolumeSim, VolumeUniforms};
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, frame_dt, read_scene_props,
    scene_props_with_levers,
};

/// `BASE_SPEED` (`DvdRoom.js:29`).
pub const BASE_SPEED: f32 = 3.5;
/// The initial logo position (`:206`).
pub const LOGO_START_POSITION: [f32; 3] = [-2.0, 1.0, -4.0];
/// The initial velocity direction (`:207`).
pub const LOGO_START_VELOCITY: [f32; 3] = [1.0, 0.6, 0.8];
/// `PALETTE` — the classic screensaver cycle (never random).
pub const PALETTE: [u32; 7] = [
    0x0000_b3ff,
    0x00ff_8c1a,
    0x0059_ff5e,
    0x00ff_3bff,
    0x00ff_e63b,
    0x00ff_3b3b,
    0x00ff_ffff,
];
/// `SPHERE_RADIUS` — the reference emitter's radius (`SmokeRoom.js:73`).
pub const SPHERE_RADIUS: f32 = 0.9;
/// `SphereGeometry(0.9, 64, 40)`'s position count, the emission reference.
pub const REFERENCE_EMITTER_VERTICES: usize = (64 + 1) * (40 + 1);
/// The orbit radius (`DvdRoom.js:433-437`).
pub const CAMERA_ORBIT_RADIUS: f32 = 17.0;
/// The DVD room's fire-intensity default (`sceneProps.js`).
pub const FIRE_INTENSITY: f32 = 22.0;
/// `uFireIntensity` for the volume, dimmed so the logo stays readable
/// (`DvdRoom.js:332`: `this._uFireIntensity.value = 22.0`).
pub const VOLUME_FIRE_INTENSITY: f32 = 22.0;
/// The emitter density default (shared with SmokeRoom).
pub const EMIT_DENSITY: f32 = 28.0;
/// `uEmitTemperature` default.
pub const EMIT_TEMPERATURE: f32 = 22.0;

/// `DVD_ROOM_PROPS` (`sceneProps.js:40-44`).
const DVD_ROOM_PROPS: &[SceneProp] = &[
    SceneProp::float("smokeDensity", "Smoke Density", 28.0, 0.0, 100.0, 0.1),
    SceneProp::float("fireIntensity", "Fire Intensity", 22.0, 0.0, 100.0, 0.1),
    SceneProp::float("smokeLifespan", "Smoke Lifespan", 3.5, 0.1, 20.0, 0.1),
    SceneProp::float("fireLifespan", "Fire Lifespan", 1.3, 0.1, 5.0, 0.1),
    SceneProp::float("simulationSpeed", "Simulation Speed", 1.2, 0.1, 4.0, 0.01),
    SceneProp::float("boxScale", "Box Scale", 1.0, 0.5, 5.0, 0.05),
    SceneProp::float("boxXYScale", "Box Width & Height", 1.0, 0.5, 5.0, 0.05),
];

/// The logo shader's uniform block (208 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct LogoUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    /// The palette colour (linear).
    colour: [f32; 4],
    /// The logo's world position (the fire light origin).
    logo_pos: [f32; 4],
    /// The fire light colour (linear) and intensity (w).
    fire: [f32; 4],
    /// x emissive intensity, y corner flash, z time, w glass glow.
    params: [f32; 4],
}

/// The glass shader's uniform block (192 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GlassUniforms {
    view_proj: Mat4,
    model: Mat4,
    camera_pos: [f32; 4],
    logo_pos: [f32; 4],
    fire: [f32; 4],
    /// x opacity, y time, z smoothed bass.
    params: [f32; 4],
}

/// GPU resources built in `init`.
struct DvdRoomGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    sim: VolumeSim,
    stage: VolumeStage,
    logo: Material,
    glass: Material,
    logo_mesh: MeshGpu,
    glass_mesh: MeshGpu,
}

/// DvdRoom scene.
pub struct DvdRoom {
    /// `this.pos`.
    position: [f32; 3],
    /// `this.vel`.
    velocity: [f32; 3],
    /// `this.colorIdx`.
    colour_index: usize,
    /// `this.pop`.
    pop: f32,
    /// `this.cornerFlash`.
    corner_flash: f32,
    /// `this.burst`.
    burst: f32,
    /// `this.bounces`.
    bounces: u32,
    /// `this.smBass`.
    smoothed_bass: f32,
    /// `this.smTreble`.
    smoothed_treble: f32,
    /// SmokeRoom's own `this._bass` (first-five-bin mean, `dt * 8`).
    smoke_bass: f32,
    /// SmokeRoom's hi-hat band.
    hi: f32,
    /// SmokeRoom's snare band.
    snare: f32,
    /// SmokeRoom's `uFireSize`.
    fire_size: f32,
    /// `this.bassAvg`.
    bass_average: f32,
    /// `this.lastBeat`.
    last_beat: f32,
    /// `this.time` (`BaseSceneWebGPU.update` accumulates the raw delta).
    time: f32,
    /// The frame delta handed to `update`.
    dt: f32,
    /// `this._orbitYaw`.
    orbit_yaw: f32,
    /// The camera position (`this.camera.position`).
    camera: [f32; 3],
    /// The smoke/fire drop pulse (`uFireHue`).
    pulse: DropColorState,
    /// The seed stream behind the smoke pulse.
    seed: u32,
    /// `uFireHue`.
    fire_hue: f32,
    /// `uEmitGain` from the emitter vertex counts.
    emit_gain: f32,
    /// `this.logoHalf`, measured from the built logo mesh.
    logo_half: [f32; 3],
    /// The simulation clock and its CPU noise.
    clock: SimulationClock,
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
    /// The room fit (`glass.scale.y` and `glass.position.y`).
    glass_height: f32,
    glass_centre_y: f32,
    camera_aspect: f32,
    size: (u32, u32),
    /// The simulation grid (`VolumeGrid::JS` unless a test scales it).
    grid: VolumeGrid,
    gpu: Option<DvdRoomGpu>,
}

impl Default for DvdRoom {
    fn default() -> Self {
        Self::new()
    }
}

impl DvdRoom {
    /// Create the scene (`DvdRoom.js:186-221` defaults).
    pub fn new() -> Self {
        let direction = normalise(LOGO_START_VELOCITY);
        let logo = build_logo();
        Self {
            position: LOGO_START_POSITION,
            velocity: [
                direction[0] * BASE_SPEED,
                direction[1] * BASE_SPEED,
                direction[2] * BASE_SPEED,
            ],
            colour_index: 0,
            pop: 0.0,
            corner_flash: 0.0,
            burst: 0.0,
            bounces: 0,
            smoothed_bass: 0.0,
            smoothed_treble: 0.0,
            smoke_bass: 0.0,
            hi: 0.0,
            snare: 0.0,
            fire_size: 1.0,
            bass_average: 0.0,
            last_beat: -1.0,
            time: 0.0,
            dt: 0.0,
            orbit_yaw: 0.0,
            camera: CAMERA_START,
            pulse: DropColorState::new(),
            seed: 0,
            fire_hue: 0.0,
            emit_gain: 6.0,
            logo_half: logo.half_extents,
            clock: SimulationClock::default(),
            noise: ImprovedNoise,
            frame: 0,
            smoke_density: EMIT_DENSITY,
            fire_intensity: FIRE_INTENSITY,
            smoke_lifespan: 3.5,
            fire_lifespan: 1.3,
            simulation_speed: 1.2,
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

    /// A scene with a different simulation grid (tests and the offline
    /// reference renderer use a smaller grid; the scene defaults to the JS
    /// `150 x 150 x 300`).
    pub fn with_grid(grid: VolumeGrid) -> Self {
        let mut scene = Self::new();
        scene.grid = grid;
        scene
    }

    /// Apply the resolved modV props (`sceneProps.js:40-44`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(DVD_ROOM_PROPS, props) {
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

    /// `this.pos`.
    pub fn position(&self) -> [f32; 3] {
        self.position
    }

    /// `this.vel`.
    pub fn velocity(&self) -> [f32; 3] {
        self.velocity
    }

    /// `this.colorIdx`.
    pub fn colour_index(&self) -> usize {
        self.colour_index
    }

    /// `this.bounces`.
    pub fn bounces(&self) -> u32 {
        self.bounces
    }

    /// `this.cornerFlash`.
    pub fn corner_flash(&self) -> f32 {
        self.corner_flash
    }

    /// `this.camera.position`.
    pub fn camera_position(&self) -> [f32; 3] {
        self.camera
    }

    /// SmokeRoom's `uFireSize`.
    pub fn fire_size(&self) -> f32 {
        self.fire_size
    }

    /// The smoke-room bloom strength (`SmokeRoom.js:1395-1400`).
    pub fn bloom_strength(&self) -> f32 {
        (BLOOM_BASE + self.smoke_bass * BLOOM_PER_BASS).min(BLOOM_MAX)
    }

    /// The glass pane opacity (`DvdRoom.js:445`):
    /// `0.34 + this.smBass * 0.06`, where the JS's `smBass` is smoothed from
    /// `this._band(1, 9)` — not the smoke room's first-five-bin bloom bass.
    pub fn glass_opacity(&self) -> f32 {
        0.34 + self.smoothed_bass * 0.06
    }

    /// The palette colour at the current index, linear.
    pub fn logo_colour(&self) -> [f32; 3] {
        srgb_hex_to_linear(PALETTE[self.colour_index % PALETTE.len()])
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// The volume grid the scene runs.
    pub fn grid(&self) -> VolumeGrid {
        self.grid
    }

    /// `this.logoHalf`, measured from the extruded SVG.
    pub fn logo_half_extents(&self) -> [f32; 3] {
        self.logo_half
    }

    /// The bounce bounds `VOLUME_WORLD_SIZE / 2 - logoHalf` (`:351-355`).
    ///
    /// Note the JS does **not** scale these by `boxScale`; the port keeps that.
    pub fn bounce_bounds(&self) -> [f32; 3] {
        [
            (crate::point10::volume::VOLUME_WORLD_SIZE_X / 2.0 - self.logo_half[0]).max(0.05),
            (crate::point10::volume::VOLUME_WORLD_SIZE_Y / 2.0 - self.logo_half[1]).max(0.05),
            (crate::point10::volume::VOLUME_WORLD_SIZE_Z / 2.0 - self.logo_half[2]).max(0.05),
        ]
    }

    fn fire_light(&self) -> [f32; 4] {
        // `uPointLightSurfaceIntensity` 10 x the `0.35` the DVD room sets
        // (`DvdRoom.js:329`), modulated by the fire intensity.
        let colour = fire_colour(0.72, self.fire_hue);
        let intensity = (self.fire_intensity / FIRE_INTENSITY).max(0.0) * 0.35;
        [
            colour[0] * intensity * 10.0,
            colour[1] * intensity * 10.0,
            colour[2] * intensity * 10.0,
            intensity,
        ]
    }

    fn emissive_intensity(&self) -> f32 {
        1.0 + self.smoothed_treble * 2.5 + self.corner_flash * 1.5
    }

    fn logo_model(&self) -> Mat4 {
        let face_yaw = (self.camera[0] - self.position[0]).atan2(self.camera[2] - self.position[2]);
        let rotation = [
            (self.time * 0.3).sin() * 0.25,
            face_yaw + (self.time * 0.4).sin() * 0.5,
            0.0,
        ];
        let s = 1.0 + self.pop * 0.12;
        trs(self.position, rotation, [s, s, s])
    }

    fn glass_model(&self) -> Mat4 {
        mat4_mul(
            &translation(0.0, self.glass_centre_y, 0.0),
            &scale(
                GLASS_HALF_X * 2.0 * self.box_scale * self.box_xy_scale,
                self.glass_height,
                GLASS_HALF_Z * 2.0 * self.box_scale,
            ),
        )
    }

    fn view_proj(&self) -> Mat4 {
        mat4_mul(
            &perspective(
                CAMERA_FOV.to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &look_at(self.camera, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        )
    }

    fn refresh_glass_fit(&mut self) {
        let top = room_top(self.camera, self.box_scale, self.box_xy_scale);
        let (height, centre_y) = glass_fit(top, self.box_scale, self.box_xy_scale);
        self.glass_height = height;
        self.glass_centre_y = centre_y;
    }

    /// `stepBounce(dt, speedMul)` (`:350-374`), using the measured logo half
    /// extents.
    pub fn step_bounce(&mut self, dt: f32, speed_mul: f32) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let speed_mul = if speed_mul.is_finite() {
            speed_mul
        } else {
            1.0
        };
        let bounds = self.bounce_bounds();
        for (position, velocity) in self.position.iter_mut().zip(self.velocity.iter()) {
            *position += velocity * dt * speed_mul;
        }
        let mut hit = 0;
        for ((position, velocity), limit) in self
            .position
            .iter_mut()
            .zip(self.velocity.iter_mut())
            .zip(bounds.iter())
        {
            if *position > *limit {
                *position = *limit;
                *velocity = -velocity.abs();
                hit += 1;
            } else if *position < -*limit {
                *position = -*limit;
                *velocity = velocity.abs();
                hit += 1;
            }
        }
        if hit > 0 {
            self.bounce();
            if hit >= 2 {
                self.corner_flash = 1.0;
            }
        }
    }

    fn bounce(&mut self) {
        self.bounces += 1;
        self.colour_index = (self.colour_index + 1) % PALETTE.len();
        self.pop = 1.0;
    }

    fn band(
        &self,
        freq_mag: &[f32; crate::point10::audio_bridge::AUDIO_SAMPLES],
        lo: usize,
        hi: usize,
    ) -> f32 {
        crate::point10::volume::room::band(freq_mag, lo, hi)
    }

    fn glass_uniforms(&self) -> GlassUniforms {
        GlassUniforms {
            view_proj: self.view_proj(),
            model: self.glass_model(),
            camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
            logo_pos: [self.position[0], self.position[1], self.position[2], 1.0],
            fire: self.fire_light(),
            params: [self.glass_opacity(), self.time, self.smoothed_bass, 0.0],
        }
    }

    fn logo_uniforms(&self) -> LogoUniforms {
        let colour = self.logo_colour();
        LogoUniforms {
            view_proj: self.view_proj(),
            model: self.logo_model(),
            camera_pos: [self.camera[0], self.camera[1], self.camera[2], 1.0],
            colour: [colour[0], colour[1], colour[2], 1.0],
            logo_pos: [self.position[0], self.position[1], self.position[2], 1.0],
            fire: self.fire_light(),
            params: [
                self.emissive_intensity(),
                self.corner_flash,
                self.time,
                self.smoke_bass,
            ],
        }
    }

    fn volume_uniforms(
        &self,
        temporal: &FireTemporal,
        tick: &crate::point10::volume::room::SimTick,
    ) -> VolumeUniforms {
        let scale = self.box_scale;
        let xy = self.box_xy_scale;
        let world = [12.0 * scale * xy, 12.0 * scale * xy, 24.0 * scale];
        let mut uniforms = VolumeUniforms::for_grid(self.grid);
        uniforms.grid[3] = tick.step;
        uniforms.world = [world[0], world[1], world[2], temporal.time];
        // The grid never moves: `_afterInitVolume` pins the origin to the box
        // bottom (`DvdRoom.js:320`), unlike SmokeRoom's emitter-following one.
        uniforms.origin = [0.0, -world[1] * 0.5, 0.0, scale];
        uniforms.sphere_pos = [self.position[0], self.position[1], self.position[2], 0.0];
        uniforms.sphere_vel = [self.velocity[0], self.velocity[1], self.velocity[2], 0.25];
        uniforms.sphere_pos[3] = self.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
        uniforms.camera = [
            self.camera[0],
            self.camera[1],
            self.camera[2],
            self.frame as f32,
        ];
        uniforms.set_camera(self.camera, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        uniforms.fire_start = fire_stop(
            crate::point10::volume::room::FIRE_START_HEX,
            VOLUME_FIRE_INTENSITY,
        );
        uniforms.fire_mid = fire_stop(crate::point10::volume::room::FIRE_MID_HEX, 5.0);
        uniforms.fire_end = fire_stop(crate::point10::volume::room::FIRE_END_HEX, self.fire_hue);
        uniforms.turb = [3.0, 0.15, tick.turbulence, 0.1];
        uniforms.turb2 = [10.0, 0.25, tick.cooling, tick.dissipation];
        uniforms.emit = [
            self.smoke_density,
            EMIT_TEMPERATURE,
            self.fire_size,
            self.emit_gain,
        ];
        uniforms.emit2 = [6.5, temporal.flicker, temporal.flame_height, 0.0];
        uniforms.light = [2.0, 0.5, 1.0, 0.59];
        uniforms.point_light = {
            let colour = volume_point_light(
                temporal,
                EMIT_TEMPERATURE,
                self.smoke_density,
                VOLUME_FIRE_INTENSITY,
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
        uniforms.set_emitter_model(self.logo_model());
        uniforms
    }
}

/// A fire ramp stop in linear RGB plus its scalar slot.
fn fire_stop(hex: u32, scalar: f32) -> [f32; 4] {
    let colour = srgb_hex_to_linear(hex);
    [colour[0], colour[1], colour[2], scalar]
}

#[cfg(test)]
impl DvdRoom {
    /// The live simulation, for the debug readback in the reference renderer.
    pub(crate) fn debug_sim(&self) -> Option<&crate::point10::volume::VolumeSim> {
        self.gpu.as_ref().map(|gpu| &gpu.sim)
    }
}

impl Point10Scene for DvdRoom {
    fn name(&self) -> &'static str {
        "Point10 DVD Room"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width.max(1), ctx.height.max(1));
        self.camera_aspect = ctx.aspect();
        self.refresh_glass_fit();

        let LogoMesh { mesh, half_extents } = build_logo();
        self.logo_half = half_extents;
        // `_emitGainValue = (refCount / max(1, logoVerts)) * 6`
        // (`DvdRoom.js:297-299`).
        self.emit_gain =
            (REFERENCE_EMITTER_VERTICES as f32 / mesh.vertices.len().max(1) as f32) * 6.0;
        let emitter_positions: Vec<[f32; 3]> =
            mesh.vertices.iter().map(|vertex| vertex.position).collect();

        let sim = VolumeSim::new(&device, &queue, self.grid, &emitter_positions);
        let stage = VolumeStage::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);

        let logo = Material::new(
            &device,
            "point10-dvd-room-logo",
            DVD_ROOM_LOGO_WGSL,
            std::mem::size_of::<LogoUniforms>() as u64,
            Some(crate::point10::scenes::cube_mesh::Vertex::layout()),
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // `MeshStandardMaterial({ side: DoubleSide })`: the flattened
                // SVG windings are not all outward-facing.
                cull_mode: None,
                ..Default::default()
            },
            Some(depth_state()),
            None,
        );
        let glass = Material::new(
            &device,
            "point10-dvd-room-glass",
            DVD_ROOM_GLASS_WGSL,
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

        let logo_mesh = MeshGpu::new(&device, "point10-dvd-room-logo-mesh", &mesh);
        let glass_mesh = MeshGpu::new(
            &device,
            "point10-dvd-room-glass-mesh",
            &build_box(1.0, 1.0, 1.0),
        );

        self.gpu = Some(DvdRoomGpu {
            device,
            queue,
            sim,
            stage,
            logo,
            glass,
            logo_mesh,
            glass_mesh,
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

        // ---- SmokeRoom's own audio mapping (the inherited `update`) --------
        if audio.is_available() {
            let freq_mag = &audio.freq_mag;
            let bass = smoke_bass(freq_mag);
            self.smoke_bass += (bass - self.smoke_bass) * (dt * 8.0).min(1.0);
            let hi = treble_band(freq_mag);
            let snare = crate::point10::volume::room::snare_band(freq_mag);
            self.hi += (hi - self.hi) * (dt * 10.0).min(1.0);
            self.snare += (snare - self.snare) * (dt * 12.0).min(1.0);
            self.fire_size = fire_size(self.hi, self.snare);

            let sample = seeded_rand(0, self.seed as i32);
            if drop_color_pulse(&mut self.pulse, self.smoke_bass, dt, sample) {
                self.seed = self.seed.wrapping_add(1);
            }
            self.fire_hue = ease_phrase_hue(self.fire_hue, self.pulse.hue, dt, 1.2);
        }

        // ---- DvdRoom's audio mapping (`:376-408`) -------------------------
        let bass = self.band(&audio.freq_mag, BAND_TWO_START, BAND_TWO_END);
        let treble = if audio.is_available() {
            treble_band(&audio.freq_mag)
        } else {
            0.0
        };
        self.smoothed_bass += (bass - self.smoothed_bass)
            * (dt * if bass > self.smoothed_bass { 16.0 } else { 5.0 }).min(1.0);
        self.smoothed_treble += (treble - self.smoothed_treble)
            * (dt
                * if treble > self.smoothed_treble {
                    16.0
                } else {
                    5.0
                })
            .min(1.0);

        // Kick attack: a sharp jump of the bass envelope.
        self.bass_average = self.bass_average * 0.95 + bass * 0.05;
        if audio.is_available()
            && bass > 0.07
            && bass > self.bass_average * 1.4
            && self.time - self.last_beat > 0.12
        {
            self.last_beat = self.time;
            self.pop = 1.0;
            self.burst = (self.burst + 0.8).min(1.5);
        }
        self.burst *= (-dt * 2.0).exp();
        self.pop *= (-dt * 6.0).exp();
        self.corner_flash *= (-dt * 3.0).exp();

        // Bass speeds the logo up.
        let speed_mul = 1.0 + self.smoothed_bass.min(0.6) + self.burst * 0.5;
        self.step_bounce(dt, speed_mul);

        // Free float: a slow bass-driven orbit around the room centre.
        self.orbit_yaw += (self.smoothed_bass * 0.35 - self.orbit_yaw) * (dt * 0.4).min(1.0);
        let k = (dt * 1.2).min(1.0);
        let target = [
            self.orbit_yaw.cos() * CAMERA_ORBIT_RADIUS,
            6.5 + (self.time * 0.23).sin() * 1.2,
            self.orbit_yaw.sin() * CAMERA_ORBIT_RADIUS,
        ];
        for (camera, target) in self.camera.iter_mut().zip(target.iter()) {
            *camera += (target - *camera) * k;
        }
        self.refresh_glass_fit();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let tick = self.clock.pending(
            frame_dt(self.dt),
            self.simulation_speed,
            self.smoke_lifespan,
            self.fire_lifespan,
        );

        let mut steps: Vec<VolumeUniforms> = Vec::with_capacity(tick.steps as usize);
        let mut temporal = fire_temporal(&self.noise, self.clock.time());
        for index in 0..tick.steps {
            let step_temporal = fire_temporal(&self.noise, tick.time_of(index));
            steps.push(self.volume_uniforms(&step_temporal, &tick));
            temporal = step_temporal;
        }
        let idle_uniforms = if steps.is_empty() {
            Some(self.volume_uniforms(&temporal, &tick))
        } else {
            None
        };
        let logo_uniforms = self.logo_uniforms();
        let glass_uniforms = self.glass_uniforms();
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
                label: Some("point10-dvd-room-sim"),
            });
            gpu.sim.write(&queue, uniforms);
            gpu.sim.step(&mut sim_encoder);
            queue.submit(Some(sim_encoder.finish()));
        }

        gpu.logo.write(&queue, bytemuck::bytes_of(&logo_uniforms));
        gpu.glass.write(&queue, bytemuck::bytes_of(&glass_uniforms));

        {
            let mut pass = gpu.stage.begin_scene(encoder);
            gpu.logo.draw(&mut pass, &gpu.logo_mesh);
            gpu.glass.draw(&mut pass, &gpu.glass_mesh);
        }

        gpu.stage
            .encode_post(encoder, &gpu.sim, bloom_strength, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(DVD_ROOM_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 DVD Room", "3d-scenes")
    }
}

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

const DVD_ROOM_LOGO_WGSL: &str = r#"
struct LogoUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  colour: vec4<f32>,
  logo_pos: vec4<f32>,
  fire: vec4<f32>,
  // x emissive intensity, y corner flash, z time, w smoke bass
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: LogoUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) w_pos: vec3<f32>,
  @location(2) uv: vec2<f32>,
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
  // The model is a rotation + uniform scale, so the normal needs no inverse
  // transpose.
  out.normal = normalize((u.model * vec4<f32>(normal, 0.0)).xyz);
  out.w_pos = world.xyz;
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.w_pos);

  // HemisphereLight(0xbfd4ff, 0x0a0a12, 0.9) + the overhead key light.
  let hemi = mix(
    vec3<f32>(0.003, 0.003, 0.005),
    vec3<f32>(0.48, 0.64, 1.0),
    normal.y * 0.5 + 0.5,
  ) * 0.9;
  let key_dir = normalize(vec3<f32>(0.0, 1.0, 0.0));
  let key = 1.8 * max(dot(normal, key_dir), 0.0) * 0.5;

  // The dimmed 0.35 fire light on the logo.
  let to_fire = u.logo_pos.xyz - in.w_pos;
  let fire_distance = length(to_fire);
  let fire_dir = to_fire / max(fire_distance, 0.001);
  let fire_diffuse = max(dot(normal, fire_dir), 0.0)
    * u.fire.a / (1.0 + fire_distance * fire_distance);
  let fire_light = u.fire.rgb * fire_diffuse;

  // `MeshStandardMaterial`: colour + emissive (the palette), emissiveIntensity.
  let lit = u.colour.rgb * (hemi + key + fire_light);
  let emissive = u.colour.rgb * u.params.x;
  let edge = smoothstep(0.0, 0.18, abs(in.uv.x - 0.5) * 2.0)
    + (1.0 - smoothstep(0.0, 0.18, abs(in.uv.y - 0.5) * 2.0));
  let corner = u.params.y * 0.8 * (edge * 0.5) * u.colour.rgb;

  return vec4<f32>(lit + emissive + corner, 1.0);
}
"#;

const DVD_ROOM_GLASS_WGSL: &str = r#"
struct GlassUniforms {
  view_proj: mat4x4<f32>,
  model: mat4x4<f32>,
  camera_pos: vec4<f32>,
  logo_pos: vec4<f32>,
  fire: vec4<f32>,
  // x opacity, y time, z smoothed bass (band(1,9), DvdRoom.js)
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: GlassUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) w_pos: vec3<f32>,
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
  out.w_pos = world.xyz;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.w_pos);
  let fresnel = pow(1.0 - clamp(abs(dot(normal, view_dir)), 0.0, 1.0), 3.0);

  // The JS panes are lit by a dark PMREM studio and the fire point light: a
  // standard material reads that as almost-black glass with a specular streak,
  // so the env stays dark and the highlight carries the reflection.
  let tint = vec3<f32>(0.58, 0.73, 0.9);
  let env = mix(vec3<f32>(0.0015, 0.002, 0.004), vec3<f32>(0.018, 0.026, 0.042), normal.y * 0.5 + 0.5);
  let streak_dir = normalize(vec3<f32>(-0.4, 0.25, 0.6));
  let streak = pow(max(dot(reflect(-view_dir, normal), streak_dir), 0.0), 16.0) * 0.5
    + pow(max(dot(normal, normalize(vec3<f32>(0.0, 1.0, 0.35))), 0.0), 6.0) * 0.35;

  let to_fire = u.logo_pos.xyz - in.w_pos;
  let fire_distance = length(to_fire);
  let fire_dir = to_fire / max(fire_distance, 0.001);
  let fire_term = max(dot(normal, fire_dir), 0.0)
    * u.fire.a / (1.0 + fire_distance * fire_distance);

  let colour = tint * (env * 2.5 + streak) + u.fire.rgb * fire_term + tint * fresnel * 0.35;
  return vec4<f32>(colour, u.params.x + fresnel * 0.15);
}
"#;

#[cfg(test)]
fn shader_programs() -> Vec<(&'static str, String)> {
    vec![
        ("logo", DVD_ROOM_LOGO_WGSL.to_string()),
        ("glass", DVD_ROOM_GLASS_WGSL.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::audio_bridge::{AUDIO_SAMPLES, BandPayload};
    use crate::point10::scenes::dvd_logo::{LOGO_DEPTH, LOGO_WIDTH};

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn glass_opacity_uses_the_dvd_room_bass_band() {
        // `DvdRoom.js:445`: `opacity = 0.34 + smBass * 0.06` where `smBass`
        // is smoothed from `this._band(1, 9)`. The SmokeRoom bloom bass (the
        // mean of the first five bins) must not drive it.
        let mut scene = DvdRoom::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag[0] = 1.0;
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());
        scene.update(0.1, &bridge);
        assert!(scene.smoke_bass > 0.0, "bin 0 drives the smoke bass");
        assert!(scene.smoothed_bass.abs() < 1e-6, "band(1,9) is silent");
        assert!(
            (scene.glass_opacity() - 0.34).abs() < 1e-6,
            "glass opacity must ignore the smoke bass"
        );

        let mut scene = DvdRoom::new();
        let mut payload = BandPayload::silent(44_100.0);
        for value in payload.freq_mag.iter_mut().take(9).skip(1) {
            *value = 1.0;
        }
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());
        scene.update(0.1, &bridge);
        assert!(scene.smoothed_bass > 0.0, "bins 1..9 drive smBass");
        assert!((scene.glass_opacity() - (0.34 + scene.smoothed_bass * 0.06)).abs() < 1e-6);
        assert!(scene.glass_opacity() > 0.34);
    }

    #[test]
    fn the_logo_bounces_inside_the_js_bounds() {
        let scene = DvdRoom::new();
        // The extruded SVG measures 2.1 x 0.924 x 0.175 half extents.
        close(scene.logo_half_extents()[0], 2.1, 2e-3);
        close(scene.logo_half_extents()[2], LOGO_DEPTH * 0.5, 1e-5);
        let bounds = scene.bounce_bounds();
        close(bounds[0], 6.0 - scene.logo_half_extents()[0], 1e-4);
        close(bounds[1], 6.0 - scene.logo_half_extents()[1], 1e-4);
        close(bounds[2], 12.0 - scene.logo_half_extents()[2], 1e-4);
        assert!(scene.logo_half_extents()[0] * 2.0 > 4.0);
    }

    #[test]
    fn step_bounce_reflects_and_counts_the_axes() {
        let mut scene = DvdRoom::new();
        let bounds = scene.bounce_bounds();
        scene.position = [bounds[0] - 0.01, 0.0, 0.0];
        scene.velocity = [20.0, 0.0, 0.0];
        scene.step_bounce(0.01, 1.0);

        assert_eq!(scene.bounces(), 1);
        assert_eq!(scene.position()[0], bounds[0]);
        assert_eq!(scene.velocity()[0], -20.0);
        assert_eq!(scene.colour_index(), 1);
        assert!(scene.pop > 0.0);
        assert_eq!(scene.corner_flash(), 0.0);

        // A corner hit counts two axes and flashes.
        let mut scene = DvdRoom::new();
        let bounds = scene.bounce_bounds();
        scene.position = [bounds[0] - 0.01, bounds[1] - 0.01, 0.0];
        scene.velocity = [20.0, 20.0, 0.0];
        scene.step_bounce(0.01, 1.0);
        assert_eq!(scene.bounces(), 1);
        assert!(scene.corner_flash() > 0.99);
    }

    #[test]
    fn camera_orbit_lerps_toward_the_js_target() {
        let mut scene = DvdRoom::new();
        scene.update(1.0 / 60.0, &AudioBridge::new());
        let k = (1.0 / 60.0 * 1.2_f32).min(1.0);
        let target = [
            CAMERA_ORBIT_RADIUS,
            6.5 + ((1.0_f32 / 60.0) * 0.23).sin() * 1.2,
            0.0,
        ];
        close(
            scene.camera_position()[0],
            CAMERA_START[0] + (target[0] - CAMERA_START[0]) * k,
            1e-5,
        );
        close(
            scene.camera_position()[1],
            CAMERA_START[1] + (target[1] - CAMERA_START[1]) * k,
            1e-5,
        );
        close(
            scene.camera_position()[2],
            CAMERA_START[2] + (target[2] - CAMERA_START[2]) * k,
            1e-5,
        );
    }

    #[test]
    fn palettes_and_bounce_cycle_the_classic_colours() {
        assert_eq!(PALETTE.len(), 7);
        let scene = DvdRoom::new();
        assert_eq!(scene.colour_index(), 0);
        assert_eq!(scene.logo_colour(), srgb_hex_to_linear(PALETTE[0]));
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!((BASE_SPEED, FIRE_INTENSITY), (3.5, 22.0));
        assert_eq!(LOGO_START_POSITION, [-2.0, 1.0, -4.0]);
        assert_eq!(LOGO_START_VELOCITY, [1.0, 0.6, 0.8]);
        assert_eq!((LOGO_WIDTH, LOGO_DEPTH), (4.2, 0.35));
        assert_eq!((BLOOM_BASE, BLOOM_PER_BASS, BLOOM_MAX), (0.12, 2.2, 2.8));
        assert_eq!(CAMERA_ORBIT_RADIUS, 17.0);
        assert_eq!(CLEAR_ALPHA, 0.0);
        // The emission gain is normalised against the sphere's vertex count.
        let logo = build_logo();
        let gain = (REFERENCE_EMITTER_VERTICES as f32 / logo.mesh.vertices.len() as f32) * 6.0;
        assert!(gain > 1.0 && gain < 100.0, "emit gain {gain}");
    }

    #[test]
    fn a_loud_bass_attack_fires_the_kick_and_scales_the_logo() {
        // `DvdRoom.js:392-405`: a bass attack pops the logo scale
        // (`1 + pop * 0.12`) and speeds the bounce up (`burst`). This is the
        // visible music pulse; a silent bridge must not trigger it.
        let mut scene = DvdRoom::new();
        let mut payload = BandPayload::silent(44_100.0);
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());
        scene.update(0.1, &bridge);
        assert_eq!(scene.pop, 0.0, "silence must not pop the logo");

        // Bins 1..9 are the band `_band(1, 9)` reads.
        for value in payload.freq_mag.iter_mut().take(9).skip(1) {
            *value = 0.8;
        }
        bridge.update(0.1, Some(&payload), &Default::default());
        scene.update(0.1, &bridge);
        assert!(scene.pop > 0.5, "a bass attack pops the logo (pop {})", scene.pop);
        assert!(scene.burst > 0.0, "a bass attack bursts the logo speed");

        // The model scale carries the pop into the drawn geometry
        // (`Mat4` is column-major, so column 0's length is the x scale).
        let model = scene.logo_model();
        let scale_x = (model[0][0] * model[0][0]
            + model[1][0] * model[1][0]
            + model[2][0] * model[2][0])
            .sqrt();
        assert!(scale_x > 1.05, "logo scale must pulse (got {scale_x})");
    }

    #[test]
    fn treble_raises_the_logo_emissive() {
        // `DvdRoom.js:441-443`: the top-15% treble mean drives the emissive
        // glow (`1 + smTreble * 2.5`).
        let mut scene = DvdRoom::new();
        let mut payload = BandPayload::silent(44_100.0);
        let lo = (AUDIO_SAMPLES as f32 * 0.85).floor() as usize;
        for value in payload.freq_mag.iter_mut().skip(lo) {
            *value = 0.9;
        }
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());
        scene.update(0.1, &bridge);
        assert!(
            scene.emissive_intensity() > 1.5,
            "treble must glow the logo (emissive {})",
            scene.emissive_intensity()
        );
    }

    #[test]
    fn update_bounces_the_logo_and_advances_the_camera() {
        let mut scene = DvdRoom::new();
        let mut payload = BandPayload::silent(44_100.0);
        payload.freq_mag = [0.5; AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        let start = scene.position();
        for _ in 0..600 {
            scene.update(1.0 / 60.0, &bridge);
        }
        // Six seconds at >3.5 units/s must hit at least one wall.
        assert!(scene.bounces() > 0, "position stayed at {start:?}");
        assert_ne!(scene.position(), start);
        assert_ne!(scene.camera_position(), CAMERA_START);
        assert!(scene.fire_size() >= 1.0);
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = DvdRoom::new();
        scene.apply_props(|key| match key {
            "smokeDensity" => Some(200.0),
            "fireIntensity" => Some(-3.0),
            "smokeLifespan" => Some(0.0),
            "fireLifespan" => Some(99.0),
            "simulationSpeed" => Some(9.0),
            "boxScale" => Some(9.0),
            "boxXYScale" => Some(0.1),
            _ => None,
        });
        assert_eq!(scene.smoke_density, 100.0);
        assert_eq!(scene.fire_intensity, 0.0);
        assert_eq!(scene.smoke_lifespan, 0.1);
        assert_eq!(scene.fire_lifespan, 5.0);
        assert_eq!(scene.simulation_speed, 4.0);
        assert_eq!(scene.box_scale, 5.0);
        assert_eq!(scene.box_xy_scale, 0.5);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<LogoUniforms>(), 208);
        assert_eq!(std::mem::size_of::<GlassUniforms>(), 192);
    }

    #[test]
    fn the_volume_uniforms_follow_the_logo() {
        let mut scene = DvdRoom::new();
        scene.position = [1.0, 2.0, 3.0];
        scene.velocity = [1.0, 0.0, 0.0];
        scene.fire_size = 2.0;
        scene.camera_aspect = 1.0;
        scene.size = (64, 64);
        let temporal = fire_temporal(&ImprovedNoise, 5.0);
        let tick = crate::point10::volume::room::SimTick {
            steps: 1,
            step: 0.01,
            start_time: 0.0,
            turbulence: 3.0,
            dissipation: 0.2,
            cooling: 0.5,
        };
        let uniforms = scene.volume_uniforms(&temporal, &tick);
        assert_eq!(uniforms.sphere_pos[0..3], [1.0, 2.0, 3.0]);
        close(uniforms.sphere_pos[3], 1.0, 1e-6);
        assert_eq!(uniforms.sphere_vel[0..3], [1.0, 0.0, 0.0]);
        // The emitter matrix carries the bounce position.
        let origin = [
            uniforms.emitter_model[3][0],
            uniforms.emitter_model[3][1],
            uniforms.emitter_model[3][2],
        ];
        assert_eq!(origin, [1.0, 2.0, 3.0]);
        assert_eq!(uniforms.emit[3], scene.emit_gain);
        assert!(uniforms.emit[3] > 0.0);
        assert_eq!(uniforms.view[2], 32.0);
        assert_eq!(uniforms.view[3], 32.0);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let scene = DvdRoom::new();
        assert_eq!(scene.meta().name, "Point10 DVD Room");
        assert_eq!(scene.meta().category, "3d-scenes");
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
    fn shaders_keep_the_bounce_and_lighting_maths() {
        for needle in [
            "let hemi = mix(",
            "let emissive = u.colour.rgb * u.params.x;",
            "u.params.y * 0.8",
        ] {
            assert!(DVD_ROOM_LOGO_WGSL.contains(needle), "logo lost {needle}");
        }
        for needle in [
            "let fresnel = pow(1.0 - clamp(abs(dot(normal, view_dir)), 0.0, 1.0), 3.0);",
            "return vec4<f32>(colour, u.params.x + fresnel * 0.15);",
        ] {
            assert!(DVD_ROOM_GLASS_WGSL.contains(needle), "glass lost {needle}");
        }
    }
}
