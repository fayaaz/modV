import { describe, expect, it } from "vitest";
import {
  AUTO_BASS_EXCLUDED_PROPS,
  AUTO_BPM_EXCLUDED_PROPS,
  DEFAULT_SCENE_BPM_TWEEN,
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
