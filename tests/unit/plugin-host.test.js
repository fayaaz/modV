/**
 * Engine-mode plugin host (MODV_ENGINE=1).
 *
 * Engine mode has no worker, so `worker/index.worker.js` never registers the
 * built-in plugins and `worker/loop.js` never calls the frame hooks. The
 * renderer host in `application/plugin-host.js` takes over: it adds plugins to
 * the mirror store, gives their hooks a facade (mirror reads, engine writes),
 * runs `init`/`shutdown` and the per-frame hooks, and applies plugin-local
 * mutations to the mirror (the engine does not know them).
 *
 * These tests drive the real mirror store singleton (the same module the
 * renderer uses) so the facade and the plugin store module registration follow
 * the production path. No Electron/WebGL is involved.
 */

import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

// `worker/store` touches the worker global at module scope (`tweens.js` shims
// anime.js DOM globals onto `self`); recreate it before the import.
globalThis.self = globalThis;

const { default: store } =
  await import("../../src/renderer/src/application/worker/store/index.js");
const {
  PluginHost,
  createEngineStoreFacade,
  createPluginHost,
  getActivePluginHost,
  setActivePluginHost,
} = await import("../../src/renderer/src/application/plugin-host.js");

/** Engine `commit`/`dispatch` spy shaped like `engineClient.storeFacade`. */
function createEngineStub() {
  const calls = { dispatch: [], commit: [] };

  return {
    state: store.state,
    getters: store.getters,
    calls,
    dispatch(type, payload) {
      calls.dispatch.push({ type, payload });
      return Promise.resolve({ type, payload });
    },
    commit(type, payload) {
      calls.commit.push({ type, payload });
    },
  };
}

/** Logger stub with the three levels the host uses. */
function createLoggerStub() {
  return { info: vi.fn(), warn: vi.fn(), error: vi.fn() };
}

