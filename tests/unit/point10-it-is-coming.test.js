import { describe, expect, it } from "vitest";
import {
  BLOOM,
  CLEAR_COLOR,
  INSTANCE_COUNT,
  PERMRAND,
  buildItIsComingFragmentShader,
  buildItIsComingVertexShader,
  itIsComingDrive,
  makePerm,
} from "../../src/renderer/src/application/point10/scenes/ItIsComingForYou.js";

// It Is Coming For You draws 30000 shader points inside a composer chain, so
// the draw itself needs WebGL. The Expression (perm / gIn1 / gIn2), the seeded
// random and the preset constants are plain arithmetic - those are what the
// music controls, so they are tested in isolation.
describe("ItIsComing perm drive", () => {
  it("starts perm at the preset's RandomSeed and falls back for bad input", () => {
    expect(makePerm()).toBe(1);
    expect(makePerm(3)).toBe(3);
    expect(makePerm(Number.NaN)).toBe(1);
  });

  it("advances perm from the bass band with the minimum creep", () => {
    const loud = itIsComingDrive(0.5, 0, 0.1, 1);
    const silent = itIsComingDrive(0, 0, 0.1, 0);

    expect(loud.perm).toBeCloseTo(1 + (0.5 * 20 + 0.1) * 0.1);
    expect(silent.perm).toBeCloseTo(0.01);
  });

  it("caps the spot term (in1.x) at 4", () => {
    expect(itIsComingDrive(0, 0, 0, 1).in1.x).toBe(0);
    expect(itIsComingDrive(0, 0.25, 0, 1).in1.x).toBeCloseTo(1);
    expect(itIsComingDrive(0, 1, 0, 1).in1.x).toBe(4);
    expect(itIsComingDrive(0, 5, 0, 1).in1.x).toBe(4);
  });

  it("clamps the bass glow (in1.y) to 0.25", () => {
    expect(itIsComingDrive(0.1, 0, 0, 1).in1.y).toBeCloseTo(0.1 * 5 * 0.25);
    expect(itIsComingDrive(1, 0, 0, 1).in1.y).toBeCloseTo(0.25);
    expect(itIsComingDrive(9, 0, 0, 1).in1.y).toBe(0.25);
    expect(itIsComingDrive(-1, 0, 0, 1).in1.y).toBe(0);
  });

  it("exposes the seeded phase in in1.z", () => {
    for (const perm of [1, 2.5, 100]) {
      expect(itIsComingDrive(0, 0, 0, perm).in1.z).toBeCloseTo(
        perm + PERMRAND * 10,
      );
    }
    expect(PERMRAND).toBeGreaterThanOrEqual(0);
    expect(PERMRAND).toBeLessThan(1);
  });

  it("keeps the preset's fixed in2 cubic-pulse band", () => {
    const drive = itIsComingDrive(0.2, 0.3, 0.1, 1);

    expect(drive.in2).toEqual({ x: 0.222, y: 0.087, z: 0 });
  });

  it("sanitises negative and non-finite input", () => {
    const negative = itIsComingDrive(-1, -1, -1, Number.NaN);
    const bad = itIsComingDrive(Number.NaN, Number.NaN, Number.NaN, Number.NaN);

    expect(negative.perm).toBe(1);
    expect(negative.in1.x).toBe(0);
    expect(negative.in1.y).toBe(0);
    expect(bad.perm).toBe(1);
  });
});

describe("ItIsComing preset constants", () => {
  it("draws the preset's 30000 points and bloom", () => {
    expect(INSTANCE_COUNT).toBe(30000);
    expect(BLOOM).toEqual({ strength: 1, radius: 0.85, threshold: 0.5 });
  });

  it("uses the preset's dark red clear colour", () => {
    expect(CLEAR_COLOR).toEqual({ r: 0.592, g: 0.075, b: 0.075 });
  });
});

describe("ItIsComing shaders", () => {
  it("keeps the points shader math and instance attribute", () => {
    const vertex = buildItIsComingVertexShader();

    expect(vertex).toContain("attribute float aInstance");
    expect(vertex).toContain("instanceIDFloat / 30000.0");
    expect(vertex).toContain("gl_PointSize = 50.0");
    expect(vertex).toContain("_axisAngleToQuaternion");
    expect(vertex).toContain("_hsv2rgb");
    expect(vertex).not.toContain("gl_VertexID");
  });

  it("contains the lighting, cubic pulse and blur terms", () => {
    const fragment = buildItIsComingFragmentShader();

    expect(fragment).toContain("_lightLambert");
    expect(fragment).toContain("_cubicpulse");
    expect(fragment).toContain("c*5.0");
    expect(fragment).toContain("gl_PointCoord");
  });
});
