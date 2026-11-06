import { describe, expect, it, vi } from "vitest";
import constants from "../../src/renderer/src/application/constants.js";
import {
  assignSceneRowIndexes,
  getRowCount,
  getRowGroup,
  getRowLabel,
  getSceneGroups,
  isRowCellEmpty,
  isSceneEnabled,
  normalizeRowNames,
  pickNextRowScene,
  shouldAdvanceRow,
} from "../../src/renderer/src/application/worker/scene-rows.js";

const GALLERY = constants.GALLERY_GROUP_NAME;

function group(name, modules = [], extra = {}) {
  return { name, modules, ...extra };
}

function scene(id, groups, extra = {}) {
  return { id, name: id, groups, ...extra };
}

const a = (name = "A") => group(name, ["module-a"]);
const b = (name = "B") => group(name, ["module-b"]);
const c = (name = "C") => group(name, ["module-c"]);
const empty = (name = "Empty") => group(name, []);

describe("getSceneGroups", () => {
  it("returns the scene groups", () => {
    const groups = [a(), b()];
    expect(getSceneGroups({ groups })).toBe(groups);
  });

  it("returns an empty array when the scene has no groups", () => {
    expect(getSceneGroups({})).toEqual([]);
    expect(getSceneGroups(null)).toEqual([]);
    expect(getSceneGroups({ groups: "nope" })).toEqual([]);
  });
});

describe("isSceneEnabled", () => {
  it("treats only an explicit false as disabled", () => {
    expect(isSceneEnabled({ enabled: false })).toBe(false);
    expect(isSceneEnabled({ enabled: true })).toBe(true);
    expect(isSceneEnabled({})).toBe(true);
    expect(isSceneEnabled({ enabled: 0 })).toBe(true);
  });
});

describe("isRowCellEmpty", () => {
  it("is empty when the group is missing", () => {
    expect(isRowCellEmpty(scene("s", [a()]), 1)).toBe(true);
    expect(isRowCellEmpty({}, 0)).toBe(true);
  });

  it("is empty when the group has no modules", () => {
    expect(isRowCellEmpty(scene("s", [empty()]), 0)).toBe(true);
    expect(isRowCellEmpty(scene("s", [{ name: "No modules" }]), 0)).toBe(true);
  });

  it("is not empty when the group has modules", () => {
    expect(isRowCellEmpty(scene("s", [a()]), 0)).toBe(false);
  });

  it("supports object-shaped module maps", () => {
    expect(
      isRowCellEmpty(scene("s", [{ name: "Object", modules: {} }]), 0),
    ).toBe(true);
    expect(
      isRowCellEmpty(scene("s", [{ name: "Object", modules: { m: {} } }]), 0),
    ).toBe(false);
  });
});