describe("engine-mode plugin host", () => {
  let host;
  let engine;
  let logger;
  let registeredModules;
  let registeredModuleNames;

  beforeEach(() => {
    store.state.plugins.splice(0);
    engine = createEngineStub();
    logger = createLoggerStub();
    registeredModules = [];
    registeredModuleNames = [];
    host = new PluginHost({
      mirrorStore: store,
      engineStore: engine,
      logger,
      getFeatures: () => ({ bass: 0.5, mid: 0.2, treble: 0.1 }),
    });
    setActivePluginHost(null);
  });

  afterEach(() => {
    for (const name of registeredModules) {
      try {
        store.unregisterModule(name);
      } catch (error) {
        // The module may not have been registered (duplicate test name).
      }
    }

    for (const name of registeredModuleNames) {
      store.commit("modules/REMOVE_REGISTERED_MODULE", { moduleName: name });
    }

    store.state.plugins.splice(0);
    setActivePluginHost(null);
  });

  function trackPluginModule(name) {
    registeredModules.push(name);
    return name;
  }

  test("add registers the plugin store module on the mirror and resolves $props", async () => {
    const storeName = trackPluginModule("pluginHostDemoStore");

    const plugin = {
      name: "Plugin Host Demo",
      storeName,
      store: {
        namespaced: true,
        state: { count: 0 },
        mutations: {
          INC(state) {
            state.count += 1;
          },
        },
      },
      props: {
        // Primitive defaults keep the worker's `getPropDefault` semantics.
        amount: { type: "int", default: 7 },
        // `vec2` comes from the mirror's registered dataTypes module, so the
        // default is only available once the engine's state map exists.
        point: { type: "vec2" },
        mode: {
          type: "enum",
          enum: [{ value: "a" }, { value: "b", selected: true }],
        },
      },
    };

    const added = await host.add(plugin);

    expect(added.id).toBeTypeOf("string");
    expect(added.enabled).toBe(false);
    expect(added.$props).toEqual({
      amount: 7,
      point: { 0: 0, 1: 0 },
      mode: "b",
    });
    expect(store.state[storeName].count).toBe(0);
    expect(store.state.plugins.some((item) => item.id === added.id)).toBe(true);
  });

  test("setEnabled runs init/shutdown with the facade and mirrors enabled", async () => {
    const init = vi.fn();
    const shutdown = vi.fn();
    const plugin = { name: "Lifecycle Plugin", init, shutdown };

    await host.add(plugin);

    expect(plugin.enabled).toBe(false);

    await expect(
      host.setEnabled({ pluginId: plugin.id, enabled: true }),
    ).resolves.toBe(true);

    expect(init).toHaveBeenCalledTimes(1);
    expect(init.mock.calls[0][0].store).toBe(host.store);
    expect(init.mock.calls[0][0].props).toEqual(plugin.$props);
    expect(
      store.state.plugins.find((item) => item.id === plugin.id).enabled,
    ).toBe(true);

    await host.setEnabled({ pluginId: plugin.id, enabled: false });

    expect(shutdown).toHaveBeenCalledTimes(1);
    expect(shutdown.mock.calls[0][0].store).toBe(host.store);
    expect(
      store.state.plugins.find((item) => item.id === plugin.id).enabled,
    ).toBe(false);
  });

  test("frame hooks read the mirror, forward engine dispatches and respect enabled", async () => {
    const order = [];
    const enabledPlugin = {
      name: "Hook Plugin",
      preProcessFrame({ features, store: facade, props }) {
        order.push("pre");
        facade.dispatch("modules/updateProp", {
          moduleId: "module-1",
          prop: "amount",
          data: features.bass,
        });
        expect(props).toEqual(enabledPlugin.$props);
      },
      postProcessFrame({ features }) {
        order.push("post");
        expect(features.bass).toBe(0.42);
      },
    };
    const disabledPlugin = {
      name: "Disabled Plugin",
      preProcessFrame: vi.fn(),
      postProcessFrame: vi.fn(),
    };

    await host.add(enabledPlugin);
    await host.add(disabledPlugin);
    await host.setEnabled({ pluginId: enabledPlugin.id, enabled: true });

    const features = { bass: 0.42, mid: 0.1, treble: 0.05 };
    host.runPreProcessFrame(features);
    host.runPostProcessFrame(features);

    expect(order).toEqual(["pre", "post"]);
    expect(engine.calls.dispatch).toEqual([
      {
        type: "modules/updateProp",
        payload: { moduleId: "module-1", prop: "amount", data: 0.42 },
      },
    ]);
    expect(disabledPlugin.preProcessFrame).not.toHaveBeenCalled();
    expect(disabledPlugin.postProcessFrame).not.toHaveBeenCalled();
  });

  test("the facade exposes the mirror's registered module map", async () => {
    const moduleName = "PluginHostWitness";
    registeredModuleNames.push(moduleName);

    store.commit("modules/ADD_REGISTERED_MODULE", {
      module: { meta: { name: moduleName, type: "2d" }, props: {} },
    });

    let observed;
    const plugin = {
      name: "Registered Map Plugin",
      preProcessFrame({ store: facade }) {
        observed = facade.state.modules.registered[moduleName]?.meta?.name;
      },
    };

    await host.add(plugin);
    await host.setEnabled({ pluginId: plugin.id, enabled: true });
    host.runPreProcessFrame({});

    expect(observed).toBe(moduleName);
  });

  test("a plugin mutation the engine does not know is mirrored and warned once", async () => {
    const storeName = trackPluginModule("pluginHostMutationStore");

    const plugin = {
      name: "Mirror Mutation Plugin",
      storeName,
      store: {
        namespaced: true,
        state: { count: 0 },
        mutations: {
          INC(state) {
            state.count += 1;
          },
        },
      },
      preProcessFrame({ store: facade }) {
        facade.commit(`${storeName}/INC`);
        facade.commit(`${storeName}/INC`);
      },
    };

    await host.add(plugin);
    await host.setEnabled({ pluginId: plugin.id, enabled: true });
    host.runPreProcessFrame({});

    expect(store.state[storeName].count).toBe(2);
    // Nothing about the plugin module reached the engine.
    expect(engine.calls.commit).toEqual([]);

    const mirrorOnlyWarnings = logger.warn.mock.calls.filter(([message]) =>
      String(message).includes("mirror only"),
    );
    expect(mirrorOnlyWarnings).toHaveLength(1);
    expect(mirrorOnlyWarnings[0][1]).toEqual({ type: `${storeName}/INC` });
  });

  test("core commits and dispatches still forward to the engine", async () => {
    host.store.commit("beats/SET_BPM", { bpm: 128 });
    await host.store.dispatch("groups/createGroup", { enabled: 1 });

    expect(engine.calls.commit).toEqual([
      { type: "beats/SET_BPM", payload: { bpm: 128 } },
    ]);
    expect(engine.calls.dispatch).toEqual([
      { type: "groups/createGroup", payload: { enabled: 1 } },
    ]);
  });

  test("a throwing frame hook is logged once and cannot stop the other hooks", async () => {
    const good = { name: "Good Plugin", preProcessFrame: vi.fn() };
    const bad = {
      name: "Throwing Plugin",
      preProcessFrame() {
        throw new Error("boom");
      },
    };

    await host.add(good);
    await host.add(bad);
    await host.setEnabled({ pluginId: good.id, enabled: true });
    await host.setEnabled({ pluginId: bad.id, enabled: true });

    host.runPreProcessFrame({});
    host.runPreProcessFrame({});

    expect(good.preProcessFrame).toHaveBeenCalledTimes(2);
    const hookWarnings = logger.warn.mock.calls.filter(([message]) =>
      String(message).includes("frame hook failed"),
    );
    expect(hookWarnings).toHaveLength(1);
  });

  test("plugins marked unsupported are registered but refuse to enable", async () => {
    const init = vi.fn();
    const plugin = {
      name: "Unsupported Plugin",
      supportsEngine: false,
      engineModeReason: "no output canvas",
      init,
    };

    await host.add(plugin);

    expect(logger.warn).toHaveBeenCalledWith(
      expect.stringContaining("not available in engine mode"),
      expect.objectContaining({
        plugin: "Unsupported Plugin",
        reason: "no output canvas",
      }),
    );

    await expect(
      host.setEnabled({ pluginId: plugin.id, enabled: true }),
    ).resolves.toBe(false);

    expect(init).not.toHaveBeenCalled();
    expect(plugin.enabled).toBe(false);
    // Only the add/install warning fired; enabling does not repeat it.
    const unsupportedWarnings = logger.warn.mock.calls.filter(([message]) =>
      String(message).includes("not available in engine mode"),
    );
    expect(unsupportedWarnings).toHaveLength(1);
  });

  test("the engine store facade routes plugin actions to the host", async () => {
    const facade = createEngineStoreFacade(engine, host);
    const plugin = { name: "Routed Plugin", init: vi.fn() };

    await facade.dispatch("plugins/add", plugin);
    await facade.dispatch("plugins/setEnabled", {
      pluginId: plugin.id,
      enabled: true,
    });

    expect(plugin.init).toHaveBeenCalledTimes(1);
    expect(store.state.plugins.some((item) => item.id === plugin.id)).toBe(
      true,
    );
    // None of these actions were handed to the engine's catch-all.
    expect(engine.calls.dispatch).toEqual([]);

    facade.dispatch("groups/createGroup", { enabled: 1 });
    expect(engine.calls.dispatch).toEqual([
      { type: "groups/createGroup", payload: { enabled: 1 } },
    ]);

    // Plugin-owned mutations commit to the mirror, never to the engine.
    facade.commit("plugins/SET_PLUGIN_ENABLE", {
      pluginId: plugin.id,
      enabled: false,
    });
    expect(store.state.plugins[0].enabled).toBe(false);
    expect(engine.calls.commit).toEqual([]);
  });

  test("legacy mode constructs no host and keeps the worker path", () => {
    setActivePluginHost(null);

    expect(
      createPluginHost({
        engineMode: false,
        mirrorStore: store,
        engineStore: engine,
      }),
    ).toBeNull();
    expect(getActivePluginHost()).toBeNull();

    const engineHost = createPluginHost({
      engineMode: true,
      mirrorStore: store,
      engineStore: engine,
    });

    expect(engineHost).toBeInstanceOf(PluginHost);

    setActivePluginHost(engineHost);
    expect(getActivePluginHost()).toBe(engineHost);
  });
});
