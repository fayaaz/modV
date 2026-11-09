import { describe, expect, it } from "vitest";
import { seededRand } from "../../src/renderer/src/application/point10/shared/random.js";
import {
  CAMERA,
  CAMERA_MIN_Z,
  CLEAR_ALPHA,
  FBM_OCTAVES,
  FBM_SCALE,
  INNER_POSITION,
  INNER_SCALE,
  MAX_DT,
  MirrorCube,
  OUTER_POSITION,
  PERM_SEED,
  RTT_SIZE,
  buildMirrorCubeFbmShader,
  buildMirrorCubeFrameShader,
  buildMirrorCubeInnerFragmentShader,
  buildMirrorCubeInnerVertexShader,
  buildMirrorCubeOuterFragmentShader,
  buildMirrorCubeOuterVertexShader,
  makePerm,
  mirrorCameraZ,
  mirrorCubeDrive,
  valueNoise2,
} from "../../src/renderer/src/application/point10/scenes/MirrorCube.js";

// Mirror Cube needs WebGL for its 1024x1024 feedback chain, but the rotation
// Expression, the camera zoom, the seeded noise and the shader sources are
// plain arithmetic/strings. Those are what the audio controls, so they are
// tested in isolation.

describe("Mirror Cube expression drive", () => {
  it("starts perm at the seeded per-scene phase and falls back for bad input", () => {
    expect(makePerm()).toBe(PERM_SEED);
    expect(makePerm(4)).toBe(4);
    expect(makePerm(Number.NaN)).toBe(PERM_SEED);
  });

  it("keeps the seeded phase a stable value in [0, 1)", () => {
    expect(PERM_SEED).toBeGreaterThanOrEqual(0);
    expect(PERM_SEED).toBeLessThan(1);
  });

  it("advances perm by beat(0.1) * 0.1 * dt", () => {
    const silent = mirrorCubeDrive(0, 0.1, 1);
    const loud = mirrorCubeDrive(0.5, 0.1, 1);

    expect(silent.perm).toBeCloseTo(1);
    expect(loud.perm).toBeCloseTo(1 + 0.5 * 0.1 * 0.1);
  });

  it("clamps the frame step and sanitises negative / non-finite input", () => {
    const longFrame = mirrorCubeDrive(1, 5, 1);
    const negative = mirrorCubeDrive(-1, -1, Number.NaN);
    const bad = mirrorCubeDrive(Number.NaN, Number.NaN, Number.NaN);

    expect(longFrame.perm).toBeCloseTo(1 + 0.1 * MAX_DT);
    expect(negative.perm).toBeCloseTo(PERM_SEED);
    expect(bad.perm).toBeCloseTo(PERM_SEED);
  });

  it("derives each rotation axis from noise2(perm, n) * 360", () => {
    const drive = mirrorCubeDrive(0.2, 0.1, 2);

    expect(drive.rotation.x).toBeCloseTo(valueNoise2(drive.perm, 0) * 360);
    expect(drive.rotation.y).toBeCloseTo(valueNoise2(drive.perm, 1) * 360);
    expect(drive.rotation.z).toBeCloseTo(valueNoise2(drive.perm, 2) * 360);
  });

  it("scales the beat-driven rotation with spinScale", () => {
    const drive = mirrorCubeDrive(0, 0, 2, 2);

    expect(drive.rotation.x).toBeCloseTo(valueNoise2(2, 0) * 720);
    expect(drive.rotation.y).toBeCloseTo(valueNoise2(2, 1) * 720);
    expect(drive.rotation.z).toBeCloseTo(valueNoise2(2, 2) * 720);
  });
});

describe("Mirror Cube noise and camera", () => {
  it("matches Plane9's value-noise at integer cells", () => {
    expect(valueNoise2(3, 5)).toBeCloseTo(seededRand(3, 5) * 2 - 1);
    expect(valueNoise2(0, 0)).toBeCloseTo(seededRand(0, 0) * 2 - 1);
  });

  it("stays deterministic and inside [-1, 1)", () => {
    const a = valueNoise2(1.25, 7.5);
    const b = valueNoise2(1.25, 7.5);

    expect(a).toBe(b);
    expect(a).toBeGreaterThanOrEqual(-1);
    expect(a).toBeLessThan(1);
  });

  it("zooms the camera in with the mid band and stays outside the cube", () => {
    expect(mirrorCameraZ(0)).toBeCloseTo(CAMERA.z);
    expect(mirrorCameraZ(0.2)).toBeCloseTo(CAMERA.z - 1);
    expect(mirrorCameraZ(5)).toBeCloseTo(CAMERA_MIN_Z);
    expect(mirrorCameraZ(-1)).toBeCloseTo(CAMERA.z);
    expect(mirrorCameraZ(Number.NaN)).toBeCloseTo(CAMERA.z);
  });
});

