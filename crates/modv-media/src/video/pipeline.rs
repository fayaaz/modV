//! Source configuration and the `gst-launch` pipeline description builder.
//!
//! The description is built as text and launched with
//! [`gst::parse::launch`](gst::parse::launch), which keeps the pipeline
//! inspectable (it is exactly the string a developer would paste into
//! `gst-launch-1.0`, and it appears in error messages). Everything here is a
//! pure function of the config so the tests can check device/property escaping
//! and the caps without a GPU, a camera or a plugin registry.
//!
//! Cameras, files and test patterns funnel through the same tail:
//!
//! ```text
//! v4l2src device=/dev/video0
//!   ! videoconvert ! videoscale ! videorate
//!   ! video/x-raw,format=RGBA,width=1280,height=720,framerate=30/1
//!   ! appsink name=modv_video_sink max-buffers=2 drop=true sync=false
//! ```
//!
//! Screen sources use a shorter tail that keeps the PipeWire node's native
//! size and rate:
//!
//! ```text
//! pipewiresrc path=42 ! videoconvert ! video/x-raw,format=RGBA
//!   ! appsink name=modv_video_sink max-buffers=2 drop=true sync=false
//! ```
//!
//! * `videoconvert` normalises the source's format to RGBA (v4l2 hands over
//!   YUY2/NV12, decodebin hands over whatever the file encodes).
//! * `videoscale` makes the requested size a *requirement the pipeline can
//!   always meet*, so a 1080p-only camera still produces the configured
//!   texture size instead of failing to negotiate (the legacy
//!   `getUserMedia({ width: { ideal } })` did the same by scaling).
//! * `videorate` holds the requested frame rate (files at 24 fps become 30 fps
//!   textures, cameras that ignore `framerate` get corrected).
//! * `appsink max-buffers=2 drop=true` is the in-element half of
//!   "newest frame wins": at most two samples are queued, older ones are
//!   replaced before they reach the callback.
//! * `sync=false` for live sources (camera/screen/test pattern) so the sink
//!   never waits on the clock — the device already paces capture. Video files
//!   use `sync=true` so a file plays back at real time like the legacy
//!   `<video>` element instead of as fast as the decoder runs.

use std::path::Path;

use super::source::{VideoError, ensure_gst};

/// Engine default frame width (matches the legacy `videoResolution` default).
pub const DEFAULT_WIDTH: u32 = 1280;
/// Engine default frame height.
pub const DEFAULT_HEIGHT: u32 = 720;
/// Engine default frame rate.
pub const DEFAULT_FPS: u32 = 30;

/// Largest requested dimension; anything above is clamped (a texture larger
/// than this is a configuration mistake, not a camera mode).
pub const MAX_DIMENSION: u32 = 16_384;
/// Largest requested frame rate.
pub const MAX_FPS: u32 = 1_000;

/// Name of the appsink element in every description, so the source can find it
/// after `gst::parse::launch`.
pub const SINK_NAME: &str = "modv_video_sink";

/// Resolution presets the legacy picker offered
/// (`video-capabilities.js:RESOLUTION_PRESETS`). Kept so the engine's camera
/// mode list is derived the same way the UI's picker was.
pub const RESOLUTION_PRESETS: &[(u32, u32)] = &[
    (176, 144),
    (320, 240),
    (352, 288),
    (640, 360),
    (640, 480),
    (800, 600),
    (960, 540),
    (1024, 768),
    (1280, 720),
    (1280, 960),
    (1366, 768),
    (1600, 900),
    (1600, 1200),
    (1920, 1080),
    (2048, 1536),
    (2560, 1440),
    (2592, 1944),
    (3264, 2448),
    (3840, 2160),
    (4096, 2160),
];

/// Frame-rate presets the legacy picker offered
/// (`video-capabilities.js:FRAME_RATE_PRESETS`).
pub const FRAME_RATE_PRESETS: &[u32] = &[15, 20, 24, 25, 30, 48, 50, 60, 72, 90, 120, 144, 240];

