//! Tests for the 2D canvas facade, its path/text helpers and the 15 modules.
//!
//! GPU tests render on the fallback (software / lavapipe) adapter and read the
//! canvas texture back — the same pattern as
//! `crates/modv-render/src/graph_tests.rs`. Every GPU test is skipped with a
//! log line when no adapter is available, so a GPU-less environment still
//! compiles and runs the pure unit tests.

use serde_json::{Value, json};

use crate::two_d::modules::{
    DrawContext, ModuleContext, TwoDProp, all_modules, default_props, prop_i64,
};
use crate::two_d::path::{self, Affine};
use crate::two_d::{AudioFrame, Canvas};

const SIZE: u32 = 64;
const MODULE_SIZE: u32 = 128;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Minimal `std`-only `block_on`, same approach as `modv-render/src/device.rs`.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, Wake};

    struct NoopWaker;
    impl Wake for NoopWaker {
        fn wake(self: std::sync::Arc<Self>) {}
    }

    let waker = std::sync::Arc::new(NoopWaker).into();
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: true,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .map_err(|error| eprintln!("skipping 2D canvas GPU test: no adapter ({error})"))
    .ok()?;
    block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("modv-2d-test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .map_err(|error| eprintln!("skipping 2D canvas GPU test: no device ({error})"))
    .ok()
}

fn canvas(width: u32, height: u32) -> Option<Canvas> {
    let (device, queue) = gpu()?;
    Some(Canvas::new(&device, &queue, width, height))
}

fn pixel(canvas: &Canvas, x: u32, y: u32) -> [u8; 4] {
    let pixels = canvas.read_pixels();
    let index = ((y * canvas.width() + x) * 4) as usize;
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

fn assert_close(actual: [u8; 4], expected: [u8; 4], tolerance: i16, context: &str) {
    for channel in 0..4 {
        let difference = actual[channel] as i16 - expected[channel] as i16;
        assert!(
            difference.abs() <= tolerance,
            "{context}: pixel {actual:?} differs from {expected:?} (tolerance {tolerance})"
        );
    }
}

fn any_opaque(canvas: &Canvas) -> bool {
    canvas.read_pixels().chunks_exact(4).any(|p| p[3] > 0)
}

/// A solid RGBA texture plus its view.
fn solid_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    rgba: [u8; 4],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modv-2d-test-source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let data: Vec<u8> = rgba
        .iter()
        .copied()
        .cycle()
        .take((width * height * 4) as usize)
        .collect();
    queue.write_texture(
        texture.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

// ---------------------------------------------------------------------------
// Facade primitives
// ---------------------------------------------------------------------------

#[test]
fn fill_rect_colour_and_bounds() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, 32.0, 32.0);
    canvas.submit();

    assert_close(
        pixel(&canvas, 16, 16),
        [255, 0, 0, 255],
        1,
        "inside fillRect",
    );
    assert_close(pixel(&canvas, 48, 48), [0, 0, 0, 0], 1, "outside fillRect");
    assert_close(
        pixel(&canvas, 31, 31),
        [255, 0, 0, 255],
        1,
        "last covered pixel",
    );
    assert_close(
        pixel(&canvas, 32, 32),
        [0, 0, 0, 0],
        1,
        "first uncovered pixel",
    );
}

#[test]
fn fill_rect_global_alpha_is_premultiplied() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_global_alpha(0.5);
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.submit();

    // Premultiplied storage: 0.5 red → (128, 0, 0, 128).
    assert_close(
        pixel(&canvas, 10, 10),
        [128, 0, 0, 128],
        2,
        "globalAlpha 0.5",
    );
}

#[test]
fn clear_rect_erases_only_its_area() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([0.0, 0.0, 1.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.clear_rect(32.0, 0.0, 32.0, SIZE as f32);
    canvas.submit();

    assert_close(pixel(&canvas, 8, 8), [0, 0, 255, 255], 1, "left stays blue");
    assert_close(pixel(&canvas, 56, 8), [0, 0, 0, 0], 1, "right is cleared");
}

#[test]
fn clear_all_clears_everything() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([0.0, 1.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.submit();
    canvas.begin_frame();
    canvas.clear_all();
    canvas.submit();
    assert!(
        !any_opaque(&canvas),
        "clear_all must leave a transparent canvas"
    );
}

#[test]
fn fill_triangle_path() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([0.0, 1.0, 0.0, 1.0]);
    canvas.begin_path();
    canvas.move_to(8.0, 8.0);
    canvas.line_to(56.0, 8.0);
    canvas.line_to(32.0, 56.0);
    canvas.close_path();
    canvas.fill();
    canvas.submit();

    assert_close(
        pixel(&canvas, 32, 20),
        [0, 255, 0, 255],
        1,
        "inside triangle",
    );
    assert_close(pixel(&canvas, 8, 56), [0, 0, 0, 0], 1, "outside triangle");
}

#[test]
fn stroke_line_hits_pixels_along_it() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_stroke_style([1.0, 1.0, 1.0, 1.0]);
    canvas.set_line_width(4.0);
    canvas.begin_path();
    canvas.move_to(8.0, 32.0);
    canvas.line_to(56.0, 32.0);
    canvas.stroke();
    canvas.submit();

    let on_line = pixel(&canvas, 32, 32);
    assert!(on_line[3] > 200, "stroke pixel alpha {on_line:?}");
    assert_close(pixel(&canvas, 32, 8), [0, 0, 0, 0], 1, "off the line");
}

