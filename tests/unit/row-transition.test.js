import { describe, expect, it } from "vitest";
import {
  getRowFadeAlphas,
  renderRowFade,
} from "../../src/renderer/src/application/worker/row-transition.js";

describe("getRowFadeAlphas", () => {
  it("starts with the outgoing row fully visible", () => {
    expect(
      getRowFadeAlphas({ progress: 0, fromAlpha: 0.8, toAlpha: 1 }),
    ).toEqual({
      progress: 0,
      eased: 0,
      fromAlpha: 0.8,
      toAlpha: 0,
    });
  });

  it("ends with the incoming row fully visible", () => {
    expect(
      getRowFadeAlphas({ progress: 1, fromAlpha: 0.8, toAlpha: 1 }),
    ).toEqual({
      progress: 1,
      eased: 1,
      fromAlpha: 0,
      toAlpha: 1,
    });
  });

  it("splits both halves evenly at the midpoint", () => {
    expect(getRowFadeAlphas({ progress: 0.5 })).toEqual({
      progress: 0.5,
      eased: 0.5,
      fromAlpha: 0.5,
      toAlpha: 0.5,
    });
  });

  it("clamps out-of-range progress and missing alphas", () => {
    expect(getRowFadeAlphas({ progress: -1 }).fromAlpha).toBe(1);
    expect(getRowFadeAlphas({ progress: 2 }).toAlpha).toBe(1);
    expect(getRowFadeAlphas({ progress: "nope" }).eased).toBe(0);
    expect(
      getRowFadeAlphas({ progress: 0.5, fromAlpha: 4, toAlpha: -2 }),
    ).toMatchObject({ fromAlpha: 0.5, toAlpha: 0 });
  });
});

describe("renderRowFade", () => {
  it("returns the incoming canvas when the fade cannot be rendered", () => {
    const toCanvas = { width: 10, height: 10 };

    expect(
      renderRowFade({
        fromCanvas: null,
        toCanvas,
        progress: 0.5,
        width: 0,
        height: 10,
        rowIndex: 0,
      }),
    ).toBe(toCanvas);
  });
});
