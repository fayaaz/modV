import { describe, expect, it, vi } from "vitest";
import constants from "../../src/renderer/src/application/constants.js";
import {
  getRowCount,
  getRowGroup,
  getRowLabel,
  getSceneGroups,
  isRowCellEmpty,
  isSceneEnabled,
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

  it("lets the first occurrence win a tie", () => {
    const scenes = [
      scene("one", [group("First")]),
      scene("two", [group("Second")]),
    ];
    expect(getRowLabel(scenes, 0)).toBe("First");
  });

  it("lets the first occurrence win a tie even when the later name reaches the count first", () => {
    const scenes = [
      scene("one", [group("First")]),
      scene("two", [group("Second")]),
      scene("three", [group("Second")]),
      scene("four", [group("First")]),
    ];
    expect(getRowLabel(scenes, 0)).toBe("First");
  });

  it("ignores empty names", () => {
    const scenes = [
      scene("one", [group("")]),
      scene("two", [group("Named")]),
      scene("three", [group("")]),
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
});
