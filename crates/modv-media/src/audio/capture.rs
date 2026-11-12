//! Audio input-device capture (WS-9).
//!
//! Replaces the renderer's `getUserMedia` input path with `cpal` capture that
//! delivers interleaved f32 PCM in fixed [`CaptureConfig::block_frames`]
//! chunks. The analyser wants mono input, so [`downmix_mono`] folds the
//! capture channels down before [`super::analyser::AudioAnalyser::push_mono`].
//!
//! # Threading
//!
//! The cpal data callback runs on the audio thread and must never block. Every
//! callback both quantises the device format to f32 and fills a [`ChunkQueue`]
//! behind a mutex; when the consumer falls behind, the queue **drops the
//! oldest chunk** instead of waiting (the "capture never blocks the audio
//! callback" rule from WS-9). The consumer side is [`AudioCapture::next_chunk`],
//! which blocks on a condvar up to a caller-supplied timeout.
//!
//! `cpal::Stream` is not `Send` on every backend, so [`AudioCapture`] is meant
//! to live on the engine/render thread that also runs the analysis loop. The
//! PipeWire monitor in [`super::pipewire`] does its capture on a dedicated
//! thread instead.
//!
//! Device hot-plug is out of scope: enumerate with [`input_devices`], start
//! once, and restart the capture when the device disappears (WS-10 will add
//! hot-plug notifications).

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};

/// Engine capture sample rate (the band-analyser constants assume it).
pub const DEFAULT_SAMPLE_RATE: u32 = 48_000;
/// Engine capture channel count.
pub const DEFAULT_CHANNELS: u16 = 2;
/// Frames per chunk (`pw-record` transport constant).
pub const DEFAULT_BLOCK_FRAMES: usize = 480;
/// Chunks queued before the oldest is dropped.
pub const QUEUE_CAPACITY: usize = 64;

/// Capture request. `device` is a [`InputDeviceInfo::id`]; `None` selects the
/// default input device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureConfig {
    /// Device id, or `None` for the host default input device.
    pub device: Option<String>,
    /// Requested sample rate in Hz.
    pub sample_rate: u32,
    /// Requested channel count; the device may negotiate a different one.
    pub channels: u16,
    /// Frames per emitted chunk.
    pub block_frames: usize,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            device: None,
            sample_rate: DEFAULT_SAMPLE_RATE,
            channels: DEFAULT_CHANNELS,
            block_frames: DEFAULT_BLOCK_FRAMES,
        }
    }
}

/// One enumerable input device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// Human-readable device name.
    pub name: String,
    /// Stable cpal device id (`host:device`, see `cpal::DeviceId`).
    pub id: String,
    /// Default input channel count.
    pub channels: u16,
    /// Supported sample rates (the endpoints of every advertised range).
    pub sample_rates: Vec<u32>,
    /// Default input sample rate.
    pub default_sample_rate: u32,
}

/// Capture failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// No host backend / no devices at all.
    BackendUnavailable(String),
    /// The requested device was not found.
    DeviceNotFound(String),
    /// The device rejected the stream configuration.
    ConfigUnsupported(String),
    /// The stream could not be built or errored while running.
    Stream(String),
    /// The PipeWire server could not be reached.
    ServerUnavailable(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BackendUnavailable(message) => write!(f, "audio backend unavailable: {message}"),
            Self::DeviceNotFound(name) => write!(f, "audio input device not found: {name}"),
            Self::ConfigUnsupported(message) => {
                write!(f, "unsupported audio configuration: {message}")
            }
            Self::Stream(message) => write!(f, "audio capture stream error: {message}"),
            Self::ServerUnavailable(message) => {
                write!(f, "PipeWire server unavailable: {message}")
            }
        }
    }
}

impl std::error::Error for CaptureError {}

/// A bounded, blocking chunk queue shared by the capture callback and the
/// consumer. Pushing is non-blocking and evicts the oldest chunk when full.
#[derive(Clone, Debug)]
pub(crate) struct ChunkQueue {
    inner: Arc<(Mutex<QueueState>, Condvar)>,
    capacity: usize,
}

