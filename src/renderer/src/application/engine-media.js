// WS-10 — renderer-side plumbing for engine-owned media capture.
//
// With `MODV_ENGINE=1` the Rust engine captures cameras, video files and
// screen shares through GStreamer; the renderer keeps the device-management UI
// and the saved selection (`mediaStream.selectedVideoSources`) but must never
// open `getUserMedia`, an `ImageCapture` loop or a WebCodecs decoder.
//
// The split is deliberate:
//
// * this module is pure — DTO mapping, source-id/order derivation, path
//   conversion, `media/status` formatting and error mapping — so it can be
//   unit tested in plain Node (`tests/unit/engine-media.test.js`);
// * `createEngineMediaController` is a thin stateful wrapper around
//   `window.engine`: it remembers which sources *this* renderer started (so a
//   camera can be stopped, retried or stopped again on quit), de-duplicates
//   start-failure notices and reconciles `media/status` into stop/restart
//   decisions. It takes the engine object as an argument so tests can pass a
//   fake.
//
// Wire contract (implemented by `crates/modv-app`):
//
//   media/list-cameras  → { cameras: [{ id, name, path, caps: [{width,height,fps}] }] }
//   media/start-camera   { sourceId, cameraId, width, height, fps } → { sourceId, index }
//   media/start-file     { sourceId, path, width, height, fps, loop } → { sourceId, index }
//   media/start-screen   { sourceId, nodeId } → { sourceId, index }
//   media/stop-source    { sourceId } → { sourceId }
//   media/status        → { sources: [{ sourceId, kind, index, state, width,
//                                      height, frames, dropped, lastError }] }
//
// Errors: `media_unavailable`, `media_plugin_missing`, `media_start_failed`.

const CAMERA_PREFIX = "camera:";
const FILE_PREFIX = "video:";
const SCREEN_PREFIX = "screen:";

/** Status poll cadence while the Audio/Video panel is open. */
export const ENGINE_MEDIA_STATUS_POLL_MS = 1000;

/**
 * How long a camera that reported `error`/`stopped` (or vanished from the
 * engine's status) is left alone before the renderer asks the engine to start
 * it again. Matches the legacy renderer's 1.5 s reconnect delay in spirit
 * without hammering a wedged device.
 */
export const ENGINE_MEDIA_RESTART_INTERVAL_MS = 5000;

/** Human-readable labels for `media/status` source states. */
export const MEDIA_SOURCE_STATE_LABELS = Object.freeze({
  starting: "Starting",
  running: "Live",
  stopped: "Stopped",
  error: "Error",
  failed: "Error",
});

/** Engine failure codes the UI explains. */
export const ENGINE_MEDIA_ERROR_HINTS = Object.freeze({
  media_unavailable: "Engine video capture is unavailable",
  media_plugin_missing:
    "A GStreamer plugin required for this source is missing",
  media_start_failed: "The engine could not start the video source",
});

const KNOWN_ERROR_CODES = Object.keys(ENGINE_MEDIA_ERROR_HINTS);

/** Source kinds the controller tracks (also the `kind` field of `media/status`). */
const KIND_CAMERA = "camera";
const KIND_FILE = "file";
const KIND_SCREEN = "screen";

function isPositiveInteger(value) {
  return Number.isInteger(value) && value > 0;
}

function positiveDimension(value) {
  const number = Math.round(Number(value));

  return Number.isFinite(number) && number > 0 ? number : 0;
}

// -- engine surface --------------------------------------------------------

/**
 * Whether engine-owned capture is active. `window.engine.isEnabled()` is a
 * synchronous IPC round-trip in the preload, so a throwing implementation must
 * fall back to the legacy path instead of taking the renderer down.
 *
 * @param {object} [engine] defaults to `window.engine`
 * @returns {boolean}
 */
export function isEngineMediaEnabled(engine) {
  const surface =
    engine === undefined
      ? typeof window !== "undefined"
        ? window.engine
        : undefined
      : engine;

  if (!surface || typeof surface.isEnabled !== "function") {
    return false;
  }

  try {
    return surface.isEnabled() === true;
  } catch (_error) {
    return false;
  }
}

// -- camera DTOs -----------------------------------------------------------

