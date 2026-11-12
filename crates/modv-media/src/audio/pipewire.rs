//! Native PipeWire monitor-source capture (WS-9, Linux only).
//!
//! Replaces the `pw-record` child process in
//! `src/main/system-audio-pipewire.js`: a `pipewire` crate stream connects to
//! the default server, captures the **default sink's monitor** (the same
//! `stream.capture.sink = true` mode `pw-record` was given), and delivers
//! interleaved f32 chunks through the same
//! [`super::capture::ChunkQueue`] as [`super::capture::AudioCapture`]. The
//! requested shape is 48 kHz / 2 ch / 480 frames; the server may negotiate
//! another supported rate/channel count, which is decoded and reported instead
//! of assumed (see *Format negotiation* below).
//!
//! # Threading model
//!
//! The `pipewire` crate's main loop owns `Rc`-based objects and must run on the
//! thread that created it, so [`PipeWireMonitor::start`] spawns a dedicated
//! thread (`modv-pw-monitor`) that builds the loop/context/stream and then
//! blocks in `MainLoop::run`. The data callback assembles chunks and pushes
//! them into the bounded queue (oldest dropped on overflow), exactly like the
//! cpal path. Dropping the monitor sends a quit message over a
//! `pipewire::channel` receiver attached to the loop and joins the thread, so
//! no loop thread is leaked.
//!
//! The stream is connected **without** `RT_PROCESS`: the callback runs on the
//! main loop thread, which is what makes it safe to lock the shared queue
//! there. `MAP_BUFFERS` gives the callback a mapped byte slice.
//!
//! # Format negotiation (R-2 / R-3)
//!
//! The server picks the final stream format; the `EnumFormat` pod is only a
//! request. `param_changed` is therefore the **only** place a format is
//! accepted:
//!
//! * [`negotiate`] accepts interleaved `F32LE`/`S16LE` at 8 kHz – 192 kHz with
//!   1–8 channels. [`decode_interleaved`] converts S16 to full-scale f32, so a
//!   44.1 kHz S16 monitor is analysed correctly instead of having its `i16`
//!   samples reinterpreted as garbage `f32`s.
//! * The `process` callback **parks** (drops) buffers until a format has been
//!   accepted, so no buffer is ever decoded with the wrong layout.
//! * A rejected format latches a readable error ([`ChunkQueue::error`]) *and*
//!   fails the readiness channel; it stays latched until the next accepted
//!   format (the queue clears it), so callers can distinguish "renegotiated
//!   successfully" from a one-shot transient.
//! * [`PipeWireMonitor::start`] returns only after the server negotiated a
//!   format, and reports the **negotiated** rate/channel count — the analyser
//!   is built from [`PipeWireMonitor::sample_rate`] /
//!   [`PipeWireMonitor::channels`], not from the request.
//!
//! # Errors
//!
//! `start` returns [`CaptureError::ServerUnavailable`] when no PipeWire daemon
//! answers (or negotiates a format) within [`START_TIMEOUT`]; callers (and the
//! smoke test) treat that as "no system audio available on this host" rather
//! than a fatal error. A negotiated format this port cannot decode fails
//! startup with [`CaptureError::ConfigUnsupported`]. Errors raised *after*
//! startup (a later renegotiation) surface through
//! [`PipeWireMonitor::next_chunk`] as [`CaptureError::Stream`].

use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use spa::param::ParamType;
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::pod::Pod;

use super::capture::{CaptureConfig, CaptureError, ChunkAssembler, ChunkQueue};

/// How long [`PipeWireMonitor::start`] waits for the loop thread to negotiate a
/// stream format.
pub const START_TIMEOUT: Duration = Duration::from_secs(5);
/// Stream name shown in `pw-top` / `wpctl`.
pub const STREAM_NAME: &str = "modv-system-audio";

/// Lowest sample rate the monitor decodes (anything below is a broken server
/// negotiation, not a usable stream).
pub const MIN_SUPPORTED_RATE: u32 = 8_000;
/// Highest sample rate the monitor decodes.
pub const MAX_SUPPORTED_RATE: u32 = 192_000;
/// Highest channel count the monitor decodes (surround monitors are downmixed
/// to their front L/R by [`super::capture::downmix_mono`]).
pub const MAX_SUPPORTED_CHANNELS: u32 = 8;