/// A `videotestsrc` pattern, used for pipeline tests and the "no camera"
/// fallback the engine can show while a device is being picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestPattern {
    /// `smpte` — colour bars.
    Smpte,
    /// `snow` — noise, useful for spotting a stalled pipeline.
    Snow,
    /// `black`.
    Black,
    /// `white`.
    White,
    /// `ball` — a moving ball.
    Ball,
    /// `circular`.
    Circular,
    /// `checkers-1`.
    Checkers1,
    /// `pinwheel`.
    Pinwheel,
    /// `gradient`.
    Gradient,
    /// Any other `videotestsrc` pattern name, passed through verbatim.
    Named(String),
}

impl TestPattern {
    /// Map a `videotestsrc pattern` name to a typed pattern.
    pub fn from_name(name: &str) -> Self {
        match name {
            "smpte" => Self::Smpte,
            "snow" => Self::Snow,
            "black" => Self::Black,
            "white" => Self::White,
            "ball" => Self::Ball,
            "circular" => Self::Circular,
            "checkers-1" => Self::Checkers1,
            "pinwheel" => Self::Pinwheel,
            "gradient" => Self::Gradient,
            other => Self::Named(other.to_string()),
        }
    }

    /// The `videotestsrc pattern` property value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Smpte => "smpte",
            Self::Snow => "snow",
            Self::Black => "black",
            Self::White => "white",
            Self::Ball => "ball",
            Self::Circular => "circular",
            Self::Checkers1 => "checkers-1",
            Self::Pinwheel => "pinwheel",
            Self::Gradient => "gradient",
            Self::Named(name) => name,
        }
    }
}

/// Where a [`super::VideoSource`] gets its frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoSourceKind {
    /// A V4L2 camera. `id` is [`super::CameraInfo::id`] (the device path, or a
    /// plain `/dev/videoN` path, or a bare index such as `"0"`).
    Camera {
        /// Camera id from [`super::list_cameras`].
        id: String,
    },
    /// A video file. `uri` is a GStreamer URI (`file:///…`, `http://…`); use
    /// [`VideoSourceConfig::file`] to build one from a path.
    File {
        /// GStreamer URI handed to `uridecodebin`.
        uri: String,
    },
    /// A PipeWire screen/application node. The portal consent dialog stays in
    /// Electron (`components/dialogs/ScreenPickerDialog.vue`); the engine only
    /// consumes the node id it hands over.
    ///
    /// The pipeline keeps the node's **native** size and frame rate: a portal
    /// screencast node advertises one capture format, and `pipewiresrc` fails
    /// negotiation on GStreamer 1.24/1.26 if asked for a different
    /// size/framerate. The delivered [`super::VideoFrame`] carries the actual
    /// dimensions, which the engine uses to size/scale its texture.
    Screen {
        /// PipeWire node id.
        node_id: u32,
    },
    /// A synthetic `videotestsrc` pattern.
    TestPattern {
        /// Pattern to generate.
        pattern: TestPattern,
    },
}

impl VideoSourceKind {
    /// Whether the source is paced by hardware (and therefore gets
    /// `appsink sync=false`).
    pub fn is_live(&self) -> bool {
        !matches!(self, Self::File { .. })
    }

    /// The `kind` label used in logs and status snapshots.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Camera { .. } => "camera",
            Self::File { .. } => "file",
            Self::Screen { .. } => "screen",
            Self::TestPattern { .. } => "test-pattern",
        }
    }
}

/// What to capture, and at which size/rate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSourceConfig {
    /// Frame origin.
    pub kind: VideoSourceKind,
    /// Output width; defaults to [`DEFAULT_WIDTH`].
    pub width: u32,
    /// Output height; defaults to [`DEFAULT_HEIGHT`].
    pub height: u32,
    /// Output frame rate; defaults to [`DEFAULT_FPS`].
    pub fps: u32,
}

impl Default for VideoSourceConfig {
    fn default() -> Self {
        Self {
            kind: VideoSourceKind::TestPattern {
                pattern: TestPattern::Smpte,
            },
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            fps: DEFAULT_FPS,
        }
    }
}

impl VideoSourceConfig {
    fn of(kind: VideoSourceKind) -> Self {
        Self {
            kind,
            ..Self::default()
        }
    }

    /// Capture from a camera (`CameraInfo::id`, `/dev/videoN` or an index).
    pub fn camera(id: impl Into<String>) -> Self {
        Self::of(VideoSourceKind::Camera { id: id.into() })
    }

