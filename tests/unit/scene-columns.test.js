import { describe, expect, it } from "vitest";
import {
  createEmptyScene,
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
