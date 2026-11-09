import { describe, expect, it, vi } from "vitest";
import { AudioBands } from "../../src/renderer/src/application/audio-bands.js";
import { Point10AudioBridge } from "../../src/renderer/src/application/point10/audioBridge.js";
import {
  CLEAR_ALPHA,
  SpiralParticles,
} from "../../src/renderer/src/application/point10/scenes/SpiralParticles.js";

const levers = {
  gain: 1,
  smoothing: 0,
  bassGain: 1,
  midGain: 1,
  trebleGain: 1,
  useAgc: true,
};

// Drive the scene with the same chain the adapter uses: AudioBands ->
// Point10AudioBridge -> byte spectrum -> scene.update(dt, audioData).
function spectrumFromAudioBands(value = 200) {
  const fftSize = 1024;
  const byteFrequencyData = new Uint8Array(fftSize);

  byteFrequencyData[0] = value;

  const payload = new AudioBands().update({
    floatFrequencyData: new Float32Array(fftSize).fill(-100),
    byteTimeDomainData: new Uint8Array(fftSize).fill(128),
    byteFrequencyData,
    sampleRate: 44100,
  });

  const bridge = new Point10AudioBridge();
  bridge.update(0.1, payload, levers);

  return bridge.spectrum;
}

function snapshot(scene) {
  return {
    rotation: scene.points.rotation.y,
    positions: Array.from(scene.geometry.attributes.position.array),
    colors: Array.from(scene.geometry.attributes.color.array),
  };
}

describe("SpiralParticles transparency", () => {
  it("clears to a fully transparent background", () => {
    expect(CLEAR_ALPHA).toBe(0);

    const renderer = { setClearColor: vi.fn() };
    const scene = new SpiralParticles(renderer, null);

    expect(renderer.setClearColor).toHaveBeenCalledWith(0x000000, 0);
    expect(scene.material.transparent).toBe(true);
    expect(scene.material.depthWrite).toBe(false);
    // The sprite's transparent border is discarded instead of blended, so the
    // point quads never leave a dark square against the transparent clear.
    expect(scene.material.alphaTest).toBeGreaterThan(0);
  });
});

describe("SpiralParticles", () => {
  it("uses the shared circular sprite map instead of a square point", () => {
    const scene = new SpiralParticles(null, null);

    expect(scene.material.map?.isDataTexture).toBe(true);
  });

  it("responds to the real Point10 bridge spectrum", () => {
    const scene = new SpiralParticles(null, null);
    const before = snapshot(scene);
    const audioData = spectrumFromAudioBands(200);

    expect(audioData.length).toBeGreaterThan(0);
    expect(audioData[0]).toBe(200);

    scene.update(0.25, audioData);

    const energy = 200 / 255;
    const after = snapshot(scene);

    expect(after.rotation - before.rotation).toBeCloseTo(
      0.25 * (0.1 + energy * 0.5),
    );

    // energy > 0.6, so the spiral jitters vertically...
    const moved = after.positions.filter(
      (value, index) => value !== before.positions[index],
    );

    expect(moved.length).toBeGreaterThan(0);

    // ...and every particle is recoloured from the same energy.
    const recoloured = after.colors.filter(
      (value, index) => value !== before.colors[index],
    );

    expect(recoloured.length).toBeGreaterThan(0);
    for (let i = 0; i < after.positions.length; i++) {
      expect(Number.isFinite(after.positions[i])).toBe(true);
    }
    for (let i = 0; i < after.colors.length; i++) {
      expect(Number.isFinite(after.colors[i])).toBe(true);
    }
  });

  it("leaves the spiral unchanged without a spectrum", () => {
    const scene = new SpiralParticles(null, null);
    const before = snapshot(scene);

    scene.update(0.25, null);

    const after = snapshot(scene);

    expect(after.rotation).toBe(before.rotation);
    expect(after.positions).toEqual(before.positions);
    expect(after.colors).toEqual(before.colors);
  });

  it("leaves the spiral unchanged for an empty spectrum", () => {
    const scene = new SpiralParticles(null, null);
    const before = snapshot(scene);

    scene.update(0.25, new Uint8Array(0));

    const after = snapshot(scene);

    expect(after.rotation).toBe(before.rotation);
    expect(after.positions).toEqual(before.positions);
    expect(after.colors).toEqual(before.colors);
  });
});
