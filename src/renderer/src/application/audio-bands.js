// AGC-smoothed bass/mid/treble bands, ported from the Point10 audio engine
// (musicvis src/audio/AudioEngine.js + src/plane9/P9Audio.js semantics).
//
// Raw band values are linear FFT magnitudes normalized by a time-domain AGC.
// The exposed bass/mid/treble features are then mapped onto a -60..0 dB
// perceptual scale so treble reads comparably to bass; the payload's bands,
// lanes and cells (consumed by Point10 scenes) stay linear. The 16 log-spaced
// band cells follow the P10 band grid (bass = cells 0-4, mid = 5-9, treble =
// cells 10-15, starting at 2 kHz) low-passed with the lane-1 damping hint
// (tau ~0.05s).

const BAND_EDGES = [
  30, 50, 80, 130, 200, 300, 450, 700, 1000, 1500, 2000, 3300, 5000, 7500,
  11000, 16000, 20000,
];

const BAND_CELLS = [
  [0, 4],
  [5, 9],
  [10, 15],
];

// WebAudio's default FFT window reads a full-scale bin at ~0.42; undo only
// that window loss (P10 AudioEngine FFT_GAIN).
const FFT_GAIN = 1 / 0.42;

// P10 damping lanes (band(channel, damping, bandnr)): lane 0 is the raw band,
// lanes 1..3 are low-passed with tau 0.05s / 0.12s / 0.25s.
const DAMPING_TAU = [0.05, 0.12, 0.25];

const BAND_FEATURES = ["bass", "mid", "treble"];

const clamp01 = (x) => (x > 1 ? 1 : x > 0 ? x : 0);

// The exposed features use a perceptual dB scale (-60 dB .. 0 dB -> 0..1) so
// musically-present treble is not dwarfed by bass under a pink-ish spectrum.
// Only bass/mid/treble go through this; the raw linear grid/lanes stay as-is
// for the Point10 scenes.
const FEATURE_DB_FLOOR = -60;

function featureValue(magnitude) {
  if (!(magnitude > 0)) {
    return 0;
  }

  const db = 20 * Math.log10(magnitude);

  return clamp01((db - FEATURE_DB_FLOOR) / -FEATURE_DB_FLOOR);
}

function isBandFeature(feature) {
  return BAND_FEATURES.indexOf(feature) > -1;
}

class AudioBands {
  constructor() {
    this.maxLevel = 0.1;
    this.freqMag = new Float32Array(0);
    this.grid = new Float32Array(16);
    this.bands = new Float32Array(3);
    this.smoothed = new Float32Array(3);
    // lane 0 = raw band, lanes 1..3 = the P10 damping lanes (index d * 3 + b)
    this.lanes = new Float32Array(12);
    this.beatEnv = 0;
    this.bassSmoothed = 0;
    this.wave = new Float32Array(1024);
    this.byteFrequencyData = new Uint8Array(0);
    this.cells = null;
    this.cellKey = "";
    this.lastT = 0;
  }

  // plane9 CMusicManager AGC: the time-domain chunk peak attacks the session
  // max level instantly; otherwise the max level decays in three linear tiers
  // (0.0765625 / 0.00765625 / 0.000765625 per second above 0.01 / 0.001 /
  // 0.0001) with a 1e-4 floor. gain = 1/maxLevel is applied to the linear FFT
  // magnitudes, so a loud spike stops re-normalizing everything and quiet
  // material gets boosted. The peak is the TIME-DOMAIN level, not the FFT
  // peak - measuring it from FFT bins pegs every bass read to ~1.0.
  _agc(dt, byteTimeDomainData) {
    let peak = 0;

    if (byteTimeDomainData) {
      for (let i = 0; i < byteTimeDomainData.length; i++) {
        const v = Math.abs(byteTimeDomainData[i] - 128) / 128;
        if (v > peak) {
          peak = v;
        }
      }
    }

    const ml = this.maxLevel;

    if (peak >= ml) {
      this.maxLevel = peak;
    } else {
      const perSec =
        ml > 0.01
          ? 0.0765625
          : ml > 0.001
            ? 0.00765625
            : ml > 0.0001
              ? 0.000765625
              : 0;
      this.maxLevel = Math.max(0.0001, ml - perSec * dt);
    }

    return 1 / this.maxLevel;
  }

  _binRange(loHz, hiHz, n, sampleRate) {
    const hz = (i) => (i * sampleRate) / 2 / n;
    let a = 0;
    let b = n - 1;

    while (a < n - 1 && hz(a) < loHz) {
      a++;
    }
    while (b > 0 && hz(b) > hiHz) {
      b--;
    }

    return [a, Math.min(b, n - 1)];
  }

  _bandCells(n, sampleRate) {
    const key = n + ":" + sampleRate;

    if (this.cellKey !== key) {
      this.cellKey = key;
      this.cells = [];

      for (let c = 0; c < 16; c++) {
        this.cells.push(
          this._binRange(BAND_EDGES[c], BAND_EDGES[c + 1], n, sampleRate),
        );
      }
    }

    return this.cells;
  }

  _fillWave(byteTimeDomainData) {
    if (!byteTimeDomainData) {
      return;
    }

    const n = Math.min(byteTimeDomainData.length, this.wave.length);
    const offset = byteTimeDomainData.length - n;

    for (let i = 0; i < n; i++) {
      this.wave[i] = (byteTimeDomainData[offset + i] / 255) * 2 - 1;
    }
  }

