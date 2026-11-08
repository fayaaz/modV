import { describe, expect, it } from "vitest";
import { getOtherEnabledGalleryModuleIds } from "../../src/renderer/src/application/worker/gallery-focus";

describe("getOtherEnabledGalleryModuleIds", () => {
  it("returns enabled gallery module ids excluding the target", () => {
    const activeModules = {
      a: { meta: { enabled: true } },
      b: { meta: { enabled: false } },
      c: { meta: { enabled: true } },
    };

    expect(
      getOtherEnabledGalleryModuleIds(["a", "b", "c"], activeModules, "c"),
    ).toEqual(["a"]);
  });

  it("ignores module ids missing from the active modules", () => {
    const activeModules = {
      a: { meta: { enabled: true } },
    };

    expect(
      getOtherEnabledGalleryModuleIds(["a", "ghost", "b"], activeModules, "b"),
    ).toEqual(["a"]);
  });

  it("returns an empty list when no other module is enabled", () => {
    const activeModules = {
      a: { meta: { enabled: true } },
      b: { meta: { enabled: false } },
    };

    expect(
      getOtherEnabledGalleryModuleIds(["a", "b"], activeModules, "a"),
    ).toEqual([]);
  });

  it("treats a missing meta.enabled as disabled", () => {
    const activeModules = {
      a: { meta: {} },
      b: { meta: { enabled: true } },
    };

    expect(
      getOtherEnabledGalleryModuleIds(["a", "b"], activeModules, "a"),
    ).toEqual(["b"]);
  });
});
