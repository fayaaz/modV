import { describe, expect, it, vi } from "vitest";
import {
  clearWarmInstances,
  createWarmCache,
  registerWarmCache,
} from "../../src/renderer/src/application/point10/warm-cache.js";

const value = (name) => ({ name });

describe("createWarmCache", () => {
  it("takes an entry out of the cache exactly once", () => {
    const cache = createWarmCache({ limit: 2 });
    const entry = value("a");

    cache.set("a", entry);

    expect(cache.has("a")).toBe(true);
    expect(cache.take("a")).toBe(entry);
    expect(cache.has("a")).toBe(false);
    expect(cache.take("a")).toBeNull();
  });

  it("evicts the least recently used entry over the limit", () => {
    const onEvict = vi.fn();
    const cache = createWarmCache({ limit: 2, onEvict });
    const a = value("a");
    const b = value("b");
    const c = value("c");

    cache.set("a", a);
    cache.set("b", b);
    // Touch "a" so "b" becomes the least recently used.
    cache.set("a", a);
    cache.set("c", c);

    expect(onEvict).toHaveBeenCalledTimes(1);
    expect(onEvict).toHaveBeenCalledWith(b);

    expect(cache.has("a")).toBe(true);
    expect(cache.has("b")).toBe(false);
    expect(cache.has("c")).toBe(true);
  });

  it("disposes every entry when cleared", () => {
    const onEvict = vi.fn();
    const cache = createWarmCache({ limit: 4, onEvict });
    const a = value("a");
    const b = value("b");

    cache.set("a", a);
    cache.set("b", b);
    cache.clear();

    expect(cache.size).toBe(0);
    expect(onEvict).toHaveBeenCalledTimes(2);
    expect(onEvict).toHaveBeenCalledWith(a);
    expect(onEvict).toHaveBeenCalledWith(b);
  });

  it("clears the cache registered by the renderer (store teardown)", () => {
    const onEvict = vi.fn();
    const cache = createWarmCache({ limit: 2, onEvict });

    registerWarmCache(cache);
    cache.set("a", value("a"));
    clearWarmInstances();

    expect(cache.size).toBe(0);
    expect(onEvict).toHaveBeenCalledTimes(1);
  });

  it("replaces an entry under the same key without evicting it", () => {
    const onEvict = vi.fn();
    const cache = createWarmCache({ limit: 2, onEvict });
    const older = value("older");
    const newer = value("newer");

    cache.set("same", older);
    cache.set("same", newer);

    expect(onEvict).not.toHaveBeenCalled();
    expect(cache.size).toBe(1);
    expect(cache.take("same")).toBe(newer);
  });
});
