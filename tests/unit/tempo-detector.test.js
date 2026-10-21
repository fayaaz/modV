import { describe, expect, it } from "vitest";
import {
  MAX_BPM,
  MIN_BPM,
  TempoDetector,
  createTempoDetector,
} from "../../src/renderer/src/application/tempo-detector.js";

const FPS = 60;

function createRandom(seed) {
  let state = seed >>> 0;

  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

function makeOnsetSignal({ bpm, seconds = 12, seed = 1 }) {
  const random = createRandom(seed);
  const period = 60 / bpm;
  const frames = Math.round(seconds * FPS);
  const signal = new Float64Array(frames);

  for (let beat = 0; beat < seconds; beat += period) {
    signal[Math.round(beat * FPS)] = 1;
  }

  for (let i = 1; i < frames; i++) {
    signal[i] = Math.max(signal[i], signal[i - 1] * 0.5 + random() * 0.001);
  }

  return signal;
}

function feed(detector, signal) {
  for (let i = 0; i < signal.length; i++) {
    detector.process(i / FPS, signal[i]);
  }
}

describe("TempoDetector", () => {
  it("exposes a pure detector API with empty defaults", () => {
    const detector = createTempoDetector();

    expect(detector).toBeInstanceOf(TempoDetector);
    expect(detector.bpm).toBe(0);
    expect(detector.confidence).toBe(0);
    expect(detector.minBpm).toBe(MIN_BPM);
    expect(detector.maxBpm).toBe(MAX_BPM);
  });

  it("detects a 120 BPM onset pattern within 2 BPM", () => {
    const detector = new TempoDetector();

    feed(detector, makeOnsetSignal({ bpm: 120 }));

    expect(Number.isFinite(detector.bpm)).toBe(true);
    expect(Number.isFinite(detector.confidence)).toBe(true);
    expect(Math.abs(detector.bpm - 120)).toBeLessThanOrEqual(2);
    expect(detector.confidence).toBeGreaterThan(0.2);
  });

  it("keeps a genuine half-time 87 BPM pattern at 87 BPM", () => {
    const detector = new TempoDetector();

    feed(detector, makeOnsetSignal({ bpm: 87, seed: 11 }));

    expect(Math.abs(detector.bpm - 87)).toBeLessThanOrEqual(2);
    expect(detector.bpm).toBeLessThan(100);
  });

  it("stays inside the supported BPM range", () => {
    const detector = new TempoDetector({ minBpm: 70, maxBpm: 200 });

    feed(detector, makeOnsetSignal({ bpm: 174, seed: 12 }));

    expect(detector.bpm).toBeGreaterThanOrEqual(70);
    expect(detector.bpm).toBeLessThanOrEqual(200);
  });

  it("reports no tempo and low confidence for silence", () => {
    const detector = new TempoDetector();
    const silence = new Float64Array(12 * FPS);

    feed(detector, silence);

    expect(detector.bpm).toBe(0);
    expect(detector.confidence).toBeLessThan(0.05);
  });

  it("resets to the initial state", () => {
    const detector = new TempoDetector();

    feed(detector, makeOnsetSignal({ bpm: 120, seconds: 6 }));
    detector.reset();

    expect(detector.bpm).toBe(0);
    expect(detector.confidence).toBe(0);
  });

  it("ignores non-finite input", () => {
    const detector = new TempoDetector();

    detector.process(NaN, 1);
    detector.process(0, NaN);

    expect(detector.bpm).toBe(0);
    expect(detector.confidence).toBe(0);
  });
});
