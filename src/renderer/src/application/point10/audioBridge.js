// Bridge between modV's audio payload (features.p10, computed on the main
// thread by AudioBands) and the Point10 scene API. It applies the per-module
// responsiveness levers (gain, smoothing, per-band gain, AGC on/off) to the
// band signal once per frame, so every vendored scene reads the same
// lever-adjusted values through the P9Audio-compatible shim.

import { BAND_EDGES } from "../audio-bands.js";

const BAND_OF_CELL = (cell) => (cell <= 4 ? 0 : cell <= 9 ? 1 : 2);

const clamp = (x) => (x > 1 ? 1 : x > 0 ? x : 0);

class Point10AudioBridge {
  constructor() {
    this.hasAudio = false;
    this.bands = [0, 0, 0];
    this.lanes = new Array(12).fill(0);
    this.grid = new Float32Array(16);
    this.beatEnv = 0;
    this.wave = new Float32Array(1024);
    this.freqMag = new Float32Array(1024);
    this.sampleRate = 44100;
    // P9Audio engine aliases: the vendored plane9 helper reads the raw engine
    // shape (freqArray/dataArray/freqMagL/freqMagR/ctx). modV's payload is
    // mono, so the stereo arrays share the mono FFT; dataArray is the byte
    // waveform P9Audio converts back to -1..1.
    this.freqArray = this.freqMag;
    this.freqMagL = this.freqMag;
    this.freqMagR = this.freqMag;
    this.dataArray = new Uint8Array(1024);
    this.ctx = { sampleRate: this.sampleRate };
    this.spectrum = new Uint8Array(0);
    this._spectrumKey = "";
    this._binBand = null;
  }

  // The shim's `hasAudio` gate mirrors P9Audio's `engine.analyser` check.
  get analyser() {
    return this.hasAudio ? this : null;
  }

  _binBands(n, sampleRate) {
    const key = n + ":" + sampleRate;

    if (this._spectrumKey !== key) {
      this._spectrumKey = key;
      this._binBand = new Uint8Array(n);

      for (let i = 0; i < n; i++) {
        const hz = (i * sampleRate) / 2 / n;
        let cell = 15;

        for (let c = 0; c < 16; c++) {
          if (hz < BAND_EDGES[c + 1]) {
            cell = c;
            break;
          }
        }

        this._binBand[i] = BAND_OF_CELL(cell);
      }
    }

    return this._binBand;
  }

  update(dt, payload, levers) {
    if (!payload) {
      this.hasAudio = false;
      return;
    }

    this.hasAudio = true;
    this.sampleRate = payload.sampleRate || 44100;
    this.ctx.sampleRate = this.sampleRate;

    const src = levers.useAgc ? payload : payload.raw;
    const bandGain = [
      levers.gain * levers.bassGain,
      levers.gain * levers.midGain,
      levers.gain * levers.trebleGain,
    ];

    // extra smoothing lever: a one-pole low-pass on top of the P10 lanes
    const tau = 0.02 + levers.smoothing * 0.6;
    const k = levers.smoothing > 0 ? 1 - Math.exp(-dt / tau) : 1;

    for (let d = 0; d < 4; d++) {
      for (let b = 0; b < 3; b++) {
        const i = d * 3 + b;
        const target = clamp(src.lanes[i] * bandGain[b]);
        this.lanes[i] += (target - this.lanes[i]) * k;
      }
    }

    for (let b = 0; b < 3; b++) {
      this.bands[b] = this.lanes[b];
    }

    for (let c = 0; c < 16; c++) {
      const target = clamp(src.cells[c] * bandGain[BAND_OF_CELL(c)]);
      this.grid[c] += (target - this.grid[c]) * k;
    }

    const beat = clamp(payload.beat * bandGain[0]);
    this.beatEnv += (beat - this.beatEnv) * k;

    const waveGain = Math.min(1, levers.gain);

    for (let i = 0; i < this.wave.length; i++) {
      const v = (payload.wave[i] || 0) * waveGain;
      this.wave[i] = v > 1 ? 1 : v < -1 ? -1 : v;
      this.dataArray[i] = Math.round(((this.wave[i] + 1) / 2) * 255);
    }

    const magGain = Math.min(4, levers.gain);

    for (let i = 0; i < this.freqMag.length; i++) {
      this.freqMag[i] = (payload.freqMag && payload.freqMag[i]) * magGain;
    }

    this._updateSpectrum(payload, bandGain);
  }

  // The byte FFT array scenes receive as `audioData` (they read it as
  // audioData[i] / 255), scaled by the same levers.
  _updateSpectrum(payload, bandGain) {
    const data = payload.byteFrequencyData;

    if (!data || !data.length) {
      this.spectrum = new Uint8Array(0);
      return;
    }

    if (this.spectrum.length !== data.length) {
      this.spectrum = new Uint8Array(data.length);
    }

    const binBand = this._binBands(
      data.length,
      payload.sampleRate || this.sampleRate,
    );

    for (let i = 0; i < data.length; i++) {
      const v = data[i] * bandGain[binBand[i]];
      this.spectrum[i] = v > 255 ? 255 : v < 0 ? 0 : v;
    }
  }
}

export { Point10AudioBridge };
