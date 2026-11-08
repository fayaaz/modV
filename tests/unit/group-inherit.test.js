import { describe, expect, it } from "vitest";

import { resolveInheritCanvas } from "../../src/renderer/src/application/worker/group-inherit";

function fakeGroup(contextCanvas = {}, id = "group-id") {
  return { id, context: { context: { canvas: contextCanvas } } };
}

describe("resolveInheritCanvas", () => {
  it("returns nothing for the previous frame when there is no previous Group yet", () => {
    expect(
      resolveInheritCanvas({
        inheritFrom: -1,
        lastCanvas: null,
        inheritedGroup: null,
        renderedCanvasByGroupId: {},
      }),
    ).toBeNull();
  });

  it("returns the previous Group's canvas when one has been rendered", () => {
    const canvas = { name: "previous" };

    expect(
      resolveInheritCanvas({
        inheritFrom: -1,
        lastCanvas: canvas,
        inheritedGroup: null,
        renderedCanvasByGroupId: {},
      }),
    ).toBe(canvas);
  });

  it("returns the rendered canvas of an explicitly named Group", () => {
    const rendered = { name: "rendered" };
    const fallback = { name: "fallback" };

    expect(
      resolveInheritCanvas({
        inheritFrom: "group-id",
        lastCanvas: null,
        inheritedGroup: fakeGroup(fallback),
        renderedCanvasByGroupId: { "group-id": rendered },
      }),
    ).toBe(rendered);
  });

  it("falls back to the named Group's own canvas", () => {
    const fallback = { name: "fallback" };

    expect(
      resolveInheritCanvas({
        inheritFrom: "group-id",
        lastCanvas: null,
        inheritedGroup: fakeGroup(fallback),
        renderedCanvasByGroupId: {},
      }),
    ).toBe(fallback);
  });

  it("returns nothing when the named Group no longer exists", () => {
    expect(
      resolveInheritCanvas({
        inheritFrom: "missing-id",
        lastCanvas: null,
        inheritedGroup: null,
        renderedCanvasByGroupId: {},
      }),
    ).toBeNull();
  });
});