#[derive(Debug)]
struct QueueState {
    chunks: VecDeque<Vec<f32>>,
    closed: bool,
    error: Option<String>,
}

impl ChunkQueue {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new((
                Mutex::new(QueueState {
                    chunks: VecDeque::with_capacity(capacity),
                    closed: false,
                    error: None,
                }),
                Condvar::new(),
            )),
            capacity: capacity.max(1),
        }
    }

    /// Push a chunk without blocking; evicts the oldest when full.
    pub(crate) fn push(&self, chunk: Vec<f32>) {
        let (lock, condvar) = &*self.inner;
        let mut state = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        if state.chunks.len() >= self.capacity {
            state.chunks.pop_front();
        }

        state.chunks.push_back(chunk);
        condvar.notify_one();
    }

    /// Pop the oldest chunk, waiting up to `timeout` for one to arrive.
    pub(crate) fn pop(&self, timeout: Duration) -> Option<Vec<f32>> {
        let (lock, condvar) = &*self.inner;
        let mut state = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        if state.chunks.is_empty() && !state.closed {
            state = condvar
                .wait_timeout_while(state, timeout, |state| {
                    state.chunks.is_empty() && !state.closed
                })
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poison| poison.into_inner().0);
        }

        state.chunks.pop_front()
    }

    /// Mark the queue closed so a waiting consumer wakes up.
    pub(crate) fn close(&self) {
        let (lock, condvar) = &*self.inner;
        let mut state = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        state.closed = true;
        condvar.notify_all();
    }

    /// Record a stream error for [`AudioCapture::last_error`].
    pub(crate) fn set_error(&self, message: String) {
        let (lock, _) = &*self.inner;
        let mut state = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        state.error = Some(message);
    }

    pub(crate) fn take_error(&self) -> Option<String> {
        let (lock, _) = &*self.inner;
        let mut state = lock.lock().unwrap_or_else(|poison| poison.into_inner());

        state.error.take()
    }
}

/// Re-blocks a stream of interleaved samples into fixed-size chunks.
#[derive(Clone, Debug)]
pub(crate) struct ChunkAssembler {
    channels: usize,
    block_frames: usize,
    pending: Vec<f32>,
}

impl ChunkAssembler {
    pub(crate) fn new(channels: u16, block_frames: usize) -> Self {
        Self {
            channels: usize::from(channels).max(1),
            block_frames: block_frames.max(1),
            pending: Vec::new(),
        }
    }

    pub(crate) fn channels(&self) -> usize {
        self.channels
    }

    /// Change the channel count (a PipeWire format renegotiation).
    pub(crate) fn set_channels(&mut self, channels: u16) {
        self.channels = usize::from(channels).max(1);
        self.pending.clear();
    }

    /// Append interleaved samples and emit every complete block.
    pub(crate) fn push<F: FnMut(Vec<f32>)>(&mut self, interleaved: &[f32], mut emit: F) {
        self.pending.extend_from_slice(interleaved);

        let block_samples = self.channels * self.block_frames;

        while self.pending.len() >= block_samples {
            let rest = self.pending.split_off(block_samples);
            let block = std::mem::replace(&mut self.pending, rest);
            emit(block);
        }

        // A trailing partial frame would desynchronise the interleaving; keep
        // it until the rest of the frame arrives, but drop a permanently
        // dangling partial frame.
        if self.pending.len() >= self.channels {
            let aligned = self.pending.len() - (self.pending.len() % self.channels);
            self.pending.truncate(aligned);
        }
    }
}

