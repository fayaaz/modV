// WS-10 — validation for the engine media channels.
//
// `engine-ipc.js` is the only place that writes to the engine's control socket
// and is bound to Electron, so the request building lives here: `ipcRenderer.
// invoke` arguments are untrusted renderer input and a malformed payload would
// otherwise reach GStreamer (a wrong path, a bogus node id, a missing camera).
//
// Keep this dependency-free so `tests/unit/engine-media-request.test.js` can
// drive it in plain Node.
//
// Error propagation note: `ipcRenderer.invoke` rejects with an `Error` whose
// custom properties are dropped, so the engine's `media_*` code is folded into
// the message (`"media_start_failed: device busy"`) and recovered in the
// renderer by `mapEngineMediaError`.

/** Channel → engine method, timeout and payload builder. */
const CHANNELS = new Map([
  [
    "engine/media-list",
    { command: "media/list-cameras", timeoutMs: 8000, build: () => null },
  ],
  [
    "engine/media-start-camera",
    { command: "media/start-camera", timeoutMs: 15000, build: cameraPayload },
  ],
  [
    "engine/media-start-file",
    { command: "media/start-file", timeoutMs: 15000, build: filePayload },
  ],
  [
    "engine/media-start-screen",
    { command: "media/start-screen", timeoutMs: 15000, build: screenPayload },
  ],
  [
    "engine/media-stop",
    { command: "media/stop-source", timeoutMs: 8000, build: stopPayload },
  ],
  // Status is polled at ~1 Hz from the Audio/Video panel, so a short timeout
  // keeps a wedged engine from queueing polls behind a stalled request.
  [
    "engine/media-status",
    { command: "media/status", timeoutMs: 4000, build: () => null },
  ],
]);

/** The channels `registerEngineIpc` handles, in registration order. */
const ENGINE_MEDIA_CHANNEL_NAMES = [...CHANNELS.keys()];

function requiredString(message, field) {
  const value = message?.[field];

  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }

  return value;
}

function dimension(value) {
  const number = Math.round(Number(value));

  return Number.isFinite(number) && number > 0 ? number : 0;
}

function cameraPayload(message = {}) {
  const payload = {
    sourceId: requiredString(message, "sourceId"),
    cameraId: requiredString(message, "cameraId"),
    width: dimension(message.width),
    height: dimension(message.height),
    fps: dimension(message.fps),
  };

  const index = Math.round(Number(message.index));
  if (Number.isFinite(index) && index > 0) {
    payload.index = index;
  }

  return payload;
}

function filePayload(message = {}) {
  return {
    sourceId: requiredString(message, "sourceId"),
    path: requiredString(message, "path"),
    width: dimension(message.width),
    height: dimension(message.height),
    fps: dimension(message.fps),
    // The legacy <video> element looped, so keep that behaviour unless the
    // caller says otherwise.
    loop: message.loop === undefined ? true : !!message.loop,
  };
}

function screenPayload(message = {}) {
  const nodeId = message?.nodeId;

  if (typeof nodeId !== "number" || !Number.isInteger(nodeId) || nodeId <= 0) {
    throw new Error("nodeId must be a positive integer PipeWire node id");
  }

  return {
    sourceId: requiredString(message, "sourceId"),
    nodeId,
  };
}

function stopPayload(message = {}) {
  return { sourceId: requiredString(message, "sourceId") };
}

/**
 * Turn one `engine/media-*` invoke into the engine request it performs.
 *
 * @param {string} channel e.g. `"engine/media-start-camera"`
 * @param {object} [message] the renderer's invoke argument
 * @returns {{ command: string, payload: object | null, timeoutMs: number }}
 * @throws {Error} for an unknown channel or an invalid payload
 */
function buildEngineMediaRequest(channel, message) {
  const entry = CHANNELS.get(channel);

  if (!entry) {
    throw new Error(`unknown engine media channel "${channel}"`);
  }

  return {
    command: entry.command,
    payload: entry.build(message),
    timeoutMs: entry.timeoutMs,
  };
}

/**
 * The timeout `buildEngineMediaRequest` uses for `channel` (0 for unknown).
 * @param {string} channel
 * @returns {number}
 */
function mediaRequestTimeoutMs(channel) {
  return CHANNELS.get(channel)?.timeoutMs ?? 0;
}

/** Engine codes the renderer surfaces to the user. */
const MEDIA_ERROR_CODES = new Set([
  "media_unavailable",
  "media_plugin_missing",
  "media_start_failed",
]);

/**
 * Normalise a rejected engine request into an `Error` whose message starts with
 * a `media_*` code, so the renderer can map it after `ipcRenderer.invoke` has
 * dropped the custom fields.
 *
 * @param {unknown} error the rejection from `EngineProtocolClient.request`
 * @param {string} command the engine method that failed
 * @returns {Error}
 */
function mediaInvokeError(error, command) {
  const raw = error instanceof Error ? error : new Error(String(error));
  const message = raw.message || "engine media request failed";
  const code = MEDIA_ERROR_CODES.has(raw.code)
    ? raw.code
    : command === "media/list-cameras" || command === "media/status"
      ? "media_unavailable"
      : "media_start_failed";

  // Already coded (a nested handler) — do not stack prefixes.
  if (/^media_[a-z_]+:\s/.test(message)) {
    return new Error(message);
  }

  return new Error(`${code}: ${message}`);
}

export {
  buildEngineMediaRequest,
  ENGINE_MEDIA_CHANNEL_NAMES,
  MEDIA_ERROR_CODES,
  mediaInvokeError,
  mediaRequestTimeoutMs,
};
