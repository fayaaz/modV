//! Engine-owned audio capture and the analysis → render payload mapping (WS-9).
//!
//! `docs/rust-hybrid.md` ends WS-9 with the engine owning capture and analysis
//! end to end. This module is the engine half of that transport:
//!
//! * [`open_feed`] builds a [`modv_media::audio::engine::AudioFeed`] for the
//!   `--audio-source` selection — the default sink's PipeWire monitor
//!   (`pipewire`) or a cpal input device (`input`, `input:<device>`).
//! * [`snapshot_to_frame`] maps one
//!   [`modv_media::audio::analyser::AudioSnapshot`] onto the
//!   [`modv_renderers::two_d::AudioFrame`] shape the compositor, Input Links
//!   and renderers already read. The mapping is pure, so the wire-shape
//!   parity (band/cell/lane lengths, the 1024-slot byte spectrum, NaN
//!   sanitising, the sample-rate fallback) is unit-testable in plain Node-free
//!   Rust without a device.
//! * [`describe_devices`] renders the input-device table used by
//!   `--list-audio-devices` and by the `input:` validation error.
//!
//! The renderer's pump remains the default (`--audio-source renderer`); while
//! the engine owns capture, [`crate::store_ops::Store::set_engine_audio`] is
//! the only writer of [`crate::store_ops::Store::audio`] and renderer
//! `meyda/SET_BANDS` frames are ignored (see that method for the precedence
//! rules and the 10 Hz mirror publish).

use modv_media::audio::analyser::AudioSnapshot;
use modv_media::audio::capture::{CaptureConfig, InputDeviceInfo, input_devices};
use modv_media::audio::engine::AudioFeed;
use modv_renderers::two_d::{AudioFeatures, AudioFrame};

use crate::cli::AudioSourceSelection;

/// Sample rate reported when a snapshot carries a non-finite/zero rate
/// (the same 44.1 kHz fallback `meyda/SET_BANDS` uses for the renderer).
pub const FALLBACK_SAMPLE_RATE: f32 = 44_100.0;

/// Build the engine-owned feed for `selection`.
///
/// * `Pipewire` captures the default sink's monitor (`device` optionally names
///   a target sink node, passed through as `target.object`); it is Linux only.
/// * `Input` captures an input device (`device` is a name or cpal id resolved
///   through [`resolve_input_device`]; `None` is the host default input).
/// * `Renderer` has no engine feed; it is a programming error and reported as
///   such rather than silently starting a silent feed.
///
/// Errors are strings because every caller logs them and falls back to the
/// renderer's pump; the engine must keep running without audio.
pub fn open_feed(
    selection: AudioSourceSelection,
    device: Option<&str>,
) -> Result<AudioFeed, String> {
    match selection {
        AudioSourceSelection::Renderer => {
            Err("the renderer owns capture for --audio-source renderer".to_string())
        }
        AudioSourceSelection::Pipewire => open_monitor(device),
        AudioSourceSelection::Input => {
            let devices = input_devices();
            let device = match device {
                Some(wanted) => Some(resolve_input_device(wanted, &devices).ok_or_else(|| {
                    format!(
                        "no input device matches {wanted:?}\n{}",
                        describe_devices(&devices)
                    )
                })?),
                None => None,
            };

            AudioFeed::with_capture(CaptureConfig {
                device,
                ..CaptureConfig::default()
            })
            .map_err(|error| error.to_string())
        }
    }
}

/// Start the PipeWire monitor feed, or explain why this platform cannot.
#[cfg(target_os = "linux")]
fn open_monitor(device: Option<&str>) -> Result<AudioFeed, String> {
    AudioFeed::with_monitor(CaptureConfig {
        device: device.map(str::to_string),
        ..CaptureConfig::default()
    })
    .map_err(|error| error.to_string())
}

/// The native PipeWire monitor is Linux only (`modv-media` compiles it behind
/// `cfg(target_os = "linux")`).
#[cfg(not(target_os = "linux"))]
fn open_monitor(_device: Option<&str>) -> Result<AudioFeed, String> {
    Err("the pipewire source is Linux only; use --audio-source input".to_string())
}

