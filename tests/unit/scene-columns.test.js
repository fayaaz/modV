import { describe, expect, it } from "vitest";
import {
  createEmptyScene,
  moveSceneToIndex,
  nextSceneName,
} from "../../src/renderer/src/application/worker/scene-columns.js";
import { isSceneDocument } from "../../src/renderer/src/application/worker/preset-serializer.js";

describe("nextSceneName", () => {
  it("starts at Scene 1 when no names are taken", () => {
    expect(nextSceneName([])).toBe("Scene 1");
  });

  it("skips names already in use", () => {
    expect(nextSceneName(["Scene 1", "Scene 2"])).toBe("Scene 3");
    expect(nextSceneName(["Scene 2"])).toBe("Scene 1");
  });

  it("matches names case-insensitively and ignoring whitespace", () => {
    expect(nextSceneName(["scene 1", "  SCENE 2  "])).toBe("Scene 3");
  });

  it("ignores blank and non-string names", () => {
    expect(nextSceneName([null, undefined, "", "   ", 42])).toBe("Scene 1");
  });
});

describe("createEmptyScene", () => {
  const timing = { minSeconds: 4, maxSeconds: 9 };

  it("creates an empty scene document with a generated unique name", () => {
    const scene = createEmptyScene({
      id: "scene-id",
      existingNames: ["Scene 1"],
      timing,
    });

    expect(scene).toMatchObject({
      id: "scene-id",
      name: "Scene 2",
      enabled: true,
      minSeconds: 4,
      maxSeconds: 9,
      timingOverride: false,
      groups: [],
      modules: {},
    });
    expect(isSceneDocument(scene)).toBe(true);
  });

  it("uses an explicitly supplied name", () => {
    const scene = createEmptyScene({
      id: "scene-id",
      name: "  New Column  ",
      existingNames: ["Scene 1"],
      timing,
    });

    expect(scene.name).toBe("New Column");
  });

  it("contains no Groups or Modules to copy", () => {
    const scene = createEmptyScene({ id: "scene-id", timing });

    expect(scene.groups).toEqual([]);
    expect(scene.modules).toEqual({});
    expect(scene.inputs).toEqual({
      focusedInput: { id: null, title: null },
      inputs: {},
      inputLinks: {},
    });
    expect(scene.expressions).toEqual({ assignments: {} });
    expect(scene.tweens).toEqual([]);
    expect(scene.midi).toEqual({});
  });
});

describe("moveSceneToIndex", () => {
  const scenes = [{ id: "a" }, { id: "b" }, { id: "c" }];

  it("moves a Scene later in the list", () => {
    expect(moveSceneToIndex(scenes, "a", 2).map((scene) => scene.id)).toEqual([
      "b",
      "c",
      "a",
    ]);
  });

  it("moves a Scene earlier in the list", () => {
    expect(moveSceneToIndex(scenes, "c", 0).map((scene) => scene.id)).toEqual([
      "c",
      "a",
      "b",
    ]);
  });

  it("does not mutate the input list", () => {
    const original = scenes.slice();

    moveSceneToIndex(scenes, "a", 2);

    expect(scenes).toEqual(original);
  });

  it("returns the same list for a no-op move", () => {
    expect(moveSceneToIndex(scenes, "b", 1)).toBe(scenes);
  });

  it("ignores unknown Scenes and invalid target indexes", () => {
    expect(moveSceneToIndex(scenes, "missing", 1)).toBe(scenes);
    expect(moveSceneToIndex(scenes, "a", -1)).toBe(scenes);
    expect(moveSceneToIndex(scenes, "a", 3)).toBe(scenes);
    expect(moveSceneToIndex(scenes, "a", 1.5)).toBe(scenes);
    expect(moveSceneToIndex(null, "a", 0)).toBe(null);
  });
});
