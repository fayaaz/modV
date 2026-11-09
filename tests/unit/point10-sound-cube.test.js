import { describe, expect, it } from "vitest";
import {
  BLUR,
  CAMERA,
  CLEAR_ALPHA,
  CUBE_SIZE,
  ROTATOR,
  RTT_SIZE,
  SCALE_AUDIO,
  SCALE_BASE,
  SoundCube,
  TRAIL_BRIGHTNESS,
  TRAIL_ZOOM,
  advanceSoundCubeRotation,
  buildSoundCubeBlurFragment,
  buildSoundCubeCompositeFragment,
  buildSoundCubeFragmentShader,
  buildSoundCubeTrailFragment,
  buildSoundCubeVertexShader,
  soundCubeDrive,
  soundCubeFeedback,
} from "../../src/renderer/src/application/point10/scenes/SoundCube.js";

// Sound Cube is a rotating cube captured into a 256x256 feedback buffer, so
// the draw itself needs WebGL. The Expression (bass -> cube scale), the
// rotator, the trail persistence and the preset constants are plain
// arithmetic - those are what the music and the scene inputs drive, so they
// are tested in isolation.
const DEG = Math.PI / 180;

describe("SoundCube Expression1 drive", () => {
  it("maps the smoothed bass band to 0.3 + band * 8", () => {
    expect(soundCubeDrive(0, 1).scale).toBeCloseTo(SCALE_BASE);
    expect(soundCubeDrive(0.05, 1).scale).toBeCloseTo(SCALE_BASE + 0.4);
    expect(soundCubeDrive(0.5, 1).scale).toBeCloseTo(0.3 + 4);
  });

  it("returns the raw band and the preset expression separately", () => {
    const drive = soundCubeDrive(0.1, 1);

    expect(drive.band).toBeCloseTo(0.1);
    expect(drive.expression).toBeCloseTo(SCALE_BASE + 0.1 * SCALE_AUDIO);
    expect(drive.scale).toBeCloseTo(drive.expression);
  });

  it("applies pulseScale on top of the expression", () => {
    const drive = soundCubeDrive(0.2, 2);

    expect(drive.scale).toBeCloseTo((SCALE_BASE + 0.2 * SCALE_AUDIO) * 2);
    expect(soundCubeDrive(0.2, 0).scale).toBe(0);
  });

  it("sanitises negative / non-finite bands and pulse scales", () => {
    expect(soundCubeDrive(-1, 1).scale).toBeCloseTo(SCALE_BASE);
    expect(soundCubeDrive(Number.NaN, 1).scale).toBeCloseTo(SCALE_BASE);
    expect(soundCubeDrive(0.1, -2).scale).toBe(0);
    expect(soundCubeDrive(0.1, Number.NaN).scale).toBeCloseTo(
      SCALE_BASE + 0.1 * SCALE_AUDIO,
    );
  });
});

describe("SoundCube rotator", () => {
  it("advances each axis at the preset's degrees per second", () => {
    const next = advanceSoundCubeRotation({ x: 0, y: 0, z: 0 }, 0.1, 1);

    expect(next.x).toBeCloseTo(ROTATOR.x * DEG * 0.1);
    expect(next.y).toBeCloseTo(ROTATOR.y * DEG * 0.1);
    expect(next.z).toBeCloseTo(ROTATOR.z * DEG * 0.1);
  });

  it("scales the rotation speed and keeps the accumulated angle", () => {
    const start = { x: 1, y: 2, z: 3 };
    const next = advanceSoundCubeRotation(start, 0.1, 2);

    expect(next.x).toBeCloseTo(1 + ROTATOR.x * DEG * 0.2);
    expect(next.y).toBeCloseTo(2 + ROTATOR.y * DEG * 0.2);
    expect(next.z).toBeCloseTo(3 + ROTATOR.z * DEG * 0.2);
  });

  it("caps long frames and ignores negative / bad dt and spin values", () => {
    const capped = advanceSoundCubeRotation({ x: 0, y: 0, z: 0 }, 1, 1);
    const clamped = advanceSoundCubeRotation({ x: 0, y: 0, z: 0 }, 0.1, 1);
    const negative = advanceSoundCubeRotation({ x: 0, y: 0, z: 0 }, -1, 1);
    const bad = advanceSoundCubeRotation(
      { x: Number.NaN, y: Number.NaN, z: Number.NaN },
      Number.NaN,
      Number.NaN,
    );

    expect(capped.x).toBeCloseTo(clamped.x);
    expect(negative.x).toBe(0);
    expect(bad).toEqual({ x: 0, y: 0, z: 0 });
  });
});

