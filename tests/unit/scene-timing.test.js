import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_PERFORMANCE_TIMING,
  MIN_TRANSITION_SECONDS,
  clampCellDwell,
  normalizeTransitionDuration,
  pickCellDwell,
} from "../../src/renderer/src/application/worker/scene-timing.js";

describe("pickCellDwell", () => {
  it("returns the minimum at a roll of zero", () => {
    expect(
      pickCellDwell({ minSeconds: 5, maxSeconds: 10, random: () => 0 }),
    ).toBe(5);
  });

  it("stays below the maximum for a roll just under one", () => {
    const value = pickCellDwell({
      minSeconds: 5,
      maxSeconds: 10,
      random: () => 0.999999,
    });

    expect(value).toBeGreaterThan(5);
    expect(value).toBeLessThan(10);
  });

  it("returns the maximum at a roll of one", () => {
    expect(
      pickCellDwell({ minSeconds: 5, maxSeconds: 10, random: () => 1 }),
    ).toBe(10);
  });

  it("clamps out-of-range rolls into the interval", () => {
    expect(
      pickCellDwell({ minSeconds: 5, maxSeconds: 10, random: () => -2 }),
    ).toBe(5);
    expect(
      pickCellDwell({ minSeconds: 5, maxSeconds: 10, random: () => 2 }),
    ).toBe(10);
  });

  it("falls back to the minimum for a non-finite roll", () => {
    expect(
      pickCellDwell({
        minSeconds: 5,
        maxSeconds: 10,
        random: () => Number.NaN,
      }),
    ).toBe(5);
  });

  it("uses the injected random function exactly once", () => {
    const random = vi.fn(() => 0.5);

    expect(pickCellDwell({ minSeconds: 5, maxSeconds: 10, random })).toBe(7.5);
    expect(random).toHaveBeenCalledTimes(1);
  });

  it("collapses equal and inverted bounds to the minimum", () => {
    expect(
      pickCellDwell({ minSeconds: 5, maxSeconds: 5, random: () => 0.5 }),
    ).toBe(5);
    expect(
      pickCellDwell({ minSeconds: 10, maxSeconds: 5, random: () => 0.5 }),
    ).toBe(10);
    expect(
      pickCellDwell({ minSeconds: 10, maxSeconds: 5, random: () => 1 }),
    ).toBe(10);
  });

  it("uses finite fallbacks for missing or invalid bounds", () => {
    expect(
      pickCellDwell({
        minSeconds: undefined,
        maxSeconds: undefined,
        random: () => 0,
      }),
    ).toBe(DEFAULT_PERFORMANCE_TIMING.minSeconds);

    expect(
      pickCellDwell({ minSeconds: 5, maxSeconds: Number.NaN, random: () => 0 }),
    ).toBe(5);

    const value = pickCellDwell({
      minSeconds: 5,
      maxSeconds: Number.POSITIVE_INFINITY,
      random: () => 0.5,
    });

    expect(Number.isFinite(value)).toBe(true);
    expect(value).toBeGreaterThanOrEqual(5);
  });
});

describe("clampCellDwell", () => {
  it("keeps a value inside the interval", () => {
    expect(clampCellDwell(7, 5, 10)).toBe(7);
  });

  it("clamps values below the minimum and above the maximum", () => {
    expect(clampCellDwell(2, 5, 10)).toBe(5);
    expect(clampCellDwell(20, 5, 10)).toBe(10);
  });

  it("falls back to the maximum for a non-finite value", () => {
    expect(clampCellDwell(Number.NaN, 5, 10)).toBe(10);
    expect(clampCellDwell(undefined, 5, 10)).toBe(10);
  });

  it("collapses equal and inverted bounds to the minimum", () => {
    expect(clampCellDwell(7, 5, 5)).toBe(5);
    expect(clampCellDwell(7, 10, 5)).toBe(10);
  });

  it("uses finite fallbacks for missing bounds", () => {
    const value = clampCellDwell(7, undefined, undefined);

    expect(Number.isFinite(value)).toBe(true);
    expect(value).toBeGreaterThanOrEqual(DEFAULT_PERFORMANCE_TIMING.minSeconds);
  });
});

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