    /// Play a video file. Accepts an absolute/relative path or a URI:
    /// `"/videos/a.mp4"` becomes `file:///videos/a.mp4`, while
    /// `"file:///videos/a.mp4"` and `"https://…"` are kept as they are.
    pub fn file(path_or_uri: impl AsRef<Path>) -> Self {
        let text = path_or_uri.as_ref().to_string_lossy();

        let uri = if text.contains("://") {
            text.into_owned()
        } else {
            path_to_file_uri(path_or_uri)
        };

        Self::of(VideoSourceKind::File { uri })
    }

    /// Capture a PipeWire screen/application node.
    pub fn screen(node_id: u32) -> Self {
        Self::of(VideoSourceKind::Screen { node_id })
    }

    /// Generate a test pattern.
    pub fn test_pattern(pattern: TestPattern) -> Self {
        Self::of(VideoSourceKind::TestPattern { pattern })
    }

    /// Set the output size.
    pub fn with_size(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Set the output frame rate.
    pub fn with_fps(mut self, fps: u32) -> Self {
        self.fps = fps;
        self
    }

    /// Set size and rate in one call.
    pub fn with_mode(self, width: u32, height: u32, fps: u32) -> Self {
        self.with_size(width, height).with_fps(fps)
    }

    /// Clamp missing/absurd values to something a pipeline can negotiate.
    ///
    /// Zero means "use the default", matching the store's behaviour where an
    /// unset `videoResolution` falls back to 1280×720@30.
    pub fn normalised(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            width: normalise_dimension(self.width, DEFAULT_WIDTH),
            height: normalise_dimension(self.height, DEFAULT_HEIGHT),
            fps: normalise_fps(self.fps),
        }
    }

    /// The negotiated output caps (`video/x-raw,format=RGBA,…`).
    pub fn caps_string(&self) -> String {
        let config = self.normalised();

        format!(
            "video/x-raw,format=RGBA,width={},height={},framerate={}/1",
            config.width, config.height, config.fps
        )
    }
}

fn normalise_dimension(value: u32, fallback: u32) -> u32 {
    if value == 0 {
        fallback
    } else {
        value.min(MAX_DIMENSION)
    }
}

fn normalise_fps(value: u32) -> u32 {
    if value == 0 {
        DEFAULT_FPS
    } else {
        value.min(MAX_FPS)
    }
}

/// Build the `gst-launch` description for a source.
///
/// Pure: no GStreamer call is made, so tests can assert the exact text (and the
/// escaping of device paths/URIs with spaces) without a plugin registry.
pub fn pipeline_description(config: &VideoSourceConfig) -> Result<String, VideoError> {
    let config = config.normalised();

    let source = match &config.kind {
        VideoSourceKind::Camera { id } => {
            let device = camera_device_argument(id)?;

            format!("v4l2src device={}", escape_launch_value(&device))
        }
        VideoSourceKind::File { uri } => {
            if !uri.contains("://") {
                return Err(VideoError::Unsupported(format!(
                    "video file uri {uri:?} is not absolute; use VideoSourceConfig::file(path)"
                )));
            }

            format!("uridecodebin uri={}", escape_launch_value(uri))
        }
        VideoSourceKind::Screen { node_id } => format!("pipewiresrc path={node_id}"),
        VideoSourceKind::TestPattern { pattern } => format!(
            "videotestsrc is-live=true pattern={}",
            escape_launch_value(pattern.as_str())
        ),
    };

    let sync = if config.kind.is_live() {
        "false"
    } else {
        "true"
    };

    // Screen nodes keep their native size and rate. A portal screencast node
    // advertises the capture size it was opened with, and `pipewiresrc` maps a
    // fixed downstream `width`/`height`/`framerate` back onto the node's
    // enumerated formats; asking for anything else fails negotiation on
    // GStreamer 1.24/1.26 (`no more input formats`). The engine sizes its
    // texture from `VideoFrame::width`/`height` instead.
    if matches!(config.kind, VideoSourceKind::Screen { .. }) {
        return Ok(format!(
            "{source} ! videoconvert ! video/x-raw,format=RGBA ! appsink name={SINK_NAME} max-buffers=2 drop=true sync={sync}"
        ));
    }

    Ok(format!(
        "{source} ! videoconvert ! videoscale ! videorate ! {} ! appsink name={SINK_NAME} max-buffers=2 drop=true sync={sync}",
        config.caps_string()
    ))
}

