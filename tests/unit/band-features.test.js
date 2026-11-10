import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BandFeatures } from "../../src/renderer/src/application/band-features.js";

const SAMPLE_RATE = 44100;
// The analyser hands over frequencyBinCount bins (fftSize 2048 -> 1024).
const BIN_COUNT = 1024;

let now = 0;

beforeEach(() => {
  now = 1000;
  vi.spyOn(performance, "now").mockImplementation(() => now);
});

afterEach(() => {
  vi.restoreAllMocks();
});

function advance(seconds) {
  now += seconds * 1000;
}

function tone({ from, to, db = -10 }) {
  const data = new Float32Array(BIN_COUNT).fill(-Infinity);

  for (let i = 0; i < BIN_COUNT; i += 1) {
    const hz = (i * SAMPLE_RATE) / 2 / BIN_COUNT;

    if (hz >= from && hz <= to) {
      data[i] = db;
    }
  }

  return data;
}

const silence = () => new Float32Array(BIN_COUNT).fill(-Infinity);

function settle(features, options, seconds = 1, step = 0.02) {
  const steps = Math.ceil(seconds / step);
  let output;

  for (let i = 0; i < steps; i += 1) {
    output = features.update(options);
    advance(step);
  }

  return output;
}

// Pink-ish noise: linear magnitude falls as 1/sqrt(f) (power ~1/f).
function pinkSpectrum() {
  const data = new Float32Array(BIN_COUNT).fill(-Infinity);

  for (let i = 1; i < BIN_COUNT; i += 1) {
    const hz = (i * SAMPLE_RATE) / 2 / BIN_COUNT;
    data[i] = -10 * Math.log10(hz) - 20;
  }

  return data;
}

describe("band boundaries", () => {
  it("reports silence as zeros", () => {
    const features = new BandFeatures();
    const out = features.update({
      floatFrequencyData: silence(),
      sampleRate: SAMPLE_RATE,
    });

    expect(out).toEqual({ bass: 0, mid: 0, treble: 0 });
  });

  it("routes a 60 Hz tone to bass only", () => {
    const features = new BandFeatures();
    const out = features.update({
      floatFrequencyData: tone({ from: 40, to: 80 }),
      sampleRate: SAMPLE_RATE,
    });

    expect(out.bass).toBeCloseTo(1, 5);
    expect(out.mid).toBe(0);
    expect(out.treble).toBe(0);
  });

  it("routes a 1 kHz tone to mid only", () => {
    const features = new BandFeatures();
    const out = features.update({
      floatFrequencyData: tone({ from: 900, to: 1100 }),
      sampleRate: SAMPLE_RATE,
    });

    expect(out.mid).toBeCloseTo(1, 5);
    expect(out.bass).toBe(0);
    expect(out.treble).toBe(0);
  });

  it("routes an 8 kHz tone to treble only", () => {
    const features = new BandFeatures();
    const out = features.update({
      floatFrequencyData: tone({ from: 7000, to: 9000 }),
      sampleRate: SAMPLE_RATE,
    });

    expect(out.treble).toBeCloseTo(1, 5);
    expect(out.bass).toBe(0);
    expect(out.mid).toBe(0);
  });

  it("ignores bins below 20 Hz and above 16 kHz", () => {
    const features = new BandFeatures();
    const out = features.update({
      floatFrequencyData: tone({ from: 16500, to: 20000 }),
      sampleRate: SAMPLE_RATE,
    });

    expect(out.treble).toBe(0);

    const features2 = new BandFeatures();
    // Only the first bin exists below 20 Hz at 44.1 kHz (bin 0, DC).
    const data = new Float32Array(BIN_COUNT).fill(-Infinity);
    data[0] = 0;
    const out2 = features2.update({
      floatFrequencyData: data,
      sampleRate: SAMPLE_RATE,
    });

    expect(out2.bass).toBe(0);
  });

  it("works at 48 kHz sample rates", () => {
    const features = new BandFeatures();
    const data = new Float32Array(BIN_COUNT).fill(-Infinity);

    for (let i = 0; i < BIN_COUNT; i += 1) {
      const hz = (i * 48000) / 2 / BIN_COUNT;

      if (hz >= 7000 && hz <= 9000) {
        data[i] = -10;
      }
    }

    const out = features.update({
      floatFrequencyData: data,
      sampleRate: 48000,
    });

    expect(out.treble).toBeCloseTo(1, 5);
    expect(out.mid).toBe(0);
    expect(out.bass).toBe(0);
  });
});