/// Sample layout of a negotiated raw-audio stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmFormat {
    /// Interleaved 32-bit float (the requested format).
    F32Le,
    /// Interleaved signed 16-bit.
    S16Le,
}

impl PcmFormat {
    /// Bytes per sample.
    pub fn bytes_per_sample(self) -> usize {
        match self {
            Self::F32Le => 4,
            Self::S16Le => 2,
        }
    }
}

/// A stream format the monitor accepted and knows how to decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NegotiatedFormat {
    /// Sample layout on the wire.
    pub sample: PcmFormat,
    /// Negotiated sample rate in Hz.
    pub rate: u32,
    /// Negotiated channel count (interleaved).
    pub channels: u32,
}

/// Decide whether a negotiated SPA audio format can be decoded.
///
/// Pure so the negotiation rules are testable without a PipeWire server. The
/// request is f32/stereo/48 kHz, but the server may hand back anything; only
/// interleaved `F32LE` and `S16LE` are decodable here (planar layouts and
/// 24/32-bit integer layouts are rejected with a descriptive error rather than
/// silently misinterpreted).
pub fn negotiate(
    format: spa::param::audio::AudioFormat,
    rate: u32,
    channels: u32,
) -> Result<NegotiatedFormat, String> {
    let sample = if format == spa::param::audio::AudioFormat::F32LE {
        PcmFormat::F32Le
    } else if format == spa::param::audio::AudioFormat::S16LE {
        PcmFormat::S16Le
    } else {
        return Err(format!(
            "unsupported PipeWire sample format {format:?} (need F32LE or S16LE interleaved)"
        ));
    };

    if !(MIN_SUPPORTED_RATE..=MAX_SUPPORTED_RATE).contains(&rate) {
        return Err(format!(
            "unsupported PipeWire sample rate {rate} Hz (supported {MIN_SUPPORTED_RATE}-{MAX_SUPPORTED_RATE} Hz)"
        ));
    }

    if !(1..=MAX_SUPPORTED_CHANNELS).contains(&channels) {
        return Err(format!(
            "unsupported PipeWire channel count {channels} (supported 1-{MAX_SUPPORTED_CHANNELS})"
        ));
    }

    Ok(NegotiatedFormat {
        sample,
        rate,
        channels,
    })
}

/// Decode one interleaved buffer of `format` PCM into `f32` samples.
///
/// A trailing partial sample (odd byte count for S16, non-multiple of four for
/// f32) is ignored; [`ChunkAssembler`] then re-blocks frames and drops a
/// dangling partial frame. S16 uses the same full-scale mapping as
/// [`super::capture`]'s cpal conversion (`i16 / 32768`).
///
/// Allocates a fresh `Vec`; the live `process` callback uses
/// [`decode_interleaved_into`] with maintained scratch space instead.
pub fn decode_interleaved(bytes: &[u8], format: PcmFormat) -> Vec<f32> {
    let mut samples = Vec::new();
    decode_interleaved_into(bytes, format, &mut samples);
    samples
}

/// Decode one interleaved buffer into `out`, reusing its allocation.
///
/// **Real-time constraint:** the `process` callback runs on the PipeWire loop
/// thread and must not allocate in steady state (R-22), so `out` lives in the
/// callback user data and is only reserved when a buffer arrives that is
/// bigger than the current capacity (a format/buffer-size change). The
/// capacity is kept afterwards so the next buffer reuses it.
pub fn decode_interleaved_into(bytes: &[u8], format: PcmFormat, out: &mut Vec<f32>) {
    out.clear();

    match format {
        PcmFormat::F32Le => {
            let chunks = bytes.as_chunks::<4>().0;

            if out.capacity() < chunks.len() {
                out.reserve(chunks.len());
            }

            for sample in chunks {
                out.push(f32::from_le_bytes(*sample));
            }
        }
        PcmFormat::S16Le => {
            let chunks = bytes.as_chunks::<2>().0;

            if out.capacity() < chunks.len() {
                out.reserve(chunks.len());
            }

            for sample in chunks {
                out.push(f32::from(i16::from_le_bytes(*sample)) / 32_768.0);
            }
        }
    }
}

