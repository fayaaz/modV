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

const variedBatch = DEFAULT_SCENES.filter(
  (scene) => scene.introducedVersion === 4,
);

const filterBatch = DEFAULT_SCENES.filter(
  (scene) => scene.introducedVersion === 5,
);

const FILTER_MODULES = [
  "ASCII Art",
  "CMYK Halftone",
  "Color Invert",
  "Color Posterize",
  "Dither-Bayer",
  "Dot Screen",
  "Edge Trace",
  "Edges",
  "Emboss",
  "Pixellate",
  "Radial Replicate",
  "RGB Halftone",
];

const getGroup = (scene, name) =>
  scene.groups.find((group) => group.name === name);

const getModuleNames = (scene) =>
  scene.groups.flatMap((group) =>
    (group.modules || []).map((module) => module.name),
  );

const getModulesInGroup = (scene, name) =>
  (getGroup(scene, name)?.modules || []).map((module) => module.name);

describe("default scenes", () => {
  it("bumps the defaults version to 5", () => {
    expect(DEFAULTS_VERSION).toBe(5);
  });

  it("ships at least 55 default scenes with unique names", () => {
    expect(DEFAULT_SCENES.length).toBeGreaterThanOrEqual(55);

    const keys = DEFAULT_SCENES.map((scene) => sceneKey(scene.name));

    expect(new Set(keys).size).toBe(keys.length);
    expect(keys.every((key) => key.length > 0)).toBe(true);
  });

  it("keeps the original scenes on their original introduced version", () => {
    const legacy = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion < 3,
    );

    expect(legacy.length).toBeGreaterThanOrEqual(20);
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

  it("adds at least 12 varied Point10 scenes in the version 4 batch", () => {
    expect(variedBatch.length).toBeGreaterThanOrEqual(12);
  });

  it("uses the previously unused Point10 scenes in the version 4 batch", () => {
    const unusedPoint10Scenes = [
      "Point10 Around Plus",
      "Point10 Attractor Particles",
      "Point10 Boxy",
      "Point10 DVD Room",
      "Point10 It Is Coming For You",
      "Point10 Light Tunnel",
      "Point10 Square Tunnel",
      "Point10 Wormhole",
    ];
    const names = new Set(variedBatch.flatMap(getModuleNames));

    for (const name of unusedPoint10Scenes) {
      expect(names.has(name), name).toBe(true);
    }
  });

  it("does not use the removed Point10 Ring Of Fire module", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("Point10 Ring Of Fire"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("pins some version 4 foreground or background controls to bass", () => {
    const withLinks = variedBatch.filter(
      (scene) => Array.isArray(scene.inputLinks) && scene.inputLinks.length > 0,
    );

    expect(withLinks.length).toBeGreaterThanOrEqual(8);

    for (const scene of withLinks) {
      for (const link of scene.inputLinks) {
        expect(["Background", "Foreground"]).toContain(link.group);
        expect(link.control).toBe("alpha");
        expect(typeof link.feature).toBe("string");
        expect(link.min).toBeLessThan(link.max);

        const group = getGroup(scene, link.group);

        expect(group).toBeTruthy();

        if (link.module) {
          expect(getModulesInGroup(scene, link.group)).toContain(link.module);
        }
      }
    }
  });

  it("adds a varied version 5 filter batch with ASCII Art", () => {
    expect(filterBatch.length).toBeGreaterThanOrEqual(8);

    const asciiScenes = filterBatch.filter((scene) =>
      getModuleNames(scene).includes("ASCII Art"),
    );

    expect(asciiScenes.length).toBeGreaterThanOrEqual(2);

    const usedFilters = new Set(
      filterBatch.flatMap((scene) =>
        getModuleNames(scene).filter((name) => FILTER_MODULES.includes(name)),
      ),
    );

    expect(usedFilters.size).toBeGreaterThanOrEqual(8);
    expect(usedFilters.has("ASCII Art")).toBe(true);
  });

  it("does not use the Webcam module in any default scene", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("Webcam"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
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

  it("does not use Dirty Lens in any default scene", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("Dirty Lens"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("does not use the removed Point10 Disco Ball module", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("Point10 Disco Ball"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
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

  it("seeds the version 3, 4 and 5 batches for installs on version 2", () => {
    const selected = getScenesToSeed({
      defaultsVersion: 2,
      seededNames: getDefaultSceneNamesUpTo(2),
      existingNames: [],
    });
    const newerBatch = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion > 2,
    );

    expect(selected.map((scene) => scene.name)).toEqual(
      newerBatch.map((scene) => scene.name),
    );
  });

  it("seeds the version 4 and 5 batches for installs on version 3", () => {
    const selected = getScenesToSeed({
      defaultsVersion: 3,
      seededNames: getDefaultSceneNamesUpTo(3),
      existingNames: [],
    });
    const newerBatch = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion > 3,
    );

    expect(selected.map((scene) => scene.name)).toEqual(
      newerBatch.map((scene) => scene.name),
    );
  });

  it("does not offer any default batch to installs on the latest version", () => {
    const selected = getScenesToSeed({
      defaultsVersion: DEFAULTS_VERSION,
      seededNames: getDefaultSceneNamesUpTo(DEFAULTS_VERSION),
      existingNames: [],
    });

    expect(selected).toEqual([]);
  });
});
