// Media library mirroring for engine mode.
//
// In engine mode the renderer's Vuex store is a read-only mirror of the engine
// (`engine-client.js`), so the main process' media-manager IPC handlers
// (`media-manager-state`, `media-manager-update`, `media-manager-path-changed`)
// used to dispatch straight into the engine facade and never reached the
// mirror. The UI reads `media.media[currentProject]` (TextureControl's image and
// video selects) and `media.path` (the base directory a saved media path is
// relative to) from the mirror, so without this replay engine mode had an empty
// video list and no way to resolve a clip's absolute path for
// `media/start-file`.
//
// The helpers are pure (they only call `store.commit`) so they can be unit
// tested with a recording fake store.

/**
 * Replay the full `media-manager-state` payload (the main process' media store
 * snapshot) into the mirror.
 *
 * @param {{commit: (type: string, payload?: unknown) => void}} store
 * @param {{path?: string, media?: object} | null | undefined} state
 */
export function applyMediaStateToMirror(store, state) {
  if (!store || !state || typeof state !== "object") {
    return;
  }

  store.commit("media/CLEAR_MEDIA_STATE");

  if (typeof state.path === "string" && state.path.length > 0) {
    store.commit("media/SET_MEDIA_DIRECTORY_PATH", { path: state.path });
  }

  const media = state.media || {};

  for (const project of Object.keys(media)) {
    const folders = media[project] || {};

    for (const folder of Object.keys(folders)) {
      const items = folders[folder] || {};

      for (const item of Object.values(items)) {
        if (item && typeof item === "object") {
          store.commit("media/ADD", { project, folder, item });
        }
      }
    }
  }
}

/**
 * Replay one media-manager mutation (`media-manager-update`) into the mirror.
 *
 * @param {{commit: (type: string, payload?: unknown) => void}} store
 * @param {{type?: string, payload?: {project?: string, folder?: string, item?: object}} | null | undefined} message
 */
export function applyMediaUpdateToMirror(store, message) {
  const { project, folder, item } = message?.payload || {};

  if (!store || !project || !folder || !item || typeof item !== "object") {
    return;
  }

  store.commit("media/ADD", { project, folder, item });
}

/**
 * Replay a `media-manager-path-changed` message into the mirror.
 *
 * @param {{commit: (type: string, payload?: unknown) => void}} store
 * @param {{path?: string} | null | undefined} payload
 */
export function applyMediaPathToMirror(store, payload) {
  if (
    !store ||
    typeof payload?.path !== "string" ||
    payload.path.length === 0
  ) {
    return;
  }

  store.commit("media/SET_MEDIA_DIRECTORY_PATH", { path: payload.path });
}
