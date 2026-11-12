//! The headless code path: render into an offscreen target, optionally save a
//! PNG, and always print the JSON frame report (headless implies `--bench`).

use std::time::{Duration, Instant};

use modv_render::headless::{create_headless, read_back_rgba8};
use modv_render::wgpu;
use modv_render::{Engine, FrameStats};

use modv_renderers::point10::chromatic_line::{ChromaticLine, SyntheticSine};

use crate::cli::Cli;
use crate::report::{AdapterSummary, report_json};

/// Render offscreen for `cli.headless_frames()` frames.
pub fn run(cli: Cli, engine: Engine, adapter: AdapterSummary) -> Result<(), String> {
    let width = cli.width.max(1);
    let height = cli.height.max(1);
    let target = create_headless(&engine, width, height);
    let mut scene = ChromaticLine::new(
        &engine.device,
        &engine.queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
    scene.resize(width, height);
    let mut fixture =
        (cli.audio_fixture > 0.0).then(|| SyntheticSine::new(cli.audio_fixture as f32));

    let frames = cli.headless_frames();
    let mut stats = FrameStats::start();
    let mut last_frame = Instant::now();
    for frame_index in 0..frames {
        let now = Instant::now();
        let dt_duration = if frame_index == 0 {
            Duration::from_secs_f64(1.0 / 60.0)
        } else {
            now.saturating_duration_since(last_frame)
        };
        last_frame = now;
        let dt_seconds = dt_duration.as_secs_f32();

        scene.update(&engine.queue, dt_seconds, &mut fixture);

        let mut encoder = engine
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("headless-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless-frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Same near-black (#0b0b0f) clear as the windowed path.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.043,
                            g: 0.043,
                            b: 0.059,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            scene.render(&mut pass);
        }
        engine.queue.submit(Some(encoder.finish()));
        stats.record_frame(dt_duration);
    }

    if let Some(path) = &cli.screenshot {
        let raw = read_back_rgba8(&engine, &target)
            .map_err(|e| format!("failed to read back the frame: {e}"))?;
        image::save_buffer(path, &raw, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|e| format!("failed to save screenshot {}: {e}", path.display()))?;
        eprintln!("screenshot written to {}", path.display());
        if !cli.bench && !cli.headless {
            return Ok(());
        }
    }

    // Headless mode implies --bench: always print the report on exit.
    let report = stats.finish();
    let json = report_json(&report, (width, height), cli.audio_fixture, &adapter);
    match serde_json::to_string_pretty(&json) {
        Ok(text) => println!("{text}"),
        Err(err) => return Err(format!("failed to serialise the frame report: {err}")),
    }
    eprintln!(
        "frames={} fps={:.2} p99={:.3}ms",
        report.frames, report.frames_per_second, report.p99_ms
    );
    Ok(())
}
