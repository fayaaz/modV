//! A running capture source: pipeline, appsink, latest-frame slot, status.
//!
//! # Threading
//!
//! ```text
//! GStreamer streaming thread                  engine thread
//! --------------------------                  -------------
//! appsink new-sample ──▶ FrameSlot::publish ──▶ take_into(&mut VideoFrame)
//!   (copy + recycle buffer)                     (buffer swap, no copy)
//! bus messages ────────── polled by status/latest_frame/stop
//! ```
//!
//! The callback copies out of the mapped `GstBuffer` (safe: `map_readable`
//! gives a `&[u8]`) and publishes into a [`FrameSlot`]. It never blocks waiting
//! for the consumer: if a frame is already waiting it is replaced and counted
//! as dropped, and its buffer is recycled. All capture/decoding happens on
//! GStreamer's own threads, so the engine thread only does the swap.
//!
//! **No GLib main loop.** Bus messages are drained with `Bus::pop` from the
//! engine thread whenever it polls status or drains a frame. A pipeline error is
//! latched into the source and the pipeline is moved to `NULL` (releasing the
//! camera); `status().last_error` then reports it until the source is stopped.
//! The engine is expected to poll at least once per rendered frame, which it
//! already does to get the texture.
//!
//! # Latency
//!
//! `appsink max-buffers=2 drop=true` plus one ready frame means at most ~3
//! frames are in flight between the device and the engine, i.e. 50 ms at 60 fps
//! before the *engine* starts missing frames. Missing them is safe: only the
//! newest frame is ever read.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gst::prelude::*;
use gst_app::AppSinkCallbacks;

use super::frame::{FrameSlot, SlotStats, VideoFrame};
use super::pipeline::{
    SINK_NAME, VideoSourceConfig, VideoSourceKind, check_required_elements, pipeline_description,
    screen_support,
};

/// How long [`VideoSource::start`] waits for the pipeline to preroll.
///
/// Live cameras legitimately stay `ASYNC` for longer, so a timeout is not a
/// failure; only an explicit `FAILURE` (or a bus error) aborts `start`.
pub const START_TIMEOUT_MS: u64 = 1_000;
/// How long [`VideoSource::stop`] waits for the streaming threads to join.
pub const STOP_TIMEOUT_MS: u64 = 2_000;

/// Where a source is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSourceState {
    /// The pipeline is playing and frames may be arriving.
    Playing,
    /// The pipeline is prerolling/paused (typical right after `start`).
    Paused,
    /// A file reached end-of-stream (and looping is disabled).
    Ended,
    /// The pipeline reported an error; see the status' `last_error`.
    Error,
    /// [`VideoSource::stop`] was called, or the pipeline went to `NULL`.
    Stopped,
}

impl VideoSourceState {
    /// Whether frames can still arrive.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Playing | Self::Paused)
    }
}

/// A snapshot of a source's health and counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSourceStatus {
    /// Source kind label (`camera`, `file`, `screen`, `test-pattern`).
    pub kind: &'static str,
    /// Lifecycle state.
    pub state: VideoSourceState,
    /// Width of the newest frame.
    pub width: u32,
    /// Height of the newest frame.
    pub height: u32,
    /// Frames delivered by the pipeline.
    pub frames: u64,
    /// Frames replaced before the engine read them ("newest wins" drops).
    pub dropped: u64,
    /// Sequence of the newest delivered frame.
    pub sequence: u64,
    /// Buffers the slot allocated (flat in steady state).
    pub buffers_allocated: u64,
    /// The latched pipeline error, if any.
    pub last_error: Option<String>,
}

impl VideoSourceStatus {
    /// Frames can still arrive.
    pub fn is_running(&self) -> bool {
        self.state.is_running()
    }

    /// The source needs a restart (error or end of a non-looping file).
    pub fn needs_restart(&self) -> bool {
        matches!(
            self.state,
            VideoSourceState::Error | VideoSourceState::Ended
        )
    }
}

