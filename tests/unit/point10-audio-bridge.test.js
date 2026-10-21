import { describe, expect, it } from "vitest";
import { AudioBands } from "../../src/renderer/src/application/audio-bands.js";
import { Point10AudioBridge } from "../../src/renderer/src/application/point10/audioBridge.js";
import { P9Audio } from "../../src/renderer/src/application/point10/plane9/P9Audio.js";

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

  it("builds a real spectrum from the AudioBands payload", () => {
    const fftSize = 1024;
    const sampleRate = 44100;
    const byteFrequencyData = new Uint8Array(fftSize);

    byteFrequencyData[2] = 100;
    byteFrequencyData[14] = 50;
    byteFrequencyData[fftSize - 1] = 200;

    const audioBands = new AudioBands();
    const payload = audioBands.update({
      floatFrequencyData: new Float32Array(fftSize).fill(-100),
      byteTimeDomainData: new Uint8Array(fftSize).fill(128),
      byteFrequencyData,
      sampleRate,
    });

    const bridge = new Point10AudioBridge();
    bridge.update(0.1, payload, defaultLevers());

    expect(payload.byteFrequencyData).toHaveLength(fftSize);
    expect(bridge.spectrum).toHaveLength(fftSize);
    expect(bridge.spectrum[2]).toBe(100);
    expect(bridge.spectrum[14]).toBe(50);
    expect(bridge.spectrum[fftSize - 1]).toBe(200);
    expect(Array.from(bridge.spectrum)).toEqual(Array.from(byteFrequencyData));
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

  it("exposes the engine fields the vendored P9Audio reads", () => {
    const bridge = new Point10AudioBridge();

    bridge.update(0.1, makePayload(), defaultLevers());

    expect(bridge.freqArray).toHaveLength(1024);
    expect(bridge.freqArray[0]).toBe(0.5);
    expect(bridge.freqMagL).toHaveLength(1024);
    expect(bridge.freqMagR).toHaveLength(1024);
    expect(bridge.freqMagL[0]).toBe(0.5);
    expect(bridge.freqMagR[0]).toBe(0.5);
    expect(bridge.ctx.sampleRate).toBe(44100);
    expect(bridge.dataArray).toHaveLength(1024);
    expect(bridge.dataArray[0]).toBe(191);
  });

  it("tracks sample rate changes on the P9Audio ctx alias", () => {
    const bridge = new Point10AudioBridge();

    bridge.update(0.1, makePayload({ sampleRate: 48000 }), defaultLevers());

    expect(bridge.sampleRate).toBe(48000);
    expect(bridge.ctx.sampleRate).toBe(48000);
  });

  it("drives the vendored P9Audio from the bridge", () => {
    const bridge = new Point10AudioBridge();

    bridge.update(0.1, makePayload(), defaultLevers());

    const p9 = new P9Audio(bridge);
    p9.update(0.1);

    expect(p9.hasAudio).toBe(true);
    expect(p9.freq[0]).toBe(0.5);
    expect(p9.wave[0]).toBeCloseTo(0.5, 1);
    expect(p9.bands[0]).toBeGreaterThan(0);
    expect(p9.band(-1, 0, 0)).toBe(p9.bands[0]);
  });
});
