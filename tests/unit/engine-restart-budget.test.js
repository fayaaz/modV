/**
 * Restart-budget accounting for the Rust engine supervisor (review R-24).
 *
 * The supervisor used to keep `restartTimestamps` for the lifetime of the
 * process: a clean shutdown followed by a start inherited the previous crash
 * budget, so a normal stop/start cycle could reach "fatal" after only a couple
 * of crashes. It also had no testable notion of "this attempt exhausted the
 * budget". The pure helper below owns both concerns; the supervisor resets it
 * on `start()`/`shutdown()`.
 */

import { describe, expect, it } from "vitest";

import {
  RESTART_BUDGET,
  RESTART_WINDOW_MS,
  recordRestartAttempt,
  resetRestartBudget,
} from "../../src/main/engine-restart-budget";

describe("recordRestartAttempt", () => {
  it("records the attempt and prunes timestamps outside the window", () => {
    const now = 1_000_000;
    const { timestamps, count, exhausted } = recordRestartAttempt(
      [now - RESTART_WINDOW_MS - 1, now - 10],
      now,
    );

    expect(timestamps).toEqual([now - 10, now]);
    expect(count).toBe(2);
    expect(exhausted).toBe(false);
  });

  it("keeps the budget semantics: exhausted only when count exceeds the budget", () => {
    const now = 5_000;
    const budget = 2;
    const within = [now - 3, now - 2, now - 1];

    expect(recordRestartAttempt(within, now, { budget }).exhausted).toBe(true);
    expect(recordRestartAttempt([now - 2], now, { budget }).exhausted).toBe(
      false,
    );
    expect(
      recordRestartAttempt([now - 3, now - 2], now, { budget }).exhausted,
    ).toBe(true);
  });

  it("ignores non-finite timestamps instead of poisoning the window", () => {
    const now = 2_000;
    const { timestamps, count } = recordRestartAttempt(
      [Number.NaN, undefined, now - 50],
      now,
    );

    expect(timestamps).toEqual([now - 50, now]);
    expect(count).toBe(2);
  });

  it("tolerates a missing/cleared timestamp list", () => {
    const now = 3_000;

    expect(recordRestartAttempt(undefined, now).timestamps).toEqual([now]);
    expect(recordRestartAttempt(null, now).timestamps).toEqual([now]);
    expect(recordRestartAttempt([], now).count).toBe(1);
  });

  it("defaults to the supervisor's budget and window", () => {
    const now = 10_000;
    const attempts = Array.from(
      { length: RESTART_BUDGET + 1 },
      (_, index) => now - index,
    );

    expect(recordRestartAttempt(attempts, now).exhausted).toBe(true);
    expect(RESTART_WINDOW_MS).toBe(60_000);
  });
});

describe("resetRestartBudget", () => {
  it("returns an empty list so stop/start cycles never inherit a budget", () => {
    expect(resetRestartBudget()).toEqual([]);
  });
});
