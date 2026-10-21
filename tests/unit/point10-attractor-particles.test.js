import { describe, expect, it } from "vitest";
import { audioUniforms } from "../../src/renderer/src/application/point10/scenes/AttractorParticles.js";

// The scene's pure audio -> GPU-uniform mapping. The scene itself needs WebGPU
// to render, but the drive curve is plain arithmetic and is what the music
// actually controls, so it is tested in isolation.
describe("AttractorParticles audioUniforms", () => {
  it("uses the example's silence defaults", () => {
    expect(audioUniforms(0, 0, 0, 0)).toEqual({
      maxSpeed: 8,
      attractorMass: 1e6,
      spinningStrength: 2.75,
      particleGlobalMass: 1e4,
    });
  });

  it("drives maxSpeed and the spin with bass", () => {
    const u = audioUniforms(1, 0, 0, 0);

    expect(u.maxSpeed).toBe(14);
    expect(u.spinningStrength).toBe(12.75);
    // snare + hi are silent, so only the bass assist lifts the exponent
    expect(u.attractorMass).toBe(1e7);
  });

  it("lets percussion and energy raise the mass exponents and spin", () => {
    const u = audioUniforms(1, 1, 1, 1);

    expect(u.attractorMass).toBe(1e8);
    expect(u.particleGlobalMass).toBe(1e6);
    expect(u.spinningStrength).toBe(14.75);
  });

  it("clamps the percussion exponent assist at the top of its range", () => {
    const loud = audioUniforms(0, 1, 1, 0);

    // (snare + hi) * 1.5 clamps to 1 -> 10^(6 + 2)
    expect(loud.attractorMass).toBe(1e8);
    expect(loud.particleGlobalMass).toBe(1e4);
  });

  it("scales smoothly between the documented ranges", () => {
    const u = audioUniforms(0.5, 0.25, 0, 0.5);

    expect(u.maxSpeed).toBeCloseTo(11);
    expect(u.spinningStrength).toBeCloseTo(2.75 + 5 + 1);
    expect(u.attractorMass).toBeCloseTo(Math.pow(10, 6 + (0.375 + 0.25) * 2));
    expect(u.particleGlobalMass).toBeCloseTo(1e5);
  });
});
