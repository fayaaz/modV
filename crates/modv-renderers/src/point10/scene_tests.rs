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

/// [`render_owned_chain_two_frames_with`] over many frames: the volumetric
/// scenes need the simulation to build up a field before there is anything to
/// look at (and the JS fades the emitter in over three seconds).
///
/// Returns the last "settled" frame and the frame after it, rendered with
/// `final_payload` — the structural tests compare the pair.
struct RenderedFrames {
    before: Vec<u8>,
    after: Vec<u8>,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn render_owned_chain_frames(
    scene: &mut dyn Point10Scene,
    size: u32,
    frames: usize,
    payload: &BandPayload,
    final_payload: &BandPayload,
) -> Option<RenderedFrames> {
    let (device, queue) = gpu()?;

    let mut ctx = SceneContext::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    ctx.resize(size, size);
    scene.init(&mut ctx);
    scene.resize(size, size);

    let mut bridge = AudioBridge::new();
    let target = TestTarget::new(&device, size, size);

    let mut draw = |payload: &BandPayload| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("point10-scene-owned-chain-frames"),
        });
        bridge.update(1.0 / 60.0, Some(payload), &Default::default());
        scene.update(1.0 / 60.0, &bridge);
        scene.render(&mut encoder, &target.view);
        queue.submit(Some(encoder.finish()));
    };

    for _ in 0..frames {
        draw(payload);
    }
    draw(payload);
    let before = target.read(&device, &queue);
    draw(final_payload);
    let after = target.read(&device, &queue);
    Some(RenderedFrames {
        before,
        after,
        device,
        queue,
    })
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
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255),
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
    for pixel in pixels.as_chunks::<4>().0 {
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
        .as_chunks::<4>()
        .0
        .iter()
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

// ————— the volumetric room scenes (SmokeRoom / DvdRoom) —————
//
// Both scenes run the JS `150 x 150 x 300` grid in production. The GPU tests
// scale it down: a `32 x 32 x 64` grid keeps every compute pass and the whole
// raymarch, but a fraction of the voxel work, so a software adapter finishes
// the simulation in reasonable time. Every world-space parameter (grid origin,
// volume size, ray step count) is unchanged.

/// The grid the volumetric tests run at.
///
/// Half the JS resolution: the emission is per emitter vertex, so a much
/// smaller grid concentrates it into fewer voxels and saturates faster; `64 x
/// 64 x 128` is the smallest grid that still reads like the JS field. The
/// production scenes run [`crate::point10::volume::VolumeGrid::JS`].
fn volume_test_grid() -> crate::point10::volume::VolumeGrid {
    crate::point10::volume::VolumeGrid::new(64, 64, 128)
}

/// Small on purpose: the volume covers most of the frame at any size and the
/// structural checks are all per-pixel.
const VOLUME_SIZE: u32 = 128;
/// Frames of simulation before the structural checks (two steps each, so ~240
/// simulation steps: past the JS's three-second emissive fade-in, with a
/// developed, turbulent dye field).
const VOLUME_WARMUP: usize = 120;

fn smoke_room_scene() -> crate::point10::scenes::smoke_room::SmokeRoom {
    crate::point10::scenes::smoke_room::SmokeRoom::with_grid(volume_test_grid())
}

fn dvd_room_scene() -> crate::point10::scenes::dvd_room::DvdRoom {
    crate::point10::scenes::dvd_room::DvdRoom::with_grid(volume_test_grid())
}

/// Fraction of the frame that stays dark once composited over black.
///
/// The JS's `alpha = max(rgb)` layer is usually semi-transparent, so the
/// composite (not the raw alpha) is what the viewer sees; a working room scene
/// leaves most of the frame dark around the fire.
fn dark_fraction(pixels: &[u8]) -> f32 {
    let mut dark = 0_usize;
    let mut total = 0_usize;
    for pixel in pixels.as_chunks::<4>().0 {
        let alpha = pixel[3] as f32 / 255.0;
        let luminance =
            (0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32)
                * alpha;
        if luminance < 16.0 {
            dark += 1;
        }
        total += 1;
    }
    dark as f32 / total.max(1) as f32
}

/// Standard deviation of the luminance of the drawn (alpha > 0) pixels.
///
/// A flat haze or a solid clear would score ~0; a raymarched fire/smoke field
/// has hot cores, warm edges and transparent background.
fn luminance_spread(pixels: &[u8]) -> f32 {
    let mut values = Vec::new();
    for pixel in pixels.as_chunks::<4>().0 {
        if pixel[3] == 0 {
            continue;
        }
        values.push(0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32);
    }
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    (values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f32>()
        / values.len() as f32)
        .sqrt()
}

/// The module composites over the layers below: the room's emptiness (and the
/// glass) must stay translucent while the fire reaches a strong alpha.
fn assert_translucent_layer(pixels: &[u8]) {
    let translucent = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[3] < 128)
        .count();
    let strong = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[3] > 200)
        .count();
    let total = pixels.len() / 4;
    assert!(
        translucent > total / 10,
        "the room must stay translucent around the fire ({translucent} of {total} px)"
    );
    assert!(
        strong > 0,
        "expected some strongly covered fire/logo pixels (max alpha {})",
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .max()
            .unwrap_or(0)
    );
}