describe("SoundCube feedback", () => {
  it("uses the preset's 0.95 zoom and 0.8 trail brightness", () => {
    expect(soundCubeFeedback(1)).toEqual({
      zoom: TRAIL_ZOOM,
      brightness: TRAIL_BRIGHTNESS,
    });
  });

  it("scales the feedback persistence with trailScale", () => {
    expect(soundCubeFeedback(0.5).brightness).toBeCloseTo(0.4);
    expect(soundCubeFeedback(0).brightness).toBe(0);
    expect(soundCubeFeedback(-1).brightness).toBe(0);
    expect(soundCubeFeedback(Number.NaN).brightness).toBe(TRAIL_BRIGHTNESS);
  });
});

describe("SoundCube preset constants", () => {
  it("uses the preset's cube, camera and clear settings", () => {
    expect(CUBE_SIZE).toBe(1);
    expect(CAMERA).toEqual({ fov: 45, near: 0.1, far: 100 });
    expect(CLEAR_ALPHA).toBe(0);
  });

  it("uses the preset's 256x256 feedback target and rotator speeds", () => {
    expect(RTT_SIZE).toBe(256);
    expect(ROTATOR).toEqual({ x: 1.5, y: 1, z: 0.5 });
    expect(SCALE_BASE).toBe(0.3);
    expect(SCALE_AUDIO).toBe(8);
  });

  it("uses the preset's Blur1 direction and width", () => {
    expect(BLUR).toEqual({ dir: 2, width: 4, brightness: 1 });
    expect(TRAIL_ZOOM).toBe(0.95);
    expect(TRAIL_BRIGHTNESS).toBe(0.8);
  });
});

describe("SoundCube scene fields", () => {
  it("initialises the four parent-wired scales to 1", () => {
    const scene = new SoundCube(null, null);

    expect(scene.spinScale).toBe(1);
    expect(scene.pulseScale).toBe(1);
    expect(scene.trailScale).toBe(1);
    expect(scene.brightnessScale).toBe(1);
  });

  it("consumes spinScale and pulseScale in update", () => {
    const scene = new SoundCube(null, null);

    scene.spinScale = 0;
    scene.pulseScale = 2;
    scene.update(0.1, null);

    expect(scene.rotation).toEqual({ x: 0, y: 0, z: 0 });
    // Silence: (0.3 + 0 * 8) * 2
    expect(scene.cubeGroup.scale.x).toBeCloseTo(0.6);
    expect(scene.trailMaterial.uniforms.gTrailScale.value).toBeCloseTo(
      TRAIL_BRIGHTNESS,
    );
    expect(scene.compositeMaterial.uniforms.gBrightness.value).toBe(1);
  });
});

describe("SoundCube shaders", () => {
  it("rotates the cube from the transform matrices and keeps the uv", () => {
    const vertex = buildSoundCubeVertexShader();

    expect(vertex).toContain("vTex = uv");
    expect(vertex).toContain("gColor * gColor1");
    expect(vertex).toContain("projectionMatrix * modelViewMatrix");
  });

  it("keeps Shader1's bright ring / inner face pattern", () => {
    const fragment = buildSoundCubeFragmentShader();

    expect(fragment).toContain("smoothstep(1.0, 0.5, abs(p.x))");
    expect(fragment).toContain("smoothstep(1.0, 0.5, abs(p.y))");
    expect(fragment).toContain("smoothstep(0.4, 0.6, length(p))");
    expect(fragment).toContain("vec4(vec3(v * inner)");
  });

  it("composites mix(bg, c, c.a) + c and stays transparent on empty space", () => {
    const fragment = buildSoundCubeCompositeFragment();

    expect(fragment).toContain("texture2D(gTexture2, vUv)");
    expect(fragment).toContain("texture2D(gTexture1, vUv)");
    expect(fragment).toContain("mix(bg, c, c.a) + c");
    expect(fragment).toContain("gBrightness");
    // An opaque screen clear would hide the layers underneath.
    expect(fragment).not.toMatch(
      /gl_FragColor\s*=\s*vec4\([\s\S]*?,\s*1\.0\s*\)/,
    );
  });

  it("copies the feedback texture scaled by 0.95 with alpha 0", () => {
    const fragment = buildSoundCubeTrailFragment();

    expect(fragment).toContain("(p * 0.95) * 0.5 + 0.5");
    expect(fragment).toContain("c.rgb * 0.8 * gTrailScale");
    expect(fragment).toContain("0.0");
  });

  it("blurs in both directions using the direction and texel size", () => {
    const fragment = buildSoundCubeBlurFragment();

    expect(fragment).toContain("gDir");
    expect(fragment).toContain("gTexel");
    expect(fragment).toContain("gWidth");
    expect(fragment).toContain("for (int i = -4; i <= 4; i++)");
  });
});
