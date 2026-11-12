//! Video capture (WS-10): cameras, video files and screen sources.
//!
//! This replaces the renderer's `getUserMedia` → hidden `<video>` →
//! `ImageCapture.grabFrame()` → `postMessage(ImageBitmap)` chain (cameras), the
//! `<video src>` + WebCodecs path (video files) and `getDisplayMedia` →
//! `ScreenPickerDialog` (screen capture) with one GStreamer graph per source.
//!
//! ```text
//! Electron renderer                         engine (this crate)
//! ----------------                         -------------------
//! mediaDevices.enumerateDevices()  ──────▶ list_cameras()
//! ScreenPickerDialog (portal consent) ────▶ VideoSourceKind::Screen { node_id }
//! videos/assignVideoStream             ───▶ VideoSourceKind::File { uri }
//! ```
//!
//! # Layout
//!
//! * [`camera`] — `CameraInfo`/`VideoMode` enumeration through
//!   `gst::DeviceMonitor`, plus the 1-based `camera` prop resolver.
//! * [`pipeline`] — `VideoSourceConfig` and the pure pipeline-description
//!   builders (so the exact `gst-launch` text can be unit tested).
//! * [`source`] — a running [`VideoSource`]: pipeline, appsink callbacks, the
//!   latest-frame slot, bus polling, status and stop.
//! * [`hub`] — [`VideoHub`], several sources keyed by a caller id.
//!
//! # Threading model
//!
//! GStreamer owns the capture/decode threads. Each source wires its `appsink`
//! to a `new-sample` callback that runs on the streaming thread, copies the
//! sample into a tightly packed RGBA8 frame and publishes it into a
//! [`frame::FrameSlot`]. The slot holds **one** ready frame plus a two-buffer
//! pool, so the streaming thread never grows memory when the consumer is slow:
//! a newer frame simply replaces the unread one and the old buffer is recycled
//! (the "newest frame wins" rule). The source publishes a `sequence` counter so
//! a consumer can tell whether it is looking at a new frame.
//!
//! The engine thread only ever takes the slot mutex for the duration of a
//! buffer swap ([`VideoSource::latest_frame_into`], which is allocation-free
//! once the two buffers exist). No GLib main loop is required: bus messages
//! (errors, end-of-stream) are drained from the engine thread whenever
//! [`VideoSource::status`], [`VideoSource::last_error`] or a frame drain is
//! called, so a dead camera surfaces as a latched error on the next poll.
//!
//! # Formats
//!
//! Camera, file and test-pattern pipelines end in
//! `video/x-raw,format=RGBA,width=W,height=H,framerate=F/1`; screen pipelines
//! keep the PipeWire node's native size/rate and only force RGBA (see
//! [`pipeline::VideoSourceKind::Screen`]). Either way
//! [`VideoFrame::rgba`] is always tightly packed row-major RGBA8 with
//! `width * height * 4` bytes and [`VideoFrame::width`]/[`VideoFrame::height`]
//! describe it — exactly what a `wgpu::TextureFormat::Rgba8Unorm` upload wants.
//! The legacy renderer drew the `ImageBitmap` into a per-device auxiliary
//! canvas; the engine instead uploads this buffer into a texture (see
//! `docs/rust-hybrid.md` WS-10).

pub mod camera;
pub mod frame;
pub mod hub;
pub mod pipeline;
pub mod source;

#[cfg(test)]
pub(crate) mod test_support;

pub use camera::{CameraInfo, VideoMode, list_cameras, select_camera};
pub use frame::VideoFrame;
pub use hub::VideoHub;
pub use pipeline::{
    DEFAULT_FPS, DEFAULT_HEIGHT, DEFAULT_WIDTH, FRAME_RATE_PRESETS, RESOLUTION_PRESETS,
    ScreenSupport, TestPattern, VideoSourceConfig, VideoSourceKind, path_to_file_uri,
    pipeline_description, screen_support,
};
pub use source::{
    VideoError, VideoSource, VideoSourceState, VideoSourceStatus, ensure_gst, gstreamer_version,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine owns sources and hands frames between threads; the public
    /// types have to move with them.
    #[test]
    fn public_types_can_move_between_threads() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<VideoSource>();
        assert_send::<VideoHub>();
        assert_send::<VideoSourceConfig>();
        assert_send::<CameraInfo>();
        assert_send::<VideoFrame>();
        assert_send::<VideoError>();
        assert_sync::<VideoHub>();
    }

    #[test]
    fn the_default_mode_is_the_legacy_texture_size() {
        let config = VideoSourceConfig::default();

        assert_eq!(
            (config.width, config.height, config.fps),
            (1280, 720, 30),
            "the engine's default texture is the legacy videoResolution"
        );
        assert!(config.caps_string().contains("format=RGBA"));
    }
}
