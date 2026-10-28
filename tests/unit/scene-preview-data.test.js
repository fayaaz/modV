import { describe, expect, it } from "vitest";
import {
  createPreviewModule,
  getSceneRowPreview,
} from "../../src/renderer/src/application/worker/scene-preview-data.js";

const definition = {
  meta: {
    name: "Ball",
    type: "2d",
    enabledInputId: "registered-enabled",
  },
  props: {
    size: { type: "int", default: 2 },
    color: { type: "tween", default: [0, 0, 0] },
  },
  data: { frames: 0 },
};

function snapshot(extra = {}) {
  return {
    $id: "scene-module",
    $moduleName: "Ball",
    meta: {
      name: "Ball",
      type: "2d",
      enabled: true,
      alpha: 0.75,
      compositeOperation: "overlay",
      enabledInputId: "scene-enabled",
    },
    props: {
      size: 8,
      color: [255, 0, 0],
    },
    $props: {
      size: { type: "int", default: 2, id: "scene-size" },
      color: { type: "tween", default: [0, 0, 0], id: "scene-color" },
    },
    data: { frames: 12 },
    $status: [{ type: "error", message: "old status" }],
    ...extra,
  };
}

describe("getSceneRowPreview", () => {
  it("returns the group and its module snapshots", () => {
    const moduleSnapshot = snapshot();
    const scene = {
      id: "scene",
      groups: [{ name: "Row", modules: ["scene-module"] }],
      modules: { "scene-module": moduleSnapshot },
    };

    const preview = getSceneRowPreview(scene, 0);

    expect(preview.group).toBe(scene.groups[0]);
    expect(preview.modules).toEqual([moduleSnapshot]);
  });

  it("skips module ids without snapshots", () => {
    const moduleSnapshot = snapshot();
    const scene = {
      groups: [{ name: "Row", modules: ["missing", "scene-module"] }],
      modules: { "scene-module": moduleSnapshot },
    };

    expect(getSceneRowPreview(scene, 0).modules).toEqual([moduleSnapshot]);
  });

  it("returns null for missing or empty cells", () => {
    expect(getSceneRowPreview({ groups: [] }, 0)).toBeNull();
    expect(
      getSceneRowPreview(
        { groups: [{ name: "Empty", modules: [] }], modules: {} },
        0,
      ),
    ).toBeNull();
  });

  it("supports object-shaped module maps", () => {
    const scene = {
      groups: [{ name: "Row", modules: { "scene-module": true } }],
      modules: { "scene-module": snapshot() },
    };

    expect(getSceneRowPreview(scene, 0).modules).toHaveLength(1);
  });
});

describe("createPreviewModule", () => {
  it("creates an isolated preview module with a fresh id", () => {
    const source = snapshot();

    const module = createPreviewModule(source, definition, "preview-module");

    expect(module.$id).toBe("preview-module");
    expect(module.$id).not.toBe(source.$id);
    expect(module.$moduleName).toBe("Ball");
    expect(module.meta.scenePreview).toBe(true);
    expect(module.meta.isGallery).toBe(false);
    expect(module.meta.enabled).toBe(true);
    expect(module.meta.alpha).toBe(0.75);
    expect(module.meta.compositeOperation).toBe("overlay");
    expect(module.props).toEqual({ size: 8, color: [255, 0, 0] });
    expect(module.data).toEqual({ frames: 12 });
    expect(module.$status).toEqual([]);
  });

  it("does not share nested objects with the scene snapshot", () => {
    const source = snapshot();

    const module = createPreviewModule(source, definition, "preview-module");

    module.props.color[0] = 1;
    module.data.frames = 99;

    expect(source.props.color[0]).toBe(255);
    expect(source.data.frames).toBe(12);
  });

  it("falls back to the registered definition props and data", () => {
    const source = snapshot({ props: undefined, data: undefined });

    const module = createPreviewModule(source, definition, "preview-module");

    expect(module.props).toEqual(definition.props);
    expect(module.data).toEqual(definition.data);
  });

  it("returns null when the module is unavailable", () => {
    expect(createPreviewModule(snapshot(), undefined, "preview")).toBeNull();
    expect(createPreviewModule(undefined, definition, "preview")).toBeNull();
  });
});
