//! The windowed code path, implemented as a winit 0.30 `ApplicationHandler`.
//!
//! Each frame: poll the control socket (mutations into the [`Store`]), render
//! the state through the [`Compositor`], present the main texture, and publish
//! a preview frame at ≤ 30 fps. Frame pacing relies on
//! `wgpu::PresentMode::Fifo` (blocking present); the window requests a redraw
//! at the end of each `RedrawRequested` handler, so frames are produced
//! back-to-back at the display's refresh rate.
//!
//! M2 removed the M0 `ChromaticLine`/`SyntheticSine` demo from this path: the
//! window now renders whatever the store describes (an empty state presents
//! the near-black clear). `--audio-fixture` is still parsed for the M0
//! headless/report path but has no effect here.
//!
//! Audio is owned by exactly one side. With the default `--audio-source
//! renderer` the renderer's audio pump commits the full analyser payload
//! (`meyda/SET_BANDS`, WS-9) and the engine only stores it. With
//! `--audio-source pipewire|input` the engine opens an
//! [`modv_media::audio::engine::AudioFeed`] before the loop, pumps it each
//! frame before Input Links are evaluated, and pushes the analysis into the
//! store ([`Store::set_engine_audio`]); a capture failure logs once, stops the
//! feed and hands ownership back to the renderer instead of killing the loop.
//!
//! Each frame also dispatches `scenes/tick` with the real frame delta before
//! rendering (`worker/loop.js:94-111`), but only while performance mode is
//! running or a crossfade is active, so idle frames stay cheap. Input Links
//! are evaluated between the state update and the render, so audio-reactive
//! props are current in the same frame they were computed.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use modv_media::audio::engine::AudioFeed;
use modv_render::surface::{SurfaceTarget, create_surface};
use modv_render::wgpu;
use modv_render::{Engine, FrameReport, FrameStats};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

use crate::cli::Cli;
use crate::compositor::{Compositor, FrameInfo, RowFadeInfo};
use crate::report::{AdapterSummary, report_json};
use crate::scene_preview;
use crate::store_ops::Store;
use crate::supervisor::{EngineSupervisor, PreviewKind};

/// Most preview requests answered in one frame.
///
/// Each answer can block on a GPU read-back (≤ 1 MiB at 512²), so a burst of
/// hover requests rolls over to later frames instead of stalling present. The
/// cap is shared by Gallery thumbnails (`preview/module`) and Scene hover
/// cells (`preview/scene`).
const MAX_PREVIEWS_PER_FRAME: usize = 4;

