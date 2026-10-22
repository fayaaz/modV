import { describe, expect, test, vi } from "vitest";
import {
  createPresetData,
  isSceneDocument,
  loadPresetData,
} from "../../src/renderer/src/application/worker/preset-serializer.js";

function createFakeStore() {
  const cleanup = vi.fn();
  const presetModules = {
    groups: { groups: [{ id: "group-1" }] },
    inputs: { inputs: {}, inputLinks: {}, focusedInput: {} },
    modules: { "module-1": { $id: "module-1" } },
    expressions: { assignments: {} },
    tweens: [],
    midi: { devices: {}, learning: false },
  };

  const store = {
    state: {
      ...presetModules,
      beats: { bpm: 120 },
      scenes: { scenes: [] },
    },
    dispatch: vi.fn(async (type) => {
      const [moduleKey, action] = type.split("/");

      if (action === "createPresetData") {
        if (!(moduleKey in presetModules)) {
          throw new Error(`No createPresetData for ${moduleKey}`);
        }

        return presetModules[moduleKey];
      }

      if (action === "loadPresetData") {
        if (!(moduleKey in presetModules)) {
          throw new Error(`No loadPresetData for ${moduleKey}`);
        }

        return moduleKey === "groups" ? cleanup : undefined;
      }

      return undefined;
    }),
    commit: vi.fn(),
  };

  return { store, cleanup, presetModules };
}

describe("preset serializer", () => {
  test("serialises every store module with preset data, and nothing else", async () => {
    const { store } = createFakeStore();

    const preset = await createPresetData(store);

    expect(Object.keys(preset).sort()).toEqual([
      "expressions",
      "groups",
      "inputs",
      "midi",
      "modules",
      "tweens",
    ]);
    expect(preset.groups).toEqual(store.state.groups);
  });

  test("returns an independent snapshot", async () => {
    const { store } = createFakeStore();

    const preset = await createPresetData(store);
    preset.groups.groups[0].id = "changed";

    expect(store.state.groups.groups[0].id).toBe("group-1");
  });

  test("loads preset modules and swaps the live buffers", async () => {
    const { store, cleanup } = createFakeStore();
    const preset = {
      groups: [],
      inputs: {},
      modules: {},
      expressions: {},
      tweens: [],
      midi: {},
    };

    await loadPresetData(store, preset);

    expect(store.dispatch).toHaveBeenCalledWith("groups/loadPresetData", []);
    expect(store.dispatch).toHaveBeenCalledWith("modules/loadPresetData", {});
    expect(store.commit).toHaveBeenCalledWith("groups/SWAP");
    expect(store.commit).toHaveBeenCalledWith("modules/SWAP");
    expect(store.commit).toHaveBeenCalledWith("inputs/SWAP");
    expect(store.commit).toHaveBeenCalledWith("expressions/SWAP");
    expect(cleanup).toHaveBeenCalled();
  });

  test("ignores Scene metadata when loaded as a preset", async () => {
    const { store } = createFakeStore();
    const scene = {
      id: "scene-1",
      name: "A Scene",
      enabled: true,
      minSeconds: 8,
      maxSeconds: 24,
      groups: [],
      inputs: {},
      modules: {},
      expressions: {},
      tweens: [],
      midi: {},
    };

    await loadPresetData(store, scene);

    expect(store.dispatch).not.toHaveBeenCalledWith(
      "id/loadPresetData",
      expect.anything(),
    );
    expect(store.dispatch).not.toHaveBeenCalledWith(
      "name/loadPresetData",
      expect.anything(),
    );
    expect(store.dispatch).toHaveBeenCalledWith("groups/loadPresetData", []);
  });
});

describe("isSceneDocument", () => {
  test("detects a scene document", () => {
    expect(
      isSceneDocument({
        id: "scene-1",
        name: "A Scene",
        groups: [],
        modules: {},
      }),
    ).toBe(true);
  });

  test("rejects a preset document", () => {
    expect(
      isSceneDocument({
        groups: [],
        modules: {},
        inputs: {},
        expressions: {},
        tweens: [],
        midi: {},
      }),
    ).toBe(false);
  });

  test("rejects empty or legacy shape-less values", () => {
    expect(isSceneDocument(null)).toBe(false);
    expect(isSceneDocument({})).toBe(false);
    expect(isSceneDocument({ id: "scene-1", name: "A Scene" })).toBe(false);
  });
});
