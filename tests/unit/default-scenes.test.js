import { describe, expect, it } from "vitest";
import DEFAULT_SCENES, {
  DEFAULTS_VERSION,
  REFRESHED_DEFAULT_SCENES,
  REMOVED_DEFAULT_SCENES,
  getDefaultSceneNamesUpTo,
  getDefaultScenesToRemove,
  getDefaultScenesToReplace,
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

const EFFECT_MODULES = [
  "Bloom",
  "Chromatic Abberation",
  "Vignette",
  "Mirror",
  "VHS Glitch",
  "RGB Halftone",
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

const digitalBatch = DEFAULT_SCENES.filter(
  (scene) => scene.introducedVersion === 6,
);

const version7Batch = DEFAULT_SCENES.filter(
  (scene) => scene.introducedVersion === 7,
);

const REMOVED_CUBE_FLOOR_MODULE = "Point10 Cube Floor";

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
  it("bumps the defaults version to 11", () => {
    expect(DEFAULTS_VERSION).toBe(11);
  });

  it("ships at least 54 default scenes with unique names", () => {
    expect(DEFAULT_SCENES.length).toBeGreaterThanOrEqual(54);

    const keys = DEFAULT_SCENES.map((scene) => sceneKey(scene.name));

    expect(new Set(keys).size).toBe(keys.length);
    expect(keys.every((key) => key.length > 0)).toBe(true);
  });

  it("keeps the original scenes on their original introduced version", () => {
    const legacy = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion < 3,
    );

    expect(legacy.length).toBeGreaterThanOrEqual(15);
    expect(
      legacy.every(
        (scene) =>
          scene.introducedVersion === 1 || scene.introducedVersion === 2,
      ),
    ).toBe(true);
  });

  it("adds at least 9 scenes in the version 3 batch", () => {
    expect(newBatch.length).toBeGreaterThanOrEqual(9);
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

  it("pins some version 4 alpha controls to bass", () => {
    const withAlphaLinks = variedBatch
      .map((scene) => ({
        scene,
        links: (scene.inputLinks || []).filter(
          (link) => link.control === "alpha",
        ),
      }))
      .filter((entry) => entry.links.length > 0);

    expect(withAlphaLinks.length).toBeGreaterThanOrEqual(8);

    for (const { scene, links } of withAlphaLinks) {
      for (const link of links) {
        expect(GROUP_NAMES).toContain(link.group);
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

  it("adds a version 6 Digital Crystal Tunnel scene", () => {
    const scene = digitalBatch.find((candidate) =>
      getModulesInGroup(candidate, "Background").includes(
        "digital-crystal-tunnel",
      ),
    );

    expect(scene).toBeTruthy();
    expect(scene.name).toBe("Digital Crystal Tunnel");
    expect(Number(scene.minSeconds)).toBeLessThan(Number(scene.maxSeconds));

    for (const name of GROUP_NAMES) {
      expect(getGroup(scene, name), name).toBeTruthy();
    }
  });

  it("adds a version 6 MBOX Light Tunnel scene with audio reactive layers", () => {
    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "MBOX Light Tunnel",
    );

    expect(scene).toBeTruthy();
    expect(scene.introducedVersion).toBe(6);
    expect(digitalBatch).toContain(scene);
    expect(Number(scene.minSeconds)).toBeLessThan(Number(scene.maxSeconds));
    expect(getModulesInGroup(scene, "Background")).toContain("MBOX3");
    expect(getModulesInGroup(scene, "Foreground")).toContain(
      "Point10 Light Tunnel",
    );

    const alphaLink = (scene.inputLinks || []).find(
      (link) =>
        link.group === "Foreground" &&
        link.module === "Point10 Light Tunnel" &&
        link.control === "alpha" &&
        typeof link.feature === "string" &&
        link.feature.length > 0,
    );

    expect(alphaLink).toBeTruthy();
    expect(alphaLink.min).toBeLessThan(alphaLink.max);
  });

  it("drives the MBOX hue with a beat tween, scale with bass and a near-zero threshold", () => {
    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "MBOX Light Tunnel",
    );

    expect(scene).toBeTruthy();

    const background = getGroup(scene, "Background");
    const mbox = (background?.modules || []).find(
      (module) => module.name === "MBOX3",
    );

    expect(mbox).toBeTruthy();
    expect(Number(mbox.props.dthresh)).toBeLessThanOrEqual(0.1);
    expect(Number(mbox.props.speed)).toBeLessThanOrEqual(30);

    const mboxLinks = (scene.inputLinks || []).filter(
      (link) => link.group === "Background" && link.module === "MBOX3",
    );

    const scaleLink = mboxLinks.find((link) => link.control === "SCALE");

    expect(scaleLink).toBeTruthy();
    expect(scaleLink.source).not.toBe("tween");
    expect(scaleLink.feature).toBe("bass");
    expect(scaleLink.min).toBeLessThan(scaleLink.max);

    const hueLink = mboxLinks.find((link) => link.control === "dhue");

    expect(hueLink).toBeTruthy();
    expect(hueLink.source).toBe("tween");
    expect(hueLink.min).toBeLessThan(hueLink.max);

    const speedLink = mboxLinks.find((link) => link.control === "speed");

    expect(
      speedLink,
      "MBOX speed should stay constant rather than follow audio",
    ).toBeUndefined();

    const trebleHueLink = mboxLinks.find(
      (link) => link.control === "dhue" && link.feature === "treble",
    );

    expect(trebleHueLink).toBeUndefined();
  });

  it("does not use the removed Point10 Cube Floor module", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes(REMOVED_CUBE_FLOOR_MODULE),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("renames Cube Floor Drive to Waveform Drive", () => {
    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Waveform Drive",
    );

    expect(scene).toBeTruthy();
    expect(scene.introducedVersion).toBe(2);
    expect(
      DEFAULT_SCENES.some((candidate) => candidate.name === "Cube Floor Drive"),
    ).toBe(false);
    // The background Waveform was removed in version 9 and the row is left
    // empty rather than filled with a replacement.
    expect(getModulesInGroup(scene, "Background")).toEqual([]);
  });

  it("adds the version 7 Circular Mountain scene", () => {
    expect(version7Batch.length).toBeGreaterThanOrEqual(1);

    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Circular Mountain",
    );

    expect(scene).toBeTruthy();
    expect(scene.introducedVersion).toBe(7);
    expect(version7Batch).toContain(scene);
    expect(Number(scene.minSeconds)).toBeLessThan(Number(scene.maxSeconds));

    for (const name of GROUP_NAMES) {
      expect(getGroup(scene, name), name).toBeTruthy();
    }

    expect(getModulesInGroup(scene, "Background")).toEqual([
      "fractal-mountain-tunnel",
    ]);
    expect(getModulesInGroup(scene, "Foreground")).toEqual(
      expect.arrayContaining(["Circular Screen", "Corner Color Tint", "badtv"]),
    );
    expect(getModulesInGroup(scene, "Effects")).toEqual(["Bloom"]);

    const links = scene.inputLinks || [];
    const findLink = (module, control) =>
      links.find((link) => link.module === module && link.control === control);

    expect(findLink("fractal-mountain-tunnel", "speed")).toMatchObject({
      group: "Background",
      feature: "bass",
      min: 0,
      max: 1,
    });
    expect(findLink("Corner Color Tint", "alpha")).toMatchObject({
      group: "Foreground",
      feature: "rms",
      min: 0,
      max: 2,
    });
    expect(findLink("Corner Color Tint", "rotationAngle")).toMatchObject({
      group: "Foreground",
      feature: "rms",
      min: 0,
      max: 1,
    });
    expect(findLink("badtv", "scanLineIntensity")).toMatchObject({
      group: "Foreground",
      feature: "treble",
      min: 0,
      max: 1,
    });
  });

  it("does not use the Kaleidoscope or Kaleidoscope Tile modules in any default scene", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).some((name) => KALEIDOSCOPE_MODULES.includes(name)),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("does not use the Radial Replicate filter in any default scene", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("Radial Replicate"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("leaves Effects rows empty when Radial Replicate was the only effect", () => {
    for (const name of ["Hex Bloom", "Hexagon Halo"]) {
      const scene = DEFAULT_SCENES.find((candidate) => candidate.name === name);

      expect(scene, name).toBeTruthy();
      expect(getModulesInGroup(scene, "Effects"), name).toEqual([]);
    }
  });

  it("does not ship Roto Zoom Riot and retires it from existing installs", () => {
    expect(
      DEFAULT_SCENES.some((scene) => scene.name === "Roto Zoom Riot"),
    ).toBe(false);
    expect(REMOVED_DEFAULT_SCENES).toContain("Roto Zoom Riot");
    expect(REFRESHED_DEFAULT_SCENES).not.toContain("Roto Zoom Riot");
  });

  it("uses circular screen, posterize and ASCII Art filters in Effects groups", () => {
    const effectsModules = DEFAULT_SCENES.map((scene) =>
      getModulesInGroup(scene, "Effects"),
    );

    expect(
      effectsModules.some((names) => names.includes("Circular Screen")),
    ).toBe(true);
    expect(
      effectsModules.some(
        (names) =>
          names.includes("Posterize") || names.includes("Color Posterize"),
      ),
    ).toBe(true);
    expect(effectsModules.some((names) => names.includes("ASCII Art"))).toBe(
      true,
    );
  });

  it("does not use the Webcam module in any default scene", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("Webcam"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("lets a Scene use any subset of the Group roles", () => {
    for (const scene of DEFAULT_SCENES) {
      const groupNames = scene.groups.map((group) => group.name);

      expect(groupNames.length).toBeGreaterThanOrEqual(1);
      expect(groupNames.length).toBeLessThanOrEqual(GROUP_NAMES.length);
      expect(new Set(groupNames).size).toBe(groupNames.length);

      for (const name of groupNames) {
        expect(GROUP_NAMES).toContain(name);
      }

      for (let i = 0; i < scene.groups.length; i++) {
        const group = scene.groups[i];

        expect(Array.isArray(group.modules)).toBe(true);
        // An intentionally empty Group is allowed - Scenes may leave a row
        // blank instead of filling it with a replacement visual.
        // The first Group is the base layer; later Groups inherit the frame
        // produced by the row above.
        expect(group.inherit).toBe(i > 0);
        expect(typeof group.clearing).toBe("boolean");
        expect(typeof group.alpha).toBe("number");
        expect(typeof group.compositeOperation).toBe("string");
      }

      expect(getModuleNames(scene).length).toBeGreaterThan(0);
    }
  });

  it("does not use plain Waveform or Bar modules", () => {
    const banned = ["Waveform", "Point10 Waveform", "Bar"];
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).some((name) => banned.includes(name)),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("leaves rows empty rather than replacing a removed waveform or bar", () => {
    const emptyGroups = DEFAULT_SCENES.flatMap((scene) =>
      scene.groups
        .filter((group) => group.modules.length === 0)
        .map((group) => `${scene.name}:${group.name}`),
    );

    expect(emptyGroups).toEqual(
      expect.arrayContaining([
        "Mycelium Bloom:Foreground",
        "Waveform Drive:Background",
        "Terminal Bloom:Foreground",
      ]),
    );
  });

  it("gives every scene a sensible duration", () => {
    for (const scene of DEFAULT_SCENES) {
      expect(Number.isFinite(Number(scene.minSeconds))).toBe(true);
      expect(Number.isFinite(Number(scene.maxSeconds))).toBe(true);
      expect(Number(scene.minSeconds)).toBeLessThan(Number(scene.maxSeconds));
    }
  });

  it("uses an audio reactive glitch effect in most of the new batch", () => {
    const withGlitch = newBatch.filter((scene) =>
      getModuleNames(scene).some((name) => AUDIO_GLITCH_MODULES.includes(name)),
    );

    // The batch is glitch-led, but not every Scene needs one on top of its
    // other layers - the busy defaults were trimmed back in version 9.
    expect(withGlitch.length).toBeGreaterThanOrEqual(8);
  });

  it("keeps Twirl only in It Twirls At Night", () => {
    const twirlScenes = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("Twirl"),
    );

    expect(twirlScenes.map((scene) => scene.name)).toEqual([
      "It Twirls At Night",
    ]);
  });

  it("does not use RGB Halftone in any default scene", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModuleNames(scene).includes("RGB Halftone"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("does not put the Mesmer 2D Scene in an Effects group", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModulesInGroup(scene, "Effects").includes("Mesmer"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("does not put the Bands 2D shape in an Effects group", () => {
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModulesInGroup(scene, "Effects").includes("Bands"),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);
  });

  it("keeps Strobe subtle and synced to the beat", () => {
    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Strobe Riot",
    );

    expect(scene).toBeTruthy();

    const strobe = getGroup(scene, "Effects").modules.find(
      (module) => module.name === "Strobe",
    );

    expect(strobe).toBeTruthy();
    expect(strobe.meta.alpha).toBeLessThanOrEqual(0.25);
    expect(Number(strobe.props.strobeRate)).toBeGreaterThanOrEqual(0.4);

    const link = (scene.inputLinks || []).find(
      (candidate) =>
        candidate.module === "Strobe" && candidate.control === "strobeRate",
    );

    expect(link).toMatchObject({ group: "Effects", source: "tween" });
    expect(link.min).toBeLessThan(link.max);
    expect(link.bpmDivision).toBeGreaterThan(0);
  });

  it("keeps RGB Shift and Digital Glitch in Effects groups", () => {
    const glitchEffects = ["Point10 RGB Shift", "Point10 Digital Glitch"];
    const offenders = DEFAULT_SCENES.filter((scene) =>
      getModulesInGroup(scene, "Foreground").some((name) =>
        glitchEffects.includes(name),
      ),
    );

    expect(offenders.map((scene) => scene.name)).toEqual([]);

    const scenesWithEffects = DEFAULT_SCENES.filter((scene) =>
      getModulesInGroup(scene, "Effects").some((name) =>
        glitchEffects.includes(name),
      ),
    ).map((scene) => scene.name);

    expect(scenesWithEffects).toEqual(
      expect.arrayContaining([
        "Prism Reliquary",
        "Glitch Bloom Cathedral",
        "VHS Reliquary",
        "It Is Coming",
        "It Twirls At Night",
        "Boxy Arcade",
        "DVD Nightmare",
        "Rave.exe",
      ]),
    );
  });

  it("moves the background into Foreground when the glitch effect was the only foreground layer", () => {
    const names = [
      "Prism Reliquary",
      "Glitch Bloom Cathedral",
      "VHS Reliquary",
      "It Is Coming",
      "It Twirls At Night",
      "Boxy Arcade",
      "DVD Nightmare",
      "Rave.exe",
    ];

    for (const name of names) {
      const scene = DEFAULT_SCENES.find((candidate) => candidate.name === name);

      expect(scene, name).toBeTruthy();
      expect(
        scene.groups.map((group) => group.name),
        name,
      ).toEqual(["Foreground", "Effects"]);
      expect(
        getModulesInGroup(scene, "Foreground").length,
        name,
      ).toBeGreaterThan(0);
      expect(getModulesInGroup(scene, "Effects").length, name).toBeGreaterThan(
        1,
      );
      expect(
        (scene.inputLinks || []).some((link) => link.group === "Background"),
        name,
      ).toBe(false);
    }
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

  it("seeds the version 6 batch for installs on version 5", () => {
    const selected = getScenesToSeed({
      defaultsVersion: 5,
      seededNames: getDefaultSceneNamesUpTo(5),
      existingNames: [],
    });
    const newerBatch = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion > 5,
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

describe("default scene migration", () => {
  const cubeFloorScene = (name, id = name) => ({
    id,
    name,
    modules: {
      "module-id": {
        $moduleName: REMOVED_CUBE_FLOOR_MODULE,
      },
    },
  });

  it("rebuilds existing defaults that still use a removed module", () => {
    const scenes = [
      cubeFloorScene("Smoke Tunnel", "smoke-tunnel-id"),
      cubeFloorScene("Mycelium Bloom", "mycelium-id"),
      cubeFloorScene("Dither District", "dither-id"),
    ];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 6,
      scenes,
      removedModuleNames: [REMOVED_CUBE_FLOOR_MODULE],
    });

    expect(result.removals).toEqual([]);
    expect(result.additions).toEqual([]);
    expect(result.replacements.map((replacement) => replacement.name)).toEqual(
      expect.arrayContaining([
        "Smoke Tunnel",
        "Mycelium Bloom",
        "Dither District",
      ]),
    );

    const smokeTunnel = result.replacements.find(
      (replacement) => replacement.name === "Smoke Tunnel",
    );

    expect(smokeTunnel.id).toBe("smoke-tunnel-id");
    expect(smokeTunnel.definition.name).toBe("Smoke Tunnel");
    expect(
      smokeTunnel.definition.groups.flatMap((group) =>
        group.modules.map((module) => module.name),
      ),
    ).not.toContain(REMOVED_CUBE_FLOOR_MODULE);

    for (const replacement of result.replacements) {
      expect(
        replacement.definition.groups.flatMap((group) =>
          group.modules.map((module) => module.name),
        ),
      ).not.toContain(REMOVED_CUBE_FLOOR_MODULE);
    }
  });

  it("preserves user edits when a scene no longer uses the removed module", () => {
    const scenes = [
      {
        id: "smoke-tunnel-id",
        name: "Smoke Tunnel",
        modules: {
          "module-id": { $moduleName: "Point10 Light Tunnel" },
        },
      },
    ];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 6,
      scenes,
      removedModuleNames: [REMOVED_CUBE_FLOOR_MODULE],
    });

    expect(result.replacements).toEqual([]);
    expect(result.removals).toEqual([]);
    expect(result.additions).toEqual([]);
  });

  it("removes the renamed Cube Floor Drive and seeds Waveform Drive", () => {
    const scenes = [cubeFloorScene("Cube Floor Drive", "cube-drive-id")];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 6,
      scenes,
      removedModuleNames: [REMOVED_CUBE_FLOOR_MODULE],
    });

    expect(result.replacements).toEqual([]);
    expect(result.removals).toEqual([
      { id: "cube-drive-id", name: "Cube Floor Drive" },
    ]);
    expect(result.additions.map((definition) => definition.name)).toEqual([
      "Waveform Drive",
    ]);
  });

  it("ignores user scenes that do not share a default name", () => {
    const scenes = [cubeFloorScene("My Cube Floor Experiment")];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 6,
      scenes,
      removedModuleNames: [REMOVED_CUBE_FLOOR_MODULE],
    });

    expect(result.replacements).toEqual([]);
    expect(result.removals).toEqual([]);
    expect(result.additions).toEqual([]);
  });

  it("does not migrate installs that are already on the new defaults version", () => {
    const scenes = [cubeFloorScene("Smoke Tunnel", "smoke-tunnel-id")];
    const result = getDefaultScenesToReplace({
      defaultsVersion: DEFAULTS_VERSION,
      scenes,
      removedModuleNames: [REMOVED_CUBE_FLOOR_MODULE],
    });

    expect(result.replacements).toEqual([]);
    expect(result.removals).toEqual([]);
    expect(result.additions).toEqual([]);
  });
});

describe("version 8 default scenes", () => {
  it("removes the retired scenes", () => {
    expect(REMOVED_DEFAULT_SCENES).toEqual([
      "Ferrofluid Pulse",
      "Kaleidoscope Cathedral",
      "Slit Scan Horizon",
      "Spiral Particles",
      "Spectral Bloom",
      "Fractured Kaleidoscope",
      "Broken Mirror Ball",
      "Roto Zoom Riot",
    ]);

    const names = DEFAULT_SCENES.map((scene) => sceneKey(scene.name));

    for (const removed of REMOVED_DEFAULT_SCENES) {
      expect(names).not.toContain(sceneKey(removed));
    }
  });

  it("renames Chromatic Sphere to Chromatic Waveform", () => {
    expect(
      DEFAULT_SCENES.some((scene) => scene.name === "Chromatic Sphere"),
    ).toBe(false);

    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Chromatic Waveform",
    );

    expect(scene).toBeTruthy();
    expect(getModuleNames(scene)).toContain("Point10 Chromatic Wave");
  });

  it("renames and updates VHS Afterglow to Digital Black Hole", () => {
    expect(DEFAULT_SCENES.some((scene) => scene.name === "VHS Afterglow")).toBe(
      false,
    );

    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Digital Black Hole",
    );

    expect(scene).toBeTruthy();

    const foreground = getGroup(scene, "Foreground");
    const rain = foreground.modules.find(
      (module) => module.name === "Singularity Rain",
    );

    expect(rain).toBeTruthy();
    expect(rain.props.speed).toBe(0);
    expect(rain.props.swirl).toBeCloseTo(0.5642799587694213, 10);
    expect(rain.meta.alpha).toBeCloseTo(0.607261398057038, 10);
    expect(foreground.modules.some((module) => module.name === "Bar")).toBe(
      false,
    );

    const effects = getGroup(scene, "Effects");
    const glitch = effects.modules.find(
      (module) => module.name === "VHS Glitch",
    );

    expect(glitch.props.xScanlineSize).toBe(0);
    expect(glitch.props.grainLevel).toBe(1.65);
    expect(glitch.props.analogDistort).toBe(0.5);

    const links = scene.inputLinks || [];
    const alphaLink = links.find(
      (link) => link.module === "Singularity Rain" && link.control === "alpha",
    );

    expect(alphaLink).toMatchObject({
      group: "Foreground",
      feature: "bass",
      min: 0,
      max: 1,
    });

    const midLink = links.find(
      (link) => link.module === "Singularity Rain" && link.control === "glow",
    );

    expect(midLink).toMatchObject({
      group: "Foreground",
      feature: "mid",
      min: 0,
      max: 1,
    });
    expect(midLink.smoothing).toBeCloseTo(0.3486748374187093, 10);

    const tweenLink = links.find(
      (link) => link.module === "VHS Glitch" && link.control === "xScanline",
    );

    expect(tweenLink).toMatchObject({
      group: "Effects",
      source: "tween",
      min: 0,
      max: 1,
      bpmDivision: 128,
    });

    const bleedLink = links.find(
      (link) => link.module === "VHS Glitch" && link.control === "bleedAmount",
    );

    expect(bleedLink).toMatchObject({
      group: "Effects",
      feature: "bass",
      min: 0.54,
      max: 1.26,
    });

    expect(Number(scene.minSeconds)).toBe(8);
    expect(Number(scene.maxSeconds)).toBe(24);
  });

  it("renames and updates It Comes At Night to It Twirls At Night", () => {
    expect(
      DEFAULT_SCENES.some((scene) => scene.name === "It Comes At Night"),
    ).toBe(false);

    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "It Twirls At Night",
    );

    expect(scene).toBeTruthy();

    const foreground = getGroup(scene, "Foreground");
    const itIsComing = foreground.modules.find(
      (module) => module.name === "Point10 It Is Coming For You",
    );

    expect(itIsComing.meta.alpha).toBeCloseTo(0.7446732251408736, 10);
    expect(itIsComing.props.smoothing).toBe(0);

    const effects = getGroup(scene, "Effects");
    const twirl = effects.modules.find((module) => module.name === "Twirl");

    expect(twirl.meta.alpha).toBeCloseTo(0.616, 10);

    const bloom = effects.modules.find((module) => module.name === "Bloom");

    expect(bloom.props.blurAmount).toBeCloseTo(5.710711967387224, 10);

    const links = scene.inputLinks || [];
    const foregroundLink = links.find(
      (link) =>
        link.group === "Foreground" &&
        link.module === "Point10 It Is Coming For You" &&
        link.control === "alpha",
    );

    expect(foregroundLink).toMatchObject({
      feature: "bass",
      min: 0.25,
      max: 1,
    });

    const tweenLink = links.find(
      (link) => link.module === "Bloom" && link.control === "blurAmount",
    );

    expect(tweenLink.group).toBe("Effects");
    expect(tweenLink.source).toBe("tween");
    expect(tweenLink.min).toBeCloseTo(3.6, 5);
    expect(tweenLink.max).toBeCloseTo(8.4, 5);

    const bassLink = links.find(
      (link) => link.module === "Bloom" && link.control === "intensity",
    );

    expect(bassLink).toMatchObject({
      group: "Effects",
      feature: "bass",
      min: 0.3,
      max: 0.7,
    });
  });

  it("renames and updates Neon Grid to Smoke Tunnel", () => {
    expect(DEFAULT_SCENES.some((scene) => scene.name === "Neon Grid")).toBe(
      false,
    );

    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Smoke Tunnel",
    );

    expect(scene).toBeTruthy();
    expect(getModulesInGroup(scene, "Foreground")).toContain(
      "Point10 Smoke Room",
    );

    const effects = getGroup(scene, "Effects");
    const neon = effects.modules.find((module) => module.name === "Neon");

    expect(neon.props.intensity).toBeCloseTo(3.6, 5);
    expect(neon.props.neonColor).toEqual([1, 0.4, 0.64, 1]);

    const links = scene.inputLinks || [];
    const intensityLink = links.find(
      (link) => link.module === "Neon" && link.control === "intensity",
    );

    expect(intensityLink.group).toBe("Effects");
    expect(intensityLink.feature).toBe("bass");
    expect(intensityLink.min).toBeCloseTo(3.6, 5);
    expect(intensityLink.max).toBeCloseTo(8.4, 5);

    const gainLink = links.find(
      (link) => link.module === "Neon" && link.control === "gain",
    );

    expect(gainLink.group).toBe("Effects");
    expect(gainLink.source).toBe("tween");
    expect(gainLink.min).toBeCloseTo(0.6, 5);
    expect(gainLink.max).toBe(1);

    expect(Number(scene.minSeconds)).toBe(8);
    expect(Number(scene.maxSeconds)).toBe(24);
  });

  it("drops Twirl from Twirl Tunnel and renames it", () => {
    expect(DEFAULT_SCENES.some((scene) => scene.name === "Twirl Tunnel")).toBe(
      false,
    );

    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Golden Spiral Tunnel",
    );

    expect(scene).toBeTruthy();

    const names = getModuleNames(scene);

    expect(names).not.toContain("Twirl");
    expect(names).toContain("LogTransWarpSpiral");
    expect(names).toContain("Hue-Saturation");
    expect(getModulesInGroup(scene, "Background")).toEqual([
      "Point10 Geometric Tunnel",
    ]);
  });

  it("adds a version 8 rave.exe preset", () => {
    const scene = DEFAULT_SCENES.find(
      (candidate) => candidate.name === "Rave.exe",
    );

    expect(scene).toBeTruthy();
    expect(scene.introducedVersion).toBe(8);
    expect(getModulesInGroup(scene, "Foreground")).toContain("rave.exe");

    const kickLink = (scene.inputLinks || []).find(
      (link) => link.module === "rave.exe" && link.control === "kick",
    );

    expect(kickLink).toMatchObject({
      group: "Foreground",
      feature: "bass",
      min: 0,
      max: 1,
    });

    expect(Number(scene.minSeconds)).toBeLessThan(Number(scene.maxSeconds));
  });

  it("seeds the version 8 batch for installs on version 7", () => {
    const selected = getScenesToSeed({
      defaultsVersion: 7,
      seededNames: getDefaultSceneNamesUpTo(7),
      existingNames: [],
    });
    const newerBatch = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion > 7,
    );

    expect(selected.map((scene) => scene.name)).toEqual(
      newerBatch.map((scene) => scene.name),
    );
  });

  it("does not offer the removed scenes to fresh installs", () => {
    const selected = getScenesToSeed({
      defaultsVersion: 0,
      seededNames: [],
      existingNames: [],
    });
    const names = selected.map((scene) => scene.name);

    for (const removed of REMOVED_DEFAULT_SCENES) {
      expect(names).not.toContain(removed);
    }
  });
});

