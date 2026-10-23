import { describe, expect, it } from "vitest";
import {
  DEFAULT_PERFORMANCE_TIMING,
  MIN_TRANSITION_SECONDS,
  normalizeTransitionDuration,
} from "../../src/renderer/src/application/worker/scene-timing.js";

describe("normalizeTransitionDuration", () => {
  it("keeps a valid duration", () => {
    expect(normalizeTransitionDuration(2.5)).toBe(2.5);
    expect(normalizeTransitionDuration("3")).toBe(3);
  });

  it("falls back to the default for missing or non-finite values", () => {
    expect(normalizeTransitionDuration(undefined)).toBe(
      DEFAULT_PERFORMANCE_TIMING.transitionSeconds,
    );
    expect(normalizeTransitionDuration(null)).toBe(
      DEFAULT_PERFORMANCE_TIMING.transitionSeconds,
    );
    expect(normalizeTransitionDuration(Number.NaN)).toBe(
      DEFAULT_PERFORMANCE_TIMING.transitionSeconds,
    );
    expect(normalizeTransitionDuration(Number.POSITIVE_INFINITY)).toBe(
      DEFAULT_PERFORMANCE_TIMING.transitionSeconds,
    );
  });

  it("never returns a duration shorter than the minimum", () => {
    expect(normalizeTransitionDuration(0)).toBe(MIN_TRANSITION_SECONDS);
    expect(normalizeTransitionDuration(-5)).toBe(MIN_TRANSITION_SECONDS);
  });
});
