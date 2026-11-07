export const CATEGORY_ALIASES = {
  effect: "effects",
  effects: "effects",
  "2d": "2d-scenes",
  "2d-scene": "2d-scenes",
  "2d-scenes": "2d-scenes",
  "3d": "3d-scenes",
  "3d-scene": "3d-scenes",
  "3d-scenes": "3d-scenes",
  three: "three",
  "three-js": "three",
  threejs: "three",
};

export function normalizeModuleCategory(category) {
  if (typeof category !== "string" || !category.trim()) {
    return null;
  }

  const key = category
    .toLowerCase()
    .trim()
    .replace(/[\s_]+/g, "-");

  return CATEGORY_ALIASES[key] || null;
}

export function getIsfGalleryCategory(metadata) {
  if (!metadata || typeof metadata !== "object") {
    return undefined;
  }

  return normalizeModuleCategory(metadata.MODV_CATEGORY) || undefined;
}
