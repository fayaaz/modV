//! Native PipeWire monitor-source capture (WS-9, Linux only).
//!
//! Replaces the `pw-record` child process in
//! `src/main/system-audio-pipewire.js`: a `pipewire` crate stream connects to
//! the default server, captures the **default sink's monitor** (the same
//! `stream.capture.sink = true` mode `pw-record` was given), and delivers the
//! same 48 kHz / 2 ch / f32 / 480-frame chunks through the same
//! [`super::capture::ChunkQueue`] as [`super::capture::AudioCapture`].
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
//! # Errors
//!
//! `start` returns [`CaptureError::ServerUnavailable`] when no PipeWire daemon
//! answers within [`START_TIMEOUT`]; callers (and the smoke test) treat that as
//! "no system audio available on this host" rather than a fatal error. A
//! format the server negotiates that is not the requested 48 kHz stereo f32 is
//! reported as a stream error and leaves capture silent.

use std::sync::mpsc::{RecvTimeoutError, sync_channel};
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

/// How long [`PipeWireMonitor::start`] waits for the loop thread to connect.
pub const START_TIMEOUT: Duration = Duration::from_secs(5);
/// Stream name shown in `pw-top` / `wpctl`.
pub const STREAM_NAME: &str = "modv-system-audio";

/// User data handed to the PipeWire callbacks.
struct UserData {
    format: spa::param::audio::AudioInfoRaw,
    requested: CaptureConfig,
    assembler: ChunkAssembler,
    queue: ChunkQueue,
}

/// Whether a negotiated stream format is the 48 kHz stereo f32 the band
/// analyser assumes.
///
/// Kept pure so negotiation can be tested without a server.
pub fn format_is_supported(
    format: spa::param::audio::AudioFormat,
    rate: u32,
    channels: u32,
    requested_sample_rate: u32,
    requested_channels: u16,
) -> bool {
    format == spa::param::audio::AudioFormat::F32LE
        && rate == requested_sample_rate
        && channels == u32::from(requested_channels)
}

/// A running PipeWire monitor capture.
pub struct PipeWireMonitor {
    quit: Option<pw::channel::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    queue: ChunkQueue,
    sample_rate: u32,
    channels: u16,
}

impl PipeWireMonitor {
    /// Connect to the default PipeWire server and start capturing the default
    /// sink's monitor.
    ///
    /// `config.device` is an optional target sink `node.name` / object id
    /// (`target.object`); `None` follows the default sink.
    pub fn start(config: CaptureConfig) -> Result<Self, CaptureError> {
        let queue = ChunkQueue::new(super::capture::QUEUE_CAPACITY);
        let (quit_tx, quit_rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = sync_channel::<Result<(), CaptureError>>(1);
        let thread_queue = queue.clone();
        let thread_config = config.clone();

        let handle = thread::Builder::new()
            .name("modv-pw-monitor".to_string())
            .spawn(move || monitor_thread(thread_config, thread_queue, ready_tx, quit_rx))
            .map_err(|error| CaptureError::ServerUnavailable(error.to_string()))?;

        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                quit: Some(quit_tx),
                handle: Some(handle),
                queue,
                sample_rate: config.sample_rate,
                channels: config.channels,
            }),
            Ok(Err(error)) => {
                let _ = quit_tx.send(());
                let _ = handle.join();
                Err(error)
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = quit_tx.send(());
                let _ = handle.join();
                Err(CaptureError::ServerUnavailable(
                    "timed out waiting for the PipeWire stream".to_string(),
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = handle.join();
                Err(CaptureError::ServerUnavailable(
                    "PipeWire monitor thread exited during startup".to_string(),
                ))
            }
        }
    }

    /// The negotiated sample rate in Hz (the requested rate on success).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The negotiated channel count.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Wait up to `timeout` for the next interleaved f32 chunk.
    ///
    /// Returns `Ok(None)` on timeout. A format the server refused (or a stream
    /// error) is returned as [`CaptureError::Stream`].
    pub fn next_chunk(&mut self, timeout: Duration) -> Result<Option<Vec<f32>>, CaptureError> {
        if let Some(error) = self.queue.take_error() {
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
    ready: std::sync::mpsc::SyncSender<Result<(), CaptureError>>,
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
        requested: config.clone(),
        assembler: ChunkAssembler::new(config.channels, config.block_frames),
        queue: queue.clone(),
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
                return;
            };

            if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                user_data
                    .queue
                    .set_error("PipeWire negotiated a non-raw audio format".to_string());
                return;
            }

            if user_data.format.parse(param).is_err() {
                user_data
                    .queue
                    .set_error("PipeWire sent an unparsable audio format".to_string());
                return;
            }

            let format = user_data.format.format();
            let rate = user_data.format.rate();
            let channels = user_data.format.channels();

            if !format_is_supported(
                format,
                rate,
                channels,
                user_data.requested.sample_rate,
                user_data.requested.channels,
            ) {
                user_data.queue.set_error(format!(
                    "unsupported PipeWire format {format:?} at {rate} Hz / {channels} ch"
                ));
            } else {
                user_data.assembler.set_channels(channels as u16);
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
            let byte_len = size.min(bytes.len()) & !3;
            let samples: Vec<f32> = bytes[..byte_len]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|sample| f32::from_le_bytes(*sample))
                .collect();

            let channels = user_data.format.channels();

            if channels > 0 && channels as usize != user_data.assembler.channels() {
                user_data.assembler.set_channels(channels as u16);
            }

            let UserData {
                assembler, queue, ..
            } = user_data;

            assembler.push(&samples, |chunk| queue.push(chunk));
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

    let _ = ready.send(Ok(()));
    mainloop.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use spa::param::audio::AudioFormat;

    #[test]
    fn supported_format_requires_f32_at_the_requested_rate_and_channels() {
        assert!(format_is_supported(
            AudioFormat::F32LE,
            48_000,
            2,
            48_000,
            2
        ));
        assert!(!format_is_supported(
            AudioFormat::F32LE,
            44_100,
            2,
            48_000,
            2
        ));
        assert!(!format_is_supported(
            AudioFormat::F32LE,
            48_000,
            1,
            48_000,
            2
        ));
        assert!(!format_is_supported(
            AudioFormat::S16LE,
            48_000,
            2,
            48_000,
            2
        ));
        assert!(!format_is_supported(
            AudioFormat::F32LE,
            96_000,
            2,
            48_000,
            2
        ));
    }

    /// Live smoke test: starts the monitor when a server is reachable and
    /// prints a skip line otherwise (the Docker test image has no daemon).
    #[test]
    fn monitor_smoke_test_or_skip_without_a_server() {
        match PipeWireMonitor::start(CaptureConfig::default()) {
            Ok(mut monitor) => {
                let chunk = monitor.next_chunk(Duration::from_millis(500));

                println!(
                    "pipewire monitor: {} Hz / {} ch, first chunk: {:?}",
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
