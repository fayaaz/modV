import { describe, expect, it } from "vitest";
import { valueNoise } from "../../src/renderer/src/application/point10/plane9/P9Expression.js";
import {
  EARTH,
  buildEarthAtmosphereFragment,
  buildEarthSkyFragment,
  buildEarthSurfaceFragment,
  buildEarthSurfaceVertex,
  earthCamera,
  earthLightLevel,
  earthSunPosition,
  earthTexturePhase,
} from "../../src/renderer/src/application/point10/scenes/Earth.js";

// Earth needs WebGL for the planet/atmosphere/sky render and bloom composer,
// but the Expression drives (camera, sun orbit, texture scroll, night lights)
// are plain arithmetic and are what the music controls, so they are tested in
// isolation - the same split the other imported Point10 scenes use.

describe("Earth preset constants", () => {
  it("keeps the Plane9 sphere, atmosphere and camera values", () => {
    expect(EARTH.radius).toBe(10);
    expect(EARTH.atmosphereRadius).toBe(10.2);
    expect(EARTH.skyRadius).toBe(150);
    expect(EARTH.sunDistance).toBe(100);
    expect(EARTH.fov).toBe(60.4);
    expect(EARTH.near).toBe(0.1);
    expect(EARTH.far).toBe(200);
  });

  it("adopts the Point10 tuning (zoomed-out camera, slower sun orbit)", () => {
    expect(EARTH.cameraDistance).toBe(34);
    expect(EARTH.cameraDistanceVar).toBe(5.5);
    expect(EARTH.sunTimeScale).toBe(0.04);
  });

  it("keeps the preset bloom settings", () => {
    expect(EARTH.bloom).toEqual({ strength: 0.2, radius: 0.85, threshold: 0 });
  });
});

describe("Earth camera Expression", () => {
  it("starts at the preset's per-scene phase", () => {
    const drive = earthCamera(0);
    const t = EARTH.seed * 15;
    const rot = valueNoise(2, t * 0.2, 0);
    const dist =
      EARTH.cameraDistance + valueNoise(2, t * 2, 2) * EARTH.cameraDistanceVar;

    expect(drive.position.x).toBeCloseTo(Math.cos(rot * Math.PI * 4) * dist);
    expect(drive.position.y).toBeCloseTo(valueNoise(2, t, 1) * 10);
    expect(drive.position.z).toBeCloseTo(Math.sin(rot * Math.PI * 4) * dist);
  });

  it("wanders the look-at target inside the preset's five-unit box", () => {
    for (const time of [0, 5, 30, 120]) {
      const drive = earthCamera(time);

      expect(Math.abs(drive.lookAt.x)).toBeLessThanOrEqual(5);
      expect(Math.abs(drive.lookAt.y)).toBeLessThanOrEqual(5);
      expect(Math.abs(drive.lookAt.z)).toBeLessThanOrEqual(5);
    }
  });

  it("keeps the camera at the tuned distance from the planet", () => {
    for (const time of [0, 5, 30, 120]) {
      const drive = earthCamera(time);
      const radius = Math.hypot(
        drive.position.x,
        drive.position.y,
        drive.position.z,
      );

      expect(radius).toBeGreaterThanOrEqual(
        EARTH.cameraDistance - EARTH.cameraDistanceVar - 0.001,
      );
      expect(radius).toBeLessThanOrEqual(
        Math.hypot(EARTH.cameraDistance + EARTH.cameraDistanceVar, 10) + 0.001,
      );
    }
  });

  it("moves the camera slowly (0.005 time scale)", () => {
    const a = earthCamera(0);
    const b = earthCamera(1);

    expect(a.position.x).not.toBe(b.position.x);
    expect(
      Math.abs(b.position.x - a.position.x) +
        Math.abs(b.position.z - a.position.z),
    ).toBeLessThan(1);
  });
});

describe("Earth sun orbit Expression", () => {
  it("starts at the preset's initial angle and distance", () => {
    const sun = earthSunPosition(0);
    const t = 1 + 10 * EARTH.seed;

    expect(sun.x).toBeCloseTo(Math.sin(-t) * 100);
    expect(sun.y).toBe(0);
    expect(sun.z).toBeCloseTo(Math.cos(-t) * 100);
    expect(Math.hypot(sun.x, sun.z)).toBeCloseTo(100);
  });

  it("uses the tuned 0.04 orbit rate and scales with the module input", () => {
    const normal = earthSunPosition(1, 1);
    const fast = earthSunPosition(1, 2);

    expect(normal.x).toBeCloseTo(
      Math.sin(-(1 + 10 * EARTH.seed + 1 * 0.04)) * 100,
    );
    expect(fast.x).toBeCloseTo(
      Math.sin(-(1 + 10 * EARTH.seed + 1 * 0.04 * 2)) * 100,
    );
  });
});

describe("Earth texture scroll Expression", () => {
  it("starts at the preset's ten-unit texture offset", () => {
    expect(earthTexturePhase(0)).toBe(10);
  });

  it("scrolls at 0.002/s and scales with spin", () => {
    expect(earthTexturePhase(100)).toBeCloseTo(10 + 100 * 0.002);
    expect(earthTexturePhase(100, 2)).toBeCloseTo(10 + 100 * 0.002 * 2);
    expect(earthTexturePhase(100, 0)).toBe(10);
  });
});

describe("Earth night-light Expression", () => {
  it("keeps a dim floor in silence and clamps at the preset ceiling", () => {
    expect(earthLightLevel(0)).toBeCloseTo(0.2);
    expect(earthLightLevel(0.1)).toBeCloseTo(0.5);
    expect(earthLightLevel(1)).toBeCloseTo(1);
    expect(earthLightLevel(-1)).toBeCloseTo(0.2);
  });

  it("scales with the module input", () => {
    expect(earthLightLevel(0.1, 2)).toBeCloseTo(1);
  });
});

describe("Earth shader sources", () => {
  it("keeps the planet atmosphere scattering math", () => {
    const vertex = buildEarthSurfaceVertex();
    const fragment = buildEarthSurfaceFragment();

    expect(vertex).toContain("InvWaveLength");
    expect(vertex).toContain("getNearIntersection");
    expect(fragment).toContain(
      "_brightnessSaturationContrast(c.rgb, 1.2, 2.2, 1.05)",
    );
    expect(fragment).toContain("texture2D(gTexture2");
    expect(fragment).toContain("gIn3.x");
  });

  it("draws the planet opaque so the adapter does not fade it", () => {
    // Plane9's SrcBlend=1/DstBlend=0 ignored the shader's 0.1 day-side alpha;
    // the Point10 adapter composites the canvas, so the planet must be opaque.
    expect(buildEarthSurfaceFragment()).toContain(
      "gl_FragColor = vec4((c + lights).rgb, 1.0);",
    );
  });

  it("keeps the sky star and panorama math", () => {
    const fragment = buildEarthSkyFragment();

    expect(fragment).toContain("_texturePanoramic");
    expect(fragment).toContain("smoothstep(0.7, 1.0, _noise(p*1.5))");
    expect(fragment).toContain("smoothstep(0.8, 1.0, _noise(p*0.75))");
  });

  it("keeps the atmosphere shell offset", () => {
    expect(buildEarthAtmosphereFragment()).toContain("RayleighPhase");
  });
});
