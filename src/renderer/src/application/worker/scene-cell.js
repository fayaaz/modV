import { getRowGroup } from "./scene-rows";

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function getModuleIds(group) {
  const modules = group && group.modules;

  if (Array.isArray(modules)) {
    return modules.slice();
  }

  if (modules && typeof modules === "object") {
    return Object.keys(modules);
  }

  return [];
}

function pruneUnreferencedModules(groups, modules) {
  const referenced = new Set();

  for (let i = 0, len = groups.length; i < len; i++) {
    for (const moduleId of getModuleIds(groups[i])) {
      referenced.add(moduleId);
    }
  }

  for (const moduleId of Object.keys(modules)) {
    if (!referenced.has(moduleId)) {
      delete modules[moduleId];
    }
  }
}

function mergeTweens(existing, incoming) {
  const asObject =
    !Array.isArray(existing) && !!existing && typeof existing === "object";
  const map = new Map();

  const add = (tweens) => {
    if (Array.isArray(tweens)) {
      for (const tween of tweens) {
        if (tween && tween.id) {
          map.set(tween.id, tween);
        }
      }
      return;
    }

    if (tweens && typeof tweens === "object") {
      for (const [id, tween] of Object.entries(tweens)) {
        if (tween) {
          map.set(tween.id || id, tween);
        }
      }
    }
  };

  add(existing);
  add(incoming);

  if (asObject) {
    return Object.fromEntries(map);
  }

  return Array.from(map.values());
}

/**
 * Removes every Module from the Group saved in a performance-grid cell. The
 * returned `moduleIds` lets the caller clean up the same Modules from the live
 * Group when that cell is currently applied.
 */
export function clearSceneCellModules(scene, rowIndex, rowNames) {
  const sourceGroups = Array.isArray(scene && scene.groups) ? scene.groups : [];
  const group = getRowGroup(scene, rowIndex, rowNames);
  const index = group ? sourceGroups.indexOf(group) : -1;

  if (index < 0) {
    return null;
  }

  const moduleIds = getModuleIds(group);

  if (!moduleIds.length) {
    return null;
  }

  const groups = clone(sourceGroups);
  groups[index].modules = [];

  const modules = clone((scene && scene.modules) || {});

  for (const moduleId of moduleIds) {
    delete modules[moduleId];
  }

  return { groups, modules, moduleIds };
}

/**
 * Writes an edited live Group back into a performance-grid cell, merging the
 * cell's input links, expressions and tweens with the rest of the Scene.
 */
export function mergeSceneCell(scene, rowIndex, rowNames, cell = {}) {
  if (!cell.group) {
    return null;
  }

  const sourceGroups = Array.isArray(scene && scene.groups) ? scene.groups : [];
  const existing = getRowGroup(scene, rowIndex, rowNames);
  const index = existing ? sourceGroups.indexOf(existing) : -1;
  const groups = clone(sourceGroups);
  const group = clone(cell.group);

  if (index > -1) {
    groups[index] = group;
  } else {
    groups.push(group);
  }

  const modules = {
    ...clone((scene && scene.modules) || {}),
    ...clone(cell.modules || {}),
  };

  pruneUnreferencedModules(groups, modules);

  const baseInputs =
    scene && scene.inputs && typeof scene.inputs === "object"
      ? scene.inputs
      : {};
  const cellInputs =
    cell.inputs && typeof cell.inputs === "object" ? cell.inputs : {};
  const inputs = {
    ...baseInputs,
    focusedInput: baseInputs.focusedInput || { id: null, title: null },
    inputs: { ...(baseInputs.inputs || {}), ...(cellInputs.inputs || {}) },
    inputLinks: {
      ...(baseInputs.inputLinks || {}),
      ...(cellInputs.inputLinks || {}),
    },
  };

  const baseAssignments =
    (scene && scene.expressions && scene.expressions.assignments) || {};
  const cellAssignments =
    (cell.expressions && cell.expressions.assignments) || {};

  return {
    groups,
    modules,
    inputs,
    expressions: {
      ...((scene && scene.expressions) || {}),
      assignments: { ...baseAssignments, ...cellAssignments },
    },
    tweens: mergeTweens(scene && scene.tweens, cell.tweens),
  };
}
