//! WS-10 — engine-owned video capture: cameras, video files, screen nodes.
//!
//! `modv_media::video` owns the GStreamer graphs ([`VideoHub`], one
//! [`modv_media::video::VideoSource`] per source). This module is the engine's
//! adapter around it:
//!
//! * [`MediaHub`] keeps the **start order** (the order the renderer asked for
//!   sources in) because the Webcam module's 1-based `camera` prop indexes that
//!   list, exactly like `selectWebcamSource` (`media-inputs.js:150-158`) indexed
//!   `mediaStream.selectedVideoSources`;
//! * it drains the newest frame of every source into a **reused**
//!   [`VideoFrame`] (allocation-free once the source's two-buffer pool is warm)
//!   and hands the compositor `(source id, frame, is_new)` triples;
//! * it polls source health once a second and restarts a source that errored,
//!   ended, or disappeared, with an exponential backoff;
//! * it turns the hub's `VideoSourceStatus` into the `media/status` wire shape.
//!
//! The engine owns the sources, not the Scenes: a source keeps running when the
//! Webcam module that draws it is deleted, when another Scene is applied, and
//! across `engine/restore`. Only `media/stop-source`, `engine/shutdown` and
//! process exit stop a source — the UI owns the capture lifetime, the same way
//! the renderer's `mediaStream` owned it in JS.
//!
//! ## Startup
//!
//! [`init`] calls [`modv_media::video::ensure_gst`] once (GStreamer is
//! idempotent and process-wide) and logs the linked version. A failure is not
//! fatal: every `media/start-*` then answers `media_unavailable`, and the rest
//! of the engine (rendering, audio, control socket) is unaffected.
//!
//! ## Wire contract (answered by the supervisor)
//!
//! | method | params | result |
//! | --- | --- | --- |
//! | `media/list-cameras` | — | `{ cameras: [{ id, name, path, caps: [{ width, height, fps }] }] }` |
//! | `media/start-camera` | `{ sourceId, cameraId, width, height, fps }` | `{ sourceId, index }` |
//! | `media/start-file` | `{ sourceId, path, width, height, fps, loop }` | `{ sourceId, index }` |
//! | `media/start-screen` | `{ sourceId, nodeId }` | `{ sourceId, index }` |
//! | `media/start-test-pattern` | `{ sourceId, pattern, width, height, fps }` | `{ sourceId, index }` (diagnostic) |
//! | `media/stop-source` | `{ sourceId }` | `{ sourceId }` |
//! | `media/status` | — | `{ sources: [{ sourceId, kind, index, state, width, height, frames, dropped, lastError }] }` |
//!
//! `index` is 1-based and counts the **start order of the running sources**;
//! the Webcam module resolves its `camera` prop against
//! [`MediaHub::select_source_id`]. The list is all running sources (cameras,
//! files, screens, test patterns) rather than cameras only: the legacy
//! renderer's Webcam module also fell back to the primary `video` element when
//! no webcam was selected, and a single ordered list keeps the prop and the
//! stop/start indexes unambiguous. (The renderer's `reconcileCameraIndex`
//! therefore compares against the engine's own index, not a cameras-only one.)
//!
//! `state` is the renderer's vocabulary (`engine-media.js`
//! `MEDIA_SOURCE_STATE_LABELS`): `running`, `starting`, `stopped` or `error`.
//! A source the engine is retrying reports `starting` (with the failure in
//! `lastError`), never `error`, so the renderer's own camera reconcile does not
//! race the engine's reconnect loop.
//!
//! Start failures are `Response::err` with `media_unavailable` (GStreamer did
//! not initialise), `media_plugin_missing` (a GStreamer element is not
//! installed) or `media_start_failed` (the device/file/node refused to start).
//! None of them stop the engine or drop the other sources.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use modv_media::video::{
    self, CameraInfo, ScreenSupport, VideoError, VideoFrame, VideoHub, VideoSourceConfig,
    VideoSourceKind, VideoSourceState, list_cameras,
};
use modv_render::wgpu;

/// How often [`MediaHub::poll_health`] checks every source.
pub const HEALTH_INTERVAL: Duration = Duration::from_secs(1);

/// Delay before the first restart attempt of a failed source.
pub const RESTART_BASE_DELAY: Duration = Duration::from_millis(500);

/// Largest restart delay (a camera that stays unplugged is retried every 8 s).
pub const RESTART_MAX_DELAY: Duration = Duration::from_secs(8);

/// A `media/start-*` failure, mapped to a wire error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// GStreamer could not be initialised (the engine runs without capture).
    Unavailable(String),
    /// A GStreamer element the source needs is not installed.
    PluginMissing(String),
    /// The pipeline refused to start (busy device, dead node, bad path).
    StartFailed(String),
}

