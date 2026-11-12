// WS-14 — renderer-side IPC client for the Rust engine.
//
// Mirrors the worker facade exactly (store.commit / store.dispatch) so the
// 59 Vue components are untouched, and replays `state/delta` events into the
// mirror store with the same {type, payload} records the worker's commitQueue
// produced (application/index.js:199-211 is replaced by this path).
//
// window.engine comes from the preload (src/preload/index.js) and fronts the
// main process's engine-protocol.js — the renderer never touches the socket.

import { createLogger } from "./logger";

const logger = createLogger("renderer:engine-client");

/** Pixel format ids from the engine's VMP1 preview ring (crates/modv-ipc). */
const FORMAT_RGBA8 = 1;
const FORMAT_BGRA8 = 2;

/** The engine publishes frames with 32-bit pixels. */
const BYTES_PER_PIXEL = 4;

/**
 * Scene hover previews poll at the legacy worker's cadence
 * (`scene-preview.js` `PREVIEW_FPS = 30`).
 */
const SCENE_PREVIEW_FPS = 30;

/**
 * Back-off after a failed or empty Scene preview.
 *
 * The Scene may not exist in the engine yet (a just-saved Scene can reach the
 * engine a moment after the mirror shows it), so a fast retry would only
 * repeat the same error. 500 ms keeps a hover responsive without flooding the
 * control socket.
 */
const SCENE_PREVIEW_RETRY_MS = 500;

/**
 * Pure throttle used by both preview surfaces: is a pull due at `nowMs`?
 *
 * `lastMs` is the timestamp of the last pull; `null`/`undefined` means "nothing
 * pulled yet", which is always due. A non-positive or non-finite `fps` disables
 * polling entirely (used while a panel is hidden).
 */
function shouldRefresh(nowMs, lastMs, fps) {
  if (!Number.isFinite(nowMs)) return false;
  if (!Number.isFinite(fps) || fps <= 0) return false;
  if (lastMs === null || lastMs === undefined) return true;
  if (!Number.isFinite(lastMs)) return false;
  return nowMs - lastMs >= 1000 / fps;
}

/**
 * Decode the base64 RGBA8 payload of a `preview/module` response.
 *
 * Decoded here in the renderer rather than in the preload/main so the bytes
 * exist in exactly one place (this side of the bridge); the IPC payload stays a
 * plain string, which is what `ipcRenderer.invoke` serialises cheapest.
 *
 * @returns {Uint8Array | null} null for a missing/empty/undecodable payload.
 */
