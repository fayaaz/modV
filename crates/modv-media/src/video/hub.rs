//! Several capture sources keyed by a caller id.
//!
//! The engine runs one source per selected camera, per screen share and per
//! video-file module, and addresses them with the id the control plane gave
//! them (the renderer's `deviceId`, a screen node id, a video module id). The
//! hub is just that map plus a start/stop lifecycle: it owns no threads of its
//! own — each [`VideoSource`] owns its pipeline, and GStreamer owns the
//! streaming threads.

use std::collections::BTreeMap;

use super::frame::VideoFrame;
use super::pipeline::VideoSourceConfig;
use super::source::{VideoError, VideoSource, VideoSourceStatus};

/// A collection of running video sources, keyed by caller id.
///
/// Ids are kept in a `BTreeMap` so [`Self::statuses`] and [`Self::ids`] are
/// deterministic.
#[derive(Default)]
pub struct VideoHub {
    sources: BTreeMap<String, VideoSource>,
}

impl VideoHub {
    /// An empty hub.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start (or replace) the source registered as `id`.
    ///
    /// A restart **stops the previous pipeline for `id` first**: a V4L2 device
    /// can only be opened by one streaming client, so a new `v4l2src` fails
    /// with "Device is busy" while the old one is still running. The
    /// consequence is that a failed restart leaves no source registered for
    /// `id` (no half-started pipeline either); the engine retries on its next
    /// poll, exactly like the legacy renderer's reconnect loop.
    pub fn start_source(
        &mut self,
        id: impl Into<String>,
        config: VideoSourceConfig,
    ) -> Result<(), VideoError> {
        let id = id.into();

        if let Some(mut previous) = self.sources.remove(&id) {
            previous.stop();

            // Join the old pipeline's threads before the new start touches the
            // device.
            drop(previous);
        }

        let source = VideoSource::start(config)?;

        self.sources.insert(id, source);

        Ok(())
    }

    /// Stop and forget the source registered as `id`.
    ///
    /// Returns `false` when no source had that id.
    pub fn stop_source(&mut self, id: &str) -> bool {
        match self.sources.remove(id) {
            Some(mut source) => {
                source.stop();

                true
            }
            None => false,
        }
    }

    /// Stop every source.
    pub fn stop_all(&mut self) {
        for source in self.sources.values_mut() {
            source.stop();
        }

        self.sources.clear();
    }

    /// Drain the newest frame of `id` (allocation-free).
    ///
    /// Returns `false` when the id is unknown or no new frame is waiting.
    pub fn latest_into(&mut self, id: &str, frame: &mut VideoFrame) -> bool {
        self.sources
            .get_mut(id)
            .is_some_and(|source| source.latest_frame_into(frame))
    }

    /// Take the newest frame of `id` by value.
    pub fn latest(&mut self, id: &str) -> Option<VideoFrame> {
        self.sources.get_mut(id).and_then(VideoSource::latest_frame)
    }

    /// Status of `id`, or `None` when it is not running.
    pub fn status(&self, id: &str) -> Option<VideoSourceStatus> {
        self.sources.get(id).map(VideoSource::status)
    }

    /// Status of every source, sorted by id.
    pub fn statuses(&self) -> Vec<(String, VideoSourceStatus)> {
        self.sources
            .iter()
            .map(|(id, source)| (id.clone(), source.status()))
            .collect()
    }

    /// Whether `id` has a source that has not been stopped.
    pub fn is_running(&self, id: &str) -> bool {
        self.sources
            .get(id)
            .is_some_and(|source| source.is_running())
    }

    /// The registered ids, sorted.
    pub fn ids(&self) -> Vec<&str> {
        self.sources.keys().map(String::as_str).collect()
    }

    /// How many sources are registered.
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// Whether no source is registered.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Access one source (e.g. to toggle file looping).
    pub fn source(&self, id: &str) -> Option<&VideoSource> {
        self.sources.get(id)
    }

    /// Mutable access to one source.
    pub fn source_mut(&mut self, id: &str) -> Option<&mut VideoSource> {
        self.sources.get_mut(id)
    }
}

impl Drop for VideoHub {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::test_support;
    use crate::video::{TestPattern, VideoFrame};

    #[test]
    fn an_empty_hub_answers_missing_ids() {
        let mut hub = VideoHub::new();

        assert!(hub.is_empty());
        assert_eq!(hub.len(), 0);
        assert!(!hub.latest_into("missing", &mut VideoFrame::default()));
        assert!(hub.latest("missing").is_none());
        assert!(hub.status("missing").is_none());
        assert!(!hub.stop_source("missing"));
        assert!(!hub.is_running("missing"));
        assert_eq!(hub.ids(), Vec::<&str>::new());
        assert_eq!(hub.statuses(), Vec::new());
    }

