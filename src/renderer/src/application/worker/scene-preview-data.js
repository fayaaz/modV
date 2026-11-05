import { getRowGroup, isRowCellEmpty } from "./scene-rows.js";

const clone = (value) =>
  value === undefined ? undefined : JSON.parse(JSON.stringify(value));

const getModuleIds = (group) => {
  const modules = group && group.modules;

  if (Array.isArray(modules)) {
    return modules;
  }

  if (modules && typeof modules === "object") {
    return Object.keys(modules);
  }

  return [];
};

/**
 * Returns the saved Group/module snapshots for a grid cell. Preview modules
 * are built from these snapshots, never from the live Groups, so hovering a
 * cell cannot disturb the running composition.
 */
export function getSceneRowPreview(scene, rowIndex, rowNames) {
  if (isRowCellEmpty(scene, rowIndex, rowNames)) {
    return null;
  }

  const group = getRowGroup(scene, rowIndex, rowNames);
  const moduleSnapshots = scene.modules || {};
  const modules = getModuleIds(group)
    .map((moduleId) => moduleSnapshots[moduleId])
    .filter(Boolean);

  if (!modules.length) {
    return null;
  }

  return { group, modules };
}

/**
 * Builds a standalone Module for the preview renderer. It gets a fresh id so
 * it cannot collide with a live Module's shader command, ISF renderer or
 * Point10 scene instance, and it is excluded from presets by living outside
 * the worker store's active Modules.
 */
export function createPreviewModule(snapshot, definition, id) {
  if (!snapshot || !definition || !id) {
    return null;
  }

  const module = {
    ...clone(snapshot),
    $id: id,
    $moduleName:
      snapshot.$moduleName || (definition.meta && definition.meta.name),
    meta: {
      ...(definition.meta || {}),
      ...clone(snapshot.meta),
      isGallery: false,
      scenePreview: true,
    },
    props: {
      ...clone(definition.props || {}),
      ...clone(snapshot.props || {}),
    },
    $props: clone(snapshot.$props) || clone(definition.props || {}),
    data:
      snapshot.data === undefined
        ? clone(definition.data || {})
        : clone(snapshot.data),
    $status: [],
  };

  if (module.meta.alpha === undefined) {
    module.meta.alpha = 1;
  }

  if (module.meta.enabled === undefined) {
    module.meta.enabled = true;
  }

  if (!module.meta.compositeOperation) {
    module.meta.compositeOperation = "source-over";
  }

  return module;
}
