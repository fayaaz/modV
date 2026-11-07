import { describe, expect, it, vi } from "vitest";
import { pickTransitionName } from "../../src/renderer/src/application/worker/scene-transition-pick.js";
import { FADE_TRANSITION } from "../../src/renderer/src/application/scene-transition-defs.js";

describe("pickTransitionName", () => {
  it("always fades while performance mode is running, even when shader or None transitions are allowed", () => {
    expect(
      pickTransitionName({
        performanceEnabled: true,
        allowedTransitions: ["None"],
        availableTransitionNames: [FADE_TRANSITION, "None"],
        random: () => 0,
      }),
    ).toBe(FADE_TRANSITION);

    expect(
      pickTransitionName({
        performanceEnabled: true,
        allowedTransitions: ["Three Wipe"],
        availableTransitionNames: [FADE_TRANSITION, "None", "Three Wipe"],
        random: () => 0,
      }),
    ).toBe(FADE_TRANSITION);

    expect(
      pickTransitionName({
        performanceEnabled: true,
        allowedTransitions: [],
        availableTransitionNames: [FADE_TRANSITION],
        random: () => 0,
      }),
    ).toBe(FADE_TRANSITION);
  });

  it("does not roll the dice while performance mode is running", () => {
    const random = vi.fn(() => 0);

    pickTransitionName({
      performanceEnabled: true,
      allowedTransitions: ["Three Wipe"],
      availableTransitionNames: [FADE_TRANSITION, "Three Wipe"],
      random,
    });

    expect(random).not.toHaveBeenCalled();
  });

  it("picks from the allowed list when performance mode is off", () => {
    expect(
      pickTransitionName({
        performanceEnabled: false,
        allowedTransitions: ["Three Wipe"],
        availableTransitionNames: [FADE_TRANSITION, "None", "Three Wipe"],
        random: () => 0,
      }),
    ).toBe("Three Wipe");
  });

  it("uses the random roll to pick when performance mode is off", () => {
    const random = vi.fn(() => 0.6);
    const names = ["A", "B", "C", "D"];

    expect(
      pickTransitionName({
        performanceEnabled: false,
        allowedTransitions: names,
        availableTransitionNames: names,
        random,
      }),
    ).toBe("C");
    expect(random).toHaveBeenCalledTimes(1);
  });

  it("drops allowed names that are not available before picking", () => {
    expect(
      pickTransitionName({
        performanceEnabled: false,
        allowedTransitions: ["E2E Missing Transition"],
        availableTransitionNames: [FADE_TRANSITION, "None"],
        random: () => 0,
      }),
    ).toBe(FADE_TRANSITION);
  });

  it("fades when nothing is allowed while performance mode is off", () => {
    expect(
      pickTransitionName({
        performanceEnabled: false,
        allowedTransitions: [],
        availableTransitionNames: [FADE_TRANSITION, "None", "Three Wipe"],
        random: () => 0.5,
      }),
    ).toBe(FADE_TRANSITION);
  });
});
