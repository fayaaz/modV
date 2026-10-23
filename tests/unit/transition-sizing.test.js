import { describe, expect, it } from "vitest";
import {
  getTransitionRenderSize,
  MAX_TRANSITION_PIXELS,
} from "../../src/renderer/src/application/worker/transition-sizing.js";

describe("getTransitionRenderSize", () => {
  it("keeps output that is already within the pixel budget", () => {
    expect(getTransitionRenderSize(1280, 720)).toEqual({
      width: 1280,
      height: 720,
      scaled: false,
    });
  });

  it("scales large output down while preserving the aspect ratio", () => {
    const size = getTransitionRenderSize(3840, 2160);

    expect(size.scaled).toBe(true);
    expect(size.width * size.height).toBeLessThanOrEqual(MAX_TRANSITION_PIXELS);
    expect(size.width / size.height).toBeCloseTo(3840 / 2160, 2);
  });

  it("handles ultrawide output", () => {
    const size = getTransitionRenderSize(5120, 1440);
    const ratio = size.width / size.height;

    expect(ratio).toBeCloseTo(5120 / 1440, 2);
    expect(size.width * size.height).toBeLessThanOrEqual(MAX_TRANSITION_PIXELS);
  });

  it("accepts a custom pixel budget", () => {
    const size = getTransitionRenderSize(1920, 1080, 1280 * 720);

    expect(size.scaled).toBe(true);
    expect(size.width * size.height).toBeLessThanOrEqual(1280 * 720);
  });

  it("clamps invalid sizes to at least one pixel", () => {
    expect(getTransitionRenderSize(0, 0)).toEqual({
      width: 1,
      height: 1,
      scaled: false,
    });
  });

  it("ignores an invalid pixel budget", () => {
    expect(getTransitionRenderSize(800, 600, Number.NaN).scaled).toBe(false);
    expect(getTransitionRenderSize(800, 600, 0).scaled).toBe(false);
  });
});