/// Turn a camera id into the value of `v4l2src device=`.
///
/// Accepts `CameraInfo::id` (a device path), a bare `"/dev/videoN"`, a bare
/// index (`"0"` → `/dev/video0`) and GStreamer's `v4l2:`-prefixed device names.
pub fn camera_device_argument(id: &str) -> Result<String, VideoError> {
    let trimmed = id.trim();

    if trimmed.is_empty() {
        return Err(VideoError::Unsupported(
            "camera id is empty; pass CameraInfo::id from list_cameras()".to_string(),
        ));
    }

    if trimmed.starts_with('/') {
        return Ok(trimmed.to_string());
    }

    match trimmed.strip_prefix("v4l2:") {
        Some(path) if path.starts_with('/') => return Ok(path.to_string()),
        _ => {}
    }

    if trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Ok(format!("/dev/video{trimmed}"));
    }

    Ok(trimmed.to_string())
}

/// Quote a property value for a `gst-launch` description.
///
/// Values made only of the characters GStreamer parses literally (`device
/// paths`, URIs, pattern names) stay unquoted so descriptions remain readable;
/// anything with spaces, quotes, backslashes, `!`, `;` or other syntax
/// characters is wrapped in double quotes with `\"`/`\\`/`\n` escapes.
pub fn escape_launch_value(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value
            .chars()
            .any(|character| !is_launch_safe_character(character));

    if !needs_quotes {
        return value.to_string();
    }

    let mut escaped = String::with_capacity(value.len() + 2);

    escaped.push('"');

    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }

    escaped.push('"');

    escaped
}

fn is_launch_safe_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(
            character,
            '-' | '_' | '.' | '/' | ':' | '@' | '%' | '+' | '=' | '~' | ',' | '#'
        )
}

/// Percent-encode a filesystem path into a `file://` URI.
///
/// Relative paths are joined onto the current directory first. Spaces and other
/// characters that `gst-launch` or `uridecodebin` would mis-parse are
/// percent-encoded, so the result is safe to hand to `uridecodebin uri=…`
/// without quoting.
pub fn path_to_file_uri(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    let text = absolute.to_string_lossy();
    let mut uri = String::with_capacity(text.len() + 8);

    uri.push_str("file://");

    for byte in text.as_bytes() {
        let character = *byte as char;

        if character.is_ascii_alphanumeric()
            || matches!(character, '-' | '_' | '.' | '~' | '/' | ':')
        {
            uri.push(character);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }

    uri
}

/// GStreamer elements a kind needs; checked before launching so a missing
/// plugin produces a readable error instead of a parse failure.
pub fn required_elements(kind: &VideoSourceKind) -> &'static [&'static str] {
    match kind {
        VideoSourceKind::Camera { .. } => &[
            "v4l2src",
            "videoconvert",
            "videoscale",
            "videorate",
            "appsink",
        ],
        VideoSourceKind::File { .. } => &[
            "uridecodebin",
            "videoconvert",
            "videoscale",
            "videorate",
            "appsink",
        ],
        VideoSourceKind::Screen { .. } => &["pipewiresrc", "videoconvert", "appsink"],
        VideoSourceKind::TestPattern { .. } => &[
            "videotestsrc",
            "videoconvert",
            "videoscale",
            "videorate",
            "appsink",
        ],
    }
}

/// Which Debian/Ubuntu package ships the element, for the error message.
pub fn plugin_hint(element: &str) -> &'static str {
    match element {
        "v4l2src" => "gstreamer1.0-plugins-good",
        "pipewiresrc" => "gstreamer1.0-pipewire",
        "uridecodebin" => "gstreamer1.0-plugins-base",
        "videoconvert" | "videoscale" | "videorate" | "videotestsrc" | "appsink" => {
            "gstreamer1.0-plugins-base"
        }
        _ => "the matching gstreamer1.0-plugins-* package",
    }
}

/// Verify the elements a source needs exist before launching its pipeline.
pub fn check_required_elements(kind: &VideoSourceKind) -> Result<(), VideoError> {
    ensure_gst()?;

    for element in required_elements(kind) {
        if gst::ElementFactory::find(element).is_none() {
            return Err(VideoError::PluginMissing(format!(
                "GStreamer element {element:?} is missing (install {})",
                plugin_hint(element)
            )));
        }
    }

    Ok(())
}

