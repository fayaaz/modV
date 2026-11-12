// Engine-mode plugin host (MODV_ENGINE=1).
//
// In legacy mode the plugins live in the worker: `worker/index.worker.js`
// registers the built-in plugin and `worker/loop.js` calls `preProcessFrame`
// around the composition before `postProcessFrame` after it. Engine mode has
// no worker and no renderer-side output canvas, so the renderer owns the
// plugin lifecycle instead:
//
// - `installPlugin`/`modV.use` route through `PluginHost` (see
//   `install-plugin.js`): the plugin is added to the mirror store and its
//   `store` module is registered there.
// - `plugins/add`, `plugins/setEnabled` and `plugins/updateProp` are
//   intercepted on the engine store facade (`createEngineStoreFacade`), so the
//   Plugins panel drives the host instead of dispatching actions the engine
//   silently discards. `init`/`shutdown` receive the plugin store facade and
//   `enabled` is kept in the mirror.
// - `ModV.tick` runs `preProcessFrame`/`postProcessFrame` once per renderer
//   animation frame. The engine composites in its own process and never hands
//   the renderer a frame to interleave with, so the hooks run just before and
//   just after the renderer's audio/feature publish (`ModV.loop`). They are
//   therefore one frame behind the engine's compositor; the ordering between
//   pre and post hooks is preserved, and both receive the feature payload the
//   renderer has at that moment.
//
// Store facade semantics/limits:
//
// - reads (`state`, `getters`) come from the mirror store;
// - `dispatch`/`commit` for core mutations go to the engine, and the engine's
//   `state/delta` stream keeps the mirror current;
// - `registerModule` registers on the mirror only. Mutations/actions in a
//   namespace a plugin registered are unknown to the engine, so they are
//   applied to the mirror directly and logged once per type instead of being
//   forwarded (or throwing out of the frame hook);
// - `postProcessFrame` is called without a `canvas`: the engine owns the
//   output surface. A plugin that needs composited pixels must be adapted to
//   the engine's preview ring; `grab-canvas` is marked unsupported for that
//   reason (`plugins/grab-canvas.js`, `supportsEngine: false`).

import { v4 as uuidv4 } from "uuid";

import { createLogger } from "./logger";
import getPropDefault from "./utils/get-prop-default";

const logger = createLogger("renderer:plugin-host");

function camelize(str) {
  return str.replace(/(?:^\w|[A-Z]|\b\w|\s+)/g, (match, index) => {
    if (+match === 0) {
      return "";
    }

    return index === 0 ? match.toLowerCase() : match.toUpperCase();
  });
}

function mutationTypesFor(name, module) {
  const namespaced = module?.namespaced === true;
  const prefix = namespaced ? `${name}/` : "";

  return {
    mutations: new Set(
      Object.keys(module?.mutations || {}).map((type) => `${prefix}${type}`),
    ),
    actions: new Set(
      Object.keys(module?.actions || {}).map((type) => `${prefix}${type}`),
    ),
  };
}

export class PluginHost {
  /**
   * @param {object} options
   * @param {import("vuex").Store} options.mirrorStore renderer mirror store
   * @param {object} [options.engineStore] engine store facade (commit/dispatch)
   * @param {() => unknown} [options.getFeatures] latest audio feature payload
   * @param {object} [options.logger] logger with info/warn/error
   * @param {Function} [options.resolvePropDefault] injectable for tests
   */
  constructor({
    mirrorStore,
    engineStore = null,
    getFeatures = () => undefined,
    logger: log = logger,
    resolvePropDefault = getPropDefault,
  } = {}) {
    if (!mirrorStore) {
      throw new Error("PluginHost requires the mirror store");
    }

    this.mirrorStore = mirrorStore;
    this.engineStore = engineStore;
    this.logger = log;
    this._getFeatures = getFeatures;
    this._resolvePropDefault = resolvePropDefault;
    // Namespaces/action types registered by plugins: the engine does not know
    // them, so they are mirrored locally.
    this._pluginMutations = new Set();
    this._pluginActions = new Set();
    this._warnedMutations = new Set();
    this._warnedFailures = new Set();
    this._warnedUnsupported = new Set();
    this.store = this._createStoreFacade();
  }

  /** Plugins currently held in the mirror (the same array the UI reads). */
  get plugins() {
    const plugins = this.mirrorStore.state.plugins;
    return Array.isArray(plugins) ? plugins : [];
  }

  features() {
    return this._getFeatures();
  }

