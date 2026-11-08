import { describe, expect, it } from "vitest";
import {
  AUTO_BASS_EXCLUDED_PROPS,
  AUTO_BPM_EXCLUDED_PROPS,
  DEFAULT_SCENE_BPM_TWEEN,
  getBpmDivision,
  getFeatureSmoothing,
  isAutoLinkExcluded,
} from "../../src/renderer/src/application/worker/scene-link-defaults.js";

describe("scene link defaults", () => {
  it("runs the automatic BPM tween slowly with a sine ease", () => {
    expect(DEFAULT_SCENE_BPM_TWEEN.bpmDivision).toBe(32);
    expect(DEFAULT_SCENE_BPM_TWEEN.easing).toBe("easeInOutSine");
    expect(Object.isFrozen(DEFAULT_SCENE_BPM_TWEEN)).toBe(true);
  });

  it("excludes Twirl geometry props for bass and BPM", () => {
    expect(isAutoLinkExcluded("Twirl", "radius", "bass")).toBe(true);
    expect(isAutoLinkExcluded("Twirl", "amount", "bass")).toBe(true);
    expect(isAutoLinkExcluded("Twirl", "radius", "bpm")).toBe(true);
    expect(isAutoLinkExcluded("Twirl", "amount", "bpm")).toBe(true);
    expect(isAutoLinkExcluded("Twirl", "center", "bass")).toBe(false);
  });

  it("excludes Kaleidoscope sides but not unrelated props", () => {
    expect(isAutoLinkExcluded("Kaleidoscope", "sides", "bass")).toBe(true);
    expect(isAutoLinkExcluded("Kaleidoscope", "sides", "bpm")).toBe(true);
    expect(isAutoLinkExcluded("Kaleidoscope", "rotation", "bpm")).toBe(false);
  });

  it("only allows Kaleidoscope Tile rotation for BPM", () => {
    for (const prop of ["sides", "size", "rotation", "angle"]) {
      expect(isAutoLinkExcluded("Kaleidoscope Tile", prop, "bass")).toBe(true);
    }

    expect(isAutoLinkExcluded("Kaleidoscope Tile", "sides", "bpm")).toBe(true);
    expect(isAutoLinkExcluded("Kaleidoscope Tile", "size", "bpm")).toBe(true);
    expect(isAutoLinkExcluded("Kaleidoscope Tile", "angle", "bpm")).toBe(true);
    expect(isAutoLinkExcluded("Kaleidoscope Tile", "rotation", "bpm")).toBe(
      false,
    );
  });

  it("leaves unrelated modules and unknown modes unaffected", () => {
    expect(isAutoLinkExcluded("Bloom", "intensity", "bass")).toBe(false);
    expect(isAutoLinkExcluded("Bloom", "blurAmount", "bpm")).toBe(false);
    expect(isAutoLinkExcluded("Twirl", "radius", "explode")).toBe(false);
    expect(isAutoLinkExcluded(undefined, "radius", "bpm")).toBe(false);
  });

  it("keeps the exported maps keyed by module name", () => {
    expect(AUTO_BASS_EXCLUDED_PROPS.Twirl).toContain("radius");
    expect(AUTO_BPM_EXCLUDED_PROPS.Twirl).toContain("amount");
    expect(AUTO_BASS_EXCLUDED_PROPS["Kaleidoscope Tile"]).toContain("rotation");
    expect(AUTO_BPM_EXCLUDED_PROPS["Kaleidoscope Tile"]).not.toContain(
      "rotation",
    );
  });
});

describe("scene link options", () => {
  it("keeps an explicit integer BPM tween division", () => {
    expect(getBpmDivision(128)).toBe(128);
    expect(getBpmDivision(8)).toBe(8);
    expect(getBpmDivision(1)).toBe(1);
  });

  it("falls back to the default division for unusable input", () => {
    expect(getBpmDivision(undefined)).toBe(32);
    expect(getBpmDivision(NaN)).toBe(32);
    expect(getBpmDivision(0)).toBe(32);
    expect(getBpmDivision(-4)).toBe(32);
    expect(getBpmDivision(3.5)).toBe(32);
    expect(getBpmDivision("abc")).toBe(32);
  });

  it("keeps in-range feature smoothing values", () => {
    expect(getFeatureSmoothing(0.3486748374187093)).toBeCloseTo(
      0.3486748374187093,
      10,
    );
    expect(getFeatureSmoothing(1)).toBe(1);
    expect(getFeatureSmoothing(0.001)).toBe(0.001);
  });

  it("rejects unusable smoothing values", () => {
    expect(getFeatureSmoothing(undefined)).toBeNull();
    expect(getFeatureSmoothing(NaN)).toBeNull();
    expect(getFeatureSmoothing(0)).toBeNull();
    expect(getFeatureSmoothing(-0.5)).toBeNull();
    expect(getFeatureSmoothing(2)).toBeNull();
    expect(getFeatureSmoothing("abc")).toBeNull();
  });
});
