import { describe, expect, it } from "vitest";
import {
  DROP,
  createPhraseState,
  switchChance,
  updatePhrase,
} from "../../src/renderer/src/application/worker/phrase-detector.js";

describe("createPhraseState", () => {
  it("starts with no previous samples and a zero timer", () => {
    expect(createPhraseState()).toEqual({
      t: 0,
      low: undefined,
      phraseMax: undefined,
      prev: undefined,
      peakRise: 0,
    });
  });
});

describe("updatePhrase", () => {
  it("flips when bass recharges out of a trough and rolls over", () => {
    const state = {
      t: DROP.rearmSec,
      low: 0.05,
      phraseMax: 1,
      prev: 1,
      peakRise: 0,
    };

    const result = updatePhrase(state, 0.8, 0.1);

    expect(result.flipped).toBe(true);
    expect(result.state.t).toBe(0);
    expect(result.state.phraseMax).toBe(1);
    expect(result.state.prev).toBe(0.8);
  });

  it("does not flip before the rearm interval elapses", () => {
    const state = { t: 5, low: 0.05, phraseMax: 1, prev: 1, peakRise: 0 };

    const result = updatePhrase(state, 0.8, 0.1);

    expect(result.flipped).toBe(false);
    expect(result.state.t).toBeCloseTo(5.1);
  });

  it("does not flip below the crest threshold", () => {
    const state = {
      t: DROP.rearmSec,
      low: 0.05,
      phraseMax: 1,
      prev: 1,
      peakRise: 0,
    };

    const result = updatePhrase(state, 0.5, 0.1);

    expect(result.flipped).toBe(false);
  });

  it("requires a strong rise rather than a small wobble", () => {
    const state = {
      t: DROP.rearmSec,
      low: 0.5,
      phraseMax: 1,
      prev: 1,
      peakRise: 0,
    };

    const result = updatePhrase(state, 0.8, 0.1);

    expect(result.flipped).toBe(false);
  });

  it("tracks a running maximum that is never reset", () => {
    const state = { t: 0, low: 0, phraseMax: 0.9, prev: 0.9, peakRise: 0 };

    const result = updatePhrase(state, 1, 0.1);

    expect(result.flipped).toBe(false);
    expect(result.state.phraseMax).toBe(1);
  });
});

describe("switchChance", () => {
  it("never changes before the minimum time", () => {
    expect(switchChance(0, 10, 60)).toBe(0);
    expect(switchChance(10, 10, 60)).toBe(0);
  });

  it("ramps linearly between the minimum and maximum time", () => {
    expect(switchChance(35, 10, 60)).toBeCloseTo(0.5);
  });

  it("always changes once the maximum time has passed", () => {
    expect(switchChance(60, 10, 60)).toBe(1);
    expect(switchChance(120, 10, 60)).toBe(1);
  });

  it("always changes when the window is empty or inverted", () => {
    expect(switchChance(0, 10, 10)).toBe(1);
    expect(switchChance(0, 60, 10)).toBe(1);
  });
});
