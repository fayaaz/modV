//! Spiral Particles — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/SpiralParticles.js`.
//!
//! Four-armed spiral of 8000 sprite dots, slowly spinning, with the whole
//! point cloud's Y rotation driven by the first spectrum bin. Empty space stays
//! fully transparent so the scene composites over the layers below instead of
//! painting a black frame (`SpiralParticles.js:8-10`, `CLEAR_ALPHA = 0`).
//!
//! # Ported exactly
//!
//! * the arm layout (`:24-46`): `arm = i % 4`, `armAngle = arm/4 * 2π`,
//!   `distance = rand * 15`, `spiralAngle = armAngle + distance * 0.3`,
//!   `spread = (rand - 0.5) * (1 + distance * 0.2)`, `y = (rand-0.5) *
//!   (0.5 + distance * 0.05)`, colour `HSL((0.6 + distance*0.02) % 1, 0.8,
//!   0.5 + rand * 0.3)`;
//! * the per-frame spin (`:79-81`): `rotation.y += dt * (0.1 + bin0/255 * 0.5) * spinScale`;
//! * `material.size = 0.1 * sizeScale` (`:82`);
//! * turbulence (`:90-94`): above `energy > 0.6`, each particle jitters
//!   vertically by `(rand - 0.5) * energy * turbulence`;
//! * recolouring (`:96-99`): `HSL((0.6 + dist*0.02) % 1, 0.8, 0.5 + energy*0.3)`
//!   where `dist = sqrt(x² + z²)` of the **pre-jitter** position.
//!
//! # Deviations
//!
//! * `Math.random()` in `_init` → [`seeded_rand`] keyed on the particle index;
//!   the per-frame turbulence jitter (`Math.random()` each frame) uses
//!   `seeded_rand(i, frame)` so a run is reproducible while still jittering.
//! * `THREE.Points` + `PointsMaterial` → instanced billboards
//!   ([`super::particle_quad`]); the object's `rotation.y` becomes the
//!   `ParticleUniforms::spin` uniform.
//! * `depthWrite: false` is moot — the scene contract has no depth attachment.
//! * `renderer.setClearColor(0x000000, 0)` is the same transparent clear the
//!   adapter applies to every Point10 scene (`adapter.js:297`).

#![forbid(unsafe_code)]

use super::particle_quad::{
    ParticleInstance, ParticleQuadRenderer, SPIRAL_SPRITE, hsl_to_linear_rgb,
};
use crate::point10::audio_bridge::AudioBridge;
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, read_scene_props, scene_props_with_levers,
};

/// The spiral is a particle overlay: empty space stays fully transparent.
/// `SpiralParticles.js: export const CLEAR_ALPHA = 0`.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `SpiralParticles.js: this.particleCount = 8000`.
const PARTICLE_COUNT: usize = 8000;
/// `SpiralParticles.js: this.armCount = 4`.
const ARM_COUNT: usize = 4;
/// Base `PointsMaterial.size` (`:58`, scaled by `sizeScale` each frame).
const BASE_SIZE: f32 = 0.1;
/// `SpiralParticles.js: if (energy > 0.6)`.
const TURBULENCE_THRESHOLD: f32 = 0.6;
/// `createParticleTexture(32)` (`:62`).
const SPRITE_SIZE: u32 = SPIRAL_SPRITE;

