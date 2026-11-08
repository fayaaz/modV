/**
 * The ids of the Gallery group's currently enabled preview modules, minus the
 * one that just took focus. Only one preview module should render at a time.
 * Module ids with no entry in the active modules are ignored rather than
 * throwing: the lists can briefly diverge while modules are created/removed.
 */
export function getOtherEnabledGalleryModuleIds(
  galleryModuleIds,
  activeModules,
  focusedModuleId,
) {
  const toDisable = [];

  for (let i = 0; i < galleryModuleIds.length; i++) {
    const moduleId = galleryModuleIds[i];

    if (moduleId === focusedModuleId) {
      continue;
    }

    const module = activeModules[moduleId];

    if (module && module.meta && module.meta.enabled) {
      toDisable.push(moduleId);
    }
  }

  return toDisable;
}
