//! Capture → analysis driver (WS-9).
//!
//! [`AudioFeed`] owns one [`AudioAnalyser`] and, optionally, a live capture
//! source. Each [`AudioFeed::frame`] drains whatever chunks the source has
//! ready, downmixes them to mono, pushes them into the analyser and returns the
//! frame's [`AudioSnapshot`]. The engine wires this straight into the scene
//! render loop, replacing the `postMessage` feature transport.
//!
//! Sources:
//!
//! * [`AudioFeed::new`] is a silent feed — feed it with
//!   [`AudioFeed::push_mono`] / [`AudioFeed::push_interleaved`] (used by tests
//!   and by the engine's offline/oscilloscope fixtures).
//! * [`AudioFeed::with_capture`] wraps an input device
//!   ([`super::capture::AudioCapture`]).
//! * [`AudioFeed::with_monitor`] wraps the native PipeWire monitor
//!   ([`super::pipewire::PipeWireMonitor`], Linux only).
//!
//! Capture errors never abort a frame: they are stored and surfaced through
//! [`AudioFeed::take_error`] so the engine can log/restart the capture without
//! dropping the visualiser.

use std::time::Duration;

use super::analyser::{AudioAnalyser, AudioSnapshot};
use super::capture::{AudioCapture, CaptureConfig, CaptureError, downmix_mono};

#[cfg(target_os = "linux")]
use super::pipewire::PipeWireMonitor;

/// Where a feed's PCM comes from.
enum Source {
    /// cpal input-device capture.
    Capture(AudioCapture),
    /// Native PipeWire monitor capture (Linux).
    #[cfg(target_os = "linux")]
    Monitor(PipeWireMonitor),
}

/// An analyser plus an optional live capture source.
pub struct AudioFeed {
    analyser: AudioAnalyser,
    source: Option<Source>,
    sample_rate: f32,
    last_error: Option<CaptureError>,
}

