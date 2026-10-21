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
