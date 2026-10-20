import { expect, test } from "@playwright/test";
import {
  TempoDetector,
  createTempoDetector,
} from "../../../../src/renderer/src/application/tempo-detector";

const FPS = 60;
const MIN_BPM = 70;
const MAX_BPM = 200;

function createRandom(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 0x100000000;
  };
}

function makeOnsetSignal({
  bpm,
  seconds = 12,
  jitter = 0,
  noise = 0,
  ghosts = [],
  seed = 1,
}) {
  const random = createRandom(seed);
  const period = 60 / bpm;
  const events = [];

  for (let beat = 0; beat < seconds; beat += period) {
    events.push({
      time: beat + (random() * 2 - 1) * jitter,
      strength: 1,
    });

    for (let i = 0; i < ghosts.length; i++) {
      const ghost = ghosts[i];
      events.push({
        time: beat + period * ghost.offset,
        strength: ghost.strength,
      });
    }
  }

  const frames = Math.round(seconds * FPS);
  const signal = new Float64Array(frames);

  for (let i = 0; i < frames; i++) {
    const time = i / FPS;

    for (let j = 0; j < events.length; j++) {
      const event = events[j];
      const distance = Math.abs(event.time - time);

      if (distance < 0.06) {
        signal[i] += event.strength * Math.exp(-Math.pow(distance / 0.025, 2));
      }
    }

    if (noise) {
      signal[i] += random() * noise;
    }
  }

  return signal;
}

function feed(detector, signal) {
  for (let i = 0; i < signal.length; i++) {
    detector.process(i / FPS, signal[i]);
  }
}

function expectValidTempo(detector) {
  expect(Number.isFinite(detector.bpm)).toBe(true);
  expect(Number.isFinite(detector.confidence)).toBe(true);
  expect(detector.bpm).toBeGreaterThanOrEqual(MIN_BPM);
  expect(detector.bpm).toBeLessThanOrEqual(MAX_BPM);
  expect(detector.confidence).toBeGreaterThanOrEqual(0);
  expect(detector.confidence).toBeLessThanOrEqual(1);
}

test.describe("tempo detector", () => {
  test("exposes the pure detector API", () => {
    const detector = createTempoDetector();

    expect(detector).toBeInstanceOf(TempoDetector);
    expect(detector.bpm).toBe(0);
    expect(detector.confidence).toBe(0);

    detector.process(0, 1);
    detector.addOnset(0.5, 1);
    detector.reset();

    expect(detector.bpm).toBe(0);
    expect(detector.confidence).toBe(0);
  });

  const tempos = [120, 128, 140, 174, 178];

  for (let i = 0; i < tempos.length; i++) {
    const bpm = tempos[i];

    test(`detects ${bpm} BPM within 2 BPM`, () => {
      const detector = new TempoDetector();
      feed(detector, makeOnsetSignal({ bpm, seconds: 12, seed: i + 1 }));

      expectValidTempo(detector);
      expect(Math.abs(detector.bpm - bpm)).toBeLessThanOrEqual(2);
      expect(detector.confidence).toBeGreaterThan(0.2);
    });
  }

  test("keeps a genuine half-time 87 BPM pattern at 87 BPM", () => {
    const detector = new TempoDetector();
    feed(detector, makeOnsetSignal({ bpm: 87, seconds: 12, seed: 11 }));

    expectValidTempo(detector);
    expect(Math.abs(detector.bpm - 87)).toBeLessThanOrEqual(2);
    expect(detector.bpm).toBeLessThan(100);
  });

  test("does not report fast drum & bass at half time", () => {
    const detector = new TempoDetector();
    feed(
      detector,
      makeOnsetSignal({
        bpm: 174,
        seconds: 12,
        ghosts: [{ offset: 0.5, strength: 0.25 }],
        seed: 12,
      }),
    );

    expectValidTempo(detector);
    expect(detector.bpm).toBeGreaterThanOrEqual(170);
    expect(detector.bpm).toBeLessThanOrEqual(178);
  });

  test("handles timing jitter, noise and weak off-beat ghosts", () => {
    const detector = new TempoDetector();
    feed(
      detector,
      makeOnsetSignal({
        bpm: 174,
        seconds: 12,
        jitter: 0.012,
        noise: 0.04,
        ghosts: [
          { offset: 0.5, strength: 0.25 },
          { offset: 0.25, strength: 0.12 },
        ],
        seed: 13,
      }),
    );

    expectValidTempo(detector);
    expect(Math.abs(detector.bpm - 174)).toBeLessThanOrEqual(2);
  });

  test("handles jitter and noise at slow tempos", () => {
    const detector = new TempoDetector();
    feed(
      detector,
      makeOnsetSignal({
        bpm: 87,
        seconds: 12,
        jitter: 0.012,
        noise: 0.04,
        seed: 14,
      }),
    );

    expectValidTempo(detector);
    expect(Math.abs(detector.bpm - 87)).toBeLessThanOrEqual(2);
    expect(detector.bpm).toBeLessThan(100);
  });

  test("stays stable once it has a confident estimate", () => {
    const detector = new TempoDetector();
    const signal = makeOnsetSignal({
      bpm: 174,
      seconds: 14,
      jitter: 0.008,
      noise: 0.02,
      ghosts: [{ offset: 0.5, strength: 0.2 }],
      seed: 15,
    });

    feed(detector, signal.subarray(0, 8 * FPS));
    const earlyBpm = detector.bpm;
    const earlyConfidence = detector.confidence;

    feed(detector, signal.subarray(8 * FPS));

    expectValidTempo(detector);
    expect(earlyConfidence).toBeGreaterThan(0.2);
    expect(Math.abs(earlyBpm - 174)).toBeLessThanOrEqual(2);
    expect(Math.abs(detector.bpm - earlyBpm)).toBeLessThanOrEqual(2);
  });

  test("reports no tempo and low confidence for silence", () => {
    const detector = new TempoDetector();
    const silence = new Float64Array(12 * FPS);

    feed(detector, silence);

    expect(Number.isFinite(detector.bpm)).toBe(true);
    expect(Number.isFinite(detector.confidence)).toBe(true);
    expect(detector.bpm).toBe(0);
    expect(detector.confidence).toBeLessThan(0.05);
  });

  test("never leaves the supported BPM range", () => {
    const cases = [
      { bpm: 70, seconds: 12 },
      { bpm: 200, seconds: 12 },
      { bpm: 174, seconds: 12, jitter: 0.02, noise: 0.08 },
      { bpm: 87, seconds: 12, jitter: 0.02, noise: 0.08 },
    ];

    for (let i = 0; i < cases.length; i++) {
      const detector = new TempoDetector();
      feed(detector, makeOnsetSignal({ ...cases[i], seed: 20 + i }));

      expectValidTempo(detector);
      expect(Math.abs(detector.bpm - cases[i].bpm)).toBeLessThanOrEqual(2);
    }
  });
});
