import * as THREE from "three";
import { describe, expect, it } from "vitest";
import {
  BLOOM,
  BLOOM_SMOOTHNESS,
  CAMERA,
  CLEAR_ALPHA,
  CUBE_SIZE,
  FBM_OCTAVES,
  FBM_SIZE,
  HollowCube,
  INSTANCE_COUNT,
  LATTICE_COUNT,
  buildHollowCubeFragmentShader,
  buildHollowCubeFbmFragment,
  buildHollowCubeGeometry,
  buildHollowCubeShader2Fragment,
  buildHollowCubeVertexShader,
  hollowCubeDrive,
  hollowCubePos,
  makePerm,
} from "../../src/renderer/src/application/point10/scenes/HollowCube.js";

// The scene draws 15000 instanced cubes, so rendering needs WebGL. The
// Expression drive, the hollow-lattice mapping and the shader constants are
// plain arithmetic/strings and are what the music and the preset geometry
// control, so they are tested in isolation.

describe("HollowCube drive", () => {
  it("starts perm at the preset's RandomSeed and falls back for bad input", () => {
    expect(makePerm()).toBe(1);
    expect(makePerm(4)).toBe(4);
    expect(makePerm(Number.NaN)).toBe(1);
  });

  it("accumulates perm from the bass band plus the 0.01 creep", () => {
    const quiet = hollowCubeDrive(0, 0, 0, 0.1, 1);
    const loud = hollowCubeDrive(0.1, 0, 0, 0.1, 0);
    const louder = hollowCubeDrive(5, 0, 0, 0.1, 0);

    expect(quiet.perm).toBeCloseTo(1 + (0 + 0.01) * 40 * 0.1);
    expect(loud.perm).toBeCloseTo((0.1 + 0.01) * 40 * 0.1);
    // min(0.5, band + 0.01) * 40 caps the phase advance at 20 units/second.
    expect(louder.perm).toBeCloseTo(0.5 * 40 * 0.1);
  });

  it("maps the damped bass lanes to explosion, colour/rotation/size", () => {
    const drive = hollowCubeDrive(0, 0.1, 0.5, 0.1, 0);

    expect(drive.in1.x).toBeCloseTo(Math.min(1.2, 0.5 * 2.2) * 0.6);
    expect(drive.in1.y).toBeCloseTo(Math.min(0.55, 0.1 * 4 + 0.4));
    expect(drive.in1.z).toBeCloseTo(Math.min(1, 0.1 * 1.5));
    expect(drive.in2.x).toBeCloseTo(drive.perm);
    expect(drive.in2.y).toBeCloseTo(1 + Math.min(0.2, 0.1));
  });

  it("clamps the colour/size/rotation terms to the preset maxima", () => {
    const drive = hollowCubeDrive(10, 10, 10, 0.1, 0);

    expect(drive.in1.x).toBeCloseTo(0.72);
    expect(drive.in1.y).toBeCloseTo(0.55);
    expect(drive.in1.z).toBeCloseTo(1);
    expect(drive.in2.y).toBeCloseTo(1.2);
  });

  it("sanitises negative, non-finite and oversized frame times", () => {
    const negative = hollowCubeDrive(-1, -2, -3, -1, Number.NaN);
    const bad = hollowCubeDrive(
      Number.NaN,
      Number.NaN,
      Number.NaN,
      Number.NaN,
      Number.NaN,
    );
    const longFrame = hollowCubeDrive(1, 1, 1, 10, 0);

    expect(negative.perm).toBeCloseTo(1 + 0.01 * 40 * 0);
    expect(negative.in1).toEqual({ x: 0, y: 0.4, z: 0 });
    expect(bad.perm).toBeCloseTo(1);
    // P9 deltatime is clamped, so a suspended tab cannot skip the pulse.
    expect(longFrame.perm).toBeCloseTo(0.5 * 40 * 0.1);
  });
});

describe("HollowCube lattice", () => {
  it("uses the preset's 15000 instances on a 50x50x50 shell", () => {
    expect(INSTANCE_COUNT).toBe(15000);
    expect(LATTICE_COUNT).toBe(50);
    expect(CUBE_SIZE).toBe(0.01);
  });

  it("maps instance ids onto the six faces of the lattice", () => {
    const perFace = LATTICE_COUNT * LATTICE_COUNT;

    expect(hollowCubePos(0)).toEqual({ x: 0, y: 0, z: 0 });
    expect(hollowCubePos(1)).toEqual({
      x: 0,
      y: 1 / LATTICE_COUNT,
      z: 0,
    });
    expect(hollowCubePos(perFace)).toEqual({ x: 1, y: 0, z: 0 });
    expect(hollowCubePos(perFace * 4)).toEqual({ x: 0, y: 0, z: 0 });
    expect(hollowCubePos(INSTANCE_COUNT - 1)).toEqual({
      x: (LATTICE_COUNT - 1) / LATTICE_COUNT,
      y: (LATTICE_COUNT - 1) / LATTICE_COUNT,
      z: 1,
    });
  });

  it("keeps every instance on the shell of the lattice", () => {
    for (let i = 0; i < INSTANCE_COUNT; i += 137) {
      const pos = hollowCubePos(i);

      expect(
        pos.x === 0 ||
          pos.x === 1 ||
          pos.y === 0 ||
          pos.y === 1 ||
          pos.z === 0 ||
          pos.z === 1,
      ).toBe(true);
    }
  });

  it("builds one instanced buffer with a float instance attribute", () => {
    const geometry = buildHollowCubeGeometry();
    const ids = geometry.getAttribute("aInstance");

    expect(ids).toBeTruthy();
    expect(ids.count).toBe(INSTANCE_COUNT);
    expect(ids.itemSize).toBe(1);
    expect(ids.array[0]).toBe(0);
    expect(ids.array[INSTANCE_COUNT - 1]).toBe(INSTANCE_COUNT - 1);
  });
});