describe("version 9 sparse default scenes", () => {
  const sparseBatch = DEFAULT_SCENES.filter(
    (scene) => scene.introducedVersion === 9,
  );

  it("adds Background-only and Foreground-only Scenes", () => {
    expect(sparseBatch.length).toBeGreaterThanOrEqual(5);

    const only = (groupName) =>
      sparseBatch.filter(
        (scene) =>
          scene.groups.length === 1 && scene.groups[0].name === groupName,
      );

    expect(only("Background").length).toBeGreaterThanOrEqual(2);
    expect(only("Foreground").length).toBeGreaterThanOrEqual(1);
  });

  it("keeps Bands in a Background group because it is a 2D shape", () => {
    const softBands = DEFAULT_SCENES.find(
      (scene) => scene.name === "Soft Bands",
    );

    expect(softBands.groups.map((group) => group.name)).toEqual(["Background"]);
    expect(getModulesInGroup(softBands, "Background")).toEqual(["Bands"]);

    const quietBloom = DEFAULT_SCENES.find(
      (scene) => scene.name === "Quiet Bloom",
    );

    expect(quietBloom.groups.map((group) => group.name)).toEqual([
      "Background",
      "Effects",
    ]);
    expect(getModulesInGroup(quietBloom, "Background")).toEqual(["Bands"]);
    expect(getModulesInGroup(quietBloom, "Effects")).toEqual(["Bloom"]);
  });

  it("chains an effect inside a Foreground group", () => {
    const chained = sparseBatch.filter((scene) => {
      const foreground = scene.groups.find(
        (group) => group.name === "Foreground",
      );
      const names = (foreground?.modules || []).map((module) => module.name);
      const hasGenerator = names.some((name) => !EFFECT_MODULES.includes(name));

      return (
        hasGenerator && names.some((name) => EFFECT_MODULES.includes(name))
      );
    });

    expect(chained.length).toBeGreaterThanOrEqual(1);
    expect(
      chained.some((scene) =>
        scene.groups
          .find((group) => group.name === "Foreground")
          .modules.some((module) => module.name === "Chromatic Abberation"),
      ),
    ).toBe(true);
  });

  it("seeds the version 9 batch for installs on version 8", () => {
    const selected = getScenesToSeed({
      defaultsVersion: 8,
      seededNames: getDefaultSceneNamesUpTo(8),
      existingNames: [],
    });
    const newerBatch = DEFAULT_SCENES.filter(
      (scene) => scene.introducedVersion > 8,
    );

    expect(selected.map((scene) => scene.name)).toEqual(
      newerBatch.map((scene) => scene.name),
    );
  });
});