/// A capture failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    /// GStreamer could not be initialised.
    Init(String),
    /// An element the source needs is not installed.
    PluginMissing(String),
    /// `gst_parse_launch` rejected the description or produced no pipeline.
    Pipeline(String),
    /// The configuration cannot be turned into a pipeline.
    Unsupported(String),
    /// The pipeline refused to start or errored while running.
    State(String),
    /// PipeWire/portal capture is not usable on this host.
    ServerUnavailable(String),
    /// [`super::VideoHub`] was asked about an id that is not running.
    UnknownSource(String),
    /// The source was stopped.
    Stopped(String),
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Init(message) => {
                write!(formatter, "GStreamer could not be initialised: {message}")
            }
            Self::PluginMissing(message) => write!(formatter, "{message}"),
            Self::Pipeline(message) => write!(formatter, "video pipeline error: {message}"),
            Self::Unsupported(message) => write!(formatter, "unsupported video source: {message}"),
            Self::State(message) => write!(formatter, "video pipeline failed to start: {message}"),
            Self::ServerUnavailable(message) => {
                write!(formatter, "screen capture unavailable: {message}")
            }
            Self::UnknownSource(id) => {
                write!(formatter, "no video source is running for id {id:?}")
            }
            Self::Stopped(message) => write!(formatter, "video source stopped: {message}"),
        }
    }
}

impl std::error::Error for VideoError {}

static GST_INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Initialise GStreamer once per process.
///
/// Safe to call from any thread and any number of times; the first result is
/// cached. Every public entry point in [`super`] calls it, so callers do not
/// have to.
pub fn ensure_gst() -> Result<(), VideoError> {
    match GST_INIT.get_or_init(|| gst::init().map_err(|error| error.to_string())) {
        Ok(()) => Ok(()),
        Err(message) => Err(VideoError::Init(message.clone())),
    }
}

/// The linked GStreamer version, for logs and the integration report.
pub fn gstreamer_version() -> Option<String> {
    ensure_gst().ok()?;

    let (major, minor, micro, nano) = gst::version();

    Some(if nano == 0 {
        format!("{major}.{minor}.{micro}")
    } else {
        format!("{major}.{minor}.{micro}.{nano}")
    })
}

/// A running video source.
///
/// Created with [`VideoSource::start`]; frames are drained with
/// [`Self::latest_frame_into`] (allocation-free) or [`Self::latest_frame`], and
/// health with [`Self::status`]. [`Self::stop`] (also run on drop) tears the
/// pipeline down and releases the device.
pub struct VideoSource {
    pipeline: gst::Pipeline,
    bus: gst::Bus,
    sink: gst_app::AppSink,
    slot: Arc<FrameSlot>,
    error: Arc<Mutex<Option<String>>>,
    ended: Arc<AtomicBool>,
    config: VideoSourceConfig,
    looping: bool,
    stopped: bool,
}

impl VideoSource {
    /// Build and start the pipeline for `config`.
    ///
    /// Fails cleanly (with a message naming the missing plugin, the bad URI or
    /// the pipeline error) instead of leaving a half-started pipeline behind:
    /// a failure here always moves the pipeline to `NULL` first.
    pub fn start(config: VideoSourceConfig) -> Result<Self, VideoError> {
        ensure_gst()?;

        let config = config.normalised();

        if let VideoSourceKind::Screen { .. } = config.kind {
            let support = screen_support();

            if !support.plugin {
                return Err(VideoError::PluginMissing(
                    support
                        .reason
                        .unwrap_or_else(|| "pipewiresrc is missing".to_string()),
                ));
            }
        }

        let description = pipeline_description(&config)?;

        check_required_elements(&config.kind)?;

        let element = gst::parse::launch(&description)
            .map_err(|error| VideoError::Pipeline(format!("{error} (pipeline: {description})")))?;
        let pipeline = element.downcast::<gst::Pipeline>().map_err(|_| {
            VideoError::Pipeline(format!("gst_parse_launch built no pipeline: {description}"))
        })?;
        let sink = pipeline
            .by_name(SINK_NAME)
            .ok_or_else(|| {
                VideoError::Pipeline(format!(
                    "the pipeline has no {SINK_NAME} appsink: {description}"
                ))
            })?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| {
                VideoError::Pipeline(format!("{SINK_NAME} is not an appsink: {description}"))
            })?;

        let slot = Arc::new(FrameSlot::new());
        let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let callback_slot = Arc::clone(&slot);
        let callback_error = Arc::clone(&error);

        sink.set_callbacks(
            AppSinkCallbacks::builder()
                .new_sample(move |sink| on_new_sample(sink, &callback_slot, &callback_error))
                .build(),
        );