describe("HollowCube transparency and preset constants", () => {
  it("clears to a fully transparent background", () => {
    expect(CLEAR_ALPHA).toBe(0);
  });

  it("uses the preset's camera", () => {
    expect(CAMERA).toEqual({
      fov: 50,
      near: 0.01,
      far: 100,
      position: [0, 0, 2.2],
      lookAt: [0, 0, -1],
    });
  });

  it("uses the preset's bloom and 256px fbm capture", () => {
    expect(BLOOM).toEqual({ strength: 0.51, radius: 0.85, threshold: 0 });
    expect(BLOOM_SMOOTHNESS).toBe(0.5);
    expect(FBM_SIZE).toBe(256);
    expect(FBM_OCTAVES).toBe(3);
  });

  it("initialises the wireable scene controls to one", () => {
    const scene = new HollowCube(null, null);

    expect(scene.spinScale).toBe(1);
    expect(scene.explodeScale).toBe(1);
    expect(scene.colorScale).toBe(1);
    expect(scene.bloomScale).toBe(1);
  });
});

describe("HollowCube shaders", () => {
  it("positions every cube from aInstance instead of gl_InstanceID", () => {
    const vertex = buildHollowCubeVertexShader();

    expect(vertex).toContain("attribute float aInstance");
    expect(vertex).toContain("float instanceIdFloat = aInstance");
    expect(vertex).toContain("float objCount = 50.0");
    expect(vertex).toContain("mod(instanceIdFloat, objCount)");
    expect(vertex).toContain("mod(instanceIdFloat / objCount, objCount)");
    expect(vertex).toContain("_cubicpulse");
    expect(vertex).toContain("_axisAngleToQuaternion");
    expect(vertex).toContain("_rotateVector");
    expect(vertex).toContain("gIn1.x");
    expect(vertex).toContain("gIn2.y");
    expect(vertex).toContain("projectionMatrix * modelViewMatrix");
    expect(vertex).not.toContain("gl_InstanceID");
  });

  it("keeps the preset's half-Lambert/Blinn-Phong lighting", () => {
    const fragment = buildHollowCubeFragmentShader();

    expect(fragment).toContain("_lightBlinnPhongHalfLambert");
    expect(fragment).toContain("specularHardness = 10.0");
    expect(fragment).toContain("vec3(0.5, 0.8, 1.0)");
    expect(fragment).toContain("vec3(0.8, 0.0, 0.0)");
    expect(fragment).toContain("gIn1.y");
  });

  it("renders the 256x256 fbm capture with the preset's scale and speed", () => {
    const fbm = buildHollowCubeFbmFragment();

    expect(fbm).toContain("_fbm");
    expect(fbm).toContain("vec2(10.0)");
    expect(fbm).toContain("gTime * 0.1");
    expect(fbm).toContain("3.0");
  });

  it("composites bloom, capture and background with the Shader2 formula", () => {
    const shader = buildHollowCubeShader2Fragment();

    expect(shader).toContain("mix(bg.rgb, _tonemapACES(c2.rgb), c2.a)");
    expect(shader).toContain("_tonemapACES(c1.rgb * c1.rgb)");
    expect(shader).toContain("texture2D(tCube");
    expect(shader).toContain("texture2D(tBloom");
    expect(shader).toContain("texture2D(tFbm");
  });
});

describe("HollowCube instancing", () => {
  it("draws all instances from a single InstancedMesh", () => {
    const scene = new HollowCube(null, null);

    expect(scene.mesh.isInstancedMesh).toBe(true);
    expect(scene.mesh.count).toBe(INSTANCE_COUNT);
    expect(scene.mesh.frustumCulled).toBe(false);
    expect(scene.mesh.geometry.getAttribute("aInstance").count).toBe(
      INSTANCE_COUNT,
    );
    expect(scene.mesh.instanceMatrix).toBeInstanceOf(
      THREE.InstancedBufferAttribute,
    );
  });
});