/// `alpha = max(rgb)` (the JS's `outputNode`): alpha may only be zero where
/// the pixel is black, and every drawn pixel carries alpha.
fn assert_alpha_is_max_rgb(pixels: &[u8]) {
    for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        let black = pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0;
        assert_eq!(
            pixel[3] == 0,
            black,
            "pixel {index} ({pixel:?}) breaks alpha = max(rgb)"
        );
    }
}

#[test]
fn smoke_room_renders() {
    let mut scene = smoke_room_scene();
    let payload = loud_payload();
    let Some(frames) =
        render_owned_chain_frames(&mut scene, VOLUME_SIZE, VOLUME_WARMUP, &payload, &payload)
    else {
        return;
    };
    assert!(
        any_lit(&frames.before),
        "SmokeRoom produced an empty frame at {VOLUME_SIZE}x{VOLUME_SIZE}"
    );
}

/// The raymarch must resolve structure: a non-uniform, warm-lit field that
/// fills the room, follows the `alpha = max(rgb)` rule and reacts to audio.
#[test]
fn smoke_room_volume_is_structured() {
    let mut scene = smoke_room_scene();
    assert_eq!(scene.clear_alpha(), 0.0);
    assert_eq!(scene.grid().cell_count(), 64 * 64 * 128);

    // Quiet audio for the structural frame (the field does not saturate), then
    // a loud frame to prove the audio mapping reaches the volume.
    let Some(RenderedFrames {
        before: pixels,
        after: next,
        ..
    }) = render_owned_chain_frames(
        &mut scene,
        VOLUME_SIZE,
        VOLUME_WARMUP,
        &quiet_payload(),
        &loud_payload(),
    )
    else {
        return;
    };

    // Non-uniform: the drawn pixels span a wide luminance range.
    let spread = luminance_spread(&pixels);
    assert!(
        spread > 12.0,
        "the volume looks flat (luminance spread {spread})"
    );
    // Warm-lit: the average drawn pixel is red-dominant (the fire ramp).
    let mut red = 0.0;
    let mut blue = 0.0;
    let mut drawn = 0.0;
    for pixel in pixels.as_chunks::<4>().0 {
        if pixel[3] == 0 {
            continue;
        }
        red += pixel[0] as f32;
        blue += pixel[2] as f32;
        drawn += 1.0;
    }
    assert!(drawn > 0.0);
    assert!(
        red / drawn > blue / drawn * 1.2,
        "the volume is not warm-lit (r {:.1}, b {:.1})",
        red / drawn,
        blue / drawn
    );
    // Fills the room but leaves the frame dark around the fire (the glass box
    // and the empty grid read as background).
    let dark = dark_fraction(&pixels);
    assert!(
        dark > 0.15,
        "the fire covers the whole frame (only {:.0}% dark) — the room bounds are lost",
        dark * 100.0
    );
    assert_alpha_is_max_rgb(&pixels);
    assert_translucent_layer(&pixels);

    // Two audio frames produce different images (the emission pulses).
    assert_ne!(pixels, next, "consecutive audio frames are identical");
}

#[test]
fn dvd_room_renders() {
    let mut scene = dvd_room_scene();
    let payload = loud_payload();
    let Some(frames) =
        render_owned_chain_frames(&mut scene, VOLUME_SIZE, VOLUME_WARMUP, &payload, &payload)
    else {
        return;
    };
    assert!(
        any_lit(&frames.before),
        "DvdRoom produced an empty frame at {VOLUME_SIZE}x{VOLUME_SIZE}"
    );
}

