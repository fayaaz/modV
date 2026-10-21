// Plane9 audio semantics over the WebAudio analyser.
// band(channel, damping, bandnr, nomusic) reads the smoothed 3-band energy
// (0=bass, 1=middle, 2=high). channel -1 = both, 0/1 = left/right (WebAudio
// FFT data is mono, so both sides read the same grid). damping 0..3 is a
// longer smoothing hint. Values are linear FFT magnitudes: silence reads ~0
// (no explosion), a full-scale tone ~1, dense loud music ~0.2-0.5 (the AGC
// normalizes the time-domain peak, so dense music keeps its sub-1 shape).
// Per the wiki, music functions take a `nomusic` fallback ("if no music is
// playing 'nomusic' is returned") - waveform/spectrum honor it when NO stream
// exists at all (Gravity Spectrum's sweep dips flat in silence now), but
// band/beat do not: a live-but-quiet stream must report ~0, not a hardcoded
// fallback that would fire the scene's audio effects in silence. beat(x)
// returns a decaying bass envelope. spectrum() reads the analyzer linearly
// (bin = pos*399, interpolated - decompile 0x100234f0), see the function.

const BAND_EDGES = [
  30, 50, 80, 130, 200, 300, 450, 700, 1000, 1500, 2200, 3300, 5000, 7500,
  11000, 16000, 20000,
];

// plane9's three bands over the 16 log cells: bass .-450 Hz, middle ..3.3 kHz, high above.
const BAND_CELLS = [
  [0, 4],
  [5, 9],
  [10, 15],
];

export class P9Audio {
  constructor(engine) {
    this.engine = engine;
    this.freq = new Float32Array(1024);
    this.freqL = new Float32Array(1024);
    this.freqR = new Float32Array(1024);
    this.wave = new Float32Array(1024);
    this.grid = new Float32Array(16);
    this.bands = [0, 0, 0];
    this.bass = 0;
    this.beatEnv = 0;
    this.cells = null;
    // Smooth band reads per damping hint (plane9's `damping` arg: 0..3,
    // higher = longer smoothing). Keyed [damping*3+band]; the raw band is the
    // damping-0 lane, higher lanes low-pass transients like kick drums.
    this._smoo = new Array(12).fill(0);
  }

  get hasAudio() {
    return !!(this.engine && this.engine.analyser);
  }

  update(dt) {
    const e = this.engine;
    if (!this.hasAudio || !e.freqArray) return;
    const n = Math.min(e.freqMag.length, this.freq.length);
    for (let i = 0; i < n; i++) this.freq[i] = e.freqMag[i] ?? 0;
    if (e.freqMagL?.length) {
      for (let i = 0; i < Math.min(e.freqMagL.length, n); i++) {
        this.freqL[i] = e.freqMagL[i] ?? 0;
        this.freqR[i] = e.freqMagR[i] ?? 0;
      }
    }
    const wn = Math.min(e.dataArray.length, this.wave.length);
    for (let i = 0; i < wn; i++) this.wave[i] = (e.dataArray[i] / 255) * 2 - 1;

    if (!this.cells)
      this.cells = BAND_EDGES.slice(0, 16).map((lo, i) =>
        this.binRange(lo, BAND_EDGES[i + 1]),
      );
    const target = new Float32Array(16);
    for (let c = 0; c < 16; c++) {
      const [a, b] = this.cells[c];
      let sum = 0;
      for (let i = a; i <= b; i++) sum += this.freq[i];
      target[c] = sum / (b - a + 1);
    }
    for (let c = 0; c < 16; c++) {
      const g = this.grid[c];
      if (target[c] >= g)
        this.grid[c] = g + (target[c] - g) * Math.min(1, dt * 30);
      else this.grid[c] = g * Math.exp(-dt * (6 + c));
    }

    for (let b = 0; b < 3; b++) {
      const [lo, hi] = BAND_CELLS[b];
      let sum = 0;
      for (let c = lo; c <= hi; c++) sum += this.grid[c];
      this.bands[b] = sum / (hi - lo + 1);
    }
    // Advance the per-damping smoothed lanes toward the raw band (higher
    // damping = slower). Lane 0 stays raw; drums like kicks are low-passed on
    // the 1..3 lanes that drive tumble/rotation reads.
    for (let d = 1; d <= 3; d++) {
      const k = Math.exp(-dt / [0.05, 0.12, 0.25][d - 1]);
      for (let b = 0; b < 3; b++) {
        const i = d * 3 + b;
        this._smoo[i] += (this.bands[b] - this._smoo[i]) * (1 - k);
      }
    }

    const bass = (this.grid[0] + this.grid[1]) * 0.5;
    this.beatEnv *= Math.exp(-dt * 3.5);
    // ponytail: refractory gate - the beat re-fires only after the envelope
    // decayed below 0.2 (tau 0.29 s → ~0.5 s between triggers, max ~2/s).
    // Without it, a busy mix's bass fluctuations re-spiked the envelope on
    // every kick and pinned it near 1 - perm terms like Sea Of Bricks'
    // beat*50 then advanced at a constant ~35 units/s and the tubes' diameter
    // wave slid continuously instead of pulsing per kick (plane9's beat is a
    // short transient; the rhythm is the point).
    if (this.beatEnv < 0.2 && bass > this.bass * 1.25 && bass > 0.03) {
      this.beatEnv = Math.min(
        1,
        Math.max(this.beatEnv, (bass - this.bass) * 12),
      );
    }
    this.bass = this.bass + (bass - this.bass) * Math.min(1, dt * 4);
  }

