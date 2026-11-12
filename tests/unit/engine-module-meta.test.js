/**
 * Module `meta.enabled` normalisation for engine mode.
 *
 * Legacy modV renders a Module when `!module.meta.enabled` is false, i.e. on
 * JS truthiness. Saved Scene/preset documents therefore contain
 * `meta.enabled: 1` (the Enable Input Link stores a number) and legacy renders
 * them. The Rust compositor is stricter — it only draws a Module whose
 * `meta.enabled` is the JSON boolean `true`
 * (`crates/modv-app/src/compositor.rs:1410`, `input_links.rs:164`) — so a
 * restored Webcam (and every other Module) with `enabled: 1` silently draws
 * nothing in engine mode.
 *
 * `engineModuleEnabledPatches` is the pure half of the renderer's fix: it
 * reports which Modules need their `enabled` flag re-written as a boolean so
 * the engine renders the same set legacy did.
 */

import { describe, expect, it } from "vitest";

import {
  booleanModuleEnabled,
  engineModuleEnabledPatches,
} from "../../src/renderer/src/application/engine-module-meta.js";

const moduleWith = (meta) => ({
  $id: "module-1",
  $moduleName: "Webcam",
  meta,
  props: {},
});

describe("booleanModuleEnabled", () => {
  it("leaves real booleans alone", () => {
    expect(booleanModuleEnabled(moduleWith({ enabled: true }))).toBeNull();
    expect(booleanModuleEnabled(moduleWith({ enabled: false }))).toBeNull();
  });

  it("leaves a missing flag alone (legacy treats it as disabled)", () => {
    expect(booleanModuleEnabled(moduleWith({}))).toBeNull();
    expect(booleanModuleEnabled(moduleWith(undefined))).toBeNull();
    expect(booleanModuleEnabled(undefined)).toBeNull();
  });

  it("treats numeric flags the way legacy JS truthiness did", () => {
    expect(booleanModuleEnabled(moduleWith({ enabled: 1 }))).toBe(true);
    expect(booleanModuleEnabled(moduleWith({ enabled: 0 }))).toBe(false);
    expect(booleanModuleEnabled(moduleWith({ enabled: 2 }))).toBe(true);
  });

  it("coerces string flags with JS truthiness", () => {
    expect(booleanModuleEnabled(moduleWith({ enabled: "1" }))).toBe(true);
    expect(booleanModuleEnabled(moduleWith({ enabled: "" }))).toBe(false);
    // `!"0" === false`, so legacy rendered it; keep that behaviour.
    expect(booleanModuleEnabled(moduleWith({ enabled: "0" }))).toBe(true);
  });
});

describe("engineModuleEnabledPatches", () => {
  it("returns a patch per legacy-encoded module, keyed by module id", () => {
    const modules = {
      "module-1": { $id: "module-1", meta: { name: "Webcam", enabled: 1 } },
      "module-2": { $id: "module-2", meta: { name: "Edge Blur", enabled: 0 } },
      "module-3": { $id: "module-3", meta: { name: "Circle", enabled: true } },
    };

    expect(engineModuleEnabledPatches(modules)).toEqual([
      { id: "module-1", enabled: true },
      { id: "module-2", enabled: false },
    ]);
  });

  it("accepts an array of modules and the engine's `{ module }` payload", () => {
    const module = { $id: "module-1", meta: { enabled: 1 } };

    expect(engineModuleEnabledPatches([module])).toEqual([
      { id: "module-1", enabled: true },
    ]);
    expect(engineModuleEnabledPatches({ module })).toEqual([
      { id: "module-1", enabled: true },
    ]);
  });

  it("accepts one Module object as well as a map of Modules", () => {
    expect(
      engineModuleEnabledPatches({ $id: "module-1", meta: { enabled: 1 } }),
    ).toEqual([{ id: "module-1", enabled: true }]);
    expect(
      engineModuleEnabledPatches({
        $id: "module-1",
        $moduleName: "Webcam",
        meta: { name: "Webcam", enabled: 0 },
      }),
    ).toEqual([{ id: "module-1", enabled: false }]);
  });

  it("falls back to the map key and the `id` field for the module id", () => {
    expect(
      engineModuleEnabledPatches({
        "map-id": { meta: { enabled: 1 } },
        other: { id: "field-id", meta: { enabled: 1 } },
      }),
    ).toEqual([
      { id: "map-id", enabled: true },
      { id: "field-id", enabled: true },
    ]);
  });

  it("skips modules it cannot identify", () => {
    expect(engineModuleEnabledPatches([{ meta: { enabled: 1 } }])).toEqual([]);
    expect(engineModuleEnabledPatches(null)).toEqual([]);
    expect(engineModuleEnabledPatches(undefined)).toEqual([]);
    expect(engineModuleEnabledPatches("nope")).toEqual([]);
  });
});