#[test]
fn arc_fill_is_a_circle() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 0.0, 1.0, 1.0]);
    canvas.begin_path();
    canvas.arc(32.0, 32.0, 12.0, 0.0, std::f32::consts::TAU, false);
    canvas.fill();
    canvas.submit();

    assert_close(
        pixel(&canvas, 32, 32),
        [255, 0, 255, 255],
        1,
        "circle centre",
    );
    assert_close(pixel(&canvas, 32, 8), [0, 0, 0, 0], 1, "above the circle");
    assert_close(pixel(&canvas, 60, 60), [0, 0, 0, 0], 1, "corner");
}

#[test]
fn draw_image_copies_a_solid_source() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let (_source, view) = solid_texture(&device, &queue, 4, 4, [255, 0, 0, 255]);
    let mut canvas = Canvas::new(&device, &queue, SIZE, SIZE);
    canvas.begin_frame();
    canvas.draw_image(&view, 0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.submit();

    assert_close(pixel(&canvas, 10, 10), [255, 0, 0, 255], 1, "copied source");
    assert_close(pixel(&canvas, 60, 60), [255, 0, 0, 255], 1, "copied corner");
}

#[test]
fn draw_image_region_samples_a_sub_rect() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    // Left half red, right half blue.
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modv-2d-test-halves"),
        size: wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &[255, 0, 0, 255, 0, 0, 255, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut canvas = Canvas::new(&device, &queue, SIZE, SIZE);
    canvas.begin_frame();
    canvas.set_image_smoothing(false);
    // Right half of the source, stretched over the canvas.
    canvas.draw_image_region(
        &view,
        0.5,
        0.0,
        0.5,
        1.0,
        0.0,
        0.0,
        SIZE as f32,
        SIZE as f32,
    );
    canvas.submit();

    assert_close(pixel(&canvas, 32, 32), [0, 0, 255, 255], 1, "right half");
}

#[test]
fn save_restore_round_trips_the_transform() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.save();
    canvas.translate(16.0, 16.0);
    canvas.fill_rect(0.0, 0.0, 8.0, 8.0);
    canvas.restore();
    canvas.fill_rect(0.0, 0.0, 8.0, 8.0);
    canvas.submit();

    assert_close(
        pixel(&canvas, 20, 20),
        [255, 0, 0, 255],
        1,
        "translated rect",
    );
    assert_close(pixel(&canvas, 4, 4), [255, 0, 0, 255], 1, "restored rect");
    assert_close(pixel(&canvas, 4, 20), [0, 0, 0, 0], 1, "no leak");
}

#[test]
fn rotate_transforms_path_points_at_add_time() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([0.0, 1.0, 1.0, 1.0]);
    canvas.save();
    canvas.translate(32.0, 32.0);
    canvas.rotate(std::f32::consts::FRAC_PI_2);
    canvas.fill_rect(-4.0, -4.0, 8.0, 8.0);
    canvas.restore();
    canvas.submit();

    // A square rotated by 90° about its centre is still a square at (32,32).
    assert_close(
        pixel(&canvas, 32, 32),
        [0, 255, 255, 255],
        1,
        "rotated square",
    );
}