describe("getRowGroup", () => {
  it("returns the group at the given index", () => {
    const first = a();
    const second = b();
    expect(getRowGroup(scene("s", [first, second]), 1)).toBe(second);
  });

  it("returns null when the group is missing", () => {
    expect(getRowGroup(scene("s", [a()]), 3)).toBeNull();
    expect(getRowGroup({}, 0)).toBeNull();
    expect(getRowGroup(null, 0)).toBeNull();
  });

  it("matches a named row case-insensitively by group name", () => {
    const webcam = group("WEBCAM", ["cam"]);
    expect(getRowGroup(scene("s", [webcam, b()]), 0, ["webcam"])).toBe(webcam);
    expect(getRowGroup(scene("s", [a(), webcam, b()]), 0, ["Webcam"])).toBe(
      webcam,
    );
  });

  it("matches word-prefixed group names but not unrelated names", () => {
    const cam2 = group("Webcam 2", ["cam-2"]);
    expect(getRowGroup(scene("s", [cam2]), 0, ["webcam"])).toBe(cam2);
    expect(getRowGroup(scene("s", [cam2]), 0, ["webcam 2"])).toBe(cam2);
    expect(getRowGroup(scene("s", [cam2]), 0, ["cam"])).toBeNull();
    expect(getRowGroup(scene("s", [cam2]), 1, ["cam", "webcam 2"])).toBe(cam2);
  });

  it("matches digit-suffixed group names with or without a separator", () => {
    const fx = group("Effects", ["fx"]);
    const fx2 = group("Effects 2", ["fx-2"]);
    const fxTight = group("Effects2", ["fx-tight"]);

    // "effects2" typed without a space still finds the "Effects" group
    expect(getRowGroup(scene("s", [fx]), 0, ["effects2"])).toBe(fx);
    // ...and a named row finds a digit-suffixed group without a space
    expect(getRowGroup(scene("s", [fxTight]), 0, ["effects"])).toBe(fxTight);
    expect(getRowGroup(scene("s", [fx2]), 0, ["effects"])).toBe(fx2);
    // larger suffixes match the same base name
    expect(getRowGroup(scene("s", [fx]), 0, ["Effects 10"])).toBe(fx);
  });

  it("solo digit-suffixed rows match the base group, but not when another row owns the base name", () => {
    const fx = group("Effects", ["fx"]);
    const get = (rowNames, rowIndex) =>
      getRowGroup(scene("s", [fx, b()]), rowIndex, rowNames);

    // A solo "effects2" row finds the "Effects" group...
    expect(get(["effects2"], 0)).toBe(fx);
    // ...but when a sibling row is already the "Effects" family, the
    // suffixed row stays empty instead of claiming the same group twice.
    expect(get(["Effects", "effects2"], 1)).toBeNull();
    expect(get(["Effects", "effects2"], 0)).toBe(fx);
    // Three-digit family members get their own rows when groups exist.
    const fx2 = group("Effects 2", ["fx-2"]);
    expect(
      getRowGroup(scene("s", [fx, fx2]), 1, ["Effects", "Effects 2"]),
    ).toBe(fx2);
  });

  it("does not match digit-suffixed names across different base names", () => {
    expect(
      getRowGroup(scene("s", [group("Webcam 2", ["cam-2"])]), 0, ["cam"]),
    ).toBeNull();
    // but "cam2" belongs to the same family as a "cam" row
    expect(
      getRowGroup(scene("s", [group("CAM2", ["cam-2"])]), 0, ["cam"]),
    ).not.toBeNull();
  });

  it("returns empty for a named row with no matching group even at other indexes", () => {
    expect(getRowGroup(scene("s", [a(), b()]), 0, ["webcam"])).toBeNull();
  });

  it("returns only the first matching group for a named row", () => {
    const first = group("Webcam", ["cam-1"]);
    const second = group("webcam", ["cam-2"]);
    expect(getRowGroup(scene("s", [first, second]), 0, ["webcam"])).toBe(first);
  });

  it("falls back to positional lookup without row names", () => {
    const list = [a(), b()];
    expect(getRowGroup(scene("s", list), 1, [])).toBe(list[1]);
    expect(getRowGroup(scene("s", list), 1, [null])).toBe(list[1]);
    expect(getRowGroup(scene("s", list), 1, undefined)).toBe(list[1]);
  });

  it("treats a blank name in rowNames as positional", () => {
    const list = [a(), b()];
    expect(getRowGroup(scene("s", list), 1, ["", "  "])).toBe(list[1]);
  });
});

