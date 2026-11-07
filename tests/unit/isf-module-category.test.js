import fs from "fs";
import path from "path";
import { describe, expect, it } from "vitest";
import {
  getIsfGalleryCategory,
  normalizeModuleCategory,
} from "../../src/renderer/src/application/module-category.js";

const ISF_DIR = path.resolve(
  "src/renderer/src/application/renderers/isf/modules/isf",
);

function readIsf(fileName) {
  const source = fs.readFileSync(path.join(ISF_DIR, fileName), "utf8");
  const match = source.match(/\/\*([\s\S]*?)\*\//);

  if (!match) {
    throw new Error(`No ISF metadata block in ${fileName}`);
  }

  return { metadata: JSON.parse(match[1]), source };
}

describe("module category", () => {
  it("normalises gallery category aliases", () => {
    expect(normalizeModuleCategory("3D Scenes")).toBe("3d-scenes");
    expect(normalizeModuleCategory("3d")).toBe("3d-scenes");
    expect(normalizeModuleCategory("2d-scene")).toBe("2d-scenes");
    expect(normalizeModuleCategory("effects")).toBe("effects");
    expect(normalizeModuleCategory("nonsense")).toBeNull();
    expect(normalizeModuleCategory(undefined)).toBeNull();
  });

  it("reads the gallery category from ISF metadata", () => {
    expect(getIsfGalleryCategory({ MODV_CATEGORY: "3D Scenes" })).toBe(
      "3d-scenes",
    );
    expect(getIsfGalleryCategory({})).toBeUndefined();
    expect(getIsfGalleryCategory(null)).toBeUndefined();
  });
});

describe("3D scene ISF modules", () => {
  const files = [
    "digital-crystal-tunnel.fs",
    "pixel-tunnel.fs",
    "fractal-mage-tunnel.fs",
    "lopy-frac-tunnel.fs",
    "fractal-mountain-tunnel.fs",
  ];

  it.each(files)("%s is tagged as a 3D scene", (fileName) => {
    const { metadata } = readIsf(fileName);

    expect(getIsfGalleryCategory(metadata)).toBe("3d-scenes");
  });

  it.each(files.slice(1))("%s keeps an MIT licence", (fileName) => {
    const { source } = readIsf(fileName);

    expect(source).toMatch(/MIT License/);
  });

  it.each(files)("%s only uses GLSL ES 1.00 builtins", (fileName) => {
    const { source } = readIsf(fileName);

    expect(source).not.toMatch(
      /\b(texture|tanh|round|trunc|textureLod|textureGrad)\s*\(/,
    );
  });
});

describe("CC0 scene ISF modules", () => {
  const files = [
    ["Cant Find My Way Out.fs", "3d-scenes"],
    ["Neon Hyperspace Corridor.fs", "3d-scenes"],
    ["Warp Conduit.fs", "3d-scenes"],
    ["The Pool Rooms.fs", "3d-scenes"],
    ["Back The Way You Came.fs", "3d-scenes"],
    ["River Walk.fs", "3d-scenes"],
    ["Ponder Orb.fs", "3d-scenes"],
    ["Plexus.fs", "3d-scenes"],
    ["Mesmer.fs", "2d-scenes"],
    ["Singularity Rain.fs", "2d-scenes"],
    ["rave.exe.fs", "2d-scenes"],
  ];

  it.each(files)("%s is tagged as %s", (fileName, category) => {
    const { metadata } = readIsf(fileName);

    expect(getIsfGalleryCategory(metadata)).toBe(category);
  });

  it.each(files)("%s keeps its CC0 licence", (fileName) => {
    const { source } = readIsf(fileName);

    expect(source).toMatch(/CC0-1\.0/);
  });

  it.each(files)("%s only uses GLSL ES 1.00 builtins", (fileName) => {
    const { source } = readIsf(fileName);

    expect(source).not.toMatch(
      /\b(texture|tanh|round|trunc|textureLod|textureGrad)\s*\(/,
    );
  });

  it.each(files)(
    "%s declares VALUES and LABELS for every long input",
    (fileName) => {
      const { metadata } = readIsf(fileName);
      const longs = (metadata.INPUTS || []).filter(
        (input) => input.TYPE === "long",
      );

      for (const input of longs) {
        expect(input.VALUES, `${input.NAME} must declare VALUES`).toBeDefined();
        expect(input.LABELS, `${input.NAME} must declare LABELS`).toHaveLength(
          input.VALUES.length,
        );
      }
    },
  );
});
