import { describe, expect, it } from "vitest";
import {
  PARTICLE_SPRITE_INNER,
  buildParticleAlphaMap,
} from "../../src/renderer/src/application/point10/shared/particleAlpha.js";
import { createParticleTexture } from "../../src/renderer/src/application/point10/shared/particleSprite.js";
import { ParticleField } from "../../src/renderer/src/application/point10/scenes/ParticleField.js";

const SIZE = 32;

function alphaAt(data, x, y) {
  return data[(y * SIZE + x) * 4 + 3];
}

describe("Point10 particle sprites", () => {
  const alphaMap = buildParticleAlphaMap(SIZE);

  it("builds a deterministic RGBA map at the requested size", () => {
    expect(alphaMap).toHaveLength(SIZE * SIZE * 4);
    expect(buildParticleAlphaMap(SIZE)).toEqual(alphaMap);
    expect(alphaMap[0]).toBe(255);
    expect(alphaMap[1]).toBe(255);
    expect(alphaMap[2]).toBe(255);
  });

  it("is opaque in the middle and transparent at the corners", () => {
    const centre = Math.floor((SIZE - 1) / 2);

    expect(alphaAt(alphaMap, centre, centre)).toBe(255);

    for (const [x, y] of [
      [0, 0],
      [SIZE - 1, 0],
      [0, SIZE - 1],
      [SIZE - 1, SIZE - 1],
    ]) {
      expect(alphaAt(alphaMap, x, y)).toBe(0);
    }
  });

  it("clears a circular edge instead of writing a square", () => {
    const centre = Math.floor((SIZE - 1) / 2);

    for (const [x, y] of [
      [0, centre],
      [SIZE - 1, centre],
      [centre, 0],
      [centre, SIZE - 1],
    ]) {
      expect(alphaAt(alphaMap, x, y)).toBe(0);
    }
  });

  it("fades smoothly from the fully opaque inner disc to the edge", () => {
    const centre = Math.floor((SIZE - 1) / 2);
    let previous = alphaAt(alphaMap, centre, centre);

    for (let y = centre; y < SIZE; y++) {
      const alpha = alphaAt(alphaMap, centre, y);
      expect(alpha).toBeLessThanOrEqual(previous);
      previous = alpha;
    }

    expect(PARTICLE_SPRITE_INNER).toBeGreaterThan(0);
    expect(PARTICLE_SPRITE_INNER).toBeLessThan(1);
  });

  it("wraps the alpha map in a linear-filtered texture", () => {
    const texture = createParticleTexture(SIZE);

    expect(texture.isDataTexture).toBe(true);
    expect(texture.image.width).toBe(SIZE);
    expect(texture.image.height).toBe(SIZE);
    expect(texture.image.data).toEqual(alphaMap);
  });
});

describe("ParticleField sprites", () => {
  it("uses the circular sprite map on its points material", () => {
    const scene = new ParticleField(null, null);

    expect(scene.material.map).toBeTruthy();
    expect(scene.material.map.isDataTexture).toBe(true);
    expect(scene.material.alphaTest).toBeGreaterThan(0);
  });
});
