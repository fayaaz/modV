import { contextBridge } from "electron";
import { electronAPI } from "@electron-toolkit/preload";
import * as remote from "@electron/remote";
import { engineApiScope } from "./engine-api-scope";
const { vibrate } = require("hapticjs");
const { ipcRenderer } = require("electron");

// WS-14 — window.engine: the renderer's IPC surface to the Rust engine
// (docs/rust-hybrid.md, Protocol specification). Commit is fire-and-forget;
// dispatch resolves with the action's return value; subscribe wires engine
// deltas ({type, payload}, commitQueue-compatible) to the caller.
//
// R-25: only the main window gets this surface. Output windows inherit this
// preload but run no script, the splash window has no preload and the color
// picker uses `colorPicker.js`; every other path must not receive a mutating
// engine API.
const engineApi = {
  commit(type, payload) {
    ipcRenderer.send("engine/commit", { type, payload });
  },
  dispatch(type, payload) {
    return ipcRenderer.invoke("engine/dispatch", { type, payload });
  },
  subscribe(callback) {
    const listener = (_event, data) => callback(data);
    ipcRenderer.on("engine/delta", listener);
    return () => ipcRenderer.removeListener("engine/delta", listener);
  },
  onEngineState(callback) {
    const listener = (_event, data) => callback(data);
    ipcRenderer.on("engine/state", listener);
    return () => ipcRenderer.removeListener("engine/state", listener);
  },
  onPreviewFrame(callback) {
    const listener = (_event, data) => callback(data);
    ipcRenderer.on("engine/preview-frame", listener);
    return () => ipcRenderer.removeListener("engine/preview-frame", listener);
  },
  onStateSync(callback) {
    const listener = (_event, data) => callback(data);
    ipcRenderer.on("engine/state-sync", listener);
    return () => ipcRenderer.removeListener("engine/state-sync", listener);
  },
  requestPreview() {
    return ipcRenderer.invoke("engine/preview");
  },
  // Gallery thumbnails: the engine renders one module off-graph and answers
  // with { width, height, pixels } where pixels is base64 RGBA8. The renderer
  // decodes it (application/engine-client.js) so no bytes are duplicated here.
  requestModulePreview({ moduleName, width, height } = {}) {
    return ipcRenderer.invoke("engine/preview-module", {
      moduleName,
      width,
      height,
    });
  },
  // Scene hover previews: the engine renders the Scene cell's saved
  // Group/Module snapshots off-graph and answers with { width, height, pixels }
  // (base64 RGBA8), decoded in application/engine-client.js.
  requestScenePreview({ sceneId, rowIndex, width, height } = {}) {
    return ipcRenderer.invoke("engine/preview-scene", {
      sceneId,
      rowIndex,
      width,
      height,
    });
  },
  // WS-10 — engine-owned media capture (GStreamer). The renderer keeps the
  // device UI; these calls only carry the capture intent. Every method resolves
  // with the engine's reply or rejects with an error whose message starts with
  // a `media_*` code (`application/engine-media.js#mapEngineMediaError`).
  listCameras() {
    return ipcRenderer.invoke("engine/media-list");
  },
  startEngineCamera({ sourceId, cameraId, index, width, height, fps } = {}) {
    return ipcRenderer.invoke("engine/media-start-camera", {
      sourceId,
      cameraId,
      index,
      width,
      height,
      fps,
    });
  },
  startEngineFile({ sourceId, path, width, height, fps, loop } = {}) {
    return ipcRenderer.invoke("engine/media-start-file", {
      sourceId,
      path,
      width,
      height,
      fps,
      loop,
    });
  },
  startEngineScreen({ sourceId, nodeId } = {}) {
    return ipcRenderer.invoke("engine/media-start-screen", {
      sourceId,
      nodeId,
    });
  },
  stopEngineSource(sourceId) {
    return ipcRenderer.invoke("engine/media-stop", { sourceId });
  },
  engineMediaStatus() {
    return ipcRenderer.invoke("engine/media-status");
  },
  getState() {
    return ipcRenderer.invoke("engine/get-state");
  },
  // Late-joiner state sync: resolves with the same payload the `state/sync`
  // event carries (preset document + `registered` + Scenes/performance). The
  // renderer pulls it after subscribing because IPC events are not queued, so
  // a sync emitted before the subscription would be lost.
  getSync() {
    return ipcRenderer.invoke("engine/get-sync");
  },
  snapshot() {
    return ipcRenderer.invoke("engine/snapshot");
  },
  isEnabled() {
    return ipcRenderer.sendSync("engine/enabled-sync");
  },
};

// `window.location.pathname` is `/index.html` in development and the packaged
// file URL path in production. Any other window (output window, splash, color
// picker, unknown page) gets `window.engine === undefined`.
const engineSurface =
  engineApiScope(window.location?.pathname) === "main" ? engineApi : undefined;

// Custom APIs for renderer
const api = {
  vibrate,
};

// Use `contextBridge` APIs to expose Electron APIs to
// renderer only if context isolation is enabled, otherwise
// just add to the DOM global.
if (process.contextIsolated) {
  try {
    contextBridge.exposeInMainWorld("electron", electronAPI);
    contextBridge.exposeInMainWorld("api", api);
    contextBridge.exposeInMainWorld("remote", remote);
    if (engineSurface) {
      contextBridge.exposeInMainWorld("engine", engineSurface);
    }
  } catch (error) {
    console.error(error);
  }
} else {
  window.electron = electronAPI;
  window.remote = remote;
  window.api = api;
  window.engine = engineSurface;
}

ipcRenderer.on("port:colorPickerUI", (e) => {
  // mainwindow got port from main

  if (window.electronMessagePort) {
    // mainwindow closed existing port
    window.electronMessagePort.close();
  }

  // port received, make it globally available.
  window.electronMessagePort = e.ports[0];
});