/// Resolve a user-provided device name or id against the enumerated inputs.
///
/// Matching order: exact id (cpal's stable `host:device`), then a
/// case-insensitive exact name, then a case-insensitive exact id, then the
/// first case-insensitive name substring. Returns the cpal id that
/// [`CaptureConfig::device`] expects, or `None` when nothing matches (including
/// when the host enumerates no devices at all).
pub fn resolve_input_device(wanted: &str, devices: &[InputDeviceInfo]) -> Option<String> {
    let wanted = wanted.trim();

    if wanted.is_empty() {
        return None;
    }

    if let Some(device) = devices.iter().find(|device| device.id == wanted) {
        return Some(device.id.clone());
    }

    let lowered = wanted.to_lowercase();

    if let Some(device) = devices
        .iter()
        .find(|device| device.name.to_lowercase() == lowered)
    {
        return Some(device.id.clone());
    }

    if let Some(device) = devices
        .iter()
        .find(|device| device.id.to_lowercase() == lowered)
    {
        return Some(device.id.clone());
    }

    devices
        .iter()
        .find(|device| device.name.to_lowercase().contains(&lowered))
        .map(|device| device.id.clone())
}

/// Render the input-device table `--list-audio-devices` prints (and the
/// missing-device error appends).
///
/// An empty list is not an error: headless/CI hosts often have no ALSA
/// device, and the hint points at the system-audio source instead.
pub fn describe_devices(devices: &[InputDeviceInfo]) -> String {
    if devices.is_empty() {
        return "no audio input devices found (check the host's ALSA/PipeWire inputs, or use \
                MODV_AUDIO_SOURCE=pipewire to capture the system monitor)"
            .to_string();
    }

    let mut table = format!("audio input devices ({}):", devices.len());

    for (index, device) in devices.iter().enumerate() {
        let rates = if device.sample_rates.is_empty() {
            "unknown".to_string()
        } else {
            device
                .sample_rates
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };

        table.push_str(&format!(
            "\n  [{index}] {name}\n      id: {id}\n      channels: {channels}\n      sample rates (Hz): {rates}",
            name = device.name,
            id = device.id,
            channels = device.channels,
        ));
    }

    table
}

/// Print the host's input devices (`--list-audio-devices`).
pub fn print_devices() {
    println!("{}", describe_devices(&input_devices()));
}

/// Map one analyser snapshot onto the renderer wire shape.
///
/// Every scalar and array element is sanitised (`NaN`/`±∞` → `0`) because
/// Meyda's flatness/slope divide by sums that are zero for digital silence —
/// those `NaN`s would otherwise reach shader uniforms and prop writes. The
/// byte spectrum is already `0..255`, so it is copied verbatim (512 real bins
/// followed by the zero padding the wire shape keeps).
///
/// `features.buffer` is the 1024-sample wave rather than the analyser's
/// 512-sample Meyda window: that is the invariant `meyda/SET_BANDS` maintains
/// for `Waveform.js`, and keeping both writers identical means switching audio
/// sources never changes what a module sees.
pub fn snapshot_to_frame(snapshot: &AudioSnapshot) -> AudioFrame {
    let bands = sanitize_array(&snapshot.bands);
    let wave = sanitize_array(&snapshot.wave);
    let features = &snapshot.features;
    let sample_rate = if snapshot.sample_rate.is_finite() && snapshot.sample_rate > 0.0 {
        snapshot.sample_rate
    } else {
        FALLBACK_SAMPLE_RATE
    };

    AudioFrame {
        bass: bands[0],
        mid: bands[1],
        treble: bands[2],
        beat: sanitize(snapshot.beat),
        wave,
        byte_frequency_data: snapshot.byte_frequency_data,
        lanes: sanitize_array(&snapshot.lanes),
        cells: sanitize_array(&snapshot.cells),
        raw_lanes: sanitize_array(&snapshot.raw_lanes),
        raw_cells: sanitize_array(&snapshot.raw_cells),
        freq_mag: sanitize_array(&snapshot.freq_mag),
        sample_rate,
        features: AudioFeatures {
            rms: sanitize(features.rms),
            zcr: sanitize(features.zcr),
            energy: sanitize(features.energy),
            spectral_centroid: sanitize(features.spectral_centroid),
            spectral_flatness: sanitize(features.spectral_flatness),
            spectral_slope: sanitize(features.spectral_slope),
            spectral_rolloff: sanitize(features.spectral_rolloff),
            spectral_spread: sanitize(features.spectral_spread),
            spectral_skewness: sanitize(features.spectral_skewness),
            spectral_kurtosis: sanitize(features.spectral_kurtosis),
            perceptual_spread: sanitize(features.perceptual_spread),
            perceptual_sharpness: sanitize(features.perceptual_sharpness),
            buffer: wave.to_vec(),
        },
    }
}

