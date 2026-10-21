import { describe, expect, it } from "vitest";
import {
  BLOOM,
  PERMRAND,
  TUNNEL_SIZE,
  buildTunnelFragment,
  makePerm,
  tunnelDrive,
} from "../../src/renderer/src/application/point10/scenes/LightTunnel.js";

// The Light Tunnel scene is a fullscreen fragment shader, so the render itself
// needs WebGL. The expression drive (perm / gIn1), the seeded per-scene random
// and the preset constants are plain arithmetic and are what the music
// actually controls, so they are tested in isolation.
describe("LightTunnel perm drive", () => {
  it("starts perm at the preset's RandomSeed", () => {
    expect(makePerm()).toBe(1);
    expect(makePerm(4)).toBe(4);
  });

  it("advances perm by the bass band and the minimum creep", () => {
    const next = tunnelDrive(0.5, 0, 0.1, 1);

    expect(next.perm).toBeCloseTo(1 + (0.5 * 3 + 0.01) * 0.1);
    expect(next.perm).toBeGreaterThan(1);
  });

  it("creeps forward slowly in silence and stays bounded", () => {
    const next = tunnelDrive(0, 0, 1, 0);

    expect(next.perm).toBeGreaterThan(0);
    expect(next.perm).toBeLessThanOrEqual(0.01);
  });

  it("drives in1.x from perm plus the seeded permrand offset", () => {
    const next = tunnelDrive(0.2, 0, 0.5, 2);

    expect(next.in1.x).toBeCloseTo(next.perm + PERMRAND * 10);
    expect(next.in1.x).toBeGreaterThan(next.perm);
  });

  it("clamps in1.y (band 1) to 1 and ignores negatives", () => {
    expect(tunnelDrive(0, 2, 0.1, 0).in1.y).toBe(1);
    expect(tunnelDrive(0, 0.4, 0.1, 0).in1.y).toBeCloseTo(0.4);
    expect(tunnelDrive(0, -1, 0.1, 0).in1.y).toBe(0);
  });

  it("keeps permrand a stable value in [0, 1)", () => {
    expect(PERMRAND).toBeGreaterThanOrEqual(0);
    expect(PERMRAND).toBeLessThan(1);
  });
});

describe("LightTunnel preset geometry", () => {
  it("uses a fullscreen quad and the preset bloom settings", () => {
    expect(TUNNEL_SIZE).toBe(2);
    expect(BLOOM).toEqual({ strength: 0.17, radius: 0.85, threshold: 0 });
  });

  it("keeps the tunnel shader's key constants", () => {
    const fragment = buildTunnelFragment();

    expect(fragment).toContain("0.25 / r");
    expect(fragment).toContain("_turbulencefast");
    expect(fragment).toContain("0.953, 0.047, 0.090");
    expect(fragment).toContain("500.0");
  });
});
