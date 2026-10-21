import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  AudioBands,
  BAND_CELLS,
  BAND_EDGES,
  BAND_FEATURES,
  isBandFeature,
} from "../../src/renderer/src/application/audio-bands.js";

const SAMPLE_RATE = 44100;
const FFT_SIZE = 1024;

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

function tone({ from, to, db = 0 }) {
  const data = new Float32Array(FFT_SIZE).fill(-Infinity);

  for (let i = 0; i < FFT_SIZE; i++) {
    const hz = (i * SAMPLE_RATE) / 2 / FFT_SIZE;

    if (hz >= from && hz <= to) {
      data[i] = db;
    }
  }

  return data;
}

const quietTimeDomain = () => new Uint8Array(FFT_SIZE).fill(128);
const loudTimeDomain = () => new Uint8Array(FFT_SIZE).fill(230);

function settle(bands, options, seconds = 2, step = 0.05) {
  const steps = Math.ceil(seconds / step);
  let payload;

  for (let i = 0; i < steps; i++) {
    payload = bands.update(options);
    advance(step);
  }

  return payload;
}

// Pink-ish noise: linear magnitude falls as 1/sqrt(f) (power ~1/f).
function pinkSpectrum() {
  const data = new Float32Array(FFT_SIZE).fill(-Infinity);

  for (let i = 1; i < FFT_SIZE; i++) {
    const hz = (i * SAMPLE_RATE) / 2 / FFT_SIZE;
    data[i] = -10 * Math.log10(hz);
  }

  return data;
}

const clamp01 = (x) => (x > 1 ? 1 : x > 0 ? x : 0);

describe("band definitions", () => {
  it("exposes the three AGC band features", () => {
    expect(BAND_FEATURES).toEqual(["bass", "mid", "treble"]);
    expect(isBandFeature("bass")).toBe(true);
    expect(isBandFeature("mid")).toBe(true);
    expect(isBandFeature("treble")).toBe(true);
    expect(isBandFeature("kick")).toBe(false);
  });

  it("maps 16 cells across the 17 frequency edges", () => {
    expect(BAND_EDGES).toHaveLength(17);
    expect(BAND_EDGES[10]).toBe(2000);
    expect(BAND_CELLS).toEqual([
      [0, 4],
      [5, 9],
      [10, 15],
    ]);
  });
});