    #[test]
    fn hub_runs_independent_sources_and_stops_them() {
        if let Some(reason) = test_support::missing_elements(&["videotestsrc", "appsink"]) {
            println!("skipping the hub capture: {reason}");
            return;
        }

        let mut hub = VideoHub::new();

        hub.start_source(
            "camera-a",
            VideoSourceConfig::test_pattern(TestPattern::Smpte).with_mode(160, 120, 30),
        )
        .expect("first source");
        hub.start_source(
            "camera-b",
            VideoSourceConfig::test_pattern(TestPattern::Snow).with_mode(320, 240, 30),
        )
        .expect("second source");

        assert_eq!(hub.ids(), vec!["camera-a", "camera-b"]);
        assert!(hub.is_running("camera-a"));

        let mut first = VideoFrame::with_size(160, 120);
        let mut second = VideoFrame::with_size(320, 240);
        let deadline = std::time::Instant::now() + test_support::FRAME_TIMEOUT;
        let (mut saw_first, mut saw_second) = (false, false);

        while !(saw_first && saw_second) && std::time::Instant::now() < deadline {
            saw_first |= hub.latest_into("camera-a", &mut first);
            saw_second |= hub.latest_into("camera-b", &mut second);

            if !(saw_first && saw_second) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        assert!(saw_first, "camera-a delivered no frame");
        assert!(saw_second, "camera-b delivered no frame");
        assert_eq!((first.width, first.height), (160, 120));
        assert_eq!((second.width, second.height), (320, 240));

        let statuses = hub.statuses();

        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].0, "camera-a");
        assert!(statuses[0].1.frames >= 1);

        assert!(hub.stop_source("camera-a"));
        assert!(!hub.stop_source("camera-a"));
        assert_eq!(hub.ids(), vec!["camera-b"]);

        hub.stop_all();

        assert!(hub.is_empty());
        assert!(!hub.is_running("camera-b"));
    }

    #[test]
    fn a_failed_restart_releases_the_previous_source() {
        if let Some(reason) = test_support::missing_elements(&["videotestsrc", "appsink"]) {
            println!("skipping the hub restart: {reason}");
            return;
        }

        let mut hub = VideoHub::new();

        hub.start_source(
            "camera",
            VideoSourceConfig::test_pattern(TestPattern::Smpte).with_mode(64, 48, 30),
        )
        .expect("initial source");

        let failure = hub
            .start_source(
                "camera",
                // A file URI that does not exist fails to preroll.
                VideoSourceConfig::file("/nonexistent/modv-engine-test.mp4").with_mode(64, 48, 30),
            )
            .expect_err("the missing file must fail");

        assert!(matches!(failure, VideoError::State(_)), "{failure}");
        assert!(
            !hub.is_running("camera"),
            "the old pipeline is released before the new one starts"
        );
        assert_eq!(hub.len(), 0, "no half-started source is registered");

        // Re-registering works: the engine's reconnect path just calls again.
        hub.start_source(
            "camera",
            VideoSourceConfig::test_pattern(TestPattern::Smpte).with_mode(64, 48, 30),
        )
        .expect("re-registering");

        assert!(hub.is_running("camera"));

        hub.stop_all();
    }

    #[test]
    fn restarting_a_camera_releases_the_device_first() {
        if let Some(reason) = test_support::missing_elements(&["v4l2src", "appsink"]) {
            println!("skipping the camera restart: {reason}");
            return;
        }

        let _camera = test_support::live_video_lock();
        let id = match std::env::var("MODV_TEST_CAMERA_ID") {
            Ok(id) => id,
            Err(_) => {
                let cameras = crate::video::list_cameras();

                match cameras.first() {
                    Some(camera) => camera.id.clone(),
                    None => {
                        println!(
                            "skipping the camera restart: no V4L2 source devices on this host"
                        );

                        return;
                    }
                }
            }
        };

        let mut hub = VideoHub::new();

        // The device can be held by another client (a PipeWire camera node, a
        // browser); skip instead of failing a shared-hardware run.
        if let Err(error) = hub.start_source(
            "camera",
            VideoSourceConfig::camera(id.clone()).with_mode(640, 480, 30),
        ) {
            println!("skipping the camera restart ({id:?}): {error}");

            return;
        }

        let mut frame = VideoFrame::with_size(640, 480);
        let deadline = std::time::Instant::now() + test_support::FRAME_TIMEOUT;

        while std::time::Instant::now() < deadline && !hub.latest_into("camera", &mut frame) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!((frame.width, frame.height), (640, 480));

        // A second v4l2src on the same device fails with "Device is busy"
        // unless the hub released the first pipeline.
        hub.start_source(
            "camera",
            VideoSourceConfig::camera(id.clone()).with_mode(320, 240, 30),
        )
        .unwrap_or_else(|error| panic!("restart of {id:?} while it was busy: {error}"));

        let mut frame = VideoFrame::with_size(320, 240);
        let deadline = std::time::Instant::now() + test_support::FRAME_TIMEOUT;
        let mut restarted = false;

        while std::time::Instant::now() < deadline {
            if hub.latest_into("camera", &mut frame) {
                restarted = true;

                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(
            restarted,
            "the restarted camera delivered no frame: {:?}",
            hub.status("camera")
        );
        assert_eq!((frame.width, frame.height), (320, 240));

        hub.stop_all();
    }
}