describe("assignSceneRowIndexes", () => {
  it("keeps positional indexes without row names", () => {
    expect(assignSceneRowIndexes([a(), b(), c()], [])).toEqual([0, 1, 2]);
    expect(assignSceneRowIndexes([a(), b()], undefined)).toEqual([0, 1]);
    expect(assignSceneRowIndexes([null, b()], ["", "  "])).toEqual([0, 1]);
  });

  it("pins named groups to their rows regardless of group order", () => {
    const cam = group("WEBCAM");
    const fg = group("Foreground");
    const fx = group("Effects");
    const rowNames = [
      "webcam",
      "Background",
      "Foreground",
      "Effects",
      "effects2",
    ];

    expect(assignSceneRowIndexes([cam, fg, fx], rowNames)).toEqual([0, 2, 3]);
    expect(assignSceneRowIndexes([fx, fg, cam], rowNames)).toEqual([3, 2, 0]);
  });

  it("slots groups without a matching named row below the named rows", () => {
    const cam = group("WEBCAM");
    const bg = group("Background");
    const fx = group("Effects");

    expect(assignSceneRowIndexes([cam, bg, fx], ["webcam", "Effects"])).toEqual(
      [0, 2, 1],
    );
  });

  it("matches case-insensitively and by word prefix", () => {
    const cam2 = group("Webcam 2");
    expect(assignSceneRowIndexes([cam2], ["webcam"])).toEqual([0]);
    expect(assignSceneRowIndexes([cam2], ["Webcam 2"])).toEqual([0]);
    expect(assignSceneRowIndexes([cam2], ["cam"])).toEqual([1]);
  });

  it("does not let a suffixed row claim a group owned by its base row", () => {
    const fx = group("Effects");

    expect(assignSceneRowIndexes([fx], ["Effects", "effects2"])).toEqual([0]);
    expect(assignSceneRowIndexes([fx], ["effects2"])).toEqual([0]);
  });

  it("leaves holes null when named rows are in use", () => {
    expect(assignSceneRowIndexes([null, group("Effects")], [])).toEqual([0, 1]);
    expect(
      assignSceneRowIndexes([null, group("Effects")], ["Effects"]),
    ).toEqual([null, 0]);
  });
});

describe("getRowCount", () => {
  it("returns zero for no scenes", () => {
    expect(getRowCount([])).toBe(0);
    expect(getRowCount(undefined)).toBe(0);
  });

  it("uses the scene with the most groups without padding others", () => {
    const scenes = [
      scene("one", [a(), b()]),
      scene("two", [a(), b(), c()]),
      scene("three", [a()]),
    ];
    expect(getRowCount(scenes)).toBe(3);
  });

  it("ignores the gallery group in scenes", () => {
    const scenes = [
      scene("one", [group(GALLERY, ["gallery-module"]), a(), b()]),
      scene("two", [a(), group(GALLERY), b(), c()]),
    ];
    expect(getRowCount(scenes)).toBe(3);
  });

  it("includes live groups by sceneRow", () => {
    const liveGroups = [
      group("A", ["a"], { sceneRow: 0 }),
      group("D", ["d"], { sceneRow: 3 }),
    ];
    expect(getRowCount([], liveGroups)).toBe(4);
  });

  it("ignores live gallery groups and non-gallery live groups still count", () => {
    const liveGroups = [
      group("A", ["a"], { sceneRow: 0 }),
      group(GALLERY, ["gallery"], { sceneRow: 9 }),
      group("B", ["b"], { sceneRow: 1 }),
    ];
    expect(getRowCount([], liveGroups)).toBe(2);
  });

  it("ignores outgoing ghost groups while a row fades", () => {
    const liveGroups = [
      group("A", ["a"], { sceneRow: 0 }),
      group("A ghost", ["old"], { sceneRow: 0, transitionGhost: true }),
      group("B", ["b"], { sceneRow: 1 }),
    ];
    expect(getRowCount([], liveGroups)).toBe(2);
  });

  it("skips holes in a partial live Group list", () => {
    const liveGroups = [
      group("A", ["a"], { sceneRow: 0 }),
      undefined,
      group("B", ["b"], { sceneRow: 2 }),
    ];
    expect(getRowCount([], liveGroups)).toBe(3);
  });

  it("combines scene and live group counts", () => {
    expect(
      getRowCount(
        [scene("one", [a(), b()])],
        [group("A", ["a"], { sceneRow: 4 })],
      ),
    ).toBe(5);
  });

  it("never returns a negative count", () => {
    expect(getRowCount([], [])).toBe(0);
    expect(getRowCount([scene("one", [])])).toBe(0);
  });

  it("grows to accommodate named rows", () => {
    expect(getRowCount([], [], ["webcam", null])).toBe(2);
    expect(getRowCount([scene("one", [a()])], [], ["webcam", "fx", null])).toBe(
      3,
    );
  });

  it("keeps the longer of named rows and positional counts", () => {
    expect(getRowCount([scene("one", [a(), b(), c()])], [], ["x"])).toBe(3);
    expect(getRowCount([scene("one", [a()])], [], ["x", "y", "z"])).toBe(3);
  });

  it("ignores non-array row names", () => {
    expect(getRowCount([scene("one", [a()])], [], "nope")).toBe(1);
  });
});

