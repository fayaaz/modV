const SCENE_NAME_PREFIX = "Scene";

const normalizeName = (name) =>
  typeof name === "string" ? name.trim().toLowerCase() : "";

/**
 * Returns the first unused "Scene N" name, so adding a column never produces
 * duplicate Scene names that the rename action would then reject.
 */
export function nextSceneName(existingNames = []) {
  const taken = new Set();

  for (let i = 0, len = existingNames.length; i < len; i++) {
    const key = normalizeName(existingNames[i]);

    if (key) {
      taken.add(key);
    }
  }

  let index = 1;
  let name = `${SCENE_NAME_PREFIX} ${index}`;

  while (taken.has(normalizeName(name))) {
    index += 1;
    name = `${SCENE_NAME_PREFIX} ${index}`;
  }

  return name;
}

/**
 * Returns the Scene list with `sceneId` moved to `toIndex`, or the original
 * list when the move is a no-op so callers can skip persisting it.
 */
export function moveSceneToIndex(scenes, sceneId, toIndex) {
  if (!Array.isArray(scenes)) {
    return scenes;
  }

  const fromIndex = scenes.findIndex((scene) => scene && scene.id === sceneId);

  if (
    fromIndex === -1 ||
    !Number.isInteger(toIndex) ||
    toIndex < 0 ||
    toIndex >= scenes.length ||
    toIndex === fromIndex
  ) {
    return scenes;
  }

  const reordered = scenes.slice();
  const [moved] = reordered.splice(fromIndex, 1);
  reordered.splice(toIndex, 0, moved);

  return reordered;
}

/**
 * Builds a Scene document with no Groups or Modules, ready to be filled by
 * Ctrl+dragging cells from other Scenes (or saved into later).
 */
export function createEmptyScene({
  id,
  name,
  existingNames = [],
  timing = {},
} = {}) {
  const requestedName = typeof name === "string" ? name.trim() : "";

  return {
    id,
    name: requestedName || nextSceneName(existingNames),
    enabled: true,
    minSeconds: timing.minSeconds,
    maxSeconds: timing.maxSeconds,
    timingOverride: false,
    groups: [],
    modules: {},
    inputs: {
      focusedInput: { id: null, title: null },
      inputs: {},
      inputLinks: {},
    },
    expressions: { assignments: {} },
    tweens: [],
    midi: {},
  };
}
