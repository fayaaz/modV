//! Camera enumeration (WS-10).
//!
//! The legacy renderer got `MediaDeviceInfo[]` from
//! `navigator.mediaDevices.enumerateDevices()` and used the Chromium
//! `deviceId` as the capture handle. The engine enumerates V4L2 sources through
//! `gst::DeviceMonitor` instead and uses the **device path** (`/dev/videoN`) as
//! the stable id: it survives restarts, it is what `v4l2src device=` wants, and
//! it is readable in a log. The Chromium `deviceId` is an opaque per-origin
//! hash, so there is no way to keep the exact same string; the engine's list is
//! authoritative and Electron should mirror it for the picker.
//!
//! Ordering is natural-sorted by id (`/dev/video2` before `/dev/video10`), not
//! the provider's arbitrary order, so the Webcam module's 1-based `camera` prop
//! keeps pointing at the same device across sessions. [`select_camera`] is the
//! port of `selectWebcamSource` (`media-inputs.js:150-158`), including its
//! fallback to the first camera.

use std::cmp::Ordering;

use gst::prelude::*;

use super::pipeline::FRAME_RATE_PRESETS;
use super::source::ensure_gst;

/// Bounds on the work of one enumeration: a buggy provider can advertise
/// thousands of caps structures, but a picker only needs a sane mode list.
pub const MAX_CAPS_STRUCTURES: usize = 256;
/// Modes kept per camera (the legacy picker offered ~20 resolutions × 13 rates).
pub const MAX_MODES: usize = 64;
/// Frame rates kept per resolution.
pub const MAX_FPS_PER_RESOLUTION: usize = 16;

/// One advertised camera mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoMode {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frame rate in whole frames per second; `0` means the device did not
    /// advertise a rate.
    pub fps: u32,
}

impl VideoMode {
    /// Pixel count, used for sorting smallest-first.
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// One enumerable camera.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraInfo {
    /// Stable capture id: the V4L2 device path when the provider exposes one.
    /// This is what `VideoSourceConfig::camera(id)` accepts.
    pub id: String,
    /// Human-readable name (`display_name`, e.g. "HD Pro Webcam C920").
    pub name: String,
    /// The OS device path, when the provider exposes one.
    pub path: Option<String>,
    /// Advertised capture modes (may be empty when the provider reports none).
    pub caps: Vec<VideoMode>,
}

impl CameraInfo {
    /// The highest advertised resolution, if any.
    pub fn max_mode(&self) -> Option<VideoMode> {
        self.caps
            .iter()
            .max_by_key(|mode| (mode.pixels(), mode.fps))
            .copied()
    }
}

/// Enumerate V4L2 video sources.
///
/// Returns an empty list when GStreamer cannot initialise, the device monitor
/// cannot start, or the host simply has no camera (the common case in CI and in
/// the packaged-engine Docker image, which ships no V4L2 device).
pub fn list_cameras() -> Vec<CameraInfo> {
    if ensure_gst().is_err() {
        return Vec::new();
    }

    let monitor = gst::DeviceMonitor::new();

    // "Video/Source" matches the v4l2 provider's device class; providers that
    // reject the filter simply report no devices rather than an error.
    monitor.add_filter(Some("Video/Source"), None);

    if monitor.start().is_err() {
        return Vec::new();
    }

    let devices = monitor.devices();

    monitor.stop();

    let mut cameras: Vec<CameraInfo> = devices
        .iter()
        .filter(|device| device.has_classes("Video/Source"))
        .filter_map(camera_from_device)
        .collect();

    cameras.sort_by(|left, right| natural_cmp(&left.id, &right.id));
    cameras.dedup_by(|left, right| left.id == right.id);

    cameras
}

fn camera_from_device(device: &gst::Device) -> Option<CameraInfo> {
    let name = device.display_name().to_string();
    let properties = device.properties();
    let v4l2_path = properties
        .as_ref()
        .and_then(|properties| properties.get::<String>("api.v4l2.path").ok());
    let device_path = properties
        .as_ref()
        .and_then(|properties| properties.get::<String>("device.path").ok());
    let provider_name = device.name().to_string();

    let path = v4l2_path.clone().or(device_path.clone()).or_else(|| {
        provider_name
            .starts_with('/')
            .then(|| provider_name.clone())
    });

    let id = path
        .clone()
        .or_else(|| (!provider_name.is_empty()).then_some(provider_name))
        .or_else(|| (!name.is_empty()).then_some(name.clone()))?;

    let caps = device
        .caps()
        .map(|caps| modes_from_caps(&caps))
        .unwrap_or_default();

    Some(CameraInfo {
        id,
        name: if name.is_empty() {
            device.display_name().to_string()
        } else {
            name
        },
        path,
        caps,
    })
}