/// Whether the host can capture a PipeWire screen node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenSupport {
    /// `pipewiresrc` is available (the `gstreamer1.0-pipewire` plugin).
    pub plugin: bool,
    /// The PipeWire server socket, when one was found.
    pub server_socket: Option<std::path::PathBuf>,
    /// Why screen capture is unavailable, when it is.
    pub reason: Option<String>,
}

impl ScreenSupport {
    /// Both the element and a server are available.
    pub fn available(&self) -> bool {
        self.plugin && self.server_socket.is_some()
    }
}

/// Detect PipeWire screen-capture support without connecting to the server.
///
/// The portal consent UI runs in Electron; the engine only needs to know
/// whether a node id can be handed to `pipewiresrc`.
pub fn screen_support() -> ScreenSupport {
    let plugin = match ensure_gst() {
        Ok(()) => gst::ElementFactory::find("pipewiresrc").is_some(),
        Err(_) => false,
    };
    let server_socket = pipewire_socket();
    let reason = match (plugin, server_socket.is_some()) {
        (true, true) => None,
        (false, _) => {
            Some("the pipewiresrc element is missing (install gstreamer1.0-pipewire)".to_string())
        }
        (true, false) => Some(
            "no PipeWire server socket was found (is PipeWire/WirePlumber running?)".to_string(),
        ),
    };

    ScreenSupport {
        plugin,
        server_socket,
        reason,
    }
}

/// The PipeWire server socket, honouring `PIPEWIRE_REMOTE`.
fn pipewire_socket() -> Option<std::path::PathBuf> {
    let remote = std::env::var("PIPEWIRE_REMOTE")
        .ok()
        .filter(|remote| !remote.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute() && path.exists());

    if remote.is_some() {
        return remote;
    }

    let mut candidates = Vec::new();
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|runtime| !runtime.is_empty())
        .map(std::path::PathBuf::from);

    if let Some(runtime) = runtime {
        candidates.push(runtime.join("pipewire-0"));
    }

    // rustix/libc would be heavier than a single path probe here.
    candidates.push(std::path::PathBuf::from(format!(
        "/run/user/{}/pipewire-0",
        user_id()
    )));

    candidates.into_iter().find(|path| path.exists())
}

