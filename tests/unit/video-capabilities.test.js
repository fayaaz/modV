import { describe, expect, it } from "vitest";
import {
  buildFrameRateOptions,
  buildResolutionOptions,
  buildVideoOptions,
} from "../../src/renderer/src/application/video-capabilities.js";

function resolutionKeys(options) {
  return options.map(({ width, height }) => `${width}x${height}`);
}

function frameRateValues(options) {
  return options.map(({ value }) => value);
}

describe("buildResolutionOptions", () => {
  it("offers only modes inside the device's reported range", () => {
    const options = buildResolutionOptions(
      {
        width: { min: 320, max: 1920 },
        height: { min: 240, max: 1080 },
      },
      { width: 1280, height: 720 },
    );

    expect(resolutionKeys(options)).toContain("640x480");
    expect(resolutionKeys(options)).toContain("1280x720");
    expect(resolutionKeys(options)).toContain("1920x1080");
    expect(resolutionKeys(options)).not.toContain("2560x1440");
    expect(resolutionKeys(options)).not.toContain("320x180");
  });

  it("includes the device maximum and current mode", () => {
    const options = buildResolutionOptions(
      {
        width: { min: 320, max: 2560 },
        height: { min: 200, max: 1440 },
      },
      { width: 2304, height: 1296 },
    );

    expect(resolutionKeys(options)).toContain("2560x1440");
    expect(resolutionKeys(options)).toContain("2304x1296");
  });

  it("supports explicit capability value lists", () => {
    const options = buildResolutionOptions(
      {
        width: [640, 1280, 1920],
        height: [480, 720, 1080],
      },
      {},
    );

    expect(resolutionKeys(options)).toEqual(
      expect.arrayContaining(["640x480", "1280x720", "1920x1080"]),
    );
    expect(resolutionKeys(options)).not.toContain("800x600");
  });

  it("falls back to the common ladder when the device reports nothing", () => {
    const options = buildResolutionOptions({}, {});

    expect(resolutionKeys(options)).toContain("640x480");
    expect(resolutionKeys(options)).toContain("1920x1080");
  });
});

describe("buildFrameRateOptions", () => {
  it("offers rates inside the device's reported range", () => {
    const options = buildFrameRateOptions(
      { frameRate: { min: 15, max: 60 } },
      { frameRate: 30 },
    );

    expect(frameRateValues(options)).toContain(15);
    expect(frameRateValues(options)).toContain(30);
    expect(frameRateValues(options)).toContain(60);
    expect(frameRateValues(options)).not.toContain(120);
  });

  it("rounds and includes an unusual maximum rate", () => {
    const options = buildFrameRateOptions(
      { frameRate: { min: 1, max: 29.97 } },
      { frameRate: 29.97 },
    );

    expect(frameRateValues(options)).toContain(30);
  });

  it("supports explicit capability value lists", () => {
    const options = buildFrameRateOptions(
      { frameRate: [24, 25, 50] },
      { frameRate: 50 },
    );

    expect(frameRateValues(options)).toEqual([24, 25, 50]);
  });

  it("falls back to the common ladder when the device reports nothing", () => {
    const options = buildFrameRateOptions({}, {});

    expect(frameRateValues(options)).toContain(30);
    expect(frameRateValues(options)).toContain(60);
  });
});

describe("buildVideoOptions", () => {
  it("returns de-duplicated, sorted resolutions and frame rates", () => {
    const { resolutions, frameRates } = buildVideoOptions(
      {
        width: { min: 640, max: 1920 },
        height: { min: 480, max: 1080 },
        frameRate: { min: 30, max: 60 },
      },
      { width: 1280, height: 720, frameRate: 60 },
    );

    const keys = resolutionKeys(resolutions);
    expect(keys).toEqual([...new Set(keys)]);

    const areas = resolutions.map(({ width, height }) => width * height);
    expect(areas).toEqual([...areas].sort((a, b) => a - b));

    expect(frameRates.map(({ label }) => label)).toContain("60 fps");
  });
});