describe("normalizeRowNames", () => {
  it("trims names and keeps positions", () => {
    expect(normalizeRowNames([" webcam ", "  ", "fx"])).toEqual([
      "webcam",
      "",
      "fx",
    ]);
  });

  it("keeps duplicates so row indexes stay aligned", () => {
    expect(normalizeRowNames(["Webcam", "webcam", "FX"])).toEqual([
      "Webcam",
      "webcam",
      "FX",
    ]);
  });

  it("returns an empty array for non-inputs", () => {
    expect(normalizeRowNames(undefined)).toEqual([]);
    expect(normalizeRowNames("nope")).toEqual([]);
  });
});

describe("getRowLabel", () => {
  it("returns the most common group name at the row", () => {
    const scenes = [
      scene("one", [group("Effects"), group("Blur")]),
      scene("two", [group("Effects"), group("Tunnel")]),
      scene("three", [group("Contrast"), group("Blur")]),
    ];
    expect(getRowLabel(scenes, 0)).toBe("Effects");
    expect(getRowLabel(scenes, 1)).toBe("Blur");
  });

  it("falls back to a numbered row when a name only covers a minority of scenes", () => {
    // Three legacy WEBCAM scenes with a fourth "Effects" group must not make
    // an unnamed positional row look like it is called "Effects".
    const plain = () =>
      scene("main", [group("Background"), group("Foreground")]);
    const legacy = () =>
      scene("legacy", [
        group("WEBCAM"),
        group("Background"),
        group("Foreground"),
        group("Effects"),
      ]);

    const scenes = [
      plain(),
      plain(),
      plain(),
      plain(),
      plain(),
      legacy(),
      legacy(),
      legacy(),
    ];
    // Row 3 (the fourth one): only the three legacy Scenes have a group there.
    expect(getRowLabel(scenes, 3)).toBe("Row 4");
    // ...and the shared Foreground row (index 1) still gets its label.
    expect(getRowLabel(scenes, 1)).toBe("Foreground");
  });

  it("falls back to a numbered row on a tie, first majority-winner otherwise", () => {
    const scenes = [
      scene("one", [group("First")]),
      scene("two", [group("Second")]),
      scene("three", [group("Second")]),
      scene("four", [group("First")]),
    ];
    expect(getRowLabel(scenes, 0)).toBe("Row 1");
  });

  it("lets a strict majority win", () => {
    const scenes = [
      scene("one", [group("First")]),
      scene("two", [group("Second")]),
      scene("three", [group("Second")]),
    ];
    expect(getRowLabel(scenes, 0)).toBe("Second");
  });

  it("ignores empty names", () => {
    const scenes = [
      scene("one", [group("")]),
      scene("two", [group("Named")]),
      scene("three", [group("Named")]),
    ];
    expect(getRowLabel(scenes, 0)).toBe("Named");
  });

  it("falls back to a numbered row", () => {
    expect(getRowLabel([], 0)).toBe("Row 1");
    expect(getRowLabel([scene("one", [])], 2)).toBe("Row 3");
    expect(getRowLabel([scene("one", [group("")]), scene("two", [])], 0)).toBe(
      "Row 1",
    );
  });

  it("uses the explicit row name when given", () => {
    const scenes = [scene("one", [group("Effects"), group("Blur")])];
    expect(getRowLabel(scenes, 0, ["webcam"])).toBe("webcam");
    expect(getRowLabel(scenes, 1, ["", "Fog layer"])).toBe("Fog layer");
  });
});