/// The DVD room adds the bouncing logo on top of the same volume: the logo
/// must be visible (opaque-ish pixels), the smoke field structured, the
/// background transparent in the corners and the audio-reactive palette must
/// change the frame.
#[test]
fn dvd_room_volume_is_structured() {
    let mut scene = dvd_room_scene();
    assert_eq!(scene.clear_alpha(), 0.0);
    assert_eq!(scene.grid().cell_count(), 64 * 64 * 128);

    // Quiet audio for the structural frame (the field does not saturate), then
    // a loud frame to prove the audio mapping reaches the volume.
    let Some(RenderedFrames {
        before: pixels,
        after: next,
        ..
    }) = render_owned_chain_frames(
        &mut scene,
        VOLUME_SIZE,
        VOLUME_WARMUP,
        &quiet_payload(),
        &loud_payload(),
    )
    else {
        return;
    };

    let spread = luminance_spread(&pixels);
    assert!(
        spread > 12.0,
        "the DVD room volume looks flat (luminance spread {spread})"
    );
    let dark = dark_fraction(&pixels);
    assert!(
        dark > 0.15,
        "the fire covers the whole frame (only {:.0}% dark) — the room bounds are lost",
        dark * 100.0
    );
    // The logo is bright: some pixel is well above the smoke's own level.
    let max_luminance = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| 0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32)
        .fold(0.0, f32::max);
    assert!(
        max_luminance > 180.0,
        "the DVD logo never lights up (max luminance {max_luminance})"
    );
    assert_alpha_is_max_rgb(&pixels);
    assert_translucent_layer(&pixels);
    assert_ne!(pixels, next, "consecutive audio frames are identical");
}

#[test]
fn latent_backrooms_renders() {
    let mut scene = crate::point10::scenes::latent_backrooms::LatentBackrooms::new();
    let Some(pixels) = render_owned_chain_two_frames(&mut scene, SIZE) else {
        return;
    };
    assert!(
        any_lit(&pixels),
        "LatentBackrooms produced an empty frame at {SIZE}x{SIZE}"
    );
    // `scene.background = 0x050505` and the CRT pass forces alpha 1, so the
    // module is opaque everywhere (its layer alpha is 1, not transparent).
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255),
        "LatentBackrooms must stay opaque (CRT pass alpha = 1)"
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

