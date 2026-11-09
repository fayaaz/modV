import * as THREE from "three";
import { describe, expect, it } from "vitest";
import {
  BLOOM,
  CAMERA,
  CLEAR_ALPHA,
  CUBE_SIZE,
  DISCARD_MAX,
  GRID_SIZE,
  INSTANCE_COUNT,
  LIGHT_POSITION,
  ROTATOR,
  ColorCube,
  buildColorCubeFragmentShader,
  buildColorCubeVertexShader,
  cloneColor,
  clonePosition,
  colorCubeDrive,
  computeCloneGrid,
  makePerm,
} from "../../src/renderer/src/application/point10/scenes/ColorCube.js";

// Color Cube renders 512 instanced cubes with a custom shader, so the draw
// needs WebGL. The CloneExpression positions/colours, the Expression drive and
// the preset constants are plain arithmetic and are tested directly.
describe("ColorCube clone expression", () => {
  it("uses the preset's 8x8x8 grid of 0.05 cubes", () => {
    expect(GRID_SIZE).toBe(8);
    expect(INSTANCE_COUNT).toBe(512);
    expect(CUBE_SIZE).toBe(0.05);
  });

  it("maps clone indices to the preset's 1-based grid colours", () => {
    expect(cloneColor(1, 1, 1)).toEqual({ r: 0.125, g: 0.125, b: 0.875 });
    expect(cloneColor(8, 8, 8)).toEqual({ r: 1, g: 1, b: 0 });
    expect(cloneColor(4, 6, 2)).toEqual({ r: 0.5, g: 0.75, b: 0.75 });
  });

  it("places a clone from its grid coordinates, spread and perm phase", () => {
    const flat = clonePosition(2, 3, 4, 0, 0);

    expect(flat.x).toBeCloseTo(0.1);
    expect(flat.y).toBeCloseTo(0.15);
    expect(flat.z).toBeCloseTo(0.2);

    const spread = clonePosition(2, 3, 4, 1, 0);
    const s = 1 * 0.025;

    expect(spread.x).toBeCloseTo(2 * 2 * 0.05 + Math.sin(2 - 3 - 4) * s);
    expect(spread.y).toBeCloseTo(3 * 2 * 0.05 + Math.sin(2 + 3 + 4) * s);
    expect(spread.z).toBeCloseTo(4 * 2 * 0.05 + Math.sin(3 * 4 * 5) * s);
  });

  it("centres the 512-instance grid on the origin", () => {
    const grid = computeCloneGrid(0, 0);

    expect(grid.count).toBe(INSTANCE_COUNT);
    expect(grid.positions).toHaveLength(INSTANCE_COUNT * 3);
    expect(grid.colors).toHaveLength(INSTANCE_COUNT * 3);
    // Clone coordinates are 1-based, so the first cube is (1, 1, 1) and the
    // last is (8, 8, 8); Center subtracts the bounding-box midpoint.
    expect(grid.positions[0]).toBeCloseTo(-0.175);
    expect(grid.positions[1]).toBeCloseTo(-0.175);
    expect(grid.positions[2]).toBeCloseTo(-0.175);
    expect(grid.positions[(INSTANCE_COUNT - 1) * 3]).toBeCloseTo(0.175);
    expect(grid.center.x).toBeCloseTo(0.225);
    expect(Array.from(grid.colors.slice(0, 3))).toEqual([0.125, 0.125, 0.875]);
    expect(Array.from(grid.colors.slice((INSTANCE_COUNT - 1) * 3))).toEqual([
      1, 1, 0,
    ]);
  });

  it("spreads the grid further as the audio dist grows", () => {
    const span = (grid) => {
      let min = Infinity;
      let max = -Infinity;

      for (let i = 0; i < grid.positions.length; i += 3) {
        min = Math.min(min, grid.positions[i]);
        max = Math.max(max, grid.positions[i]);
      }

      return max - min;
    };

    expect(span(computeCloneGrid(0, 1))).toBeGreaterThan(
      span(computeCloneGrid(0, 0)),
    );
  });
});

