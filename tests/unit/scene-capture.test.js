import { describe, expect, it } from "vitest";
import constants from "../../src/renderer/src/application/constants.js";
import {
  buildCapturedGroups,
  buildCapturedGroupsFromLive,
  canCaptureScene,
  collectSceneGroups,
  sceneHasGroups,
} from "../../src/renderer/src/application/worker/scene-capture.js";

const GALLERY = constants.GALLERY_GROUP_NAME;

function scene(id, groups, extra = {}) {
  return { id, name: id, groups, ...extra };
}

function liveGroup(id, name, extra = {}) {
  return { id, name, modules: [], ...extra };
}

describe("buildCapturedGroups", () => {
  it("returns null when the scene is not applied", () => {
    expect(
      buildCapturedGroups(scene("s1", []), [
        liveGroup("g1", "Other", { sceneId: "s2", sceneRow: 0 }),
        liveGroup("g2", "Unassigned", {}),
      ]),
    ).toBeNull();
  });

  it("captures only the live groups belonging to the scene", () => {
    const captured = buildCapturedGroups(scene("s1", [null]), [
      liveGroup("g1", "Other", { sceneId: "s2", sceneRow: 0 }),
      liveGroup("g2", "Scene group", {
        sceneId: "s1",
        sceneRow: 0,
        alpha: 0.5,
      }),
      liveGroup("g3", "Unassigned", {}),
    ]);

    expect(captured).not.toBeNull();
    expect(captured.groups).toHaveLength(1);
    expect(captured.groups[0].id).toBe("g2");
    expect(captured.groups[0].alpha).toBe(0.5);
  });

  it("excludes transition ghosts and the Gallery group", () => {
    const captured = buildCapturedGroups(scene("s1", []), [
      liveGroup("g1", "Ghost", {
        sceneId: "s1",
        sceneRow: 0,
        transitionGhost: true,
      }),
      liveGroup("g2", GALLERY, { sceneId: "s1", sceneRow: 1 }),
      liveGroup("g3", "Real", { sceneId: "s1", sceneRow: 2 }),
    ]);

    // The excluded rows become holes rather than collapsing positions.
    expect(captured.groups.map((group) => group && group.id)).toEqual([
      null,
      null,
      "g3",
    ]);
  });

  it("orders groups by row and leaves holes as null", () => {
    const captured = buildCapturedGroups(scene("s1", []), [
      liveGroup("g2", "Row 1", { sceneId: "s1", sceneRow: 2 }),
      liveGroup("g0", "Row 0", { sceneId: "s1", sceneRow: 0 }),
    ]);

    // Row 1 became a hole because its live group is gone; the remaining rows
    // keep their positions.
    expect(captured.groups.map((group) => group && group.id)).toEqual([
      "g0",
      null,
      "g2",
    ]);
  });

  it("clones the captured groups off the live store objects", () => {
    const live = liveGroup("g1", "Real", {
      sceneId: "s1",
      sceneRow: 0,
      context: { ctx: true },
      writeToSwap: true,
      readFromSwap: false,
    });
    const captured = buildCapturedGroups(scene("s1", []), [live]);

    const capturedGroup = captured.groups[0];
    expect(capturedGroup).not.toBe(live);
    expect(capturedGroup.context).toBeUndefined();
    expect(capturedGroup.writeToSwap).toBeUndefined();
    expect(capturedGroup.readFromSwap).toBeUndefined();

    // Mutating the captured group does not touch the live group either.
    expect(live.name).toBe("Real");
    capturedGroup.name = "Changed";
    expect(live.name).toBe("Real");
  });

  it("ignores groups without a scene row", () => {
    const captured = buildCapturedGroups(scene("s1", []), [
      liveGroup("g1", "Dangling", { sceneId: "s1" }),
    ]);

    expect(captured).toBeNull();
  });
});