        let bus = pipeline
            .bus()
            .ok_or_else(|| VideoError::Pipeline("the pipeline has no bus".to_string()))?;

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|failure| start_error(&config, failure.to_string()))?;

        let (state_change, _current, _pending) =
            pipeline.state(gst::ClockTime::from_mseconds(START_TIMEOUT_MS));

        if state_change.is_err() {
            // `set_state`/`get_state` only report "it failed"; the reason (a
            // busy camera, an unplugged device, a PipeWire node that offers no
            // raw video format) is on the bus.
            let detail = wait_bus_error(&bus, START_TIMEOUT_MS).unwrap_or_else(|| {
                "the pipeline refused to start (no bus error was posted)".to_string()
            });

            let _ = pipeline.set_state(gst::State::Null);

            return Err(start_error(&config, detail));
        }

        Ok(Self {
            pipeline,
            bus,
            sink,
            slot,
            error,
            ended: Arc::new(AtomicBool::new(false)),
            config,
            looping: true,
            stopped: false,
        })
    }

    /// The normalised configuration this source is running.
    pub fn config(&self) -> &VideoSourceConfig {
        &self.config
    }

    /// The source kind label (`camera`, `file`, `screen`, `test-pattern`).
    pub fn kind_label(&self) -> &'static str {
        self.config.kind.label()
    }

    /// Whether video files restart from the beginning at end-of-stream.
    pub fn is_looping(&self) -> bool {
        self.looping
    }

    /// Enable/disable looping for file sources (looping is on by default, like
    /// the legacy `<video loop>` element).
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    /// Whether [`Self::stop`] has been called.
    pub fn is_running(&self) -> bool {
        !self.stopped
    }

    /// Drain the newest frame into `frame`, reusing both buffers.
    ///
    /// Returns `false` when no new frame is waiting (the engine should keep the
    /// previous texture in that case). This is the allocation-free render path.
    pub fn latest_frame_into(&mut self, frame: &mut VideoFrame) -> bool {
        if self.stopped {
            return false;
        }

        self.poll_bus();

        self.slot.take_into(frame)
    }

    /// Take the newest frame by value.
    ///
    /// Convenient, but the buffer leaves the source when the caller drops the
    /// frame; prefer [`Self::latest_frame_into`] on the render path.
    pub fn latest_frame(&mut self) -> Option<VideoFrame> {
        if self.stopped {
            return None;
        }

        self.poll_bus();

        self.slot.take()
    }

    /// Health and counters. Also drains the bus, so an error that happened since
    /// the last poll shows up here.
    pub fn status(&self) -> VideoSourceStatus {
        if !self.stopped {
            self.poll_bus();
        }

        let stats = self.slot.stats();
        let last_error = self.error();
        let state = self.state(last_error.as_deref());

        VideoSourceStatus {
            kind: self.config.kind.label(),
            state,
            width: stats.width,
            height: stats.height,
            frames: stats.frames,
            dropped: stats.dropped,
            sequence: stats.sequence,
            buffers_allocated: stats.buffers_allocated,
            last_error,
        }
    }

    /// Raw slot counters (no bus polling).
    pub fn frame_stats(&self) -> SlotStats {
        self.slot.stats()
    }

    /// The latched pipeline error, if any. Drains the bus first.
    pub fn last_error(&self) -> Option<String> {
        if !self.stopped {
            self.poll_bus();
        }

        self.error()
    }

    /// Stop the pipeline and release the device. Safe to call more than once.
    ///
    /// The state change to `NULL` joins GStreamer's streaming threads, so after
    /// this returns no callback can touch the frame slot.
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }

        self.stopped = true;

        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = self
            .pipeline
            .state(gst::ClockTime::from_mseconds(STOP_TIMEOUT_MS));

        self.sink.set_callbacks(AppSinkCallbacks::builder().build());
        self.slot.clear();
    }

    /// Whether the sink reported end-of-stream (files).
    pub fn is_eos(&self) -> bool {
        self.sink.is_eos()
    }

    fn error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn state(&self, error: Option<&str>) -> VideoSourceState {
        if error.is_some() {
            return VideoSourceState::Error;
        }

        if self.stopped {
            return VideoSourceState::Stopped;
        }

        if self.ended.load(Ordering::SeqCst) {
            return VideoSourceState::Ended;
        }

        match self.pipeline.current_state() {
            gst::State::Playing => VideoSourceState::Playing,
            gst::State::Paused => VideoSourceState::Paused,
            _ => VideoSourceState::Stopped,
        }
    }

    /// Drain pending bus messages: latch errors and handle end-of-stream.
    fn poll_bus(&self) {
        while let Some(message) = self.bus.pop() {
            match message.view() {
                gst::MessageView::Eos(..) => self.on_eos(),
                gst::MessageView::Error(error) => {
                    let source = message
                        .src()
                        .map(|source| source.name().to_string())
                        .unwrap_or_else(|| "pipeline".to_string());
                    let debug = error
                        .debug()
                        .map(|debug| format!(" ({debug})"))
                        .unwrap_or_default();
                    let text = format!("{source}: {}{debug}", error.error());

                    self.latch_error(text);

                    // Release the device; the source has to be restarted.
                    let _ = self.pipeline.set_state(gst::State::Null);

                    return;
                }
                _ => {}
            }
        }
    }

    fn on_eos(&self) {
        if self.looping && matches!(self.config.kind, VideoSourceKind::File { .. }) {
            let seeked = self
                .pipeline
                .seek_simple(
                    gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                    gst::ClockTime::ZERO,
                )
                .is_ok();

            if seeked {
                return;
            }

            self.latch_error("could not restart the video file".to_string());
        }

        self.ended.store(true, Ordering::SeqCst);
    }

    fn latch_error(&self, message: String) {
        let mut error = self
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        *error = Some(message);
    }
}

