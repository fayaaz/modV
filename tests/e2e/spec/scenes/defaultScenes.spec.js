import { expect, test } from "@playwright/test";
import DEFAULT_SCENES, {
  DEFAULTS_VERSION,
  getDefaultSceneNamesUpTo,
  getScenesToSeed,
} from "../../../../src/renderer/src/application/worker/store/default-scenes";
import { modVApp } from "../../pageObjectModel";

const GROUP_NAMES = ["Background", "Foreground", "Effects"];

const REQUIRED_ISF_FILTERS = [
  "Bloom",
  "Dirty Lens",
  "VVMotionBlur 3.0",
  "Vignette",
  "Hue-Saturation",
  "Kaleidoscope",
  "Mirror",
  "Slit Scan",
];

const sceneKey = (name) =>
  String(name || "")
    .trim()
    .toLowerCase();

function getModuleNames(scene) {
  return scene.groups.flatMap((group) =>
    (group.modules || []).map((module) => module.name),
  );
}

function getEffectsModuleNames(scene) {
  const effects = scene.groups.find((group) => group.name === "Effects");

  return (effects?.modules || []).map((module) => module.name);
}

test("ships at least 18 default scenes with unique names", () => {
  expect(DEFAULT_SCENES.length).toBeGreaterThanOrEqual(18);
  expect(DEFAULTS_VERSION).toBeGreaterThanOrEqual(2);

  const keys = DEFAULT_SCENES.map((scene) => sceneKey(scene.name));

  expect(new Set(keys).size).toBe(keys.length);
  expect(keys.every((key) => key.length > 0)).toBe(true);
});

test("every default scene references registered modules and valid groups", async () => {
  const registered = await modVApp.evaluateWorkerState("modules.registered");
  const registeredNames = new Set(Object.keys(registered));

  for (const scene of DEFAULT_SCENES) {
    expect(scene.groups.length).toBeGreaterThanOrEqual(GROUP_NAMES.length);

    const groupNames = scene.groups.map((group) => group.name);
    expect(groupNames).toEqual(expect.arrayContaining(GROUP_NAMES));

    for (const group of scene.groups) {
      expect(Array.isArray(group.modules)).toBe(true);
      expect(group.modules.length).toBeGreaterThan(0);
      expect(group.inherit).toBe(group.name !== "Background");

      for (const module of group.modules) {
        expect(typeof module.name).toBe("string");
        expect(module.name.length).toBeGreaterThan(0);
        expect(registeredNames.has(module.name)).toBe(true);
      }
    }

    expect(Number(scene.minSeconds)).toBeLessThan(Number(scene.maxSeconds));
  }
});

test("uses a spread of registered ISF filters in Effects groups", async () => {
  const registered = await modVApp.evaluateWorkerState("modules.registered");
  const isfNames = new Set(
    Object.values(registered)
      .filter((module) => module.meta.type === "isf")
      .map((module) => module.meta.name),
  );

  for (const filter of REQUIRED_ISF_FILTERS) {
    expect(isfNames.has(filter)).toBe(true);
  }

  const scenesWithFilters = DEFAULT_SCENES.filter((scene) =>
    getEffectsModuleNames(scene).some((name) => isfNames.has(name)),
  );
  const usedFilters = new Set(
    scenesWithFilters.flatMap((scene) =>
      getEffectsModuleNames(scene).filter((name) => isfNames.has(name)),
    ),
  );

  expect(scenesWithFilters.length).toBeGreaterThanOrEqual(12);
  expect(usedFilters.size).toBeGreaterThanOrEqual(REQUIRED_ISF_FILTERS.length);

  for (const filter of REQUIRED_ISF_FILTERS) {
    expect(usedFilters.has(filter)).toBe(true);
  }

  expect(
    DEFAULT_SCENES.some((scene) => getModuleNames(scene).includes("Bands")),
  ).toBe(true);
});

