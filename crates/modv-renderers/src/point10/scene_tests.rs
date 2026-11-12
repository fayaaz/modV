//! Per-scene GPU smoke tests: two frames into a 64x64 target, assert non-empty
//! output.
//!
//! Rendered through [`crate::point10::postfx::Composer`] so the test also
//! exercises the real presentation path (scene target → output pass). Scenes
//! that own their `EffectComposer` in the JS present themselves instead
//! ([`render_owned_chain_two_frames`]). Skipped with a log line when no
//! fallback adapter is available.

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

/// A quiet payload for the scenes whose bass lane is a *scale* (SoundCube's
/// `0.3 + bass * 8`): at `loud_payload`'s 0.6 the cube is 5.1 units wide and the
/// camera at z = 5 ends up inside it.
fn quiet_payload() -> BandPayload {
    let mut payload = BandPayload::silent(44_100.0);
    payload.byte_frequency_data = vec![40; 1024];
    payload.wave = [0.2; AUDIO_SAMPLES];
    payload.beat = 0.2;
    payload.lanes = [0.05; 12];
    payload.cells = [0.05; 16];
    payload.freq_mag = [0.05; AUDIO_SAMPLES];
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

/// [`render_two_frames`] for scenes that own their `EffectComposer`
/// (LightTunnel, Wormhole, ItIsComingForYou).
///
/// Those scenes present themselves: their `render` runs their own
/// `RenderPass -> UnrealBloomPass -> OutputPass` chain straight into the target
/// it is handed. Wrapping them in [`render_two_frames`]' composer would
/// tone-map and sRGB-encode twice, so this path initialises the scene with the
/// destination format and lets it draw into the destination directly — the
/// same contract the engine uses (`Point10Scene::render` writes the presented
/// frame).
fn render_owned_chain_two_frames(scene: &mut dyn Point10Scene, size: u32) -> Option<Vec<u8>> {
    let payload = loud_payload();
    render_owned_chain_two_frames_with(scene, size, &payload)
}

/// [`render_owned_chain_two_frames`] with an explicit payload.
fn render_owned_chain_two_frames_with(
    scene: &mut dyn Point10Scene,
    size: u32,
    payload: &BandPayload,
) -> Option<Vec<u8>> {
    let (device, queue) = gpu()?;

    let mut ctx = SceneContext::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    ctx.resize(size, size);
    scene.init(&mut ctx);
    scene.resize(size, size);

    let mut bridge = AudioBridge::new();
    let target = TestTarget::new(&device, size, size);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("point10-scene-owned-chain-smoke"),
    });

    for _ in 0..2 {
        bridge.update(1.0 / 60.0, Some(payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);
        scene.render(&mut encoder, &target.view);
    }
    queue.submit(Some(encoder.finish()));

    Some(target.read(&device, &queue))
}

#[test]
fn chromatic_line_renders() {
    let mut scenes = all_scenes();
    let Some(pixels) = render_two_frames(scenes[0].as_mut(), SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "ChromaticLine produced an empty frame at {SIZE}x{SIZE}"
    );
}

#[test]
fn chromatic_wave_renders() {
    let mut scenes = all_scenes();
    let Some(pixels) = render_two_frames(scenes[1].as_mut(), SIZE) else {
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
    let Some(pixels) = render_two_frames(scenes[4].as_mut(), SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "Waveform produced an empty frame at {SIZE}x{SIZE}"
    );
}

// ————— batch 2 —————

#[test]
fn square_tunnel_renders() {
    let mut scene = crate::point10::scenes::square_tunnel::SquareTunnel::new();
    let Some(pixels) = render_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "SquareTunnel produced an empty frame at {SIZE}x{SIZE}"
    );
    // `Clear1` is opaque black, so every pixel has alpha 255.
    assert!(
        pixels.chunks_exact(4).all(|pixel| pixel[3] == 255),
        "SquareTunnel must stay opaque (setClearColor(0x000000, 1))"
    );
}

#[test]
fn light_tunnel_renders() {
    let mut scene = crate::point10::scenes::light_tunnel::LightTunnel::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "LightTunnel produced an empty frame at {SIZE}x{SIZE}"
    );
}

#[test]
fn wormhole_renders() {
    let mut scene = crate::point10::scenes::wormhole::Wormhole::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "Wormhole produced an empty frame at {SIZE}x{SIZE}"
    );
}

/// ItIsComingForYou's sprites are 50-60 px, so a 64 px frame is almost all
/// sprite. Render at 256 px to see both the cloud and the gaps around it; the
/// fragment count is the same (the sprite size is constant in pixels).
const IT_IS_COMING_SIZE: u32 = 256;

