import { describe, expect, it } from "vitest";
import {
  BLOOM,
  CAMERA,
  CUBE_SIZE,
  INSTANCE_COUNT,
  PERMRAND,
  ROUGHNESS_FLOOR,
  boxyDrive,
  buildBoxyFragmentShader,
  buildBoxyVertexShader,
  makePerm,
} from "../../src/renderer/src/application/point10/scenes/Boxy.js";

// Boxy renders 1000 instanced cubes with a custom shader, so the draw needs
// WebGL. The Expression (perm / gIn1 / gIn2), the seeded random and the preset
// constants are plain arithmetic - those are what the audio controls, so they
// are tested in isolation.
describe("Boxy perm drive", () => {
  it("starts perm at the preset's RandomSeed and falls back for bad input", () => {
    expect(makePerm()).toBe(1);
    expect(makePerm(4)).toBe(4);
    expect(makePerm(Number.NaN)).toBe(1);
  });

  it("maps the bass band to in1.x and the mid band to in1.y", () => {
    const drive = boxyDrive(0.05, 0.02, 0.5, 1);

    expect(drive.in1.x).toBeCloseTo(1);
    expect(drive.in1.y).toBeCloseTo(0.4);
  });

  it("advances perm from the bass band, capped at 2 units per second", () => {
    const quiet = boxyDrive(0.05, 0, 0.1, 1);
    const loud = boxyDrive(1, 0, 0.1, 0);
    const louder = boxyDrive(10, 0, 0.1, 0);

    expect(quiet.perm).toBeCloseTo(1 + Math.min(2, 0.05 * 20 + 0.2) * 0.1);
    expect(loud.perm).toBeCloseTo(0.2);
    expect(louder.perm).toBeCloseTo(loud.perm);
  });

  it("creeps forward in silence and sanitises negative / non-finite input", () => {
    const silent = boxyDrive(0, 0, 0.1, 0);
    const negative = boxyDrive(-1, -2, -1, Number.NaN);
    const bad = boxyDrive(Number.NaN, Number.NaN, Number.NaN, Number.NaN);

    expect(silent.perm).toBeCloseTo(0.2 * 0.1);
    expect(negative.perm).toBeCloseTo(1);
    expect(negative.in1.x).toBe(0);
    expect(negative.in1.y).toBe(0);
    expect(bad.perm).toBeCloseTo(1);
  });

  it("exposes the seeded phase in in1.z and the 0.3 roughness floor", () => {
    const drive = boxyDrive(0, 0, 0, 3);

    expect(drive.in1.z).toBeCloseTo(3 + PERMRAND * 10);
    expect(drive.in2).toEqual({ x: 0.3, y: 0, z: 0 });
    expect(drive.in2.x).toBe(ROUGHNESS_FLOOR);
  });

  it("keeps permrand a stable value in [0, 1)", () => {
    expect(PERMRAND).toBeGreaterThanOrEqual(0);
    expect(PERMRAND).toBeLessThan(1);
  });
});

describe("Boxy preset constants", () => {
  it("uses the preset's 1000 instanced unit cubes", () => {
    expect(INSTANCE_COUNT).toBe(1000);
    expect(CUBE_SIZE).toBe(1);
  });

  it("uses the preset's camera and bloom settings", () => {
    expect(CAMERA).toEqual({ fov: 70, near: 0.01, far: 100 });
    expect(BLOOM).toEqual({ strength: 1, radius: 0.85, threshold: 0 });
  });
});

describe("Boxy shaders", () => {
  it("places every cube from aInstance instead of gl_InstanceID", () => {
    const vertex = buildBoxyVertexShader();

    expect(vertex).toContain("attribute float aInstance");
    expect(vertex).toContain("float d = aInstance / 1000.0");
    expect(vertex).toContain("_rand3(d)");
    expect(vertex).toContain("_rand3(aInstance * 0.825931)");
    expect(vertex).toContain("mod(d * 20.0 - gIn1.z, 20.0)");
    expect(vertex).toContain("projectionMatrix * modelViewMatrix");
    expect(vertex).not.toContain("gl_InstanceID");
  });

  it("keeps the fragment's lighting, environment and fog math", () => {
    const fragment = buildBoxyFragmentShader();

    expect(fragment).toContain("_palette");
    expect(fragment).toContain("_lightDirectional");
    expect(fragment).toContain("_fresnelRoughness");
    expect(fragment).toContain("textureLod(gTexture2");
    expect(fragment).toContain("smoothstep(0.91, 0.911");
    expect(fragment).toContain("simpleFog");
    expect(fragment).toContain("gIn2.x");
  });

  it("builds a tangent frame so the normal map works without tangents", () => {
    const fragment = buildBoxyFragmentShader();

    expect(fragment).toContain("cross(normalSurf");
    expect(fragment).toContain("normalTex.x * tangent");
    expect(fragment).toContain("normalTex.y * bitangent");
  });
});
