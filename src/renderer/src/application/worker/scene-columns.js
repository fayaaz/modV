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