#[test]
fn clip_rect_limits_drawing() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.save();
    canvas.clip_rect(0.0, 0.0, 32.0, SIZE as f32);
    canvas.set_fill_style([1.0, 1.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.restore();
    canvas.submit();

    assert_close(pixel(&canvas, 8, 32), [255, 255, 0, 255], 1, "inside clip");
    assert_close(pixel(&canvas, 56, 32), [0, 0, 0, 0], 1, "outside clip");
}

#[test]
fn clip_circle_from_arc_path() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.save();
    canvas.begin_path();
    canvas.arc(32.0, 32.0, 10.0, 0.0, std::f32::consts::TAU, true);
    canvas.clip();
    canvas.set_fill_style([1.0, 0.0, 1.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.restore();
    canvas.submit();

    assert_close(
        pixel(&canvas, 32, 32),
        [255, 0, 255, 255],
        1,
        "circle clip centre",
    );
    // Inside the bbox but outside the circle: the analytic circle clip keeps
    // this transparent.
    assert_close(
        pixel(&canvas, 24, 24),
        [0, 0, 0, 0],
        1,
        "bbox corner is clipped",
    );
    assert_close(
        pixel(&canvas, 32, 15),
        [0, 0, 0, 0],
        1,
        "above circle centre",
    );
}

#[test]
fn draw_image_self_snapshots_current_content() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, 32.0, 32.0);
    canvas.draw_image_self(0.0, 32.0, SIZE as f32, SIZE as f32);
    canvas.submit();

    // The snapshot contains the top-left red quadrant; the copy lands at
    // (0, 32) so the same quadrant appears in the bottom-left.
    assert_close(pixel(&canvas, 16, 40), [255, 0, 0, 255], 1, "snapshot copy");
    assert_close(pixel(&canvas, 48, 40), [0, 0, 0, 0], 1, "empty source area");
}

#[test]
fn canvas_starts_transparent() {
    // wgpu does not zero-initialise textures; the facade must, or the first
    // frame samples undefined pixels (and self-sampling modules read garbage).
    let Some(canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    assert!(
        !any_opaque(&canvas),
        "a fresh canvas must be transparent black"
    );
}

#[test]
fn snapshot_samples_the_previous_submit_not_pending_commands() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut canvas = Canvas::new(&device, &queue, SIZE, SIZE);

    // Frame N-1: opaque red.
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.submit();

    // Frame N: a blue fill is recorded but not submitted. The snapshot must
    // still hold frame N-1 (red), never the live target being written.
    canvas.begin_frame();
    canvas.set_fill_style([0.0, 0.0, 1.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    let snapshot = canvas.snapshot();

    let mut probe = Canvas::new(&device, &queue, SIZE, SIZE);
    probe.begin_frame();
    probe.draw_image(&snapshot, 0.0, 0.0, SIZE as f32, SIZE as f32);
    probe.submit();
    assert_close(
        pixel(&probe, 32, 32),
        [255, 0, 0, 255],
        1,
        "snapshot must hold frame N-1",
    );

    // The live canvas, once the pending command is submitted, is blue.
    canvas.submit();
    assert_close(
        pixel(&canvas, 32, 32),
        [0, 0, 255, 255],
        1,
        "live canvas after submit",
    );
}

#[test]
fn active_clip_intersects_rectangles() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.save();
    // First clip: x 0..48. Second: x 0..32 — Canvas2D intersects, so the
    // result is x 0..32, not the replacement 0..32 over the full height.
    canvas.clip_rect(0.0, 0.0, 48.0, 48.0);
    canvas.clip_rect(0.0, 0.0, 32.0, SIZE as f32);
    canvas.set_fill_style([1.0, 1.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.restore();
    canvas.submit();

    assert_close(
        pixel(&canvas, 16, 16),
        [255, 255, 0, 255],
        1,
        "inside union",
    );
    assert_close(
        pixel(&canvas, 40, 16),
        [0, 0, 0, 0],
        1,
        "outside the second clip",
    );
    assert_close(
        pixel(&canvas, 16, 56),
        [0, 0, 0, 0],
        1,
        "outside the first clip",
    );
}

#[test]
fn active_clip_intersects_a_circle_with_a_rect() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.save();
    // Rect covers the top half; the circle is centred on the boundary, so
    // only its upper half may draw.
    canvas.clip_rect(0.0, 0.0, SIZE as f32, 32.0);
    canvas.begin_path();
    canvas.arc(32.0, 32.0, 20.0, 0.0, std::f32::consts::TAU, true);
    canvas.clip();
    canvas.set_fill_style([0.0, 1.0, 1.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.restore();
    canvas.submit();

    assert_close(
        pixel(&canvas, 32, 16),
        [0, 255, 255, 255],
        1,
        "inside both clips",
    );
    assert_close(
        pixel(&canvas, 32, 24),
        [0, 255, 255, 255],
        1,
        "upper circle half",
    );
    assert_close(
        pixel(&canvas, 32, 40),
        [0, 0, 0, 0],
        1,
        "circle half below the rect",
    );
    assert_close(
        pixel(&canvas, 8, 16),
        [0, 0, 0, 0],
        1,
        "inside the rect but outside the circle",
    );
}

#[test]
fn full_viewport_clear_rect_erases_earlier_draws() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([0.0, 0.0, 1.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, 32.0, 32.0);
    // A full-viewport clearRect must erase via a Replace draw, not drop the
    // commands recorded before it (which would also be fine here, but a bug
    // would show up as retained red).
    canvas.clear_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.submit();

    assert!(
        !any_opaque(&canvas),
        "a full-viewport clearRect must erase earlier same-frame draws"
    );
}

#[test]
fn fill_text_draws_or_reports_unavailable() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 1.0, 1.0, 1.0]);
    canvas.set_text_size(48.0);
    canvas.fill_text("M", 8.0, 56.0, None);
    canvas.submit();

    if canvas.text_unavailable() {
        eprintln!("skipping text pixel assertion: no system font available");
        return;
    }
    assert!(
        any_opaque(&canvas),
        "fill_text(\"M\") must leave glyph coverage"
    );
}

#[test]
fn text_block_height_tracks_font_size() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.set_text_size(20.0);
    let single = canvas.text_block_height("one line", Some(200.0));
    if canvas.text_unavailable() {
        return;
    }
    assert!((single - 20.0).abs() < 0.01, "block height {single}");
}

