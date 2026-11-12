//! Optional real-music check for the engine-owned analyser (WS-9).
//!
//! CI has no audio device and no network, so the fixture is opt-in: point
//! `MODV_AUDIO_FIXTURE` at a raw little-endian `f32` mono 48 kHz file (e.g.
//! `ffmpeg -i https://radio.noiseblock.live/stream -t 12 -ac 1 -ar 48000 \
//!        -f f32le /tmp/opencode/radio.f32`)
//! and run
//!
//! ```text
//! MODV_AUDIO_FIXTURE=/fixtures/radio.f32 \
//!   cargo test -p modv-media --release --test audio_stream_fixture -- --nocapture
//! ```
//!
//! Without the variable (or with a missing/empty file) the test prints a skip
//! line and passes, so `cargo test --workspace` stays hermetic.

use std::fs;

use modv_media::audio::analyser::AudioAnalyser;

const BLOCK_FRAMES: usize = 480;
const SAMPLE_RATE: f32 = 48_000.0;

/// Decode the raw fixture into mono samples. Returns `None` (with a printed
/// skip line) when the fixture is absent or unusable.
fn load_fixture() -> Option<Vec<f32>> {
    let path = match std::env::var("MODV_AUDIO_FIXTURE") {
        Ok(path) => path,
        Err(_) => {
            eprintln!("skipping real-music analyser test: MODV_AUDIO_FIXTURE is not set");
            return None;
        }
    };

    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("skipping real-music analyser test: cannot read {path}: {error}");
            return None;
        }
    };

    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    if samples.len() < BLOCK_FRAMES * 4 {
        eprintln!("skipping real-music analyser test: {path} is too short");
        return None;
    }

    Some(samples)
}

/// Real music must move every band, produce a kick envelope, a non-zero
/// waveform/FFT pair and finite Meyda features. This is the behaviour-parity
/// gate for "music actually drives the visualiser", using a real stream
/// instead of a synthetic tone.
#[test]
fn real_music_drives_bands_beats_and_features() {
    let Some(samples) = load_fixture() else {
        return;
    };

    let mut analyser = AudioAnalyser::new(SAMPLE_RATE);
    let mut frames = 0_usize;
    let mut max_bands = [0.0_f32; 3];
    let mut max_beat = 0.0_f32;
    let mut max_rms = 0.0_f32;
    let mut max_wave = 0.0_f32;
    let mut max_fft = 0.0_f32;
    let mut lane_spread_max = 0.0_f32;
    let mut bpm = None;

    for block in samples.chunks_exact(BLOCK_FRAMES) {
        analyser.push_mono(block);
        let snapshot = analyser.frame(BLOCK_FRAMES as f32 / SAMPLE_RATE);
        frames += 1;

        for (max, value) in max_bands.iter_mut().zip(snapshot.bands) {
            assert!(
                value.is_finite() && (0.0..=1.0).contains(&value),
                "band out of range: {value}"
            );
            *max = max.max(value);
        }

        max_beat = max_beat.max(snapshot.beat);
        max_rms = max_rms.max(snapshot.features.rms);
        max_wave = max_wave.max(
            snapshot
                .wave
                .iter()
                .fold(0.0_f32, |acc, s| acc.max(s.abs())),
        );
        max_fft = max_fft.max(snapshot.freq_mag.iter().copied().fold(0.0_f32, f32::max));
        lane_spread_max = lane_spread_max.max(
            snapshot.lanes.iter().copied().fold(0.0_f32, f32::max)
                - snapshot.lanes.iter().copied().fold(1.0_f32, f32::min),
        );
        if bpm.is_none() {
            bpm = snapshot.bpm;
        }
    }

    assert!(frames >= 10, "only {frames} analysis frames");
    for (name, max) in ["bass", "mid", "treble"].iter().zip(max_bands) {
        assert!(max > 0.05, "{name} never rose above 0.05 (max {max})");
    }
    assert!(max_beat > 0.1, "no kick envelope (max {max_beat})");
    assert!(max_rms > 0.01, "rms stayed silent (max {max_rms})");
    assert!(max_wave > 0.05, "waveform stayed flat (max {max_wave})");
    assert!(max_fft > 0.001, "spectrum stayed flat (max {max_fft})");
    assert!(
        lane_spread_max > 0.01,
        "damping lanes never diverged (spread {lane_spread_max})"
    );

    eprintln!(
        "real music: {frames} frames, bands max {max_bands:?}, beat {max_beat:.3}, \
         rms {max_rms:.3}, lane spread {lane_spread_max:.3}, bpm {bpm:?}"
    );
}