describe("buildCapturedGroupsFromLive", () => {
  it("returns null when there are no user groups", () => {
    expect(buildCapturedGroupsFromLive([])).toBeNull();
    expect(
      buildCapturedGroupsFromLive([
        liveGroup("g1", GALLERY),
        liveGroup("g2", "Ghost", { transitionGhost: true }),
        null,
      ]),
    ).toBeNull();
  });

  it("collects every live user group and drops the Gallery and ghosts", () => {
    const captured = buildCapturedGroupsFromLive([
      liveGroup("g1", GALLERY),
      liveGroup("g2", "Ghost", { transitionGhost: true }),
      liveGroup("g3", "Foreground"),
    ]);

    expect(captured.groups.map((group) => group && group.id)).toEqual(["g3"]);
  });

  it("uses scene rows when present and keeps holes", () => {
    const captured = buildCapturedGroupsFromLive([
      liveGroup("g2", "Row 2", { sceneRow: 2 }),
      liveGroup("g0", "Row 0", { sceneRow: 0 }),
    ]);

    expect(captured.groups.map((group) => group && group.id)).toEqual([
      "g0",
      null,
      "g2",
    ]);
  });

  it("falls back to the live order for unstamped groups", () => {
    const captured = buildCapturedGroupsFromLive([
      liveGroup("g0", "First"),
      liveGroup("g1", "Second"),
    ]);

    expect(captured.groups.map((group) => group && group.id)).toEqual([
      "g0",
      "g1",
    ]);
  });

  it("clones groups and strips render-only fields", () => {
    const live = liveGroup("g1", "Real", {
      context: { ctx: true },
      writeToSwap: true,
      readFromSwap: false,
    });
    const captured = buildCapturedGroupsFromLive([live]);

    expect(captured.groups[0]).not.toBe(live);
    expect(captured.groups[0].context).toBeUndefined();
    expect(captured.groups[0].writeToSwap).toBeUndefined();
    expect(captured.groups[0].readFromSwap).toBeUndefined();
    expect(live.name).toBe("Real");
  });
});

describe("sceneHasGroups", () => {
  it("detects populated and empty scene documents", () => {
    expect(sceneHasGroups(scene("s1", []))).toBe(false);
    expect(sceneHasGroups(scene("s1", [null, null]))).toBe(false);
    expect(sceneHasGroups(scene("s1", [null, { id: "g1" }]))).toBe(true);
    expect(sceneHasGroups({ id: "legacy", layers: [] })).toBe(false);
    expect(sceneHasGroups({ id: "legacy", layers: [{ $id: "m1" }] })).toBe(
      true,
    );
  });
});

describe("collectSceneGroups", () => {
  it("returns null for a populated Scene that is not applied", () => {
    expect(
      collectSceneGroups(scene("s1", [{ id: "g1" }]), [
        liveGroup("g1", "Other", { sceneId: "s2", sceneRow: 0 }),
      ]),
    ).toBeNull();
  });

  it("captures an applied Scene's stamped groups", () => {
    const collected = collectSceneGroups(scene("s1", []), [
      liveGroup("g1", "Own", { sceneId: "s1", sceneRow: 0 }),
    ]);

    expect(collected.fromLive).toBe(false);
    expect(collected.groups.map((group) => group && group.id)).toEqual(["g1"]);
  });

  it("captures the live layer stack into an empty Scene column", () => {
    const collected = collectSceneGroups(scene("s1", []), [
      liveGroup("g1", "Foreground"),
      liveGroup("g2", "Background"),
    ]);

    expect(collected.fromLive).toBe(true);
    expect(collected.groups.map((group) => group && group.id)).toEqual([
      "g1",
      "g2",
    ]);
  });

  it("returns null for an empty Scene when there is nothing live", () => {
    expect(
      collectSceneGroups(scene("s1", []), [liveGroup("g1", GALLERY)]),
    ).toBeNull();
  });
});

describe("canCaptureScene", () => {
  it("allows applied and empty Scenes, but not unapplied populated ones", () => {
    const empty = scene("s1", []);
    const populated = scene("s2", [{ id: "g1" }]);
    const live = [liveGroup("g1", "Own", { sceneId: "s1", sceneRow: 0 })];

    expect(canCaptureScene(empty, live)).toBe(true);
    expect(canCaptureScene(populated, live)).toBe(false);
  });
});