/**
 * Resolution/frame-rate options from the engine's caps list, in the shape the
 * `mediaStream` store and `AudioVideo.vue` already render.
 *
 * @param {Array<{width?: number, height?: number, fps?: number}>} caps
 * @returns {{ resolutions: Array<{width: number, height: number}>, frameRates: Array<{value: number, label: string}> }}
 */
export function cameraCapabilities(caps) {
  const resolutions = new Map();
  const frameRates = new Map();

  for (const cap of Array.isArray(caps) ? caps : []) {
    if (!cap || typeof cap !== "object") {
      continue;
    }

    const width = Math.round(Number(cap.width));
    const height = Math.round(Number(cap.height));
    const fps = Math.round(Number(cap.fps));

    if (width > 0 && height > 0) {
      resolutions.set(`${width}x${height}`, { width, height });
    }

    if (fps > 0) {
      frameRates.set(fps, { value: fps, label: `${fps} fps` });
    }
  }

  return {
    resolutions: [...resolutions.values()].sort(
      (a, b) => a.width * a.height - b.width * b.height || a.width - b.width,
    ),
    frameRates: [...frameRates.values()].sort((a, b) => a.value - b.value),
  };
}

/**
 * Map one engine camera into the `mediaStream.video` device shape.
 *
 * The engine's camera id is the V4L2 device path (``/dev/videoN``) — stable
 * across restarts and readable in a log — so it doubles as the `deviceId`.
 * Chromium's opaque per-origin `deviceId` cannot be preserved, which is why a
 * legacy selection does not survive a switch into engine mode.
 *
 * @param {{id?: string, name?: string, path?: string, caps?: Array}} camera
 * @returns {{deviceId: string, label: string, kind: string, engine: boolean, engineCameraId: string, caps: Array} | null}
 */
export function engineCameraDevice(camera) {
  if (!camera || typeof camera !== "object") {
    return null;
  }

  const id = typeof camera.id === "string" ? camera.id : "";
  const path = typeof camera.path === "string" ? camera.path : "";
  const deviceId = path || id;

  if (!deviceId) {
    return null;
  }

  return {
    deviceId,
    label: camera.name || deviceId,
    kind: "videoinput",
    engine: true,
    engineCameraId: id || deviceId,
    caps: Array.isArray(camera.caps) ? camera.caps : [],
  };
}

/**
 * Map a `media/list-cameras` reply (or its `cameras` array) to devices.
 *
 * @param {{cameras?: Array} | Array | null | undefined} response
 * @returns {Array<ReturnType<typeof engineCameraDevice>>}
 */
export function engineCameraDevices(response) {
  const cameras = Array.isArray(response)
    ? response
    : Array.isArray(response?.cameras)
      ? response.cameras
      : [];

  return cameras.map(engineCameraDevice).filter(Boolean);
}

// -- source ids and camera order -------------------------------------------

/**
 * Stable engine source id for a camera device.
 * @param {string} deviceId
 * @returns {string | null}
 */
export function cameraSourceId(deviceId) {
  if (typeof deviceId !== "string" || deviceId.length === 0) {
    return null;
  }

  return `${CAMERA_PREFIX}${deviceId}`;
}

/**
 * Inverse of {@link cameraSourceId} (null for any other source kind).
 * @param {string} sourceId
 * @returns {string | null}
 */
export function cameraDeviceIdFromSourceId(sourceId) {
  if (typeof sourceId !== "string" || !sourceId.startsWith(CAMERA_PREFIX)) {
    return null;
  }

  const deviceId = sourceId.slice(CAMERA_PREFIX.length);

  return deviceId.length ? deviceId : null;
}

/**
 * The 1-based order the legacy Webcam module's `camera` prop indexes: the
 * position in the saved `mediaStream.selectedVideoSources` list.
 *
 * @param {string[]} selectedIds
 * @param {string} deviceId
 * @returns {number | null} null when the camera is not in the saved order
 */
export function cameraIndexForDevice(selectedIds, deviceId) {
  if (!Array.isArray(selectedIds) || typeof deviceId !== "string") {
    return null;
  }

  if (deviceId.length === 0) {
    return null;
  }

  const index = selectedIds.indexOf(deviceId);

  return index === -1 ? null : index + 1;
}