/// Render SmokeRoom and DvdRoom at 256x256 and write PNGs for inspection.
///
/// `MODV_VOLUME_GRID="x,y,z"` overrides the simulation grid,
/// `MODV_VOLUME_FRAMES` the frame count, `MODV_VOLUME_SIZE` the image size and
/// `MODV_VOLUME_DIR` the output directory (default `/app/crates/target`).
///
/// Ignored by default: it is the offline fidelity check, not a CI test.
#[test]
#[ignore = "renders reference frames for manual inspection"]
fn dump_volume_reference_frames() {
    let size = std::env::var("MODV_VOLUME_SIZE")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(256);
    let frames = std::env::var("MODV_VOLUME_FRAMES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(60);
    let grid = std::env::var("MODV_VOLUME_GRID")
        .ok()
        .and_then(|value| {
            let parts: Vec<u32> = value
                .split(',')
                .filter_map(|part| part.trim().parse::<u32>().ok())
                .collect();
            (parts.len() == 3)
                .then(|| crate::point10::volume::VolumeGrid::new(parts[0], parts[1], parts[2]))
        })
        .unwrap_or(crate::point10::volume::VolumeGrid::JS);
    let directory =
        std::env::var("MODV_VOLUME_DIR").unwrap_or_else(|_| "/app/crates/target".to_string());

    let quiet = std::env::var("MODV_VOLUME_QUIET").is_ok();
    let no_fire = std::env::var("MODV_VOLUME_NO_FIRE").is_ok();
    let density_override = std::env::var("MODV_VOLUME_DENSITY")
        .ok()
        .and_then(|value| value.parse::<f32>().ok());
    let mut smoke = crate::point10::scenes::smoke_room::SmokeRoom::with_grid(grid);
    if let Some(density) = density_override {
        smoke.apply_props(|key| (key == "smokeDensity").then_some(density));
    }
    if no_fire {
        smoke.apply_props(|key| match key {
            "fireIntensity" => Some(0.0),
            "smokeDensity" => Some(0.0),
            _ => None,
        });
    }
    let payload = if quiet {
        quiet_payload()
    } else if std::env::var("MODV_VOLUME_SILENT").is_ok() {
        BandPayload::silent(44_100.0)
    } else {
        loud_payload()
    };
    let mut dye_report: Option<String> = None;
    if let Some(rendered) = render_owned_chain_frames(&mut smoke, size, frames, &payload, &payload)
    {
        let (transparent, opaque, total) = alpha_counts(&rendered.before);
        eprintln!(
            "smoke: grid {:?} cell_count {} frames {frames} transparent {transparent}/{total} opaque {opaque} spread {:.2}",
            grid,
            grid.cell_count(),
            luminance_spread(&rendered.before)
        );
        write_png(
            &format!("{directory}/smoke-room.png"),
            &rendered.before,
            size,
        );
        write_png(
            &format!("{directory}/smoke-room-next.png"),
            &rendered.after,
            size,
        );
        if let Some(sim) = smoke.debug_sim() {
            dye_report = report_dye(sim, &rendered.device, &rendered.queue, grid);
        }
    }
    if let Some(report) = dye_report {
        eprintln!("{report}");
    }

    let mut dvd = crate::point10::scenes::dvd_room::DvdRoom::with_grid(grid);
    let mut dvd_dye: Option<String> = None;
    if let Some(rendered) = render_owned_chain_frames(&mut dvd, size, frames, &payload, &payload) {
        let (transparent, opaque, total) = alpha_counts(&rendered.before);
        eprintln!(
            "dvd:   grid {:?} cell_count {} frames {frames} transparent {transparent}/{total} opaque {opaque} spread {:.2}",
            grid,
            grid.cell_count(),
            luminance_spread(&rendered.before)
        );
        write_png(&format!("{directory}/dvd-room.png"), &rendered.before, size);
        write_png(
            &format!("{directory}/dvd-room-next.png"),
            &rendered.after,
            size,
        );
        if let Some(sim) = dvd.debug_sim() {
            dvd_dye = report_dye(sim, &rendered.device, &rendered.queue, grid);
        }
    }
    if let Some(report) = dvd_dye {
        eprintln!("{report}");
    }
}

/// The dye field's statistics and an ASCII slice through the centre.
#[cfg(test)]
fn report_dye(
    sim: &crate::point10::volume::VolumeSim,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    grid: crate::point10::volume::VolumeGrid,
) -> Option<String> {
    let voxels = sim.read_dye(device, queue);
    let mut max = 0.0_f32;
    let mut sum = 0.0_f64;
    let mut nonzero = 0_usize;
    let mut max_temp = 0.0_f32;
    for voxel in &voxels {
        max = max.max(voxel[0]);
        max_temp = max_temp.max(voxel[1]);
        sum += voxel[0] as f64;
        if voxel[0] > 0.01 {
            nonzero += 1;
        }
    }
    let mut out = format!(
        "dye: mean {:.4} max {:.3} max_temp {:.3} nonzero {}/{}",
        sum / voxels.len() as f64,
        max,
        max_temp,
        nonzero,
        voxels.len()
    );
    let z = grid.z / 2;
    let rows = 18;
    let columns = 36;
    for row in 0..rows {
        let y = (grid.y as usize - 1) * (rows - 1 - row) / (rows - 1).max(1);
        let mut line = String::new();
        for column in 0..columns {
            let x = (grid.x as usize - 1) * column / (columns - 1).max(1);
            let density = voxels[(z as usize * grid.y as usize + y) * grid.x as usize + x][0];
            let level = (density * 4.0).clamp(0.0, 1.0);
            let glyph = if level < 0.05 {
                ' '
            } else if level < 0.25 {
                '.'
            } else if level < 0.5 {
                ':'
            } else if level < 0.75 {
                '*'
            } else {
                '#'
            };
            line.push(glyph);
        }
        out.push_str(&format!("\n  |{line}|"));
    }
    Some(out)
}

#[cfg(test)]
fn write_png(path: &str, pixels: &[u8], size: u32) {
    let image = image::RgbaImage::from_raw(size, size, pixels.to_vec())
        .unwrap_or_else(|| panic!("{path}: pixel count does not match {size}x{size}"));
    image
        .save(path)
        .unwrap_or_else(|error| panic!("{path}: {error}"));
}