/// `SPIRAL_PARTICLES_PROPS` (`sceneProps.js:112-116`).
const SPIRAL_PARTICLES_PROPS: &[SceneProp] = &[
    SceneProp::float("spinSpeed", "Spin Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("particleSize", "Particle Size", 1.0, 0.2, 4.0, 0.01),
    SceneProp::float("turbulence", "Turbulence", 0.2, 0.0, 1.0, 0.01),
];

/// CPU-side spiral state — the JS `positions`/`colors` arrays plus the object
/// rotation.
pub(crate) struct SpiralSim {
    positions: Vec<[f32; 3]>,
    colors: Vec<[f32; 3]>,
    /// `points.rotation.y` in radians.
    spin: f32,
    /// Frame counter for the deterministic turbulence jitter.
    frame: u32,
}

impl SpiralSim {
    /// `SpiralParticles.js:_init` (lines 24-46) with [`seeded_rand`] instead of
    /// `Math.random`.
    pub(crate) fn new(count: usize) -> Self {
        let mut positions = Vec::with_capacity(count);
        let mut colors = Vec::with_capacity(count);

        for index in 0..count {
            let seed = index as i32;
            let arm = index % ARM_COUNT;
            let arm_angle = arm as f32 / ARM_COUNT as f32 * std::f32::consts::TAU;
            let distance = seeded_rand(seed, 1) * 15.0;
            let spiral_angle = arm_angle + distance * 0.3;
            let spread = (seeded_rand(seed, 2) - 0.5) * (1.0 + distance * 0.2);

            let position = [
                spiral_angle.cos() * distance + spread,
                (seeded_rand(seed, 3) - 0.5) * (0.5 + distance * 0.05),
                spiral_angle.sin() * distance + spread,
            ];
            let hue = 0.6 + distance * 0.02;
            let color =
                hsl_to_linear_rgb(hue.rem_euclid(1.0), 0.8, 0.5 + seeded_rand(seed, 4) * 0.3);

            positions.push(position);
            colors.push(color);
        }

        Self {
            positions,
            colors,
            spin: 0.0,
            frame: 0,
        }
    }

    /// `SpiralParticles.js:update` (lines 76-104): spin, turbulence and
    /// recolour. `energy` is `audioData[0] / 255`.
    fn step(
        &mut self,
        dt: f32,
        energy: f32,
        spin_scale: f32,
        size_scale: f32,
        turbulence: f32,
    ) -> f32 {
        if dt.is_finite() && dt > 0.0 {
            self.spin += dt * (0.1 + energy * 0.5) * spin_scale;
        }
        let material_size = BASE_SIZE * size_scale;
        let lightness = 0.5 + energy * 0.3;
        self.frame = self.frame.wrapping_add(1);

        for index in 0..self.positions.len() {
            let position = &mut self.positions[index];
            let x = position[0];
            let z = position[2];
            let dist = (x * x + z * z).sqrt();

            if energy > TURBULENCE_THRESHOLD {
                // `(Math.random() - 0.5) * energy * turbulence`
                position[1] +=
                    (seeded_rand(index as i32, self.frame as i32) - 0.5) * energy * turbulence;
            }

            let hue = (0.6 + dist * 0.02).rem_euclid(1.0);
            self.colors[index] = hsl_to_linear_rgb(hue, 0.8, lightness);
        }

        material_size
    }

    /// `points.rotation.y` (for tests).
    pub(crate) fn spin(&self) -> f32 {
        self.spin
    }

    /// Positions (for tests).
    pub(crate) fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Colours (for tests).
    pub(crate) fn colors(&self) -> &[[f32; 3]] {
        &self.colors
    }
}

/// Spiral Particles scene.
pub struct SpiralParticles {
    sim: SpiralSim,
    /// `this.spinScale = 1` (`:18`).
    spin_scale: f32,
    /// `this.sizeScale = 1` (`:19`).
    size_scale: f32,
    /// `this.turbulence = 0.2` (`:20`).
    turbulence: f32,
    size: (u32, u32),
    gpu: Option<ParticleQuadRenderer>,
}

impl Default for SpiralParticles {
    fn default() -> Self {
        Self::new()
    }
}

impl SpiralParticles {
    /// Create the scene (CPU state only; GPU resources come from `init`).
    pub fn new() -> Self {
        Self {
            sim: SpiralSim::new(PARTICLE_COUNT),
            spin_scale: 1.0,
            size_scale: 1.0,
            turbulence: 0.2,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:406-413`):
    /// `spinScale = spinSpeed`, `sizeScale = particleSize`,
    /// `turbulence`.
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(SPIRAL_PARTICLES_PROPS, props) {
            match key {
                "spinSpeed" => self.spin_scale = value,
                "particleSize" => self.size_scale = value,
                "turbulence" => self.turbulence = value,
                _ => {}
            }
        }
    }

    /// Number of particles (`this.particleCount`).
    pub fn count(&self) -> usize {
        PARTICLE_COUNT
    }

    /// The object's Y rotation in radians (`points.rotation.y`).
    pub fn spin(&self) -> f32 {
        self.sim.spin()
    }

    /// CPU particle positions (for tests).
    pub fn positions(&self) -> &[[f32; 3]] {
        self.sim.positions()
    }

    /// CPU particle colours (for tests).
    pub fn colors(&self) -> &[[f32; 3]] {
        self.sim.colors()
    }

    fn instances(&self) -> Vec<ParticleInstance> {
        self.sim
            .positions()
            .iter()
            .zip(self.sim.colors().iter())
            .map(|(position, color)| ParticleInstance {
                position: *position,
                color: *color,
            })
            .collect()
    }
}

impl Point10Scene for SpiralParticles {
    fn name(&self) -> &'static str {
        "Point10 Spiral Particles"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        self.size = (ctx.width, ctx.height);
        let mut gpu = ParticleQuadRenderer::new(
            &ctx.device,
            &ctx.queue,
            ctx.target_format,
            PARTICLE_COUNT,
            SPRITE_SIZE,
        );
        gpu.set_state(
            self.size.0,
            self.size.1,
            BASE_SIZE * self.size_scale,
            0.9,
            self.sim.spin(),
        );
        let instances = self.instances();
        gpu.upload(&instances);
        self.gpu = Some(gpu);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.set_state(
                self.size.0,
                self.size.1,
                BASE_SIZE * self.size_scale,
                0.9,
                self.sim.spin(),
            );
        }
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        // `if (!audioData || audioData.length === 0) return;` — the JS returns
        // before `super.update`? No: `super.update(dt, audioData)` runs first
        // (`:77`), so the clock still advances. SpiralSim has no clock of its
        // own; its spin only integrates on an audio frame.
        let Some(&first) = audio.spectrum.first() else {
            return;
        };
        let energy = first as f32 / 255.0;

        let material_size = self.sim.step(
            dt,
            energy,
            self.spin_scale,
            self.size_scale,
            self.turbulence,
        );

        let instances = self.instances();
        if let Some(gpu) = self.gpu.as_mut() {
            // `transparent: true, opacity: 0.9` (`:59-60`)
            gpu.set_state(
                self.size.0,
                self.size.1,
                material_size,
                0.9,
                self.sim.spin(),
            );
            gpu.upload(&instances);
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-spiral-particles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // CLEAR_ALPHA = 0: transparent clear so the spiral
                    // composites over the layers below (`:8-10`).
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        gpu.draw(&mut pass, PARTICLE_COUNT as u32);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(SPIRAL_PARTICLES_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 Spiral Particles", "3d-scenes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arms_are_laid_out_deterministically_and_inside_the_radius() {
        let a = SpiralSim::new(PARTICLE_COUNT);
        let b = SpiralSim::new(PARTICLE_COUNT);
        assert_eq!(a.positions(), b.positions());
        assert_eq!(a.colors(), b.colors());

        for (index, position) in a.positions().iter().enumerate() {
            let radius = (position[0] * position[0] + position[2] * position[2]).sqrt();
            // distance <= 15 plus a spread of up to (1 + 15*0.2)/2 = 2, so the
            // worst case is (15 + 2) along one axis and 2 along the other.
            assert!(radius <= 17.2, "particle {index} at radius {radius}");
            // y stays inside (0.5 + 15*0.05)/2 = 0.625.
            assert!(position[1].abs() <= 0.625 + 1e-5);
        }

        // The four arms start at 0, π/2, π, 3π/2: particle i is in arm i % 4.
        for arm in 0..ARM_COUNT {
            let angle = arm as f32 / ARM_COUNT as f32 * std::f32::consts::TAU;
            assert!((angle - arm as f32 * std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        }
    }

    #[test]
    fn spin_integrates_with_dt_and_energy() {
        let mut sim = SpiralSim::new(PARTICLE_COUNT);
        // energy 0.2 -> dt * (0.1 + 0.2 * 0.5) = 0.1 * 0.2 = 0.02 at 1x.
        let size = sim.step(0.1, 0.2, 1.0, 1.0, 0.2);
        assert!((sim.spin() - 0.02).abs() < 1e-6);
        assert!((size - BASE_SIZE).abs() < 1e-6);

        // 2x spin scale and 4x size scale.
        let mut fast = SpiralSim::new(PARTICLE_COUNT);
        let size = fast.step(0.1, 0.2, 2.0, 4.0, 0.2);
        assert!((fast.spin() - 0.04).abs() < 1e-6);
        assert!((size - BASE_SIZE * 4.0).abs() < 1e-6);
    }

    #[test]
    fn turbulence_only_lifts_above_the_threshold_and_respects_its_scale() {
        let mut quiet = SpiralSim::new(PARTICLE_COUNT);
        let before = quiet.positions()[3][1];
        quiet.step(0.1, 0.5, 1.0, 1.0, 1.0);
        assert_eq!(quiet.positions()[3][1], before, "no jitter below 0.6");

        let mut loud = SpiralSim::new(PARTICLE_COUNT);
        let mut previous_y = loud.positions()[3][1];
        let mut moved = false;
        for _ in 0..8 {
            loud.step(0.1, 1.0, 1.0, 1.0, 0.2);
            let y = loud.positions()[3][1];
            if (y - previous_y).abs() > 1e-6 {
                moved = true;
            }
            // |(rand - 0.5) * energy * turbulence| <= 0.5 * 1 * 0.2
            assert!((y - previous_y).abs() <= 0.1 + 1e-6);
            previous_y = y;
        }
        assert!(moved, "turbulence must jitter the y position");
    }

    #[test]
    fn every_particle_is_recoloured_from_energy() {
        let mut quiet = SpiralSim::new(PARTICLE_COUNT);
        let mut loud = SpiralSim::new(PARTICLE_COUNT);
        // Capture the init colours first: `_init` already sets HSL(..., 0.6).
        let initial = quiet.colors()[5];
        quiet.step(0.1, 0.0, 1.0, 1.0, 0.2);
        loud.step(0.1, 1.0, 1.0, 1.0, 0.2);

        // energy 0.0 -> lightness 0.5, so the colour changes from the 0.5 +
        // rand * 0.3 init value.
        assert_ne!(quiet.colors()[5], initial);
        let quiet_luma: f32 = quiet.colors()[5].iter().sum();
        let loud_luma: f32 = loud.colors()[5].iter().sum();
        assert!(loud_luma > quiet_luma);
        for color in loud.colors() {
            for channel in color {
                assert!((0.0..=1.0).contains(channel));
            }
        }
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut spiral = SpiralParticles::new();
        spiral.apply_props(|key| match key {
            "spinSpeed" => Some(2.0),
            "particleSize" => Some(0.05),
            "turbulence" => Some(2.0),
            _ => None,
        });
        assert_eq!(spiral.spin_scale, 2.0);
        assert_eq!(spiral.size_scale, 0.2, "particleSize clamps to 0.2");
        assert_eq!(spiral.turbulence, 1.0, "turbulence clamps to 1");

        let mut defaults = SpiralParticles::new();
        defaults.apply_props(|_| None);
        assert_eq!(defaults.spin_scale, 1.0);
        assert_eq!(defaults.size_scale, 1.0);
        assert_eq!(defaults.turbulence, 0.2);
    }

    #[test]
    fn clear_alpha_is_transparent_and_update_without_spectrum_is_a_no_op() {
        assert_eq!(CLEAR_ALPHA, 0.0);
        let mut spiral = SpiralParticles::new();
        let before = spiral.positions()[0];
        let bridge = AudioBridge::new();
        spiral.update(0.25, &bridge);
        assert_eq!(spiral.spin(), 0.0);
        assert_eq!(spiral.positions()[0], before);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let spiral = SpiralParticles::new();
        assert_eq!(spiral.meta().name, "Point10 Spiral Particles");
        let names: Vec<&str> = spiral.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 3..],
            &["spinSpeed", "particleSize", "turbulence"]
        );
        assert_eq!(names[0], "gain");
    }
}