  binRange(loHz, hiHz) {
    const n = this.freq.length;
    const rate = this.engine.ctx ? this.engine.ctx.sampleRate : 44100;
    const hz = (i) => (i * rate) / 2 / n;
    let a = 0,
      b = n - 1;
    while (a < n - 1 && hz(a) < loHz) a++;
    while (b > 0 && hz(b) > hiHz) b--;
    return [a, Math.min(b, n - 1)];
  }

  // eslint-disable-next-line no-unused-vars
  band(channel, damping, bandnr, nomusic) {
    const idx = Math.max(0, Math.min(2, Math.round(bandnr ?? 0)));
    const d = Math.max(0, Math.min(3, Math.round(damping ?? 0)));
    // damping 0 = raw per-frame band; 1..3 = low-passed lane of the same band.
    return d > 0 ? this._smoo[d * 3 + idx] ?? 0 : this.bands[idx] ?? 0;
  }

  // eslint-disable-next-line no-unused-vars
  beat(fallback) {
    return Math.min(1, this.beatEnv);
  }

  waveform(channel, pos, nomusic) {
    if (!this.hasAudio) return num01(nomusic);
    const i = Math.floor(num01(pos) * (this.wave.length - 1));
    return this.wave[i] ?? 0;
  }

  spectrum(channel, pos, nomusic) {
    if (!this.hasAudio) return num01(nomusic);
    // plane9's spectrum() reads the analyzer spectrum LINEARLY: bin = pos*399,
    // interpolated between the two adjacent bins (decompile 0x100234f0 - no
    // log mapping, no window average). The analyzer's FFT runs DECIMATED by 4
    // (the 1/11025 = 1/(fs/4) constant in the band-edge math), so the 400
    // bins cover only the bass/mid range - ~4.3 kHz at 44.1 k. The port's log
    // 30 Hz..20 kHz mapping was wrong (Monster's treble bars read ~0 on
    // bass-heavy music while plane9's bars all respond). Scale: plane9 bin b =
    // b*(fs/4096) Hz; in our 2048-bin analyser that's bin = pos*399/2, with a
    // +1 offset to skip the DC bin (WebAudio inflates bin 0 - it pegged bar 1
    // spuriously; plane9's band math excludes its lowest bins too).
    // Channel -1 = (L+R)/2, 0 = left, 1 = right (plane9 reads L/R separately -
    // Monster's top row uses channel 0, bottom row channel 1). Falls back to
    // the mono spectrum when the engine has no stereo analysers (tests, stubs).
    let arr = this.freq;
    if (channel === 0 && this.engine?.freqMagL?.length) arr = this.freqL;
    else if (channel === 1 && this.engine?.freqMagR?.length) arr = this.freqR;
    else if (channel === -1 && this.engine?.freqMagL?.length) {
      const p0 = Math.max(0, Math.min(1, num01(pos)));
      const j = p0 * 199.5 + 1;
      const a0 = Math.floor(j),
        b0 = Math.min(this.freqL.length - 1, a0 + 1),
        fr0 = j - a0;
      return (
        ((this.freqL[a0] ?? 0) +
          ((this.freqL[b0] ?? 0) - (this.freqL[a0] ?? 0)) * fr0 +
          (this.freqR[a0] ?? 0) +
          ((this.freqR[b0] ?? 0) - (this.freqR[a0] ?? 0)) * fr0) *
        0.5
      );
    }
    const p = Math.max(0, Math.min(1, num01(pos)));
    const j = p * 199.5 + 1;
    const a = Math.floor(j);
    const b = Math.min(arr.length - 1, a + 1);
    const frac = j - a;
    const va = arr[a] ?? 0;
    const vb = arr[b] ?? 0;
    return va + (vb - va) * frac;
  }
}

const num01 = (v) => (typeof v === "number" && isFinite(v) ? v : 0);
