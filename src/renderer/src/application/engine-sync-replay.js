// Late-joiner replay of the engine's `state/sync` payload (WS-14, review R-4 /
// R-11).
//
// The engine seeds default Scenes and can apply documents before the renderer's
// socket client exists, so `state/delta` alone leaves the mirror stale. It
// sends one `state/sync` event on accept, but Electron does not queue IPC: a
// renderer that subscribes later never sees it. The renderer therefore replays
// the event payload *and* a pull of the same payload (`engine/get-sync`), so
// this function must be idempotent — applying it twice leaves the same state.
//
// It is deliberately free of Electron/worker imports so it can be replayed
// against the real mirror store in `tests/unit/engine-sync-replay.test.js`.

/**
 * Tween descriptors from any shape `state.tweens` can take in a preset/sync
 * document: the preset array, the live id-keyed map, or `{ tweens: ... }`.
 *
 * @param {unknown} tweens
 * @returns {object[]}
 */
function tweenEntries(tweens) {
  if (Array.isArray(tweens)) {
    return tweens.filter((tween) => tween && typeof tween === "object");
  }
  if (!tweens || typeof tweens !== "object") {
    return [];
  }
  if (tweens.tweens !== undefined) {
    return tweenEntries(tweens.tweens);
  }
  return Object.values(tweens).filter(
    (tween) => tween && typeof tween === "object",
  );
}

/**
 * Replay an engine `state/sync` payload into the renderer's mirror store.
 *
 * Every collection is removed and re-added (or replaced) from the payload, so
 * a second call with the same payload is a no-op in terms of the resulting
 * state. `snapshot.registered` is only added: the registry is append-only for
 * the lifetime of an engine session.
 *
 * @param {import("vuex").Store} store the mirror store
 * @param {unknown} snapshot the `state/sync` / `engine/get-sync` payload
 */
export function applyEngineSync(store, snapshot) {
  if (!snapshot || typeof snapshot !== "object") {
    return;
  }

  if (Array.isArray(snapshot.groups)) {
    store.commit("groups/REPLACE_GROUPS", { groups: snapshot.groups });
  }

  const active = store.state.modules.active || {};
  for (const id of Object.keys(active)) {
    store.commit("modules/REMOVE_ACTIVE_MODULE", { moduleId: id });
  }
  for (const module of Object.values(snapshot.modules || {})) {
    store.commit("modules/ADD_ACTIVE_MODULE", { module });
  }

  // Engine mode has no worker to register module definitions, so the engine
  // ships the registry (meta + prop descriptors) with the sync. The Gallery
  // reads `state.modules.registered`.
  if (snapshot.registered && typeof snapshot.registered === "object") {
    for (const module of Object.values(snapshot.registered)) {
      if (module && module.meta && module.meta.name) {
        store.commit("modules/ADD_REGISTERED_MODULE", { module });
      }
    }
  }

  const inputState = snapshot.inputs || {};
  for (const id of Object.keys(store.state.inputs.inputs || {})) {
    store.commit("inputs/REMOVE_INPUT", { inputId: id });
  }
  for (const input of Object.values(inputState.inputs || {})) {
    store.commit("inputs/ADD_INPUT", { input });
  }
  for (const id of Object.keys(store.state.inputs.inputLinks || {})) {
    store.commit("inputs/REMOVE_INPUT_LINK", { inputId: id });
  }
  for (const link of Object.values(inputState.inputLinks || {})) {
    store.commit("inputs/ADD_INPUT_LINK", { inputLink: link });
  }

  // Tweens (R-11): the engine ships the preset document verbatim, so any tween
  // it holds at boot (or after a snapshot restore) must reach the mirror —
  // otherwise tween-typed props read `undefined` and `dataTypes/tween` lookups
  // break. `ADD_TWEEN`'s payload is the descriptor itself.
  const mirrorTweens = store.state.tweens?.tweens || {};
  for (const id of Object.keys(mirrorTweens)) {
    store.commit("tweens/REMOVE_TWEEN", { id });
  }
  for (const tween of tweenEntries(snapshot.tweens)) {
    store.commit("tweens/ADD_TWEEN", tween);
  }

  // Expressions (R-11): `expressions/ADD_EXPRESSION` takes `{ assignment }`.
  const assignments = snapshot.expressions?.assignments || {};
  const mirrorAssignments = store.state.expressions?.assignments || {};
  for (const id of Object.keys(mirrorAssignments)) {
    store.commit("expressions/REMOVE_EXPRESSION", { id });
  }
  for (const assignment of Object.values(assignments)) {
    if (assignment && typeof assignment === "object") {
      store.commit("expressions/ADD_EXPRESSION", { assignment });
    }
  }

  if (Array.isArray(snapshot.scenes)) {
    store.commit("scenes/REPLACE_SCENES", snapshot.scenes);
  }
  if (snapshot.performance && typeof snapshot.performance === "object") {
    store.commit("scenes/UPDATE_PERFORMANCE", snapshot.performance);
  }
  store.commit("scenes/SET_DEFAULTS_SEEDED", !!snapshot.defaultsSeeded);
  if (typeof snapshot.defaultsVersion === "number") {
    store.commit("scenes/SET_DEFAULTS_VERSION", snapshot.defaultsVersion);
  }
  if (Array.isArray(snapshot.seededDefaultScenes)) {
    store.commit(
      "scenes/SET_SEEDED_DEFAULT_SCENES",
      snapshot.seededDefaultScenes,
    );
  }
  store.commit("scenes/UPDATE_PERFORMANCE", {
    currentSceneId: snapshot.appliedSceneId || null,
  });
}

export { tweenEntries };