describe("per-band running peak normalisation", () => {
  it("reads a treble hit comparably to a bass hit of the same level", () => {
    const bass = new BandFeatures();
    const bassOut = settle(
      bass,
      {
        floatFrequencyData: tone({ from: 40, to: 80, db: -10 }),
        sampleRate: SAMPLE_RATE,
      },
      1,
    );

    const treble = new BandFeatures();
    const trebleOut = settle(
      treble,
      {
        floatFrequencyData: tone({ from: 7000, to: 9000, db: -10 }),
        sampleRate: SAMPLE_RATE,
      },
      1,
    );

    expect(bassOut.bass).toBeGreaterThan(0.9);
    expect(trebleOut.treble).toBeGreaterThan(0.9);
  });

  it("reads pink-ish noise comparably across the three bands", () => {
    const features = new BandFeatures();
    const out = settle(
      features,
      {
        floatFrequencyData: pinkSpectrum(),
        sampleRate: SAMPLE_RATE,
      },
      1,
    );

    const levels = [out.bass, out.mid, out.treble];
    const max = Math.max(...levels);
    const min = Math.min(...levels);

    expect(max - min).toBeLessThan(0.5);
    expect(min).toBeGreaterThan(0.1);
  });

  it("does not let a loud bass history drag down quiet treble hits", () => {
    const features = new BandFeatures();

    // Two seconds of a loud bass tone: the global AGC of old would set its
    // max-level from this and bury everything else.
    settle(
      features,
      {
        floatFrequencyData: tone({ from: 40, to: 80, db: -3 }),
        sampleRate: SAMPLE_RATE,
      },
      2,
    );

    advance(0.05);

    // A quiet but present 8 kHz tone must still reach ~1 on its own hits.
    const out = settle(
      features,
      {
        floatFrequencyData: tone({ from: 7000, to: 9000, db: -50 }),
        sampleRate: SAMPLE_RATE,
      },
      0.3,
    );

    expect(out.treble).toBeGreaterThan(0.9);
  });

  it("normaliser adapts back down after loud history ends", () => {
    const features = new BandFeatures();

    settle(
      features,
      {
        floatFrequencyData: tone({ from: 900, to: 1100, db: 0 }),
        sampleRate: SAMPLE_RATE,
      },
      1,
    );

    // Silence for long enough that the normaliser decays away (~2 s time
    // constant; needs ~10 s to fall below a -40 dB tone) and the quiet tone
    // becomes the new peak.
    settle(
      features,
      {
        floatFrequencyData: silence(),
        sampleRate: SAMPLE_RATE,
      },
      10,
    );

    const out = features.update({
      floatFrequencyData: tone({ from: 900, to: 1100, db: -40 }),
      sampleRate: SAMPLE_RATE,
    });

    expect(out.mid).toBeCloseTo(1, 1);
  });
});

describe("envelope timing", () => {
  it("attacks fast and releases with ~120 ms time constant", () => {
    const features = new BandFeatures();
    const loud = {
      floatFrequencyData: tone({ from: 900, to: 1100, db: -10 }),
      sampleRate: SAMPLE_RATE,
    };

    const out = settle(features, loud, 0.5);
    expect(out.mid).toBeGreaterThan(0.9);

    // Silence: envelope follows e^(-t/0.12). After 50 ms ~66%, after
    // 120 ms ~37%.
    advance(0.05);
    const after50 = features.update({
      floatFrequencyData: silence(),
      sampleRate: SAMPLE_RATE,
    });
    advance(0.07);
    const after120 = features.update({
      floatFrequencyData: silence(),
      sampleRate: SAMPLE_RATE,
    });

    expect(after50.mid).toBeGreaterThan(0.4);
    expect(after50.mid).toBeLessThan(0.85);
    expect(after120.mid).toBeLessThan(0.45);
  });

  it("decays to zero when audio stops entirely", () => {
    const features = new BandFeatures();

    settle(
      features,
      {
        floatFrequencyData: tone({ from: 40, to: 80, db: -10 }),
        sampleRate: SAMPLE_RATE,
      },
      1,
    );

    const out = settle(
      features,
      {
        floatFrequencyData: silence(),
        sampleRate: SAMPLE_RATE,
      },
      2,
    );

    expect(out.bass).toBeLessThan(0.01);
    expect(out.mid).toBeLessThan(0.01);
    expect(out.treble).toBeLessThan(0.01);
  });

  it("treats missing float data as silence rather than crashing", () => {
    const features = new BandFeatures();
    const out = features.update({ sampleRate: SAMPLE_RATE });

    expect(out).toEqual({ bass: 0, mid: 0, treble: 0 });
  });
});
