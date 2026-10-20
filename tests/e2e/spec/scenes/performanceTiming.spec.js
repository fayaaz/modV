import { expect, test } from "@playwright/test";
import {
  normalizePerformanceTiming,
  normalizeSceneTiming,
  resolveSceneTiming,
} from "../../../../src/renderer/src/application/worker/scene-timing";

const GLOBAL_TIMING = { minSeconds: 5, maxSeconds: 6 };

test.describe("scene timing helpers", () => {
  test("uses the global performance timing when a scene has no override", () => {
    const withoutFlag = {
      minSeconds: 1,
      maxSeconds: 1,
    };
    const explicitlyFalse = {
      minSeconds: 1,
      maxSeconds: 1,
      timingOverride: false,
    };

    expect(resolveSceneTiming(withoutFlag, GLOBAL_TIMING)).toEqual({
      minSeconds: 5,
      maxSeconds: 6,
      timingOverride: false,
    });
    expect(resolveSceneTiming(explicitlyFalse, GLOBAL_TIMING)).toEqual({
      minSeconds: 5,
      maxSeconds: 6,
      timingOverride: false,
    });
  });

  test("uses per-scene timing when the override flag is true", () => {
    expect(
      resolveSceneTiming(
        { minSeconds: 1.2, maxSeconds: 2.8, timingOverride: true },
        GLOBAL_TIMING,
      ),
    ).toEqual({
      minSeconds: 1.2,
      maxSeconds: 2.8,
      timingOverride: true,
    });
  });

  test("hydrates legacy scenes with scene-level times as global defaults", () => {
    const legacyScene = { minSeconds: 3, maxSeconds: 4 };

    expect(normalizeSceneTiming(legacyScene, GLOBAL_TIMING)).toEqual({
      minSeconds: 5,
      maxSeconds: 6,
      timingOverride: false,
    });
    expect(
      normalizeSceneTiming(
        { minSeconds: 3, maxSeconds: 4, timingOverride: true },
        GLOBAL_TIMING,
      ),
    ).toEqual({
      minSeconds: 3,
      maxSeconds: 4,
      timingOverride: true,
    });
  });

  test("sanitises numeric values from the UI", () => {
    expect(
      normalizePerformanceTiming({
        minSeconds: "5",
        maxSeconds: "6",
        transitionSeconds: "0",
      }),
    ).toEqual({
      minSeconds: 5,
      maxSeconds: 6,
      transitionSeconds: 0.1,
    });

    expect(
      normalizeSceneTiming(
        { minSeconds: "2.5", maxSeconds: "4.2", timingOverride: true },
        GLOBAL_TIMING,
      ),
    ).toEqual({
      minSeconds: 2.5,
      maxSeconds: 4.2,
      timingOverride: true,
    });

    expect(
      normalizePerformanceTiming({
        minSeconds: "",
        maxSeconds: Number.NaN,
        transitionSeconds: undefined,
      }),
    ).toEqual({
      minSeconds: 8,
      maxSeconds: 24,
      transitionSeconds: 2,
    });

    expect(
      normalizeSceneTiming(
        {
          minSeconds: "",
          maxSeconds: "not a number",
          timingOverride: true,
        },
        GLOBAL_TIMING,
      ),
    ).toEqual({
      minSeconds: 5,
      maxSeconds: 6,
      timingOverride: true,
    });
  });

  test("keeps minSeconds below maxSeconds", () => {
    const equal = normalizePerformanceTiming({
      minSeconds: 8,
      maxSeconds: 8,
    });
    expect(equal.maxSeconds).toBeGreaterThan(equal.minSeconds);

    const inverted = normalizePerformanceTiming({
      minSeconds: 10,
      maxSeconds: 2,
    });
    expect(inverted.maxSeconds).toBeGreaterThan(inverted.minSeconds);

    const sceneInverted = normalizeSceneTiming(
      { minSeconds: 9, maxSeconds: 2, timingOverride: true },
      GLOBAL_TIMING,
    );
    expect(sceneInverted.maxSeconds).toBeGreaterThan(sceneInverted.minSeconds);
  });
});
