import { describe, expect, it } from "vitest";
import { createFrameClock } from "../../src/renderer/src/application/point10/timing.js";

describe("Point10 frame clock", () => {
  it("reports no elapsed time on the first frame", () => {
    const tick = createFrameClock();

    expect(tick(1000)).toBe(0);
  });

  it("converts the elapsed milliseconds between frames to seconds", () => {
    const tick = createFrameClock();

    tick(1000);

    expect(tick(1016.7)).toBeCloseTo(0.0167);
  });

  it("clamps long gaps so a stalled frame does not jump the animation", () => {
    const tick = createFrameClock();

    tick(1000);

    expect(tick(5000)).toBe(0.1);
  });

  it("never reports negative time when timestamps go backwards", () => {
    const tick = createFrameClock();

    tick(1000);

    expect(tick(900)).toBe(0);
  });
});