/**
 * Build the `media/start-camera` payload.
 *
 * `index` is the renderer's saved order. The engine echoes it back so a
 * mismatch is visible (see {@link reconcileCameraIndex}); sending it means the
 * engine can honour the renderer's order even though the source ids are opaque
 * to it.
 *
 * @param {{deviceId?: string, cameraId?: string, selectedIds?: string[], resolution?: {width?: number, height?: number}, frameRate?: number}} options
 * @returns {{sourceId: string, cameraId: string, width: number, height: number, fps: number, index?: number} | null}
 */
export function buildCameraStartRequest({
  deviceId,
  cameraId,
  selectedIds,
  resolution,
  frameRate,
} = {}) {
  const sourceId = cameraSourceId(deviceId);

  if (!sourceId) {
    return null;
  }

  const request = {
    sourceId,
    cameraId:
      typeof cameraId === "string" && cameraId.length ? cameraId : deviceId,
    // 0 means "engine default" on both sides (`VideoSourceConfig::normalised`).
    width: positiveDimension(resolution?.width),
    height: positiveDimension(resolution?.height),
    fps: positiveDimension(frameRate),
  };

  const index = cameraIndexForDevice(selectedIds, deviceId);

  if (index !== null) {
    request.index = index;
  }

  return request;
}

/**
 * Compare the order the renderer asked for with the one the engine reported.
 * A missing engine index is "unknown", not a mismatch.
 *
 * @param {{request?: {cameraId?: string, index?: number}, response?: {index?: number}}} options
 * @returns {{ok: boolean, expected: number | null, actual: number | null}}
 */
export function reconcileCameraIndex({ request, response } = {}) {
  const expected = Number.isFinite(request?.index) ? request.index : null;
  const actual = Number.isFinite(response?.index) ? response.index : null;

  return {
    ok: expected === null || actual === null || expected === actual,
    expected,
    actual,
  };
}

// -- video files -----------------------------------------------------------

/**
 * Whether `filePath` is already absolute (POSIX, or a Windows drive/UNC path).
 * @param {string} filePath
 * @returns {boolean}
 */
function isAbsolutePath(filePath) {
  return /^([a-zA-Z]:[\\/]|\\\\|\/)/.test(filePath);
}

/**
 * Resolve a media-store path (`default/video/clip.mp4`, relative to the media
 * directory) into the absolute path the engine's `media/start-file` wants.
 *
 * `VideoSourceConfig::file` accepts either a path or a `file://` URI
 * (`modv-media/src/video/pipeline.rs::path_to_file_uri`), so the renderer sends
 * the plain absolute path and lets the engine do the encoding.
 *
 * @param {string | null | undefined} mediaPath `media/path` from the store
 * @param {string | null | undefined} filePath the media item's `path`
 * @param {string} [platform] `process.platform` (injectable for tests)
 * @returns {string | null}
 */
export function toEngineMediaPath(mediaPath, filePath, platform = "linux") {
  if (typeof filePath !== "string") {
    return null;
  }

  const trimmed = filePath.trim();

  if (trimmed.length === 0) {
    return null;
  }

  const windows = platform === "win32";
  const separator = windows ? "\\" : "/";

  if (isAbsolutePath(trimmed)) {
    // A bare root ("/", "C:\\") is not a usable video path.
    return /^[\\/]+$/.test(trimmed) ? null : trimmed;
  }

  if (typeof mediaPath !== "string" || mediaPath.trim().length === 0) {
    return null;
  }

  const segments = trimmed.split(/[\\/]+/).filter((segment) => segment.length);

  if (segments.length === 0) {
    return null;
  }

  const root = mediaPath.replace(/[\\/]+$/, "");

  return `${root}${separator}${segments.join(separator)}`;
}

/**
 * Stable engine source id for a video texture.
 * @param {string} key texture id (or absolute path) identifying the source
 * @returns {string | null}
 */
export function videoFileSourceId(key) {
  if (typeof key !== "string" || key.length === 0) {
    return null;
  }

  return key.startsWith(FILE_PREFIX) ? key : `${FILE_PREFIX}${key}`;
}

/**
 * Build the `media/start-file` payload for a video texture.
 *
 * @param {{path?: string, mediaPath?: string, textureId?: string, sourceId?: string, resolution?: {width?: number, height?: number}, frameRate?: number, loop?: boolean, platform?: string}} options
 * @returns {{sourceId: string, path: string, width: number, height: number, fps: number, loop: boolean} | null}
 */