describe("Mirror Cube preset constants", () => {
  it("renders a 1024x1024 feedback texture and composites transparently", () => {
    expect(RTT_SIZE).toBe(1024);
    expect(CLEAR_ALPHA).toBe(0);
  });

  it("uses the preset's fbm octaves / scale", () => {
    expect(FBM_OCTAVES).toBe(3);
    expect(FBM_SCALE).toBe(10);
  });

  it("uses the preset's camera and cube transforms", () => {
    expect(CAMERA).toMatchObject({ fov: 45, near: 0.1, far: 1000, z: 4.5 });
    expect(INNER_POSITION).toEqual({ x: 0, y: 0, z: 1 });
    expect(OUTER_POSITION).toEqual({ x: 0, y: 0, z: 1.2 });
    expect(INNER_SCALE.x).toBeCloseTo(1.01251);
    expect(INNER_SCALE.y).toBeCloseTo(1.01251);
    expect(INNER_SCALE.z).toBe(1);
  });
});

describe("Mirror Cube scene fields", () => {
  it("defaults the parent-facing inputs to 1 and consumes them", () => {
    const scene = new MirrorCube(null, null);

    expect(scene.spinScale).toBe(1);
    expect(scene.brightnessScale).toBe(1);
    expect(scene.outerMaterial.uniforms.gBrightness.value).toBe(1);

    scene.spinScale = 2;
    scene.brightnessScale = 3;
    scene.update(0, null);

    expect(scene.innerMesh.rotation.y).toBeCloseTo(
      valueNoise2(PERM_SEED, 1) * 720 * (Math.PI / 180),
    );
    expect(scene.outerMesh.rotation.y).toBeCloseTo(scene.innerMesh.rotation.y);
    expect(scene.outerMaterial.uniforms.gBrightness.value).toBe(3);
  });
});

describe("Mirror Cube shaders", () => {
  it("generates the RenderToTexture fbm base", () => {
    const shader = buildMirrorCubeFbmShader();

    expect(shader).toContain("_fbm(vec3(");
    expect(shader).toContain("vec2(10.0)");
    expect(shader).toContain("gTime * 0.1");
    expect(shader).toContain("vec4(v, 1.0)");
  });

  it("draws the edge frame from the preset's smoothstep bands", () => {
    const shader = buildMirrorCubeFrameShader();

    expect(shader).toContain("vec2 p = vTex * vec2(2.0) - vec2(1.0)");
    expect(shader).toContain("smoothstep(0.95, 0.96, abs(p.x))");
    expect(shader).toContain("smoothstep(0.95, 0.96, abs(p.y))");
    expect(shader).toContain("smoothstep(0.04, 0.05, abs(p.x))");
    expect(shader).toContain("smoothstep(0.04, 0.05, abs(p.y))");
  });

  it("lights the inner cube by its world normal and samples the stored RTT", () => {
    const vertex = buildMirrorCubeInnerVertexShader();
    const fragment = buildMirrorCubeInnerFragmentShader();

    expect(vertex).toContain("normalize(mat3(modelMatrix) * normal)");
    expect(fragment).toContain("dot(vec3(0.0, 1.0, 0.0), normal)");
    expect(fragment).toContain("texture2D(gTexture1, vTex)");
    expect(fragment).toContain("* 0.5 * v");
  });

  it("samples the current RTT on the outer cube and applies brightness", () => {
    const vertex = buildMirrorCubeOuterVertexShader();
    const fragment = buildMirrorCubeOuterFragmentShader();

    expect(vertex).toContain("projectionMatrix * modelViewMatrix");
    expect(fragment).toContain("texture2D(gTexture1, vTex)");
    expect(fragment).toContain("gBrightness");
  });

  it("does not use gl_InstanceID", () => {
    expect(buildMirrorCubeFbmShader()).not.toContain("gl_InstanceID");
    expect(buildMirrorCubeFrameShader()).not.toContain("gl_InstanceID");
    expect(buildMirrorCubeInnerVertexShader()).not.toContain("gl_InstanceID");
    expect(buildMirrorCubeInnerFragmentShader()).not.toContain("gl_InstanceID");
    expect(buildMirrorCubeOuterVertexShader()).not.toContain("gl_InstanceID");
    expect(buildMirrorCubeOuterFragmentShader()).not.toContain("gl_InstanceID");
  });
});
