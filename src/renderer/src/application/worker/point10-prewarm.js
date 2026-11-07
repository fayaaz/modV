// Helpers for warming a Scene's Point10 scenes before it is applied.
//
// A heavy WebGPU Scene (Smoke Room, Ball of Fire) builds a large TSL graph and
// allocates its simulation textures when its Module becomes active. Doing that
// on the Scene-change path stalls the worker and freezes the crossfade. Instead
// the performance loop picks the next Scene for a row ahead of time, warms its
// Point10 scenes as reusable templates, and applies the same Scene later when
// the move becomes due (see `resolveRowAdvanceScene`).

const getModuleName = (snapshot) => {
  if (!snapshot) {
    return null;
  }

  return snapshot.$moduleName || (snapshot.meta && snapshot.meta.name) || null;
};

const addSnapshotIds = (moduleIds, modules) => {
  if (Array.isArray(modules)) {
    for (let i = 0; i < modules.length; i++) {
      moduleIds.push(modules[i]);
    }

    return;
  }

  if (modules && typeof modules === "object") {
    const keys = Object.keys(modules);

    for (let i = 0; i < keys.length; i++) {
      moduleIds.push(keys[i]);
    }
  }
};

/**
 * The default value for every prop a Module definition declares. A warmed
 * template is built from these, then the live Module's props are applied when
 * it is adopted, so the template only needs to be good enough to compile the
 * scene's shaders.
 */
export function modulePropDefaults(definition) {
  const defaults = {};
  const props = (definition && definition.props) || {};
  const keys = Object.keys(props);

  for (let i = 0; i < keys.length; i++) {
    const key = keys[i];
    const prop = props[key];

    if (prop && Object.prototype.hasOwnProperty.call(prop, "default")) {
      defaults[key] = prop.default;
    }
  }

  return defaults;
}

/**
 * Every distinct Point10 Module used by a Scene that can be warmed ahead of
 * time. Only WebGPU scenes are warmed: they are the expensive ones, and their
 * instances do not compete for the shared WebGL context budget. Non-Point10
 * Modules have no `prewarm` hook and are skipped.
 */
export function getPoint10PrewarmTargets(scene, registered) {
  if (!scene || !registered) {
    return [];
  }

  const groups = Array.isArray(scene.groups) ? scene.groups : [];
  const moduleSnapshots = scene.modules || {};
  const moduleIds = [];

  for (let i = 0; i < groups.length; i++) {
    addSnapshotIds(moduleIds, groups[i] && groups[i].modules);
  }

  const seen = new Set();
  const targets = [];

  for (let i = 0; i < moduleIds.length; i++) {
    const snapshot = moduleSnapshots[moduleIds[i]];
    const name = getModuleName(snapshot);
    const definition = name && registered[name];

    if (
      !definition ||
      typeof definition.prewarm !== "function" ||
      !definition.webgpu ||
      seen.has(name)
    ) {
      continue;
    }

    seen.add(name);
    targets.push({
      name,
      definition,
      defaults: modulePropDefaults(definition),
    });
  }

  return targets;
}

/**
 * Warms every Point10 scene a Scene uses. Failures are isolated so one
 * unavailable scene (no WebGPU adapter, for example) does not prevent the
 * rest from warming.
 */
export async function prewarmSceneModules({
  scene,
  registered,
  width,
  height,
}) {
  const targets = getPoint10PrewarmTargets(scene, registered);

  for (let i = 0; i < targets.length; i++) {
    const { name, definition, defaults } = targets[i];

    try {
      await definition.prewarm({
        module: {
          $id: `prewarm:${name}`,
          props: defaults,
          meta: {},
        },
        width,
        height,
        warmOnly: true,
      });
    } catch (error) {
      console.warn(`Could not prewarm Point10 Scene "${name}":`, error);
    }
  }

  return targets.length;
}