#[test]
fn resize_clears_and_changes_dimensions() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut canvas = Canvas::new(&device, &queue, SIZE, SIZE);
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, SIZE as f32, SIZE as f32);
    canvas.submit();

    canvas.resize(&device, &queue, 32, 48);
    assert_eq!((canvas.width(), canvas.height()), (32, 48));
    assert!(
        !any_opaque(&canvas),
        "resize must zero-initialise the new texture immediately"
    );
    canvas.submit();
    assert!(!any_opaque(&canvas), "resize discards content like JS");
}

#[test]
fn submit_is_idempotent_when_nothing_is_recorded() {
    let Some(mut canvas) = canvas(SIZE, SIZE) else {
        return;
    };
    canvas.begin_frame();
    canvas.submit();
    canvas.submit();
    assert!(!any_opaque(&canvas));
}

// ---------------------------------------------------------------------------
// Path/affine unit tests (no GPU)
// ---------------------------------------------------------------------------

#[test]
fn affine_operations_apply_in_canvas_order() {
    // translate(10, 0) then rotate(90°): a point at (1, 0) rotates first, so
    // (1, 0) → (0, 1) → then the translation applies to the rotated space
    // origin: (0 + 10, 1 + 0) = (10, 1).
    let transform =
        Affine::translate(10.0, 0.0).multiply(&Affine::rotate(std::f32::consts::FRAC_PI_2));
    let point = transform.transform_point([1.0, 0.0]);
    assert!((point[0] - 10.0).abs() < 1e-5, "{point:?}");
    assert!((point[1] - 1.0).abs() < 1e-5, "{point:?}");
}