test("seeds every default scene on a fresh install", () => {
  const selected = getScenesToSeed({
    defaultsVersion: 0,
    seededNames: [],
    existingNames: [],
  });

  expect(selected.map((scene) => scene.name)).toEqual(
    DEFAULT_SCENES.map((scene) => scene.name),
  );
});

test("appends only missing new defaults without duplicating old or user scenes", () => {
  const oldNames = getDefaultSceneNamesUpTo(1);
  const newDefaults = DEFAULT_SCENES.filter(
    (scene) => (scene.introducedVersion || 1) > 1,
  );
  const userName = "My User Scene";
  const existingNames = [
    userName,
    ...oldNames.map((name) => name.toUpperCase()),
  ];

  const selected = getScenesToSeed({
    defaultsVersion: 1,
    seededNames: oldNames,
    existingNames,
  });

  expect(selected.map((scene) => scene.name)).toEqual(
    newDefaults.map((scene) => scene.name),
  );
  expect(
    selected.every((scene) => sceneKey(scene.name) !== sceneKey(userName)),
  ).toBe(true);
  expect(selected.every((scene) => !oldNames.includes(scene.name))).toBe(true);
});

test("does not resurrect deleted defaults or duplicate case-insensitive names", () => {
  const oldNames = getDefaultSceneNamesUpTo(1);
  const newDefaults = DEFAULT_SCENES.filter(
    (scene) => (scene.introducedVersion || 1) > 1,
  );
  const deleted = newDefaults[0];
  const alreadyPresent = newDefaults[1];
  const seededNames = [...oldNames, deleted.name];

  const selected = getScenesToSeed({
    defaultsVersion: 1,
    seededNames,
    existingNames: ["E2E User Scene", alreadyPresent.name.toUpperCase()],
  });

  const selectedKeys = selected.map((scene) => sceneKey(scene.name));

  expect(selectedKeys).not.toContain(sceneKey(deleted.name));
  expect(selectedKeys).not.toContain(sceneKey(alreadyPresent.name));
  expect(new Set(selectedKeys).size).toBe(selectedKeys.length);
});

test("hydrates legacy files that only have defaultsSeeded", async () => {
  const expectedLegacyNames = getDefaultSceneNamesUpTo(1);

  const result = await modVApp.evaluateWorker(async () => {
    const fs = require("fs");
    const path = require("path");
    const filePath = path.join(self.__userData, "scenes.json");
    let previous = null;

    try {
      previous = fs.readFileSync(filePath, "utf8");
    } catch (e) {
      if (e.code !== "ENOENT") {
        throw e;
      }
    }

    const before = {
      defaultsSeeded: self.store.state.scenes.defaultsSeeded,
      defaultsVersion: self.store.state.scenes.defaultsVersion,
      seededDefaultScenes: [...self.store.state.scenes.seededDefaultScenes],
    };

    try {
      fs.mkdirSync(path.dirname(filePath), { recursive: true });
      fs.writeFileSync(
        filePath,
        JSON.stringify({
          version: 1,
          defaultsSeeded: true,
          scenes: [],
        }),
      );

      await self.store.dispatch("scenes/hydrateScenes");

      return {
        defaultsSeeded: self.store.state.scenes.defaultsSeeded,
        defaultsVersion: self.store.state.scenes.defaultsVersion,
        seededDefaultScenes: [...self.store.state.scenes.seededDefaultScenes],
      };
    } finally {
      self.store.commit("scenes/SET_DEFAULTS_SEEDED", before.defaultsSeeded);
      self.store.commit("scenes/SET_DEFAULTS_VERSION", before.defaultsVersion);
      self.store.commit(
        "scenes/SET_SEEDED_DEFAULT_SCENES",
        before.seededDefaultScenes,
      );

      if (previous === null) {
        fs.rmSync(filePath, { force: true });
      } else {
        fs.writeFileSync(filePath, previous);
      }
    }
  });

  expect(result.defaultsSeeded).toBe(true);
  expect(result.defaultsVersion).toBe(1);
  expect(result.seededDefaultScenes.length).toBe(expectedLegacyNames.length);
  expect(result.seededDefaultScenes).toContain("Mycelium Bloom");
  expect(result.seededDefaultScenes).not.toContain("Burning Mycelium Circle");
});

