// Module `meta.enabled` normalisation for engine mode.
//
// Legacy modV renders a Module when `!module.meta.enabled` is false, i.e. on JS
// truthiness (`worker/loop.js:389`). The Enable Input Link therefore stores a
// *number* (`1`/`0`), and Scene/preset documents saved by the legacy renderer
// carry `meta.enabled: 1` for their Modules. The Rust compositor is stricter:
// it only draws a Module whose `meta.enabled` is the JSON boolean `true`
// (`crates/modv-app/src/compositor.rs:1410`, `crates/modv-app/src/input_links.rs:164`),
// so every Module restored from such a Scene (a Webcam included) silently
// draws nothing in engine mode — the camera source is running and the panel
// says Live, but the layer is skipped.
//
// The renderer owns the Scene documents (they are written by its own capture /
// preset paths), so it repairs the encoding when it sees the engine's Modules:
// `engineModuleEnabledPatches` reports which Modules need `modules/updateMeta`
// with a boolean, and `index.js` dispatches it once per Module.
//
// Kept dependency-free so `tests/unit/engine-module-meta.test.js` can drive it
// in plain Node.

/**
 * The boolean a Module's `enabled` flag should be, or `null` when the current
 * value is already acceptable (a real boolean, or absent — legacy treats a
 * missing flag as disabled, which is what the engine does too).
 *
 * Non-boolean values are coerced with JS truthiness, exactly as the legacy
 * render loop did (`!module.meta.enabled`), so a numeric `1` from the Enable
 * Input Link stays enabled and a `0` stays disabled.
 *
 * @param {{meta?: {enabled?: unknown}, $id?: string, id?: string} | null | undefined} module
 * @returns {boolean | null}
 */
export function booleanModuleEnabled(module) {
  const enabled = module?.meta?.enabled;

  if (enabled === undefined || enabled === null) {
    return null;
  }

  if (typeof enabled === "boolean") {
    return null;
  }

  return !!enabled;
}

/**
 * One patch per Module whose `enabled` flag is not a real boolean.
 *
 * Accepts the shapes the engine uses: the `state.modules.active` map
 * (`{ [id]: module }`), an array of Modules, or a single `{ module }` payload
 * (the `modules/ADD_ACTIVE_MODULE` delta). The id comes from `$id`, `id`, or
 * the map key, in that order.
 *
 * @param {unknown} modules
 * @returns {Array<{id: string, enabled: boolean}>}
 */
export function engineModuleEnabledPatches(modules) {
  if (!modules) {
    return [];
  }

  const entries = Array.isArray(modules)
    ? modules.map((module) => [undefined, module])
    : typeof modules !== "object"
      ? []
      : modules.module
        ? [[undefined, modules.module]]
        : // A single Module object (the delta payload's `module`) has a `meta`
          // map of its own; a `state.modules.active` map does not.
          typeof modules.meta === "object" ||
            typeof modules.$moduleName === "string"
          ? [[undefined, modules]]
          : Object.entries(modules);

  const patches = [];

  for (const [key, value] of entries) {
    if (!value || typeof value !== "object") {
      continue;
    }

    const enabled = booleanModuleEnabled(value);

    if (enabled === null) {
      continue;
    }

    const id =
      (typeof value.$id === "string" && value.$id) ||
      (typeof value.id === "string" && value.id) ||
      (typeof key === "string" && key) ||
      "";

    if (!id) {
      continue;
    }

    patches.push({ id, enabled });
  }

  return patches;
}