#[test]
fn it_is_coming_for_you_renders() {
    let mut scene = crate::point10::scenes::it_is_coming_for_you::ItIsComingForYou::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, IT_IS_COMING_SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "ItIsComingForYou produced an empty frame at {IT_IS_COMING_SIZE}x{IT_IS_COMING_SIZE}"
    );
}

/// The scene is deliberately background-free (`CLEAR_ALPHA = 0`,
/// `ItIsComingForYou.js:51-52`): the cloud must composite over the layers
/// below, so the pixels that no point covers keep alpha 0 while the cloud
/// itself is opaque.
#[test]
fn it_is_coming_for_you_keeps_the_background_transparent() {
    let mut scene = crate::point10::scenes::it_is_coming_for_you::ItIsComingForYou::new();
    assert_eq!(scene.clear_alpha(), 0.0);

    let Some(pixels) = render_owned_chain_two_frames(&mut scene, IT_IS_COMING_SIZE) else {
        return;
    };

    assert_transparency(&pixels);
}

// ————— batch 3 —————

/// Count the fully transparent and fully opaque pixels.
fn alpha_counts(pixels: &[u8]) -> (usize, usize, usize) {
    let mut transparent = 0_usize;
    let mut opaque = 0_usize;
    for pixel in pixels.chunks_exact(4) {
        if pixel[3] == 0 {
            transparent += 1;
        }
        if pixel[3] == 255 {
            opaque += 1;
        }
    }
    (transparent, opaque, pixels.len() / 4)
}

/// Assert the frame has an alpha-0 background *and* drawn pixels, with most of
/// the frame still background.
fn assert_transparency(pixels: &[u8]) {
    let (transparent, opaque, total) = alpha_counts(pixels);
    assert!(
        transparent > 0,
        "expected an alpha-0 background, but every pixel was drawn ({total} px)"
    );
    assert!(
        opaque > 0,
        "expected opaque pixels, but none reached alpha 255"
    );
    assert!(
        transparent > total / 4,
        "the scene covers too much of the frame: {transparent} transparent px of {total}"
    );
}

#[test]
fn water_ripple_renders() {
    let mut scene = crate::point10::scenes::water_ripple::WaterRipple::new();
    let Some(pixels) = render_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "WaterRipple produced an empty frame at {SIZE}x{SIZE}"
    );
}

/// The ripple is a circular overlay (`CLEAR_ALPHA = 0`): the corners of the
/// frame are outside `inCircle` and must stay transparent.
#[test]
fn water_ripple_keeps_the_corners_transparent() {
    let mut scene = crate::point10::scenes::water_ripple::WaterRipple::new();
    assert_eq!(scene.clear_alpha(), 0.0);

    let Some(pixels) = render_two_frames(&mut scene, SIZE) else {
        return;
    };
    let (transparent, opaque, total) = alpha_counts(&pixels);
    assert!(
        transparent > 0,
        "expected an alpha-0 background, but every pixel was drawn ({total} px)"
    );
    assert!(
        opaque > 0,
        "expected opaque pixels, but none reached alpha 255"
    );

    // The artwork is a disc of radius ~1 in screenP (the JS default is 0.9),
    // so at most a fifth of the square is background; unlike ItIsComingForYou
    // the coverage is most of the frame. What must hold is that the *extreme
    // corners* (length(screenP) ≈ 1.39) sit outside radius + 0.1 and are
    // exactly transparent.
    let size = SIZE as usize;
    for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
        let alpha = pixels[(y * size + x) * 4 + 3];
        assert_eq!(alpha, 0, "corner ({x}, {y}) must stay transparent");
    }
}

#[test]
fn glass_lens_renders() {
    let mut scene = crate::point10::scenes::glass_lens::GlassLens::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "GlassLens produced an empty frame at {SIZE}x{SIZE}"
    );
    // Outside the glass block the frame is the Chromatic Wave backdrop, whose
    // alpha fades away from the line, so the scene still composites.
    let (transparent, opaque, _) = alpha_counts(&pixels);
    assert!(
        transparent > 0,
        "the backdrop lost its transparent background"
    );
    assert!(opaque > 0, "the glass block must be opaque");
}

#[test]
fn mycelium_renders() {
    let mut scene = crate::point10::scenes::mycelium::Mycelium::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "Mycelium produced an empty frame at {SIZE}x{SIZE}"
    );
    // `gl_FragColor = vec4(col, 1.0)` — the colony is opaque.
    let (_, opaque, total) = alpha_counts(&pixels);
    assert_eq!(opaque, total, "Mycelium must stay opaque");
}