describe("version 10 default scene refresh", () => {
  const seededScene = (name) => ({ id: `${name}-id`, name, modules: {} });

  it("rebuilds the trimmed seeded defaults for installs on version 10", () => {
    const scenes = [
      seededScene("Circuit Ritual"),
      seededScene("Burning Mycelium Circle"),
      seededScene("Soft Bands"),
      seededScene("Quiet Bloom"),
    ];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 10,
      scenes,
      removedModuleNames: [],
      seededNames: scenes.map((scene) => scene.name),
    });
    const names = result.replacements.map((replacement) => replacement.name);

    expect(names).toEqual(
      expect.arrayContaining([
        "Circuit Ritual",
        "Burning Mycelium Circle",
        "Soft Bands",
        "Quiet Bloom",
      ]),
    );

    const circuit = result.replacements.find(
      (replacement) => replacement.name === "Circuit Ritual",
    );

    expect(circuit.id).toBe("Circuit Ritual-id");
    expect(
      circuit.definition.groups.flatMap((group) =>
        group.modules.map((module) => module.name),
      ),
    ).not.toContain("RGB Halftone");

    const burning = result.replacements.find(
      (replacement) => replacement.name === "Burning Mycelium Circle",
    );

    expect(burning.definition.groups.map((group) => group.name)).toEqual([
      "Background",
      "Effects",
    ]);

    const softBands = result.replacements.find(
      (replacement) => replacement.name === "Soft Bands",
    );

    expect(softBands.id).toBe("Soft Bands-id");
    expect(softBands.definition.groups.map((group) => group.name)).toEqual([
      "Background",
    ]);

    const quietBloom = result.replacements.find(
      (replacement) => replacement.name === "Quiet Bloom",
    );

    expect(quietBloom.id).toBe("Quiet Bloom-id");
    expect(quietBloom.definition.groups.map((group) => group.name)).toEqual([
      "Background",
      "Effects",
    ]);
    expect(
      quietBloom.definition.groups
        .find((group) => group.name === "Effects")
        .modules.map((module) => module.name),
    ).toEqual(["Bloom"]);
  });

  it("lists the Scenes whose definitions changed", () => {
    expect(REFRESHED_DEFAULT_SCENES).toContain("Circuit Ritual");
    expect(REFRESHED_DEFAULT_SCENES).toContain("Strobe Riot");
    expect(REFRESHED_DEFAULT_SCENES).toContain("Soft Bands");
    expect(REFRESHED_DEFAULT_SCENES).toContain("Quiet Bloom");
    expect(REFRESHED_DEFAULT_SCENES).toContain("It Twirls At Night");
    expect(REFRESHED_DEFAULT_SCENES).not.toContain("It Comes At Night");
  });

  it("does not overwrite user Scenes that were not seeded", () => {
    const scenes = [seededScene("Circuit Ritual")];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 10,
      scenes,
      removedModuleNames: [],
      seededNames: [],
    });

    expect(result.replacements).toEqual([]);
  });

  it("does not refresh installs already on the latest version", () => {
    const scenes = [seededScene("Circuit Ritual")];
    const result = getDefaultScenesToReplace({
      defaultsVersion: DEFAULTS_VERSION,
      scenes,
      removedModuleNames: [],
      seededNames: ["Circuit Ritual"],
    });

    expect(result.replacements).toEqual([]);
  });
});