#[cfg(unix)]
fn user_id() -> u32 {
    use std::os::unix::fs::MetadataExt;

    // `/proc/self` is owned by the effective uid; this avoids a libc dependency.
    std::fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn user_id() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description(config: &VideoSourceConfig) -> String {
        pipeline_description(config).expect("pipeline description")
    }

    #[test]
    fn default_config_matches_the_legacy_transport() {
        let config = VideoSourceConfig::default();

        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
        assert_eq!(config.fps, 30);
        assert_eq!(
            config.kind,
            VideoSourceKind::TestPattern {
                pattern: TestPattern::Smpte
            }
        );
        assert!(!description(&config).contains("format=I420"));
    }

    #[test]
    fn normalisation_fills_missing_values_and_clamps_absurd_ones() {
        let config = VideoSourceConfig::camera("/dev/video0")
            .with_mode(0, 0, 0)
            .normalised();

        assert_eq!((config.width, config.height, config.fps), (1280, 720, 30));

        let clamped = VideoSourceConfig::camera("/dev/video0")
            .with_mode(MAX_DIMENSION + 10, MAX_DIMENSION + 10, MAX_FPS + 1)
            .normalised();

        assert_eq!(clamped.width, MAX_DIMENSION);
        assert_eq!(clamped.height, MAX_DIMENSION);
        assert_eq!(clamped.fps, MAX_FPS);
    }

    #[test]
    fn caps_string_is_tightly_packed_rgba_at_the_requested_mode() {
        let config = VideoSourceConfig::camera("/dev/video0").with_mode(640, 480, 60);

        assert_eq!(
            config.caps_string(),
            "video/x-raw,format=RGBA,width=640,height=480,framerate=60/1"
        );
    }

    #[test]
    fn camera_pipeline_uses_v4l2src_with_the_device_and_caps() {
        let text = description(&VideoSourceConfig::camera("/dev/video0").with_mode(640, 480, 30));

        assert!(text.starts_with("v4l2src device=/dev/video0 ! "), "{text}");
        assert!(
            text.contains(
                "! video/x-raw,format=RGBA,width=640,height=480,framerate=30/1 ! appsink name=modv_video_sink"
            ),
            "{text}"
        );
        assert!(text.contains("max-buffers=2 drop=true"), "{text}");
        assert!(
            text.ends_with("sync=false"),
            "a camera is live, so the sink must not wait on the clock: {text}"
        );
    }

    #[test]
    fn camera_ids_accept_paths_indexes_and_gstreamer_names() {
        assert_eq!(
            camera_device_argument("/dev/video3").expect("path"),
            "/dev/video3"
        );
        assert_eq!(camera_device_argument("3").expect("index"), "/dev/video3");
        assert_eq!(
            camera_device_argument("v4l2:/dev/video2").expect("gst name"),
            "/dev/video2"
        );
        assert_eq!(
            camera_device_argument("  /dev/video1  ").expect("padded path"),
            "/dev/video1"
        );
        assert!(camera_device_argument("").is_err());
        assert!(camera_device_argument("   ").is_err());
    }

    #[test]
    fn launch_values_are_quoted_only_when_needed() {
        assert_eq!(escape_launch_value("/dev/video0"), "/dev/video0");
        assert_eq!(escape_launch_value("smpte"), "smpte");
        assert_eq!(
            escape_launch_value("file:///tmp/a.mp4"),
            "file:///tmp/a.mp4"
        );
        assert_eq!(
            escape_launch_value("/dev/video 0"),
            "\"/dev/video 0\"",
            "a space would split the property"
        );
        assert_eq!(
            escape_launch_value(r#"/tmp/he said "hi".mp4"#),
            r#""/tmp/he said \"hi\".mp4""#,
            "quotes are escaped inside the quoted value"
        );
        assert_eq!(
            escape_launch_value("a\\b"),
            "\"a\\\\b\"",
            "backslashes are escaped"
        );
        assert_eq!(
            escape_launch_value("a!b"),
            "\"a!b\"",
            "! separates elements"
        );
        assert_eq!(escape_launch_value(""), "\"\"");
        assert_eq!(escape_launch_value("line\nbreak"), "\"line\\nbreak\"");
    }

    #[test]
    fn camera_pipeline_escapes_a_device_path_with_spaces() {
        let text = description(&VideoSourceConfig::camera("/dev/my camera"));

        assert!(
            text.starts_with("v4l2src device=\"/dev/my camera\" ! "),
            "{text}"
        );
    }

    #[test]
    fn file_pipeline_uses_uridecodebin_and_a_clock_synced_sink() {
        let config = VideoSourceConfig::file("/videos/clip.mp4").with_mode(320, 240, 30);
        let text = description(&config);

        assert!(
            text.starts_with("uridecodebin uri=file:///videos/clip.mp4 ! "),
            "{text}"
        );
        assert!(
            text.ends_with("sync=true"),
            "a file must play at real time, not as fast as it decodes: {text}"
        );
    }

    #[test]
    fn file_config_accepts_uris_and_paths() {
        assert_eq!(
            VideoSourceConfig::file("/videos/a b.mp4").kind,
            VideoSourceKind::File {
                uri: "file:///videos/a%20b.mp4".to_string()
            }
        );
        assert_eq!(
            VideoSourceConfig::file("https://example.com/a.mp4").kind,
            VideoSourceKind::File {
                uri: "https://example.com/a.mp4".to_string()
            }
        );
        assert_eq!(
            VideoSourceConfig::file("file:///already/a.uri").kind,
            VideoSourceKind::File {
                uri: "file:///already/a.uri".to_string()
            }
        );
    }

    #[test]
    fn file_pipeline_rejects_a_non_uri() {
        let config = VideoSourceConfig::of(VideoSourceKind::File {
            uri: "/videos/clip.mp4".to_string(),
        });

        let error = pipeline_description(&config).expect_err("a bare path is not a URI");

        assert!(matches!(error, VideoError::Unsupported(_)), "{error}");
    }

    #[test]
    fn path_to_file_uri_percent_encodes_reserved_characters() {
        assert_eq!(path_to_file_uri("/tmp/clip.mp4"), "file:///tmp/clip.mp4");
        assert_eq!(
            path_to_file_uri("/tmp/a b#c?.mp4"),
            "file:///tmp/a%20b%23c%3F.mp4"
        );
        assert_eq!(path_to_file_uri("/tmp/100%.mp4"), "file:///tmp/100%25.mp4");
        assert_eq!(
            path_to_file_uri("/tmp/caf\u{e9}.mp4"),
            "file:///tmp/caf%C3%A9.mp4"
        );
    }

    #[test]
    fn screen_pipeline_uses_the_pipewire_node_id_and_native_size() {
        let text = description(&VideoSourceConfig::screen(42).with_mode(1920, 1080, 60));

        assert!(
            text.starts_with("pipewiresrc path=42 ! videoconvert ! "),
            "{text}"
        );
        assert_eq!(
            text,
            "pipewiresrc path=42 ! videoconvert ! video/x-raw,format=RGBA ! appsink name=modv_video_sink max-buffers=2 drop=true sync=false",
            "a screen keeps the node's native size/rate so 1.24/1.26 pipewiresrc negotiates"
        );
    }

    #[test]
    fn test_pattern_pipeline_uses_the_pattern_name() {
        let text = description(&VideoSourceConfig::test_pattern(TestPattern::Snow));

        assert!(
            text.starts_with("videotestsrc is-live=true pattern=snow ! "),
            "{text}"
        );

        let custom = description(&VideoSourceConfig::test_pattern(TestPattern::from_name(
            "checkers-8",
        )));

        assert!(custom.contains("pattern=checkers-8"), "{custom}");
    }

    #[test]
    fn test_pattern_names_round_trip() {
        assert_eq!(TestPattern::from_name("smpte"), TestPattern::Smpte);
        assert_eq!(TestPattern::from_name("gradient").as_str(), "gradient");
        assert_eq!(
            TestPattern::from_name("something-new"),
            TestPattern::Named("something-new".to_string())
        );
        assert_eq!(
            TestPattern::from_name("something-new").as_str(),
            "something-new"
        );
    }

    #[test]
    fn required_elements_match_each_source_kind() {
        assert!(
            required_elements(&VideoSourceKind::Camera {
                id: "/dev/video0".to_string()
            })
            .contains(&"v4l2src")
        );
        assert_eq!(
            required_elements(&VideoSourceKind::Screen { node_id: 1 }),
            &["pipewiresrc", "videoconvert", "appsink"]
        );
        assert!(
            required_elements(&VideoSourceKind::File {
                uri: "file:///a.mp4".to_string()
            })
            .contains(&"uridecodebin")
        );
        assert!(
            required_elements(&VideoSourceKind::TestPattern {
                pattern: TestPattern::Smpte
            })
            .contains(&"videotestsrc")
        );

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
            for element in required_elements(&kind) {
                assert!(!plugin_hint(element).is_empty(), "{element} has a hint");
            }
        }
    }

    #[test]
    fn only_file_sources_are_not_live() {
        assert!(
            VideoSourceKind::Camera {
                id: "/dev/video0".to_string()
            }
            .is_live()
        );
        assert!(VideoSourceKind::Screen { node_id: 1 }.is_live());
        assert!(
            VideoSourceKind::TestPattern {
                pattern: TestPattern::Smpte
            }
            .is_live()
        );
        assert!(
            !VideoSourceKind::File {
                uri: "file:///a.mp4".to_string()
            }
            .is_live()
        );
    }

    #[test]
    fn builders_keep_the_default_mode() {
        let config = VideoSourceConfig::screen(7);

        assert_eq!((config.width, config.height, config.fps), (1280, 720, 30));
        assert_eq!(config.kind.label(), "screen");
    }

    /// Screen capture needs the `gstreamer1.0-pipewire` plugin and a running
    /// PipeWire server; the Docker engine image has neither, so report why.
    #[test]
    fn screen_support_reports_what_is_missing() {
        let support = screen_support();

        if support.available() {
            println!(
                "screen capture available through {:?}",
                support.server_socket
            );
            assert!(support.reason.is_none());
        } else {
            println!(
                "skipping live screen capture (plugin: {}, socket: {:?}): {}",
                support.plugin,
                support.server_socket,
                support.reason.as_deref().unwrap_or("unknown")
            );
            assert!(support.reason.is_some());
        }
    }
}