describe("pickNextRowScene", () => {
  const scenes = [scene("a", [a()]), scene("b", [b()]), scene("c", [c()])];

  it("returns null when there are no candidates", () => {
    expect(
      pickNextRowScene({
        scenes: [scene("a", [empty()]), scene("b", [])],
        rowIndex: 0,
        currentSceneId: "a",
      }),
    ).toBeNull();
  });

  it("returns the only candidate", () => {
    const only = [scene("a", [a()]), scene("b", [])];
    expect(
      pickNextRowScene({
        scenes: only,
        rowIndex: 0,
        currentSceneId: "b",
      }),
    ).toBe(only[0]);
  });

  it("picks the candidate after the current scene in order", () => {
    expect(pickNextRowScene({ scenes, rowIndex: 0, currentSceneId: "a" })).toBe(
      scenes[1],
    );
  });

  it("wraps around at the end", () => {
    expect(pickNextRowScene({ scenes, rowIndex: 0, currentSceneId: "c" })).toBe(
      scenes[0],
    );
  });

  it("picks the first candidate when the current scene is not a candidate", () => {
    const withEmpty = [
      scene("a", [empty()]),
      scene("b", [b()]),
      scene("c", [empty()]),
      scene("d", [group("D")]),
    ];
    expect(
      pickNextRowScene({
        scenes: withEmpty,
        rowIndex: 0,
        currentSceneId: "a",
      }),
    ).toBe(withEmpty[1]);
    expect(
      pickNextRowScene({
        scenes: withEmpty,
        rowIndex: 0,
        currentSceneId: "missing",
      }),
    ).toBe(withEmpty[1]);
  });

  it("excludes disabled scenes", () => {
    const withDisabled = [
      scene("a", [a()]),
      scene("b", [b()], { enabled: false }),
      scene("c", [c()]),
    ];
    expect(
      pickNextRowScene({
        scenes: withDisabled,
        rowIndex: 0,
        currentSceneId: "a",
      }),
    ).toBe(withDisabled[2]);
  });

  it("skips cells that are empty at the row", () => {
    const staggered = [
      scene("a", [a(), empty()]),
      scene("b", [empty(), b()]),
      scene("c", [c(), c()]),
    ];
    expect(
      pickNextRowScene({
        scenes: staggered,
        rowIndex: 1,
        currentSceneId: "a",
      }),
    ).toBe(staggered[1]);
  });

  it("honours named rows when choosing a candidate", () => {
    const namedA = group("A", ["m-a"]);
    const namedB = group("B", ["m-b"]);
    const namedC = group("C", ["m-c"]);
    const named = [
      scene("a", [namedA]),
      scene("b", [namedB]),
      scene("c", [namedC]),
    ];
    // On a "B" named row only scene "b" has a matching, non-empty group.
    expect(
      pickNextRowScene({
        scenes: named,
        rowIndex: 0,
        currentSceneId: "a",
        rowNames: ["B"],
      }),
    ).toBe(named[1]);
  });

  it("picks randomly and excludes the current scene when there is more than one candidate", () => {
    const random = vi.fn(() => 0);
    expect(
      pickNextRowScene({
        scenes,
        rowIndex: 0,
        currentSceneId: "b",
        order: "random",
        random,
      }),
    ).toBe(scenes[0]);
    expect(random).toHaveBeenCalledTimes(1);

    expect(
      pickNextRowScene({
        scenes,
        rowIndex: 0,
        currentSceneId: "b",
        order: "random",
        random: () => 0.99,
      }),
    ).toBe(scenes[2]);
  });

  it("can still pick the current scene randomly when it is the only candidate", () => {
    const only = [scene("a", [a()]), scene("b", [])];
    const random = vi.fn(() => 0);
    expect(
      pickNextRowScene({
        scenes: only,
        rowIndex: 0,
        currentSceneId: "a",
        order: "random",
        random,
      }),
    ).toBe(only[0]);
    expect(random).not.toHaveBeenCalled();
  });

  it("does not exclude the current scene randomly when it is not a candidate", () => {
    const candidates = [scene("a", [a()]), scene("b", [b()])];
    expect(
      pickNextRowScene({
        scenes: candidates,
        rowIndex: 0,
        currentSceneId: "missing",
        order: "random",
        random: () => 0,
      }),
    ).toBe(candidates[0]);
  });
});