/// Downmix interleaved PCM to mono.
///
/// * 1 channel is passed through unchanged;
/// * 2 or more channels average channels 0 and 1 (front L/R — for a 5.1
///   device the centre/LFE/surrounds are intentionally ignored);
/// * 0 channels, or a buffer shorter than one full frame, yields no output;
/// * a trailing partial frame is ignored.
pub fn downmix_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels);

    if channels == 0 {
        return Vec::new();
    }

    let frames = interleaved.len() / channels;
    let mut mono = Vec::with_capacity(frames);

    if channels == 1 {
        mono.extend_from_slice(&interleaved[..frames]);
        return mono;
    }

    for frame in 0..frames {
        let left = interleaved[frame * channels];
        let right = interleaved[frame * channels + 1];

        mono.push((left + right) * 0.5);
    }

    mono
}

/// One advertised input configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigRange {
    /// Channel count.
    pub channels: u16,
    /// Lowest supported sample rate.
    pub min_sample_rate: u32,
    /// Highest supported sample rate.
    pub max_sample_rate: u32,
    /// Sample format.
    pub sample_format: SampleFormat,
}

/// Pick the best advertised configuration for `sample_rate`/`channels`.
///
/// Preference order: a range covering the rate wins; among those, the exact
/// channel count and `f32` format are preferred, then the exact channel count,
/// then anything covering the rate. Returns an index into `ranges`.
pub fn choose_config(ranges: &[ConfigRange], sample_rate: u32, channels: u16) -> Option<usize> {
    let mut best: Option<(usize, u8)> = None;

    for (index, range) in ranges.iter().enumerate() {
        if sample_rate < range.min_sample_rate || sample_rate > range.max_sample_rate {
            continue;
        }

        let mut score = 1;

        if range.channels == channels {
            score += 2;
        }
        if range.sample_format == SampleFormat::F32 {
            score += 1;
        }

        if best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((index, score));
        }
    }

    best.map(|(index, _)| index)
}

/// Enumerate the host's input devices. Backend failures yield an empty list.
pub fn input_devices() -> Vec<InputDeviceInfo> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };

    let mut infos = Vec::new();

    for device in devices {
        let name = device
            .description()
            .map(|description| description.name().to_string())
            .unwrap_or_else(|_| "Unknown input".to_string());
        let id = device
            .id()
            .map(|id| id.to_string())
            .unwrap_or_else(|_| name.clone());
        let default = device.default_input_config().ok();
        let channels = default
            .as_ref()
            .map(|config| config.channels())
            .unwrap_or(0);
        let default_sample_rate = default
            .as_ref()
            .map(|config| config.sample_rate())
            .unwrap_or(DEFAULT_SAMPLE_RATE);

        let mut sample_rates = Vec::new();

        if let Ok(configs) = device.supported_input_configs() {
            for config in configs {
                sample_rates.push(config.min_sample_rate());
                sample_rates.push(config.max_sample_rate());
            }
        }

        sample_rates.push(default_sample_rate);
        sample_rates.sort_unstable();
        sample_rates.dedup();

        infos.push(InputDeviceInfo {
            name,
            id,
            channels,
            sample_rates,
            default_sample_rate,
        });
    }

    infos
}

/// Converts one cpal sample to an f32 in `-1..=1`.
trait SampleToF32 {
    fn to_f32(self) -> f32;
}

impl SampleToF32 for f32 {
    fn to_f32(self) -> f32 {
        self
    }
}

impl SampleToF32 for f64 {
    fn to_f32(self) -> f32 {
        self as f32
    }
}

impl SampleToF32 for i8 {
    fn to_f32(self) -> f32 {
        self as f32 / 128.0
    }
}

impl SampleToF32 for i16 {
    fn to_f32(self) -> f32 {
        self as f32 / 32_768.0
    }
}

impl SampleToF32 for i32 {
    fn to_f32(self) -> f32 {
        self as f32 / 2_147_483_648.0
    }
}

impl SampleToF32 for i64 {
    fn to_f32(self) -> f32 {
        self as f32 / 9_223_372_036_854_775_808.0
    }
}

impl SampleToF32 for u8 {
    fn to_f32(self) -> f32 {
        (self as f32 - 128.0) / 128.0
    }
}

impl SampleToF32 for u16 {
    fn to_f32(self) -> f32 {
        (self as f32 - 32_768.0) / 32_768.0
    }
}