#[test]
fn arc_sweep_matches_canvas_semantics() {
    let quarter = path::arc_sweep(0.0, std::f32::consts::FRAC_PI_2, false);
    assert!((quarter - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    let full_ccw = path::arc_sweep(0.0, std::f32::consts::TAU, true);
    assert!((full_ccw + std::f32::consts::TAU).abs() < 1e-5);
    assert!(path::arc_is_full_turn(0.0, std::f32::consts::TAU, false));
    assert!(!path::arc_is_full_turn(0.0, 1.0, false));
}

#[test]
fn ear_clipping_covers_convex_polygons() {
    let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    let triangles = path::ear_clip(&square);
    assert_eq!(triangles.len(), 6, "two triangles");
    let area: f32 = triangles
        .chunks_exact(3)
        .map(|triangle| path::signed_area(triangle).abs() / 2.0)
        .sum();
    assert!((area - 100.0).abs() < 1e-3, "area {area}");
}

#[test]
fn ear_clipping_handles_concave_polygons() {
    // An L-shape: area = 75.
    let shape = [
        [0.0, 0.0],
        [10.0, 0.0],
        [10.0, 5.0],
        [5.0, 5.0],
        [5.0, 10.0],
        [0.0, 10.0],
    ];
    let triangles = path::ear_clip(&shape);
    let area: f32 = triangles
        .chunks_exact(3)
        .map(|triangle| path::signed_area(triangle).abs() / 2.0)
        .sum();
    assert!((area - 75.0).abs() < 1e-3, "area {area}");
}

#[test]
fn stroke_geometry_makes_a_quad_per_segment() {
    let triangles = path::stroke_geometry(&[[0.0, 0.0], [10.0, 0.0]], false, 1.0);
    assert_eq!(triangles.len(), 6, "one segment, two triangles");
    let area: f32 = triangles
        .chunks_exact(3)
        .map(|triangle| path::signed_area(triangle).abs() / 2.0)
        .sum();
    // 10 px long, 2 px wide.
    assert!((area - 20.0).abs() < 1e-3, "area {area}");
}

#[test]
fn transformed_bounds_include_rotation() {
    let bounds = path::transformed_bounds(
        0.0,
        0.0,
        10.0,
        20.0,
        &Affine::rotate(std::f32::consts::FRAC_PI_2),
    );
    // Rotating a 10×20 rect by 90° swaps its extents.
    assert!((bounds[2] - bounds[0] - 20.0).abs() < 1e-4, "{bounds:?}");
    assert!((bounds[3] - bounds[1] - 10.0).abs() < 1e-4, "{bounds:?}");
}

#[test]
fn circle_subpath_triangulates_as_a_fan() {
    let mut path = path::Path::default();
    path.move_to([10.0, 0.0]);
    let points = path::flatten_arc(0.0, 0.0, 10.0, 0.0, std::f32::consts::TAU, true);
    for point in points {
        path.line_to(point);
    }
    path.set_last_circle(path::Circle {
        cx: 0.0,
        cy: 0.0,
        r: 10.0,
    });
    let triangles = path::triangulate_subpath(&path.subpaths[0]);
    assert!(triangles.len() >= 3 * 8);
    let area: f32 = triangles
        .chunks_exact(3)
        .map(|triangle| path::signed_area(triangle).abs() / 2.0)
        .sum();
    // A 32-gon inscribed in a radius-10 circle ≈ π·100.
    assert!(area > 290.0 && area < 315.0, "area {area}");
}

// ---------------------------------------------------------------------------
// Audio payload
// ---------------------------------------------------------------------------

#[test]
fn synthetic_audio_frame_is_usable() {
    let frame = AudioFrame::synthetic();
    assert!(frame.features.rms > 0.0);
    assert_eq!(frame.features.buffer.len(), 1024);
    assert!(frame.wave.iter().any(|sample| *sample != 0.0));
}

// ---------------------------------------------------------------------------
// Module registry / props
// ---------------------------------------------------------------------------

fn all_prop_descriptors() -> Vec<(&'static str, Vec<TwoDProp>)> {
    all_modules()
        .iter()
        .map(|module| (module.meta().name, module.props()))
        .collect()
}

#[test]
fn registry_has_the_fifteen_plan_modules() {
    let modules = all_modules();
    assert_eq!(modules.len(), 15);
    let names: Vec<&'static str> = modules.iter().map(|module| module.meta().name).collect();
    assert_eq!(
        names,
        vec![
            "Ball",
            "Text",
            "Polygon",
            "Texture 2D",
            "Buffer",
            "Concentrics",
            "X Drips",
            "Line",
            "Pixelate",
            "Waveform",
            "Grid Stretch",
            "Bar",
            "Webcam",
            "Smear",
            "Counter",
        ]
    );
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "module names must be unique");
}

#[test]
fn every_module_declares_valid_prop_defaults() {
    for (name, props) in all_prop_descriptors() {
        for prop in &props {
            match prop.prop_type {
                "int" | "float" => {
                    assert!(
                        prop.default.is_number(),
                        "{name}.{}: default must be a number",
                        prop.name
                    );
                }
                "bool" => assert!(
                    prop.default.is_boolean(),
                    "{name}.{}: default must be a bool",
                    prop.name
                ),
                "text" => assert!(
                    prop.default.is_string(),
                    "{name}.{}: default must be a string",
                    prop.name
                ),
                "enum" => {
                    let default = prop.default.as_str().unwrap_or_else(|| {
                        panic!("{name}.{}: enum default must be a string", prop.name)
                    });
                    let members = prop
                        .enum_members
                        .as_ref()
                        .unwrap_or_else(|| panic!("{name}.{}: enum without members", prop.name));
                    assert!(
                        members.iter().any(|member| member.value == default),
                        "{name}.{}: default {default:?} is not a member",
                        prop.name
                    );
                }
                "color" => {
                    let colour = prop.default.as_object().unwrap_or_else(|| {
                        panic!("{name}.{}: colour must be an object", prop.name)
                    });
                    for channel in ["r", "g", "b", "a"] {
                        assert!(
                            colour.get(channel).is_some_and(Value::is_number),
                            "{name}.{}: colour needs {channel}",
                            prop.name
                        );
                    }
                }
                "tween" => {
                    let tween = prop.default.as_object().unwrap_or_else(|| {
                        panic!("{name}.{}: tween default must be an object", prop.name)
                    });
                    assert!(
                        tween.get("data").is_some_and(Value::is_array),
                        "{name}.{}: tween needs a palette",
                        prop.name
                    );
                }
                "vec2" => {
                    let vector = prop.default.as_array().unwrap_or_else(|| {
                        panic!("{name}.{}: vec2 default must be an array", prop.name)
                    });
                    assert_eq!(
                        vector.len(),
                        2,
                        "{name}.{}: vec2 needs two entries",
                        prop.name
                    );
                }
                // `texture`/`event` props have no default value.
                "texture" | "event" => {}
                other => panic!("{name}.{}: unexpected prop type {other}", prop.name),
            }
            // Ranges are sane when present.
            if let (Some(min), Some(max)) = (prop.min, prop.max) {
                assert!(min <= max, "{name}.{}: min > max", prop.name);
            }
        }
    }
}

#[test]
fn ball_props_match_the_js() {
    let module = all_modules()
        .into_iter()
        .find(|module| module.meta().name == "Ball")
        .expect("Ball is registered");
    let props = module.props();
    let amount = props.iter().find(|prop| prop.name == "amount").unwrap();
    assert_eq!(amount.prop_type, "int");
    assert_eq!(amount.default, json!(10));
    assert_eq!(amount.min, Some(1.0));
    assert_eq!(amount.max, Some(300.0));
    assert_eq!(amount.label, Some("Amount"));
    let colour = props.iter().find(|prop| prop.name == "color").unwrap();
    assert_eq!(colour.prop_type, "tween");
    assert_eq!(colour.component, Some("PaletteControl"));
    let meta = module.meta();
    assert_eq!(meta.audio_features, &["zcr", "rms"]);
    assert_eq!(meta.version, "1.0.1");
}

#[test]
fn module_metadata_matches_the_js() {
    let modules = all_modules();
    let find = |name: &str| {
        modules
            .iter()
            .find(|module| module.meta().name == name)
            .unwrap_or_else(|| panic!("{name} is registered"))
            .meta()
    };
    assert_eq!(find("X Drips").category, Some("effects"));
    assert_eq!(find("Pixelate").category, Some("effects"));
    assert_eq!(find("Grid Stretch").category, Some("effects"));
    assert_eq!(find("Smear").category, Some("effects"));
    assert_eq!(find("Ball").category, None);
    assert_eq!(find("Waveform").audio_features, &["buffer"]);
    assert_eq!(find("Smear").audio_features, &["rms", "energy"]);
    assert_eq!(find("Texture 2D").author, "NERDDISCO");
}

#[test]
fn enum_props_match_the_js_members() {
    let props: Vec<(&'static str, Vec<TwoDProp>)> = all_prop_descriptors();
    let waveform = props
        .iter()
        .find(|(name, _)| *name == "Waveform")
        .map(|(_, props)| props)
        .unwrap();
    let windowing = waveform
        .iter()
        .find(|prop| prop.name == "windowing")
        .unwrap();
    let members = windowing.enum_members.as_ref().unwrap();
    assert_eq!(members.len(), 5);
    assert_eq!(members[0].value, "rect");
    assert_eq!(windowing.default, json!("hanning"));

    let texture = props
        .iter()
        .find(|(name, _)| *name == "Texture 2D")
        .map(|(_, props)| props)
        .unwrap();
    let constrain = texture
        .iter()
        .find(|prop| prop.name == "constrain")
        .unwrap();
    assert_eq!(constrain.enum_members.as_ref().unwrap().len(), 3);

    let image = props
        .iter()
        .find(|(name, _)| *name == "Webcam")
        .map(|(_, props)| props)
        .unwrap();
    let position = image.iter().find(|prop| prop.name == "position").unwrap();
    assert_eq!(position.prop_type, "vec2");
    assert_eq!(position.default, json!([0.5, 0.5]));
}

#[test]
fn text_props_match_the_js() {
    let module = all_modules()
        .into_iter()
        .find(|module| module.meta().name == "Text")
        .unwrap();
    let props = module.props();
    let names: Vec<&str> = props.iter().map(|prop| prop.name).collect();
    assert_eq!(
        names,
        vec![
            "text",
            "size",
            "offsetX",
            "offsetY",
            "strokeSize",
            "font",
            "weight",
            "fill",
            "fillColor",
            "stroke",
            "strokeColor",
        ]
    );
    let font = props.iter().find(|prop| prop.name == "font").unwrap();
    assert_eq!(font.component, Some("FontControl"));
    assert_eq!(font.default, json!("sans-serif"));
    let weight = props.iter().find(|prop| prop.name == "weight").unwrap();
    assert_eq!(weight.default, json!("bold"));
}

// ---------------------------------------------------------------------------
// Module smoke tests
// ---------------------------------------------------------------------------

/// Run one module through its lifecycle on a 128×128 canvas and require
/// opaque output.
fn smoke(module_name: &str, overrides: &[(&str, Value)], seed: bool, input: Option<[u8; 4]>) {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut module = all_modules()
        .into_iter()
        .find(|module| module.meta().name == module_name)
        .unwrap_or_else(|| panic!("{module_name} is registered"));

    let mut canvas = Canvas::new(&device, &queue, MODULE_SIZE, MODULE_SIZE);
    if seed {
        // Self-sampling modules need content to copy (the JS relies on the
        // group canvas already having some).
        canvas.begin_frame();
        canvas.set_fill_style([0.25, 0.5, 0.75, 1.0]);
        canvas.fill_rect(0.0, 0.0, MODULE_SIZE as f32, MODULE_SIZE as f32);
        canvas.submit();
    }

    let mut props = default_props(&module.props());
    if let Some(object) = props.as_object_mut() {
        for (key, value) in overrides {
            object.insert((*key).to_owned(), value.clone());
        }
    }

    let audio = AudioFrame::synthetic();
    let mut module_context = ModuleContext {
        canvas_width: MODULE_SIZE,
        canvas_height: MODULE_SIZE,
        delta: 1234.5,
        audio: &audio,
        frame: 1,
        props: &props,
        device: &device,
        queue: &queue,
    };
    module.init(&mut module_context);
    module.resize(&mut module_context);
    module.update(&mut module_context);

    let input_texture =
        input.map(|rgba| solid_texture(&device, &queue, MODULE_SIZE, MODULE_SIZE, rgba));

    canvas.begin_frame();
    {
        let mut draw_context = DrawContext {
            canvas: &mut canvas,
            props: &props,
            audio: &audio,
            delta: 1234.5,
            frame: 1,
            device: &device,
            queue: &queue,
            input_texture: input_texture.as_ref().map(|(_, view)| view),
            input_size: input_texture.as_ref().map(|_| (MODULE_SIZE, MODULE_SIZE)),
            is_gallery: false,
        };
        module.draw(&mut draw_context);
    }
    canvas.submit();

    if module_name == "Text" && canvas.text_unavailable() {
        eprintln!("skipping Text smoke assertion: no system font available");
        return;
    }
    assert!(
        any_opaque(&canvas),
        "{module_name} must draw something (seeded: {seed})"
    );
}

#[test]
fn smoke_ball() {
    smoke("Ball", &[], false, None);
}

#[test]
fn smoke_text() {
    smoke("Text", &[("text", json!("MODV"))], false, None);
}

#[test]
fn smoke_polygon() {
    // The synthetic fixture's RMS (~0.5) drives the audio-scaled radius far
    // outside a 128 px canvas; pin intensity to 0 so the default shape is
    // visible. (Real music RMS keeps the scaled radius small.)
    smoke("Polygon", &[("intensity", json!(0))], false, None);
}

#[test]
fn smoke_texture_2d() {
    smoke("Texture 2D", &[], false, Some([255, 0, 0, 255]));
}

#[test]
fn smoke_buffer() {
    smoke("Buffer", &[("bufferOpacity", json!(0.5))], true, None);
}

#[test]
fn smoke_concentrics() {
    smoke("Concentrics", &[], false, None);
}

#[test]
fn smoke_x_drips() {
    smoke("X Drips", &[], false, None);
}

#[test]
fn smoke_line() {
    smoke("Line", &[], false, None);
}

#[test]
fn smoke_pixelate() {
    smoke("Pixelate", &[], true, None);
}

#[test]
fn smoke_waveform() {
    smoke("Waveform", &[], false, None);
}

#[test]
fn smoke_grid_stretch() {
    smoke("Grid Stretch", &[], true, None);
}

/// Run one module over a canvas seeded with a red/blue half-and-half frame
/// (submitted before the module's frame) and return the resulting pixels.
fn self_sampling_output(module_name: &str, overrides: &[(&str, Value)]) -> Option<Vec<u8>> {
    let (device, queue) = gpu()?;
    let mut module = all_modules()
        .into_iter()
        .find(|module| module.meta().name == module_name)
        .unwrap_or_else(|| panic!("{module_name} is registered"));

    let mut canvas = Canvas::new(&device, &queue, MODULE_SIZE, MODULE_SIZE);
    // Frame N-1: left half red, right half blue.
    canvas.begin_frame();
    canvas.set_fill_style([1.0, 0.0, 0.0, 1.0]);
    canvas.fill_rect(0.0, 0.0, MODULE_SIZE as f32 / 2.0, MODULE_SIZE as f32);
    canvas.set_fill_style([0.0, 0.0, 1.0, 1.0]);
    canvas.fill_rect(
        MODULE_SIZE as f32 / 2.0,
        0.0,
        MODULE_SIZE as f32 / 2.0,
        MODULE_SIZE as f32,
    );
    canvas.submit();

    let mut props = default_props(&module.props());
    if let Some(object) = props.as_object_mut() {
        for (key, value) in overrides {
            object.insert((*key).to_owned(), value.clone());
        }
    }
    let audio = AudioFrame::synthetic();
    let mut module_context = ModuleContext {
        canvas_width: MODULE_SIZE,
        canvas_height: MODULE_SIZE,
        delta: 1234.5,
        audio: &audio,
        frame: 1,
        props: &props,
        device: &device,
        queue: &queue,
    };
    module.init(&mut module_context);
    module.resize(&mut module_context);
    module.update(&mut module_context);

    canvas.begin_frame();
    {
        let mut draw_context = DrawContext {
            canvas: &mut canvas,
            props: &props,
            audio: &audio,
            delta: 1234.5,
            frame: 1,
            device: &device,
            queue: &queue,
            input_texture: None,
            input_size: None,
            is_gallery: false,
        };
        module.draw(&mut draw_context);
    }
    canvas.submit();
    Some(canvas.read_pixels())
}

/// The three self-sampling modules must read the **snapshot** of the canvas
/// (frame N-1), not the live target they are writing. A red/blue seeded frame
/// therefore survives their draw; an undefined live read would not.
#[test]
fn self_sampling_modules_render_the_snapshot() {
    let cases: &[(&str, &[(&str, Value)])] = &[
        ("Pixelate", &[]),
        ("Grid Stretch", &[("intensity", json!(0))]),
        ("Buffer", &[("bufferOpacity", json!(1.0))]),
    ];
    for (name, overrides) in cases {
        let Some(pixels) = self_sampling_output(name, overrides) else {
            return;
        };
        let red = pixels
            .chunks(4)
            .any(|p| p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60);
        let blue = pixels
            .chunks(4)
            .any(|p| p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60);
        assert!(
            red && blue,
            "{name} did not reproduce the seeded frame (red: {red}, blue: {blue})"
        );
    }
}

#[test]
fn smoke_bar() {
    smoke("Bar", &[], false, None);
}

#[test]
fn smoke_webcam() {
    smoke("Webcam", &[], false, Some([0, 255, 0, 255]));
}

#[test]
fn smoke_smear() {
    smoke("Smear", &[], true, None);
}

#[test]
fn smoke_counter() {
    smoke("Counter", &[], false, None);
}

#[test]
fn modules_that_need_input_draw_nothing_without_it() {
    for module_name in ["Texture 2D", "Webcam"] {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let mut module = all_modules()
            .into_iter()
            .find(|module| module.meta().name == module_name)
            .unwrap();
        let props = default_props(&module.props());
        let audio = AudioFrame::synthetic();
        let mut canvas = Canvas::new(&device, &queue, MODULE_SIZE, MODULE_SIZE);
        canvas.begin_frame();
        let mut context = DrawContext {
            canvas: &mut canvas,
            props: &props,
            audio: &audio,
            delta: 1000.0,
            frame: 1,
            device: &device,
            queue: &queue,
            input_texture: None,
            input_size: None,
            is_gallery: false,
        };
        module.draw(&mut context);
        canvas.submit();
        assert!(
            !any_opaque(&canvas),
            "{module_name} must skip its draw without an input texture"
        );
    }
}

#[test]
fn counter_advances_and_draws_by_default() {
    let module = all_modules()
        .into_iter()
        .find(|module| module.meta().name == "Counter")
        .unwrap();
    let props = default_props(&module.props());
    // `prop_i64` rounds non-integer JSON numbers, so 0.5 becomes 1.
    assert_eq!(prop_i64(&props, "input1", 0), 1);
    assert_eq!(
        props.get("input1").and_then(Value::as_f64),
        Some(0.5),
        "Counter input1 default"
    );
}