impl MediaError {
    /// The `Response::err` code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "media_unavailable",
            Self::PluginMissing(_) => "media_plugin_missing",
            Self::StartFailed(_) => "media_start_failed",
        }
    }

    /// The human-readable detail.
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(message)
            | Self::PluginMissing(message)
            | Self::StartFailed(message) => message,
        }
    }
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())
    }
}

impl From<VideoError> for MediaError {
    fn from(error: VideoError) -> Self {
        match error {
            VideoError::Init(message) => Self::Unavailable(message),
            VideoError::PluginMissing(message) => Self::PluginMissing(message),
            other => Self::StartFailed(other.to_string()),
        }
    }
}

/// GStreamer's initialisation outcome, initialised (and logged) once.
static GST_INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();

/// Initialise GStreamer once per process and log the linked version.
///
/// Safe to call any number of times from any thread; only the first call does
/// the work (and the log line). The windowed run calls it before the loop even
/// when no control socket (and therefore no [`MediaHub`]) exists, so a missing
/// GStreamer is reported at startup rather than on the first camera.
pub fn init() -> Result<(), String> {
    GST_INIT
        .get_or_init(|| {
            let result = video::ensure_gst().map_err(|error| error.to_string());

            match &result {
                Ok(()) => {
                    let version =
                        video::gstreamer_version().unwrap_or_else(|| "unknown".to_string());
                    eprintln!("engine: INFO GStreamer {version} ready for video capture");
                }
                Err(message) => {
                    eprintln!(
                        "engine: WARN video capture unavailable ({message}); media/start-* answers \
                         media_unavailable"
                    );
                }
            }

            result
        })
        .clone()
}

/// One video source the engine is capturing.
struct SourceEntry {
    /// The configuration the source was started with; a health restart replays
    /// it verbatim (same device, same mode).
    config: VideoSourceConfig,
    /// The reusable frame buffer [`VideoHub::latest_into`] fills.
    frame: VideoFrame,
    /// `VideoSource::set_looping` for file sources (`<video loop>` default).
    looping: bool,
    /// Whether the pipeline is currently registered with the hub. A source in
    /// restart backoff keeps its entry, its order index and its last texture.
    running: bool,
    /// Failed restart attempts since the source was last healthy.
    attempts: u32,
    /// When the next restart attempt is due (`None` while running).
    retry_at: Option<Instant>,
    /// Why the source is not running (pipeline error / restart failure).
    last_error: Option<String>,
    /// Set once per failure episode so a permanently broken device logs once.
    logged_failure: bool,
    /// The last width/height a frame reported, kept for `media/status` while
    /// the source restarts.
    last_size: (u32, u32),
    /// Frame counter at the moment the source failed (reported while it
    /// restarts, when the hub no longer knows the source).
    last_frames: u64,
    /// Drop counter at the moment the source failed.
    last_dropped: u64,
}

/// The engine's video-capture registry.
pub struct MediaHub {
    hub: VideoHub,
    /// Source ids in start order (the Webcam `camera` prop order).
    order: Vec<String>,
    entries: HashMap<String, SourceEntry>,
    /// GStreamer initialisation outcome (`Err` = capture unavailable).
    init: Result<(), String>,
    last_health: Instant,
    /// `screen_support()` is logged once, when the first screen start is asked
    /// for, so a host without PipeWire says why.
    screen_logged: bool,
}

impl MediaHub {
    /// Initialise GStreamer and log the linked version.
    pub fn new() -> Self {
        let init = init();

        Self {
            hub: VideoHub::new(),
            order: Vec::new(),
            entries: HashMap::new(),
            init,
            last_health: Instant::now(),
            screen_logged: false,
        }
    }

    /// Whether capture can start (GStreamer initialised).
    pub fn is_available(&self) -> bool {
        self.init.is_ok()
    }

    /// Why capture is unavailable, when it is.
    pub fn init_error(&self) -> Option<&str> {
        self.init.as_ref().err().map(String::as_str)
    }

    /// Enumerate V4L2 cameras (empty when the host has none).
    pub fn list_cameras(&self) -> Vec<CameraInfo> {
        list_cameras()
    }

    /// PipeWire screen-capture support, logged once.
    pub fn screen_support(&mut self) -> ScreenSupport {
        let support = video::screen_support();

        if !self.screen_logged {
            self.screen_logged = true;
            match support.reason.as_deref() {
                Some(reason) => {
                    eprintln!("engine: WARN screen capture unavailable: {reason}");
                }
                None => {
                    eprintln!("engine: INFO screen capture available (PipeWire)");
                }
            }
        }

        support
    }

