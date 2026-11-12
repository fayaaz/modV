//! The windowed code path, implemented as a winit 0.30 `ApplicationHandler`.
//!
//! Frame pacing relies on `wgpu::PresentMode::Fifo` (blocking present). The
//! window requests a redraw at the end of each `RedrawRequested` handler, so
//! frames are produced back-to-back at the display's refresh rate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use modv_render::surface::{SurfaceTarget, create_surface};
use modv_render::wgpu;
use modv_render::{Engine, FrameReport, FrameStats};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

use modv_renderers::point10::chromatic_line::{ChromaticLine, SyntheticSine};

use crate::cli::Cli;
use crate::report::{AdapterSummary, report_json};
use crate::supervisor::EngineSupervisor;

/// Run the windowed event loop. Returns an error string on fatal failures
/// (window creation, surface creation, event loop).
pub fn run(cli: Cli, engine: Engine, adapter: AdapterSummary) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("failed to create event loop: {e}"))?;
    let mut app = WindowedApp::new(cli, engine, adapter);
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop failed: {e}"))?;
    // Flush the snapshot on any exit path (window close, bench deadline,
    // engine/shutdown) so a supervisor restart restores the last frame count.
    if let Some(mut supervisor) = app.supervisor.take() {
        eprintln!("engine: INFO flushing state snapshot to disk");
        match supervisor.save_snapshot() {
            Ok(()) => eprintln!("engine: INFO state snapshot flushed"),
            Err(error) => eprintln!("engine: WARN final snapshot failed: {error}"),
        }
    } else {
        eprintln!("engine: WARN no supervisor; state snapshot not flushed");
    }
    if app.failed {
        return Err("fatal error during windowed run (see above)".to_string());
    }
    Ok(())
}

/// State for the windowed run.
struct WindowedApp {
    cli: Cli,
    engine: Option<Engine>,
    surface: Option<SurfaceTarget>,
    scene: Option<ChromaticLine>,
    fixture: Option<SyntheticSine>,
    stats: Option<FrameStats>,
    adapter: AdapterSummary,
    window: Option<Arc<Window>>,
    bench_deadline: Option<Duration>,
    frames_done: u64,
    last_frame: Instant,
    start: Instant,
    done: bool,
    failed: bool,
    supervisor: Option<EngineSupervisor>,
}

// `FrameStats` contains no public Clone; store it in an Option so `finish()`
// can consume it exactly once.

impl WindowedApp {
    fn new(cli: Cli, engine: Engine, adapter: AdapterSummary) -> Self {
        let start = Instant::now();
        let supervisor = cli.socket.as_ref().and_then(|socket_path| {
            match EngineSupervisor::new(socket_path.clone(), cli.state.clone()) {
                Ok(supervisor) => {
                    eprintln!(
                        "engine: INFO control socket listening on {}",
                        socket_path.display()
                    );
                    Some(supervisor)
                }
                Err(error) => {
                    eprintln!(
                        "engine: ERROR could not bind control socket {}: {error}",
                        socket_path.display()
                    );
                    None
                }
            }
        });
        let socket_failed = cli.socket.is_some() && supervisor.is_none();
        Self {
            bench_deadline: cli.bench_deadline(),
            cli,
            engine: Some(engine),
            surface: None,
            scene: None,
            fixture: None,
            stats: None,
            adapter,
            window: None,
            frames_done: 0,
            last_frame: start,
            start,
            done: false,
            failed: false,
            supervisor,
        }
        .with_socket_failure(socket_failed)
    }

    fn with_socket_failure(mut self, failed: bool) -> Self {
        if failed {
            self.failed = true;
        }
        self
    }

