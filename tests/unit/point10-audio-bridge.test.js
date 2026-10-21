import { describe, expect, it } from "vitest";
import { Point10AudioBridge } from "../../src/renderer/src/application/point10/audioBridge.js";

const defaultLevers = () => ({
  gain: 1,
  smoothing: 0,
  bassGain: 1,
  midGain: 1,
  trebleGain: 1,
  useAgc: true,
});

function makePayload(overrides = {}) {
  const lanes = new Array(12).fill(0);
  const cells = new Array(16).fill(0);

  lanes[0] = 0.5;
  cells[0] = 0.4;

  return {
    sampleRate: 44100,
    lanes,
    cells,
    beat: 0.5,
    wave: new Float32Array(1024).fill(0.5),
    freqMag: new Float32Array(1024).fill(0.5),
    raw: {
      bands: [0.25, 0.25, 0.25],
      lanes: new Array(12).fill(0.25),
      cells: new Array(16).fill(0.25),
    },
    ...overrides,
  };
}

describe("Point10AudioBridge", () => {
  it("exposes no analyser until audio arrives", () => {
    const bridge = new Point10AudioBridge();

    expect(bridge.analyser).toBeNull();

    bridge.update(0.1, makePayload(), defaultLevers());

    expect(bridge.analyser).toBe(bridge);
  });

  it("clears the audio gate when the payload disappears", () => {
    const bridge = new Point10AudioBridge();

    bridge.update(0.1, makePayload(), defaultLevers());
    bridge.update(0.1, null, defaultLevers());

    expect(bridge.hasAudio).toBe(false);
    expect(bridge.analyser).toBeNull();
  });

  it("copies bands, cells, beat and wave through unity levers", () => {
    const bridge = new Point10AudioBridge();

    bridge.update(0.1, makePayload(), defaultLevers());

    expect(bridge.bands[0]).toBe(0.5);
    expect(bridge.grid[0]).toBeCloseTo(0.4);
    expect(bridge.beatEnv).toBeCloseTo(0.5);
    expect(bridge.wave[0]).toBe(0.5);
    expect(bridge.freqMag[0]).toBe(0.5);
    expect(bridge.sampleRate).toBe(44100);
  });

  it("applies gain and per-band gain with clamping", () => {
    const bridge = new Point10AudioBridge();
    const levers = { ...defaultLevers(), gain: 2, bassGain: 1.5 };

    bridge.update(0.1, makePayload(), levers);

    expect(bridge.lanes[0]).toBe(1);
    expect(bridge.grid[0]).toBe(1);
    expect(bridge.beatEnv).toBe(1);
    expect(bridge.freqMag[0]).toBe(1);
  });

  it("uses the raw signal when AGC is disabled", () => {
    const bridge = new Point10AudioBridge();

    bridge.update(0.1, makePayload(), {
      ...defaultLevers(),
      useAgc: false,
    });

    expect(bridge.lanes[0]).toBeCloseTo(0.25);
    expect(bridge.grid[0]).toBeCloseTo(0.25);
  });

  it("smooths towards the target when the smoothing lever is set", () => {
    const bridge = new Point10AudioBridge();

    bridge.update(0.1, makePayload(), { ...defaultLevers(), smoothing: 1 });

    expect(bridge.lanes[0]).toBeGreaterThan(0);
    expect(bridge.lanes[0]).toBeLessThan(0.5);
    expect(bridge.grid[0]).toBeGreaterThan(0);
    expect(bridge.grid[0]).toBeLessThan(0.4);
  });

  it("scales the byte spectrum by the matching band gain", () => {
    const bridge = new Point10AudioBridge();
    const byteFrequencyData = new Uint8Array(1024);

    byteFrequencyData[2] = 100;
    byteFrequencyData[14] = 50;

    bridge.update(0.1, makePayload({ byteFrequencyData }), {
      ...defaultLevers(),
      bassGain: 2,
      midGain: 3,
    });

    expect(bridge.spectrum[2]).toBe(200);
    expect(bridge.spectrum[14]).toBe(150);
  });

  it("clamps the byte spectrum to 0..255", () => {
    const bridge = new Point10AudioBridge();
    const byteFrequencyData = new Uint8Array(1024);

    byteFrequencyData[2] = 200;

    bridge.update(0.1, makePayload({ byteFrequencyData }), {
      ...defaultLevers(),
      gain: 4,
    });

    expect(bridge.spectrum[2]).toBe(255);
  });
});
