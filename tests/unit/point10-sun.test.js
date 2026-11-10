import { describe, expect, it } from "vitest";
import {
  SUN,
  buildSunCoronaCompositeFragment,
  buildSunCoronaFragment,
  buildSunScreenFragment,
  buildSunSurfaceFragment,
  buildSunTintFragment,
  sunCoronaLevel,
  sunRayWeight,
  sunSurfaceLevel,
} from "../../src/renderer/src/application/point10/scenes/Sun.js";

// Sun renders through several WebGL passes, but the audio Expressions that
// drive the surface/corona and the preset constants are plain arithmetic.
// They are what the music controls, so they are tested without a GPU.

describe("Sun preset constants", () => {
  it("keeps the Plane9 camera, sphere and god-ray values", () => {
    expect(SUN.fov).toBe(45);
    expect(SUN.near).toBe(0.1);
    expect(SUN.far).toBe(1000);
    expect(SUN.cameraPosition).toEqual([0, 0, 3.8]);
    expect(SUN.cameraLookAt).toEqual([0, 0, -1]);
    expect(SUN.radius).toBe(1);
    expect(SUN.rays.samples).toBe(50);
    expect(SUN.rays.density).toBe(1);
    expect(SUN.rays.weight).toBe(0.2);
    expect(SUN.rays.decay).toBe(0.96);
  });

  it("keeps the sun tint from CopyTexture1", () => {
    expect(SUN.tint).toEqual([0.882, 0.439, 0.024]);
  });
});

describe("Sun surface Expression", () => {
  it("keeps the preset's floor, gain and ceiling", () => {
    expect(sunSurfaceLevel(0)).toBeCloseTo(0.03);
    expect(sunSurfaceLevel(0.05)).toBeCloseTo(0.08);
    expect(sunSurfaceLevel(1)).toBeCloseTo(0.12);
    expect(sunSurfaceLevel(-1)).toBeCloseTo(0.03);
  });

  it("scales with the module input", () => {
    expect(sunSurfaceLevel(0.05, 2)).toBeCloseTo(0.16);
  });
});

describe("Sun corona Expression", () => {
  it("keeps the preset's floor, gain and ceiling", () => {
    expect(sunCoronaLevel(0)).toBeCloseTo(0.1);
    expect(sunCoronaLevel(0.01)).toBeCloseTo(0.3);
    expect(sunCoronaLevel(1)).toBeCloseTo(0.5);
    expect(sunCoronaLevel(-1)).toBeCloseTo(0.1);
  });

  it("scales with the module input", () => {
    expect(sunCoronaLevel(0.01, 2)).toBeCloseTo(0.6);
  });
});

describe("Sun god rays", () => {
  it("scales the preset weight with the module input", () => {
    expect(sunRayWeight()).toBe(0.2);
    expect(sunRayWeight(2)).toBeCloseTo(0.4);
    expect(sunRayWeight(0)).toBe(0);
  });
});

describe("Sun shader sources", () => {
  it("keeps the ridged surface and sunspots", () => {
    const fragment = buildSunSurfaceFragment();

    expect(fragment).toContain("_ridgedmffast");
    expect(fragment).toContain("gTime*0.2");
    expect(fragment).toContain("smoothstep(1.0, 0.6");
  });

  it("keeps the additive corona disc", () => {
    const fragment = buildSunCoronaFragment();

    expect(fragment).toContain("_turbulencefast");
    expect(fragment).toContain("smoothstep(1.0, 0.9, length(p))");
    expect(fragment).toContain("gIn1.x");
  });

  it("keeps the blur + sharp corona composite", () => {
    const fragment = buildSunCoronaCompositeFragment();

    expect(fragment).toContain("c1.rrr*3.0");
    expect(fragment).toContain("c2.rgb*0.9");
    expect(fragment).toContain("c2.a");
  });

  it("keeps the orange tint copy", () => {
    const fragment = buildSunTintFragment();

    expect(fragment).toContain("gColor1*texture2D(gTexture1, vUv).r");
  });

  it("keeps the 50-sample radial god rays", () => {
    const fragment = buildSunScreenFragment();

    expect(fragment).toContain("NUM_SAMPLES = 50");
    expect(fragment).toContain("illuminationDecay *= decay");
    expect(fragment).toContain("c*c");
  });
});
