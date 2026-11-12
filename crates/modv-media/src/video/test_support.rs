//! Helpers shared by the live (GPU/camera/plugin) tests.
//!
//! Every live test skips with a printed reason instead of failing when the
//! machine cannot run it: the Docker engine image has no V4L2 device and, with
//! only `libgstreamer1.0-dev` installed, no capture elements at all. The checks
//! that matter for CI (config, caps, pipeline text, escaping, buffer recycling)
//! are pure and always run.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gst::prelude::*;

use super::VideoFrame;
use super::pipeline::escape_launch_value;
use super::source::{VideoSource, ensure_gst};

/// How long a live test waits for its first frame.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(8);
/// How long the fixture encoder gets to write the test clip.
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(20);

/// `Some(reason)` when one of `elements` is not installed (or GStreamer cannot
/// initialise), `None` when the test can run.
pub(crate) fn missing_elements(elements: &[&str]) -> Option<String> {
    if let Err(error) = ensure_gst() {
        return Some(format!("GStreamer is unavailable: {error}"));
    }

    elements.iter().find_map(|element| {
        gst::ElementFactory::find(element)
            .is_none()
            .then(|| format!("the GStreamer element {element:?} is not installed"))
    })
}

/// Serialises tests that open real capture hardware.
///
/// A V4L2 device allows a single streaming client, and a PipeWire video node is
/// usually backed by the same device, so two live tests running in parallel see
/// `Device is busy` and fail for the wrong reason. The engine is unaffected:
/// it only opens the devices it was asked for.
pub(crate) fn live_video_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wait for the next frame of `source`, polling at ~200 Hz.
pub(crate) fn wait_for_frame(source: &mut VideoSource, timeout: Duration) -> Option<VideoFrame> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(frame) = source.latest_frame() {
            return Some(frame);
        }

        if Instant::now() >= deadline {
            return None;
        }

        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Write a tiny test clip and return `(path, generated)`.
///
/// `MODV_VIDEO_TEST_FILE` overrides the generated clip (useful where no encoder
/// plugin is installed but a fixture is mounted). Otherwise the clip is encoded
/// with the first available encoder of vp8/webm, h264/matroska, theora/ogg or
/// raw video in AVI (the only one that needs no external codec library, so the
/// Docker engine image can run the file test too).
/// `generated` is `false` when no encoder worked; the caller then skips.
pub(crate) fn video_file_fixture() -> (PathBuf, bool) {
    /// (label, required elements, file extension, description builder).
    type Candidate = (
        &'static str,
        &'static [&'static str],
        &'static str,
        fn(&str) -> String,
    );

    if let Some(path) = std::env::var_os("MODV_VIDEO_TEST_FILE").map(PathBuf::from) {
        if path.exists() {
            println!("using MODV_VIDEO_TEST_FILE={path:?}");

            return (path, true);
        }

        println!("MODV_VIDEO_TEST_FILE={path:?} does not exist; generating a clip instead");
    }

    let path = unique_temp_path("modv-video-fixture", "bin");

    let candidates: Vec<Candidate> = vec![
        ("vp8/webm", &["vp8enc", "webmmux"], "webm", |location| {
            format!(
                "videotestsrc num-buffers=30 pattern=smpte ! video/x-raw,width=160,height=120,framerate=30/1 ! vp8enc deadline=1 ! webmmux ! filesink location={location}"
            )
        }),
        (
            "h264/matroska",
            &["x264enc", "matroskamux"],
            "mkv",
            |location| {
                format!(
                    "videotestsrc num-buffers=30 pattern=smpte ! video/x-raw,width=160,height=120,framerate=30/1 ! x264enc speed-preset=ultrafast tune=zerolatency ! matroskamux ! filesink location={location}"
                )
            },
        ),
        ("theora/ogg", &["theoraenc", "oggmux"], "ogv", |location| {
            format!(
                "videotestsrc num-buffers=30 pattern=smpte ! video/x-raw,width=160,height=120,framerate=30/1 ! theoraenc ! oggmux ! filesink location={location}"
            )
        }),
        ("raw/avi", &["avimux", "avidemux"], "avi", |location| {
            format!(
                "videotestsrc num-buffers=30 pattern=smpte ! video/x-raw,format=YUY2,width=160,height=120,framerate=30/1 ! avimux ! filesink location={location}"
            )
        }),
    ];

    for (label, elements, extension, build) in candidates {
        if missing_elements(elements).is_some() {
            continue;
        }

        let path = path.with_extension(extension);
        let description = build(&escape_launch_value(&path.to_string_lossy()));

        match encode_clip(&description) {
            Ok(()) => {
                println!("generated the test clip with {label}: {path:?}");

                return (path, true);
            }
            Err(error) => {
                println!("could not encode the test clip with {label}: {error}");
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    (path, false)
}

fn encode_clip(description: &str) -> Result<(), String> {
    let element = gst::parse::launch(description)
        .map_err(|error| format!("{error} (pipeline: {description})"))?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| "gst_parse_launch built no pipeline".to_string())?;
    let bus = pipeline.bus().ok_or_else(|| "no bus".to_string())?;

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|failure| failure.to_string())?;

    let timeout = gst::ClockTime::from_nseconds(FIXTURE_TIMEOUT.as_nanos() as u64);
    let message =
        bus.timed_pop_filtered(timeout, &[gst::MessageType::Eos, gst::MessageType::Error]);

    let _ = pipeline.set_state(gst::State::Null);

    match message {
        Some(message) => match message.view() {
            gst::MessageView::Eos(..) => Ok(()),
            gst::MessageView::Error(error) => Err(error.error().to_string()),
            _ => Err("unexpected bus message".to_string()),
        },
        None => Err("timed out writing the fixture".to_string()),
    }
}

/// A path in the system temp directory that no other test run uses.
pub(crate) fn unique_temp_path(prefix: &str, extension: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    std::env::temp_dir().join(format!(
        "{prefix}-{}-{stamp}-{counter}.{extension}",
        std::process::id()
    ))
}
