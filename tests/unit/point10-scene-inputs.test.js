import { describe, expect, it } from "vitest";
import { ChromaticLine } from "../../src/renderer/src/application/point10/scenes/ChromaticLine.js";
import { Mycelium } from "../../src/renderer/src/application/point10/scenes/Mycelium.js";
import { ParticleField } from "../../src/renderer/src/application/point10/scenes/ParticleField.js";
import { Waveform } from "../../src/renderer/src/application/point10/scenes/Waveform.js";

const loud = () => new Uint8Array(1024).fill(255);
const half = () => new Uint8Array(1024).fill(128);

describe("Point10 scene inputs change the output", () => {
  it("scales the Waveform height and opacity", () => {
    const scene = new Waveform(null, null);
    const audio = loud();
    scene.update(0.016, audio);
    const base = Math.max(
      ...Array.from(scene.line.geometry.attributes.position.array).filter(
        (_value, index) => index % 3 === 1,
      ),
    );

    scene.heightScale = 2;
    scene.opacity = 0.5;
    scene.update(0.016, audio);
    const scaled = Math.max(
      ...Array.from(scene.line.geometry.attributes.position.array).filter(
        (_value, index) => index % 3 === 1,
      ),
    );

    expect(scaled).toBeGreaterThan(base * 1.5);
    expect(scene.line.material.opacity).toBeCloseTo(0.4);
    expect(scene.line2.material.opacity).toBeCloseTo(0.25);
  });

  it("pauses the Particle Field at zero speed", () => {
    const scene = new ParticleField(null, null);
    const before = scene.positions.slice();
    scene.speedScale = 0;
    scene.particleSize = 0.4;
    scene.opacity = 0.3;
    scene.update(0.016, half());

    expect(scene.positions).toEqual(before);
    expect(scene.material.size).toBe(0.4);
    expect(scene.material.opacity).toBe(0.3);
  });

  it("applies the Chromatic Line amplitude and chroma", () => {
    const scene = new ChromaticLine(null, null);
    scene.ampScale = 2;
    scene.distortion = 0.2;

    scene._audioUniforms(0.1, 0.1, 0.1);

    expect(scene.uniforms.amp.value.x).toBeCloseTo(1.6);
    expect(scene.uniforms.amp.value.y).toBeCloseTo(1.6);
    expect(scene.uniforms.distortion.value).toBe(0.2);
  });

  it("scales Mycelium growth, glow and fog", () => {
    const scene = new Mycelium(null, null);
    scene.speedScale = 2;
    scene.glowScale = 2;
    scene.fogScale = 2;

    scene._audioUniforms(0, 0, 0, 0);

    expect(scene.speed).toBeCloseTo(0.5);
    expect(scene.uniforms.u_glow.value).toBeCloseTo(0.3);
    expect(scene.uniforms.u_fogDensity.value).toBeCloseTo(
      (0.25 + 0.3 * 0.22) * 2,
    );
  });
});
