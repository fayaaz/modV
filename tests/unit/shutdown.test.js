import { describe, expect, it, vi } from "vitest";
import { withTimeout } from "../../src/main/shutdown.js";

describe("withTimeout", () => {
  it("resolves with the promise value when it settles first", async () => {
    await expect(
      withTimeout(Promise.resolve("done"), 50, "fallback"),
    ).resolves.toBe("done");
  });

  it("resolves with the fallback when the promise times out", async () => {
    const never = new Promise(() => {});

    await expect(withTimeout(never, 10, "fallback")).resolves.toBe("fallback");
  });

  it("resolves with the fallback when the promise rejects", async () => {
    const thrown = Promise.reject(new Error("boom"));

    await expect(withTimeout(thrown, 50, "fallback")).resolves.toBe("fallback");
  });

  it("does not leave the timeout timer holding the process open", async () => {
    vi.useFakeTimers();

    try {
      const result = withTimeout(Promise.resolve(1), 5000, 0);

      await expect(result).resolves.toBe(1);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
