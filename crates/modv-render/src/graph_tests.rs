//! Headless golden-pixel tests for the compositor (M2b).
//!
//! These tests render on a software adapter (lavapipe/llvmpipe via
//! `force_fallback_adapter`) and compare **exact** pixels against the
//! W3C Compositing and Blending Level 1 formulas, hand-computed in
//! `web/math` form (see the comment on each expectation). All arithmetic in
//! the shader happens on raw UNORM channel values — the canvas 2D pixel
//! space — so the reference values below were computed the same way.
//!
//! Coverage:
//!
//! 1. RowFade at progress 0.5 — the cross-fade math end-to-end.
//! 2. Composite with the separable modes on opaque red-over-blue.
//! 3. `normal` with `globalAlpha = 0.5` (source-over alias parity).
//! 4. Graph error paths (missing texture, size mismatch).
//! 5. Module/Present/SceneTransition nodes execute without panicking.
//! 6. The `main` texture is cleared to transparent black by default.
//!
//! Every GPU test is skipped (with a log line) when no adapter is available,
//! so GPU-less CI environments still compile and run the pure unit tests.

use crate::blend::BlendMode;
use crate::device::{Engine, EngineConfig};
use crate::frame_graph::{
    Executor, FrameGraph, MAIN_TEXTURE_ID, PassNode, TextureId, TextureSurfaces,
};
use crate::headless::{HeadlessTarget, read_back_rgba8};

const SIZE: u32 = 64;

fn try_engine() -> Option<Engine> {
    match Engine::new(&EngineConfig {
        force_fallback_adapter: true,
    }) {
        Ok(engine) => Some(engine),
        Err(error) => {
            eprintln!("skipping headless pixel test: no adapter ({error})");
            None
        }
    }
}