#[test]
fn around_plus_renders() {
    let mut scene = crate::point10::scenes::around_plus::AroundPlus::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "AroundPlus produced an empty frame at {SIZE}x{SIZE}"
    );
    // The camera sits inside the tube: most of the frame is the concrete wall.
    let (_, opaque, total) = alpha_counts(&pixels);
    assert!(
        opaque > total / 4,
        "AroundPlus drew too little of the tube: {opaque} opaque px of {total}"
    );
}

// ————— batch 4 —————

#[test]
fn ultra_hdr_renders() {
    let mut scene = crate::point10::scenes::ultra_hdr::UltraHdr::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "UltraHDR produced an empty frame at {SIZE}x{SIZE}"
    );
}

#[test]
fn mirror_cube_renders() {
    let mut scene = crate::point10::scenes::mirror_cube::MirrorCube::new();
    // Expression2 zooms the camera to CAMERA_MIN_Z on a loud mid band, which
    // fills the frame with the cube. A quiet payload keeps the preset's default
    // 4.25 units so the frame shows both the cube and the transparent clear.
    let Some(pixels) = render_owned_chain_two_frames_with(&mut scene, SIZE, &quiet_payload())
    else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "MirrorCube produced an empty frame at {SIZE}x{SIZE}"
    );
    // The outer cube only covers the middle of the frame, so the corners stay
    // transparent (`CLEAR_ALPHA = 0`).
    let (transparent, opaque, _) = alpha_counts(&pixels);
    assert!(transparent > 0, "MirrorCube lost its transparent clear");
    assert!(opaque > 0, "MirrorCube never wrote an opaque cube pixel");
}

#[test]
fn color_cube_renders() {
    let mut scene = crate::point10::scenes::color_cube::ColorCube::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "ColorCube produced an empty frame at {SIZE}x{SIZE}"
    );
}

#[test]
fn boxy_renders() {
    let mut scene = crate::point10::scenes::boxy::Boxy::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "Boxy produced an empty frame at {SIZE}x{SIZE}"
    );
}

/// Boxy defines `preserveBloomAlpha` (`Boxy.js:73-86`) and blooms with
/// `threshold: 0`, so *every* pixel receives bloom. Without the preserve-alpha
/// combine the additive bloom alpha turns the transparent clear into an opaque
/// veil and the module's layer alpha is lost. This asserts the layer alpha
/// survives: the cleared background must still be alpha 0 while the cubes are
/// opaque.
#[test]
fn boxy_keeps_the_background_transparent() {
    let mut scene = crate::point10::scenes::boxy::Boxy::new();
    assert_eq!(scene.clear_alpha(), 0.0);

    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };

    let (transparent, opaque, total) = alpha_counts(&pixels);
    assert!(
        transparent > 0,
        "expected an alpha-0 background, but every pixel was drawn ({total} px)"
    );
    assert!(
        opaque > 0,
        "expected opaque cubes, but none reached alpha 255"
    );
}

#[test]
fn sound_cube_renders() {
    let mut scene = crate::point10::scenes::sound_cube::SoundCube::new();
    // `scale = 0.3 + bass * 8`, so the smoke test's full-scale payload would
    // put the camera inside a 5-unit cube. A quiet bass lane keeps the cube in
    // frame, which is what the preset looks like in normal playback.
    let Some(pixels) = render_owned_chain_two_frames_with(&mut scene, SIZE, &quiet_payload())
    else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "SoundCube produced an empty frame at {SIZE}x{SIZE}"
    );
    // The composite derives its alpha from the visible glow, so the frame
    // behind the cube stays transparent (`CLEAR_ALPHA = 0`).
    let (transparent, _, _) = alpha_counts(&pixels);
    assert!(transparent > 0, "SoundCube lost its transparent clear");
}

// ————— batch 5a —————

/// HollowCube's cubes are 0.01 world units in a 0.5-unit shell seen by a
/// 50° camera at z = 2.2: a 64 px frame gives them a fraction of a pixel, so
/// the smoke test renders at 128 px.
const HOLLOW_CUBE_SIZE: u32 = 128;

#[test]
fn hollow_cube_renders() {
    let mut scene = crate::point10::scenes::hollow_cube::HollowCube::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, HOLLOW_CUBE_SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "HollowCube produced an empty frame at {HOLLOW_CUBE_SIZE}x{HOLLOW_CUBE_SIZE}"
    );
}

