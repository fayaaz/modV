import { contextBridge } from "electron";
import { electronAPI } from "@electron-toolkit/preload";
import * as remote from "@electron/remote";
const { vibrate } = require("hapticjs");
const { ipcRenderer } = require("electron");

// WS-14 — window.engine: the renderer's IPC surface to the Rust engine
// (docs/rust-hybrid.md, Protocol specification). Commit is fire-and-forget;
// dispatch resolves with the action's return value; subscribe wires engine
// deltas ({type, payload}, commitQueue-compatible) to the caller.
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
  requestPreview() {
    return ipcRenderer.invoke("engine/preview");
  },
  getState() {
    return ipcRenderer.invoke("engine/get-state");
  },
  snapshot() {
    return ipcRenderer.invoke("engine/snapshot");
  },
  isEnabled() {
    return ipcRenderer.sendSync("engine/enabled-sync");
  },
};

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
    contextBridge.exposeInMainWorld("engine", engineApi);
  } catch (error) {
    console.error(error);
  }
} else {
  window.electron = electronAPI;
  window.remote = remote;
  window.api = api;
  window.engine = engineApi;
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