export function buildFileStartRequest({
  path,
  mediaPath,
  textureId,
  sourceId,
  resolution,
  frameRate,
  loop,
  platform,
} = {}) {
  const absolutePath = toEngineMediaPath(mediaPath, path, platform);

  if (!absolutePath) {
    return null;
  }

  const id = videoFileSourceId(
    typeof sourceId === "string" && sourceId.length
      ? sourceId
      : textureId || absolutePath,
  );

  if (!id) {
    return null;
  }

  return {
    sourceId: id,
    path: absolutePath,
    width: positiveDimension(resolution?.width),
    height: positiveDimension(resolution?.height),
    fps: positiveDimension(frameRate),
    // The legacy `<video>` element looped; keep that default.
    loop: loop === undefined ? true : !!loop,
  };
}

// -- screen shares ---------------------------------------------------------

/**
 * Stable engine source id for a PipeWire node.
 * @param {number} nodeId
 * @returns {string | null}
 */
export function screenSourceId(nodeId) {
  return typeof nodeId === "number" && isPositiveInteger(nodeId)
    ? `${SCREEN_PREFIX}${nodeId}`
    : null;
}

/**
 * Build the `media/start-screen` payload.
 * @param {{nodeId?: number}} options
 * @returns {{sourceId: string, nodeId: number} | null}
 */
export function buildScreenStartRequest({ nodeId } = {}) {
  const sourceId = screenSourceId(nodeId);

  return sourceId ? { sourceId, nodeId } : null;
}

/**
 * Extract a PipeWire node id from a `desktopCapturer` source *if* Electron ever
 * exposes one.
 *
 * Electron 45 does not: `desktopCapturer.getSources()` only yields Chromium's
 * `screen:<display_id>:<n>` / `window:<window_id>:<n>` ids and the
 * xdg-desktop-portal `ScreenCast` node id never reaches the app (and
 * `setDisplayMediaRequestHandler(..., { useSystemPicker: true })` hands the
 * picker to the portal entirely). Screen capture in engine mode therefore
 * reports "not available yet" rather than capturing with the renderer.
 *
 * @param {{nodeId?: number, pipewireNodeId?: number | string, id?: string} | null} source
 * @returns {number | null}
 */
export function screenNodeIdFromSource(source) {
  if (!source || typeof source !== "object") {
    return null;
  }

  for (const candidate of [source.nodeId, source.pipewireNodeId]) {
    const nodeId = Number(candidate);

    if (isPositiveInteger(nodeId)) {
      return nodeId;
    }
  }

  return null;
}

// -- media/status ----------------------------------------------------------

function errorText(value) {
  if (value === undefined || value === null || value === "") {
    return null;
  }

  if (typeof value === "string") {
    return value;
  }

  if (typeof value === "object") {
    if (typeof value.message === "string" && value.message.length) {
      return value.message;
    }

    try {
      return JSON.stringify(value);
    } catch (_error) {
      return String(value);
    }
  }

  return String(value);
}

function finiteNumber(value, fallback) {
  const number = Number(value);

  return Number.isFinite(number) ? number : fallback;
}

/**
 * Normalise one `media/status` source record. Unknown fields are dropped so the
 * UI never renders `undefined`.
 *
 * @param {object} source
 * @returns {{sourceId: string, kind: string, index: number | null, state: string, width: number, height: number, frames: number, dropped: number, lastError: string | null} | null}
 */
export function normaliseMediaSource(source) {
  if (!source || typeof source !== "object") {
    return null;
  }

  const sourceId = typeof source.sourceId === "string" ? source.sourceId : "";

  if (sourceId.length === 0) {
    return null;
  }

  const index = Math.round(Number(source.index));

  return {
    sourceId,
    kind: typeof source.kind === "string" ? source.kind : "",
    index: Number.isFinite(index) && index > 0 ? index : null,
    state: typeof source.state === "string" ? source.state : "",
    width: positiveDimension(source.width),
    height: positiveDimension(source.height),
    frames: Math.max(0, Math.floor(finiteNumber(source.frames, 0))),
    dropped: Math.max(0, Math.floor(finiteNumber(source.dropped, 0))),
    lastError: errorText(source.lastError),
  };
}

/**
 * Normalise a `media/status` reply.
 * @param {{sources?: Array, error?: string} | Array | null | undefined} response
 * @returns {{sources: Array<ReturnType<typeof normaliseMediaSource>>, error: string | null}}
 */