/// `CLEAR_ALPHA = 0` and `Shader2`'s `alpha = c2.a` (the cube capture's), so
/// the empty space around the shell stays transparent.
#[test]
fn hollow_cube_keeps_the_background_transparent() {
    let mut scene = crate::point10::scenes::hollow_cube::HollowCube::new();
    assert_eq!(scene.clear_alpha(), 0.0);

    let Some(pixels) = render_owned_chain_two_frames(&mut scene, HOLLOW_CUBE_SIZE) else {
        return;
    };
    let (transparent, opaque, total) = alpha_counts(&pixels);
    assert!(
        transparent > 0,
        "expected an alpha-0 background, but every pixel was drawn ({total} px)"
    );
    assert!(
        opaque > 0,
        "expected opaque cubes, but none reached alpha 255"
    );
}

/// The particle field is ~2 px per billboard at 1080p (`scale = 0.008 *
/// particleSize` at a 9.9-unit orbit), so the smoke test maxes `particleSize`
/// and renders at [`PARTICLE_SIZE`] to give the dots something to rasterise.
fn attractor_particles_scene() -> crate::point10::scenes::attractor_particles::AttractorParticles {
    let mut scene = crate::point10::scenes::attractor_particles::AttractorParticles::new();
    // The smoke-test frame is square, so pull the orbit back until the 5x5
    // particle field no longer fills it; max the sprite size so the ~2 px
    // 1080p billboards still rasterise at 256 px.
    scene.apply_props(|key| match key {
        "particleSize" => Some(4.0),
        "cameraDistance" => Some(20.0),
        _ => None,
    });
    scene
}

#[test]
fn attractor_particles_renders() {
    let mut scene = attractor_particles_scene();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, PARTICLE_SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "AttractorParticles produced an empty frame at {PARTICLE_SIZE}x{PARTICLE_SIZE}"
    );
    // `SCENE_BACKGROUND = null` / `CLEAR_ALPHA = 0`: the layer alpha is the
    // JS's transparent clear.
    assert_eq!(scene.clear_alpha(), 0.0);
}

/// The particle field is additive on a transparent clear. At the JS density
/// (2^18 points at `PARTICLE_SIZE`) the cloud covers the whole frame, so the
/// meaningful layer-alpha property is that it varies (some pixels stay
/// translucent) and that opaque pixels exist — not alpha-0 corners.
#[test]
fn attractor_particles_keeps_a_translucent_layer() {
    let mut scene = attractor_particles_scene();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, PARTICLE_SIZE) else {
        return;
    };
    let (_, opaque, total) = alpha_counts(&pixels);
    let min = pixels
        .chunks_exact(4)
        .map(|pixel| pixel[3])
        .min()
        .unwrap_or(0);
    assert!(
        min < 255,
        "the additive field must leave translucent pixels ({total} px, all alpha 255)"
    );
    assert!(
        opaque > 0,
        "expected opaque pixels, but none reached alpha 255"
    );
}

/// The DVD Room's logo and glass panes cover the middle of the frame; the
/// clearance around the box (and the room's transparent clear) must survive
/// the `alpha = max(rgb)` composite.
const DVD_ROOM_SIZE: u32 = 128;

#[test]
fn dvd_room_renders() {
    let mut scene = crate::point10::scenes::dvd_room::DvdRoom::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, DVD_ROOM_SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "DvdRoom produced an empty frame at {DVD_ROOM_SIZE}x{DVD_ROOM_SIZE}"
    );
}

/// SmokeRoom's `alpha = max(rgb)` composite leaves a faint haze across the
/// room instead of an alpha-0 background (the JS does the same); the logo and
/// glass still reach opaque, and the haze stays near-transparent.
#[test]
fn dvd_room_keeps_a_translucent_background() {
    let mut scene = crate::point10::scenes::dvd_room::DvdRoom::new();
    assert_eq!(scene.clear_alpha(), 0.0);

    let Some(pixels) = render_owned_chain_two_frames(&mut scene, DVD_ROOM_SIZE) else {
        return;
    };
    let (_, opaque, total) = alpha_counts(&pixels);
    let min = pixels
        .chunks_exact(4)
        .map(|pixel| pixel[3])
        .min()
        .unwrap_or(0);
    assert!(
        min <= 16,
        "the haze must stay near-transparent ({total} px, min alpha {min})"
    );
    assert!(
        opaque > 0,
        "expected opaque logo pixels, but none reached alpha 255"
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