describe("default scene removal migration", () => {
  const seededScene = (name, id = name) => ({ id, name, modules: {} });

  it("removes seeded retired scenes for installs upgrading to version 8", () => {
    const names = [
      "Ferrofluid Pulse",
      "Kaleidoscope Cathedral",
      "Slit Scan Horizon",
      "Spiral Particles",
      "Spectral Bloom",
      "Fractured Kaleidoscope",
      "Broken Mirror Ball",
    ];
    const scenes = names.map((name, index) => seededScene(name, `id-${index}`));
    const result = getDefaultScenesToRemove({
      defaultsVersion: 7,
      scenes,
      seededNames: scenes.map((scene) => scene.name),
    });

    expect(result.map((removal) => removal.name)).toEqual(names);
    expect(result.map((removal) => removal.id)).toEqual(
      scenes.map((scene) => scene.id),
    );
  });

  it("removes Roto Zoom Riot from installs upgrading to version 11", () => {
    const scenes = [seededScene("Roto Zoom Riot", "roto-id")];
    const result = getDefaultScenesToRemove({
      defaultsVersion: 10,
      scenes,
      seededNames: ["Roto Zoom Riot"],
    });

    expect(result).toEqual([{ id: "roto-id", name: "Roto Zoom Riot" }]);
  });

  it("keeps user-created scenes that share a retired name but were not seeded", () => {
    const scenes = [seededScene("Ferrofluid Pulse", "user-id")];
    const result = getDefaultScenesToRemove({
      defaultsVersion: 7,
      scenes,
      seededNames: ["Something Else"],
    });

    expect(result).toEqual([]);
  });

  it("does not remove anything for installs already on the new defaults version", () => {
    const scenes = [seededScene("Ferrofluid Pulse", "user-id")];
    const result = getDefaultScenesToRemove({
      defaultsVersion: DEFAULTS_VERSION,
      scenes,
      seededNames: ["Ferrofluid Pulse"],
    });

    expect(result).toEqual([]);
  });
});

