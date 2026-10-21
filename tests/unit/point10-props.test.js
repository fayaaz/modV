import { describe, expect, it } from "vitest";
import {
  RESPONSIVENESS_PROPS,
  readLevers,
} from "../../src/renderer/src/application/point10/props.js";

describe("RESPONSIVENESS_PROPS", () => {
  it("defines every lever the bridge consumes", () => {
    expect(Object.keys(RESPONSIVENESS_PROPS)).toEqual([
      "gain",
      "smoothing",
      "bassGain",
      "midGain",
      "trebleGain",
      "useAgc",
    ]);
  });

  it("uses neutral defaults", () => {
    expect(RESPONSIVENESS_PROPS.gain.default).toBe(1);
    expect(RESPONSIVENESS_PROPS.smoothing.default).toBe(0);
    expect(RESPONSIVENESS_PROPS.bassGain.default).toBe(1);
    expect(RESPONSIVENESS_PROPS.midGain.default).toBe(1);
    expect(RESPONSIVENESS_PROPS.trebleGain.default).toBe(1);
    expect(RESPONSIVENESS_PROPS.useAgc.default).toBe(true);
  });
});

describe("readLevers", () => {
  it("falls back to neutral defaults", () => {
    expect(readLevers()).toEqual({
      gain: 1,
      smoothing: 0,
      bassGain: 1,
      midGain: 1,
      trebleGain: 1,
      useAgc: true,
    });
  });

  it("keeps AGC on unless it is explicitly false", () => {
    expect(readLevers({ useAgc: false }).useAgc).toBe(false);
    expect(readLevers({ useAgc: 0 }).useAgc).toBe(true);
    expect(readLevers({ useAgc: undefined }).useAgc).toBe(true);
  });

  it("clamps numeric levers to their supported ranges", () => {
    expect(readLevers({ gain: 10 }).gain).toBe(4);
    expect(readLevers({ gain: -1 }).gain).toBe(0);
    expect(readLevers({ smoothing: 3 }).smoothing).toBe(1);
    expect(readLevers({ bassGain: 9 }).bassGain).toBe(4);
    expect(readLevers({ midGain: -2 }).midGain).toBe(0);
    expect(readLevers({ trebleGain: 9 }).trebleGain).toBe(4);
  });

  it("ignores non-numeric values", () => {
    expect(readLevers({ gain: "loud", smoothing: null }).gain).toBe(1);
    expect(readLevers({ gain: "loud", smoothing: null }).smoothing).toBe(0);
  });
});