describe("shouldAdvanceRow", () => {
  it("forces an advance at the maximum dwell", () => {
    expect(
      shouldAdvanceRow({
        dwell: 10,
        minSeconds: 5,
        maxSeconds: 10,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: true, musical: false, forced: true });
  });

  it("forces an advance past the maximum dwell", () => {
    expect(
      shouldAdvanceRow({
        dwell: 12,
        minSeconds: 5,
        maxSeconds: 10,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: true, musical: false, forced: true });
  });

  it("does not advance while dwelling below the minimum", () => {
    const random = vi.fn(() => 0);
    expect(
      shouldAdvanceRow({
        dwell: 2,
        minSeconds: 5,
        maxSeconds: 10,
        phraseFlipped: true,
        random,
      }),
    ).toEqual({ advance: false, musical: false, forced: false });
    expect(random).toHaveBeenCalledTimes(1);
  });

  it("advances musically when the phrase flips and the chance roll succeeds", () => {
    expect(
      shouldAdvanceRow({
        dwell: 7.5,
        minSeconds: 5,
        maxSeconds: 10,
        phraseFlipped: true,
        random: () => 0.49,
      }),
    ).toEqual({ advance: true, musical: true, forced: false });
  });

  it("does not advance musically when the chance roll fails", () => {
    expect(
      shouldAdvanceRow({
        dwell: 7.5,
        minSeconds: 5,
        maxSeconds: 10,
        phraseFlipped: true,
        random: () => 0.5,
      }),
    ).toEqual({ advance: false, musical: false, forced: false });
  });

  it("does not roll the dice without a phrase flip", () => {
    const random = vi.fn(() => 0);
    expect(
      shouldAdvanceRow({
        dwell: 7.5,
        minSeconds: 5,
        maxSeconds: 10,
        phraseFlipped: false,
        random,
      }),
    ).toEqual({ advance: false, musical: false, forced: false });
    expect(random).not.toHaveBeenCalled();
  });

  it("treats the maximum dwell as forced even on a phrase flip", () => {
    expect(
      shouldAdvanceRow({
        dwell: 10,
        minSeconds: 5,
        maxSeconds: 10,
        phraseFlipped: true,
        random: () => 1,
      }),
    ).toEqual({ advance: true, musical: false, forced: true });
  });

  it("forces at the per-cell target before the maximum", () => {
    expect(
      shouldAdvanceRow({
        dwell: 6,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: 6,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: true, musical: false, forced: true });
  });

  it("does not force before the per-cell target", () => {
    expect(
      shouldAdvanceRow({
        dwell: 5.9,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: 6,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: false, musical: false, forced: false });
  });

  it("clamps a per-cell target above the maximum", () => {
    expect(
      shouldAdvanceRow({
        dwell: 9.5,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: 20,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: false, musical: false, forced: false });
    expect(
      shouldAdvanceRow({
        dwell: 10,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: 20,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: true, musical: false, forced: true });
  });

  it("clamps a per-cell target below the minimum", () => {
    expect(
      shouldAdvanceRow({
        dwell: 4.9,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: 1,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: false, musical: false, forced: false });
    expect(
      shouldAdvanceRow({
        dwell: 5,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: 1,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: true, musical: false, forced: true });
  });

  it("treats a missing per-cell target as the maximum", () => {
    expect(
      shouldAdvanceRow({
        dwell: 9,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: undefined,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: false, musical: false, forced: false });
    expect(
      shouldAdvanceRow({
        dwell: 10,
        minSeconds: 5,
        maxSeconds: 10,
        targetSeconds: undefined,
        phraseFlipped: false,
      }),
    ).toEqual({ advance: true, musical: false, forced: true });
  });
});
