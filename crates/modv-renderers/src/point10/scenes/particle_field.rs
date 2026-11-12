//! Particle Field — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/ParticleField.js`.
//!
//! 5000 drifting points in a 40-unit cube, each a round sprite dot, with a
//! rainbow hue that scrolls through the field and saturates on loud material.
//!
//! # Ported exactly
//!
//! * initial state (`ParticleField.js:18-43`): positions `(rand-0.5)*40`,
//!   velocities `(rand-0.5)*0.02`, colour `HSL(rand, 0.8, 0.6)`;
//! * the per-frame motion (`:77-108`): speed is `1` until the first spectrum
//!   bin passes `0.6`, then `(1 + bass*3) * speedScale`; each axis bounces
//!   (velocity sign flip) once `|position| > 20`;
//! * the per-frame colour (`:103-107`): `hue = (i/count + time*0.1) % 1`,
//!   `HSL(hue, 0.8, 0.5 + energy*0.3)`;
//! * `material.size = particleSize`, `material.opacity = opacity` every frame
//!   (`:82-83`).
//!
//! # Deviations
//!
//! * `Math.random()` in `_init` becomes the shared [`seeded_rand`] keyed on the
//!   particle index, so the field is identical every run (golden-image tests
//!   need that). Same uniform distribution, deterministic layout.
//! * `THREE.Points` + `PointsMaterial` becomes an instanced billboard quad
//!   ([`super::particle_quad`]) — see that module for the mapping.
//! * `THREE.Color.setHSL` returns *working-space* colour, i.e. linear-sRGB
//!   after three's colour management; [`hsl_to_linear_rgb`] does the same
//!   conversion so the composer's sRGB output restores the HSL colour.
//! * No depth attachment: points composite in index order.

#![forbid(unsafe_code)]

use super::particle_quad::{
    PARTICLE_FIELD_SPRITE, ParticleInstance, ParticleQuadRenderer, hsl_to_linear_rgb,
};
use crate::point10::audio_bridge::AudioBridge;
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    Point10Scene, SceneContext, SceneMeta, SceneProp, read_scene_props, scene_props_with_levers,
};

/// `ParticleField.js: this.count = 5000`.
const PARTICLE_COUNT: usize = 5000;
/// `ParticleField.js: (Math.random() - 0.5) * 40` — half-extent 20.
const FIELD_HALF_EXTENT: f32 = 20.0;
/// `ParticleField.js: (Math.random() - 0.5) * 0.02`.
const VELOCITY_SPREAD: f32 = 0.02;
/// `ParticleField.js: if (bass > 0.6)`.
const BASS_BOOST_THRESHOLD: f32 = 0.6;