  /**
   * Add a plugin the way the worker's `plugins/add` action does: register its
   * store module on the mirror, resolve `$props`, then commit it disabled.
   *
   * @returns {Promise<object>} the plugin record (with `id`/`$props`/`enabled`)
   */
  async add(plugin) {
    if (!plugin || typeof plugin !== "object" || !("name" in plugin)) {
      throw new Error("Plugin requires a name");
    }

    if (plugin.id) {
      const existing = this.plugins.find((item) => item.id === plugin.id);
      if (existing) {
        return existing;
      }
    }

    if ("store" in plugin && plugin.store) {
      const storeName = plugin.storeName || camelize(plugin.name);
      this.registerModule(storeName, plugin.store);
    }

    plugin.$props = {};

    if ("props" in plugin && plugin.props) {
      const keys = Object.keys(plugin.props);

      for (let i = 0; i < keys.length; i += 1) {
        const key = keys[i];
        const value = await this._resolvePropDefault(
          plugin,
          key,
          plugin.props[key],
          false,
        );

        plugin.$props[key] = value || null;
      }
    }

    plugin.id = plugin.id || uuidv4();
    plugin.enabled = false;
    this.mirrorStore.commit("plugins/ADD_PLUGIN", plugin);

    if (plugin.supportsEngine === false) {
      this._warnUnsupported(plugin);
    }

    return plugin;
  }

  /**
   * Register a plugin's Vuex module on the mirror store. The engine never sees
   * it, so its mutations/actions are applied locally (see the class comment).
   *
   * @returns {boolean} false when the namespace is already registered
   */
  registerModule(name, module) {
    if (!name || !module) {
      return false;
    }

    if (Object.prototype.hasOwnProperty.call(this.mirrorStore.state, name)) {
      this.logger.warn(
        "plugin store module is already registered; keeping the existing one",
        { module: name },
      );
      return false;
    }

    try {
      this.mirrorStore.registerModule(name, module);
    } catch (error) {
      this.logger.warn("could not register the plugin store module", {
        module: name,
        message: error?.message || String(error),
      });
      return false;
    }

    const types = mutationTypesFor(name, module);
    for (const type of types.mutations) this._pluginMutations.add(type);
    for (const type of types.actions) this._pluginActions.add(type);

    return true;
  }

  /**
   * Enable/disable a plugin, running `init`/`shutdown` with the store facade
   * and the resolved `$props`, then reflecting `enabled` in the mirror.
   */
  async setEnabled({ pluginId, enabled } = {}) {
    const plugin = this.plugins.find((item) => item.id === pluginId);

    if (!plugin) {
      return false;
    }

    if (enabled && plugin.supportsEngine === false) {
      this._warnUnsupported(plugin);
      return false;
    }

    const initArgs = { store: this.store, props: plugin.$props };

    if (enabled) {
      if (typeof plugin.init === "function") {
        await plugin.init(initArgs);
      }
    } else if (typeof plugin.shutdown === "function") {
      await plugin.shutdown(initArgs);
    }

    this.mirrorStore.commit("plugins/SET_PLUGIN_ENABLE", { pluginId, enabled });

    return true;
  }

  /** Update a plugin prop with the same coercion as the worker action. */
  async updateProp({ pluginId, prop, data } = {}) {
    const plugin = this.plugins.find((item) => item.id === pluginId);

    if (!plugin) {
      return false;
    }

    const propData = plugin.props?.[prop];

    if (!propData) {
      return false;
    }

    if (data === plugin.$props[prop]) {
      return undefined;
    }

    let dataOut = data;
    const dataType = this.mirrorStore.state.dataTypes?.[propData.type];

    if (dataType && typeof dataType.create === "function") {
      dataOut = await dataType.create(dataOut);
    }

    if (!Array.isArray(dataOut)) {
      const { strict, min, max, abs } = propData;

      if (strict && typeof min !== "undefined" && typeof max !== "undefined") {
        dataOut = Math.min(Math.max(dataOut, min), max);
      }

      if (abs) {
        dataOut = Math.abs(dataOut);
      }

      if (propData.type === "int") {
        dataOut = Math.round(dataOut);
      }
    }

    this.mirrorStore.commit("plugins/UPDATE_PROP", {
      pluginId,
      prop,
      data: dataOut,
    });

    return dataOut;
  }

  /**
   * Run every enabled `preProcessFrame` hook once, in registration order.
   * A throwing hook is logged once and skipped so it cannot stop the renderer's
   * animation loop (the legacy worker would lose the rest of that frame too,
   * but the worker's loop keeps its own cadence).
   */
  runPreProcessFrame(features = this.features()) {
    this._runHook("preProcessFrame", features);
  }

  /**
   * Run every enabled `postProcessFrame` hook once, in registration order.
   * No `canvas` is passed: the engine owns the output surface.
   */
  runPostProcessFrame(features = this.features()) {
    this._runHook("postProcessFrame", features);
  }

  /** Run `shutdown` for every enabled plugin. Best-effort during app quit. */
  async shutdownAll() {
    const enabled = this.plugins.filter((plugin) => plugin.enabled);

    for (let i = 0; i < enabled.length; i += 1) {
      const plugin = enabled[i];

      if (typeof plugin.shutdown !== "function") {
        continue;
      }

      try {
        await plugin.shutdown({ store: this.store, props: plugin.$props });
      } catch (error) {
        this.logger.warn("plugin shutdown failed", {
          plugin: plugin.name,
          message: error?.message || String(error),
        });
      }
    }
  }

