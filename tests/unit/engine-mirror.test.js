/**
 * M2 parity gate (WS-1, `docs/rust-hybrid.md`).
 *
 * `modv-engine --emit-fixtures` runs a deterministic store session in Rust and
 * writes the two fixtures this test replays:
 *
 * - `tests/fixtures/engine/deltas.json` — the `{ type, payload }` mutation
 *   stream the engine broadcasts for the UI mirror
 *   (`worker/index.worker.js:136-144`).
 * - `tests/fixtures/engine/snapshot.json` — the engine's `Store::snapshot()`
 *   for the resulting state.
 *
 * The deltas are replayed into the **real** worker store singleton (the same
 * module the renderer's mirror store is created from) and the outcome is
 * compared with the snapshot. That closes the gate: a Rust session and the
 * JavaScript mirror store agree on the state the records produce, so the
 * engine can be trusted to drive the existing Vuex UI.
 *
 * Group and module ids in the fixtures are stable placeholders (`group-1`,
 * `module-1`, …) — the Rust emitter maps its generated UUIDs before writing
 * (see `crates/modv-app/src/fixtures.rs`), so nothing here depends on a
 * particular run.
 */

import { readFileSync } from "node:fs";
import { beforeAll, describe, expect, test } from "vitest";

import constants from "../../src/renderer/src/application/constants.js";

/**
 * The worker store touches the worker global at module scope: `tweens.js`
 * shims anime.js DOM globals onto `self`, and `store/index.js` skips
 * `vuex-persistedstate` when `self.document` is undefined (which is exactly
 * what the renderer's mirror store relies on in the worker). Node has no
 * `self`, so recreate the worker global *before* importing the store.
 * `globalThis.document` stays undefined, so the persisted-state plugin is not
 * installed — the store is a plain Vuex store, the way the mirror uses it.
 */
globalThis.self = globalThis;

const { default: store } =
  await import("../../src/renderer/src/application/worker/store/index.js");

const fixtureUrl = (name) =>
  new URL(`../fixtures/engine/${name}`, import.meta.url);

const readFixture = (name) =>
  JSON.parse(readFileSync(fixtureUrl(name), "utf8"));

const deltas = readFixture("deltas.json");
const snapshot = readFixture("snapshot.json");

/**
 * `Store::snapshot()` is the six-key preset document plus `version` and
 * `renderOrder`, so the preset keys sit at the top level. Tolerate a wrapped
 * `{ preset: {...} }` document too, in case the emitter grows a sidecar.
 */
const preset = snapshot.preset ?? snapshot;

/**
 * The preset projection of the mirror store's Groups.
 *
 * Mirrors the canonical serialiser (`groups.js` `createPresetData`,
 * `groups.js:256-270`): drop the internal Gallery Group and transition ghosts,
 * then drop the runtime-only `context` record and the `writeToSwap` /
 * `readFromSwap` action arguments. `drawToCanvasId` is dropped as well because
 * the engine's `Group::for_preset_save` drops it; this session creates no
 * auxiliary outputs, so the delete is defensive rather than load-bearing.
 */
function presetGroups(state) {
  return state.groups
    .filter(
      (group) =>
        group.name !== constants.GALLERY_GROUP_NAME &&
        group.transitionGhost !== true,
    )
    .map((group) => {
      const clone = { ...group };
      delete clone.context;
      delete clone.writeToSwap;
      delete clone.readFromSwap;
      delete clone.drawToCanvasId;
      return clone;
    });
}

/**
 * The preset projection of the mirror store's active modules
 * (`modules.js:660-683`): drop gallery modules and the live `$status` array.
 *
 * No other key is normalised: the engine's `ADD_ACTIVE_MODULE` payload already
 * carries exactly the JS module shape (`$id`, `$moduleName`, `meta`, `props`,
 * `data`, `$props`, `$status`), so `data`/`$props` are real mirror fields, not
 * engine-only additions.
 */
function presetModules(active) {
  return Object.values(active)
    .filter((module) => !module.meta.isGallery)
    .reduce((modules, module) => {
      const clone = { ...module };
      delete clone.$status;
      modules[clone.$id] = clone;
      return modules;
    }, {});
}

/** Replay the recorded mutation stream into the mirror store. */
function replay() {
  for (const delta of deltas) {
    store.commit(delta.type, delta.payload);
  }
}