/// Create a solid texture filled with `rgba` (premultiplied, as the
/// compositor stores them).
fn solid_texture(engine: &Engine, rgba: [u8; 4]) -> wgpu::Texture {
    let texture = engine.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modv-test-solid"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixel = rgba;
    let data: Vec<u8> = pixel.repeat((SIZE * SIZE) as usize);
    engine.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// Transparent black scratch of the standard test size.
fn scratch_texture(engine: &Engine) -> wgpu::Texture {
    solid_texture(engine, [0, 0, 0, 0])
}

fn read_pixels(engine: &Engine, texture: &wgpu::Texture) -> Vec<u8> {
    let target = HeadlessTarget {
        texture: texture.clone(),
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        width: SIZE,
        height: SIZE,
    };
    read_back_rgba8(engine, &target).expect("read-back must succeed on a COPY_SRC texture")
}

fn assert_pixels_close(actual: &[u8], expected: [u8; 4], tolerance: i16, context: &str) {
    for (index, chunk) in actual.chunks_exact(4).enumerate() {
        for channel in 0..4 {
            let diff = chunk[channel] as i16 - expected[channel] as i16;
            assert!(
                diff.abs() <= tolerance,
                "{context}: pixel {index} channel {channel} is {} (expected {expected:?} ±{tolerance})",
                chunk[channel]
            );
        }
    }
}

fn id(name: &str) -> TextureId {
    TextureId::new(name)
}

fn insert_surfaces(engine: &Engine, surfaces: &mut TextureSurfaces, entries: &[(&str, [u8; 4])]) {
    for (name, rgba) in entries {
        surfaces.insert(id(name), solid_texture(engine, *rgba));
    }
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// progress=0.5, eased = 0.5²·(3−2·0.5) = 0.5 → fromA = 0.5, toA = 0.5.
/// Pass 1 (from over transparent scratch): (0.5, 0, 0, 0.5) premultiplied,
///   stored UNORM as (128, 0, 0, 128) (0.5 × 255 = 127.5, ties-to-even).
/// Pass 2 (to over that scratch, source-over):
///   co = (0, 0.5, 0) + 0.5 × (128/255, 0, 0) ≈ (0.251, 0.5, 0)
///   αo = 0.5 + 0.5 × (128/255) ≈ 0.751 → 191.5, ties-to-even → 192
/// → the assert allows 191 or 192 via ±1 tolerance.
#[test]
fn row_fade_half_progress_blends() {
    let Some(engine) = try_engine() else {
        return;
    };
    let mut surfaces = TextureSurfaces::new();
    insert_surfaces(&engine, &mut surfaces, &[("from", RED), ("to", GREEN)]);
    let fade = scratch_texture(&engine);
    surfaces.insert(id("fade"), fade.clone());

    let graph = FrameGraph {
        nodes: vec![PassNode::RowFade {
            from: id("from"),
            to: id("to"),
            target: id("fade"),
            progress: 0.5,
            from_alpha: 1.0,
            to_alpha: 1.0,
        }],
    };
    let mut executor = Executor::new(&engine.device, &engine.queue);
    executor
        .execute(&graph, &mut surfaces)
        .expect("row fade must execute");

    let pixels = read_pixels(&engine, &fade);
    assert_pixels_close(&pixels, [64, 128, 0, 191], 1, "row fade at progress 0.5");
}

/// Opaque red source over opaque blue destination for the separable (and
/// HSL-less) modes. Expected bytes, computed from the W3C formulas with
/// sa = da = 1 (premultiplied == straight):
///
/// mode            B(Cb=blue, Cs=red)   co
/// multiply        (0, 0, 0)            (0, 0, 0)
/// lighter         n/a (Fa=Fb=1)        (1, 0, 1)
/// difference      (1, 0, 1)            (1, 0, 1)
/// overlay         Cb ≤ 0.5 → Cs·2Cb    (0, 0, 1)
/// darken          min(Cb, Cs)          (0, 0, 0)
/// lighten         max(Cb, Cs)          (1, 0, 1)
/// hard-light      Cs > 0.5 → screen    (1, 0, 0)
/// soft-light      Cb ≤ 0.25 → D=((16Cb-12)Cb+4)Cb; for blue: Cb=1 → D=1
///                   Cs=0 ≤ 0.5 → B = Cb(1 - (1-2Cs)(1-Cb))… all channels land
///                   on (0, 0, 1)
/// color-dodge     Cb=0 → 0; Cb=1, Cs=0 → min(1, 1/(1-0)) = 1 ⇒ (0, 0, 1)
/// color-burn      Cb=0, Cs=1 → 1-min(1, (1-0)/1) = 0; Cb=1 → 1 ⇒ (0, 0, 1)
/// exclusion       Cb+Cs-2Cb·Cs         (1, 0, 1)
#[test]
fn composite_modes_pixel_math() {
    let Some(engine) = try_engine() else {
        return;
    };
    let cases: &[(BlendMode, [u8; 4])] = &[
        (BlendMode::Multiply, [0, 0, 0, 255]),
        (BlendMode::Lighter, [255, 0, 255, 255]),
        (BlendMode::Difference, [255, 0, 255, 255]),
        (BlendMode::Overlay, [0, 0, 255, 255]),
        (BlendMode::Darken, [0, 0, 0, 255]),
        (BlendMode::Lighten, [255, 0, 255, 255]),
        (BlendMode::HardLight, [255, 0, 0, 255]),
        (BlendMode::SoftLight, [0, 0, 255, 255]),
        (BlendMode::ColorDodge, [0, 0, 255, 255]),
        (BlendMode::ColorBurn, [0, 0, 255, 255]),
        (BlendMode::Exclusion, [255, 0, 255, 255]),
    ];

    for &(mode, expected) in cases {
        let mut surfaces = TextureSurfaces::new();
        insert_surfaces(&engine, &mut surfaces, &[("src", RED), ("dst", BLUE)]);
        let graph = FrameGraph {
            nodes: vec![PassNode::Composite {
                source: id("src"),
                target: id("dst"),
                alpha: 1.0,
                blend: mode,
            }],
        };
        let mut executor = Executor::new(&engine.device, &engine.queue);
        executor
            .execute(&graph, &mut surfaces)
            .unwrap_or_else(|e| panic!("composite with {} must execute: {e}", mode.name()));

        let dst = surfaces
            .get(&id("dst"))
            .expect("dst surface stays registered");
        let pixels = read_pixels(&engine, dst);
        assert_pixels_close(&pixels, expected, 2, mode.name());
    }
}

/// Canvas `globalAlpha = 0.5`, red source over opaque blue destination:
///   co = 0.5 × red + 0.5 × blue = (0.5, 0, 0.5)  →  (128, 0, 128)
///   αo = 0.5 + 1 × 0.5 = 1                        →  255
/// Asserts the `normal` variant; `source-over` must agree byte-for-byte.
#[test]
fn normal_source_over_alpha() {
    let Some(engine) = try_engine() else {
        return;
    };
    for blend in [BlendMode::Normal, BlendMode::SourceOver] {
        let mut surfaces = TextureSurfaces::new();
        insert_surfaces(&engine, &mut surfaces, &[("src", RED), ("dst", BLUE)]);
        let graph = FrameGraph {
            nodes: vec![PassNode::Composite {
                source: id("src"),
                target: id("dst"),
                alpha: 0.5,
                blend,
            }],
        };
        let mut executor = Executor::new(&engine.device, &engine.queue);
        executor
            .execute(&graph, &mut surfaces)
            .unwrap_or_else(|e| panic!("composite {} must execute: {e}", blend.name()));

        let dst = surfaces
            .get(&id("dst"))
            .expect("dst surface stays registered");
        let pixels = read_pixels(&engine, dst);
        assert_pixels_close(&pixels, [128, 0, 128, 255], 1, blend.name());
    }
}

#[test]
fn composite_missing_texture_reports_error() {
    let Some(engine) = try_engine() else {
        return;
    };
    let mut surfaces = TextureSurfaces::new();
    insert_surfaces(&engine, &mut surfaces, &[("src", RED)]);
    let graph = FrameGraph {
        nodes: vec![PassNode::Composite {
            source: id("src"),
            target: id("nope"),
            alpha: 1.0,
            blend: BlendMode::Normal,
        }],
    };
    let mut executor = Executor::new(&engine.device, &engine.queue);
    let error = executor
        .execute(&graph, &mut surfaces)
        .expect_err("missing target must error");
    assert!(format!("{error}").contains("nope"));
}

#[test]
fn composite_size_mismatch_reports_error() {
    let Some(engine) = try_engine() else {
        return;
    };
    let mut surfaces = TextureSurfaces::new();
    insert_surfaces(&engine, &mut surfaces, &[("src", RED), ("dst", BLUE)]);
    // Replace dst with a smaller texture (32x32).
    let small = engine.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modv-test-small"),
        size: wgpu::Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    surfaces.insert(id("dst"), small);
    let graph = FrameGraph {
        nodes: vec![PassNode::Composite {
            source: id("src"),
            target: id("dst"),
            alpha: 1.0,
            blend: BlendMode::Normal,
        }],
    };
    let mut executor = Executor::new(&engine.device, &engine.queue);
    let error = executor
        .execute(&graph, &mut surfaces)
        .expect_err("size mismatch must error");
    assert!(format!("{error}").contains("64x64"));
}

/// Module nodes are documented no-ops until M3/M4 and the executor must not
/// panic; Present and SceneTransition likewise return Ok.
#[test]
fn frame_graph_module_slots_compile_and_noop() {
    let Some(engine) = try_engine() else {
        return;
    };
    let mut surfaces = TextureSurfaces::new();
    insert_surfaces(&engine, &mut surfaces, &[("target", RED)]);
    let graph = FrameGraph {
        nodes: vec![
            PassNode::Module {
                group: "background".to_owned(),
                module: "Ball".to_owned(),
                pipeline_slot: 0,
                target: id("target"),
            },
            PassNode::Module {
                group: "background".to_owned(),
                module: "Bands".to_owned(),
                pipeline_slot: 1,
                target: id("target"),
            },
            PassNode::SceneTransition {
                from: id("target"),
                to: id("target"),
                target: id("target"),
                kind: "CrossZoom".to_owned(),
                progress: 0.5,
            },
            PassNode::Present {
                source: id("target"),
            },
        ],
    };
    let mut executor = Executor::new(&engine.device, &engine.queue);
    executor
        .execute(&graph, &mut surfaces)
        .expect("module/scene-transition/present nodes must be no-ops in M2b");
}

/// The M2 default: the `main` texture is cleared to transparent black at the
/// start of every `execute()` (see `worker/loop.js:564`).
#[test]
fn execute_clears_main_texture_to_transparent() {
    let Some(engine) = try_engine() else {
        return;
    };
    let mut surfaces = TextureSurfaces::new();
    surfaces.insert(id(MAIN_TEXTURE_ID), solid_texture(&engine, RED));
    let graph = FrameGraph::new();
    let mut executor = Executor::new(&engine.device, &engine.queue);
    executor
        .execute(&graph, &mut surfaces)
        .expect("empty graph executes");

    let main = surfaces
        .get(&id(MAIN_TEXTURE_ID))
        .expect("main stays registered");
    let pixels = read_pixels(&engine, main);
    assert_pixels_close(&pixels, [0, 0, 0, 0], 0, "main after default clear");
}