describe("ColorCube drive", () => {
  it("starts perm at the preset's RandomSeed and falls back for bad input", () => {
    expect(makePerm()).toBe(1);
    expect(makePerm(4)).toBe(4);
    expect(makePerm(Number.NaN)).toBe(1);
  });

  it("advances perm from the beat envelope", () => {
    const drive = colorCubeDrive(0, 0, 1, 0.05, 1);

    expect(drive.perm).toBeCloseTo(1 + (50 + 0.01) * 0.05);
  });

  it("caps deltatime at the Plane9 expression's 0.05s", () => {
    const caughtUp = colorCubeDrive(0, 0, 0, 0.5, 1);

    expect(caughtUp.perm).toBeCloseTo(1 + 0.01 * 0.05);
  });

  it("scales the beat turbulence with turbulenceScale", () => {
    const quarter = colorCubeDrive(0, 0, 1, 0.05, 1, 0.25);

    expect(quarter.perm).toBeCloseTo(1 + (12.5 + 0.01) * 0.05);
  });

  it("derives dist from both bass bands and the discard from the damped band", () => {
    const drive = colorCubeDrive(0.5, 0.5, 0, 0.05, 1);

    expect(drive.dist).toBeCloseTo((0.3 + 0.5) * 0.5 * 20);
    expect(drive.discard).toBeCloseTo(DISCARD_MAX);
    expect(drive.in1.x).toBe(DISCARD_MAX);
    expect(drive.in1.y).toBe(LIGHT_POSITION[1]);
    expect(drive.in1.z).toBe(LIGHT_POSITION[2]);
  });

  it("can push the discard past the preset cap with turbulenceScale", () => {
    const drive = colorCubeDrive(0, 1, 0, 0, 1, 2);

    expect(drive.discard).toBeCloseTo(DISCARD_MAX * 2);
  });

  it("creeps forward in silence and sanitises negative / non-finite input", () => {
    const silent = colorCubeDrive(0, 0, 0, 0.05, 0);

    expect(silent.perm).toBeCloseTo(0.01 * 0.05);

    const negative = colorCubeDrive(-1, -2, -3, -1, Number.NaN);

    expect(negative.perm).toBeCloseTo(1);
    expect(negative.dist).toBe(0);
    expect(negative.discard).toBe(0);

    const bad = colorCubeDrive(
      Number.NaN,
      Number.NaN,
      Number.NaN,
      Number.NaN,
      Number.NaN,
      Number.NaN,
    );

    expect(bad.perm).toBeCloseTo(1);
    expect(bad.dist).toBe(0);
    expect(bad.discard).toBe(0);
  });
});

describe("ColorCube shaders", () => {
  it("places every cube with the instance matrix and a per-instance colour", () => {
    const vertex = buildColorCubeVertexShader();

    expect(vertex).toContain("attribute vec3 aColor");
    expect(vertex).toContain("instanceMatrix");
    expect(vertex).toContain("projectionMatrix");
    expect(vertex).not.toContain("gl_InstanceID");
  });

  it("keeps the preset's lighting and turbulence discard", () => {
    const fragment = buildColorCubeFragmentShader();

    expect(fragment).toContain("_turbulencefast(vWorldPos * 2.0, 5.0)");
    expect(fragment).toContain("discard");
    expect(fragment).toContain("_lightDirectional");
    expect(fragment).toContain("gIn1.x");
    expect(fragment).toContain("uBrightness");
    expect(fragment).toContain("vec3(1.0, 0.5, 0.3) * 50.0");
  });
});

describe("ColorCube scene", () => {
  it("uses one InstancedMesh of 512 cubes with the preset camera and bloom", () => {
    expect(CAMERA).toEqual({
      fov: 70,
      near: 0.01,
      far: 100,
      position: [0, 0, -1.17],
      lookAt: [0, 0, 1],
    });
    expect(ROTATOR).toEqual({ x: 1.5, y: 1, z: 0.5 });
    expect(BLOOM).toEqual({ strength: 0.5, radius: 0.85, threshold: 0 });
    expect(CLEAR_ALPHA).toBe(0);
    expect(LIGHT_POSITION).toEqual([0, 14.945, -3.827]);

    const scene = new ColorCube(null, null);

    expect(scene.mesh).toBeInstanceOf(THREE.InstancedMesh);
    expect(scene.mesh.count).toBe(INSTANCE_COUNT);
  });

  it("initialises the scene fields to 1 and consumes them in update", () => {
    const scene = new ColorCube(null, null);

    expect(scene.spinScale).toBe(1);
    expect(scene.turbulenceScale).toBe(1);
    expect(scene.bloomScale).toBe(1);
    expect(scene.brightnessScale).toBe(1);

    const start = scene.mesh.rotation.x;

    scene.spinScale = 2;
    scene.update(0.1, null);

    expect(scene.mesh.rotation.x).toBeGreaterThan(start);
    expect(scene.perm).toBeGreaterThan(1);

    scene.brightnessScale = 2;
    scene.update(0.1, null);

    expect(scene.material.uniforms.uBrightness.value).toBe(2);
  });
});