describe("default scene rename migration", () => {
  it("renames updated defaults that were not removed for a module", () => {
    const scenes = [
      { id: "vhs-id", name: "VHS Afterglow", modules: {} },
      { id: "night-id", name: "It Comes At Night", modules: {} },
      { id: "grid-id", name: "Neon Grid", modules: {} },
      { id: "sphere-id", name: "Chromatic Sphere", modules: {} },
      { id: "twirl-id", name: "Twirl Tunnel", modules: {} },
    ];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 7,
      scenes,
      removedModuleNames: [REMOVED_CUBE_FLOOR_MODULE],
      seededNames: scenes.map((scene) => scene.name),
    });

    expect(result.removals.map((removal) => removal.id)).toEqual([
      "vhs-id",
      "night-id",
      "grid-id",
      "sphere-id",
      "twirl-id",
    ]);
    expect(result.additions.map((definition) => definition.name)).toEqual([
      "Digital Black Hole",
      "It Twirls At Night",
      "Smoke Tunnel",
      "Chromatic Waveform",
      "Golden Spiral Tunnel",
    ]);
  });

  it("keeps same-named scenes that were not seeded from the defaults", () => {
    const scenes = [{ id: "mine", name: "VHS Afterglow", modules: {} }];
    const result = getDefaultScenesToReplace({
      defaultsVersion: 7,
      scenes,
      removedModuleNames: [],
      seededNames: [],
    });

    expect(result.removals).toEqual([]);
    expect(result.additions).toEqual([]);
  });
});
