import { describe, expect, it, vi } from "vitest";
import MediaManager from "../../src/main/media-manager/index.js";

function createManager(watcher) {
  const manager = Object.create(MediaManager.prototype);

  manager.watcher = watcher;

  return manager;
}

describe("MediaManager.stop", () => {
  it("closes the watcher once and forgets it", async () => {
    const watcher = { close: vi.fn(async () => {}) };
    const manager = createManager(watcher);

    await manager.stop();

    expect(watcher.close).toHaveBeenCalledTimes(1);
    expect(manager.watcher).toBeNull();
  });

  it("is idempotent", async () => {
    const watcher = { close: vi.fn(async () => {}) };
    const manager = createManager(watcher);

    await manager.stop();
    await manager.stop();

    expect(watcher.close).toHaveBeenCalledTimes(1);
  });

  it("does nothing when the watcher was never started", async () => {
    const manager = createManager(null);

    await expect(manager.stop()).resolves.toBeUndefined();
    expect(manager.watcher).toBeNull();
  });

  it("clears the watcher even when close throws", async () => {
    const watcher = {
      close: vi.fn(() => {
        throw new Error("already closed");
      }),
    };
    const manager = createManager(watcher);

    await expect(manager.stop()).resolves.toBeUndefined();
    expect(manager.watcher).toBeNull();
  });
});