    /// Start (or replace) `source_id`, looping video files by default.
    pub fn start(
        &mut self,
        source_id: &str,
        config: VideoSourceConfig,
    ) -> Result<usize, MediaError> {
        self.start_with_looping(source_id, config, true)
    }

    /// Start (or replace) `source_id` with an explicit file-loop flag.
    pub fn start_with_looping(
        &mut self,
        source_id: &str,
        config: VideoSourceConfig,
        looping: bool,
    ) -> Result<usize, MediaError> {
        if let Err(message) = &self.init {
            return Err(MediaError::Unavailable(message.clone()));
        }

        let source_id = source_id.trim();
        if source_id.is_empty() {
            return Err(MediaError::StartFailed(
                "a video source needs a non-empty sourceId".to_string(),
            ));
        }

        let config = config.normalised();
        let is_screen = matches!(config.kind, VideoSourceKind::Screen { .. });
        if is_screen {
            self.screen_support();
        }

        if let Err(error) = self.hub.start_source(source_id, config.clone()) {
            // A failed start registers nothing: the hub already released the
            // previous pipeline, so the caller can simply retry.
            self.entries.remove(source_id);
            self.order.retain(|id| id != source_id);

            return Err(error.into());
        }

        if !self.order.iter().any(|id| id == source_id) {
            self.order.push(source_id.to_string());
        }
        self.entries.insert(
            source_id.to_string(),
            SourceEntry {
                config,
                frame: VideoFrame::default(),
                looping,
                running: true,
                attempts: 0,
                retry_at: None,
                last_error: None,
                logged_failure: false,
                last_size: (0, 0),
                last_frames: 0,
                last_dropped: 0,
            },
        );
        self.apply_looping(source_id, looping);

        let index = self.index_of(source_id).unwrap_or(1);

        eprintln!(
            "engine: INFO video source {source_id:?} started (index {index}, {:?} {}x{}@{})",
            self.entries[source_id].config.kind.label(),
            self.entries[source_id].config.width,
            self.entries[source_id].config.height,
            self.entries[source_id].config.fps
        );

        Ok(index)
    }

    /// Stop and forget `source_id`; returns whether it was running.
    pub fn stop(&mut self, source_id: &str) -> bool {
        let running = self.hub.stop_source(source_id);
        let registered = self.entries.remove(source_id).is_some();
        self.order.retain(|id| id != source_id);

        if running || registered {
            eprintln!("engine: INFO video source {source_id:?} stopped");
        }

        running || registered
    }

    /// Stop every source (shutdown).
    pub fn stop_all(&mut self) {
        if !self.order.is_empty() {
            eprintln!("engine: INFO stopping {} video source(s)", self.order.len());
        }
        self.hub.stop_all();
        self.entries.clear();
        self.order.clear();
    }

    /// Source ids in start order.
    pub fn ids(&self) -> &[String] {
        &self.order
    }

    /// How many sources are registered (running or restarting).
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether no source is registered.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Drain the newest frame of the source at `index` into its reusable buffer.
    ///
    /// `(source id, frame, is_new)`; `is_new` is `false` when no new frame is
    /// waiting (the compositor keeps the previous texture in that case).
    pub fn frame_at(&mut self, index: usize) -> Option<(&str, &VideoFrame, bool)> {
        let id = self.order.get(index)?.as_str();
        let entry = self.entries.get_mut(id)?;
        let is_new = entry.running && self.hub.latest_into(id, &mut entry.frame);

        if is_new {
            entry.last_size = (entry.frame.width, entry.frame.height);
        }

        Some((id, &entry.frame, is_new))
    }

    /// Every source's status, in start order.
    pub fn statuses(&self) -> Vec<MediaSourceStatus> {
        let mut statuses = Vec::with_capacity(self.order.len());

        for (position, id) in self.order.iter().enumerate() {
            let Some(entry) = self.entries.get(id) else {
                continue;
            };
            let live = entry.running.then(|| self.hub.status(id)).flatten();
            let status = match live {
                Some(status) => MediaSourceStatus {
                    source_id: id.clone(),
                    kind: status.kind,
                    index: position + 1,
                    state: state_label(status.state),
                    width: status.width,
                    height: status.height,
                    frames: status.frames,
                    dropped: status.dropped,
                    last_error: status.last_error.or_else(|| entry.last_error.clone()),
                },
                None => MediaSourceStatus {
                    source_id: id.clone(),
                    kind: entry.config.kind.label(),
                    index: position + 1,
                    // The pipeline is released while the backoff runs, so the
                    // source is between restarts (`error`/`ended` is latched in
                    // `lastError`). Reported as `starting` so the renderer's
                    // camera reconcile leaves the engine's retry loop alone.
                    state: "starting",
                    width: entry.last_size.0,
                    height: entry.last_size.1,
                    frames: entry.last_frames,
                    dropped: entry.last_dropped,
                    last_error: entry.last_error.clone(),
                },
            };

            statuses.push(status);
        }

        statuses
    }