describe("engine → JS mirror parity", () => {
  // One replay for the whole file: the deltas are replayed exactly as the
  // renderer does once, and every assertion below inspects that one outcome.
  // The initial emptiness check makes a partially-populated singleton (or a
  // missing record) a loud failure instead of a silently passing comparison.
  beforeAll(() => {
    expect(store.state.groups.groups).toEqual([]);
    expect(store.state.modules.active).toEqual({});
    replay();
  });

  test("the fixtures are the emitted shape", () => {
    expect(Array.isArray(deltas)).toBe(true);
    expect(deltas.length).toBe(19);
    for (const delta of deltas) {
      expect(Object.keys(delta).sort()).toEqual(["payload", "type"]);
      expect(typeof delta.type).toBe("string");
      expect(delta.payload).toBeTypeOf("object");
    }
    // Six-key preset plus the engine's `version`/`renderOrder` bookkeeping.
    expect(Object.keys(preset).sort()).toEqual([
      "expressions",
      "groups",
      "inputs",
      "midi",
      "modules",
      "renderOrder",
      "tweens",
      "version",
    ]);
    expect(preset.renderOrder).toEqual([0, 2, 1]);
  });

  test("every emitted mutation is implemented by the mirror store", () => {
    // Vuex stores namespaced mutations as `namespace/TYPE`.
    const unknown = [...new Set(deltas.map((delta) => delta.type))].filter(
      (type) => !store._mutations[type],
    );
    expect(unknown).toEqual([]);
  });

  test("replaying the deltas reproduces the snapshot groups", () => {
    // The mirror also holds the Gallery Group: the engine keeps it live and
    // filters it on save, exactly like `createPresetData`.
    const mirrorGroups = store.state.groups.groups;
    expect(mirrorGroups.map((group) => group.name)).toEqual([
      "Background",
      constants.GALLERY_GROUP_NAME,
      "Effects",
    ]);

    expect(presetGroups(store.state.groups)).toEqual(preset.groups);
  });

  test("replaying the deltas reproduces the snapshot modules", () => {
    // The unregistered module record was replayed and then removed again.
    expect(Object.keys(store.state.modules.active)).toEqual([
      "module-1",
      "module-2",
      "module-3",
    ]);
    expect(presetModules(store.state.modules.active)).toEqual(preset.modules);

    // Coercion ran in the engine: `Ball.amount` is `strict` with range 1..=300,
    // so the recorded 500 arrives as 300 in both the delta and the state.
    expect(store.state.modules.active["module-3"].props.amount).toBe(300);
  });

  test("inputs and expressions stay empty in both stores", () => {
    // The scripted session never creates inputs/expressions, so both sides must
    // hold the same empty collections.
    expect(preset.inputs.inputs).toEqual({});
    expect(preset.inputs.inputLinks).toEqual({});
    expect(store.state.inputs.inputs).toEqual(preset.inputs.inputs);
    expect(store.state.inputs.inputLinks).toEqual(preset.inputs.inputLinks);

    expect(preset.expressions.assignments).toEqual({});
    expect(store.state.expressions.assignments).toEqual(
      preset.expressions.assignments,
    );
  });

  test("tweens and midi stay at their empty defaults", () => {
    // The canonical serializer emits tweens as an array built from the store's
    // tween record, so compare the emptied array rather than the live shape.
    expect(preset.tweens).toEqual([]);
    expect(Object.values(store.state.tweens.tweens)).toEqual([]);

    // Documented M2 default-shape deviation: `EngineState::new` starts `midi`
    // empty (`{}`) while the JS midi module starts
    // `{ devices: {}, learning: false }`. Nothing in the session touches MIDI,
    // so the meaningful assertion is that neither side has a device and
    // learning is off — `preset-serializer` treats absent keys as defaults.
    expect(preset.midi).toEqual({});
    expect(Object.keys(store.state.midi.devices)).toEqual([]);
    expect(store.state.midi.learning).toBe(false);

    // Same kind of documented deviation for the focused input: the engine keeps
    // `null`, the JS inputs module starts `{ id: null, title: null }`.
    expect(preset.inputs.focusedInput).toBeNull();
    expect(store.state.inputs.focusedInput.id).toBeNull();
  });
});
