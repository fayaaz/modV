const SWAP_MODULES = ["groups", "modules", "inputs", "expressions"];

export const SCENES_PLAYLIST_TYPE = "modv-scenes-playlist";

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function hasAction(store, type) {
  return !store._actions || !!store._actions[type];
}

/**
 * Serialises every store module that implements `createPresetData`. The result
 * is the canonical preset document and is also used as the payload for Scenes.
 */
export async function createPresetData(store) {
  const preset = {};
  const storeModuleKeys = Object.keys(store.state);

  for (let i = 0, len = storeModuleKeys.length; i < len; i++) {
    const storeModuleKey = storeModuleKeys[i];

    if (!hasAction(store, `${storeModuleKey}/createPresetData`)) {
      continue;
    }

    try {
      const data = await store.dispatch(`${storeModuleKey}/createPresetData`);

      if (data !== undefined) {
        preset[storeModuleKey] = data;
      }
    } catch (e) {
      // Most store modules do not provide preset data.
    }
  }

  return clone(preset);
}

/**
 * Loads a preset document into the store. Scene documents are presets with
 * extra metadata keys, so those keys are ignored because they are not part of
 * the store state.
 */
export async function loadPresetData(store, preset) {
  const cleanupFunctionsAfterSwap = [];
  const storeModuleKeys = Object.keys(preset || {});

  for (let i = 0, len = storeModuleKeys.length; i < len; i++) {
    const storeModuleKey = storeModuleKeys[i];

    if (
      !Object.prototype.hasOwnProperty.call(store.state, storeModuleKey) ||
      preset[storeModuleKey] === undefined ||
      !hasAction(store, `${storeModuleKey}/loadPresetData`)
    ) {
      continue;
    }

    try {
      const fn = await store.dispatch(
        `${storeModuleKey}/loadPresetData`,
        preset[storeModuleKey],
      );

      if (typeof fn === "function") {
        cleanupFunctionsAfterSwap.push(fn);
      }
    } catch (e) {
      console.error(e);
    }
  }

  for (let i = 0, len = SWAP_MODULES.length; i < len; i++) {
    store.commit(`${SWAP_MODULES[i]}/SWAP`);
  }

  for (let i = 0, len = cleanupFunctionsAfterSwap.length; i < len; i++) {
    cleanupFunctionsAfterSwap[i]();
  }
}

/**
 * A Scene document is a preset document plus `id`, `name` and timing metadata.
 */
export function isSceneDocument(value) {
  return !!(
    value &&
    typeof value === "object" &&
    typeof value.id === "string" &&
    value.id &&
    typeof value.name === "string" &&
    (Array.isArray(value.groups) || Array.isArray(value.layers))
  );
}

/**
 * A Scenes playlist is a portable list of Scene documents plus performance
 * configuration.
 */
export function isScenesPlaylist(value) {
  return !!(
    value &&
    typeof value === "object" &&
    value.type === SCENES_PLAYLIST_TYPE &&
    Array.isArray(value.scenes)
  );
}
