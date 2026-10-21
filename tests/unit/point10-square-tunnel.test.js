import { describe, expect, it } from "vitest";
import {
  PERMRAND,
  TUNNEL_SIZE,
  buildSquareTunnelFragment,
  makePerm,
  squareTunnelDrive,
} from "../../src/renderer/src/application/point10/scenes/SquareTunnel.js";

// Square Tunnel is a single fullscreen shader, so the draw itself needs WebGL.
// The Expression that feeds it (perm / gIn1) and the preset constants are
// plain arithmetic - those are what the music controls, so they are tested in
// isolation.
describe("SquareTunnel perm drive", () => {
  it("starts perm at the preset's RandomSeed and falls back for bad input", () => {
    expect(makePerm()).toBe(1);
    expect(makePerm(4)).toBe(4);
    expect(makePerm(Number.NaN)).toBe(1);
    expect(squareTunnelDrive(0, 0, 0.1, Number.NaN).perm).toBeCloseTo(1.001);
  });

  it("advances perm by bass * 20 plus the minimum creep", () => {
    const next = squareTunnelDrive(0.5, 0, 0.1, 1);

    expect(next.perm).toBeCloseTo(1 + (0.5 * 20 + 0.01) * 0.1);
    expect(next.perm).toBeGreaterThan(1);
  });

  it("creeps forward in silence and ignores negative bass / dt", () => {
    const silent = squareTunnelDrive(0, 0, 0.1, 0);
    const longFrame = squareTunnelDrive(0, 0, 1, 0);
    const negative = squareTunnelDrive(-2, 0, -1, 1);

    expect(silent.perm).toBeCloseTo(0.001);
    // long frames are capped at 0.1s like the P9 deltatime
    expect(longFrame.perm).toBeCloseTo(silent.perm);
    expect(negative.perm).toBeCloseTo(1);
  });

  it("drives in1.x from beat(0) * 2, clamped to the P9 beat envelope", () => {
    expect(squareTunnelDrive(0, 0.75, 0.1, 0).in1.x).toBeCloseTo(1.5);
    expect(squareTunnelDrive(0, 1, 0.1, 0).in1.x).toBe(2);
    expect(squareTunnelDrive(0, -1, 0.1, 0).in1.x).toBe(0);
    expect(squareTunnelDrive(0, Number.NaN, 0.1, 0).in1.x).toBe(0);
  });

  it("writes in1.y as the advanced perm plus the seeded permrand offset", () => {
    const next = squareTunnelDrive(0.2, 0.5, 0.5, 2);

    expect(next.in1.y).toBeCloseTo(next.perm + PERMRAND * 10);
    expect(next.in1.x).toBeCloseTo(1);
  });

  it("keeps permrand a stable value in [0, 1)", () => {
    expect(PERMRAND).toBeGreaterThanOrEqual(0);
    expect(PERMRAND).toBeLessThan(1);
  });
});

describe("SquareTunnel preset constants", () => {
  it("uses a fullscreen quad", () => {
    expect(TUNNEL_SIZE).toBe(2);
  });

  it("keeps the shader's power-15 tunnel mapping", () => {
    const fragment = buildSquareTunnelFragment();

    expect(fragment).toContain("float power = 15.0");
    expect(fragment).toContain("uv.x = 0.3 * gIn1.y + 0.5 / r");
    expect(fragment).toContain("uv.y = a / PI");
    expect(fragment).toContain("texture(gTexture1, uv * 3.0)");
    expect(fragment).toContain("col * r * (0.5 + gIn1.x) * length(p)");
  });
});