export function normaliseMediaStatus(response) {
  const list = Array.isArray(response)
    ? response
    : Array.isArray(response?.sources)
      ? response.sources
      : [];

  return {
    sources: list.map(normaliseMediaSource).filter(Boolean),
    error: errorText(response?.error),
  };
}

/**
 * The `media/status` record for a camera device, if the engine has one.
 * @param {{sources?: Array}} status
 * @param {string} deviceId
 */
export function sourceStatusForDevice(status, deviceId) {
  const sourceId = cameraSourceId(deviceId);

  if (!sourceId) {
    return null;
  }

  return (
    (Array.isArray(status?.sources) ? status.sources : []).find(
      (source) => source?.sourceId === sourceId,
    ) || null
  );
}

/**
 * Short label for a `media/status` state.
 * @param {string} state
 * @returns {string}
 */
export function mediaSourceStateLabel(state) {
  const key = typeof state === "string" ? state.toLowerCase() : "";

  return MEDIA_SOURCE_STATE_LABELS[key] || "Unknown";
}

/**
 * One-line status for the Audio/Video panel, e.g. `"Live 1280×720 · 3 dropped"`
 * or `"Error 1280×720 — device busy"`. Empty string without a source.
 *
 * @param {object} source raw or normalised `media/status` record
 * @returns {string}
 */
export function formatMediaSourceStatus(source) {
  const normalised = normaliseMediaSource(source);

  if (!normalised) {
    return "";
  }

  const size =
    normalised.width > 0 && normalised.height > 0
      ? ` ${normalised.width}×${normalised.height}`
      : "";

  let text = `${mediaSourceStateLabel(normalised.state)}${size}`;

  if (normalised.dropped > 0) {
    text += ` · ${normalised.dropped} dropped`;
  }

  if (normalised.lastError) {
    text += ` — ${normalised.lastError}`;
  }

  return text;
}

/**
 * {@link formatMediaSourceStatus} for a camera device.
 * @param {{sources?: Array}} status
 * @param {string} deviceId
 * @returns {string}
 */
export function formatDeviceStatus(status, deviceId) {
  return formatMediaSourceStatus(sourceStatusForDevice(status, deviceId));
}

// -- errors ----------------------------------------------------------------

/**
 * Parse a start/stop failure into its engine code and a readable message.
 *
 * `ipcRenderer.invoke` drops custom error fields, so `engine-ipc.js` folds the
 * code into the message (`"media_start_failed: device busy"`); the regex path
 * recovers it after the Electron wrapper text.
 *
 * @param {unknown} error
 * @returns {{code: string, message: string, hint: string}}
 */
export function mapEngineMediaError(error) {
  const declared = typeof error?.code === "string" ? error.code : null;
  const raw =
    typeof error === "string"
      ? error
      : typeof error?.message === "string"
        ? error.message
        : "";

  let code = declared && KNOWN_ERROR_CODES.includes(declared) ? declared : null;

  if (!code) {
    const match = /(media_[a-z_]+)/.exec(raw);
    code =
      match && KNOWN_ERROR_CODES.includes(match[1])
        ? match[1]
        : "media_start_failed";
  }

  const message = raw
    .replace(/^Error invoking remote method '[^']*':\s*/, "")
    .replace(/^Error:\s*/, "")
    .replace(/^media_[a-z_]+:\s*/, "");

  return {
    code,
    message: message.length ? message : "unknown error",
    hint: ENGINE_MEDIA_ERROR_HINTS[code],
  };
}

/**
 * User-facing one-liner for a start failure.
 * @param {unknown} error
 * @returns {string}
 */
export function describeEngineMediaError(error) {
  const { hint, message } = mapEngineMediaError(error);

  return message === "unknown error" ? hint : `${hint}: ${message}`;
}

/**
 * Notice text for a source that failed to start. The controller stores this
 * once per source, so a 1 Hz status poll cannot spam the panel.
 *
 * @param {{name?: string, error?: unknown}} options
 * @returns {string}
 */
export function engineMediaFailureNotice({ name, error } = {}) {
  const label = typeof name === "string" && name.length ? name : "video input";

  return `Could not start ${label} — ${describeEngineMediaError(error)}`;
}

// -- reconciliation --------------------------------------------------------

function sourceIsDead(source) {
  if (!source) {
    return true;
  }

  const state =
    typeof source.state === "string" ? source.state.toLowerCase() : "";

  return state === "error" || state === "failed" || state === "stopped";
}