function decodeBase64Pixels(base64) {
  if (typeof base64 !== "string" || base64.length === 0) {
    return null;
  }

  let binary;
  try {
    binary = atob(base64);
  } catch (_error) {
    return null;
  }

  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/**
 * Normalise a raw engine frame into `{ width, height, data }` where `data` is a
 * tightly packed RGBA8 `Uint8ClampedArray` copy — the shape
 * `new ImageData(data, width, height)` expects.
 *
 * Handles the ring's padded `stride` (rows are copied out one at a time) and
 * BGRA8 frames (B/R swapped), and refuses buffers that are too short for the
 * declared geometry.
 *
 * @param {Uint8Array | Buffer | null} source
 * @param {{ width?: number, height?: number, stride?: number, format?: number }} geometry
 * @returns {{ width: number, height: number, data: Uint8ClampedArray } | null}
 */
function toFrameData(source, { width, height, stride, format } = {}) {
  if (!source || typeof source.length !== "number") {
    return null;
  }

  const w = Number(width);
  const h = Number(height);
  if (!Number.isInteger(w) || !Number.isInteger(h) || w <= 0 || h <= 0) {
    return null;
  }

  const rowBytes = w * BYTES_PER_PIXEL;
  const sourceStride =
    Number.isInteger(stride) && stride >= rowBytes ? stride : rowBytes;
  // The last row only needs `rowBytes`; trailing padding may be truncated by
  // the main process' bounded slot read.
  const needed = sourceStride * (h - 1) + rowBytes;
  if (source.length < needed) {
    return null;
  }

  const data = new Uint8ClampedArray(rowBytes * h);
  if (sourceStride === rowBytes) {
    data.set(source.subarray(0, rowBytes * h));
  } else {
    for (let row = 0; row < h; row += 1) {
      const start = row * sourceStride;
      data.set(source.subarray(start, start + rowBytes), row * rowBytes);
    }
  }

  if (Number(format) === FORMAT_BGRA8) {
    for (let i = 0; i < data.length; i += BYTES_PER_PIXEL) {
      const b = data[i];
      data[i] = data[i + 2];
      data[i + 2] = b;
    }
  }

  return { width: w, height: h, data };
}

/**
 * `engine/preview` reply → frame data for the main-window preview panel.
 *
 * The reply is `{ info, frame, pixels }`: `info` is the `preview/ready`
 * handshake (`{ width, height, stride, format, … }`) and `frame` the
 * `preview/frame` event for the slot that was read. A missing `pixels` means
 * the engine has not produced a frame yet.
 */
function outputPreviewFromResponse(response) {
  if (!response || typeof response !== "object" || !response.pixels) {
    return null;
  }

  const { info, frame } = response;

  return toFrameData(response.pixels, {
    width: frame?.width ?? info?.width,
    height: frame?.height ?? info?.height,
    stride: frame?.stride ?? info?.stride,
    format: frame?.format ?? info?.format,
  });
}

/**
 * `preview/module` reply → frame data for a Gallery tile.
 *
 * The engine returns `{ width, height, pixels }` with base64 RGBA8 (raw bytes
 * are accepted too). The payload must be exactly one tightly packed frame;
 * anything else is treated as "no preview yet" so a half-written or stale
 * response never fills a tile with garbage.
 */
function modulePreviewFromResponse(response) {
  if (!response || typeof response !== "object") {
    return null;
  }

  const w = Number(response.width);
  const h = Number(response.height);
  if (!Number.isInteger(w) || !Number.isInteger(h) || w <= 0 || h <= 0) {
    return null;
  }

  const source =
    typeof response.pixels === "string"
      ? decodeBase64Pixels(response.pixels)
      : response.pixels;

  if (!source || source.length !== w * h * BYTES_PER_PIXEL) {
    return null;
  }

  return toFrameData(source, { width: w, height: h, format: response.format });
}

/**
 * `preview/scene` reply → frame data for a Scene grid cell.
 *
 * The engine returns `{ width, height, pixels }` with base64 RGBA8 (raw bytes
 * are accepted too). The frame is tightly packed, but a `stride` is honoured
 * when present (the same bounded-read shape as the output preview), so a
 * future padded transport keeps working. Anything that does not cover the
 * declared geometry is treated as "no preview yet".
 */
function scenePreviewFromResponse(response) {
  if (!response || typeof response !== "object") {
    return null;
  }

  const w = Number(response.width);
  const h = Number(response.height);
  if (!Number.isInteger(w) || !Number.isInteger(h) || w <= 0 || h <= 0) {
    return null;
  }

  const source =
    typeof response.pixels === "string"
      ? decodeBase64Pixels(response.pixels)
      : response.pixels;
  if (!source) {
    return null;
  }

  const rowBytes = w * BYTES_PER_PIXEL;
  const stride =
    Number.isInteger(response.stride) && response.stride >= rowBytes
      ? response.stride
      : rowBytes;
  // The last row only needs `rowBytes`; a tightly packed frame must be exactly
  // one frame, so a truncated or oversized buffer is refused.
  if (source.length < stride * (h - 1) + rowBytes) {
    return null;
  }
  if (stride === rowBytes && source.length !== rowBytes * h) {
    return null;
  }

  return toFrameData(source, {
    width: w,
    height: h,
    stride,
    format: response.format,
  });
}

/**
 * Legacy Scene-preview runtime key (`scene-preview.js`: `` `${sceneId}:${row}` ``).
 *
 * @returns {string | null} null for a missing id or a non-integer row.
 */
function scenePreviewKey(sceneId, rowIndex) {
  if (rowIndex === null || rowIndex === undefined) {
    return null;
  }

  const row = Number(rowIndex);
  if (typeof sceneId !== "string" || sceneId.length === 0) {
    return null;
  }
  if (!Number.isInteger(row) || row < 0) {
    return null;
  }
  return `${sceneId}:${row}`;
}

/**
 * Whether hovering a Scene cell must transfer its canvas to the worker.
 *
 * Transfer is a one-way operation on the DOM canvas. In engine mode the
 * renderer keeps the canvas and paints the engine's frames with
 * `putImageData`, so transferring it would make the cell undrawable; the
 * legacy worker path still transfers exactly once per canvas.
 */
function shouldTransferScenePreviewCanvas({ engineMode, alreadyTransferred }) {
  return !engineMode && !alreadyTransferred;
}

/**
 * Next Scene-preview poll delay in ms.
 *
 * - `null` while a request is in flight or while the document is hidden (the
 *   caller skips the pull and re-checks on its own cadence);
 * - [`SCENE_PREVIEW_RETRY_MS`] after a failed/empty response;
 * - otherwise the remaining time until the ~30 fps cadence (`0` = now).
 */
function scenePreviewPollDelay(
  nowMs,
  lastMs,
  { inFlight = false, hidden = false, failures = 0 } = {},
) {
  if (inFlight || hidden) {
    return null;
  }
  if (Number.isFinite(failures) && failures > 0) {
    return SCENE_PREVIEW_RETRY_MS;
  }
  if (lastMs === null || lastMs === undefined) {
    return 0;
  }
  if (!Number.isFinite(nowMs) || !Number.isFinite(lastMs)) {
    return 0;
  }
  return Math.max(0, lastMs + 1000 / SCENE_PREVIEW_FPS - nowMs);
}

export default class EngineClient {
  constructor() {
    if (!window.engine) {
      throw new Error(
        "window.engine is not exposed; is the preload engine API installed?",
      );
    }

    this.engine = window.engine;
    this._unsubscribe = { delta: null, state: null, preview: null };
    this._previewFrameCount = 0;
    this._stateCallback = null;
    this._stateFromInvoke = false;
    // Preview surfaces a warning has already been logged for (tiles poll).
    this._previewWarned = new Set();

    if (typeof window !== "undefined") {
      window.__enginePreviewCount = 0;
    }

    this.storeFacade = {
      // state/getters are bound by bindMirror() after the mirror store exists
      state: null,
      getters: null,
      commit: (type, payload) => this.engine.commit(type, payload),
      dispatch: (type, payload) => this.engine.dispatch(type, payload),
    };
  }

  /** Bind the renderer's mirror store into the facade (same objects the UI reads). */
  bindMirror(store) {
    this.storeFacade.state = store.state;
    this.storeFacade.getters = store.getters;
    return this.storeFacade;
  }

  /** Subscribe to engine deltas; cb receives { type, payload }. Returns an unsubscribe fn. */
  onDelta(callback) {
    this._unsubscribe.delta = this.engine.subscribe((delta) => callback(delta));
    return this._unsubscribe.delta;
  }

  /** Subscribe to lifecycle transitions ({ state, reason? }); returns unsubscribe fn. */
  onState(callback) {
    this._stateCallback = callback;
    this._unsubscribe.state = this.engine.onEngineState(callback);

    // The engine may have transitioned to "running" before the renderer
    // subscribed (the event is not replayed). Pin the current state through
    // the invoke fallback so a late subscriber still initialises.
    this.engine
      .getState?.()
      .then((transition) => {
        if (transition?.state === "running" && this._stateCallback) {
          this._stateCallback({ state: "running" });
        }
      })
      .catch(() => {});

    return this._unsubscribe.state;
  }

  /** Late-joiner state sync: the full mirror-relevant engine state. */
  onStateSync(callback) {
    this._unsubscribe.sync = this.engine.onStateSync(callback);
    return this._unsubscribe.sync;
  }

  /** Count preview frames into window.__enginePreviewCount (e2e hook). */
  trackPreviewFrames() {
    this._unsubscribe.preview = this.engine.onPreviewFrame(() => {
      this._previewFrameCount += 1;
      window.__enginePreviewCount = this._previewFrameCount;
    });
  }

  requestPreview() {
    return this.engine.requestPreview();
  }

  /**
   * Main-window preview frame as `{ width, height, data }` for a 2D canvas.
   *
   * Engine mode only (the preload API exists only when MODV_ENGINE=1). Resolves
   * to null while the engine is stopped or has not published a frame yet — the
   * caller leaves its canvas blank and retries.
   */
  requestOutputPreview() {
    let pending;
    try {
      pending = this.engine.requestPreview?.();
    } catch (error) {
      this._previewFailed("output", error);
      return Promise.resolve(null);
    }

    return Promise.resolve(pending)
      .then((response) => outputPreviewFromResponse(response))
      .catch((error) => {
        this._previewFailed("output", error);
        return null;
      });
  }

  /**
   * Gallery thumbnail for `moduleName`, rendered by the engine at
   * `width` × `height`. Resolves to `{ width, height, data }` or null.
   *
   * The engine side is `preview/module` (another workstream); until it exists
   * every call rejects and resolves to null, so tiles simply stay blank. Only
   * the first failure per module is logged — tiles poll, so warning every poll
   * would flood the log.
   */
  requestModulePreview(moduleName, width, height) {
    if (typeof moduleName !== "string" || moduleName.length === 0) {
      return Promise.resolve(null);
    }

    let pending;
    try {
      pending = this.engine.requestModulePreview({ moduleName, width, height });
    } catch (error) {
      this._previewFailed(`module:${moduleName}`, error);
      return Promise.resolve(null);
    }

    return Promise.resolve(pending)
      .then((response) => {
        const frame = modulePreviewFromResponse(response);
        if (frame) this._previewWarned.delete(`module:${moduleName}`);
        return frame;
      })
      .catch((error) => {
        this._previewFailed(`module:${moduleName}`, error);
        return null;
      });
  }

  /**
   * Scene cell preview for `sceneId` at `rowIndex`, rendered by the engine at
   * `width` × `height`. Resolves to `{ width, height, data }` or null.
   *
   * The engine side is `preview/scene`, which renders the Scene's saved
   * Group/Module snapshots in an isolated compositor. Errors (unknown Scene,
   * empty row, engine busy) resolve to null so the cell keeps its last frame
   * and the caller retries on its backoff cadence. Only the first failure per
   * cell is logged — a hover polls at ~30 fps.
   */
  requestScenePreview(sceneId, rowIndex, width, height) {
    const key = scenePreviewKey(sceneId, rowIndex);
    if (key === null) {
      return Promise.resolve(null);
    }

    let pending;
    try {
      pending = this.engine.requestScenePreview({
        sceneId,
        rowIndex: Number(rowIndex),
        width,
        height,
      });
    } catch (error) {
      this._previewFailed(`scene:${key}`, error);
      return Promise.resolve(null);
    }

    return Promise.resolve(pending)
      .then((response) => {
        const frame = scenePreviewFromResponse(response);
        if (frame) this._previewWarned.delete(`scene:${key}`);
        return frame;
      })
      .catch((error) => {
        this._previewFailed(`scene:${key}`, error);
        return null;
      });
  }

  /** Log the first failure of each preview surface; tiles poll, so once is enough. */
  _previewFailed(key, error) {
    if (this._previewWarned.has(key)) {
      return;
    }
    this._previewWarned.add(key);
    logger.warn("engine preview unavailable", {
      surface: key,
      message: error?.message || String(error),
    });
  }

  snapshot() {
    return this.engine.snapshot();
  }

  dispose() {
    for (const key of Object.keys(this._unsubscribe)) {
      if (this._unsubscribe[key]) {
        this._unsubscribe[key]();
        this._unsubscribe[key] = null;
      }
    }
  }
}

// Pure preview helpers, exported for the unit tests (tests/unit/engine-preview).
export {
  decodeBase64Pixels,
  modulePreviewFromResponse,
  outputPreviewFromResponse,
  scenePreviewFromResponse,
  scenePreviewKey,
  scenePreviewPollDelay,
  shouldRefresh,
  shouldTransferScenePreviewCanvas,
  toFrameData,
  FORMAT_BGRA8,
  FORMAT_RGBA8,
};
