// Per-band audio-input features (bass / mid / treble) for Input Links and the
// shader u_bass/u_mid/u_treble uniforms.
//
// These are deliberately independent of the Point10 payload produced by
// audio-bands.js: the vendored plane9/Point10 scenes and their
// bassGain/midGain/trebleGain levers are tuned against that path and must not
// change. This module is the "input config audio" path only.
//
// Why not the old grid-mean features: a tone lights roughly the same number
// of FFT bins at every frequency, so meaning bins over a 1-bin-wide bass cell
// reads ~full scale while the same tone inside a 200-bin treble cell reads
// tens of dB down, and the global time-domain AGC let one loud kick drive the
// whole spectrum's normalisation. Here each band is simply the peak bin
// magnitude, normalised against that band's own recent peak.
//
// Level model per band:
//   m = peak magnitude (10^(dB/20)) across the band's bins this frame
//   envelope    e = max(e * e^(-dt/0.12), m)   fast attack, ~120 ms release
//   normaliser  n = max(n * e^(-dt/2.0),  m)   slow release, floored
//   output      clamp01(e / n)
//
// A steady tone pegs its band at 1 (same philosophy as the previous AGC
// path and as Input Link smoothing); a loud bass history cannot drag treble
// down, because every band keeps its own running peak. Dropping the audio to
// silence lets the envelope release in ~120 ms while the normaliser decays
// over ~2 s, so quiet material becomes the new peak shortly after loud
// material stops.

const BAND_DEFS = [
  { name: "bass", lo: 20, hi: 250 },
  { name: "mid", lo: 250, hi: 4000 },
  { name: "treble", lo: 4000, hi: 16000 },
];

const ENVELOPE_RELEASE_TAU = 0.12;
const NORMALISER_RELEASE_TAU = 2.0;
const NORMALISER_FLOOR = 1e-6;

const clamp01 = (x) => (x > 1 ? 1 : x > 0 ? x : 0);

class BandFeatures {
  constructor() {
    this.envelope = new Float32Array(BAND_DEFS.length);
    this.normaliser = new Float32Array(BAND_DEFS.length);
    this.ranges = [];
    this.rangeKey = "";
    this.lastT = 0;
  }

  // hz(bin) = bin * sampleRate / fftSize; here binCount = frequencyBinCount
  // (fftSize / 2), so hz = bin * sampleRate / (2 * binCount).
  _binRange(loHz, hiHz, binCount, sampleRate) {
    const hz = (bin) => (bin * sampleRate) / 2 / binCount;
    let a = binCount - 1;
    let b = 0;

    // Lowest bin at or above loHz ...
    a = 0;
    while (a < binCount - 1 && hz(a) < loHz) {
      a += 1;
    }

    // ... highest bin at or below hiHz. Falls back to the band start when
    // hiHz sits below bin 0 (never true at supported rates).
    b = a;
    while (b < binCount - 1 && hz(b + 1) <= hiHz) {
      b += 1;
    }

    if (hz(a) > hiHz) {
      return null;
    }

    return [a, b];
  }

  _bandRanges(binCount, sampleRate) {
    const key = binCount + ":" + sampleRate;

    if (this.rangeKey !== key) {
      this.rangeKey = key;
      this.ranges = BAND_DEFS.map((def) =>
        this._binRange(def.lo, def.hi, binCount, sampleRate),
      );
    }

    return this.ranges;
  }

  update({ floatFrequencyData, sampleRate = 44100 } = {}) {
    const now = performance.now();
    const dt = Math.min(0.1, (now - (this.lastT || now)) / 1000);
    this.lastT = now;

    const envelopeDecay = Math.exp(-dt / ENVELOPE_RELEASE_TAU);
    const normaliserDecay = Math.exp(-dt / NORMALISER_RELEASE_TAU);

    const out = {};

    if (!floatFrequencyData || !floatFrequencyData.length) {
      for (let i = 0; i < BAND_DEFS.length; i += 1) {
        this.envelope[i] *= envelopeDecay;
        this.normaliser[i] = Math.max(
          this.normaliser[i] * normaliserDecay,
          NORMALISER_FLOOR,
        );
        out[BAND_DEFS[i].name] = clamp01(this.envelope[i] / this.normaliser[i]);
      }

      return out;
    }

    const binCount = floatFrequencyData.length;
    const ranges = this._bandRanges(binCount, sampleRate);

    for (let i = 0; i < BAND_DEFS.length; i += 1) {
      const range = ranges[i];
      let peak = 0;

      if (range) {
        for (let bin = range[0]; bin <= range[1]; bin += 1) {
          const magnitude = Math.pow(10, floatFrequencyData[bin] / 20);

          if (magnitude > peak) {
            peak = magnitude;
          }
        }
      }

      const envelope = Math.max(this.envelope[i] * envelopeDecay, peak);
      const normaliser = Math.max(
        this.normaliser[i] * normaliserDecay,
        peak,
        NORMALISER_FLOOR,
      );

      this.envelope[i] = envelope;
      this.normaliser[i] = normaliser;
      out[BAND_DEFS[i].name] = clamp01(envelope / normaliser);
    }

    return out;
  }
}

export { BandFeatures, BAND_DEFS };