/// Format state shared by the `param_changed` and `process` callbacks.
///
/// `param_changed` records the accepted format; `process` decodes nothing
/// until one was accepted (buffers are parked). A rejection latches its
/// message in the queue and fails the readiness channel exactly once, so
/// [`PipeWireMonitor::next_chunk`] can keep reporting it without the error
/// being a one-shot that a single log line swallows.
struct FormatState {
    negotiated: Option<NegotiatedFormat>,
    ready: Option<SyncSender<Result<NegotiatedFormat, CaptureError>>>,
    queue: ChunkQueue,
}

impl FormatState {
    fn new(ready: SyncSender<Result<NegotiatedFormat, CaptureError>>, queue: ChunkQueue) -> Self {
        Self {
            negotiated: None,
            ready: Some(ready),
            queue,
        }
    }

    /// The accepted format, if the server has negotiated a usable one.
    fn negotiated(&self) -> Option<NegotiatedFormat> {
        self.negotiated
    }

    /// Records a `SPA_PARAM_Format` outcome. Returns whether the format was
    /// accepted.
    fn set_format(
        &mut self,
        format: spa::param::audio::AudioFormat,
        rate: u32,
        channels: u32,
    ) -> bool {
        match negotiate(format, rate, channels) {
            Ok(negotiated) => {
                self.negotiated = Some(negotiated);
                // A successful (re)negotiation clears an earlier rejection.
                self.queue.clear_error();
                if let Some(ready) = self.ready.take() {
                    let _ = ready.send(Ok(negotiated));
                }
                true
            }
            Err(message) => {
                self.reject(message);
                false
            }
        }
    }

    /// Latch a negotiation failure: the stream stays parked and the caller is
    /// told once (through the readiness channel, or through
    /// [`PipeWireMonitor::next_chunk`] after startup).
    fn reject(&mut self, message: String) {
        self.negotiated = None;
        self.queue.set_error(message.clone());
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(Err(CaptureError::ConfigUnsupported(message)));
        }
    }

    /// Decode one buffer into `out`, or `false` while no format is accepted.
    ///
    /// Used by the real-time `process` callback with a scratch buffer it owns,
    /// so decoding does not allocate per buffer (R-22).
    fn decode_into(&self, bytes: &[u8], out: &mut Vec<f32>) -> bool {
        match self.negotiated() {
            Some(negotiated) => {
                decode_interleaved_into(bytes, negotiated.sample, out);
                true
            }
            None => false,
        }
    }
}

/// User data handed to the PipeWire callbacks.
struct UserData {
    format: spa::param::audio::AudioInfoRaw,
    assembler: ChunkAssembler,
    formats: FormatState,
    /// Decode scratch, owned by the loop thread so `process` does not
    /// allocate a `Vec` per buffer.
    scratch: Vec<f32>,
}

/// A running PipeWire monitor capture.
pub struct PipeWireMonitor {
    quit: Option<pw::channel::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    queue: ChunkQueue,
    format: NegotiatedFormat,
}