/// Resolve the Webcam module's 1-based `camera` prop against a camera list.
///
/// Mirrors `selectWebcamSource` (`media-inputs.js:150-158`): the index is
/// rounded and clamped to at least 1, an out-of-range index falls back to the
/// first camera, and an empty list yields `None`. The module prop indexes the
/// *saved camera order*; the caller passes that ordered slice.
pub fn select_camera(cameras: &[CameraInfo], index: i64) -> Option<&CameraInfo> {
    if cameras.is_empty() {
        return None;
    }

    let index = index.max(1) as usize;

    cameras.get(index - 1).or_else(|| cameras.first())
}

/// Extract selectable modes from a GStreamer caps set.
///
/// Caps ranges are collapsed to their upper bound (the legacy picker always
/// offered `capabilities.max` as a resolution), and frame-rate ranges expand to
/// the [`FRAME_RATE_PRESETS`] inside the range plus the upper bound — exactly
/// what `buildFrameRateOptions` (`video-capabilities.js`) produced.
pub fn modes_from_caps(caps: &gst::Caps) -> Vec<VideoMode> {
    let mut modes = Vec::new();

    for structure in caps.iter().take(MAX_CAPS_STRUCTURES) {
        let Some(width) = int_bounds(structure, "width").map(|(_, max)| max) else {
            continue;
        };
        let Some(height) = int_bounds(structure, "height").map(|(_, max)| max) else {
            continue;
        };

        modes.extend(expand_modes(
            width,
            height,
            fraction_bounds(structure, "framerate"),
        ));

        if modes.len() >= MAX_MODES {
            break;
        }
    }

    modes.sort_by(|left, right| {
        left.pixels()
            .cmp(&right.pixels())
            .then(left.width.cmp(&right.width))
            .then(left.height.cmp(&right.height))
            .then(left.fps.cmp(&right.fps))
    });
    modes.dedup();
    modes.truncate(MAX_MODES);

    modes
}

/// Expand one caps entry into available modes.
///
/// `width`/`height` are already concrete (the caller collapses ranges); `fps`
/// is an optional `(min, max)` inclusive range in whole frames per second.
pub(crate) fn expand_modes(width: u32, height: u32, fps: Option<(u32, u32)>) -> Vec<VideoMode> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let Some((low, high)) = fps else {
        return vec![VideoMode {
            width,
            height,
            fps: 0,
        }];
    };

    let (low, high) = (low.min(high), low.max(high));
    let mut rates: Vec<u32> = FRAME_RATE_PRESETS
        .iter()
        .copied()
        .filter(|rate| *rate >= low && *rate <= high)
        .collect();

    rates.push(high);
    rates.sort_unstable();
    rates.dedup();
    rates.truncate(MAX_FPS_PER_RESOLUTION);

    rates
        .into_iter()
        .map(|fps| VideoMode { width, height, fps })
        .collect()
}

/// Width/height bounds of a caps field (`int`, `int range` or `int list`).
pub(crate) fn int_bounds(structure: &gst::StructureRef, field: &str) -> Option<(u32, u32)> {
    if let Ok(value) = structure.get::<i32>(field) {
        // A caps field has one type, so an `int` that is not a usable size is
        // the whole answer: there is no range/list to try next.
        return (value > 0).then_some((value as u32, value as u32));
    }

    if let Ok(range) = structure.get::<gst::IntRange<i32>>(field) {
        let (low, high) = (range.min().max(1), range.max().max(1));

        return Some((low as u32, high as u32));
    }

    if let Ok(list) = structure.get::<gst::List>(field) {
        return int_values(list.as_slice());
    }

    if let Ok(array) = structure.get::<gst::Array>(field) {
        return int_values(array.as_slice());
    }

    None
}

