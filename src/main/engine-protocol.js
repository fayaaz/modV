// WS-14 — the client side of the engine control channel (see
// docs/rust-hybrid.md, “Protocol specification — control channel”).
//
// Framing: [ u32 LE payload length ][ u8 kind ][ payload bytes ]
//   kind 0 — JSON request   (Electron → engine)
//   kind 1 — JSON response  (engine → Electron)
//   kind 2 — JSON event     (engine → Electron)
//   kind 3 — binary         (reserved; rejected by message handlers)
//
// The renderer never sees this socket — it talks to `window.engine` (preload)
// which proxies over IPC to this class. Requests carry an auto-incremented id;
// responses resolve the matching pending call with the action's return value,
// reproducing the promise-worker semantics (worker/index.worker.js:564-582).

import { createConnection } from "node:net";
import { EventEmitter } from "node:events";

const KIND_REQUEST = 0;
const KIND_RESPONSE = 1;
const KIND_EVENT = 2;
const MAX_FRAME_BYTES = 64 * 1024 * 1024;

function encodeFrame(kind, payload) {
  const header = Buffer.allocUnsafe(5);
  header.writeUInt32LE(payload.length, 0);
  header.writeUInt8(kind, 4);
  return Buffer.concat([header, payload]);
}

class EngineProtocolClient extends EventEmitter {
  constructor() {
    super();
    this.socket = null;
    this.buffer = Buffer.alloc(0);
    this.nextId = 1;
    this.pending = new Map(); // id → { resolve, reject, timer }
    this.connected = false;
    this.previewInfo = null; // set by the preview/ready event
  }

  /**
   * Connect to the engine's Unix socket. Resolves once the connection is
   * established; rejects if the socket cannot be reached or the engine
   * disconnects before the connect completes.
   */
  connect(socketPath, timeoutMs = 2000) {
    return new Promise((resolve, reject) => {
      this.socketPath = socketPath;
      const socket = createConnection({ path: socketPath });
      this.socket = socket;

      const timer = setTimeout(() => {
        socket.destroy();
        reject(new Error(`timed out connecting to ${socketPath}`));
      }, timeoutMs);

      socket.once("connect", () => {
        clearTimeout(timer);
        this.connected = true;
        socket.on("data", (chunk) => this._onData(chunk));
        socket.on("close", () => this._onClose());
        socket.on("error", (error) => this._onError(error));
        this.emit("connected");
        resolve();
      });

      socket.once("error", (error) => {
        clearTimeout(timer);
        reject(error);
      });
    });
  }

  disconnect() {
    this.connected = false;
    if (this.socket) {
      try {
        this.socket.destroy();
      } catch (_) {
        // already dead
      }
      this.socket = null;
    }
    this._rejectAllPending(new Error("engine disconnected"));
  }

  /**
   * Request/response against the engine. `params` must be JSON-serialisable.
   * Rejects with the engine's error object (code/message) when
   * `ok: false`, or with an Error when the socket drops / times out.
   */
  request(method, params, timeoutMs = 5000) {
    if (!this.connected || !this.socket) {
      return Promise.reject(new Error("engine is not connected"));
    }
    const id = this.nextId++;
    const body = JSON.stringify({ id, method, params: params ?? null });
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`engine request "${method}" timed out after ${timeoutMs} ms`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.write(encodeFrame(KIND_REQUEST, Buffer.from(body, "utf8")));
    });
  }

  _onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    for (;;) {
      if (this.buffer.length < 5) return;
      const length = this.buffer.readUInt32LE(0);
      if (length > MAX_FRAME_BYTES) {
        this._failProtocol(`declared frame length ${length} exceeds the 64 MiB cap`);
        return;
      }
      if (this.buffer.length < 5 + length) return;
      const kind = this.buffer.readUInt8(4);
      const payload = this.buffer.subarray(5, 5 + length);
      this.buffer = this.buffer.subarray(5 + length);
      this._handleFrame(kind, payload);
    }
  }

  _handleFrame(kind, payload) {
    let message;
    try {
      message = JSON.parse(payload.toString("utf8"));
    } catch (error) {
      this._failProtocol(`engine sent invalid JSON (kind ${kind}): ${error.message}`);
      return;
    }

    if (kind === KIND_RESPONSE) {
      const pending = this.pending.get(message.id);
      if (!pending) return; // late or unsolicited; drop it
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.ok) {
        pending.resolve("result" in message ? message.result : null);
      } else {
        const engineError = message.error || {};
        const error = new Error(engineError.message || "engine request failed");
        error.code = engineError.code || "engine_error";
        pending.reject(error);
      }
      return;
    }

    if (kind === KIND_EVENT) {
      const { event, data } = message;
      if (event === "preview/ready") this.previewInfo = data;
      this.emit("engine-event", { event, data });
      if (typeof event === "string") this.emit(event, data);
      return;
    }

    this._failProtocol(`unexpected frame kind ${kind}`);
  }

  _onClose() {
    this.connected = false;
    this._rejectAllPending(new Error("engine disconnected"));
    this.emit("close");
  }

  _onError(error) {
    // socket 'error' is always followed by 'close'; log via the event so the
    // supervisor (and tests) can observe it.
    this.emit("socket-error", error);
  }

  _rejectAllPending(error) {
    for (const [, pending] of this.pending) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }

  _failProtocol(message) {
    this.emit("protocol-error", new Error(message));
    this.disconnect();
  }
}

export { EngineProtocolClient, encodeFrame, KIND_REQUEST, KIND_RESPONSE, KIND_EVENT, MAX_FRAME_BYTES };