test("ensureDefaultScenes appends a missing default once and keeps user scenes", async () => {
  const target = DEFAULT_SCENES[DEFAULT_SCENES.length - 1];
  const targetName = target.name;
  const seededNames = DEFAULT_SCENES.map((scene) => scene.name).filter(
    (name) => sceneKey(name) !== sceneKey(targetName),
  );
  const userSceneName = `E2E Upgrade User ${Date.now()}`;

  const result = await modVApp.evaluateWorker(
    async ({ targetName, seededNames, userSceneName }) => {
      const fs = require("fs");
      const path = require("path");
      const filePath = path.join(self.__userData, "scenes.json");
      let previous = null;

      try {
        previous = fs.readFileSync(filePath, "utf8");
      } catch (e) {
        if (e.code !== "ENOENT") {
          throw e;
        }
      }

      const before = {
        defaultsSeeded: self.store.state.scenes.defaultsSeeded,
        defaultsVersion: self.store.state.scenes.defaultsVersion,
        seededDefaultScenes: [...self.store.state.scenes.seededDefaultScenes],
      };
      const targetKey = targetName.trim().toLowerCase();
      const userKey = userSceneName.trim().toLowerCase();

      try {
        self.store.commit("scenes/ADD_SCENE", {
          id: `e2e-user-${Date.now()}`,
          name: userSceneName,
          enabled: true,
          groups: [],
          modules: {},
        });
        self.store.commit("scenes/SET_DEFAULTS_SEEDED", true);
        self.store.commit("scenes/SET_DEFAULTS_VERSION", 1);
        self.store.commit("scenes/SET_SEEDED_DEFAULT_SCENES", seededNames);

        await self.store.dispatch("scenes/ensureDefaultScenes");

        const afterFirst = self.store.state.scenes.scenes.map((scene) => ({
          id: scene.id,
          name: scene.name,
        }));
        const targets = afterFirst.filter(
          (scene) => scene.name.trim().toLowerCase() === targetKey,
        );
        const userKept = afterFirst.some(
          (scene) => scene.name.trim().toLowerCase() === userKey,
        );

        if (targets.length) {
          await self.store.dispatch("scenes/removeScene", {
            sceneId: targets[0].id,
          });
        }

        await self.store.dispatch("scenes/ensureDefaultScenes");

        const afterDelete = self.store.state.scenes.scenes.map((scene) =>
          scene.name.trim().toLowerCase(),
        );

        return {
          targetAdded: targets.length,
          userKept,
          resurrected: afterDelete.filter((name) => name === targetKey).length,
          seededIncludesTarget:
            self.store.state.scenes.seededDefaultScenes.some(
              (name) => name.trim().toLowerCase() === targetKey,
            ),
        };
      } finally {
        const leftoverKeys = new Set([targetKey, userKey]);
        const leftovers = self.store.state.scenes.scenes.filter((scene) =>
          leftoverKeys.has(scene.name.trim().toLowerCase()),
        );

        for (const scene of leftovers) {
          await self.store.dispatch("scenes/removeScene", {
            sceneId: scene.id,
          });
        }

        self.store.commit("scenes/SET_DEFAULTS_SEEDED", before.defaultsSeeded);
        self.store.commit(
          "scenes/SET_DEFAULTS_VERSION",
          before.defaultsVersion,
        );
        self.store.commit(
          "scenes/SET_SEEDED_DEFAULT_SCENES",
          before.seededDefaultScenes,
        );

        if (previous === null) {
          fs.rmSync(filePath, { force: true });
        } else {
          fs.writeFileSync(filePath, previous);
        }
      }
    },
    { targetName, seededNames, userSceneName },
  );

  expect(result.targetAdded).toBe(1);
  expect(result.userKept).toBe(true);
  expect(result.seededIncludesTarget).toBe(true);
  expect(result.resurrected).toBe(0);
});
