import { describe, expect, it } from "vitest";
import {
  BLOOM,
  CAMERA,
  PERMRAND,
  POINT_COUNT,
  POINT_DIST,
  POINT_SIZE,
  SPEED,
  buildWormholeFragmentShader,
  buildWormholeVertexShader,
  makePerm,
  wormholeDrive,
} from "../../src/renderer/src/application/point10/scenes/Wormhole.js";

// Wormhole is 100k additive points driven by one Expression, so the draw needs
// WebGL. The Expression (perm / gIn1), the seeded random and the preset
// constants are plain arithmetic - those are what the audio controls, so they
// are tested in isolation.
describe("Wormhole perm drive", () => {
  it("starts perm at the preset's RandomSeed and falls back for bad input", () => {
    expect(makePerm()).toBe(1);
    expect(makePerm(4)).toBe(4);
    expect(makePerm(Number.NaN)).toBe(1);
  });

  it("advances perm by the bass band plus the minimum creep", () => {
    const next = wormholeDrive(0.05, 0.5, 1);

    expect(next.perm).toBeCloseTo(1 + (0.05 * 25 + 0.25) * 0.5);
    expect(next.perm).toBeGreaterThan(1);
  });

  it("caps the bass advance at 2 units per second", () => {
    const loud = wormholeDrive(1, 0.5, 1);
    const louder = wormholeDrive(10, 0.5, 1);

    expect(loud.perm).toBeCloseTo(1 + 2 * 0.5);
    expect(louder.perm).toBeCloseTo(loud.perm);
    expect(wormholeDrive(1, 1, 1).perm).toBeCloseTo(3);
  });

  it("creeps forward in silence and ignores negative bass / dt", () => {
    const silent = wormholeDrive(0, 0.1, 0);
    const negative = wormholeDrive(-1, -1, 1);
    const bad = wormholeDrive(Number.NaN, Number.NaN, Number.NaN);

    expect(silent.perm).toBeCloseTo(0.25 * 0.1);
    expect(negative.perm).toBeCloseTo(1);
    expect(bad.perm).toBeCloseTo(0);
  });

  it("writes in1.x from permrand and in1.z from the advanced perm", () => {
    const start = wormholeDrive(0, 0, 0);
    const next = wormholeDrive(0.2, 0.5, 2);

    expect(start.in1.x).toBeCloseTo(PERMRAND * 100);
    expect(next.in1.x).toBeCloseTo(PERMRAND * 100);
    expect(next.in1.z).toBeCloseTo(next.perm + PERMRAND * 10);
    expect(next.in1.y).toBe(0);
  });

  it("keeps permrand a stable value in [0, 1)", () => {
    expect(PERMRAND).toBeGreaterThanOrEqual(0);
    expect(PERMRAND).toBeLessThan(1);
  });
});

describe("Wormhole preset constants", () => {
  it("uses the preset's 100k additive points", () => {
    expect(POINT_COUNT).toBe(100000);
    expect(POINT_SIZE).toBe(5);
    expect(POINT_DIST).toBe(300);
    expect(SPEED).toBe(0.5);
  });

  it("uses the preset bloom and camera settings", () => {
    expect(BLOOM).toEqual({ strength: 0.1, radius: 0.85, threshold: 0 });
    expect(CAMERA).toEqual({ fov: 60, near: 0.01, far: 50 });
  });
});

describe("Wormhole shaders", () => {
  it("keeps the wormhole vertex math verbatim", () => {
    const vertex = buildWormholeVertexShader();

    expect(vertex).toContain("attribute float aId");
    expect(vertex).toContain("float d = aId / 100000.0");
    expect(vertex).toContain("float t = fract(d - gIn1.z * 0.01)");
    expect(vertex).toContain("pointDist");
    expect(vertex).toContain("300.0");
    expect(vertex).toContain("_rand(vec2(paramSet, 0.123))");
    expect(vertex).toContain("_hsv2rgb(l, 0.75 + k * 0.25, 0.9)");
    expect(vertex).toContain("smoothstep(0.1, 0.2, length(nv.xy))");
    expect(vertex).toContain("gl_PointSize = 5.0");
  });

  it("keeps the fragment's round additive point falloff", () => {
    const fragment = buildWormholeFragmentShader();

    expect(fragment).toContain("gl_PointCoord * 2.0 - 1.0");
    expect(fragment).toContain("smoothstep(1.0, 0.9, length(p))");
    expect(fragment).toContain("saturate(");
    expect(fragment).toContain("vDiffuse.a");
  });
});
