import { describe, expect, it } from "vitest";
import {
  clearSceneCellModules,
  mergeSceneCell,
} from "../../src/renderer/src/application/worker/scene-cell.js";

function group(id, name, modules = []) {
  return { id, name, modules };
}

function scene(extra = {}) {
  return {
    id: "scene-1",
    name: "Scene 1",
    groups: [
      group("g0", "Background", ["m0"]),
      group("g1", "Foreground", ["m1", "m2"]),
    ],
    modules: {
      m0: { id: "m0" },
      m1: { id: "m1" },
      m2: { id: "m2" },
    },
    inputs: {
      focusedInput: { id: null, title: null },
      inputs: { i0: { id: "i0" } },
      inputLinks: { i0: { source: "bass" } },
    },
    expressions: { assignments: { a0: { inputId: "i0" } } },
    tweens: [{ id: "t0" }],
    ...extra,
  };
}

describe("clearSceneCellModules", () => {
  it("removes the cell's modules and their snapshots", () => {
    const result = clearSceneCellModules(scene(), 1, []);

    expect(result.moduleIds).toEqual(["m1", "m2"]);
    expect(result.groups[1].modules).toEqual([]);
    expect(result.groups[0].modules).toEqual(["m0"]);
    expect(Object.keys(result.modules)).toEqual(["m0"]);
  });

  it("returns null for an empty cell", () => {
    expect(clearSceneCellModules(scene(), 5, [])).toBeNull();
    expect(
      clearSceneCellModules(
        scene({ groups: [null, group("g1", "F", [])] }),
        0,
        [],
      ),
    ).toBeNull();
  });

  it("uses named rows to find the cell's group", () => {
    const value = scene({
      groups: [
        group("ga", "Effects", ["m0"]),
        group("gb", "Background", ["m1"]),
      ],
      modules: { m0: { id: "m0" }, m1: { id: "m1" } },
    });

    const result = clearSceneCellModules(value, 2, ["", "", "Background"]);

    expect(result.groups[1].modules).toEqual([]);
    expect(result.groups[0].modules).toEqual(["m0"]);
    expect(result.modules).toEqual({ m0: { id: "m0" } });
  });

  it("does not mutate the scene", () => {
    const value = scene();
    const result = clearSceneCellModules(value, 1, []);

    expect(value.groups[1].modules).toEqual(["m1", "m2"]);
    expect(result.groups).not.toBe(value.groups);
  });
});

describe("mergeSceneCell", () => {
  const cell = {
    group: group("live", "Foreground", ["m3"]),
    modules: { m3: { id: "m3" } },
    inputs: {
      inputs: { i3: { id: "i3" } },
      inputLinks: { i3: { source: "treble" } },
    },
    expressions: { assignments: { a3: { inputId: "i3" } } },
    tweens: [{ id: "t3" }],
  };

  it("replaces the group at the cell's row and keeps other rows", () => {
    const result = mergeSceneCell(scene(), 1, [], cell);

    expect(result.groups[1].id).toBe("live");
    expect(result.groups[0].id).toBe("g0");
    expect(result.groups).toHaveLength(2);
  });

  it("appends a group when the row has no snapshot", () => {
    const result = mergeSceneCell(scene({ groups: [null] }), 1, [], cell);

    expect(result.groups[0]).toBeNull();
    expect(result.groups[1].id).toBe("live");
  });

  it("pads positional rows when the new cell is below the saved groups", () => {
    const result = mergeSceneCell(scene(), 3, [], cell);

    expect(result.groups).toHaveLength(4);
    expect(result.groups[0].id).toBe("g0");
    expect(result.groups[1].id).toBe("g1");
    expect(result.groups[2]).toBeNull();
    expect(result.groups[3].id).toBe("live");
  });

  it("appends a named-row cell without padding", () => {
    const result = mergeSceneCell(scene(), 5, ["", "", "", "", "", "Webcam"], {
      ...cell,
      group: group("live", "Webcam", ["m3"]),
    });

    expect(result.groups[2].id).toBe("live");
    expect(result.groups).toHaveLength(3);
  });

  it("merges module snapshots and prunes unreferenced ones", () => {
    const result = mergeSceneCell(scene(), 1, [], cell);

    expect(result.modules).toHaveProperty("m0");
    expect(result.modules).toHaveProperty("m3");
    expect(result.modules).not.toHaveProperty("m1");
    expect(result.modules).not.toHaveProperty("m2");
  });

  it("merges inputs, links, expressions and tweens", () => {
    const result = mergeSceneCell(scene(), 1, [], cell);

    expect(result.inputs.inputs).toHaveProperty("i0");
    expect(result.inputs.inputs).toHaveProperty("i3");
    expect(result.inputs.inputLinks).toHaveProperty("i3");
    expect(result.expressions.assignments).toHaveProperty("a0");
    expect(result.expressions.assignments).toHaveProperty("a3");
    expect(result.tweens).toEqual([{ id: "t0" }, { id: "t3" }]);
  });

  it("uses named rows to replace the right group", () => {
    const value = scene({
      groups: [
        group("ga", "Effects", ["m0"]),
        group("gb", "Background", ["m1"]),
      ],
      modules: { m0: { id: "m0" }, m1: { id: "m1" } },
    });

    const result = mergeSceneCell(value, 2, ["", "", "Background"], cell);

    expect(result.groups[1].id).toBe("live");
    expect(result.groups[0].id).toBe("ga");
  });

  it("does not mutate the scene", () => {
    const value = scene();
    const result = mergeSceneCell(value, 1, [], cell);

    expect(value.groups[1].id).toBe("g1");
    expect(result).not.toBe(value);
  });
});