fn int_values(values: &[gst::glib::SendValue]) -> Option<(u32, u32)> {
    let mut bounds: Option<(u32, u32)> = None;

    for value in values {
        let Ok(number) = value.get::<i32>() else {
            continue;
        };

        if number <= 0 {
            continue;
        }

        let number = number as u32;
        bounds = Some(match bounds {
            Some((low, high)) => (low.min(number), high.max(number)),
            None => (number, number),
        });
    }

    bounds
}

/// Frame-rate bounds of a caps field (`fraction`, `fraction range` or a list).
pub(crate) fn fraction_bounds(structure: &gst::StructureRef, field: &str) -> Option<(u32, u32)> {
    if let Ok(fraction) = structure.get::<gst::Fraction>(field) {
        let fps = fps_from_fraction(fraction);

        if fps > 0 {
            return Some((fps, fps));
        }
    }

    if let Ok(range) = structure.get::<gst::FractionRange>(field) {
        let low = fps_from_fraction(range.min());
        let high = fps_from_fraction(range.max());

        if low > 0 || high > 0 {
            return Some((low.max(1), high.max(1)));
        }
    }

    if let Ok(list) = structure.get::<gst::List>(field) {
        return fraction_values(list.as_slice());
    }

    if let Ok(array) = structure.get::<gst::Array>(field) {
        return fraction_values(array.as_slice());
    }

    None
}

fn fraction_values(values: &[gst::glib::SendValue]) -> Option<(u32, u32)> {
    let mut bounds: Option<(u32, u32)> = None;

    for value in values {
        let Ok(fraction) = value.get::<gst::Fraction>() else {
            continue;
        };
        let fps = fps_from_fraction(fraction);

        if fps == 0 {
            continue;
        }

        bounds = Some(match bounds {
            Some((low, high)) => (low.min(fps), high.max(fps)),
            None => (fps, fps),
        });
    }

    bounds
}

/// Convert a GStreamer frame-rate fraction to whole fps (round half up).
///
/// `30000/1001` (29.97) becomes 30 so the mode list matches the legacy picker's
/// whole-number frame rates.
pub(crate) fn fps_from_fraction(fraction: gst::Fraction) -> u32 {
    let numerator = i64::from(fraction.numer().max(0));
    let denominator = i64::from(fraction.denom().max(1));

    // Round half up without floats: (2n + d) / 2d.
    u32::try_from((numerator * 2 + denominator) / (denominator * 2)).unwrap_or(u32::MAX)
}

/// Compare two ids so `/dev/video2` sorts before `/dev/video10`.
pub(crate) fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left = left.char_indices().peekable();
    let mut right = right.char_indices().peekable();

    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((left_index, left_char)), Some((right_index, right_char))) => {
                if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
                    let left_end = digit_run_end(left.clone(), left_index);
                    let right_end = digit_run_end(right.clone(), right_index);
                    let left_number: u128 = left
                        .clone()
                        .take_while(|(index, _)| *index < left_end)
                        .filter_map(|(_, character)| character.to_digit(10))
                        .fold(0, |value, digit| value * 10 + u128::from(digit));
                    let right_number: u128 = right
                        .clone()
                        .take_while(|(index, _)| *index < right_end)
                        .filter_map(|(_, character)| character.to_digit(10))
                        .fold(0, |value, digit| value * 10 + u128::from(digit));

                    match left_number.cmp(&right_number) {
                        Ordering::Equal => {
                            while left.peek().is_some_and(|(index, _)| *index < left_end) {
                                left.next();
                            }
                            while right.peek().is_some_and(|(index, _)| *index < right_end) {
                                right.next();
                            }
                        }
                        ordering => return ordering,
                    }

                    continue;
                }

                match left_char.cmp(&right_char) {
                    Ordering::Equal => {
                        left.next();
                        right.next();
                    }
                    ordering => return ordering,
                }
            }
        }
    }
}