impl PipeWireMonitor {
    /// Connect to the default PipeWire server and start capturing the default
    /// sink's monitor.
    ///
    /// `config.device` is an optional target sink `node.name` / object id
    /// (`target.object`); `None` follows the default sink.
    ///
    /// Returns only once the server has negotiated a decodable stream format
    /// (or with an error explaining why it did not).
    pub fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
        let queue = ChunkQueue::new(super::capture::QUEUE_CAPACITY);
        let (quit_tx, quit_rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = sync_channel::<Result<NegotiatedFormat, CaptureError>>(1);
        let thread_queue = queue.clone();

        let handle = thread::Builder::new()
            .name("modv-pw-monitor".to_string())
            .spawn(move || monitor_thread(config, thread_queue, ready_tx, quit_rx))
            .map_err(|error| CaptureError::ServerUnavailable(error.to_string()))?;

        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(format)) => Ok(Self {
                quit: Some(quit_tx),
                handle: Some(handle),
                queue,
                format,
            }),
            Ok(Err(error)) => {
                let _ = quit_tx.send(());
                let _ = handle.join();
                Err(error)
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = quit_tx.send(());
                let _ = handle.join();
                Err(CaptureError::ServerUnavailable(format!(
                    "timed out after {}s waiting for the PipeWire stream to negotiate a format",
                    START_TIMEOUT.as_secs()
                )))
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = handle.join();
                Err(CaptureError::ServerUnavailable(
                    "PipeWire monitor thread exited during startup".to_string(),
                ))
            }
        }
    }

    /// The **negotiated** sample rate in Hz (the analyser is built from it).
    pub fn sample_rate(&self) -> u32 {
        self.format.rate
    }

    /// The **negotiated** channel count (the monitor emits this interleaved).
    pub fn channels(&self) -> u16 {
        u16::try_from(self.format.channels).unwrap_or(1)
    }

    /// The full negotiated format (sample layout, rate and channels).
    pub fn negotiated_format(&self) -> NegotiatedFormat {
        self.format
    }

    /// Wait up to `timeout` for the next interleaved f32 chunk.
    ///
    /// Returns `Ok(None)` on timeout. A latched format/stream error (a
    /// renegotiation the monitor cannot decode, or a capture error) is
    /// returned as [`CaptureError::Stream`] on **every** call until a later
    /// negotiation succeeds.
    pub fn next_chunk(&mut self, timeout: Duration) -> Result<Option<Vec<f32>>, CaptureError> {
        if let Some(error) = self.queue.error() {
            return Err(CaptureError::Stream(error));
        }

        Ok(self.queue.pop(timeout))
    }

    /// Stop the loop thread and close the queue. Idempotent.
    pub fn stop(&mut self) {
        if let Some(quit) = self.quit.take() {
            let _ = quit.send(());
        }

        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }

        self.queue.close();
    }
}