    /// Restart sources that errored, ended or disappeared (once per second).
    pub fn poll_health(&mut self) {
        if self.last_health.elapsed() < HEALTH_INTERVAL {
            return;
        }
        self.last_health = Instant::now();

        for id in self.order.clone() {
            let action =
                {
                    let Some(entry) = self.entries.get(&id) else {
                        continue;
                    };

                    if !entry.running {
                        if entry.retry_at.is_some_and(|at| at <= Instant::now()) {
                            Some(HealthAction::Retry)
                        } else {
                            None
                        }
                    } else if let Some(status) = self.hub.status(&id) {
                        if status.needs_restart() {
                            let reason = status.last_error.clone().unwrap_or_else(|| match status
                                .state
                            {
                                VideoSourceState::Ended => {
                                    "the video source reached its end".to_string()
                                }
                                _ => "the video pipeline stopped".to_string(),
                            });

                            Some(HealthAction::Restart {
                                reason,
                                frames: status.frames,
                                dropped: status.dropped,
                                size: (status.width, status.height),
                            })
                        } else if entry.attempts > 0 {
                            Some(HealthAction::Healthy)
                        } else {
                            None
                        }
                    } else {
                        Some(HealthAction::Restart {
                            reason: "the source disappeared from the capture hub".to_string(),
                            frames: entry.last_frames,
                            dropped: entry.last_dropped,
                            size: entry.last_size,
                        })
                    }
                };

            match action {
                Some(HealthAction::Retry) => self.attempt_restart(&id),
                Some(HealthAction::Restart {
                    reason,
                    frames,
                    dropped,
                    size,
                }) => self.schedule_restart(&id, reason, frames, dropped, size),
                Some(HealthAction::Healthy) => {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        entry.attempts = 0;
                        entry.logged_failure = false;
                        entry.last_error = None;
                    }
                }
                None => {}
            }
        }
    }

    /// Camera-like sources in start order: the list the Webcam module's
    /// 1-based `camera` prop indexes. Files are not webcams, so they are
    /// excluded; screen nodes are treated like cameras (the legacy renderer
    /// puts screen sources in the same `selectedVideoSources` order).
    pub fn camera_ids(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|id| {
                self.entries.get(*id).is_some_and(|entry| {
                    matches!(
                        entry.config.kind,
                        VideoSourceKind::Camera { .. }
                            | VideoSourceKind::Screen { .. }
                            | VideoSourceKind::TestPattern { .. }
                    )
                })
            })
            .cloned()
            .collect()
    }

    /// Resolve the Webcam module's 1-based `camera` index against `order`.
    ///
    /// Port of `selectWebcamSource` (`media-inputs.js:150-158`): indexes below
    /// 1 clamp to the first source, an out-of-range index falls back to the
    /// first source, and an empty list resolves to `None`.
    pub fn select_source_id<'a>(order: &'a [String], index: i64) -> Option<&'a str> {
        if order.is_empty() {
            return None;
        }

        let index = index.max(1) as usize;

        order
            .get(index - 1)
            .or_else(|| order.first())
            .map(String::as_str)
    }

    /// Read the `camera` prop of a module (raw number or a resolved
    /// input/tween `{ value }` object).
    pub fn camera_index(props: &Value) -> i64 {
        let Some(value) = props.get("camera") else {
            return 1;
        };
        // An input link/tween replaces the raw value with `{ value: … }`.
        let Some(number) = value.get("value").unwrap_or(value).as_f64() else {
            return 1;
        };
        if !number.is_finite() {
            return 1;
        }

        (number.round() as i64).max(1)
    }

    /// 1-based position of `source_id` in the start order.
    fn index_of(&self, source_id: &str) -> Option<usize> {
        self.order
            .iter()
            .position(|id| id == source_id)
            .map(|position| position + 1)
    }

    /// `VideoSource::set_looping` for file sources (no-op for the others).
    fn apply_looping(&mut self, source_id: &str, looping: bool) {
        if let Some(source) = self.hub.source_mut(source_id) {
            source.set_looping(looping);
        }
    }

    /// Release a broken source and schedule the next restart attempt.
    fn schedule_restart(
        &mut self,
        source_id: &str,
        reason: String,
        frames: u64,
        dropped: u64,
        size: (u32, u32),
    ) {
        // Release the device before the retry: a busy V4L2 device refuses a
        // second pipeline, and a dead node must not keep its threads alive.
        self.hub.stop_source(source_id);

        let (delay, first_log) = {
            let Some(entry) = self.entries.get_mut(source_id) else {
                return;
            };

            entry.running = false;
            entry.last_error = Some(reason.clone());
            entry.attempts = entry.attempts.saturating_add(1);
            entry.retry_at = Some(Instant::now() + restart_delay(entry.attempts));
            entry.last_frames = frames;
            entry.last_dropped = dropped;
            if size.0 > 0 && size.1 > 0 {
                entry.last_size = size;
            }

            (restart_delay(entry.attempts), !entry.logged_failure)
        };
        if first_log {
            eprintln!(
                "engine: WARN video source {source_id:?} stopped ({reason}); retrying in {} ms",
                delay.as_millis()
            );
        }
    }

    /// Retry a source whose backoff elapsed.
    fn attempt_restart(&mut self, source_id: &str) {
        let Some((config, looping)) = self
            .entries
            .get(source_id)
            .map(|entry| (entry.config.clone(), entry.looping))
        else {
            return;
        };

        match self.hub.start_source(source_id, config) {
            Ok(()) => {
                let recovered = self
                    .entries
                    .get(source_id)
                    .is_some_and(|entry| entry.logged_failure);

                if let Some(entry) = self.entries.get_mut(source_id) {
                    entry.running = true;
                    entry.retry_at = None;
                    entry.attempts = 0;
                    entry.last_error = None;
                    entry.logged_failure = false;
                }
                self.apply_looping(source_id, looping);

                if recovered {
                    eprintln!("engine: INFO video source {source_id:?} restarted");
                }
            }
            Err(error) => {
                let message = error.to_string();

                if let Some(entry) = self.entries.get_mut(source_id) {
                    entry.attempts = entry.attempts.saturating_add(1);
                    entry.retry_at = Some(Instant::now() + restart_delay(entry.attempts));
                    entry.last_error = Some(message);
                }
            }
        }
    }
}

