import { describe, expect, it } from "vitest";
import {
  GeometricTunnel,
  TUNNEL_ENV_HEIGHT,
  TUNNEL_ENV_WIDTH,
  TUNNEL_GRAIN_SIZE,
  TUNNEL_SSAO,
  TUNNEL_SURFACE,
  buildTunnelEnvironment,
  buildTunnelGrain,
  tileableNoise,
  tunnelHash,
} from "../../src/renderer/src/application/point10/scenes/GeometricTunnel.js";

// The ring meshes need WebGL to draw, but the surface maps, the environment
// ramp and the material wiring are plain data. Those are what make the hoops
// read as anodised metal instead of flat plastic, so they are tested directly.

const channel = (data, index, offset) => data[index * 4 + offset];

const average = (values) =>
  values.reduce((sum, value) => sum + value, 0) / values.length;

describe("GeometricTunnel surface grain", () => {
  const grain = buildTunnelGrain();

  it("builds a deterministic RGBA texture at the requested size", () => {
    expect(grain).toHaveLength(TUNNEL_GRAIN_SIZE * TUNNEL_GRAIN_SIZE * 4);
    expect(buildTunnelGrain()).toEqual(grain);
    expect(grain[3]).toBe(255);
  });

  it("is grayscale so one map can drive bump, roughness and emission", () => {
    for (let i = 0; i < grain.length; i += 4) {
      expect(grain[i]).toBe(grain[i + 1]);
      expect(grain[i + 1]).toBe(grain[i + 2]);
    }
  });

  it("has real variation instead of a flat plastic sheen", () => {
    const values = [];
    for (let i = 0; i < grain.length; i += 4) values.push(grain[i]);
    const min = Math.min(...values);
    const max = Math.max(...values);

    expect(min).toBeGreaterThan(30);
    expect(max).toBeLessThan(240);
    expect(max - min).toBeGreaterThan(35);
  });

  it("keeps the value noise periodic so the maps tile without a seam", () => {
    expect(tileableNoise(0, 0.37, 8, 3)).toBe(tileableNoise(1, 0.37, 8, 3));
    expect(tileableNoise(0.37, 0, 8, 3)).toBe(tileableNoise(0.37, 1, 8, 3));
    expect(tileableNoise(0.37, 0.61, 8, 3)).toBe(
      tileableNoise(0.37, 0.61, 8, 3),
    );
  });

  it("keeps the lattice hash deterministic and in [0, 1)", () => {
    const a = tunnelHash(3, 7, 11);

    expect(a).toBe(tunnelHash(3, 7, 11));
    expect(a).toBeGreaterThanOrEqual(0);
    expect(a).toBeLessThan(1);
    expect(tunnelHash(3, 7, 12)).not.toBe(a);
  });
});

describe("GeometricTunnel environment", () => {
  const env = buildTunnelEnvironment();
  const rowLuminance = (row) => {
    const start = row * TUNNEL_ENV_WIDTH;
    const values = [];
    for (let x = 0; x < TUNNEL_ENV_WIDTH; x++) {
      const i = start + x;
      values.push((env[i * 4] + env[i * 4 + 1] + env[i * 4 + 2]) / 3);
    }
    return average(values);
  };

  it("builds a deterministic equirectangular RGBA ramp", () => {
    expect(env).toHaveLength(TUNNEL_ENV_WIDTH * TUNNEL_ENV_HEIGHT * 4);
    expect(buildTunnelEnvironment()).toEqual(env);
    expect(channel(env, 0, 3)).toBe(255);
  });

  it("lights the top of the room brighter and cooler than the floor", () => {
    const top = rowLuminance(TUNNEL_ENV_HEIGHT - 1);
    const bottom = rowLuminance(0);
    const topIndex = (TUNNEL_ENV_HEIGHT - 1) * TUNNEL_ENV_WIDTH;
    const bottomIndex = 0;
    const topBlue = channel(env, topIndex, 2) - channel(env, topIndex, 0);
    const bottomBlue =
      channel(env, bottomIndex, 2) - channel(env, bottomIndex, 0);

    expect(top).toBeGreaterThan(bottom * 1.5);
    expect(topBlue).toBeGreaterThan(bottomBlue);
  });

  it("includes bright light panels for the metal to reflect", () => {
    let brightest = 0;
    for (let i = 0; i < env.length; i += 4) {
      brightest = Math.max(brightest, env[i], env[i + 1], env[i + 2]);
    }

    expect(brightest).toBeGreaterThan(220);
  });
});

describe("GeometricTunnel material wiring", () => {
  it("uses PBR settings that keep the hoops from reading as plastic", () => {
    expect(TUNNEL_SURFACE.metalness).toBeGreaterThanOrEqual(0.8);
    // Mirror-smooth metal plus no environment is exactly the plastic look.
    expect(TUNNEL_SURFACE.roughness).toBeGreaterThan(0.3);
    expect(TUNNEL_SURFACE.envMapIntensity).toBeGreaterThan(1);
    expect(TUNNEL_SURFACE.bumpScale).toBeGreaterThan(0);
  });

  it("gives every hoop the environment and surface maps", () => {
    const scene = new GeometricTunnel(null, null);
    const material = scene.rings[0].frame.material;

    expect(scene.scene.environment).toBe(scene.envMap);
    expect(material.envMapIntensity).toBe(TUNNEL_SURFACE.envMapIntensity);
    expect(material.roughnessMap).toBe(scene.grainMap);
    expect(material.bumpMap).toBe(scene.grainMap);
    expect(material.emissiveMap).toBe(scene.emissiveMap);
    expect(material.metalness).toBeGreaterThanOrEqual(0.8);
    expect(material.roughness).toBeGreaterThan(0.3);
    // Near-black metal tint absorbs the environment reflection and reads
    // plastic; the base has to be a visible gunmetal colour.
    expect(material.color.getHex()).toBeGreaterThan(0x222222);
  });

  it("draws the dust with the circular particle sprite", () => {
    const scene = new GeometricTunnel(null, null);

    expect(scene.dust.material.map?.isDataTexture).toBe(true);
    expect(scene.dust.material.alphaTest).toBeGreaterThan(0);
  });

  it("keeps the ambient occlusion pass subtle enough for thin hoops", () => {
    expect(TUNNEL_SSAO.kernelRadius).toBeGreaterThan(0);
    expect(TUNNEL_SSAO.kernelRadius).toBeLessThan(2);
    expect(TUNNEL_SSAO.maxDistance).toBeGreaterThan(TUNNEL_SSAO.minDistance);
  });
});