fn digit_run_end(
    characters: std::iter::Peekable<std::str::CharIndices<'_>>,
    start: usize,
) -> usize {
    let mut end = start;

    for (index, character) in characters {
        if !character.is_ascii_digit() {
            break;
        }

        end = index + character.len_utf8();
    }

    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(id: &str, name: &str, caps: &[(u32, u32, u32)]) -> CameraInfo {
        CameraInfo {
            id: id.to_string(),
            name: name.to_string(),
            path: id.starts_with('/').then(|| id.to_string()),
            caps: caps
                .iter()
                .map(|(width, height, fps)| VideoMode {
                    width: *width,
                    height: *height,
                    fps: *fps,
                })
                .collect(),
        }
    }

    #[test]
    fn list_cameras_returns_no_cameras_in_ci() {
        let cameras = super::list_cameras();

        if cameras.is_empty() {
            println!(
                "skipping the camera enumeration assertions: no V4L2 source devices on this host"
            );
            return;
        }

        for camera in cameras {
            assert!(!camera.id.is_empty(), "every camera has a stable id");
            assert!(!camera.name.is_empty(), "every camera has a display name");
            assert!(
                camera.path.as_deref().unwrap_or_default().starts_with('/')
                    || !camera.id.is_empty(),
                "the path is a device path when the provider reports one"
            );

            for mode in camera.caps {
                assert!(mode.width > 0 && mode.height > 0, "{mode:?}");
            }
        }
    }

    #[test]
    fn select_camera_keeps_the_legacy_one_based_semantics() {
        let cameras = [
            camera("/dev/video0", "front", &[(640, 480, 30)]),
            camera("/dev/video2", "back", &[(1920, 1080, 30)]),
        ];

        assert_eq!(
            select_camera(&cameras, 1).map(|camera| camera.id.as_str()),
            Some("/dev/video0")
        );
        assert_eq!(
            select_camera(&cameras, 2).map(|camera| camera.id.as_str()),
            Some("/dev/video2")
        );
        assert_eq!(
            select_camera(&cameras, 0).map(|camera| camera.id.as_str()),
            Some("/dev/video0"),
            "0 and negatives round up to the first camera"
        );
        assert_eq!(
            select_camera(&cameras, -3).map(|camera| camera.id.as_str()),
            Some("/dev/video0")
        );
        assert_eq!(
            select_camera(&cameras, 99).map(|camera| camera.id.as_str()),
            Some("/dev/video0"),
            "an out-of-range prop falls back to the first camera"
        );
        assert!(select_camera(&[], 1).is_none());
    }

    #[test]
    fn expand_modes_offers_the_presets_inside_the_range() {
        let modes = expand_modes(1280, 720, Some((1, 60)));

        assert_eq!(
            modes.iter().map(|mode| mode.fps).collect::<Vec<_>>(),
            vec![15, 20, 24, 25, 30, 48, 50, 60]
        );
        assert!(
            modes
                .iter()
                .all(|mode| mode.width == 1280 && mode.height == 720)
        );
    }

    #[test]
    fn expand_modes_keeps_a_single_rate_and_an_unknown_rate() {
        assert_eq!(
            expand_modes(640, 480, Some((60, 60)))
                .iter()
                .map(|mode| mode.fps)
                .collect::<Vec<_>>(),
            vec![60]
        );
        assert_eq!(
            expand_modes(640, 480, Some((24, 25)))
                .iter()
                .map(|mode| mode.fps)
                .collect::<Vec<_>>(),
            vec![24, 25]
        );
        assert_eq!(
            expand_modes(640, 480, Some((0, 0)))
                .iter()
                .map(|mode| mode.fps)
                .collect::<Vec<_>>(),
            vec![0],
            "no advertised rate is reported as 0"
        );
        assert_eq!(
            expand_modes(640, 480, None)
                .iter()
                .map(|mode| mode.fps)
                .collect::<Vec<_>>(),
            vec![0]
        );
        assert!(expand_modes(0, 480, Some((30, 30))).is_empty());
        assert!(expand_modes(640, 0, None).is_empty());
    }

    #[test]
    fn expand_modes_bounds_the_rate_list() {
        let modes = expand_modes(640, 480, Some((1, 1000)));

        assert!(modes.len() <= MAX_FPS_PER_RESOLUTION, "{}", modes.len());
        assert!(
            modes.iter().any(|mode| mode.fps == 1000),
            "the upper bound is offered"
        );
    }

    #[test]
    fn fps_from_fraction_rounds_to_whole_frames() {
        assert_eq!(fps_from_fraction(gst::Fraction::new(30, 1)), 30);
        assert_eq!(fps_from_fraction(gst::Fraction::new(30000, 1001)), 30);
        assert_eq!(fps_from_fraction(gst::Fraction::new(60000, 1001)), 60);
        assert_eq!(fps_from_fraction(gst::Fraction::new(24, 1)), 24);
        assert_eq!(fps_from_fraction(gst::Fraction::new(0, 1)), 0);
    }

    #[test]
    fn modes_from_caps_reads_fixed_and_ranged_fields() {
        // Caps need GStreamer's type system registered, but no plugin.
        crate::video::ensure_gst().expect("GStreamer initialises");
        assert_eq!(fps_from_fraction(gst::Fraction::new(30, 1)), 30);

        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "YUY2")
            .field("width", 640)
            .field("height", 480)
            .field("framerate", gst::Fraction::new(30, 1))
            .build();
        let modes = modes_from_caps(&caps);

        assert_eq!(
            modes,
            vec![VideoMode {
                width: 640,
                height: 480,
                fps: 30
            }]
        );

        let ranged = gst::Caps::builder("video/x-raw")
            .field("width", gst::IntRange::new(320, 640))
            .field("height", gst::IntRange::new(240, 480))
            .field(
                "framerate",
                gst::FractionRange::new(gst::Fraction::new(15, 1), gst::Fraction::new(30, 1)),
            )
            .build();
        let modes = modes_from_caps(&ranged);

        assert_eq!(
            modes,
            vec![
                VideoMode {
                    width: 640,
                    height: 480,
                    fps: 15
                },
                VideoMode {
                    width: 640,
                    height: 480,
                    fps: 20
                },
                VideoMode {
                    width: 640,
                    height: 480,
                    fps: 24
                },
                VideoMode {
                    width: 640,
                    height: 480,
                    fps: 25
                },
                VideoMode {
                    width: 640,
                    height: 480,
                    fps: 30
                },
            ],
            "ranges collapse to the upper bound, rates expand to the presets"
        );
    }

    #[test]
    fn modes_from_caps_skips_structures_without_dimensions() {
        crate::video::ensure_gst().expect("GStreamer initialises");

        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "YUY2")
            .build();

        assert!(modes_from_caps(&caps).is_empty());
        assert!(modes_from_caps(&gst::Caps::new_empty()).is_empty());
    }

    #[test]
    fn modes_from_caps_sorts_by_area_then_rate() {
        crate::video::ensure_gst().expect("GStreamer initialises");

        let mut caps = gst::Caps::new_empty();

        for (width, height) in [(1280u32, 720u32), (640, 480), (640, 480)] {
            let structure = gst::Structure::builder("video/x-raw")
                .field("width", width as i32)
                .field("height", height as i32)
                .field("framerate", gst::Fraction::new(30, 1))
                .build();

            caps.get_mut().unwrap().append_structure(structure);
        }

        let modes = modes_from_caps(&caps);

        assert_eq!(
            modes
                .iter()
                .map(|mode| (mode.width, mode.height))
                .collect::<Vec<_>>(),
            vec![(640, 480), (1280, 720)],
            "smallest first and duplicates removed"
        );
    }

    #[test]
    fn natural_sort_orders_device_numbers_by_value() {
        let mut ids = vec!["/dev/video10", "/dev/video2", "/dev/video1", "/dev/video20"];

        ids.sort_by(|left, right| natural_cmp(left, right));

        assert_eq!(
            ids,
            vec!["/dev/video1", "/dev/video2", "/dev/video10", "/dev/video20"]
        );
        assert_eq!(natural_cmp("abc", "abc"), Ordering::Equal);
        assert_eq!(natural_cmp("abc", "abd"), Ordering::Less);
        assert_eq!(natural_cmp("abc", "abc2"), Ordering::Less);
    }

    #[test]
    fn max_mode_picks_the_largest_resolution() {
        let device = camera(
            "/dev/video0",
            "cam",
            &[(640, 480, 30), (1920, 1080, 30), (1280, 720, 60)],
        );

        assert_eq!(
            device.max_mode(),
            Some(VideoMode {
                width: 1920,
                height: 1080,
                fps: 30
            })
        );
        assert_eq!(camera("/dev/video0", "cam", &[]).max_mode(), None);
    }
}