/**
 * Decide what one status poll should do with the sources this renderer
 * started:
 *
 * - **stop** a camera whose device is no longer available or was unchecked;
 * - **restart** a selected camera that reported `error`/`stopped`, vanished
 *   from the engine's status (an engine restart drops every source), or whose
 *   first start failed — but only after `retryIntervalMs` since the last
 *   attempt, so a busy device is not hammered.
 *
 * Files and screen shares are UI-driven and never reconciled here.
 *
 * @param {{tracked?: object, status?: object, selectedIds?: string[], availableIds?: string[] | null, nowMs?: number, lastAttempt?: object, retryIntervalMs?: number}} options
 * @returns {{stopSources: string[], restartDevices: string[]}}
 */
export function reconcileEngineSources({
  tracked,
  status,
  selectedIds,
  availableIds,
  nowMs = Date.now(),
  lastAttempt = {},
  retryIntervalMs = ENGINE_MEDIA_RESTART_INTERVAL_MS,
} = {}) {
  const selected = new Set(Array.isArray(selectedIds) ? selectedIds : []);
  const available = Array.isArray(availableIds) ? new Set(availableIds) : null;
  const sources = new Map(
    (Array.isArray(status?.sources) ? status.sources : [])
      .filter(Boolean)
      .map((source) => [source.sourceId, source]),
  );
  const retry = Number.isFinite(retryIntervalMs)
    ? retryIntervalMs
    : ENGINE_MEDIA_RESTART_INTERVAL_MS;

  const stopSources = [];
  const restartDevices = [];
  const entries =
    tracked instanceof Map ? tracked.entries() : Object.entries(tracked || {});

  for (const [sourceId, entry] of entries) {
    if (entry?.kind !== KIND_CAMERA) {
      continue;
    }

    const deviceId = entry.deviceId;

    if (
      typeof deviceId !== "string" ||
      deviceId.length === 0 ||
      !selected.has(deviceId) ||
      (available && !available.has(deviceId))
    ) {
      stopSources.push(sourceId);
      continue;
    }

    if (!sourceIsDead(sources.get(sourceId))) {
      continue;
    }

    const last = Number(lastAttempt?.[deviceId]);
    const attemptedAt = Number.isFinite(last) ? last : 0;

    if (nowMs - attemptedAt >= retry) {
      restartDevices.push(deviceId);
    }
  }

  return { stopSources, restartDevices };
}

// -- controller ------------------------------------------------------------

/**
 * The renderer's engine media session: everything it started, the notices it
 * has shown and the status it last saw.
 *
 * All methods are no-ops (or resolve to null) when engine capture is disabled,
 * so callers never need to branch on `engineMode` around them.
 *
 * @param {{engine?: object, mirrorStore?: object, getStore?: () => object, logger?: object, now?: () => number}} options
 */