impl AudioFeed {
    /// Create a silent feed for an analyser at `sample_rate` Hz.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            analyser: AudioAnalyser::new(sample_rate),
            source: None,
            sample_rate,
            last_error: None,
        }
    }

    /// Start capturing from an input device and analyse at the device's
    /// negotiated sample rate.
    pub fn with_capture(config: CaptureConfig) -> Result<Self, CaptureError> {
        let capture = AudioCapture::start(config)?;
        let sample_rate = capture.sample_rate() as f32;

        Ok(Self {
            analyser: AudioAnalyser::new(sample_rate),
            source: Some(Source::Capture(capture)),
            sample_rate,
            last_error: None,
        })
    }

    /// Start capturing the system audio monitor and analyse at the
    /// **negotiated** PipeWire stream rate.
    ///
    /// [`PipeWireMonitor::start`] returns only after the server negotiated a
    /// decodable format, so the analyser is built for the rate the monitor
    /// actually emits (44.1 kHz / S16 monitors are converted and analysed at
    /// their own rate) instead of assuming the requested 48 kHz.
    #[cfg(target_os = "linux")]
    pub fn with_monitor(config: CaptureConfig) -> Result<Self, CaptureError> {
        let monitor = PipeWireMonitor::start(config)?;
        let sample_rate = monitor.sample_rate() as f32;

        Ok(Self {
            analyser: AudioAnalyser::new(sample_rate),
            source: Some(Source::Monitor(monitor)),
            sample_rate,
            last_error: None,
        })
    }

    /// The analysis sample rate in Hz.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// The analyser owned by this feed.
    pub fn analyser(&self) -> &AudioAnalyser {
        &self.analyser
    }

    /// Mutable analyser access for smoothing-id bookkeeping.
    pub fn analyser_mut(&mut self) -> &mut AudioAnalyser {
        &mut self.analyser
    }

    /// Whether a live capture source is attached.
    pub fn has_source(&self) -> bool {
        self.source.is_some()
    }

    /// The last capture error that was observed while pumping, if any.
    pub fn take_error(&mut self) -> Option<CaptureError> {
        self.last_error.take()
    }

    /// Push mono PCM at the feed's sample rate (synthetic/no-device input).
    pub fn push_mono(&mut self, pcm: &[f32]) {
        self.analyser.push_mono(pcm);
    }

    /// Push interleaved PCM from `channels` and downmix it to mono.
    pub fn push_interleaved(&mut self, interleaved: &[f32], channels: u16) {
        let mono = downmix_mono(interleaved, channels);

        self.analyser.push_mono(&mono);
    }

    /// Drain every chunk currently queued by the capture source.
    ///
    /// Returns the number of chunks consumed; capture errors are stored for
    /// [`Self::take_error`] and terminate the drain for that frame.
    pub fn pump(&mut self, timeout: Duration) -> usize {
        let Some(source) = self.source.as_mut() else {
            return 0;
        };

        let mut consumed = 0;

        loop {
            // Only the first poll may wait; draining the backlog is immediate
            // so a frame never stalls on a backlog-free stream.
            let wait = if consumed == 0 {
                timeout
            } else {
                Duration::ZERO
            };

            let result = match source {
                Source::Capture(capture) => capture.next_chunk(wait),
                #[cfg(target_os = "linux")]
                Source::Monitor(monitor) => monitor.next_chunk(wait),
            };

            match result {
                Ok(Some(chunk)) => {
                    let channels = match source {
                        Source::Capture(capture) => capture.channels(),
                        #[cfg(target_os = "linux")]
                        Source::Monitor(monitor) => monitor.channels(),
                    };

                    let mono = downmix_mono(&chunk, channels);
                    self.analyser.push_mono(&mono);
                    consumed += 1;
                }
                Ok(None) => break,
                Err(error) => {
                    self.last_error = Some(error);
                    break;
                }
            }
        }

        consumed
    }

    /// Pump the source (non-blocking), analyse the newest audio and return the
    /// frame's snapshot.
    pub fn frame(&mut self, dt: f32) -> AudioSnapshot {
        self.pump(Duration::ZERO);

        self.analyser.frame(dt)
    }

    /// Stop and release the live source, keeping the analyser state.
    pub fn stop(&mut self) {
        match self.source.take() {
            Some(Source::Capture(mut capture)) => capture.stop(),
            #[cfg(target_os = "linux")]
            Some(Source::Monitor(mut monitor)) => monitor.stop(),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    #[test]
    fn synthetic_feed_analyses_injected_chunks_without_a_device() {
        let mut feed = AudioFeed::new(SAMPLE_RATE);
        let chunk_frames = 800usize;

        for block in 0..120 {
            let start = block * chunk_frames;
            let interleaved: Vec<f32> = (start..start + chunk_frames)
                .flat_map(|i| {
                    let sample =
                        0.5 * (2.0 * std::f32::consts::PI * 60.0 * i as f32 / SAMPLE_RATE).sin();
                    [sample, sample]
                })
                .collect();

            feed.push_interleaved(&interleaved, 2);
            feed.frame(1.0 / 60.0);
        }

        let snapshot = feed.analyser().last_snapshot().cloned().expect("snapshot");

        assert!(snapshot.bands[0] > snapshot.bands[1]);
        assert!(snapshot.bands[0] > snapshot.bands[2]);
    }

    #[test]
    fn silent_feed_stays_silent() {
        let mut feed = AudioFeed::new(SAMPLE_RATE);
        let snapshot = feed.frame(0.0);

        assert_eq!(snapshot.bands, [0.0; 3]);
        assert!(!feed.has_source());
        assert_eq!(feed.pump(Duration::ZERO), 0);
        assert!(feed.take_error().is_none());
    }

    #[test]
    fn stop_without_a_source_is_a_no_op() {
        let mut feed = AudioFeed::new(SAMPLE_RATE);

        feed.stop();
        feed.stop();

        assert!(!feed.has_source());
    }
}
