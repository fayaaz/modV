// WS-14 — IPC surface between the renderer and the engine supervisor.
//
// Renderer → engine: commit (fire-and-forget), dispatch (request/response,
//   resolves with the action's return value), snapshot, preview fetch.
// Engine → renderer: `engine/delta` (the commitQueue-compatible {type,payload}
//   records from `state/delta` events), `engine/state` (lifecycle transitions),
//   `engine/preview-frame` (preview ring header, for the frame counter).
//
// Registered once, at app-ready, like system-audio.js — the channels outlive
// individual windows. `getSupervisor` is a thunk so the module does not need
// the singleton to exist at import time.

import { ipcMain } from "electron";
import fs from "node:fs";

import { createLogger } from "./logger";
import { windows } from "./windows";

const logger = createLogger("engine-ipc");

function mainWebContents() {
  return windows.mainWindow?.webContents || null;
}

function sendToMainWindow(channel, payload) {
  const contents = mainWebContents();
  if (contents && !contents.isDestroyed()) {
    contents.send(channel, payload);
  }
}

/**
 * Register the engine IPC channels.
 * @param {() => import("./engine-supervisor").EngineSupervisor | null} getSupervisor
 */
export function registerEngineIpc(getSupervisor) {
  // -- renderer → engine ---------------------------------------------------

  ipcMain.on("engine/enabled-sync", (event) => {
    event.returnValue = process.env.MODV_ENGINE === "1" && !!getSupervisor();
  });

  ipcMain.on("engine/commit", (_event, message) => {
    const sup = getSupervisor();
    if (!sup?.isRunning()) return;
    const { type, payload } = message || {};
    if (typeof type !== "string") return;
    // Fire-and-forget: the engine still answers, but nobody waits on it.
    sup.client.request("state/commit", { type, payload }).catch((error) => {
      logger.warn(`engine commit "${type}" failed: ${error.message}`);
    });
  });

  ipcMain.handle("engine/dispatch", async (_event, message) => {
    const sup = getSupervisor();
    if (!sup?.isRunning()) {
      throw new Error("the engine is not running (MODV_ENGINE=1 required)");
    }
    const { type, payload } = message || {};
    if (typeof type !== "string") {
      throw new Error("engine/dispatch requires a mutation/action name");
    }
    return sup.client.request("state/dispatch", { type, payload });
  });

  ipcMain.handle("engine/get-state", async () => {
    const sup = getSupervisor();
    return { state: sup ? sup.getState() : "stopped" };
  });

  ipcMain.handle("engine/snapshot", async () => {
    const sup = getSupervisor();
    if (!sup?.isRunning()) return null;
    return sup.client.request("state/snapshot");
  });

  ipcMain.handle("engine/preview", async () => {
    const sup = getSupervisor();
    if (!sup?.isRunning()) return null;

    const info = sup.client?.previewInfo;
    const frame = sup.lastPreviewFrame;
    if (!info || !frame) {
      return { info: info || null, frame: null };
    }

    const offset = frame.slot * info.slotBytes;
    const length = Math.min(frame.height * frame.stride, info.slotBytes - 32);
    const pixels = Buffer.allocUnsafe(length);

    try {
      const fd = fs.openSync(info.path, "r");
      try {
        fs.readSync(fd, pixels, 0, length, offset + 32);
      } finally {
        fs.closeSync(fd);
      }
    } catch (error) {
      logger.warn(`could not read preview slot: ${error.message}`);
      return { info, frame, pixels: null, error: error.message };
    }

    return { info, frame, pixels };
  });

  // -- engine → renderer ---------------------------------------------------

  const sup = getSupervisor();
  if (!sup) return;

  sup.on("engine-state", (transition) => {
    sendToMainWindow("engine/state", transition);

    if (transition.state !== "running") return;

    // The supervisor re-creates the client on every (re)start, so tap the
    // fresh client each time the engine (re)connects.
    const client = sup.client;
    if (!client || client.__engineIpcWired) return;
    client.__engineIpcWired = true;

    client.on("state/delta", (data) => {
      sendToMainWindow("engine/delta", data);
    });
    client.on("preview/frame", (data) => {
      sup.lastPreviewFrame = data;
      sendToMainWindow("engine/preview-frame", data);
    });
    client.on("preview/ready", () => {
      logger.info("engine preview channel ready");
    });
  });
}

/** Channels registered by registerEngineIpc; exported for destroy() symmetry. */
export const ENGINE_IPC_CHANNELS = {
  on: ["engine/enabled-sync", "engine/commit"],
  handle: [
    "engine/dispatch",
    "engine/snapshot",
    "engine/preview",
    "engine/get-state",
  ],
};
