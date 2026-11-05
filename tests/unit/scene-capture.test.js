import { describe, expect, it } from "vitest";
import constants from "../../src/renderer/src/application/constants.js";
import { buildCapturedGroups } from "../../src/renderer/src/application/worker/scene-capture.js";

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