/// Run the windowed event loop. Returns an error string on fatal failures
/// (window creation, surface creation, event loop).
pub fn run(cli: Cli, engine: Engine, adapter: AdapterSummary) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("failed to create event loop: {e}"))?;
    let mut app = WindowedApp::new(cli, engine, adapter);
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop failed: {e}"))?;
    // Flush the last <100 ms of Input Link deltas so the renderer mirror sees
    // the final values before the socket closes.
    app.store.flush_link_deltas();
    // Release the engine-owned capture (WS-9) on every exit path.
    app.stop_engine_audio();
    // Flush the snapshot on any exit path (window close, bench deadline,
    // engine/shutdown) so a supervisor restart restores the last state.
    if let Some(mut supervisor) = app.supervisor.take() {
        eprintln!("engine: INFO flushing state snapshot to disk");
        match supervisor.save_snapshot(&app.store) {
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
    compositor: Option<Compositor>,
    store: Store,
    stats: Option<FrameStats>,
    adapter: AdapterSummary,
    window: Option<Arc<Window>>,
    bench_deadline: Option<Duration>,
    frames_done: u64,
    /// Frames since the last `metrics/SET_FPS_MEASURE` publish.
    metric_frames: u32,
    last_metric: Instant,
    /// Latest surface size, republished with the status metrics so a late
    /// renderer connection still receives it.
    surface_size: (u32, u32),
    last_frame: Instant,
    start: Instant,
    done: bool,
    failed: bool,
    supervisor: Option<EngineSupervisor>,
    consecutive_timeouts: u32,
    preview_error_logged: bool,
    /// Engine-owned capture (WS-9): `Some` when `--audio-source` is not
    /// `renderer`. `None` means the renderer's pump owns the analysis (the
    /// default, and the fallback when capture could not start).
    audio_feed: Option<AudioFeed>,
    /// One-shot guard for the capture-failure log line.
    audio_error_logged: bool,
}

// `FrameStats` contains no public Clone; store it in an Option so `finish()`
// can consume it exactly once.

impl WindowedApp {
    fn new(cli: Cli, engine: Engine, adapter: AdapterSummary) -> Self {
        let start = Instant::now();
        let mut store =
            Store::with_state_path_and_isf(cli.state.as_deref(), cli.isf_dir.as_deref());
        if let Some(path) = &cli.state {
            match store.restore_from_file(path) {
                Ok(true) => eprintln!(
                    "engine: INFO restored engine state from {} ({} groups, {} modules)",
                    path.display(),
                    store.state().groups.len(),
                    store.state().modules.len()
                ),
                Ok(false) => {
                    eprintln!(
                        "engine: INFO no state snapshot at {}; starting empty",
                        path.display()
                    );
                }
                Err(error) => {
                    eprintln!(
                        "engine: WARN could not load state snapshot {}: {error}",
                        path.display()
                    );
                }
            }

            // The Scene list lives beside the snapshot. The renderer also
            // dispatches these boot actions; running them here keeps a
            // headless/supervised engine usable before the UI connects, and
            // the second pass is idempotent (`ensureDefaultScenes` seeds
            // nothing once the defaults version matches).
            if let Err(error) = store.dispatch("scenes/hydrateScenes", serde_json::json!({})) {
                eprintln!("engine: WARN could not hydrate Scenes: {error}");
            }
            if let Err(error) = store.dispatch("scenes/ensureDefaultScenes", serde_json::json!({}))
            {
                eprintln!("engine: WARN could not seed default Scenes: {error}");
            }
        }
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

        // WS-9: open the engine-owned feed before the loop when the renderer is
        // not the audio owner. A failure is not fatal: the engine runs with the
        // renderer's pump (which keeps committing `meyda/SET_BANDS`).
        let audio_feed = if cli.audio_source.engine_owned() {
            match crate::audio::open_feed(cli.audio_source, cli.audio_device.as_deref()) {
                Ok(feed) => {
                    let device_note = cli
                        .audio_device
                        .as_deref()
                        .map(|device| format!(" (device {device})"))
                        .unwrap_or_default();
                    eprintln!(
                        "engine: INFO audio source {} owns capture{} at {} Hz",
                        cli.audio_source.as_str(),
                        device_note,
                        feed.sample_rate()
                    );
                    Some(feed)
                }
                Err(error) => {
                    eprintln!(
                        "engine: WARN audio source {} unavailable ({error}); falling back to the renderer's audio pump",
                        cli.audio_source.as_str()
                    );
                    None
                }
            }
        } else {
            None
        };

        Self {
            bench_deadline: cli.bench_deadline(),
            cli,
            engine: Some(engine),
            surface: None,
            compositor: None,
            store,
            stats: None,
            adapter,
            window: None,
            frames_done: 0,
            metric_frames: 0,
            last_metric: Instant::now(),
            surface_size: (0, 0),
            last_frame: start,
            start,
            done: false,
            failed: false,
            supervisor,
            consecutive_timeouts: 0,
            preview_error_logged: false,
            audio_feed,
            audio_error_logged: false,
        }
        .with_socket_failure(socket_failed)
    }

    fn with_socket_failure(mut self, failed: bool) -> Self {
        if failed {
            self.failed = true;
        }
        self
    }

    /// Pump the engine-owned feed (WS-9) and publish the frame into the store.
    ///
    /// `dt` is the frame delta the loop hands the Scene tick; the analyser uses
    /// it for its AGC/envelope clocks. The feed's capture source is pumped
    /// non-blockingly, so a stalled device never delays a frame — it surfaces
    /// as an error instead, which stops the feed once and hands audio ownership
    /// back to the renderer's pump (logged once).
    fn pump_engine_audio(&mut self, dt: Duration) {
        let Some(feed) = self.audio_feed.as_mut() else {
            return;
        };

        let snapshot = feed.frame(dt.as_secs_f32());
        let kick = feed.analyser().kick();
        let error = feed.take_error();

        self.store
            .set_engine_audio(&snapshot, kick, self.start.elapsed());

        let Some(error) = error else {
            return;
        };

        if !self.audio_error_logged {
            self.audio_error_logged = true;
            eprintln!(
                "engine: WARN audio capture stopped ({error}); falling back to the renderer's audio pump"
            );
        }
        self.stop_engine_audio();
        self.store.disable_engine_audio();
    }

    /// Stop and release the engine-owned capture, if any (idempotent).
    ///
    /// Logs only when the engine actually owned the analysis, so a feed that
    /// failed at startup (ownership already with the renderer) stays quiet.
    fn stop_engine_audio(&mut self) {
        let Some(mut feed) = self.audio_feed.take() else {
            return;
        };

        feed.stop();
        if self.store.engine_audio_active() {
            eprintln!("engine: INFO audio capture stopped");
        }
    }

    /// Build the window, surface and compositor (first `resumed`).
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

        let mut compositor =
            Compositor::with_registry(&engine.device, &engine.queue, self.store.registry_arc());
        let (width, height) = surface.dimensions();
        compositor.resize(width, height);

        self.surface = Some(surface);
        self.compositor = Some(compositor);
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

        // 0. Engine-owned capture (WS-9): pump the feed and publish the frame
        // into the store before anything reads it. Runs ahead of the control
        // socket poll on purpose — while the engine owns the audio, renderer
        // frames committed by the socket are rejected, so the analysis that
        // drives this frame is the engine's.
        self.pump_engine_audio(dt_duration);

        let Some(engine) = self.engine.as_ref() else {
            return;
        };

        // 1. Apply control-socket mutations before rendering the frame.
        if let Some(supervisor) = &mut self.supervisor {
            supervisor.poll(&mut self.store);
            if supervisor.take_reset_request() {
                if let Some(compositor) = &mut self.compositor {
                    compositor.reset();
                }
            }
        }

        // 1b. Advance the Scene/performance state machine before rendering
        // (`loop.js:94-111`). Cheap when idle: `scenes_tick_due` is false
        // unless performance mode runs or a crossfade is active.
        if self.store.scenes_tick_due() {
            let delta = dt_duration.as_secs_f64() * 1000.0;
            if let Err(error) = self
                .store
                .dispatch("scenes/tick", serde_json::json!({ "delta": delta }))
            {
                eprintln!("engine: WARN scenes/tick failed: {error}");
            }
            if let Some(supervisor) = &mut self.supervisor {
                supervisor.broadcast_deltas(&mut self.store);
            }
        }

        // 1c. Evaluate Input Links against the frame the engine's feed (or the
        // renderer's pump) produced, then render with the updated props
        // (`worker/loop.js` runs the link loop before the renderers). `now` is
        // the monotonic frame clock; it drives the 10 Hz mirror-delta throttle.
        let audio = self.store.audio().clone();
        self.store.update_input_links(&audio, self.start.elapsed());
        if let Some(supervisor) = &mut self.supervisor {
            supervisor.broadcast_deltas(&mut self.store);
        }

        let frame_info = FrameInfo {
            time_seconds: self.start.elapsed().as_secs_f32(),
            frame_index: self.frames_done,
            date: wall_clock_date(self.start.elapsed().as_secs_f32()),
            // Either the renderer's audio pump committed the analyser payload
            // over the control socket (`meyda/SET_BANDS`) or the engine's own
            // feed just wrote it (`set_engine_audio`, WS-9): the store holds
            // the latest frame — bands, damping lanes, the 16-cell grid, wave,
            // FFT magnitudes, sample rate and the Meyda scalars.
            audio,
            row_fades: self
                .store
                .performance_runtime()
                .row_transitions
                .values()
                .map(|transition| RowFadeInfo {
                    row: transition.row,
                    group_id: transition.group_id.clone(),
                    ghost_group_id: transition.ghost_group_id.clone(),
                    from_alpha: transition.from_alpha,
                    target_alpha: transition.target_alpha,
                    progress: transition.progress(),
                })
                .collect(),
        };

        // 2. Render the state into the compositor's main texture.
        if let Some(compositor) = &mut self.compositor {
            if let Err(error) = compositor.render_frame(self.store.state(), &frame_info) {
                eprintln!("error: compositor frame failed: {error}");
                self.failed = true;
                self.fail_pending_previews();
                event_loop.exit();
                return;
            }
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
            wgpu::CurrentSurfaceTexture::Timeout => {
                // Transient: the compositor did not signal the acquire fence in
                // time (GPU contention). Skip the frame; only repeated
                // timeouts in a row are fatal.
                self.consecutive_timeouts = self.consecutive_timeouts.saturating_add(1);
                if self.consecutive_timeouts == 1 || self.consecutive_timeouts % 60 == 0 {
                    eprintln!(
                        "engine: WARN surface acquire timed out ({} in a row); skipping frame",
                        self.consecutive_timeouts
                    );
                }
                if self.consecutive_timeouts >= 120 {
                    eprintln!("error: surface acquire timed out 120 times in a row; engine stops");
                    self.failed = true;
                    event_loop.exit();
                } else {
                    self.request_redraw();
                }
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("error: failed to acquire a surface texture (validation error)");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };

        self.consecutive_timeouts = 0;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = engine
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("engine-frame"),
            });
        if let Some(compositor) = &mut self.compositor {
            compositor.present(&mut encoder, &view, surface.format());
        }
        engine.queue.submit(Some(encoder.finish()));
        engine.queue.present(frame);
        if suboptimal {
            surface.reconfigure(&engine.device);
        }

        // 2b. Answer queued preview requests (`preview/module` Gallery
        // thumbnails and `preview/scene` Scene hover cells). Bounded per frame
        // so a burst of hover requests cannot stall present; the rest stay
        // queued for the next frame. Answered after the present so the
        // read-back cost lands between frames.
        self.answer_previews();

        // 3. Publish a preview frame at the supervisor's cadence (≤ 30 fps).
        if let Some(supervisor) = &mut self.supervisor {
            if supervisor.preview_due() && self.compositor.is_some() {
                let mut preview = None;
                if let Some(ring) = supervisor.preview_ring_mut() {
                    if let Some(compositor) = &mut self.compositor {
                        preview = compositor.read_preview(ring).unwrap_or_else(|error| {
                            if !self.preview_error_logged {
                                self.preview_error_logged = true;
                                eprintln!("engine: WARN preview read-back failed: {error}");
                            }
                            None
                        });
                    }
                }
                if let Some(frame) = preview {
                    supervisor.publish_preview(frame);
                }
            }
        }

        if let Some(stats) = &mut self.stats {
            stats.record_frame(dt_duration);
        }
        self.frames_done += 1;

        // Publish the UI status metrics once a second: the legacy worker owned
        // `metrics/SET_FPS_MEASURE` and `size/SET_SIZE`, so engine mode showed
        // "0 FPS" without this.
        self.metric_frames += 1;
        let metric_now = Instant::now();
        let metric_elapsed = metric_now.saturating_duration_since(self.last_metric);
        if metric_elapsed >= Duration::from_secs(1) {
            let fps = self.metric_frames as f64 / metric_elapsed.as_secs_f64();
            self.store
                .push_delta("metrics/SET_FPS_MEASURE", serde_json::json!(fps));
            let (width, height) = self.surface_size;
            if width > 0 && height > 0 {
                self.store.push_delta(
                    "size/SET_SIZE",
                    serde_json::json!({ "width": width, "height": height, "dpr": 1.0 }),
                );
            }
            self.metric_frames = 0;
            self.last_metric = metric_now;
            if let Some(supervisor) = &mut self.supervisor {
                supervisor.broadcast_deltas(&mut self.store);
            }
        }

        if let Some(supervisor) = &mut self.supervisor {
            supervisor.note_frame_presented(&self.store);
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

    /// Answer up to [`MAX_PREVIEWS_PER_FRAME`] queued preview requests,
    /// leaving the rest for the next frame.
    ///
    /// The supervisor only queues requests (it has no compositor), and the
    /// reply can block on a GPU read-back, so the work is capped here; the cap
    /// is shared between Gallery thumbnails (`preview/module`) and Scene hover
    /// cells (`preview/scene`). Before the window/compositor exist the queue
    /// simply waits; when the run stops, [`Self::fail_pending_previews`]
    /// answers whatever is left.
    fn answer_previews(&mut self) {
        let (Some(supervisor), Some(compositor)) = (&mut self.supervisor, &mut self.compositor)
        else {
            return;
        };

        let mut requests = supervisor.pending_previews();
        if requests.is_empty() {
            return;
        }
        // Keep the leftover requests queued, in their original order.
        let overflow = if requests.len() > MAX_PREVIEWS_PER_FRAME {
            requests.split_off(MAX_PREVIEWS_PER_FRAME)
        } else {
            Vec::new()
        };
        supervisor.requeue_previews(overflow);

        let registry = self.store.registry_arc();
        for request in requests {
            let result = match &request.kind {
                PreviewKind::Module { module_name } => compositor.render_module_preview(
                    registry.as_ref(),
                    module_name,
                    request.width,
                    request.height,
                ),
                PreviewKind::Scene {
                    scene_id,
                    row_index,
                } => scene_preview::render_scene_preview(
                    compositor,
                    &self.store,
                    scene_id,
                    *row_index,
                    request.width,
                    request.height,
                ),
            };
            supervisor.complete_preview(request, result);
        }
    }

    /// Answer every queued preview request with an error (the engine is
    /// stopping, so no further frame will render it).
    fn fail_pending_previews(&mut self) {
        if let Some(supervisor) = &mut self.supervisor {
            supervisor.reject_pending_previews(
                "engine_shutting_down",
                "the engine stopped before rendering the preview",
            );
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
        self.fail_pending_previews();
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
                    if let Some(compositor) = &mut self.compositor {
                        compositor.resize(size.width, size.height);
                    }
                    let dpr = self
                        .window
                        .as_ref()
                        .map(|window| window.scale_factor())
                        .unwrap_or(1.0);
                    self.surface_size = (size.width, size.height);
                    self.store.push_delta(
                        "size/SET_SIZE",
                        serde_json::json!({
                            "width": size.width,
                            "height": size.height,
                            "dpr": dpr,
                        }),
                    );
                }
            }
            WindowEvent::RedrawRequested => self.draw_frame(event_loop),
            _ => {}
        }
    }
}