  _runHook(hook, features) {
    const plugins = this.plugins;

    for (let i = 0; i < plugins.length; i += 1) {
      const plugin = plugins[i];

      if (!plugin.enabled || typeof plugin[hook] !== "function") {
        continue;
      }

      try {
        plugin[hook]({
          features,
          store: this.store,
          props: plugin.$props,
        });
      } catch (error) {
        const key = `${plugin.id}:${hook}`;

        if (!this._warnedFailures.has(key)) {
          this._warnedFailures.add(key);
          this.logger.warn("plugin frame hook failed", {
            plugin: plugin.name,
            hook,
            message: error?.message || String(error),
          });
        }
      }
    }
  }

  _warnUnsupported(plugin) {
    if (this._warnedUnsupported.has(plugin.name)) {
      return;
    }

    this._warnedUnsupported.add(plugin.name);
    this.logger.warn(
      "plugin is not available in engine mode; it needs the worker's output canvas/auxiliary outputs",
      {
        plugin: plugin.name,
        reason: plugin.engineModeReason || null,
      },
    );
  }

  _isPluginMutation(type) {
    return typeof type === "string" && this._pluginMutations.has(type);
  }

  _isPluginAction(type) {
    return typeof type === "string" && this._pluginActions.has(type);
  }

  _warnMirrorOnly(type) {
    if (this._warnedMutations.has(type)) {
      return;
    }

    this._warnedMutations.add(type);
    this.logger.warn(
      "plugin store mutation is applied to the renderer mirror only (the engine does not know it)",
      { type },
    );
  }

  /**
   * The store plugins receive. Reads come from the mirror; writes forward to
   * the engine except for mutations/actions a plugin registered itself.
   */
  _createStoreFacade() {
    const host = this;

    return {
      get state() {
        return host.mirrorStore.state;
      },

      get getters() {
        return host.mirrorStore.getters;
      },

      dispatch(type, payload) {
        if (host._isPluginAction(type)) {
          host._warnMirrorOnly(type);
          return host.mirrorStore.dispatch(type, payload);
        }

        if (host.engineStore?.dispatch) {
          return host.engineStore.dispatch(type, payload);
        }

        return host.mirrorStore.dispatch(type, payload);
      },

      commit(type, payload) {
        if (host._isPluginMutation(type)) {
          host._warnMirrorOnly(type);
          return host.mirrorStore.commit(type, payload);
        }

        if (host.engineStore?.commit) {
          return host.engineStore.commit(type, payload);
        }

        return host.mirrorStore.commit(type, payload);
      },

      registerModule(name, module) {
        return host.registerModule(name, module);
      },
    };
  }
}

/**
 * Build the engine-mode plugin host, or null in legacy mode (where the worker
 * keeps running the plugin lifecycle exactly as before).
 */
export function createPluginHost({
  engineMode,
  mirrorStore,
  engineStore = null,
  getFeatures = () => undefined,
  logger: log = logger,
  resolvePropDefault,
} = {}) {
  if (!engineMode) {
    return null;
  }

  return new PluginHost({
    mirrorStore,
    engineStore,
    getFeatures,
    logger: log,
    ...(resolvePropDefault ? { resolvePropDefault } : {}),
  });
}

// The Plugins panel goes through the engine store facade. These actions are
// renderer-owned in engine mode, so intercept them before they reach the
// engine's catch-all (which would silently do nothing).
const HOSTED_PLUGIN_ACTIONS = new Set([
  "plugins/add",
  "plugins/setEnabled",
  "plugins/updateProp",
]);

/**
 * Wrap the engine store facade so plugin actions/commits are handled by the
 * host and everything else keeps its existing engine path.
 */
export function createEngineStoreFacade(engineStore, pluginHost) {
  return {
    get state() {
      return engineStore.state;
    },

    get getters() {
      return engineStore.getters;
    },

    commit(type, payload) {
      if (typeof type === "string" && type.startsWith("plugins/")) {
        return pluginHost.mirrorStore.commit(type, payload);
      }

      return engineStore.commit(type, payload);
    },

    dispatch(type, payload) {
      if (HOSTED_PLUGIN_ACTIONS.has(type)) {
        if (type === "plugins/add") return pluginHost.add(payload);
        if (type === "plugins/setEnabled")
          return pluginHost.setEnabled(payload);
        return pluginHost.updateProp(payload);
      }

      return engineStore.dispatch(type, payload);
    },

    registerModule(name, module) {
      return pluginHost.registerModule(name, module);
    },
  };
}

// `install-plugin.js` runs from `modV.use()` and from the boot path, both of
// which import this module directly. A module-level registry keeps the legacy
// path untouched: with no host installed the original mirror-store dispatch
// runs exactly as before.
let activePluginHost = null;

export function setActivePluginHost(host) {
  activePluginHost = host || null;
  return activePluginHost;
}

export function getActivePluginHost() {
  return activePluginHost;
}

export { camelize };
