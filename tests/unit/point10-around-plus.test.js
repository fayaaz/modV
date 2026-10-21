import { describe, expect, it } from "vitest";
import {
  WATER_DRIVE,
  WATER_INNER,
  WATER_LEVEL,
  WATER_OUTER,
  annulusUv,
  buildAroundTorus,
  makePitchWander,
  waterUniforms,
} from "../../src/renderer/src/application/point10/scenes/AroundPlus.js";

describe("AroundPlus waterUniforms", () => {
  it("uses the calm defaults in silence", () => {
    const u = waterUniforms(0, 0, 0.016);

    expect(u.smooth).toBe(0);
    expect(u.dtTime).toBeCloseTo(0.016 * WATER_DRIVE.speed);
    expect(u.distortionScale).toBe(WATER_DRIVE.distortion);
    expect(u.level).toBe(WATER_LEVEL);
  });

  it("attacks quickly and bounds the waterline swell", () => {
    const u = waterUniforms(1, 0, 0.2);

    expect(u.smooth).toBe(1);
    expect(u.dtTime).toBeCloseTo(0.2 * (WATER_DRIVE.speed + WATER_DRIVE.churn));
    expect(u.distortionScale).toBe(WATER_DRIVE.distortion + WATER_DRIVE.shake);
    expect(u.level).toBe(WATER_LEVEL + WATER_DRIVE.swell);
  });

  it("follows the bass through the one-pole smoother", () => {
    const first = waterUniforms(1, 0, 0.01);
    const second = waterUniforms(1, first.smooth, 0.01);

    expect(first.smooth).toBeCloseTo(0.1);
    expect(second.smooth).toBeGreaterThan(first.smooth);
    expect(second.smooth).toBeLessThan(1);
    expect(second.dtTime).toBeGreaterThan(first.dtTime);
  });
});

describe("AroundPlus annulusUv", () => {
  it("maps the annulus seam and radius onto the unit square", () => {
    const start = annulusUv(5, 0);
    const quarter = annulusUv(0, 5);
    const opposite = annulusUv(-5, 0);
    const threeQuarter = annulusUv(0, -5);

    expect(start.x).toBeCloseTo(0);
    expect(quarter.x).toBeCloseTo(0.25);
    expect(opposite.x).toBeCloseTo(0.5);
    expect(threeQuarter.x).toBeCloseTo(0.75);
    expect(start.y).toBeCloseTo(0.5);
    expect(quarter.y).toBeCloseTo(0.5);
  });

  it("clamps the radial coordinate at the tube walls", () => {
    expect(annulusUv(WATER_INNER, 0).y).toBeCloseTo(0);
    expect(annulusUv(WATER_OUTER, 0).y).toBeCloseTo(1);
  });

  it("wraps the angular coordinate into 0..1", () => {
    const uv = annulusUv(5, -0.001);

    expect(uv.x).toBeGreaterThanOrEqual(0);
    expect(uv.x).toBeLessThan(1);
  });
});

describe("AroundPlus buildAroundTorus", () => {
  it("builds the Plane9 torus grid with the inverted winding", () => {
    const geometry = buildAroundTorus();
    const vertexCount = (256 + 1) * (45 + 1);
    const triangleCount = 256 * 45 * 2;

    expect(geometry.getAttribute("position").count).toBe(vertexCount);
    expect(geometry.getAttribute("normal").count).toBe(vertexCount);
    expect(geometry.getAttribute("uv").count).toBe(vertexCount);
    expect(geometry.index.count).toBe(triangleCount * 3);
  });

  it("rescales the uvs for the concrete texture", () => {
    const uv = buildAroundTorus().getAttribute("uv");
    let maxU = 0;
    let maxV = 0;

    for (let i = 0; i < uv.count; i++) {
      maxU = Math.max(maxU, uv.getX(i));
      maxV = Math.max(maxV, uv.getY(i));
    }

    expect(maxU).toBeCloseTo(12.64);
    expect(maxV).toBeCloseTo(33.19);
  });
});

describe("AroundPlus makePitchWander", () => {
  it("stays inside the pitch envelope and moves off the minimum", () => {
    const wander = makePitchWander(-10, 10, 1, 1);
    let time = 0;
    let value = wander(time, 0);

    expect(value).toBe(-10);

    for (let i = 0; i < 600; i++) {
      time += 1 / 60;
      value = wander(time, 1 / 60);

      expect(value).toBeGreaterThanOrEqual(-10);
      expect(value).toBeLessThanOrEqual(10);
    }

    expect(value).not.toBe(-10);
  });
});
