// WS-14 — renderer-side IPC client for the Rust engine.
//
// Mirrors the worker facade exactly (store.commit / store.dispatch) so the
// 59 Vue components are untouched, and replays `state/delta` events into the
// mirror store with the same {type, payload} records the worker's commitQueue
// produced (application/index.js:199-211 is replaced by this path).
//
// window.engine comes from the preload (src/preload/index.js) and fronts the
// main process's engine-protocol.js — the renderer never touches the socket.

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
    this._unsubscribe.state = this.engine.onEngineState(callback);
    return this._unsubscribe.state;
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
