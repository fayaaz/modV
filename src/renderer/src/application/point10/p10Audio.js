// P9Audio-compatible read API over modV's audio payload.
//
// The vendored Point10 scenes construct `new P9Audio(this.audio)` and call
// update(dt) / band(channel, damping, bandnr) / beat() / waveform() /
// spectrum(). In musicvis those reads walk the AudioEngine's raw FFT data;
// here the band math (16 log-spaced cells, damping lanes, AGC, beat envelope)
// already ran on the main thread in AudioBands and the per-module
// responsiveness levers were applied by Point10AudioBridge, so this class is a
// thin reader over the bridge rather than a second implementation.
//
// Values are linear FFT magnitudes: silence reads ~0, a full-scale tone ~1,
// dense music ~0.2-0.5.

export class P9Audio {
  constructor(engine) {
    this.engine = engine;
    this.freq = new Float32Array(1024);
    this.wave = new Float32Array(1024);
    this.grid = new Float32Array(16);
    this.bands = [0, 0, 0];
    this.bass = 0;
    this.beatEnv = 0;
    this.cells = null;
    // [damping * 3 + band]; lane 0 is the raw band, 1..3 are low-passed.
    this._smoo = new Array(12).fill(0);
  }

  get hasAudio() {
    return !!(this.engine && this.engine.analyser);
  }

  update(dt) {
    const e = this.engine;

    if (!this.hasAudio) {
      const k = Math.exp(-dt * 8);

      for (let i = 0; i < 12; i++) {
        this._smoo[i] *= k;
      }
      for (let b = 0; b < 3; b++) {
        this.bands[b] *= k;
      }

      this.beatEnv *= k;
      this.bass *= k;

      return;
    }

    for (let i = 0; i < 12; i++) {
      this._smoo[i] = e.lanes[i];
    }
    for (let b = 0; b < 3; b++) {
      this.bands[b] = e.bands[b];
    }
    for (let c = 0; c < 16; c++) {
      this.grid[c] = e.grid[c];
    }
    for (let i = 0; i < this.wave.length; i++) {
      this.wave[i] = e.wave[i] ?? 0;
    }
    for (let i = 0; i < this.freq.length; i++) {
      this.freq[i] = e.freqMag[i] ?? 0;
    }

    this.beatEnv = e.beatEnv;
    this.bass = e.bands[0];
  }

  band(channel, damping, bandnr, nomusic) {
    if (!this.hasAudio) {
      return num01(nomusic);
    }

    const idx = Math.max(0, Math.min(2, Math.round(bandnr ?? 0)));
    const d = Math.max(0, Math.min(3, Math.round(damping ?? 0)));

    return d > 0 ? this._smoo[d * 3 + idx] ?? 0 : this.bands[idx] ?? 0;
  }

  beat(fallback) {
    if (!this.hasAudio) {
      return num01(fallback);
    }

    return Math.min(1, this.beatEnv);
  }

  waveform(channel, pos, nomusic) {
    if (!this.hasAudio) {
      return num01(nomusic);
    }

    const i = Math.floor(num01(pos) * (this.wave.length - 1));

    return this.wave[i] ?? 0;
  }

  // plane9 reads the spectrum linearly: bin = pos * 399 interpolated (its FFT
  // is decimated by 4, so 400 bins cover ~4.3 kHz). In modV's 2048-FFT
  // analyser that maps to pos * 199.5 + 1, skipping the inflated DC bin.
  spectrum(channel, pos, nomusic) {
    if (!this.hasAudio) {
      return num01(nomusic);
    }

    const p = Math.max(0, Math.min(1, num01(pos)));
    const j = p * 199.5 + 1;
    const a = Math.floor(j);
    const b = Math.min(this.freq.length - 1, a + 1);
    const frac = j - a;
    const va = this.freq[a] ?? 0;
    const vb = this.freq[b] ?? 0;

    return va + (vb - va) * frac;
  }
}

const num01 = (v) => (typeof v === "number" && isFinite(v) ? v : 0);