impl Drop for VideoSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Shape a start failure for the source kind.
///
/// A screen source failing is an environment problem (portal consent ended, the
/// node disappeared, or `pipewiresrc` could not negotiate a raw format), so it
/// is reported as [`VideoError::ServerUnavailable`] with the node id.
fn start_error(config: &VideoSourceConfig, detail: String) -> VideoError {
    match &config.kind {
        VideoSourceKind::Screen { node_id } => VideoError::ServerUnavailable(format!(
            "PipeWire node {node_id} could not be captured: {detail} \
             (the portal screen share may have ended)"
        )),
        _ => VideoError::State(detail),
    }
}

fn on_new_sample(
    sink: &gst_app::AppSink,
    slot: &FrameSlot,
    error: &Mutex<Option<String>>,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
    let caps = sample
        .caps()
        .ok_or_else(|| latch_flow_error(error, "the appsink sample has no caps"))?;
    let info = gst_video::VideoInfo::from_caps(caps)
        .map_err(|failure| latch_flow_error(error, format!("bad video caps: {failure}")))?;

    if info.format() != gst_video::VideoFormat::Rgba {
        return Err(latch_flow_error(
            error,
            format!("the pipeline negotiated {} instead of RGBA", info.format()),
        ));
    }

    let buffer = sample
        .buffer()
        .ok_or_else(|| latch_flow_error(error, "the appsink sample has no buffer"))?;
    let map = buffer.map_readable().map_err(|failure| {
        latch_flow_error(error, format!("could not map the frame: {failure}"))
    })?;
    let stride = info.stride()[0];

    if stride < 0 {
        return Err(latch_flow_error(
            error,
            format!("bottom-up frames are not supported (stride {stride})"),
        ));
    }

    slot.publish(info.width(), info.height(), stride as usize, map.as_slice())
        .map_err(|message| latch_flow_error(error, message))?;

    Ok(gst::FlowSuccess::Ok)
}

fn latch_flow_error(error: &Mutex<Option<String>>, message: impl Into<String>) -> gst::FlowError {
    let mut guard = error
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    *guard = Some(message.into());

    gst::FlowError::Error
}

fn drain_bus_error(bus: &gst::Bus) -> Option<String> {
    while let Some(message) = bus.pop() {
        if let gst::MessageView::Error(error) = message.view() {
            let debug = error
                .debug()
                .map(|debug| format!(" ({debug})"))
                .unwrap_or_default();

            return Some(format!("{}{debug}", error.error()));
        }
    }

    None
}

