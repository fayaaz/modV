import { describe, expect, it, vi } from "vitest";
import {
  CLEAR_ALPHA,
  applyWaterRippleClear,
  buildWaterRippleFragmentShader,
  waterRippleDrive,
} from "../../src/renderer/src/application/point10/scenes/WaterRipple.js";

describe("WaterRipple drive", () => {
  it("uses the seeded silence defaults", () => {
    expect(waterRippleDrive()).toEqual({
      distortion: 0.55,
      speed: 1.3,
      radius: 0.9,
      brightness: 0.68,
      dropStrength: 0.012,
    });
  });

  it("lets bass and the phrase pulse lift the pattern", () => {
    const drive = waterRippleDrive(0.5, 0.25);

    expect(drive.distortion).toBeCloseTo(0.55 + 0.5 * 1.1 + 0.25 * 0.55);
    expect(drive.brightness).toBeCloseTo(0.68 + 0.5 * 0.5 + 0.25 * 0.2);
  });

  it("clamps non-finite and out-of-range input", () => {
    expect(waterRippleDrive(-1, -1)).toEqual(waterRippleDrive(0, 0));
    expect(waterRippleDrive(Number.NaN, Number.NaN)).toEqual(
      waterRippleDrive(0, 0),
    );
    expect(waterRippleDrive(10, 10)).toEqual(waterRippleDrive(1, 1));
  });
});

describe("WaterRipple transparency", () => {
  it("clears to a fully transparent background", () => {
    expect(CLEAR_ALPHA).toBe(0);

    const renderer = { setClearColor: vi.fn() };

    expect(applyWaterRippleClear(renderer)).toBe(renderer);
    expect(renderer.setClearColor).toHaveBeenCalledWith(0x000000, 0);
    expect(applyWaterRippleClear(null)).toBe(null);
  });

  it("writes the pattern colour with inCircle as its alpha", () => {
    const fragment = buildWaterRippleFragmentShader();

    // The area outside the circular pattern must be transparent instead of a
    // hard-coded near-black backdrop.
    expect(fragment).not.toContain("vec3(0.02, 0.02, 0.05)");
    expect(fragment).not.toContain("mix(background, color, inCircle)");
    expect(fragment).toContain("gl_FragColor = vec4(color, inCircle)");
    // The soft edge still comes from inCircle's smoothstep.
    expect(fragment).toContain(
      "1.0 - smoothstep(circleRadius - 0.1, circleRadius + 0.1",
    );
  });
});