impl Drop for PipeWireMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Body of the dedicated PipeWire loop thread.
fn monitor_thread(
    config: CaptureConfig,
    queue: ChunkQueue,
    ready: SyncSender<Result<NegotiatedFormat, CaptureError>>,
    quit_rx: pw::channel::Receiver<()>,
) {
    pw::init();

    let mainloop = match pw::main_loop::MainLoopRc::new(None) {
        Ok(mainloop) => mainloop,
        Err(error) => {
            let _ = ready.send(Err(CaptureError::ServerUnavailable(error.to_string())));
            return;
        }
    };

    let context = match pw::context::ContextRc::new(&mainloop, None) {
        Ok(context) => context,
        Err(error) => {
            let _ = ready.send(Err(CaptureError::ServerUnavailable(error.to_string())));
            return;
        }
    };

    let core = match context.connect_rc(None) {
        Ok(core) => core,
        Err(error) => {
            let _ = ready.send(Err(CaptureError::ServerUnavailable(error.to_string())));
            return;
        }
    };

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        // Capture the sink monitor instead of a microphone (the property
        // `pw-record` received through `-P '{"stream.capture.sink":true}'`).
        *pw::keys::STREAM_CAPTURE_SINK => "true",
    };

    if let Some(target) = &config.device {
        // `TARGET_OBJECT` lives behind the crate's `v0_3_44` feature; the
        // property key itself is stable.
        props.insert("target.object", target.clone());
    }

    let stream = match pw::stream::StreamBox::new(&core, STREAM_NAME, props) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(CaptureError::ServerUnavailable(error.to_string())));
            return;
        }
    };

    let data = UserData {
        format: spa::param::audio::AudioInfoRaw::new(),
        assembler: ChunkAssembler::new(config.channels, config.block_frames),
        formats: FormatState::new(ready.clone(), queue.clone()),
        scratch: Vec::new(),
    };

    let listener = stream
        .add_local_listener_with_user_data(data)
        .param_changed(|_, user_data, id, param| {
            let Some(param) = param else {
                return;
            };

            if id != ParamType::Format.as_raw() {
                return;
            }

            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                user_data
                    .formats
                    .reject("PipeWire sent an unparsable audio format".to_string());
                return;
            };

            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                user_data.formats.reject(format!(
                    "PipeWire negotiated a non-raw audio format ({media_type:?}/{media_subtype:?})"
                ));
                return;
            }

            if user_data.format.parse(param).is_err() {
                user_data
                    .formats
                    .reject("PipeWire sent an unparsable audio format".to_string());
                return;
            }

            let format = user_data.format.format();
            let rate = user_data.format.rate();
            let channels = user_data.format.channels();

            if user_data.formats.set_format(format, rate, channels) {
                user_data
                    .assembler
                    .set_channels(u16::try_from(channels).unwrap_or(1));
            }
        })
        .process(|stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();

            if datas.is_empty() {
                return;
            }

            let data = &mut datas[0];
            let size = data.chunk().size() as usize;
            let Some(bytes) = data.data() else {
                return;
            };

            let Some(negotiated) = user_data.formats.negotiated() else {
                // No accepted format yet: park the buffer instead of decoding
                // it with a guessed sample layout (R-2).
                return;
            };

            let bytes = &bytes[..size.min(bytes.len())];

            let channels = u16::try_from(negotiated.channels).unwrap_or(1);

            let UserData {
                assembler,
                formats,
                scratch,
                ..
            } = user_data;

            if !formats.decode_into(bytes, scratch) {
                return;
            }

            if assembler.channels() != usize::from(channels) {
                assembler.set_channels(channels);
            }

            assembler.push(scratch, |chunk| formats.queue.push(chunk));
        })
        .register();

    let _listener = match listener {
        Ok(listener) => listener,
        Err(error) => {
            let _ = ready.send(Err(CaptureError::ServerUnavailable(error.to_string())));
            return;
        }
    };

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(config.sample_rate);
    audio_info.set_channels(u32::from(config.channels));

    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = match spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    ) {
        Ok(serialized) => serialized.0.into_inner(),
        Err(error) => {
            let _ = ready.send(Err(CaptureError::ConfigUnsupported(error.to_string())));
            return;
        }
    };
    let Some(pod) = Pod::from_bytes(&values) else {
        let _ = ready.send(Err(CaptureError::ConfigUnsupported(
            "could not build the PipeWire format pod".to_string(),
        )));
        return;
    };
    let mut params = [pod];

    if let Err(error) = stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    ) {
        let _ = ready.send(Err(CaptureError::ServerUnavailable(error.to_string())));
        return;
    }

    let _quit = quit_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    // Readiness is sent from `param_changed`: the format negotiation happens
    // while the loop runs, so this thread reports back from there (R-3).
    mainloop.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use spa::param::audio::AudioFormat;

    /// Decode through the buffered path (the one the RT callback uses).
    fn decode(formats: &FormatState, bytes: &[u8]) -> Option<Vec<f32>> {
        let mut out = Vec::new();
        formats.decode_into(bytes, &mut out).then_some(out)
    }

    #[test]
    fn accepted_formats_are_interleaved_f32_or_s16_at_a_usable_rate() {
        assert_eq!(
            negotiate(AudioFormat::F32LE, 48_000, 2),
            Ok(NegotiatedFormat {
                sample: PcmFormat::F32Le,
                rate: 48_000,
                channels: 2,
            })
        );
        assert_eq!(
            negotiate(AudioFormat::S16LE, 48_000, 2),
            Ok(NegotiatedFormat {
                sample: PcmFormat::S16Le,
                rate: 48_000,
                channels: 2,
            })
        );
        // Mono and non-48 kHz monitors are still decodable: the caller builds
        // the analyser from the negotiated rate.
        assert_eq!(
            negotiate(AudioFormat::S16LE, 44_100, 1),
            Ok(NegotiatedFormat {
                sample: PcmFormat::S16Le,
                rate: 44_100,
                channels: 1,
            })
        );
        assert!(negotiate(AudioFormat::F32LE, 96_000, 6).is_ok());
        assert!(negotiate(AudioFormat::F32LE, MIN_SUPPORTED_RATE, 1).is_ok());
        assert!(negotiate(AudioFormat::F32LE, MAX_SUPPORTED_RATE, 8).is_ok());
    }

    #[test]
    fn rejected_formats_explain_why() {
        let planar = negotiate(AudioFormat::F32P, 48_000, 2).expect_err("planar is undecodable");
        assert!(planar.contains("F32P"), "{planar}");

        let s24 = negotiate(AudioFormat::S24LE, 48_000, 2).expect_err("24-bit is undecodable");
        assert!(s24.contains("S24LE"), "{s24}");

        let unknown = negotiate(AudioFormat::Unknown, 48_000, 2).expect_err("unknown rejected");
        assert!(unknown.contains("Unknown"), "{unknown}");

        let rate = negotiate(AudioFormat::F32LE, 0, 2).expect_err("0 Hz rejected");
        assert!(rate.contains("0 Hz"), "{rate}");

        let fast = negotiate(AudioFormat::F32LE, 384_000, 2).expect_err("384 kHz rejected");
        assert!(fast.contains("384000"), "{fast}");

        let no_channels = negotiate(AudioFormat::F32LE, 48_000, 0).expect_err("0 ch rejected");
        assert!(no_channels.contains("channel count 0"), "{no_channels}");

        let many = negotiate(AudioFormat::F32LE, 48_000, 9).expect_err("9 ch rejected");
        assert!(many.contains("channel count 9"), "{many}");
    }

    #[test]
    fn f32_buffers_decode_verbatim() {
        let mut bytes = Vec::new();
        for sample in [0.0f32, 0.5, -1.0] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        assert_eq!(
            decode_interleaved(&bytes, PcmFormat::F32Le),
            vec![0.0, 0.5, -1.0]
        );
        // A trailing partial sample is ignored.
        assert_eq!(
            decode_interleaved(&[0, 0, 128], PcmFormat::F32Le),
            Vec::<f32>::new()
        );
    }

    #[test]
    fn s16_buffers_are_converted_to_full_scale_f32() {
        let mut bytes = Vec::new();
        for sample in [0i16, 16_384, -32_768, 32_767] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        assert_eq!(
            decode_interleaved(&bytes, PcmFormat::S16Le),
            vec![0.0, 0.5, -1.0, 32_767.0 / 32_768.0]
        );
        // Odd byte counts keep whole samples only.
        assert_eq!(
            decode_interleaved(&[1], PcmFormat::S16Le),
            Vec::<f32>::new()
        );
        assert_eq!(decode_interleaved(&[0, 0, 9], PcmFormat::S16Le).len(), 1);
        assert_eq!(PcmFormat::S16Le.bytes_per_sample(), 2);
        assert_eq!(PcmFormat::F32Le.bytes_per_sample(), 4);
    }

    #[test]
    fn decoding_reuses_its_scratch_buffer_without_reallocating() {
        let queue = ChunkQueue::new(1);
        let (ready_tx, _ready_rx) = sync_channel(1);
        let mut formats = FormatState::new(ready_tx, queue.clone());
        assert!(formats.set_format(AudioFormat::F32LE, 48_000, 2));

        let mut scratch = Vec::new();
        let mut bytes = Vec::new();
        for sample in [0.0f32, 0.5, -1.0] {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        assert!(formats.decode_into(&bytes, &mut scratch));
        assert_eq!(scratch, vec![0.0, 0.5, -1.0]);

        // A server-sized buffer allocates once...
        let mut block = Vec::new();
        for sample in [0.25f32; 96] {
            block.extend_from_slice(&sample.to_le_bytes());
        }

        assert!(formats.decode_into(&block, &mut scratch));
        let capacity = scratch.capacity();
        assert!(capacity >= 96);

        // ...and every following `process` callback reuses the allocation: the
        // loop thread must not touch the heap in steady state (R-22).
        for _ in 0..16 {
            assert!(formats.decode_into(&block[..192], &mut scratch));
        }

        assert_eq!(
            scratch.capacity(),
            capacity,
            "steady-state decoding must not reallocate"
        );
        assert_eq!(scratch.len(), 48);

        // A bigger buffer (a format/buffer-size change) grows once; the bigger
        // allocation is then kept for smaller buffers.
        let mut big = Vec::new();
        for sample in [0i16; 64] {
            big.extend_from_slice(&sample.to_le_bytes());
        }

        assert!(formats.set_format(AudioFormat::S16LE, 48_000, 2));
        assert!(formats.decode_into(&big, &mut scratch));
        assert_eq!(scratch.len(), 64);
        let grown = scratch.capacity();
        assert!(grown >= 64);

        assert!(formats.decode_into(&bytes[..4], &mut scratch));
        assert_eq!(scratch.capacity(), grown, "the grown buffer is kept");
        // The active format is S16 by now: four bytes are two samples.
        assert_eq!(scratch.len(), 2);
    }

    #[test]
    fn decoding_matches_the_previous_per_sample_mapping() {
        let mut out = Vec::new();

        let mut f32_bytes = Vec::new();
        for sample in [0.0f32, 0.5, -1.0] {
            f32_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        decode_interleaved_into(&f32_bytes, PcmFormat::F32Le, &mut out);
        assert_eq!(out, vec![0.0, 0.5, -1.0]);

        let mut s16_bytes = Vec::new();
        for sample in [0i16, 16_384, -32_768, 32_767] {
            s16_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        decode_interleaved_into(&s16_bytes, PcmFormat::S16Le, &mut out);
        assert_eq!(out, vec![0.0, 0.5, -1.0, 32_767.0 / 32_768.0]);

        // Partial trailing samples are still ignored, and a shorter buffer
        // truncates the previous result rather than leaving stale samples.
        decode_interleaved_into(&[0, 0, 9], PcmFormat::S16Le, &mut out);
        assert_eq!(out.len(), 1);
        decode_interleaved_into(&[0, 0, 128], PcmFormat::F32Le, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn readiness_is_reported_only_after_a_format_is_accepted() {
        let queue = ChunkQueue::new(1);
        let (ready_tx, ready_rx) = sync_channel(1);
        let mut formats = FormatState::new(ready_tx, queue.clone());

        // Nothing negotiated yet: no readiness, and buffers are parked.
        assert!(formats.negotiated().is_none());
        assert!(decode(&formats, &1.0f32.to_le_bytes()).is_none());
        assert!(
            ready_rx.try_recv().is_err(),
            "start must not report ready before the server negotiated a format"
        );

        assert!(formats.set_format(AudioFormat::F32LE, 48_000, 2));
        assert_eq!(
            ready_rx.try_recv().expect("ready after negotiation"),
            Ok(NegotiatedFormat {
                sample: PcmFormat::F32Le,
                rate: 48_000,
                channels: 2,
            })
        );
        assert_eq!(
            decode(&formats, &1.0f32.to_le_bytes()),
            Some(vec![1.0]),
            "buffers decode once the format is accepted"
        );

        // Ready stays a single report.
        assert!(formats.set_format(AudioFormat::F32LE, 44_100, 2));
        assert!(ready_rx.try_recv().is_err());
        assert_eq!(formats.negotiated().expect("renegotiated").rate, 44_100);
    }

    #[test]
    fn a_rejected_format_parks_the_stream_and_latches_its_error() {
        let queue = ChunkQueue::new(1);
        let (ready_tx, ready_rx) = sync_channel(1);
        let mut formats = FormatState::new(ready_tx, queue.clone());

        assert!(!formats.set_format(AudioFormat::S24LE, 48_000, 2));
        assert!(
            decode(&formats, &[0u8; 8]).is_none(),
            "an unsupported format must never be decoded as f32"
        );
        match ready_rx.try_recv().expect("readiness reports the failure") {
            Err(CaptureError::ConfigUnsupported(message)) => {
                assert!(message.contains("S24LE"), "{message}");
            }
            other => panic!("unexpected readiness: {other:?}"),
        }

        // The error is latched, not one-shot: the engine sees it every frame.
        let first = queue.error().expect("latched error");
        let second = queue.error().expect("still latched");
        assert_eq!(first, second);
        assert!(first.contains("S24LE"), "{first}");

        // A later accepted negotiation clears the latch (and becomes the
        // reported format).
        assert!(formats.set_format(AudioFormat::S16LE, 44_100, 1));
        assert!(queue.error().is_none());
        assert_eq!(
            formats.negotiated(),
            Some(NegotiatedFormat {
                sample: PcmFormat::S16Le,
                rate: 44_100,
                channels: 1,
            })
        );
    }

    /// Live smoke test: starts the monitor when a server is reachable and
    /// prints a skip line otherwise (the Docker test image has no daemon).
    #[test]
    fn monitor_smoke_test_or_skip_without_a_server() {
        match PipeWireMonitor::start(CaptureConfig::default()) {
            Ok(mut monitor) => {
                let chunk = monitor.next_chunk(Duration::from_millis(500));

                println!(
                    "pipewire monitor: {:?} {} Hz / {} ch, first chunk: {:?}",
                    monitor.negotiated_format().sample,
                    monitor.sample_rate(),
                    monitor.channels(),
                    chunk.map(|chunk| chunk.map(|chunk| chunk.len()))
                );
                monitor.stop();
            }
            Err(error) => {
                println!("skipping PipeWire monitor smoke test (no server): {error}");
            }
        }
    }
}
