//! Per-scene GPU smoke tests: two frames into a 64x64 target, assert non-empty
//! output.
//!
//! Rendered through [`crate::point10::postfx::Composer`] so the test also
//! exercises the real presentation path (scene target → output pass). Skipped
//! with a log line when no fallback adapter is available.

use crate::point10::audio_bridge::{AUDIO_SAMPLES, AudioBridge, BandPayload};
use crate::point10::postfx::Composer;
use crate::point10::test_support::{TestTarget, any_lit, gpu};
use crate::point10::{HDR_FORMAT, Point10Scene, SceneContext, all_scenes};

/// Scene smoke-test size.
const SIZE: u32 = 64;
/// Particle sizes are in world units, and `size_px = size * (height / 2) / 15`:
/// at 64 px the default 0.1–0.15 sizes are a third of a pixel, so the particle
/// scenes are rendered at 256 px (and with their size props maxed) to give the
/// billboards something to rasterise.
const PARTICLE_SIZE: u32 = 256;

/// An audio payload with a loud first bin, full waveform and a beat, so the
/// scenes that gate on `audioData` actually animate.
fn loud_payload() -> BandPayload {
    let mut payload = BandPayload::silent(44_100.0);
    payload.byte_frequency_data = vec![200; 1024];
    payload.wave = [0.5; AUDIO_SAMPLES];
    payload.beat = 0.8;
    payload.lanes = [0.6; 12];
    payload.cells = [0.6; 16];
    payload.freq_mag = [0.4; AUDIO_SAMPLES];
    payload
}

/// Initialise `scene`, drive it for two frames and present it; returns the
/// final RGBA8 frame.
fn render_two_frames(scene: &mut dyn Point10Scene, size: u32) -> Option<Vec<u8>> {
    let (device, queue) = gpu()?;

    let mut composer = Composer::new(&device, &queue, size, size, wgpu::TextureFormat::Rgba8Unorm);
    let mut ctx = SceneContext::new(&device, &queue, HDR_FORMAT);
    ctx.resize(size, size);
    scene.init(&mut ctx);
    scene.resize(size, size);

    let mut bridge = AudioBridge::new();
    let payload = loud_payload();
    let target = TestTarget::new(&device, size, size);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("point10-scene-smoke"),
    });

    // Two frames: the second proves per-frame state (buffers, uniforms,
    // instance uploads) survives the first render.
    for _ in 0..2 {
        bridge.update(1.0 / 60.0, Some(&payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);
        scene.render(&mut encoder, composer.scene_view());
    }
    composer.render(&mut encoder, &target.view);
    queue.submit(Some(encoder.finish()));

    Some(target.read(&device, &queue))
}

#[test]
fn chromatic_wave_renders() {
    let mut scenes = all_scenes();
    let Some(pixels) = render_two_frames(scenes[0].as_mut(), SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "ChromaticWave produced an empty frame at {SIZE}x{SIZE}"
    );
}

#[test]
fn particle_field_renders() {
    let mut scene = crate::point10::scenes::particle_field::ParticleField::new();
    // Make the dots big enough to rasterise at the smoke-test size.
    scene.apply_props(|key| (key == "particleSize").then_some(1.0));

    let Some(pixels) = render_two_frames(&mut scene, PARTICLE_SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "ParticleField produced an empty frame at {PARTICLE_SIZE}x{PARTICLE_SIZE}"
    );
}

#[test]
fn spiral_particles_renders() {
    let mut scene = crate::point10::scenes::spiral_particles::SpiralParticles::new();
    scene.apply_props(|key| (key == "particleSize").then_some(4.0));

    let Some(pixels) = render_two_frames(&mut scene, PARTICLE_SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "SpiralParticles produced an empty frame at {PARTICLE_SIZE}x{PARTICLE_SIZE}"
    );
}

#[test]
fn waveform_renders() {
    let mut scenes = all_scenes();
    let Some(pixels) = render_two_frames(scenes[3].as_mut(), SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "Waveform produced an empty frame at {SIZE}x{SIZE}"
    );
}

#[test]
fn every_scene_survives_dispose_and_reinit() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    for mut scene in all_scenes() {
        let mut ctx = SceneContext::new(&device, &queue, HDR_FORMAT);
        ctx.resize(SIZE, SIZE);
        scene.init(&mut ctx);
        let bridge = AudioBridge::new();
        scene.update(1.0 / 60.0, &bridge);
        scene.dispose();

        // Rendering after dispose is a no-op, not a panic.
        let target = TestTarget::new(&device, SIZE, SIZE);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("point10-dispose-test"),
        });
        scene.render(&mut encoder, &target.view);
        queue.submit(Some(encoder.finish()));

        // And re-initialising works.
        scene.init(&mut ctx);
        assert_eq!(scene.name(), scene.meta().name);
    }
}
