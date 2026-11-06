import { describe, expect, it } from "vitest";
import { resolveInputLinkValue } from "../../src/renderer/src/application/worker/input-link-value.js";

describe("resolveInputLinkValue", () => {
  it("maps a value into the link range", () => {
    expect(resolveInputLinkValue(0, 1, 1.2)).toBeCloseTo(1);
    expect(resolveInputLinkValue(1, 1, 1.2)).toBeCloseTo(1.2);
    expect(resolveInputLinkValue(0.5, 1, 3)).toBeCloseTo(2);
  });

  it("keeps the raw value when the link has no range", () => {
    expect(resolveInputLinkValue(0.5)).toBe(0.5);
    expect(resolveInputLinkValue(0)).toBe(0);
  });

  it("does not turn a missing feature into NaN", () => {
    expect(resolveInputLinkValue(undefined, 1, 1.2)).toBeUndefined();
    expect(resolveInputLinkValue(null, 1, 1.2)).toBeUndefined();
  });

  it("rejects non-finite resolved values", () => {
    expect(resolveInputLinkValue(Number.NaN, 1, 1.2)).toBeUndefined();
    expect(resolveInputLinkValue(Infinity, 1, 1.2)).toBeUndefined();
  });
});