/// What [`MediaHub::poll_health`] decided for one source.
enum HealthAction {
    /// The backoff elapsed: start the pipeline again.
    Retry,
    /// The source failed/ended: release it and schedule a retry.
    Restart {
        reason: String,
        frames: u64,
        dropped: u64,
        size: (u32, u32),
    },
    /// The source is delivering frames again after a failure episode.
    Healthy,
}

impl Default for MediaHub {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MediaHub {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// The `media/status` state label for a pipeline state.
///
/// The vocabulary is the renderer's (`engine-media.js`
/// `MEDIA_SOURCE_STATE_LABELS`): `running` (playing), `starting` (prerolling,
/// or released between engine restarts), `stopped` (a finished file) and
/// `error`. A source the engine is retrying is **`starting`, not `error`**, so
/// the renderer's own camera reconcile never races the engine's reconnect loop.
fn state_label(state: VideoSourceState) -> &'static str {
    match state {
        VideoSourceState::Playing => "running",
        // A live source is `Paused` until it prerolls; no frame has reached the
        // engine yet.
        VideoSourceState::Paused => "starting",
        VideoSourceState::Ended => "stopped",
        VideoSourceState::Error => "error",
        VideoSourceState::Stopped => "stopped",
    }
}

/// The `media/status` entry for one source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaSourceStatus {
    /// Caller id the source was started with.
    pub source_id: String,
    /// `camera`, `file`, `screen` or `test-pattern`.
    pub kind: &'static str,
    /// 1-based position in start order (the Webcam `camera` prop).
    pub index: usize,
    /// `running`, `starting`, `stopped` or `error` (the renderer's labels).
    pub state: &'static str,
    /// Width of the newest frame (0 before the first frame).
    pub width: u32,
    /// Height of the newest frame.
    pub height: u32,
    /// Frames the pipeline delivered.
    pub frames: u64,
    /// Frames replaced before the engine read them.
    pub dropped: u64,
    /// Why the source is not running, when it is not.
    pub last_error: Option<String>,
}

impl MediaSourceStatus {
    /// The `media/status` JSON object for this source.
    pub fn to_json(&self) -> Value {
        json!({
            "sourceId": self.source_id,
            "kind": self.kind,
            "index": self.index,
            "state": self.state,
            "width": self.width,
            "height": self.height,
            "frames": self.frames,
            "dropped": self.dropped,
            "lastError": self.last_error,
        })
    }
}

