import store from "./worker/store";
import uiStore from "../ui-store";
import { app } from "../main";
import { camelize, getActivePluginHost } from "./plugin-host";

/**
 * Install an extension plugin.
 *
 * Legacy mode (`MODV_ENGINE=0`) keeps the original behaviour: `plugins/add`
 * and the plugin's `store` module are registered on the renderer's mirror
 * store, and the worker registers/hooks its own built-in plugins.
 *
 * Engine mode installs through the renderer plugin host instead
 * (`plugin-host.js`): there is no worker to call `plugins/add`, so the host
 * adds the plugin to the mirror, registers the plugin store module there and
 * runs `init`/`shutdown`/per-frame hooks with the engine-backed store facade.
 */
export default async function installPlugin(plugin) {
  if (!("name" in plugin)) {
    throw new Error("Plugin requires a name");
  }

  const host = getActivePluginHost();

  if (host) {
    await host.add(plugin);
  } else {
    store.dispatch("plugins/add", plugin);

    if ("store" in plugin) {
      const storeName = plugin.storeName || camelize(plugin.name);
      store.registerModule(storeName, plugin.store);
    }
  }

  if ("uiStore" in plugin) {
    const uiStoreName = plugin.uiStoreName || camelize(plugin.name);
    uiStore.registerModule(uiStoreName, plugin.uiStore);
  }

  if ("galleryTabComponent" in plugin) {
    app.component(plugin.galleryTabComponent.name, plugin.galleryTabComponent);
  }

  if ("controlPanelComponent" in plugin) {
    app.component(
      plugin.controlPanelComponent.name,
      plugin.controlPanelComponent,
    );
  }

  if ("install" in plugin) {
    plugin.install(app, host ? host.store : store, uiStore);
  }

  return plugin;
}
