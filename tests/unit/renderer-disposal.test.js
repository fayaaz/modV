import { describe, expect, it, vi } from "vitest";
import {
  createBoundedCache,
  disposeISFRenderer,
  disposePexCommand,
  getModuleId,
} from "../../src/renderer/src/application/renderers/disposal.js";

describe("getModuleId", () => {
  it("accepts a module id", () => {
    expect(getModuleId("module-1")).toBe("module-1");
  });

  it("accepts a module object", () => {
    expect(getModuleId({ $id: "module-2" })).toBe("module-2");
  });

  it("returns null for missing values and objects without an id", () => {
    expect(getModuleId(null)).toBe(null);
    expect(getModuleId(undefined)).toBe(null);
    expect(getModuleId({})).toBe(null);
  });
});

describe("disposeISFRenderer", () => {
  it("releases the shader programs, buffers and uploaded textures", () => {
    const destroyBuffer = vi.fn();
    const cleanupProgram = vi.fn();
    const cleanupPaintProgram = vi.fn();
    const cleanup = vi.fn();
    const destroyTexture = vi.fn();

    const renderer = {
      cleanup,
      program: { cleanup: cleanupProgram },
      paintProgram: { cleanup: cleanupPaintProgram },
      uniforms: {
        startImage: { texture: { destroy: destroyTexture } },
        endImage: { texture: { destroy: destroyTexture } },
        progress: { value: 0 },
      },
      renderBuffers: [{ destroy: destroyBuffer }],
    };

    disposeISFRenderer(renderer);

    expect(cleanup).toHaveBeenCalledTimes(1);
    expect(cleanupProgram).toHaveBeenCalledTimes(1);
    expect(cleanupPaintProgram).toHaveBeenCalledTimes(1);
    expect(destroyTexture).toHaveBeenCalledTimes(2);
    expect(destroyBuffer).not.toHaveBeenCalled();
  });

  it("falls back to destroying buffers when the renderer has no cleanup", () => {
    const destroyBuffer = vi.fn();

    disposeISFRenderer({ renderBuffers: [{ destroy: destroyBuffer }] });

    expect(destroyBuffer).toHaveBeenCalledTimes(1);
  });

  it("ignores missing renderers", () => {
    expect(() => disposeISFRenderer(null)).not.toThrow();
  });
});

describe("disposePexCommand", () => {
  it("disposes the pipeline through the pex context", () => {
    const pipeline = { class: "pipeline" };
    const dispose = vi.fn();
    const command = { pipeline };

    disposePexCommand({ dispose }, command);

    expect(dispose).toHaveBeenCalledWith(pipeline);
    expect(command.pipeline).toBe(null);
  });

  it("ignores commands without a pipeline", () => {
    const dispose = vi.fn();

    expect(() => disposePexCommand({ dispose }, {})).not.toThrow();
    expect(dispose).not.toHaveBeenCalled();
  });
});

describe("createBoundedCache", () => {
  it("keeps the most recently used entries and disposes evicted values", () => {
    const disposed = [];
    const cache = createBoundedCache(2, (value) => disposed.push(value));

    cache.set("a", { name: "a" });
    cache.set("b", { name: "b" });
    // Touch "a" so "b" becomes the least recently used entry.
    expect(cache.get("a")).toEqual({ name: "a" });
    cache.set("c", { name: "c" });

    expect(cache.has("a")).toBe(true);
    expect(cache.has("b")).toBe(false);
    expect(cache.has("c")).toBe(true);
    expect(disposed).toEqual([{ name: "b" }]);
  });

  it("disposes every value when cleared", () => {
    const disposed = [];
    const cache = createBoundedCache(4, (value) => disposed.push(value));

    cache.set("a", "a");
    cache.set("b", "b");
    cache.clear();

    expect(cache.size).toBe(0);
    expect(disposed).toEqual(["a", "b"]);
  });

  it("returns null for a missing entry", () => {
    const cache = createBoundedCache();

    expect(cache.get("missing")).toBe(null);
  });
});