/// Wait up to `timeout_ms` for an error message, after draining what is queued.
fn wait_bus_error(bus: &gst::Bus, timeout_ms: u64) -> Option<String> {
    if let Some(error) = drain_bus_error(bus) {
        return Some(error);
    }

    let message = bus.timed_pop_filtered(
        gst::ClockTime::from_mseconds(timeout_ms),
        &[gst::MessageType::Error],
    )?;

    let gst::MessageView::Error(error) = message.view() else {
        return None;
    };
    let debug = error
        .debug()
        .map(|debug| format!(" ({debug})"))
        .unwrap_or_default();

    Some(format!("{}{debug}", error.error()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::test_support;
    use crate::video::{TestPattern, pipeline::required_elements};

    #[test]
    fn gstreamer_is_initialised_and_versioned() {
        ensure_gst().expect("GStreamer initialises");
        ensure_gst().expect("initialisation is idempotent");

        let version = gstreamer_version().expect("a version after init");

        assert!(
            version.split('.').count() >= 3,
            "unexpected version string {version:?}"
        );
        println!("GStreamer {version}");
    }

    #[test]
    fn stopping_twice_and_reading_after_stop_is_safe() {
        if let Some(reason) = test_support::missing_elements(&["videotestsrc", "appsink"]) {
            println!("skipping the stop test: {reason}");
            return;
        }

        let mut source = VideoSource::start(VideoSourceConfig::test_pattern(TestPattern::Smpte))
            .expect("test pattern source");

        source.stop();
        source.stop();

        assert!(!source.is_running());
        assert!(source.latest_frame().is_none());
        assert_eq!(source.status().state, VideoSourceState::Stopped);
    }

    #[test]
    fn a_test_pattern_delivers_varied_rgba_frames() {
        if let Some(reason) = test_support::missing_elements(&[
            "videotestsrc",
            "videoconvert",
            "videoscale",
            "videorate",
            "appsink",
        ]) {
            println!("skipping the test-pattern capture: {reason}");
            return;
        }

        let mut source = VideoSource::start(
            VideoSourceConfig::test_pattern(TestPattern::Smpte).with_mode(320, 240, 30),
        )
        .expect("test pattern source");
        let frame = test_support::wait_for_frame(&mut source, test_support::FRAME_TIMEOUT)
            .expect("a frame within the timeout");

        assert_eq!((frame.width, frame.height), (320, 240));
        assert_eq!(frame.rgba.len(), 320 * 240 * 4);
        assert!(frame.is_tightly_packed());
        assert!(frame.sequence >= 1);
        assert!(
            frame.has_varied_pixels(),
            "a colour-bar pattern must not be a flat colour"
        );

        let status = source.status();

        assert!(status.is_running(), "{status:?}");
        assert_eq!(status.width, 320);
        assert_eq!(status.height, 240);
        assert!(status.frames >= 1);
        assert!(status.last_error.is_none());

        source.stop();
    }

    #[test]
    fn frame_drains_recycle_buffers_and_restart_works() {
        if let Some(reason) = test_support::missing_elements(&["videotestsrc", "appsink"]) {
            println!("skipping the buffer-reuse capture: {reason}");
            return;
        }

        for round in 0..2 {
            let mut source = VideoSource::start(
                VideoSourceConfig::test_pattern(TestPattern::Smpte).with_mode(160, 120, 30),
            )
            .expect("test pattern source");

            let mut frame = VideoFrame::with_size(160, 120);
            let mut sequences = Vec::new();
            let deadline = std::time::Instant::now() + test_support::FRAME_TIMEOUT;

            // Warm up the two-buffer exchange: the streaming thread fills one
            // buffer while the engine holds the other.
            while sequences.len() < 2 && std::time::Instant::now() < deadline {
                if source.latest_frame_into(&mut frame) {
                    sequences.push(frame.sequence);
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }

            assert!(sequences.len() >= 2, "round {round}: {sequences:?}");

            let warmup = source.status().buffers_allocated;

            while sequences.len() < 12 && std::time::Instant::now() < deadline {
                if source.latest_frame_into(&mut frame) {
                    sequences.push(frame.sequence);

                    assert!(frame.is_tightly_packed());
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }

            assert!(sequences.len() >= 12, "round {round}: {sequences:?}");
            assert!(
                sequences.windows(2).all(|pair| pair[0] < pair[1]),
                "sequences advance: {sequences:?}"
            );

            let allocations = source.status().buffers_allocated;

            // A frame that arrives while the engine is between drains can force
            // one replacement buffer; the point is that allocations do not grow
            // with the frame count (no per-frame allocation, no leak).
            assert!(
                allocations <= warmup + 1,
                "round {round}: {allocations} allocations after {warmup} warmup allocations"
            );
            assert!(
                allocations <= crate::video::frame::POOL_CAPACITY as u64 + 2,
                "round {round}: {allocations} allocations exceed the bounded pool"
            );

            source.stop();
        }
    }

    #[test]
    fn a_camera_starts_when_one_is_connected() {
        if let Some(reason) = test_support::missing_elements(&["v4l2src", "appsink"]) {
            println!("skipping the camera capture: {reason}");
            return;
        }

        let _camera = test_support::live_video_lock();
        let cameras = crate::video::list_cameras();

        let id = match std::env::var("MODV_TEST_CAMERA_ID") {
            Ok(id) => {
                println!("using MODV_TEST_CAMERA_ID={id}");

                id
            }
            Err(_) => match cameras.first() {
                Some(camera) => camera.id.clone(),
                None => {
                    println!("skipping the camera capture: no V4L2 source devices on this host");
                    return;
                }
            },
        };

        // 1000×700 is not a camera mode: it proves `videoscale` fits the
        // device feed to the configured texture size (the legacy renderer's
        // `width/height: { ideal }` constraints did the same).
        let config = VideoSourceConfig::camera(id.clone()).with_mode(1000, 700, 30);

        match VideoSource::start(config) {
            Ok(mut source) => {
                if let Some(frame) =
                    test_support::wait_for_frame(&mut source, test_support::FRAME_TIMEOUT)
                {
                    assert_eq!((frame.width, frame.height), (1000, 700));
                    assert_eq!(frame.rgba.len(), 1000 * 700 * 4);
                    assert!(frame.is_tightly_packed());
                    println!(
                        "camera {id:?}: {}x{}, varied pixels: {}",
                        frame.width,
                        frame.height,
                        frame.has_varied_pixels()
                    );
                } else {
                    println!(
                        "camera {id:?} started but delivered no frame: {:?}",
                        source.status()
                    );
                }

                source.stop();
            }
            Err(error) => {
                // A camera can be present but held by another process in CI.
                println!("skipping the camera capture ({id:?}): {error}");
            }
        }
    }

    #[test]
    fn a_screen_source_starts_when_a_pipewire_node_is_provided() {
        let Ok(node_id) = std::env::var("MODV_TEST_PIPEWIRE_NODE") else {
            println!(
                "skipping the screen capture: set MODV_TEST_PIPEWIRE_NODE to a PipeWire node id to test it"
            );
            return;
        };
        let node_id: u32 = node_id
            .parse()
            .expect("MODV_TEST_PIPEWIRE_NODE is a number");

        let _video = test_support::live_video_lock();
        let support = crate::video::screen_support();

        if !support.available() {
            println!(
                "skipping the screen capture: {}",
                support.reason.as_deref().unwrap_or("unknown")
            );
            return;
        }

        let mut source = match VideoSource::start(VideoSourceConfig::screen(node_id)) {
            Ok(source) => source,
            Err(error) => {
                // A node id is only valid while its portal session lives, and
                // the node's producer may be held by another client.
                println!("skipping the screen capture on node {node_id}: {error}");

                return;
            }
        };

        if let Some(frame) = test_support::wait_for_frame(&mut source, test_support::FRAME_TIMEOUT)
        {
            assert!(frame.is_tightly_packed());
            println!("screen node {node_id}: {}x{}", frame.width, frame.height);
        } else {
            println!("screen node {node_id} started but delivered no frame");
        }

        source.stop();
    }

    #[test]
    fn a_video_file_decodes_to_frames() {
        if let Some(reason) = test_support::missing_elements(&[
            "videotestsrc",
            "videoconvert",
            "videoscale",
            "videorate",
            "appsink",
            "uridecodebin",
            "filesink",
        ]) {
            println!("skipping the file capture: {reason}");
            return;
        }

        let (path, generated) = test_support::video_file_fixture();

        if !generated {
            println!(
                "skipping the file capture: no suitable encoder is installed (tried vp8enc/webmmux, x264enc/matroskamux, theoraenc/oggmux)"
            );
            return;
        }

        let mut source = VideoSource::start(VideoSourceConfig::file(&path).with_mode(160, 120, 30))
            .unwrap_or_else(|error| panic!("decode {path:?}: {error}"));
        let frame = test_support::wait_for_frame(&mut source, test_support::FRAME_TIMEOUT)
            .unwrap_or_else(|| panic!("no decoded frame from {path:?}: {:?}", source.status()));

        assert_eq!(
            (frame.width, frame.height),
            (160, 120),
            "videoscale fits the file to the requested mode"
        );
        assert!(frame.is_tightly_packed());
        assert!(frame.rgba.len() == 160 * 120 * 4);
        assert!(frame.has_varied_pixels());

        source.stop();

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_video_file_loops_by_default() {
        if let Some(reason) =
            test_support::missing_elements(&["videotestsrc", "uridecodebin", "filesink", "appsink"])
        {
            println!("skipping the file loop: {reason}");
            return;
        }

        let (path, generated) = test_support::video_file_fixture();

        if !generated {
            println!("skipping the file loop: no suitable encoder is installed");
            return;
        }

        let mut source = VideoSource::start(VideoSourceConfig::file(&path).with_mode(160, 120, 30))
            .unwrap_or_else(|error| panic!("decode {path:?}: {error}"));

        assert!(
            source.is_looping(),
            "looping is the default, like <video loop>"
        );

        // The clip is one second at 30 fps; more than one clip's worth of
        // frames means the end-of-stream seek worked.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);

        while source.status().frames < 40 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let status = source.status();

        assert!(
            status.frames >= 40,
            "the file did not loop: {status:?} (clip {path:?})"
        );
        assert!(status.last_error.is_none(), "{status:?}");
        assert!(status.is_running(), "{status:?}");

        source.stop();

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_non_looping_video_file_reports_end_of_stream() {
        if let Some(reason) =
            test_support::missing_elements(&["videotestsrc", "uridecodebin", "filesink", "appsink"])
        {
            println!("skipping the file end-of-stream: {reason}");
            return;
        }

        let (path, generated) = test_support::video_file_fixture();

        if !generated {
            println!("skipping the file end-of-stream: no suitable encoder is installed");
            return;
        }

        let mut source = VideoSource::start(VideoSourceConfig::file(&path).with_mode(160, 120, 30))
            .unwrap_or_else(|error| panic!("decode {path:?}: {error}"));

        source.set_looping(false);
        assert!(!source.is_looping());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        let mut ended = false;

        while std::time::Instant::now() < deadline {
            if source.status().needs_restart() {
                ended = true;

                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let status = source.status();

        assert!(ended, "the file never reached end-of-stream: {status:?}");

        assert_eq!(
            status.state,
            VideoSourceState::Ended,
            "a finished file is `Ended`, not an error: {status:?}"
        );
        assert!(status.last_error.is_none(), "{status:?}");

        source.stop();

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn required_elements_are_checked_before_launching() {
        use crate::video::pipeline::{check_required_elements, plugin_hint};

        ensure_gst().expect("GStreamer initialises");

        for kind in [
            VideoSourceKind::Camera {
                id: "/dev/video0".to_string(),
            },
            VideoSourceKind::File {
                uri: "file:///a.mp4".to_string(),
            },
            VideoSourceKind::Screen { node_id: 1 },
            VideoSourceKind::TestPattern {
                pattern: TestPattern::Smpte,
            },
        ] {
            let missing: Vec<&str> = required_elements(&kind)
                .iter()
                .copied()
                .filter(|element| gst::ElementFactory::find(element).is_none())
                .collect();

            match check_required_elements(&kind) {
                Ok(()) => assert!(
                    missing.is_empty(),
                    "passed the check while {missing:?} are missing"
                ),
                Err(VideoError::PluginMissing(message)) => {
                    let element = *missing.first().expect("a missing element");

                    assert!(message.contains(element), "{message}");
                    assert!(
                        message.contains(plugin_hint(element)),
                        "the error names the package to install: {message}"
                    );
                    println!("{kind:?} needs {element:?}: {message}");
                }
                Err(other) => panic!("unexpected error for {kind:?}: {other}"),
            }
        }
    }
}