/// `NaN`/`±∞` → `0`, everything else unchanged.
fn sanitize(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// [`sanitize`] over a fixed-size payload array.
fn sanitize_array<const N: usize>(values: &[f32; N]) -> [f32; N] {
    let mut sanitized = [0.0; N];

    for (slot, value) in sanitized.iter_mut().zip(values) {
        *slot = sanitize(*value);
    }

    sanitized
}

#[cfg(test)]
mod tests {
    use modv_media::audio::analyser::AudioAnalyser;

    use super::*;

    /// A real analyser frame (silent), then overridden: the snapshot shape is
    /// owned by `modv-media`, so building one through the analyser keeps this
    /// test honest about new fields.
    fn snapshot() -> AudioSnapshot {
        AudioAnalyser::new(48_000.0).frame(0.0)
    }

    #[test]
    fn mapping_carries_every_payload_field() {
        let mut snapshot = snapshot();

        snapshot.bands = [0.6, 0.4, 0.3];
        snapshot.lanes = [0.11; 12];
        snapshot.raw_lanes = [1.11; 12];
        snapshot.cells = [0.25; 16];
        snapshot.raw_cells = [2.25; 16];
        snapshot.beat = 0.75;
        snapshot.wave = [0.5; 1024];
        snapshot.freq_mag = [7.0; 1024];
        snapshot.byte_frequency_data[0] = 255;
        snapshot.byte_frequency_data[512] = 0;
        snapshot.sample_rate = 48_000.0;
        snapshot.features.rms = 0.3;
        snapshot.features.zcr = 0.05;
        snapshot.features.spectral_centroid = 1200.0;

        let frame = snapshot_to_frame(&snapshot);

        assert_eq!([frame.bass, frame.mid, frame.treble], [0.6, 0.4, 0.3]);
        assert_eq!(frame.beat, 0.75);
        assert_eq!(frame.lanes, [0.11; 12]);
        assert_eq!(frame.raw_lanes, [1.11; 12]);
        assert_eq!(frame.cells, [0.25; 16]);
        assert_eq!(frame.raw_cells, [2.25; 16]);
        assert_eq!(frame.wave, [0.5; 1024]);
        assert_eq!(frame.freq_mag, [7.0; 1024]);
        assert_eq!(frame.byte_frequency_data[0], 255);
        assert_eq!(frame.byte_frequency_data[512], 0);
        assert_eq!(frame.sample_rate, 48_000.0);
        assert_eq!(frame.features.rms, 0.3);
        assert_eq!(frame.features.zcr, 0.05);
        assert_eq!(frame.features.spectral_centroid, 1200.0);
        assert_eq!(
            frame.features.buffer,
            frame.wave.to_vec(),
            "features.buffer mirrors the wave for Waveform.js"
        );
    }

    #[test]
    fn mapping_sanitises_non_finite_values_and_falls_back_on_the_rate() {
        let mut source = snapshot();

        source.bands = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        source.lanes = [f32::NAN; 12];
        source.cells = [f32::INFINITY; 16];
        source.raw_lanes = [f32::NEG_INFINITY; 12];
        source.raw_cells = [f32::NAN; 16];
        source.beat = f32::NAN;
        source.wave = [f32::NAN; 1024];
        source.freq_mag = [f32::INFINITY; 1024];
        source.sample_rate = f32::NAN;
        source.features.rms = f32::NAN;
        source.features.spectral_flatness = f32::NAN;
        source.features.perceptual_sharpness = f32::INFINITY;

        let frame = snapshot_to_frame(&source);

        assert_eq!([frame.bass, frame.mid, frame.treble], [0.0; 3]);
        assert_eq!(frame.lanes, [0.0; 12]);
        assert_eq!(frame.cells, [0.0; 16]);
        assert_eq!(frame.raw_lanes, [0.0; 12]);
        assert_eq!(frame.raw_cells, [0.0; 16]);
        assert_eq!(frame.beat, 0.0);
        assert!(frame.wave.iter().all(|value| *value == 0.0));
        assert!(frame.freq_mag.iter().all(|value| *value == 0.0));
        assert_eq!(frame.sample_rate, FALLBACK_SAMPLE_RATE);
        assert_eq!(frame.features.rms, 0.0);
        assert_eq!(frame.features.spectral_flatness, 0.0);
        assert_eq!(frame.features.perceptual_sharpness, 0.0);
        assert!(frame.features.buffer.iter().all(|value| *value == 0.0));

        source.sample_rate = 0.0;
        assert_eq!(
            snapshot_to_frame(&source).sample_rate,
            FALLBACK_SAMPLE_RATE,
            "zero is not a usable analysis rate"
        );
    }

    #[test]
    fn mapping_keeps_a_silent_frame_all_zero() {
        let frame = snapshot_to_frame(&snapshot());

        assert_eq!([frame.bass, frame.mid, frame.treble], [0.0; 3]);
        assert_eq!(frame.sample_rate, 48_000.0);
        assert_eq!(frame.byte_frequency_data, [0u8; 1024]);
    }

    fn device(name: &str, id: &str) -> InputDeviceInfo {
        InputDeviceInfo {
            name: name.to_string(),
            id: id.to_string(),
            channels: 2,
            sample_rates: vec![44_100, 48_000],
            default_sample_rate: 48_000,
        }
    }

    #[test]
    fn device_resolution_prefers_the_id_then_the_name() {
        let devices = [
            device("Built-in Audio Analog Stereo", "alsa:builtin"),
            device("USB Microphone", "alsa:usb"),
        ];

        assert_eq!(
            resolve_input_device("alsa:usb", &devices),
            Some("alsa:usb".to_string())
        );
        assert_eq!(
            resolve_input_device("usb microphone", &devices),
            Some("alsa:usb".to_string())
        );
        assert_eq!(
            resolve_input_device("analog", &devices),
            Some("alsa:builtin".to_string()),
            "a name substring is enough"
        );
        assert_eq!(resolve_input_device("missing", &devices), None);
        assert_eq!(resolve_input_device("  ", &devices), None);
        assert_eq!(resolve_input_device("usb", &[]), None);
    }

    #[test]
    fn device_table_lists_name_id_channels_and_rates() {
        let table = describe_devices(&[device("USB Microphone", "alsa:usb")]);

        assert!(table.contains("audio input devices (1)"), "{table}");
        assert!(table.contains("USB Microphone"), "{table}");
        assert!(table.contains("id: alsa:usb"), "{table}");
        assert!(table.contains("channels: 2"), "{table}");
        assert!(table.contains("44100, 48000"), "{table}");
    }

    #[test]
    fn device_table_explains_an_empty_host() {
        let table = describe_devices(&[]);

        assert!(table.contains("no audio input devices"), "{table}");
        assert!(table.contains("MODV_AUDIO_SOURCE=pipewire"), "{table}");
    }

    #[test]
    fn renderer_selection_has_no_engine_feed() {
        let error = match open_feed(AudioSourceSelection::Renderer, None) {
            Ok(_) => panic!("the renderer selection must not build a feed"),
            Err(error) => error,
        };

        assert!(error.contains("renderer"), "{error}");
    }

    /// The live smoke tests skip loudly when the host has no input device or no
    /// PipeWire server (the Docker test image normally has neither).
    #[test]
    fn live_input_feed_smoke_test_or_skip() {
        match open_feed(AudioSourceSelection::Input, None) {
            Ok(mut feed) => {
                feed.frame(1.0 / 60.0);
                let rate = feed.sample_rate();

                println!("live input feed: {rate} Hz");
                feed.stop();
            }
            Err(error) => println!("skipping live input feed smoke test (no device): {error}"),
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn live_pipewire_feed_smoke_test_or_skip() {
        match open_feed(AudioSourceSelection::Pipewire, None) {
            Ok(mut feed) => {
                let snapshot = feed.frame(1.0 / 60.0);

                println!(
                    "live pipewire feed: {} Hz, bass {:.3}",
                    feed.sample_rate(),
                    snapshot.bands[0]
                );
                feed.stop();
            }
            Err(error) => println!("skipping live pipewire feed smoke test (no server): {error}"),
        }
    }

    /// `--audio-source input:<device>` must fail with the device table when the
    /// host cannot match the name, never start a silent feed.
    #[test]
    fn unknown_input_device_reports_the_table() {
        if input_devices().is_empty() {
            println!("skipping unknown-input-device test (no input devices)");
            return;
        }

        let error = match open_feed(AudioSourceSelection::Input, Some("definitely-not-a-device")) {
            Ok(_) => panic!("an unknown device must not start a feed"),
            Err(error) => error,
        };

        assert!(error.contains("definitely-not-a-device"), "{error}");
        assert!(error.contains("audio input devices"), "{error}");
    }

    /// A frame pumped from a silent feed still maps: the wiring is exercised
    /// without a device.
    #[test]
    fn silent_feed_frames_map_to_a_silent_audio_frame() {
        let mut feed = AudioFeed::new(48_000.0);
        let snapshot = feed.frame(0.0);
        let frame = snapshot_to_frame(&snapshot);

        assert_eq!([frame.bass, frame.mid, frame.treble], [0.0; 3]);
        assert_eq!(frame.sample_rate, 48_000.0);
    }
}