impl SampleToF32 for u32 {
    fn to_f32(self) -> f32 {
        (self as f32 - 2_147_483_648.0) / 2_147_483_648.0
    }
}

impl SampleToF32 for u64 {
    fn to_f32(self) -> f32 {
        (self as f32 - 9_223_372_036_854_775_808.0) / 9_223_372_036_854_775_808.0
    }
}

impl SampleToF32 for cpal::I24 {
    fn to_f32(self) -> f32 {
        cpal::Sample::to_sample::<f32>(self)
    }
}

impl SampleToF32 for cpal::U24 {
    fn to_f32(self) -> f32 {
        cpal::Sample::to_sample::<f32>(self)
    }
}

/// Convert a raw cpal buffer to interleaved f32. Returns `None` for formats
/// this port does not handle (24-bit packed samples); callers then skip the
/// block rather than guessing at the layout.
fn convert_buffer(data: &cpal::Data) -> Option<Vec<f32>> {
    macro_rules! convert {
        ($ty:ty) => {
            data.as_slice::<$ty>()
                .map(|samples| samples.iter().map(|sample| sample.to_f32()).collect())
        };
    }

    match data.sample_format() {
        SampleFormat::F32 => convert!(f32),
        SampleFormat::F64 => convert!(f64),
        SampleFormat::I8 => convert!(i8),
        SampleFormat::I16 => convert!(i16),
        SampleFormat::I32 => convert!(i32),
        SampleFormat::I64 => convert!(i64),
        SampleFormat::U8 => convert!(u8),
        SampleFormat::U16 => convert!(u16),
        SampleFormat::U32 => convert!(u32),
        SampleFormat::U64 => convert!(u64),
        SampleFormat::I24 => convert!(cpal::I24),
        SampleFormat::U24 => convert!(cpal::U24),
        _ => None,
    }
}

/// A running input-device capture stream.
pub struct AudioCapture {
    stream: Option<cpal::Stream>,
    queue: ChunkQueue,
    sample_rate: u32,
    channels: u16,
    block_frames: usize,
}

impl AudioCapture {
    /// Start capturing from `config`.
    ///
    /// Enumerates devices, negotiates the closest advertised configuration
    /// (falling back to the device default), then builds an input stream and
    /// fills a bounded queue. [`Self::sample_rate`] / [`Self::channels`] report
    /// what the device actually negotiated.
    pub fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
        let host = cpal::default_host();
        let device = match &config.device {
            Some(wanted) => host
                .devices()
                .map_err(|error| CaptureError::BackendUnavailable(error.to_string()))?
                .find(|device| {
                    device
                        .id()
                        .map(|id| id.to_string() == *wanted)
                        .unwrap_or(false)
                })
                .ok_or_else(|| CaptureError::DeviceNotFound(wanted.clone()))?,
            None => host
                .default_input_device()
                .ok_or_else(|| CaptureError::DeviceNotFound("default input".to_string()))?,
        };

        let ranges: Vec<ConfigRange> = device
            .supported_input_configs()
            .map_err(|error| CaptureError::ConfigUnsupported(error.to_string()))?
            .map(|range| ConfigRange {
                channels: range.channels(),
                min_sample_rate: range.min_sample_rate(),
                max_sample_rate: range.max_sample_rate(),
                sample_format: range.sample_format(),
            })
            .collect();

        let (channels, sample_rate, sample_format) =
            match choose_config(&ranges, config.sample_rate, config.channels) {
                Some(index) => {
                    let range = ranges[index];

                    (range.channels, config.sample_rate, range.sample_format)
                }
                None => {
                    let default = device.default_input_config().map_err(|error| {
                        CaptureError::ConfigUnsupported(format!(
                            "no configuration covers {} Hz / {} ch: {error}",
                            config.sample_rate, config.channels
                        ))
                    })?;

                    (
                        default.channels(),
                        default.sample_rate(),
                        default.sample_format(),
                    )
                }
            };