export function createEngineMediaController({
  engine,
  mirrorStore,
  getStore,
  logger,
  now = () => Date.now(),
} = {}) {
  const surface =
    engine === undefined
      ? typeof window !== "undefined"
        ? window.engine
        : undefined
      : engine;
  const log = logger || { info() {}, warn() {}, error() {} };
  const enabled = isEngineMediaEnabled(surface);

  /** Cameras from the last {@link listCameras} call (devices, not raw DTOs). */
  let cameras = [];

  /** sourceId → { kind, deviceId, request, resolution, frameRate } */
  const tracked = new Map();
  /** sourceId → notice text (one per source, no spam) */
  const notices = new Map();
  /** deviceId → timestamp of the last start attempt */
  const lastAttempt = {};

  function store() {
    return typeof getStore === "function" ? getStore() : mirrorStore;
  }

  function mirrorCommit(type, payload) {
    const target = mirrorStore || store()?._store || store();

    try {
      target?.commit?.(type, payload);
    } catch (error) {
      log.warn("could not commit to the mirror store", {
        type,
        message: error?.message || String(error),
      });
    }
  }

  function recordNotice(sourceId, message) {
    if (!sourceId || !message) {
      return message || null;
    }

    if (notices.has(sourceId)) {
      return notices.get(sourceId);
    }

    notices.set(sourceId, message);
    log.warn("engine media source failed", { sourceId, message });

    return message;
  }

  async function stopSource(sourceId) {
    if (!sourceId) {
      return;
    }

    tracked.delete(sourceId);
    notices.delete(sourceId);

    if (!enabled || typeof surface?.stopEngineSource !== "function") {
      return;
    }

    try {
      await surface.stopEngineSource(sourceId);
    } catch (error) {
      log.warn("could not stop the engine media source", {
        sourceId,
        message: error?.message || String(error),
      });
    }
  }

  async function startCamera({
    deviceId,
    cameraId,
    selectedIds,
    resolution,
    frameRate,
  } = {}) {
    if (!enabled) {
      return null;
    }

    const request = buildCameraStartRequest({
      deviceId,
      cameraId,
      selectedIds,
      resolution,
      frameRate,
    });

    if (!request) {
      return null;
    }

    lastAttempt[deviceId] = now();

    try {
      const response = await surface.startEngineCamera(request);

      tracked.set(request.sourceId, {
        kind: KIND_CAMERA,
        deviceId,
        request,
        resolution,
        frameRate,
      });
      notices.delete(request.sourceId);
      recordAppliedSettings({
        deviceId,
        width: resolution?.width,
        height: resolution?.height,
        fps: frameRate,
      });

      const indexCheck = reconcileCameraIndex({ request, response });

      if (!indexCheck.ok) {
        log.warn("the engine assigned a different camera index", {
          deviceId,
          ...indexCheck,
        });
      }

      return response && typeof response === "object"
        ? response
        : { sourceId: request.sourceId, index: request.index ?? null };
    } catch (error) {
      recordNotice(
        request.sourceId,
        engineMediaFailureNotice({ name: cameraId || deviceId, error }),
      );

      throw error;
    }
  }

  async function stopCamera(deviceId) {
    await stopSource(cameraSourceId(deviceId));
  }

  async function startVideoFile({
    path,
    mediaPath,
    textureId,
    sourceId,
    resolution,
    frameRate,
    loop,
    previousSourceId,
    platform,
  } = {}) {
    if (!enabled) {
      return null;
    }

    const request = buildFileStartRequest({
      path,
      mediaPath,
      textureId,
      sourceId,
      resolution,
      frameRate,
      loop,
      platform,
    });

    if (!request) {
      return null;
    }

    if (previousSourceId && previousSourceId !== request.sourceId) {
      await stopSource(previousSourceId);
    }

    try {
      const response = await surface.startEngineFile(request);

      tracked.set(request.sourceId, {
        kind: KIND_FILE,
        deviceId: null,
        request,
      });
      notices.delete(request.sourceId);

      return response && typeof response === "object"
        ? response
        : { sourceId: request.sourceId, index: null };
    } catch (error) {
      recordNotice(
        request.sourceId,
        engineMediaFailureNotice({ name: path, error }),
      );

      throw error;
    }
  }

  async function startScreen({ nodeId, previousSourceId } = {}) {
    if (!enabled) {
      return null;
    }

    const request = buildScreenStartRequest({ nodeId });

    if (!request) {
      return null;
    }

    if (previousSourceId && previousSourceId !== request.sourceId) {
      await stopSource(previousSourceId);
    }

    try {
      const response = await surface.startEngineScreen(request);

      tracked.set(request.sourceId, {
        kind: KIND_SCREEN,
        deviceId: null,
        request,
      });
      notices.delete(request.sourceId);

      return response && typeof response === "object"
        ? response
        : { sourceId: request.sourceId, index: null };
    } catch (error) {
      recordNotice(
        request.sourceId,
        engineMediaFailureNotice({ name: `screen ${nodeId}`, error }),
      );

      throw error;
    }
  }

  async function stopAll() {
    for (const sourceId of [...tracked.keys()]) {
      await stopSource(sourceId);
    }
  }

  async function status() {
    if (!enabled || typeof surface?.engineMediaStatus !== "function") {
      return { sources: [], error: null };
    }

    try {
      return normaliseMediaStatus(await surface.engineMediaStatus());
    } catch (error) {
      const described = describeEngineMediaError(error);

      log.warn("could not read the engine media status", {
        message: described,
      });

      return { sources: [], error: described };
    }
  }

  /**
   * (Re)start the selected cameras that are available but not running.
   *
   * Used by a device re-scan: `pollStatus` only reconciles sources the renderer
   * already tracks, so a camera whose very first start failed (or that was
   * plugged back in) needs an explicit attempt.
   *
   * @param {{selectedIds?: string[], availableIds?: string[] | null}} options
   * @returns {Promise<string[]>} the devices that were (re)started
   */
  async function ensureCameras({ selectedIds, availableIds } = {}) {
    if (!enabled) {
      return [];
    }

    const available = Array.isArray(availableIds)
      ? new Set(availableIds)
      : null;
    const started = [];

    for (const deviceId of Array.isArray(selectedIds) ? selectedIds : []) {
      const sourceId = cameraSourceId(deviceId);

      if (!sourceId || tracked.has(sourceId)) {
        continue;
      }

      if (available && !available.has(deviceId)) {
        continue;
      }

      try {
        await startCamera({ deviceId, selectedIds });
        started.push(deviceId);
      } catch (_error) {
        // The failure recorded its own notice; the panel shows it.
      }
    }

    return started;
  }

  /**
   * One status poll: refresh `media/status`, stop removed/unchecked cameras and
   * restart dead ones (with a backoff). Never throws.
   *
   * @param {{selectedIds?: string[], availableIds?: string[] | null, nowMs?: number}} options
   * @returns {Promise<{sources: Array, stopped: string[], restarted: string[], error: string | null}>}
   */
  async function pollStatus({ selectedIds, availableIds, nowMs } = {}) {
    const current = await status();

    if (current.error) {
      return {
        sources: [],
        stopped: [],
        restarted: [],
        error: current.error,
      };
    }

    const { stopSources, restartDevices } = reconcileEngineSources({
      tracked,
      status: current,
      selectedIds,
      availableIds,
      lastAttempt,
      nowMs: Number.isFinite(nowMs) ? nowMs : now(),
    });

    for (const sourceId of stopSources) {
      await stopSource(sourceId);
    }

    for (const deviceId of restartDevices) {
      const entry = [...tracked.values()].find(
        (candidate) =>
          candidate.kind === KIND_CAMERA && candidate.deviceId === deviceId,
      );

      try {
        await startCamera({
          deviceId,
          cameraId: entry?.request?.cameraId,
          selectedIds,
          resolution: entry?.resolution,
          frameRate: entry?.frameRate,
        });
      } catch (_error) {
        // The restart already recorded its own notice.
      }
    }

    return {
      sources: current.sources,
      stopped: stopSources,
      restarted: restartDevices,
      error: null,
    };
  }

  /**
   * Write the engine's applied camera order/settings into the mirror store so
   * the panel's "(active: …)" lines and the Webcam module's `camera` prop agree
   * with what the engine is actually capturing.
   *
   * @param {{deviceId?: string, width?: number, height?: number, fps?: number}} options
   */
  function recordAppliedSettings({ deviceId, width, height, fps } = {}) {
    if (!deviceId) {
      return;
    }

    mirrorCommit("mediaStream/SET_APPLIED_VIDEO_SETTINGS", {
      deviceId,
      width: positiveDimension(width) || null,
      height: positiveDimension(height) || null,
      frameRate: positiveDimension(fps) || null,
    });
  }

  return {
    isEnabled: () => enabled,
    get cameras() {
      return cameras;
    },
    listCameras,
    startCamera,
    stopCamera,
    ensureCameras,
    startVideoFile,
    startScreen,
    stopSource,
    stopAll,
    status,
    pollStatus,
    recordAppliedSettings,
    notices: () =>
      [...notices.entries()].map(([sourceId, message]) => ({
        sourceId,
        message,
      })),
    noticeFor: (sourceId) => notices.get(sourceId) || null,
    clearNotice: (sourceId) => notices.delete(sourceId),
    isTracked: (sourceId) => tracked.has(sourceId),
    trackedSources: () => [...tracked.keys()],
    /** Test/debug hook: the device ids the controller is currently holding. */
    trackedCameras: () =>
      [...tracked.values()]
        .filter((entry) => entry.kind === KIND_CAMERA)
        .map((entry) => entry.deviceId),
  };

  async function listCameras() {
    if (!enabled || typeof surface?.listCameras !== "function") {
      cameras = [];
      return cameras;
    }

    try {
      cameras = engineCameraDevices(await surface.listCameras());
    } catch (error) {
      cameras = [];
      log.warn("could not list the engine cameras", {
        message: error?.message || String(error),
      });
    }

    return cameras;
  }
}
