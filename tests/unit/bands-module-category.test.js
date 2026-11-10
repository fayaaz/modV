import { describe, expect, it } from "vitest";
import { normalizeModuleCategory } from "../../src/renderer/src/application/module-category.js";
import Bands from "../../src/renderer/src/application/renderers/shader/modules/Bands.js";

describe("Bands module", () => {
  it("is categorised as a 2D shape rather than an effect", () => {
    expect(Bands.meta.type).toBe("shader");
    expect(normalizeModuleCategory(Bands.meta.category)).toBe("2d-scenes");
  });
});