        let stream_config = StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let queue = ChunkQueue::new(QUEUE_CAPACITY);
        let assembler = ChunkAssembler::new(channels, config.block_frames);
        let callback_queue = queue.clone();
        let error_queue = queue.clone();
        let mut assembler = assembler;

        let stream = device
            .build_input_stream_raw(
                &stream_config,
                sample_format,
                move |data, _info| {
                    let Some(samples) = convert_buffer(data) else {
                        return;
                    };

                    assembler.push(&samples, |chunk| callback_queue.push(chunk));
                },
                move |error| {
                    error_queue.set_error(error.to_string());
                },
                None,
            )
            .map_err(|error| CaptureError::Stream(error.to_string()))?;

        stream
            .play()
            .map_err(|error| CaptureError::Stream(error.to_string()))?;

        Ok(Self {
            stream: Some(stream),
            queue,
            sample_rate,
            channels,
            block_frames: config.block_frames,
        })
    }

    /// The negotiated sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The negotiated channel count.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// The configured chunk size in frames.
    pub fn block_frames(&self) -> usize {
        self.block_frames
    }

    /// The most recent stream error, if any.
    pub fn last_error(&self) -> Option<CaptureError> {
        self.queue.take_error().map(CaptureError::Stream)
    }

    /// Wait up to `timeout` for the next interleaved f32 chunk.
    ///
    /// Returns `Ok(None)` on timeout or after [`Self::stop`].
    pub fn next_chunk(&mut self, timeout: Duration) -> Result<Option<Vec<f32>>, CaptureError> {
        if let Some(error) = self.last_error() {
            return Err(error);
        }

        Ok(self.queue.pop(timeout))
    }

    /// Stop and drop the capture stream. Safe to call more than once.
    pub fn stop(&mut self) {
        self.stream = None;
        self.queue.close();
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_capture_config_matches_the_legacy_transport() {
        let config = CaptureConfig::default();

        assert_eq!(config.sample_rate, 48_000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.block_frames, 480);
        assert_eq!(config.device, None);
    }

    #[test]
    fn downmix_passes_mono_through() {
        assert_eq!(downmix_mono(&[0.1, -0.2, 0.3], 1), vec![0.1, -0.2, 0.3]);
    }

    #[test]
    fn downmix_averages_stereo_frames() {
        assert_eq!(
            downmix_mono(&[1.0, -1.0, 0.5, 0.25, 0.0, 0.0], 2),
            vec![0.0, 0.375, 0.0]
        );
    }

    #[test]
    fn downmix_takes_only_front_left_right_on_six_channels() {
        // Each frame is L, R, C, LFE, SL, SR; only L/R are averaged.
        let frame = |left: f32, right: f32| [left, right, 0.9, 0.9, 0.9, 0.9];
        let mut interleaved = Vec::new();

        for (left, right) in [(1.0, 0.0), (0.25, 0.75)] {
            interleaved.extend_from_slice(&frame(left, right));
        }

        assert_eq!(downmix_mono(&interleaved, 6), vec![0.5, 0.5]);
    }

    #[test]
    fn downmix_handles_empty_and_odd_buffers() {
        assert_eq!(downmix_mono(&[], 2), Vec::<f32>::new());
        assert_eq!(downmix_mono(&[0.5], 2), Vec::<f32>::new());
        assert_eq!(downmix_mono(&[1.0, 0.0, 0.75], 2), vec![0.5]);
        assert_eq!(downmix_mono(&[0.5, 0.25], 0), Vec::<f32>::new());
        assert_eq!(downmix_mono(&[], 0), Vec::<f32>::new());
    }

    #[test]
    fn chunk_assembler_emits_only_complete_blocks() {
        let mut assembler = ChunkAssembler::new(2, 2);
        let mut chunks = Vec::new();

        assembler.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], |chunk| chunks.push(chunk));
        assert_eq!(chunks, vec![vec![1.0, 2.0, 3.0, 4.0]]);

        assembler.push(&[7.0, 8.0], |chunk| chunks.push(chunk));
        assert_eq!(
            chunks,
            vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]]
        );
    }

    #[test]
    fn chunk_assembler_keeps_a_partial_block_between_callbacks() {
        let mut assembler = ChunkAssembler::new(1, 4);
        let mut chunks = Vec::new();

        assembler.push(&[1.0, 2.0], |chunk| chunks.push(chunk));
        assert!(chunks.is_empty());

        assembler.push(&[3.0, 4.0], |chunk| chunks.push(chunk));
        assert_eq!(chunks, vec![vec![1.0, 2.0, 3.0, 4.0]]);
    }

    #[test]
    fn chunk_queue_drops_the_oldest_chunk_on_overflow() {
        let queue = ChunkQueue::new(2);

        queue.push(vec![1.0]);
        queue.push(vec![2.0]);
        queue.push(vec![3.0]);

        assert_eq!(queue.pop(Duration::ZERO), Some(vec![2.0]));
        assert_eq!(queue.pop(Duration::ZERO), Some(vec![3.0]));
        assert_eq!(queue.pop(Duration::ZERO), None);
    }

    #[test]
    fn chunk_queue_wakes_a_waiter_within_the_timeout() {
        let queue = ChunkQueue::new(4);
        let producer = queue.clone();

        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            producer.push(vec![42.0]);
        });

        assert_eq!(queue.pop(Duration::from_secs(1)), Some(vec![42.0]));
        handle.join().expect("producer");
    }

    #[test]
    fn choose_config_prefers_f32_at_the_requested_rate_and_channels() {
        let ranges = [
            ConfigRange {
                channels: 2,
                min_sample_rate: 44_100,
                max_sample_rate: 48_000,
                sample_format: SampleFormat::I16,
            },
            ConfigRange {
                channels: 2,
                min_sample_rate: 44_100,
                max_sample_rate: 48_000,
                sample_format: SampleFormat::F32,
            },
            ConfigRange {
                channels: 1,
                min_sample_rate: 44_100,
                max_sample_rate: 48_000,
                sample_format: SampleFormat::F32,
            },
        ];

        assert_eq!(choose_config(&ranges, 48_000, 2), Some(1));
        assert_eq!(choose_config(&ranges, 48_000, 1), Some(2));
    }

    #[test]
    fn choose_config_rejects_rates_outside_every_range() {
        let ranges = [ConfigRange {
            channels: 2,
            min_sample_rate: 44_100,
            max_sample_rate: 48_000,
            sample_format: SampleFormat::F32,
        }];

        assert_eq!(choose_config(&ranges, 96_000, 2), None);
        assert_eq!(choose_config(&[], 48_000, 2), None);
    }

    #[test]
    fn sample_conversion_uses_full_scale_ranges() {
        assert_eq!(0i16.to_f32(), 0.0);
        assert_eq!(i16::MAX.to_f32(), 1.0 - 1.0 / 32_768.0);
        assert_eq!(i16::MIN.to_f32(), -1.0);
        assert_eq!(0u8.to_f32(), -1.0);
        assert_eq!(128u8.to_f32(), 0.0);
        assert_eq!(255u8.to_f32(), (255.0 - 128.0) / 128.0);
        assert_eq!(32_768u16.to_f32(), 0.0);
        assert!(f32::NAN.to_f32().is_nan());
    }

    /// The live smoke test only runs when the host actually has an input
    /// device; the Docker test image normally has none, so it skips loudly.
    #[test]
    fn live_capture_smoke_test_or_skip() {
        let config = CaptureConfig::default();

        match AudioCapture::start(config) {
            Ok(mut capture) => {
                let chunk = capture.next_chunk(Duration::from_millis(250));

                println!(
                    "live capture: {} Hz / {} ch, first chunk: {:?}",
                    capture.sample_rate(),
                    capture.channels(),
                    chunk.map(|chunk| chunk.map(|chunk| chunk.len()))
                );
                capture.stop();
            }
            Err(error) => {
                println!("skipping live capture smoke test (no input device): {error}");
            }
        }
    }
}
