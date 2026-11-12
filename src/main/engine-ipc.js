// WS-14 — IPC surface between the renderer and the engine supervisor.
//
// Renderer → engine: commit (fire-and-forget), dispatch (request/response,
//   resolves with the action's return value), snapshot, sync (the late-joiner
//   pull that replaces a missed `state/sync` event), preview fetch
//   (main output ring slot + off-graph module thumbnails for the Gallery).
// Engine → renderer: `engine/delta` (the commitQueue-compatible {type,payload}
//   records from `state/delta` events), `engine/state` (lifecycle transitions),
//   `engine/preview-frame` (preview ring header, for the frame counter).
//
// Registered once, at app-ready, like system-audio.js — the channels outlive
// individual windows. `getSupervisor` is a thunk so the module does not need
// the singleton to exist at import time.

import { ipcMain } from "electron";

import { createLogger } from "./logger";
import { windows } from "./windows";
import { readPreviewSlot } from "./engine-preview-read";

const logger = createLogger("engine-ipc");

/** Upper bound for a Gallery thumbnail request (tiles render well below this). */
const MODULE_PREVIEW_MAX_SIZE = 512;

/**
 * Upper bound for a Scene cell preview.
 *
 * `Scenes.vue` caps a cell at 480 × 270 (`scenePreviewSize`), so no request
 * needs a larger frame; the cap bounds the engine's preview targets/read-back.
 */
const SCENE_PREVIEW_MAX_SIZE = 480;

/**
 * Clamp a requested module-preview edge: tiles ask for their pixel size scaled
 * by the display's device-pixel ratio, so keep it a positive integer and stop a
 * bogus value from asking the engine for a huge off-screen render.
 */
function previewDimension(value) {
  const size = Math.round(Number(value));
  if (!Number.isFinite(size) || size <= 0) return MODULE_PREVIEW_MAX_SIZE;
  return Math.min(size, MODULE_PREVIEW_MAX_SIZE);
}

/**
 * Clamp a requested Scene-preview edge (same reasoning as
 * {@link previewDimension}, with the Scene cell's 480 px cap).
 */
function scenePreviewDimension(value) {
  const size = Math.round(Number(value));
  if (!Number.isFinite(size) || size <= 0) return SCENE_PREVIEW_MAX_SIZE;
  return Math.min(size, SCENE_PREVIEW_MAX_SIZE);
}

function mainWebContents() {
  return windows.mainWindow?.webContents || null;
}

/**
 * R-25 — the engine API belongs to the main window. The preload only exposes
 * it there, but output windows inherit the same preload and run with
 * `nodeIntegration`, so the main process validates the sender as well instead
 * of relying on the exposed surface alone.
 */