    /// Build the window, surface, scene and audio fixture (first `resumed`).
    fn init(&mut self, event_loop: &ActiveEventLoop) {
        let Some(engine) = self.engine.as_ref() else {
            eprintln!("error: engine missing during window initialisation");
            self.failed = true;
            event_loop.exit();
            return;
        };

        let mut attributes = WindowAttributes::default()
            .with_title("modV Engine")
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.cli.width.max(1),
                self.cli.height.max(1),
            ));
        if self.cli.fullscreen {
            // Wayland does not report a primary monitor; fall back to the
            // monitor the window is on (`Borderless(None)`).
            attributes = attributes
                .with_fullscreen(Some(Fullscreen::Borderless(event_loop.primary_monitor())));
        }

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                eprintln!("error: failed to create window: {err}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };

        let surface = match create_surface(engine, Arc::clone(&window)) {
            Ok(surface) => surface,
            Err(err) => {
                eprintln!("error: failed to create surface: {err}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        let mut scene = ChromaticLine::new(&engine.device, &engine.queue, surface.format());
        let (width, height) = surface.dimensions();
        scene.resize(width, height);
        self.fixture = (self.cli.audio_fixture > 0.0)
            .then(|| SyntheticSine::new(self.cli.audio_fixture as f32));
        self.surface = Some(surface);
        self.scene = Some(scene);
        self.window = Some(window);
        self.stats = Some(FrameStats::start());
        self.last_frame = Instant::now();
        self.start = self.last_frame;
        event_loop.set_control_flow(ControlFlow::Poll);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Render one frame (or skip it when the surface is not ready).
    fn draw_frame(&mut self, event_loop: &ActiveEventLoop) {
        if self.done || self.failed {
            return;
        }
        let now = Instant::now();
        let dt_duration = if self.frames_done == 0 {
            Duration::from_secs_f64(1.0 / 60.0)
        } else {
            now.saturating_duration_since(self.last_frame)
        };
        self.last_frame = now;
        let dt_seconds = dt_duration.as_secs_f32();

        let Some(engine) = self.engine.as_ref() else {
            return;
        };
        if let Some(scene) = &mut self.scene {
            scene.update(&engine.queue, dt_seconds, &mut self.fixture);
        }

        let Some(surface) = &mut self.surface else {
            return;
        };
        let (frame, suboptimal) = match surface.current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Occluded => {
                // Window is behind another window or minimised; nothing to do.
                self.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.reconfigure(&engine.device);
                self.request_redraw();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("error: failed to acquire a surface texture (timeout or validation)");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = engine
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("spike-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spike-frame"),
                // Near-black (#0b0b0f). The legacy output clears to transparent;
                // the spike window is opaque dark instead.
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
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
            if let Some(scene) = self.scene.as_ref() {
                scene.render(&mut pass);
            }
        }
        engine.queue.submit(Some(encoder.finish()));
        engine.queue.present(frame);
        if suboptimal {
            surface.reconfigure(&engine.device);
        }

        if let Some(stats) = &mut self.stats {
            stats.record_frame(dt_duration);
        }
        self.frames_done += 1;

        if let Some(supervisor) = &mut self.supervisor {
            supervisor.note_frame_presented();
            supervisor.poll();
            if supervisor.shutdown_requested() {
                self.finish_and_exit(event_loop);
                return;
            }
        }

        let reached_frames = self
            .cli
            .frames
            .map(|limit| self.frames_done >= limit)
            .unwrap_or(false);
        let reached_deadline = self
            .bench_deadline
            .map(|limit| self.start.elapsed() >= limit)
            .unwrap_or(false);
        if reached_frames || reached_deadline {
            self.finish_and_exit(event_loop);
        } else {
            self.request_redraw();
        }
    }

    /// Ask the window for another redraw, if it still exists.
    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Print the final report and leave the event loop (idempotent).
    fn finish_and_exit(&mut self, event_loop: &ActiveEventLoop) {
        if self.done {
            event_loop.exit();
            return;
        }
        self.done = true;
        let resolution = self
            .surface
            .as_ref()
            .map(SurfaceTarget::dimensions)
            .unwrap_or((self.cli.width, self.cli.height));
        if let Some(stats) = self.stats.take() {
            let report: FrameReport = stats.finish();
            if self.cli.bench {
                let json = report_json(&report, resolution, self.cli.audio_fixture, &self.adapter);
                match serde_json::to_string_pretty(&json) {
                    Ok(text) => println!("{text}"),
                    Err(err) => eprintln!("error: failed to serialise the frame report: {err}"),
                }
            }
            eprintln!(
                "frames={} fps={:.2} p99={:.3}ms",
                report.frames, report.frames_per_second, report.p99_ms
            );
        }
        event_loop.exit();
    }
}

impl ApplicationHandler for WindowedApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() || self.failed {
            return;
        }
        self.init(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => self.finish_and_exit(event_loop),
            WindowEvent::Resized(size) => {
                if let (Some(surface), Some(engine)) = (&mut self.surface, &self.engine) {
                    surface.resize(&engine.device, size.width, size.height);
                }
                if size.width > 0 && size.height > 0 {
                    if let Some(scene) = &mut self.scene {
                        scene.resize(size.width, size.height);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.draw_frame(event_loop),
            _ => {}
        }
    }
}