/// The `media/list-cameras` camera array.
pub fn cameras_json(cameras: &[CameraInfo]) -> Value {
    Value::Array(
        cameras
            .iter()
            .map(|camera| {
                json!({
                    "id": camera.id,
                    "name": camera.name,
                    "path": camera.path,
                    "caps": camera
                        .caps
                        .iter()
                        .map(|mode| json!({
                            "width": mode.width,
                            "height": mode.height,
                            "fps": mode.fps,
                        }))
                        .collect::<Vec<Value>>(),
                })
            })
            .collect(),
    )
}

/// Delay before the `attempts`-th restart attempt (`0` = first).
pub fn restart_delay(attempts: u32) -> Duration {
    // attempts 1, 2, 3 … -> 500 ms, 1 s, 2 s … capped at 8 s. A permanently
    // missing device is retried every 8 s instead of spinning.
    let exponent = attempts.saturating_sub(1).min(8);
    let delay = RESTART_BASE_DELAY * (1_u32 << exponent);

    delay.min(RESTART_MAX_DELAY)
}

/// Pad `frame`'s rows to WebGPU's 256-byte copy alignment into `scratch`.
///
/// Returns `(data, bytes_per_row)`. A frame whose row length is already aligned
/// borrows the caller's buffer untouched; an unaligned one is copied row by row
/// into `scratch` (reused across frames) so the trailing bytes stay zero.
pub(crate) fn padded_rows<'a>(frame: &'a VideoFrame, scratch: &'a mut Vec<u8>) -> (&'a [u8], u32) {
    let row = frame.width as usize * 4;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;

    if row % alignment == 0 {
        return (&frame.rgba, row as u32);
    }

    let padded = row.div_ceil(alignment) * alignment;
    let height = frame.height as usize;

    scratch.clear();
    scratch.resize(padded * height, 0);

    for row_index in 0..height {
        let source = &frame.rgba[row_index * row..(row_index + 1) * row];
        let target = &mut scratch[row_index * padded..row_index * padded + row];

        target.copy_from_slice(source);
    }

    (&scratch[..], padded as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use modv_media::video::TestPattern;

    /// A 30 fps test pattern the host can actually run, or a skipped test.
    fn start_pattern(
        media: &mut MediaHub,
        source_id: &str,
        pattern: TestPattern,
        width: u32,
        height: u32,
    ) -> Option<usize> {
        match media.start(
            source_id,
            VideoSourceConfig::test_pattern(pattern).with_mode(width, height, 30),
        ) {
            Ok(index) => Some(index),
            Err(MediaError::PluginMissing(message)) | Err(MediaError::Unavailable(message)) => {
                eprintln!("skipping media test: {message}");
                None
            }
            Err(error) => panic!("test pattern {source_id} did not start: {error}"),
        }
    }

    fn wait_for_frame(media: &mut MediaHub, source_id: &str, timeout: Duration) -> VideoFrame {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            for index in 0..media.len() {
                if media.ids().get(index).is_none_or(|id| id != source_id) {
                    continue;
                }
                if let Some((_, frame, true)) = media.frame_at(index) {
                    return frame.clone();
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("no frame from {source_id:?}: {:?}", media.statuses());
    }

    #[test]
    fn sources_are_indexed_in_start_order() {
        let mut media = MediaHub::new();
        if !media.is_available() {
            eprintln!("skipping: {}", media.init_error().unwrap_or("no GStreamer"));
            return;
        }
        let Some(first) = start_pattern(&mut media, "cam-a", TestPattern::Checkers1, 32, 32) else {
            return;
        };
        let second =
            start_pattern(&mut media, "cam-b", TestPattern::Smpte, 32, 32).expect("second source");
        let third =
            start_pattern(&mut media, "cam-c", TestPattern::White, 32, 32).expect("third source");

        assert_eq!((first, second, third), (1, 2, 3));
        assert_eq!(media.ids(), ["cam-a", "cam-b", "cam-c"]);
        assert_eq!(media.len(), 3);
        assert!(!media.is_empty());
    }

    #[test]
    fn stopping_a_source_reindexes_the_remaining_sources() {
        let mut media = MediaHub::new();
        if !media.is_available() {
            return;
        }
        if start_pattern(&mut media, "cam-a", TestPattern::Checkers1, 32, 32).is_none() {
            return;
        }
        start_pattern(&mut media, "cam-b", TestPattern::Smpte, 32, 32).expect("second source");

        assert!(media.stop("cam-a"));
        assert!(!media.stop("cam-a"), "stopping twice is a no-op");
        assert_eq!(media.ids(), ["cam-b"]);
        assert_eq!(media.statuses()[0].index, 1);

        // A new source takes the next free index, not the stopped slot.
        let index =
            start_pattern(&mut media, "cam-c", TestPattern::White, 32, 32).expect("third source");
        assert_eq!(index, 2);
        assert_eq!(media.ids(), ["cam-b", "cam-c"]);
    }

    #[test]
    fn status_reports_frames_and_the_negotiated_size() {
        let mut media = MediaHub::new();
        if !media.is_available() {
            return;
        }
        if start_pattern(&mut media, "cam-a", TestPattern::Checkers1, 64, 48).is_none() {
            return;
        }
        wait_for_frame(&mut media, "cam-a", Duration::from_secs(8));

        let statuses = media.statuses();
        assert_eq!(statuses.len(), 1);
        let status = &statuses[0];
        assert_eq!(status.source_id, "cam-a");
        assert_eq!(status.kind, "test-pattern");
        assert_eq!(status.index, 1);
        assert_eq!(status.state, "running", "a delivering source is running");
        assert_eq!((status.width, status.height), (64, 48));
        assert!(status.frames >= 1);
        assert!(status.last_error.is_none(), "{status:?}");

        let wire = status.to_json();
        assert_eq!(wire["sourceId"], json!("cam-a"));
        assert_eq!(wire["kind"], json!("test-pattern"));
        assert_eq!(wire["index"], json!(1));
        assert_eq!(wire["width"], json!(64));
        assert_eq!(wire["height"], json!(48));
        assert_eq!(wire["lastError"], Value::Null);
    }

    #[test]
    fn a_failed_start_is_reported_and_leaves_the_hub_usable() {
        let mut media = MediaHub::new();
        if !media.is_available() {
            return;
        }

        match media.start(
            "cam-a",
            VideoSourceConfig::camera("/dev/modv-engine-no-such-camera").with_mode(64, 48, 30),
        ) {
            Err(MediaError::StartFailed(message)) => assert!(!message.is_empty(), "{message:?}"),
            Err(MediaError::PluginMissing(message)) => {
                eprintln!("skipping: {message}");
                return;
            }
            Err(other) => panic!("unexpected media error: {other:?}"),
            Ok(_) => panic!("a missing device must not start"),
        }
        assert!(media.is_empty(), "a failed start registers nothing");
        assert_eq!(media.statuses(), Vec::new());

        // The hub still works for a source that can start.
        let index = start_pattern(&mut media, "cam-b", TestPattern::Checkers1, 32, 32)
            .expect("the hub recovers from a failed start");
        assert_eq!(index, 1);
    }

    /// Force the once-a-second health poll to be due, then run it (the loop
    /// calls `poll_health` every frame; a test has to advance its clock).
    fn poll_health_now(media: &mut MediaHub) {
        media.last_health = Instant::now() - HEALTH_INTERVAL;
        media.poll_health();
    }

    #[test]
    fn a_lost_source_is_restarted_with_a_backoff() {
        let mut media = MediaHub::new();
        if !media.is_available() {
            return;
        }
        if start_pattern(&mut media, "cam-a", TestPattern::Checkers1, 32, 32).is_none() {
            return;
        }
        wait_for_frame(&mut media, "cam-a", Duration::from_secs(8));

        // Simulate a camera that vanished: the pipeline is gone without the
        // hub noticing (an unplug posts a bus error, which ends up here).
        media.hub.stop_source("cam-a");

        poll_health_now(&mut media);
        let status = &media.statuses()[0];
        assert_eq!(status.state, "starting");
        assert!(status.last_error.is_some(), "{status:?}");
        assert!(
            !media.entries["cam-a"].running,
            "the entry survives for the restart"
        );

        // The backoff is respected until it elapses.
        assert!(
            media.entries["cam-a"].retry_at.unwrap() > Instant::now(),
            "the first retry waits for the backoff"
        );
        poll_health_now(&mut media);
        assert!(
            !media.entries["cam-a"].running,
            "no retry before the backoff"
        );

        // Pretend the backoff elapsed: the next poll restarts the pipeline.
        media.entries.get_mut("cam-a").unwrap().retry_at = Some(Instant::now());
        poll_health_now(&mut media);
        assert!(media.entries["cam-a"].running, "the source restarts");
        assert_eq!(media.ids(), ["cam-a"], "the order index is preserved");

        wait_for_frame(&mut media, "cam-a", Duration::from_secs(8));
        poll_health_now(&mut media);
        assert!(media.entries["cam-a"].attempts == 0);
        assert!(media.statuses()[0].last_error.is_none());
    }

    #[test]
    fn the_restart_backoff_doubles_and_caps() {
        assert_eq!(restart_delay(1), RESTART_BASE_DELAY);
        assert_eq!(restart_delay(2), RESTART_BASE_DELAY * 2);
        assert_eq!(restart_delay(3), RESTART_BASE_DELAY * 4);
        assert_eq!(restart_delay(99), RESTART_MAX_DELAY);
        assert_eq!(restart_delay(0), RESTART_BASE_DELAY);
    }

    #[test]
    fn unaligned_rows_are_padded_for_the_upload() {
        // 322 * 4 = 1288 bytes; the next 256-byte multiple is 1536.
        let mut frame = VideoFrame::with_size(322, 3);
        for (index, byte) in frame.rgba.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        let mut scratch = Vec::new();
        let (data, bytes_per_row) = padded_rows(&frame, &mut scratch);

        assert_eq!(bytes_per_row, 1536);
        assert_eq!(data.len(), 1536 * 3);
        for row in 0..3 {
            let source = &frame.rgba[row * 1288..(row + 1) * 1288];
            let target = &data[row * 1536..row * 1536 + 1288];
            assert_eq!(source, target, "row {row} is copied verbatim");
            assert!(
                data[row * 1536 + 1288..(row + 1) * 1536]
                    .iter()
                    .all(|byte| *byte == 0),
                "row {row} padding stays zero"
            );
        }

        // An aligned frame borrows its own buffer (no copy).
        let aligned = VideoFrame::with_size(64, 2);
        let mut scratch = Vec::new();
        let (data, bytes_per_row) = padded_rows(&aligned, &mut scratch);
        assert_eq!(bytes_per_row, 256);
        assert_eq!(data.len(), aligned.rgba.len());
    }

    #[test]
    fn source_selection_mirrors_select_webcam_source() {
        let order = vec!["a".to_string(), "b".to_string()];

        assert_eq!(MediaHub::select_source_id(&order, 1), Some("a"));
        assert_eq!(MediaHub::select_source_id(&order, 2), Some("b"));
        // Rounding/clamping/fallback (`media-inputs.js:150-158`).
        assert_eq!(MediaHub::select_source_id(&order, 0), Some("a"));
        assert_eq!(MediaHub::select_source_id(&order, -4), Some("a"));
        assert_eq!(MediaHub::select_source_id(&order, 9), Some("a"));
        assert_eq!(MediaHub::select_source_id(&[], 1), None);
    }

    #[test]
    fn the_camera_prop_reads_raw_numbers_and_resolved_values() {
        assert_eq!(MediaHub::camera_index(&json!({"camera": 2})), 2);
        assert_eq!(
            MediaHub::camera_index(&json!({"camera": { "value": 3 }})),
            3
        );
        assert_eq!(MediaHub::camera_index(&json!({})), 1);
        assert_eq!(MediaHub::camera_index(&json!({"camera": "2"})), 1);
        assert_eq!(MediaHub::camera_index(&json!({"camera": 0})), 1);
    }

    #[test]
    fn camera_json_carries_the_picker_fields() {
        let cameras = vec![CameraInfo {
            id: "/dev/video0".to_string(),
            name: "Test Camera".to_string(),
            path: Some("/dev/video0".to_string()),
            caps: vec![modv_media::video::VideoMode {
                width: 640,
                height: 480,
                fps: 30,
            }],
        }];
        let json = cameras_json(&cameras);

        assert_eq!(json[0]["id"], json!("/dev/video0"));
        assert_eq!(json[0]["name"], json!("Test Camera"));
        assert_eq!(json[0]["path"], json!("/dev/video0"));
        assert_eq!(json[0]["caps"][0]["width"], json!(640));
        assert_eq!(json[0]["caps"][0]["height"], json!(480));
        assert_eq!(json[0]["caps"][0]["fps"], json!(30));
        // A camera without a path reports null rather than omitting the key.
        let mut headless = cameras.clone();
        headless[0].path = None;
        assert_eq!(cameras_json(&headless)[0]["path"], Value::Null);
    }

    #[test]
    fn stopping_a_source_stops_disposing_it_and_restarting_works() {
        let mut media = MediaHub::new();
        if !media.is_available() {
            return;
        }
        if start_pattern(&mut media, "cam-a", TestPattern::Checkers1, 32, 32).is_none() {
            return;
        }
        wait_for_frame(&mut media, "cam-a", Duration::from_secs(8));

        media.stop("cam-a");
        assert!(media.is_empty());
        assert!(!media.hub.is_running("cam-a"));

        // A restart reuses the id and the first index.
        let index = start_pattern(&mut media, "cam-a", TestPattern::Smpte, 32, 32)
            .expect("restart of a test pattern");
        assert_eq!(index, 1);
        wait_for_frame(&mut media, "cam-a", Duration::from_secs(8));
        assert_eq!(media.statuses()[0].state, "running");
    }
}
