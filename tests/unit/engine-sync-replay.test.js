/**
 * Late-joiner `state/sync` replay into the renderer mirror (review R-4/R-11).
 *
 * Engine mode replays the engine's sync payload into the real worker store (the
 * renderer's mirror). Two paths can deliver it — the `state/sync` event and the
 * `engine/get-sync` pull — so the replay must be idempotent, and it must carry
 * the preset document's `tweens` and `expressions` (not just
 * groups/modules/inputs), which the earlier replay dropped.
 */

import { describe, expect, test } from "vitest";

// The worker store touches the worker global at module scope (`tweens.js`
// shims anime.js DOM globals onto `self`); recreate it before the import.
globalThis.self = globalThis;

const { default: store } =
  await import("../../src/renderer/src/application/worker/store/index.js");
const { applyEngineSync, tweenEntries } =
  await import("../../src/renderer/src/application/engine-sync-replay.js");

/** A minimal engine `state/sync` payload (shape from `Store::sync_state`). */
function syncPayload() {
  return {
    version: 1,
    groups: [
      {
        id: "group-1",
        name: "Effects",
        modules: ["module-1"],
        enabled: 1,
        alpha: 1,
        inherit: false,
        inheritFrom: -1,
        pipeline: false,
        clearing: true,
        compositeOperation: "normal",
      },
    ],
    modules: {
      "module-1": {
        $id: "module-1",
        $moduleName: "Ball",
        meta: { name: "Ball", type: "2d", enabled: true, alpha: 1 },
        props: {},
        data: {},
      },
    },
    inputs: {
      focusedInput: null,
      inputs: {
        "input-1": {
          id: "input-1",
          type: "action",
          location: "modules/updateProp",
          data: { moduleId: "module-1", prop: "size" },
        },
      },
      inputLinks: {
        "input-1": {
          id: "input-1",
          source: "tween",
          location: "tweens.tweens['tween-1'].value[0]",
          min: 0,
          max: 1,
        },
      },
    },
    expressions: {
      assignments: {
        "expression-1": {
          id: "expression-1",
          inputId: "input-1",
          expression: "inputValue * 2",
          evaluateEveryFrame: true,
        },
      },
    },
    tweens: [
      {
        id: "tween-1",
        data: [[0], [1]],
        duration: 1000,
        direction: "alternate",
        easing: "linear",
        loop: true,
        useBpm: true,
        bpmDivision: 16,
        durationAsTotalTime: false,
        steps: 0,
      },
    ],
    scenes: [{ id: "scene-1", name: "Scene", groups: [], modules: {} }],
    performance: {
      enabled: false,
      currentSceneId: "scene-1",
      allowedTransitions: ["Fade"],
      rowNames: [],
    },
    defaultsSeeded: true,
    defaultsVersion: 12,
    seededDefaultScenes: ["Scene"],
    appliedSceneId: "scene-1",
    registered: {
      Ball: { meta: { name: "Ball", type: "2d" }, props: {} },
    },
  };
}

/** A comparable projection of the mirror state the replay feeds. */
function mirrorState() {
  const state = store.state;
  return {
    groups: state.groups.groups.map((group) => group.id),
    modules: Object.keys(state.modules.active),
    registered: Object.keys(state.modules.registered),
    inputs: Object.keys(state.inputs.inputs),
    inputLinks: Object.keys(state.inputs.inputLinks),
    tweens: Object.keys(state.tweens.tweens).sort(),
    expressions: Object.keys(state.expressions.assignments).sort(),
    scenes: state.scenes.scenes.map((scene) => scene.id),
    currentSceneId: state.scenes.performance.currentSceneId,
  };
}

describe("tweenEntries", () => {
  test("accepts the preset array, the live map and the worker root", () => {
    const tween = { id: "t", data: [[0], [1]] };
    expect(tweenEntries([tween])).toEqual([tween]);
    expect(tweenEntries({ t: tween })).toEqual([tween]);
    expect(tweenEntries({ tweens: [tween] })).toEqual([tween]);
    expect(tweenEntries(null)).toEqual([]);
    expect(tweenEntries("nope")).toEqual([]);
  });
});

describe("applyEngineSync", () => {
  test("replays tweens and expressions the delta stream never carried", () => {
    applyEngineSync(store, syncPayload());

    expect(store.state.tweens.tweens["tween-1"]).toMatchObject({
      id: "tween-1",
      duration: 1000,
    });
    expect(store.state.expressions.assignments["expression-1"]).toMatchObject({
      id: "expression-1",
      inputId: "input-1",
      expression: "inputValue * 2",
    });
    expect(store.state.modules.registered.Ball).toBeTruthy();
  });

  test("is idempotent: replaying the same payload twice leaves one of each", () => {
    applyEngineSync(store, syncPayload());
    const once = mirrorState();
    applyEngineSync(store, syncPayload());

    expect(mirrorState()).toEqual(once);
    expect(once).toMatchObject({
      groups: ["group-1"],
      modules: ["module-1"],
      registered: ["Ball"],
      inputs: ["input-1"],
      inputLinks: ["input-1"],
      tweens: ["tween-1"],
      expressions: ["expression-1"],
      scenes: ["scene-1"],
      currentSceneId: "scene-1",
    });
  });

  test("replaces stale collections instead of merging them", () => {
    applyEngineSync(store, syncPayload());

    const next = syncPayload();
    next.groups = [];
    next.modules = {};
    next.inputs = { inputs: {}, inputLinks: {} };
    next.tweens = [];
    next.expressions = { assignments: {} };
    next.scenes = [];
    next.appliedSceneId = null;
    applyEngineSync(store, next);

    expect(mirrorState()).toMatchObject({
      groups: [],
      modules: [],
      inputs: [],
      inputLinks: [],
      tweens: [],
      expressions: [],
      scenes: [],
      currentSceneId: null,
    });

    // Registering is append-only for an engine session.
    expect(store.state.modules.registered.Ball).toBeTruthy();
  });

  test("ignores a non-object payload", () => {
    const before = mirrorState();
    applyEngineSync(store, null);
    applyEngineSync(store, "nope");
    expect(mirrorState()).toEqual(before);
  });
});