/// `iDate`: `[year, month (0-based), weekday (0 = Sunday), monotonic seconds]`,
/// built from the system clock exactly like the JS `Date` getters
/// (`renderers/shader/index.js:98-121`). Calendar conversion is Howard
/// Hinnant's `civil_from_days` over Unix days, so it works without a
/// chrono/time dependency.
fn wall_clock_date(monotonic_seconds: f32) -> [f32; 4] {
    let unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = unix_seconds.div_euclid(86_400);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };

    // 1970-01-01 was a Thursday; JS `getDay()` is 0 = Sunday.
    let weekday = (days + 4).rem_euclid(7);

    [
        year as f32,
        (month - 1) as f32,
        weekday as f32,
        monotonic_seconds,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_date_has_the_shader_uniform_shape() {
        let date = wall_clock_date(12.5);
        // Year is sane for this project's lifetime, month is 0-based.
        assert!(date[0] >= 2024.0 && date[0] < 2100.0, "year {}", date[0]);
        assert!((0.0..12.0).contains(&date[1]), "month {}", date[1]);
        assert!((0.0..7.0).contains(&date[2]), "weekday {}", date[2]);
        assert!(date[2].fract() == 0.0, "weekday is an integer");
        assert_eq!(date[3], 12.5);
    }
}
