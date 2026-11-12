<template>
  <grid
    ref="container"
    v-searchTerms="{
      terms: ['preview', 'canvas', 'debugger'],
      title: 'Canvas Debugger',
      type: 'Panel',
    }"
    class="preview"
  >
    <!--
      The output picker lists the worker's auxillary (debug) canvases. Engine
      mode has no worker graph to debug — the panel always shows the engine's
      composited main output — so the picker is hidden there.
    -->
    <c v-if="!engineMode" span="1..">
      <Select v-model="debugId" class="light">
        <option value="main">Main Output</option>
        <optgroup
          v-for="(outputs, group) in groups"
          :key="group"
          :label="group"
        >
          <option v-for="output in outputs" :key="output.id" :value="output.id">
            {{ output.name }}
          </option>
        </optgroup>
      </Select>
    </c>

    <c span="1.." class="canvas-container">
      <canvas ref="canvas" :class="{ letterboxed: engineMode }"></canvas>
    </c>
  </grid>
</template>

<script>
import { shouldRefresh } from "../application/engine-client.js";

// Engine mode: the engine composites and publishes frames; this panel pulls the
// latest one and paints it into its own plain 2D canvas. That is a display
// copy, not GPU work — engine mode deliberately creates no WebGL/WebGPU
// context. 15 fps is plenty for a debug panel and keeps the shm read cheap.
const ENGINE_PREVIEW_FPS = 15;
// While the engine has no frame at all (still starting, ring not published),
// retry on a slow timer instead of waiting for a preview event that never comes.
const ENGINE_PREVIEW_RETRY_MS = 500;