  // The payload the Point10 adapter consumes: the dB-mapped bass/mid/treble
  // features, the raw 3-band values, the four damping lanes, the 16-cell grid,
  // the beat envelope and the normalized time-domain wave. `raw` is the same
  // signal with the AGC gain divided back out, for modules that opt out of
  // automatic gain control.
  _payload(gain, sampleRate) {
    const bands = [];
    const rawBands = [];
    const lanes = [];
    const rawLanes = [];
    const cells = [];
    const rawCells = [];

    for (let b = 0; b < 3; b++) {
      bands.push(clamp01(this.bands[b]));
      rawBands.push(clamp01(this.bands[b] / gain));
    }

    for (let i = 0; i < 12; i++) {
      lanes.push(clamp01(this.lanes[i]));
      rawLanes.push(clamp01(this.lanes[i] / gain));
    }

    for (let c = 0; c < 16; c++) {
      cells.push(clamp01(this.grid[c]));
      rawCells.push(clamp01(this.grid[c] / gain));
    }

    return {
      bass: featureValue(this.smoothed[0]),
      mid: featureValue(this.smoothed[1]),
      treble: featureValue(this.smoothed[2]),
      bands,
      lanes,
      cells,
      beat: clamp01(this.beatEnv),
      wave: this.wave,
      freqMag: this.freqMag,
      byteFrequencyData: this.byteFrequencyData,
      raw: {
        bands: rawBands,
        lanes: rawLanes,
        cells: rawCells,
      },
      sampleRate,
    };
  }

  update({
    floatFrequencyData,
    byteTimeDomainData,
    byteFrequencyData,
    sampleRate = 44100,
  }) {
    const now = performance.now();
    const dt = Math.min(0.1, (now - (this.lastT || now)) / 1000);
    this.lastT = now;

    this._fillWave(byteTimeDomainData);
    this.byteFrequencyData = byteFrequencyData || new Uint8Array(0);

    if (!floatFrequencyData || !floatFrequencyData.length) {
      const k = Math.exp(-dt * 8);

      for (let i = 0; i < 16; i++) {
        this.grid[i] *= k;
      }
      for (let i = 0; i < 12; i++) {
        this.lanes[i] *= k;
      }
      for (let i = 0; i < 3; i++) {
        this.bands[i] = 0;
        this.smoothed[i] = 0;
      }
      for (let i = 0; i < this.freqMag.length; i++) {
        this.freqMag[i] = 0;
      }

      this.beatEnv *= Math.exp(-dt * 3.5);
      this.bassSmoothed *= k;

      return this._payload(1, sampleRate);
    }

    const n = floatFrequencyData.length;

    if (this.freqMag.length !== n) {
      this.freqMag = new Float32Array(n);
    }

    const gain = this._agc(dt, byteTimeDomainData);

    // getFloatFrequencyData is the same FFT in true dB per bin; convert each
    // bin to a linear magnitude (the byte API is dB-scaled and unusable here).
    for (let i = 0; i < n; i++) {
      this.freqMag[i] =
        Math.pow(10, floatFrequencyData[i] / 20) * FFT_GAIN * gain;
    }

    const cells = this._bandCells(n, sampleRate);

    for (let c = 0; c < 16; c++) {
      const [a, b] = cells[c];
      let sum = 0;

      for (let i = a; i <= b; i++) {
        sum += this.freqMag[i];
      }

      const target = sum / (b - a + 1);
      const g = this.grid[c];

      if (target >= g) {
        this.grid[c] = g + (target - g) * Math.min(1, dt * 30);
      } else {
        this.grid[c] = g * Math.exp(-dt * (6 + c));
      }
    }

    for (let b = 0; b < 3; b++) {
      const [lo, hi] = BAND_CELLS[b];
      let sum = 0;

      for (let c = lo; c <= hi; c++) {
        sum += this.grid[c];
      }

      this.bands[b] = sum / (hi - lo + 1);
    }

    // damping lanes: lane 0 is the raw band, 1..3 low-pass the same band
    for (let b = 0; b < 3; b++) {
      this.lanes[b] = this.bands[b];
    }

    for (let d = 1; d <= 3; d++) {
      const k = Math.exp(-dt / DAMPING_TAU[d - 1]);

      for (let b = 0; b < 3; b++) {
        const i = d * 3 + b;
        this.lanes[i] += (this.bands[b] - this.lanes[i]) * (1 - k);
      }
    }

    for (let b = 0; b < 3; b++) {
      this.smoothed[b] = this.lanes[3 + b];
    }

    // P10 beat envelope: a refractory-gated bass transient (re-arms only once
    // the envelope has decayed below 0.2, so a busy mix pulses per kick
    // instead of pinning the envelope near 1).
    const bass = (this.grid[0] + this.grid[1]) * 0.5;

    this.beatEnv *= Math.exp(-dt * 3.5);

    if (this.beatEnv < 0.2 && bass > this.bassSmoothed * 1.25 && bass > 0.03) {
      this.beatEnv = Math.min(
        1,
        Math.max(this.beatEnv, (bass - this.bassSmoothed) * 12),
      );
    }

    this.bassSmoothed += (bass - this.bassSmoothed) * Math.min(1, dt * 4);

    return this._payload(gain, sampleRate);
  }
}

export { AudioBands, BAND_FEATURES, BAND_EDGES, BAND_CELLS, isBandFeature };
