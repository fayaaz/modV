import { describe, expect, it } from "vitest";
import {
  SpectrumBars,
  spectrumBarLevels,
} from "../../src/renderer/src/application/point10/scenes/SpectrumBars.js";

const BAR_COUNT = 64;
const FFT_SIZE = 1024;

function spectrumWithBin(bin, value = 255) {
  const data = new Uint8Array(FFT_SIZE);
  data[bin] = value;
  return data;
}

function maxBarIndex(levels) {
  let index = 0;

  for (let i = 1; i < levels.length; i++) {
    if (levels[i] > levels[index]) index = i;
  }

  return index;
}

describe("spectrumBarLevels", () => {
  it("maps a high-frequency bin to a high bar instead of bar 0", () => {
    const levels = spectrumBarLevels(spectrumWithBin(500), BAR_COUNT);

    expect(levels).toHaveLength(BAR_COUNT);
    expect(levels[0]).toBe(0);
    expect(maxBarIndex(levels)).toBeGreaterThan(BAR_COUNT / 2);
    expect(levels[maxBarIndex(levels)]).toBe(255);
  });

  it("includes the top of the spectrum", () => {
    const levels = spectrumBarLevels(spectrumWithBin(FFT_SIZE - 1), BAR_COUNT);

    expect(maxBarIndex(levels)).toBeGreaterThan(BAR_COUNT - 4);
    expect(levels[maxBarIndex(levels)]).toBe(255);
  });

  it("maps a low-frequency bin to an early bar", () => {
    const levels = spectrumBarLevels(spectrumWithBin(1), BAR_COUNT);

    expect(maxBarIndex(levels)).toBeLessThanOrEqual(2);
    expect(levels[maxBarIndex(levels)]).toBe(255);
  });

  it("skips the DC bin", () => {
    const levels = spectrumBarLevels(spectrumWithBin(0), BAR_COUNT);

    expect(levels.every((level) => level === 0)).toBe(true);
  });

  it("returns finite zero levels for empty, null and short spectra", () => {
    for (const spectrum of [
      null,
      undefined,
      new Uint8Array(0),
      new Uint8Array(1),
    ]) {
      const levels = spectrumBarLevels(spectrum, BAR_COUNT);

      expect(levels).toHaveLength(BAR_COUNT);
      expect(levels.every((level) => Number.isFinite(level))).toBe(true);
      expect(levels.every((level) => level === 0)).toBe(true);
    }
  });

  it("keeps every level finite and within 0..255", () => {
    const data = new Uint8Array(FFT_SIZE);

    for (let i = 0; i < data.length; i++) {
      data[i] = (i * 37) % 256;
    }

    const levels = spectrumBarLevels(data, BAR_COUNT);

    expect(levels).toHaveLength(BAR_COUNT);
    for (const level of levels) {
      expect(Number.isFinite(level)).toBe(true);
      expect(level).toBeGreaterThanOrEqual(0);
      expect(level).toBeLessThanOrEqual(255);
    }
  });

  it("uses every bin up to the top so a full spectrum lights every bar", () => {
    const levels = spectrumBarLevels(
      new Uint8Array(FFT_SIZE).fill(255),
      BAR_COUNT,
    );

    expect(levels.every((level) => level === 255)).toBe(true);
  });

  it("drives the 3D bars from the log-spaced levels", () => {
    const scene = new SpectrumBars(null, null);
    const data = spectrumWithBin(500);

    scene.update(0.1, data);

    const highBar = maxBarIndex(spectrumBarLevels(data, BAR_COUNT));

    expect(scene.bars[highBar].scale.y).toBeGreaterThan(4);
    expect(scene.bars[0].scale.y).toBeCloseTo(0.1);
  });
});