export default {
  data() {
    return {
      resizeObserver: null,
    };
  },

  computed: {
    engineMode() {
      return this.$modV?.engineMode === true;
    },

    auxillaries() {
      return this.$modV.store.state.outputs.auxillary;
    },

    groups() {
      const groups = {};
      const auxValues = Object.values(this.auxillaries);
      for (let i = 0, len = auxValues.length; i < len; i++) {
        const aux = auxValues[i];

        if (!groups[aux.group]) {
          groups[aux.group] = {};
        }

        groups[aux.group][aux.id] = aux;
      }

      return groups;
    },

    debugId: {
      get() {
        return this.$modV.store.state.outputs.debugId;
      },

      set(value) {
        this.$modV.store.commit("outputs/SET_DEBUG_ID", value);
      },
    },
  },

  mounted() {
    if (this.engineMode) {
      this.startEnginePreview();
      return;
    }

    if (!this.$modV.$worker) {
      return;
    }

    const { canvas } = this.$refs;
    const offscreen = canvas.transferControlToOffscreen();

    this.$modV.$worker.postMessage(
      {
        type: "dispatch",
        identifier: "outputs/setDebugContext",
        payload: offscreen,
      },
      [offscreen],
    );

    this.resizeObserver = new ResizeObserver((entries) => {
      requestAnimationFrame(() => {
        if (!Array.isArray(entries) || !entries.length) {
          return;
        }

        this.resize(entries);
      });
    }).observe(canvas);

    this.$modV.store.commit("outputs/TOGGLE_DEBUG", true);
  },

  beforeUnmount() {
    // Legacy mode left the (worker-owned) debug canvas to the worker; only the
    // engine-mode frame pull holds renderer-side listeners and rAF work.
    this.stopEnginePreview();
  },

  methods: {
    /**
     * Engine mode: wire the preview panel to the engine's frame stream.
     *
     * The engine emits `preview/frame` per composited frame; those events are
     * the clock, but they only schedule a rAF tick and the tick pulls at most
     * ENGINE_PREVIEW_FPS frames a second. `requestOutputPreview` reads the
     * latest ring slot in the main process and resolves to
     * `{ width, height, data }`, painted with `putImageData`.
     */
    startEnginePreview() {
      const { canvas } = this.$refs;
      const engine = window.engine;
      const state = {
        ctx: canvas.getContext("2d"),
        raf: null,
        retryTimer: null,
        inFlight: false,
        // null → nothing pulled yet, so the first tick is due.
        lastPullMs: null,
        unsubscribeFrame: null,
        unsubscribeState: null,
      };

      this._enginePreview = state;

      state.unsubscribeFrame = engine.onPreviewFrame?.(() => {
        this.scheduleEnginePreview();
      });

      // A restart rebuilds the ring, so the previous frame is stale: force the
      // next tick to pull instead of waiting out the throttle.
      state.unsubscribeState = engine.onEngineState?.((transition) => {
        if (transition?.state !== "running") {
          return;
        }

        state.lastPullMs = null;
        this.scheduleEnginePreview();
      });

      this.scheduleEnginePreview();
    },

    stopEnginePreview() {
      const state = this._enginePreview;
      if (!state) {
        return;
      }

      this._enginePreview = null;

      if (state.raf !== null) {
        window.cancelAnimationFrame(state.raf);
        state.raf = null;
      }

      if (state.retryTimer !== null) {
        clearTimeout(state.retryTimer);
        state.retryTimer = null;
      }

      state.unsubscribeFrame?.();
      state.unsubscribeState?.();
      state.unsubscribeFrame = null;
      state.unsubscribeState = null;
    },

    /** Coalesce bursts of preview events into one rAF tick. */
    scheduleEnginePreview() {
      const state = this._enginePreview;
      if (!state || state.raf !== null) {
        return;
      }

      state.raf = window.requestAnimationFrame(() => {
        state.raf = null;
        void this.pullEnginePreview();
      });
    },

    async pullEnginePreview() {
      const state = this._enginePreview;
      if (!state || state.inFlight) {
        return;
      }

      // Only pull while the panel is actually on screen and the app is visible.
      const { canvas } = this.$refs;
      if (document.hidden || !canvas?.isConnected) {
        return;
      }

      const now = performance.now();
      if (!shouldRefresh(now, state.lastPullMs, ENGINE_PREVIEW_FPS)) {
        return;
      }

      state.lastPullMs = now;
      state.inFlight = true;

      try {
        const frame = await this.$modV.requestOutputPreview();
        // Unmounted (or restarted) while the request was in flight.
        if (this._enginePreview !== state) {
          return;
        }

        if (!frame) {
          this.retryEnginePreview(state);
          return;
        }

        this.drawEnginePreviewFrame(state, frame);
      } finally {
        state.inFlight = false;
      }
    },

    /** Slow retry while the engine has not produced any frame yet. */
    retryEnginePreview(state) {
      if (this._enginePreview !== state || state.retryTimer !== null) {
        return;
      }

      state.retryTimer = setTimeout(() => {
        state.retryTimer = null;
        this.scheduleEnginePreview();
      }, ENGINE_PREVIEW_RETRY_MS);
    },

    /** Paint an engine frame into the panel's own 2D canvas, letterboxed by CSS. */
    drawEnginePreviewFrame(state, { width, height, data }) {
      const { canvas } = this.$refs;
      if (!canvas || data.length !== width * height * 4) {
        return;
      }

      // Match the canvas backing store to the frame so putImageData is 1:1;
      // `object-fit: contain` letterboxes the result inside the panel.
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }

      const ctx = state.ctx || canvas.getContext("2d");
      state.ctx = ctx;
      ctx.putImageData(new ImageData(data, width, height), 0, 0);
    },

    resize(e) {
      if (!this.$modV.$worker) {
        return;
      }

      const { width } = e[0].contentRect;

      this.$modV.$worker.postMessage({
        type: "dispatch",
        identifier: "outputs/resizeDebug",
        payload: { width: width * window.devicePixelRatio },
      });

      // this.$refs.canvas.style.width = `${e[0].contentRect.width}px`;
      // this.$refs.canvas.style.height = `${e[0].contentRect.height}px`;
    },
  },
};
</script>

<style scoped>
.preview {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  height: 100%;
}

.canvas-container {
  height: 100%;
  flex-grow: 1;
  min-height: 0;
}

canvas {
  object-fit: contain;
  width: 100%;
  height: 100%;
}

/*
  Engine mode sizes the canvas backing store to the engine frame (320×180), so
  `object-fit: contain` keeps the aspect ratio and letterboxes it, the same way
  the worker's debug canvas did.
*/
canvas.letterboxed {
  display: block;
  background: #000;
}
</style>
