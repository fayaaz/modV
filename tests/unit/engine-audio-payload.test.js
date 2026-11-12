import { describe, expect, it } from "vitest";
import {
  buildEngineAudioPayload,
  ENGINE_AUDIO_CELL_COUNT,
  ENGINE_AUDIO_LANE_COUNT,
  ENGINE_AUDIO_SAMPLE_COUNT,
  ENGINE_AUDIO_SCALAR_FEATURES,
  finiteNumber,
  toNumberArray,
} from "../../src/renderer/src/application/engine-audio-payload.js";

function features() {
  return {
    bass: 0.5,
    mid: 0.25,
    treble: 0.75,
    rms: 0.4,
    zcr: 0.05,
    energy: 0.9,
    spectralCentroid: 1200,
    spectralFlatness: 0.2,
    spectralSlope: -0.1,
    spectralRolloff: 3000,
    spectralSpread: 800,
    spectralSkewness: 0.4,
    spectralKurtosis: 1.5,
    perceptualSpread: 0.6,
    perceptualSharpness: 0.7,
    byteFrequencyData: Uint8Array.from([1, 2, 3]),
    byteTimeDomainData: Uint8Array.from([0, 128, 255]),
    p10: {
      bass: 0.1,
      mid: 0.2,
      treble: 0.3,
      lanes: Float32Array.from(
        { length: ENGINE_AUDIO_LANE_COUNT },
        (_, i) => i / 100,
      ),
      cells: Float32Array.from(
        { length: ENGINE_AUDIO_CELL_COUNT },
        (_, i) => 0.5 + i / 100,
      ),
      beat: 0.4,
      wave: Float32Array.from(
        { length: ENGINE_AUDIO_SAMPLE_COUNT },
        (_, i) => i / 1024,
      ),
      freqMag: Float32Array.from(
        { length: ENGINE_AUDIO_SAMPLE_COUNT },
        (_, i) => i,
      ),
      sampleRate: 48000,
      raw: {
        lanes: Float32Array.from(
          { length: ENGINE_AUDIO_LANE_COUNT },
          (_, i) => 1 + i / 100,
        ),
        cells: Float32Array.from(
          { length: ENGINE_AUDIO_CELL_COUNT },
          (_, i) => 2 + i / 100,
        ),
      },
    },
  };
}

describe("toNumberArray", () => {
  it("converts typed arrays to plain number arrays", () => {
    expect(toNumberArray(Float32Array.from([0.5, 1]))).toEqual([0.5, 1]);
    expect(toNumberArray(Uint8Array.from([1, 2]))).toEqual([1, 2]);
  });

  it("returns an empty array for absent values", () => {
    expect(toNumberArray(undefined)).toEqual([]);
    expect(toNumberArray(null)).toEqual([]);
    expect(toNumberArray({})).toEqual([]);
  });

  it("replaces non-finite entries with zero", () => {
    expect(toNumberArray([1, Number.NaN, Number.POSITIVE_INFINITY])).toEqual([
      1, 0, 0,
    ]);
  });
});

describe("finiteNumber", () => {
  it("passes finite numbers through and falls back otherwise", () => {
    expect(finiteNumber(0.5)).toBe(0.5);
    expect(finiteNumber(0)).toBe(0);
    expect(finiteNumber(undefined, 44100)).toBe(44100);
    expect(finiteNumber(Number.NaN, 7)).toBe(7);
    expect(finiteNumber("nope", 3)).toBe(3);
  });
});

describe("buildEngineAudioPayload", () => {
  it("forwards every field the Rust AudioFrame stores", () => {
    const payload = buildEngineAudioPayload(features());

    expect(payload.bass).toBe(0.5);
    expect(payload.mid).toBe(0.25);
    expect(payload.treble).toBe(0.75);
    expect(payload.beat).toBe(0.4);
    expect(payload.sampleRate).toBe(48000);
    expect(payload.lanes).toHaveLength(ENGINE_AUDIO_LANE_COUNT);
    expect(payload.cells).toHaveLength(ENGINE_AUDIO_CELL_COUNT);
    expect(payload.rawLanes).toHaveLength(ENGINE_AUDIO_LANE_COUNT);
    expect(payload.rawCells).toHaveLength(ENGINE_AUDIO_CELL_COUNT);
    expect(payload.wave).toHaveLength(ENGINE_AUDIO_SAMPLE_COUNT);
    expect(payload.freqMag).toHaveLength(ENGINE_AUDIO_SAMPLE_COUNT);
    expect(payload.byteFrequencyData).toEqual([1, 2, 3]);
    expect(payload.byteTimeDomainData).toEqual([0, 128, 255]);

    // Every scalar feature survives the conversion.
    for (const name of ENGINE_AUDIO_SCALAR_FEATURES) {
      expect(payload[name]).toBe(features()[name]);
    }
  });

  it("keeps the analyser's distinct lane and cell values", () => {
    const payload = buildEngineAudioPayload(features());
    expect(payload.lanes[0]).toBe(0);
    expect(payload.lanes[11]).toBeCloseTo(0.11, 6);
    expect(payload.cells[0]).toBe(0.5);
    expect(payload.cells[15]).toBeCloseTo(0.65, 6);
    // Raw twins are forwarded separately (`useAgc: false`).
    expect(payload.rawLanes[0]).toBe(1);
    expect(payload.rawCells[15]).toBeCloseTo(2.15, 6);
  });

  it("falls back to a safe silent payload when no analyser frame exists yet", () => {
    const payload = buildEngineAudioPayload(undefined);

    expect(payload.bass).toBe(0);
    expect(payload.mid).toBe(0);
    expect(payload.treble).toBe(0);
    expect(payload.beat).toBe(0);
    expect(payload.sampleRate).toBe(44100);
    expect(payload.lanes).toEqual([]);
    expect(payload.cells).toEqual([]);
    expect(payload.wave).toEqual([]);
    expect(payload.freqMag).toEqual([]);
    expect(payload.byteFrequencyData).toEqual([]);
    expect(payload.byteTimeDomainData).toEqual([]);
    for (const name of ENGINE_AUDIO_SCALAR_FEATURES) {
      expect(payload[name]).toBe(0);
    }
  });

  it("coerces non-finite scalar features to zero", () => {
    const input = features();
    input.bass = Number.NaN;
    input.rms = Number.POSITIVE_INFINITY;
    const payload = buildEngineAudioPayload(input);
    expect(payload.bass).toBe(0);
    expect(payload.rms).toBe(0);
  });
});
