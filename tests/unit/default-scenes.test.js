import { describe, expect, it } from "vitest";
import DEFAULT_SCENES, {
  DEFAULTS_VERSION,
  getDefaultSceneNamesUpTo,
  getScenesToSeed,
} from "../../src/renderer/src/application/worker/store/default-scenes.js";

const GROUP_NAMES = ["Background", "Foreground", "Effects"];

const AUDIO_GLITCH_MODULES = [
  "Point10 Digital Glitch",
  "Point10 RGB Shift",
  "Point10 Color Distort",
  "Point10 Contrast",
];

const KALEIDOSCOPE_MODULES = ["Kaleidoscope", "Kaleidoscope Tile"];

const TWIRL_MODULES = [
  "Twirl",
  "Spiral",
  "UltimateSpiral",
  "rotozoomer",
  "Point10 Swirl Grid",
  "Point10 Banded Swirl",
  "Point10 Smooth Swirl Grid",
];

const sceneKey = (name) =>
  String(name || "")
    .trim()
    .toLowerCase();

const newBatch = DEFAULT_SCENES.filter(
  (scene) => scene.introducedVersion === 3,
);

const getGroup = (scene, name) =>
  scene.groups.find((group) => group.name === name);

const getModuleNames = (scene) =>
  scene.groups.flatMap((group) =>
    (group.modules || []).map((module) => module.name),
  );

const getModulesInGroup = (scene, name) =>
  (getGroup(scene, name)?.modules || []).map((module) => module.name);

describe("default scenes", () => {
  it("bumps the defaults version to 3", () => {
    expect(DEFAULTS_VERSION).toBe(3);
  });

  it("ships at least 33 default scenes with unique names", () => {
    expect(DEFAULT_SCENES.length).toBeGreaterThanOrEqual(33);

    const keys = DEFAULT_SCENES.map((scene) => sceneKey(scene.name));

    expect(new Set(keys).size).toBe(keys.length);
    expect(keys.every((key) => key.length > 0)).toBe(true);
  });

  it("keeps the original scenes on their original introduced version", () => {
    const legacy = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion < 3,
    );

    expect(legacy.length).toBeGreaterThanOrEqual(21);
    expect(
      legacy.every(
        (scene) =>
          scene.introducedVersion === 1 || scene.introducedVersion === 2,
      ),
    ).toBe(true);
  });

  it("adds at least 10 scenes in the version 3 batch", () => {
    expect(newBatch.length).toBeGreaterThanOrEqual(10);
  });

  it("gives every scene the three required groups with the right inherit flags", () => {
    for (const scene of DEFAULT_SCENES) {
      const groupNames = scene.groups.map((group) => group.name);

      expect(groupNames).toEqual(expect.arrayContaining(GROUP_NAMES));

      for (const group of scene.groups) {
        expect(Array.isArray(group.modules)).toBe(true);
        expect(group.modules.length).toBeGreaterThan(0);
        expect(group.inherit).toBe(group.name !== "Background");
        expect(typeof group.clearing).toBe("boolean");
        expect(typeof group.alpha).toBe("number");
        expect(typeof group.compositeOperation).toBe("string");
      }
    }
  });

  it("gives every scene a sensible duration", () => {
    for (const scene of DEFAULT_SCENES) {
      expect(Number.isFinite(Number(scene.minSeconds))).toBe(true);
      expect(Number.isFinite(Number(scene.maxSeconds))).toBe(true);
      expect(Number(scene.minSeconds)).toBeLessThan(Number(scene.maxSeconds));
    }
  });

  it("uses an audio reactive glitch effect in every new scene", () => {
    for (const scene of newBatch) {
      const names = getModuleNames(scene);

      expect(names.some((name) => AUDIO_GLITCH_MODULES.includes(name))).toBe(
        true,
      );
    }
  });

  it("uses a kaleidoscope filter in the new batch", () => {
    const kaleidoscopeScenes = newBatch.filter((scene) =>
      getModuleNames(scene).some((name) => KALEIDOSCOPE_MODULES.includes(name)),
    );

    expect(kaleidoscopeScenes.length).toBeGreaterThanOrEqual(3);
  });

  it("varies the kaleidoscope sides between scenes", () => {
    const sides = newBatch.flatMap((scene) =>
      scene.groups
        .flatMap((group) => group.modules || [])
        .filter((module) => KALEIDOSCOPE_MODULES.includes(module.name))
        .map((module) => module.props?.sides)
        .filter((value) => typeof value === "number"),
    );

    expect(sides.length).toBeGreaterThanOrEqual(3);
    expect(new Set(sides).size).toBeGreaterThanOrEqual(3);
  });

  it("uses a twirled geometric filter in the new batch", () => {
    const twirlScenes = newBatch.filter((scene) =>
      getModuleNames(scene).some((name) => TWIRL_MODULES.includes(name)),
    );

    expect(twirlScenes.length).toBeGreaterThanOrEqual(3);
  });

  it("keeps a glitch effect in the foreground group only in some scenes", () => {
    const foregroundOnly = newBatch.filter((scene) => {
      const foreground = getModulesInGroup(scene, "Foreground");
      const elsewhere = scene.groups
        .filter((group) => group.name !== "Foreground")
        .flatMap((group) => (group.modules || []).map((module) => module.name));

      return foreground.some(
        (name) =>
          AUDIO_GLITCH_MODULES.includes(name) && !elsewhere.includes(name),
      );
    });

    expect(foregroundOnly.length).toBeGreaterThanOrEqual(3);
  });

  it("dims at least one foreground module with a meta override", () => {
    const dimmed = newBatch.filter((scene) =>
      (getGroup(scene, "Foreground")?.modules || []).some(
        (module) =>
          typeof module.meta?.alpha === "number" &&
          module.meta.alpha > 0 &&
          module.meta.alpha < 1,
      ),
    );

    expect(dimmed.length).toBeGreaterThanOrEqual(3);
  });

  it("only seeds the version 3 batch for installs on version 2", () => {
    const selected = getScenesToSeed({
      defaultsVersion: 2,
      seededNames: getDefaultSceneNamesUpTo(2),
      existingNames: [],
    });

    expect(selected.map((scene) => scene.name)).toEqual(
      newBatch.map((scene) => scene.name),
    );
  });

  it("does not offer the version 3 batch to installs on version 3", () => {
    const selected = getScenesToSeed({
      defaultsVersion: DEFAULTS_VERSION,
      seededNames: getDefaultSceneNamesUpTo(DEFAULTS_VERSION),
      existingNames: [],
    });

    expect(selected).toEqual([]);
  });
});
