import { describe, expect, it } from "vitest";
import { AudioBands } from "../../src/renderer/src/application/audio-bands.js";
import { Point10AudioBridge } from "../../src/renderer/src/application/point10/audioBridge.js";
import { CubeFloor } from "../../src/renderer/src/application/point10/scenes/CubeFloor.js";

const levers = {
  gain: 1,
  smoothing: 0,
  bassGain: 1,
  midGain: 1,
  trebleGain: 1,
  useAgc: true,
};

// The shared adapter camera sits at (0, 0, 15) looking at the origin, so the
// flat grid is edge-on (and therefore black) until audio raises the cubes.
function audioDataThroughBridge() {
  const fftSize = 1024;
  const byteFrequencyData = new Uint8Array(fftSize);

  for (let i = 0; i < 64; i++) {
    byteFrequencyData[i] = 200;
  }

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

describe("CubeFloor", () => {
  it("raises cubes above the floor when the bridge provides a spectrum", () => {
    const scene = new CubeFloor(null, null);
    const audioData = audioDataThroughBridge();

    expect(audioData.length).toBeGreaterThan(0);

    scene.update(0.1, audioData);

    const raised = scene.cubes.filter(
      (cube) => cube.mesh.position.y > 0 && cube.mesh.scale.y > 0.05,
    );

    expect(raised.length).toBeGreaterThan(0);
    expect(raised[0].mesh.scale.y).toBeGreaterThan(0.05);
  });
});