describe("AudioBands", () => {
  it("reads a bass tone into the bass band only", () => {
    const bands = new AudioBands();
    const frequency = tone({ from: 30, to: 300, db: -20 });
    const options = {
      floatFrequencyData: frequency,
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    };

    bands.update(options);
    advance(0.1);
    const payload = bands.update(options);

    expect(payload.bands[0]).toBe(1);
    expect(payload.bands[1]).toBe(0);
    expect(payload.bands[2]).toBe(0);
    expect(payload.bass).toBeLessThanOrEqual(1);
    expect(payload.raw.bands[0]).toBeGreaterThan(0);
    expect(payload.raw.bands[0]).toBeLessThan(payload.bands[0]);
  });

  it("places a 2000 Hz tone in treble but not a 1.9 kHz tone", () => {
    const at2k = new AudioBands();
    const twoK = settle(at2k, {
      floatFrequencyData: tone({ from: 2000, to: 2040, db: -20 }),
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    expect(twoK.bands[2]).toBeGreaterThan(0);
    expect(twoK.treble).toBeGreaterThan(0);

    const at1900 = new AudioBands();
    const nineteenHundred = settle(at1900, {
      floatFrequencyData: tone({ from: 1880, to: 1960, db: -20 }),
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    expect(nineteenHundred.mid).toBeGreaterThan(0);
    expect(nineteenHundred.bands[2]).toBe(0);
    expect(nineteenHundred.treble).toBe(0);
  });

  it("keeps a pink spectrum's treble feature comparable to bass", () => {
    const bands = new AudioBands();
    const payload = settle(bands, {
      floatFrequencyData: pinkSpectrum(),
      byteTimeDomainData: loudTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    for (const feature of ["bass", "mid", "treble"]) {
      expect(Number.isFinite(payload[feature])).toBe(true);
      expect(payload[feature]).toBeGreaterThanOrEqual(0);
      expect(payload[feature]).toBeLessThanOrEqual(1);
    }

    expect(payload.bass).toBeGreaterThan(0);
    expect(payload.treble).toBeGreaterThanOrEqual(0.25 * payload.bass);
  });

  it("reads exactly zero for all features in silence", () => {
    const bands = new AudioBands();
    const payload = bands.update({
      floatFrequencyData: null,
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    expect(payload.bass).toBe(0);
    expect(payload.mid).toBe(0);
    expect(payload.treble).toBe(0);
  });

  it("keeps a full-scale tone at ~1 in its band", () => {
    const bands = new AudioBands();
    const payload = settle(bands, {
      floatFrequencyData: tone({ from: 30, to: 300, db: 0 }),
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    expect(payload.bass).toBeGreaterThan(0.95);
    expect(payload.bass).toBeLessThanOrEqual(1);
    expect(payload.mid).toBe(0);
    expect(payload.treble).toBe(0);
  });

  it("keeps lane and cell payloads as raw linear magnitudes", () => {
    const bands = new AudioBands();
    const payload = settle(bands, {
      floatFrequencyData: tone({ from: 30, to: 300, db: -30 }),
      byteTimeDomainData: loudTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    // A -30 dB tone with the 0.8 peak AGC stays around 0.095 linear; a dB
    // feature remap would have lifted it to ~0.66.
    expect(payload.cells[0]).toBeGreaterThan(0);
    expect(payload.cells[0]).toBeLessThan(0.2);
    expect(payload.lanes[0]).toBeLessThan(0.2);
    expect(payload.bands[0]).toBeLessThan(0.2);

    expect(Array.from(payload.cells)).toEqual(Array.from(bands.grid, clamp01));
    expect(Array.from(payload.lanes)).toEqual(Array.from(bands.lanes, clamp01));
  });

  it("normalizes quiet material with the time-domain AGC", () => {
    const bands = new AudioBands();
    const options = {
      floatFrequencyData: tone({ from: 30, to: 300, db: -20 }),
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    };

    bands.update(options);
    advance(0.1);
    const payload = bands.update(options);

    expect(payload.bands[0]).toBe(1);
    expect(payload.raw.bands[0]).toBeGreaterThan(0);
    expect(payload.raw.bands[0]).toBeLessThan(0.3);
    expect(payload.raw.bands[0]).toBeLessThan(payload.bands[0]);
  });

  it("decays bands and cells when no frequency data arrives", () => {
    const bands = new AudioBands();
    const frequency = tone({ from: 30, to: 300, db: -30 });

    bands.update({
      floatFrequencyData: frequency,
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });
    advance(0.1);
    const before = bands.update({
      floatFrequencyData: frequency,
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    advance(0.1);
    const after = bands.update({
      floatFrequencyData: null,
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    expect(before.cells[0]).toBeGreaterThan(0);
    expect(after.raw.cells[0]).toBeLessThan(before.cells[0]);
    expect(after.bands).toEqual([0, 0, 0]);
    expect(after.bass).toBe(0);
    expect(after.mid).toBe(0);
    expect(after.treble).toBe(0);
  });

  it("exposes the supplied byte frequency data in the payload", () => {
    const bands = new AudioBands();
    const byteFrequencyData = new Uint8Array(FFT_SIZE);

    byteFrequencyData[2] = 100;
    byteFrequencyData[14] = 50;
    byteFrequencyData[FFT_SIZE - 1] = 255;

    const payload = bands.update({
      floatFrequencyData: tone({ from: 30, to: 300, db: -20 }),
      byteTimeDomainData: quietTimeDomain(),
      byteFrequencyData,
      sampleRate: SAMPLE_RATE,
    });

    expect(payload.byteFrequencyData).toHaveLength(FFT_SIZE);
    expect(Array.from(payload.byteFrequencyData)).toEqual(
      Array.from(byteFrequencyData),
    );
  });

  it("defaults the payload byte frequency data to empty when it is missing", () => {
    const bands = new AudioBands();

    const payload = bands.update({
      floatFrequencyData: null,
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    });

    expect(payload.byteFrequencyData).toHaveLength(0);
  });

  it("normalizes the time-domain wave to -1..1", () => {
    const bands = new AudioBands();
    const payload = bands.update({
      floatFrequencyData: null,
      byteTimeDomainData: new Uint8Array([0, 255, 128]),
      sampleRate: SAMPLE_RATE,
    });

    expect(payload.wave).toBeInstanceOf(Float32Array);
    expect(payload.wave).toHaveLength(1024);
    expect(payload.wave[0]).toBe(-1);
    expect(payload.wave[1]).toBe(1);
    expect(payload.wave[2]).toBeCloseTo(0, 2);
  });

  it("exposes the raw signal with the AGC gain divided back out", () => {
    const bands = new AudioBands();
    const options = {
      floatFrequencyData: tone({ from: 30, to: 300, db: -20 }),
      byteTimeDomainData: quietTimeDomain(),
      sampleRate: SAMPLE_RATE,
    };

    bands.update(options);
    advance(0.1);
    const payload = bands.update(options);

    expect(payload.raw.bands).toHaveLength(3);
    expect(payload.raw.lanes).toHaveLength(12);
    expect(payload.raw.cells).toHaveLength(16);
  });
});