/// `PARTICLE_FIELD_PROPS` (`sceneProps.js:106-110`).
const PARTICLE_FIELD_PROPS: &[SceneProp] = &[
    SceneProp::float("particleSize", "Particle Size", 0.15, 0.02, 1.0, 0.01),
    SceneProp::float("speed", "Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("opacity", "Opacity", 0.8, 0.0, 1.0, 0.01),
];

/// CPU-side particle state — the exact `positions`/`velocities`/`colors`
/// arrays of the JS scene.
pub(crate) struct ParticleFieldSim {
    positions: Vec<[f32; 3]>,
    velocities: Vec<[f32; 3]>,
    colors: Vec<[f32; 3]>,
    /// `BaseScene.time` — advanced even when there is no audio.
    time: f32,
}

impl ParticleFieldSim {
    /// `ParticleField.js:_init` (lines 18-43), with [`seeded_rand`] in place of
    /// `Math.random`.
    pub(crate) fn new(count: usize) -> Self {
        let mut positions = Vec::with_capacity(count);
        let mut velocities = Vec::with_capacity(count);
        let mut colors = Vec::with_capacity(count);

        for index in 0..count {
            let seed = index as i32;
            let position = [
                (seeded_rand(seed, 1) - 0.5) * 40.0,
                (seeded_rand(seed, 2) - 0.5) * 40.0,
                (seeded_rand(seed, 3) - 0.5) * 40.0,
            ];
            let velocity = [
                (seeded_rand(seed, 4) - 0.5) * VELOCITY_SPREAD,
                (seeded_rand(seed, 5) - 0.5) * VELOCITY_SPREAD,
                (seeded_rand(seed, 6) - 0.5) * VELOCITY_SPREAD,
            ];
            let hue = seeded_rand(seed, 7);
            positions.push(position);
            velocities.push(velocity);
            colors.push(hsl_to_linear_rgb(hue, 0.8, 0.6));
        }

        Self {
            positions,
            velocities,
            colors,
            time: 0.0,
        }
    }

    /// `BaseScene.update`: `this.time += dt` (runs before the audio check).
    fn advance_time(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.time += dt;
        }
    }

    /// `ParticleField.js:update` motion + colour loop (lines 77-112).
    ///
    /// `energy` is `audioData[0] / 255`; the JS uses the same value for `bass`.
    fn step(&mut self, energy: f32, speed_scale: f32) {
        let bass = energy;
        let speed = if bass > BASS_BOOST_THRESHOLD {
            (1.0 + bass * 3.0) * speed_scale
        } else {
            speed_scale
        };

        let lightness = 0.5 + energy * 0.3;
        let count = self.positions.len().max(1) as f32;

        for index in 0..self.positions.len() {
            let position = &mut self.positions[index];
            let velocity = &mut self.velocities[index];
            for axis in 0..3 {
                position[axis] += velocity[axis] * speed;
                if position[axis].abs() > FIELD_HALF_EXTENT {
                    velocity[axis] = -velocity[axis];
                }
            }

            let hue = (index as f32 / count + self.time * 0.1).rem_euclid(1.0);
            self.colors[index] = hsl_to_linear_rgb(hue, 0.8, lightness);
        }
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

/// Particle Field scene.
pub struct ParticleField {
    sim: ParticleFieldSim,
    /// `this.particleSize = 0.15` (`:19`, then bound from the prop).
    particle_size: f32,
    /// `this.speedScale = 1` (`:21`).
    speed_scale: f32,
    /// `this.opacity = 0.8` (`:22`).
    opacity: f32,
    size: (u32, u32),
    gpu: Option<ParticleQuadRenderer>,
}

impl Default for ParticleField {
    fn default() -> Self {
        Self::new()
    }
}

impl ParticleField {
    /// Create the scene (CPU state only; GPU resources come from `init`).
    pub fn new() -> Self {
        Self {
            sim: ParticleFieldSim::new(PARTICLE_COUNT),
            particle_size: 0.15,
            speed_scale: 1.0,
            opacity: 0.8,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:400-404`):
    /// `particleSize`, `speedScale = speed`, `opacity`.
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(PARTICLE_FIELD_PROPS, props) {
            match key {
                "particleSize" => self.particle_size = value,
                "speed" => self.speed_scale = value,
                "opacity" => self.opacity = value,
                _ => {}
            }
        }
    }

    /// Number of particles (`this.count`).
    pub fn count(&self) -> usize {
        PARTICLE_COUNT
    }

    /// CPU particle positions (for tests).
    pub fn positions(&self) -> &[[f32; 3]] {
        self.sim.positions()
    }

    /// CPU particle colours (for tests).
    pub fn colors(&self) -> &[[f32; 3]] {
        self.sim.colors()
    }
}

impl Point10Scene for ParticleField {
    fn name(&self) -> &'static str {
        "Point10 Particle Field"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        self.size = (ctx.width, ctx.height);
        let mut gpu = ParticleQuadRenderer::new(
            &ctx.device,
            &ctx.queue,
            ctx.target_format,
            PARTICLE_COUNT,
            PARTICLE_FIELD_SPRITE,
        );
        gpu.set_state(
            self.size.0,
            self.size.1,
            self.particle_size,
            self.opacity,
            0.0,
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
                self.particle_size,
                self.opacity,
                0.0,
            );
        }
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        // `super.update(dt, audioData)` -> `this.time += dt`, always.
        self.sim.advance_time(dt);

        // `if (!audioData) return;` — the adapter passes `null` when the
        // bridge has no bytes (`adapter.js:812-816`).
        let Some(&first) = audio.spectrum.first() else {
            return;
        };
        let energy = first as f32 / 255.0;

        self.sim.step(energy, self.speed_scale);

        let instances = self.instances();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.set_state(
                self.size.0,
                self.size.1,
                self.particle_size,
                self.opacity,
                0.0,
            );
            gpu.upload(&instances);
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-particle-field"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // `renderer.setClearColor(0x000000, 0)` (`adapter.js:297`)
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
        scene_props_with_levers(PARTICLE_FIELD_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        // `Point10ParticleField.js` sets no `meta.category`, so the adapter's
        // "3d-scenes" default applies (`adapter.js:862`).
        SceneMeta::for_scene("Point10 Particle Field", "3d-scenes")
    }
}

impl ParticleField {
    /// Pack the CPU state into the instance buffer layout.
    fn instances(&self) -> Vec<ParticleInstance> {
        self.sim
            .positions
            .iter()
            .zip(self.sim.colors.iter())
            .map(|(position, color)| ParticleInstance {
                position: *position,
                color: *color,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_field_is_deterministic_and_inside_the_cube() {
        let a = ParticleFieldSim::new(PARTICLE_COUNT);
        let b = ParticleFieldSim::new(PARTICLE_COUNT);
        assert_eq!(a.positions(), b.positions());
        assert_eq!(a.velocities, b.velocities);

        for position in a.positions() {
            for axis in position {
                assert!(axis.abs() <= 20.0, "position {position:?} escaped the cube");
            }
        }
        // Velocities are (rand - 0.5) * 0.02, i.e. within ±0.01.
        for velocity in &a.velocities {
            for axis in velocity {
                assert!(axis.abs() <= 0.01 + 1e-6);
            }
        }
    }

    #[test]
    fn motion_scales_with_bass_above_the_threshold() {
        let base = ParticleFieldSim::new(PARTICLE_COUNT);
        let mut quiet = ParticleFieldSim::new(PARTICLE_COUNT);
        let mut loud = ParticleFieldSim::new(PARTICLE_COUNT);

        // energy 0.5 -> speed 1.0; energy 0.8 > 0.6 -> speed 1 + 0.8*3 = 3.4.
        quiet.step(0.5, 1.0);
        loud.step(0.8, 1.0);

        for axis in 0..3 {
            let quiet_move = quiet.positions()[0][axis] - base.positions()[0][axis];
            let loud_move = loud.positions()[0][axis] - base.positions()[0][axis];
            assert!(
                (loud_move - quiet_move * 3.4).abs() < 1e-5,
                "axis {axis}: {loud_move} vs {}",
                quiet_move * 3.4
            );
        }
    }

    #[test]
    fn axis_bounces_at_the_field_edge() {
        let mut sim = ParticleFieldSim::new(1);
        // Park the particle just inside the +x wall, moving outward.
        sim.positions[0] = [19.995, 0.0, 0.0];
        sim.velocities[0] = [0.01, 0.0, 0.0];

        sim.step(0.0, 1.0);
        assert!(sim.positions[0][0] > 20.0);
        assert!(sim.velocities[0][0] < 0.0, "velocity must flip at the wall");

        // Next frame it moves back inward.
        let before = sim.positions[0][0];
        sim.step(0.0, 1.0);
        assert!(sim.positions[0][0] < before);
    }

    #[test]
    fn hue_scrolls_with_time_and_saturates_with_energy() {
        let mut sim = ParticleFieldSim::new(PARTICLE_COUNT);
        let before = sim.colors()[1];
        sim.advance_time(0.25);
        sim.step(0.0, 1.0);
        let after = sim.colors()[1];
        assert_ne!(before, after, "hue must scroll with time");

        // energy 0.5 -> lightness 0.65; the colours must stay finite and in
        // gamut. Compare a quiet and a loud read of the same particle: the
        // loud version is lighter.
        let mut quiet = ParticleFieldSim::new(PARTICLE_COUNT);
        let mut loud = ParticleFieldSim::new(PARTICLE_COUNT);
        quiet.step(0.0, 1.0);
        loud.step(1.0, 1.0);
        let quiet_luma: f32 = quiet.colors()[7].iter().sum();
        let loud_luma: f32 = loud.colors()[7].iter().sum();
        assert!(
            loud_luma > quiet_luma,
            "{loud_luma} should exceed {quiet_luma}"
        );
        for color in loud.colors() {
            for channel in color {
                assert!(
                    (0.0..=1.0).contains(channel),
                    "colour {color:?} out of gamut"
                );
            }
        }
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut field = ParticleField::new();
        field.apply_props(|key| match key {
            "particleSize" => Some(4.0),
            "speed" => Some(2.0),
            "opacity" => Some(0.25),
            _ => None,
        });
        assert_eq!(field.particle_size, 1.0, "particleSize clamps to 1");
        assert_eq!(field.speed_scale, 2.0);
        assert_eq!(field.opacity, 0.25);

        let mut defaults = ParticleField::new();
        defaults.apply_props(|_| None);
        assert_eq!(defaults.particle_size, 0.15);
        assert_eq!(defaults.speed_scale, 1.0);
        assert_eq!(defaults.opacity, 0.8);
    }

    #[test]
    fn update_without_spectrum_only_advances_the_clock() {
        let mut field = ParticleField::new();
        let before = field.positions()[0];
        let bridge = AudioBridge::new();
        field.update(0.1, &bridge);
        assert_eq!(field.positions()[0], before);
    }

    #[test]
    fn meta_and_props_match_the_js_registry() {
        let field = ParticleField::new();
        assert_eq!(field.meta().name, "Point10 Particle Field");
        assert_eq!(field.meta().category, "3d-scenes");
        // Levers first (adapter.js:868-871), then the scene's own schema.
        let names: Vec<&str> = field.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 3..],
            &["particleSize", "speed", "opacity"]
        );
        assert_eq!(names[0], "gain");
    }
}