function isMainWindowSender(event) {
  const contents = mainWebContents();
  if (!contents || contents.isDestroyed()) return false;
  return event?.sender === contents;
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
    event.returnValue =
      isMainWindowSender(event) &&
      process.env.MODV_ENGINE === "1" &&
      !!getSupervisor();
  });

  ipcMain.on("engine/commit", (event, message) => {
    if (!isMainWindowSender(event)) return;
    const sup = getSupervisor();
    if (!sup?.isRunning()) return;
    const { type, payload } = message || {};
    if (typeof type !== "string") return;
    // Fire-and-forget: the engine still answers, but nobody waits on it.
    sup.client.request("state/commit", { type, payload }).catch((error) => {
      logger.warn(`engine commit "${type}" failed: ${error.message}`);
    });
  });

  ipcMain.handle("engine/dispatch", async (event, message) => {
    if (!isMainWindowSender(event)) {
      throw new Error("engine/dispatch is only available to the main window");
    }
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

  // Late-joiner fallback for `state/sync` (review R-4). The engine emits sync
  // once on accept and Electron does not queue IPC events, so a renderer that
  // subscribes after the socket connected would permanently miss the Groups,
  // Modules, `registered` map, seeded Scenes and restored tweens. The renderer
  // pulls this after subscribing and replays it idempotently.
  ipcMain.handle("engine/get-sync", async (event) => {
    if (!isMainWindowSender(event)) {
      throw new Error("engine/get-sync is only available to the main window");
    }
    const sup = getSupervisor();
    if (!sup?.isRunning()) {
      throw new Error(
        "the engine is not running, so it has no state to sync (MODV_ENGINE=1 required)",
      );
    }
    return sup.client.request("engine/get-sync", null, 5000);
  });

  ipcMain.handle("engine/snapshot", async (event) => {
    if (!isMainWindowSender(event)) return null;
    const sup = getSupervisor();
    if (!sup?.isRunning()) return null;
    return sup.client.request("state/snapshot");
  });

  // Main-window preview frame. `readPreviewSlot` mirrors the Rust ring's
  // seqlock: it bounds the slot/geometry from the `preview/ready` handshake,
  // copies exactly `stride × height` bytes and retries while the sequence
  // changes, so a mid-publish slot read is dropped instead of handing the
  // renderer a torn frame (or, on a short read, uninitialised memory).
  ipcMain.handle("engine/preview", async (event) => {
    if (!isMainWindowSender(event)) return null;
    const sup = getSupervisor();
    if (!sup?.isRunning()) return null;

    const info = sup.client?.previewInfo;
    const frame = sup.lastPreviewFrame;
    if (!info || !frame) {
      return { info: info || null, frame: null, pixels: null };
    }

    let pixels = null;
    try {
      pixels = readPreviewSlot(info, frame);
    } catch (error) {
      logger.warn(`could not read preview slot: ${error.message}`);
      return { info, frame, pixels: null, error: error.message };
    }

    if (!pixels) {
      // No consistent frame this poll: the renderer keeps the last frame and
      // asks again on its next cadence.
      return { info, frame, pixels: null };
    }

    return { info, frame, pixels };
  });

  // Gallery thumbnails. The engine renders the named module off the live graph
  // (the Rust counterpart of the worker's `isGallery` auxillary output) and
  // answers with `{ width, height, pixels }`, `pixels` being base64 RGBA8 of a
  // tightly packed width × height frame. Errors (module not implemented by the
  // engine yet, renderer busy, …) reject the invoke; the renderer treats that
  // as "no preview yet" and retries on its own cadence.
  ipcMain.handle("engine/preview-module", async (event, message) => {
    if (!isMainWindowSender(event)) return null;
    const sup = getSupervisor();
    if (!sup?.isRunning()) return null;

    const { moduleName, width, height } = message || {};
    if (typeof moduleName !== "string" || moduleName.length === 0) {
      return null;
    }

    return sup.client.request(
      "preview/module",
      {
        moduleName,
        width: previewDimension(width),
        height: previewDimension(height),
      },
      5000,
    );
  });

  // Scene hover previews. The engine renders the Scene cell's saved
  // Group/Module snapshots in an isolated compositor and answers with the same
  // `{ width, height, pixels }` shape as `preview/module` (base64 RGBA8).
  // Unknown Scenes/empty rows reject the invoke; the renderer keeps the cell's
  // last frame and retries on its backoff cadence.
  ipcMain.handle("engine/preview-scene", async (event, message) => {
    if (!isMainWindowSender(event)) return null;
    const sup = getSupervisor();
    if (!sup?.isRunning()) return null;

    const { sceneId, rowIndex, width, height } = message || {};
    if (typeof sceneId !== "string" || sceneId.length === 0) {
      return null;
    }
    const row = Number(rowIndex);
    if (!Number.isInteger(row) || row < 0) {
      return null;
    }

    return sup.client.request(
      "preview/scene",
      {
        sceneId,
        rowIndex: row,
        width: scenePreviewDimension(width),
        height: scenePreviewDimension(height),
      },
      5000,
    );
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
    client.on("state/sync", (data) => {
      sendToMainWindow("engine/state-sync", data);
    });
    client.on("preview/ready", () => {
      logger.info("engine preview channel ready");
    });
  });
}
