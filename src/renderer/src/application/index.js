import PromiseWorker from "promise-worker-transferable";
import { reactive } from "vue";
import { createWebcodecVideo } from "./createWebcodecVideo";

const { app } = window.remote;
const { ipcRenderer } = window.electron;

import {
  setupMedia,
  getFloatFrequencyData,
  getByteFrequencyData,
  getByteTimeDomainData,
} from "./setup-media";
import { AudioBands, isBandFeature } from "./audio-bands";
import { BandFeatures } from "./band-features";
import { reduceByteFrequencyData } from "./byte-frequency-data";
import { createLogger } from "./logger";
import MediaDeviceManager from "./media-device-manager";
import {
  SystemAudioCapture,
  SYSTEM_AUDIO_SOURCE_ID,
  isSystemAudioSource,
} from "./system-audio";
import {
  describeCaptureError,
  nextVideoCaptureAction,
  restoreAudioSource,
  restoreVideoSources,
  shouldReconnectVideoCapture,
} from "./media-inputs";
import setupBeatDetektor from "./setup-beat-detektor";
import setupMidi from "./setup-midi";
import store from "./worker/store";
import windowHandler from "./window-handler";
import use from "./use";
import constants, { GROUP_ENABLED } from "./constants";
import ModVWorker from "./worker/index.worker.js?worker";
import EngineClient from "./engine-client";
import { createPresetData } from "./worker/preset-serializer";

const logger = createLogger("renderer:app");

const VIDEO_CAPTURE_RETRY_DELAY_MS = 250;

if (typeof window !== "undefined") {
  window.addEventListener("error", (event) => {
    logger.error("uncaught error in the renderer", {
      message: event?.message,
      source: event?.filename,
      line: event?.lineno,
      column: event?.colno,
      stack: event?.error?.stack,
    });
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event?.reason;
    const data =
      reason instanceof Error
        ? {
            name: reason.name,
            message: reason.message,
            stack: reason.stack,
          }
        : reason === undefined
          ? { reason: "undefined", promise: String(event?.promise) }
          : { reason };

    logger.error("unhandled rejection in the renderer", data);
  });
}

// deprecated single-capture buffer kept for compatibility; unused in async pipeline
// let imageBitmap;
// const imageBitmapQueue = [];

class ModV {
  // Support multiple ImageCapture instances: deviceId -> ImageCapture
  // (initialized in constructor for parser compatibility)

  constructor() {
    let resolver = null;
    this.ready = new Promise((resolve) => {
      resolver = resolve;
    });
    this._resolveReady = resolver;
    // Bind subsystems and defaults
    this.setupMedia = setupMedia;
    this.mediaDeviceManager = new MediaDeviceManager();
    this.systemAudioCapture = new SystemAudioCapture();
    this.setupBeatDetektor = setupBeatDetektor;
    this.setupMidi = setupMidi;
    this.windowHandler = windowHandler;
    this.createWebcodecVideo = createWebcodecVideo;
    this.use = use;
    this.debug = false;
    this.audioBands = new AudioBands();
    this.bandFeatures = new BandFeatures();
    this.features = reactive({
      energy: 0,
      rms: 0,
      zcr: 0,
      bass: 0,
      mid: 0,
      treble: 0,
      spectralCentroid: 0,
      spectralFlatness: 0,
      spectralSlope: 0,
      spectralRolloff: 0,
      spectralSpread: 0,
      spectralSkewness: 0,
      spectralKurtosis: 0,
      perceptualSpread: 0,
      perceptualSharpness: 0,
    });
    this.videos = {};
    this._store = store;
    this.store = {
      state: store.state,
    };
    // Initialize capture structures
    this._imageCaptures = {};
    this._captureState = {};
    this._videoReconnectTimers = {};
    this._videoReconnectInFlight = {};
    this._shutdownStarted = false;

    // WS-14: MODV_ENGINE=1 runs the Rust engine instead of the worker. The
    // engine presents its own native window; the renderer never creates a
    // WebGL/WebGPU context in this mode.
    this.engineMode = window.engine?.isEnabled?.() === true;
    this.$worker = this.engineMode ? null : new ModVWorker();
    this.$asyncWorker = this.engineMode
      ? null
      : new PromiseWorker(this.$worker);

    if (this.engineMode) {
      this.engineClient = new EngineClient();
      this.engineClient.bindMirror(store);
      this.engineClient.trackPreviewFrames();
      this.engineClient.onDelta((delta) => {
        const d = delta?.data || delta;

        if (d && typeof d.type === "string") {
          store.commit(d.type, d.payload);
        }
      });

      this.engineClient.onState((transition) => {
        if (transition?.state === "running") {
          void this._appReady();
        }
      });
      // Fallback: never hang the app if the engine never reports running.
      setTimeout(() => {
        if (!this._appReadySent) {
          logger.warn(
            "engine did not report running within 10 s; continuing without it",
          );
          this._signalReady();
        }
      }, 10_000);
    } else {
      this.$worker.postMessage({
        type: "__dirname",
        payload: app.getAppPath(),
        userData: app.getPath("userData"),
      });

      this.$worker.addEventListener("message", async (e) => {
        const message = e.data;
        const { type } = message;

        // if (
        //   type !== "metrics/SET_FPS_MEASURE" &&
        //   type !== "modules/UPDATE_ACTIVE_MODULE_PROP" &&
        //   type !== "beats/SET_BPM" &&
        //   type !== "beats/SET_KICK" &&
        //   type !== "tick"
        // ) {
        //   console.log(`⚙️%c ${type}`, "color: red");
        // }

        if (type === "destroyed") {
          console.log("webcontents got destroyed, sending onto main");
          ipcRenderer.send("destroyed");
          return;
        }

        if (type === "createWebcodecVideo") {
          const videoContext = await this.createWebcodecVideo(message);
          this.videos[videoContext.id] = videoContext;
          return;
        }

        if (type === "removeWebcodecVideo") {
          const { video, stream } = this.videos[message.id];
          video.src = "";
          for (const track of stream.getTracks()) {
            track.stop();
          }
          delete this.videos[message.id];
        }

        if (e.data.type === "tick" && this.ready) {
          this.tick(e.data.payload);
          return;
        }

        if (type === "worker-setup-complete") {
          void this._appReady();
          return;
        }

        if (Array.isArray(message)) {
          return;
        }
        const payload = e.data.payload ? JSON.parse(e.data.payload) : undefined;

        if (type === "commitQueue") {
          for (let i = 0; i < payload.length; i++) {
            const commit = payload[i];
            store.commit(commit.type, commit.payload);
          }
        } else {
          store.commit(type, payload);
        }
      });
    }

    const that = this;

    this.store = this.engineMode
      ? this.engineClient.storeFacade
      : {
          state: store.state,
          getters: store.getters,

          commit(...args) {
            return that.$worker.postMessage(
              {
                type: "commit",
                identifier: args[0],
                payload: args[1],
              },
              args[2],
            );
          },

          dispatch(...args) {
            return that.$asyncWorker.postMessage(
              {
                __async: true,
                type: "dispatch",
                identifier: args[0],
                payload: args[1],
              },
              args[2],
            );
          },
        };

    if (this.engineMode) {
      // The worker drove rendering; the engine has its own native window.
      // Keep the renderer's audio pump on a rAF cadence so the UI meters
      // stay live until the engine owns the analysis (WS-9, M6).
      const frame = (timestamp) => {
        this.tick(timestamp);
        this._engineRaf = window.requestAnimationFrame(frame);
      };
      this._engineRaf = window.requestAnimationFrame(frame);
    }
  }

  _signalReady() {
    this._resolveReady?.();
    ipcRenderer.send("modv-ready");
  }

  // Shared ready path: the legacy worker signals worker-setup-complete, the
  // engine signals `engine/state running`. Idempotent.
  async _appReady() {
    if (this._appReadySent) {
      return;
    }
    this._appReadySent = true;

    // Make the default group
    try {
      await this.store.dispatch("groups/createGroup", {
        enabled: GROUP_ENABLED,
      });
    } catch (error) {
      logger.error("groups/createGroup failed", { message: error?.message });
    }

    // Restore/seed Scenes before declaring the app ready. Seeding builds
    // temporary Groups and Modules, so tests (and the UI) must not race it
    // or they see a changing Group/Module list.
    try {
      await this.store.dispatch("scenes/hydrateScenes");
      await this.store.dispatch("scenes/ensureDefaultScenes");
    } catch (error) {
      console.error("Could not restore Scenes", error);
    }

    try {
      await this.store.dispatch("osc/createServer", {
        host: "0.0.0.0",
        port: 3333,
      });
    } catch (error) {
      logger.error("osc/createServer failed", { message: error?.message });
    }

    this._signalReady();
  }

  async setup(canvas = document.createElement("canvas")) {
    this.windowHandler();

    this.enumerateFonts();

    try {
      // The selected cameras and audio input are persisted; capture them
      // before setupMedia enumerates devices so no default camera is opened
      // instead.
      const savedSelection = [
        ...(this._store.state.mediaStream.selectedVideoSources || []),
      ];
      const savedAudio = this._store.state.mediaStream.currentAudioSource;

      const enumerated = await this.mediaDeviceManager.enumerateDevices();

      let restoredAudioId;
      let restoredAudioStream;
      let restoredSystemAudio = null;

      if (savedAudio) {
        const restoredAudio = restoreAudioSource(
          savedAudio,
          enumerated.audio,
          SYSTEM_AUDIO_SOURCE_ID,
        );

        if (restoredAudio && isSystemAudioSource(restoredAudio)) {
          // On Linux the system audio is a silent PulseAudio/PipeWire monitor,
          // so it is safe to start again automatically. On macOS/Windows
          // auto-starting would open the display picker at launch.
          if (window.remote?.process?.platform === "linux") {
            try {
              const captured = await this.systemAudioCapture.capture();

              restoredSystemAudio = captured;
              restoredAudioId = restoredAudio;
              restoredAudioStream = captured.stream;
            } catch (e) {
              logger.warn("could not restore the system audio input", {
                error: e?.message || String(e),
              });

              this.setSystemAudioState({
                supported: true,
                active: false,
                label: null,
                error: e?.message || String(e),
              });
            }
          } else {
            logger.info(
              "not restoring the system audio input on this platform",
              { platform: window.remote?.process?.platform },
            );

            this.setSystemAudioState({
              active: false,
              label: null,
              error: null,
            });
          }
        } else if (restoredAudio) {
          restoredAudioId = restoredAudio;
        }
      }

      await this.setupMedia({
        audioId: restoredAudioId,
        audioStream: restoredAudioStream,
        useDefaultDevices: true,
        useDefaultVideoDevice: false,
      });

      if (restoredSystemAudio) {
        this.setSystemAudioState({
          supported: true,
          active: true,
          label: restoredSystemAudio.label,
          error: null,
        });

        logger.info("restored the system audio input", {
          backend: restoredSystemAudio.backend,
          label: restoredSystemAudio.label,
        });
      }

      // Only the cameras that are currently present are opened, but the
      // persisted selection keeps cameras that are temporarily missing so
      // their index stays stable and they return after a replug.
      const selected = restoreVideoSources(savedSelection, enumerated.video);
      const videoPayload = { videoIds: savedSelection };

      this.store.commit("mediaStream/SET_SELECTED_VIDEO_SOURCES", videoPayload);
      this._store.commit(
        "mediaStream/SET_SELECTED_VIDEO_SOURCES",
        videoPayload,
      );

      for (const deviceId of selected) {
        // setupMedia picks up the per-device resolution / frame rate itself.
        // Open the cameras one at a time so the video capture service is not
        // asked to configure several devices at once.
        try {
          await this.setupMedia({ videoId: deviceId });
        } catch (e) {
          logger.warn("could not open the saved camera", {
            deviceId,
            error: e?.message || String(e),
          });
        }
      }
    } catch (e) {
      console.error(e);
    }

    // listen to mediastream device changes
    navigator.mediaDevices.ondevicechange = async () => {
      await this.refreshVideoInputs();
    };

    try {
      await this.setupMidi();
    } catch (e) {
      console.error(e);
    }

    this.setupBeatDetektor();

    this.canvas = canvas;

    if (this.engineMode) {
      // The engine presents the output natively in its own window; the
      // renderer's hidden canvas stays untouched (no OffscreenCanvas handoff).
    } else {
      const offscreen = this.canvas.transferControlToOffscreen();

      this.$worker.postMessage(
        {
          type: "canvas",
          where: "output",
          payload: offscreen,
        },
        [offscreen],
      );
    }

    this.store.dispatch("windows/createWindow");

    ipcRenderer.on("media-manager-state", (event, message) => {
      this.store.dispatch("media/setState", message);
    });

    ipcRenderer.on("media-manager-update", (event, message) => {
      this.store.dispatch("media/addMedia", message.payload);
    });

    ipcRenderer.on("media-manager-path-changed", (event, message) => {
      this.store.dispatch("media/setMediaDirectoryPath", message.payload);
    });

    ipcRenderer.on("open-preset", (event, message) => {
      this.loadPreset(message);
    });

    ipcRenderer.on("generate-preset", async () => {
      ipcRenderer.send("preset-data", await this.generatePreset());
    });

    ipcRenderer.on("set-current-project", async (event, message) => {
      await this.store.dispatch("projects/setCurrentProject", message);
      ipcRenderer.send(
        "current-project",
        this.store.state.projects.currentProject,
      );
    });

    ipcRenderer.on("create-output-window", () => {
      this.store.dispatch("windows/createWindow");
    });

    ipcRenderer.on("modv-destroy", () => {
      console.log("webcontents got modv-destroy, sending onto worker");

      this.shutdown();
    });

    function messageHandler({ data: messageData }) {
      if (messageData.type === "input-update") {
        const { moduleId, prop, data } = messageData.payload;
        this.store.dispatch("modules/updateProp", {
          moduleId,
          prop,
          data,
        });
      }
    }

    window.electronMessagePort?.start();
    window.electronMessagePort?.addEventListener(
      "message",
      messageHandler.bind(this),
    );

    ipcRenderer.send("get-media-manager-state");

    window.addEventListener("beforeunload", () => {
      this.shutdown();
    });

    window.addEventListener("unload", () => {
      this.shutdown({ terminateWorker: true });
    });

    // HMR only exists in the dev server; `import.meta.hot` is undefined in a
    // production build, so guard it instead of throwing out of setup().
    if (import.meta.hot) {
      import.meta.hot.on("vite:beforeFullReload", () => {
        this.shutdown();
      });
    }
  }

  stopMediaCapture() {
    const reconnectTimers = this._videoReconnectTimers || {};

    for (const deviceId of Object.keys(reconnectTimers)) {
      clearTimeout(reconnectTimers[deviceId]);
    }

    this._videoReconnectTimers = {};
    this._videoReconnectInFlight = {};

    for (const deviceId of Object.keys(this._captureState || {})) {
      this._captureState[deviceId].running = false;
    }

    for (const deviceId of Object.keys(this._imageCaptures || {})) {
      try {
        this._imageCaptures[deviceId]?.track?.stop?.();
      } catch (e) {
        // The capture may already be stopped.
      }

      delete this._imageCaptures[deviceId];
    }

    for (const deviceId of Object.keys(this.videoStreams || {})) {
      const video = this.videoStreams[deviceId];

      try {
        video?.pause?.();
      } catch (e) {
        // The video element may already be detached.
      }

      try {
        if (video) {
          video.srcObject = null;
        }
      } catch (e) {
        // The video element may already be detached.
      }

      delete this.videoStreams[deviceId];
    }

    for (const deviceId of Object.keys(this.videos || {})) {
      const { video, stream } = this.videos[deviceId] || {};

      try {
        if (video) {
          video.src = "";
        }
      } catch (e) {
        // The video element may already be detached.
      }

      for (const track of stream?.getTracks?.() || []) {
        try {
          track.stop();
        } catch (e) {
          // The track may already be stopped.
        }
      }

      delete this.videos[deviceId];
    }

    const streams = [
      this._audioMediaStream,
      this._videoMediaStream,
      this._systemAudioStream,
    ];

    for (const stream of streams) {
      for (const track of stream?.getTracks?.() || []) {
        try {
          track.stop();
        } catch (e) {
          // The track may already be stopped.
        }
      }
    }

    this._audioMediaStream = null;
    this._videoMediaStream = null;
    this._systemAudioStream = null;

    try {
      this.meyda?.stop?.();
    } catch (e) {
      // The analyzer may already be stopped.
    }

    try {
      this.systemAudioCapture?.stop?.();
    } catch (e) {
      // The capture may already be stopped.
    }

    try {
      this.audioContext?.close?.();
    } catch (e) {
      // The context may already be closed.
    }

    this.audioContext = null;

    try {
      if (navigator.mediaDevices) {
        navigator.mediaDevices.ondevicechange = null;
      }
    } catch (e) {
      // Media devices may already be unavailable.
    }
  }

  shutdown({ terminateWorker = false } = {}) {
    if (!this._shutdownStarted) {
      this._shutdownStarted = true;

      if (this._engineRaf) {
        try {
          window.cancelAnimationFrame(this._engineRaf);
        } catch (e) {
          // already stopped
        }
      }

      this.engineClient?.dispose();

      try {
        this.$worker?.postMessage({ type: "modv-destroy" });
      } catch (e) {
        // The worker may already be gone.
      }

      try {
        ipcRenderer.send("modv-destroy");
      } catch (e) {
        // The main process may already be gone.
      }

      this.stopMediaCapture();
    }

    if (terminateWorker) {
      try {
        this.$worker?.terminate?.();
      } catch (e) {
        // The worker may already be terminated.
      }
    }
  }

  loop(delta) {
    const {
      meyda: { features: featuresToGet },
    } = this.store.state;

    // bass/mid/treble are computed from the analyser's own FFT data below, not
    // by Meyda - keep them out of the Meyda extractor list or it throws on the
    // unknown feature names.
    const meydaFeatures = [];
    for (let i = 0; i < featuresToGet.length; i += 1) {
      if (!isBandFeature(featuresToGet[i])) {
        meydaFeatures.push(featuresToGet[i]);
      }
    }

    const features = this.meyda?.get(meydaFeatures);

    if (features) {
      const timeDomain = getByteTimeDomainData();
      // The analyser hands over 1024 bins (fftSize 2048) but the ISF audioFFT
      // inputs, the worker FFT aux canvas and Meyda are all built for 512.
      // Reduce to 512 uniform segments here so every consumer sees the same
      // data and the full spectrum instead of a clipped/aliased half.
      const byteFrequencyData = reduceByteFrequencyData(
        getByteFrequencyData(),
        constants.AUDIO_BUFFER_SIZE,
      );
      const floatFrequencyData = getFloatFrequencyData();
      const sampleRate = this.audioContext
        ? this.audioContext.sampleRate
        : 44100;
      const bands = this.audioBands.update({
        floatFrequencyData,
        byteTimeDomainData: timeDomain,
        byteFrequencyData,
        sampleRate,
      });

      // bass/mid/treble for Input Links and u_bass/u_mid/u_treble come from
      // the per-band running-peak features; the Point10 payload below keeps
      // its own tuned lane/grid path so plane9/Point10 scenes are unaffected.
      const bandFeatures = this.bandFeatures.update({
        floatFrequencyData,
        sampleRate,
      });

      features.bass = bandFeatures.bass;
      features.mid = bandFeatures.mid;
      features.treble = bandFeatures.treble;

      // full P10 band payload (raw bands, damping lanes, 16-cell grid, beat,
      // time-domain wave) for the Point10 scene adapter
      features.p10 = bands;

      this.updateBeatDetektor && this.updateBeatDetektor(delta, features);
      features.byteFrequencyData = Array.from(byteFrequencyData || []);
      // ISF audio inputs expect 512 time-domain values; the analyser now hands
      // over fftSize samples for AudioBands' wave buffer.
      features.byteTimeDomainData = Array.from(
        timeDomain ? timeDomain.slice(-constants.AUDIO_BUFFER_SIZE) : [],
      );
      if (this.engineMode) {
        // Keep the UI's meters live by feeding the mirror directly; the
        // engine owns analysis proper from WS-9 (M6).
        this._store.commit("meyda/SET_BANDS", {
          bass: features.bass ?? 0,
          mid: features.mid ?? 0,
          treble: features.treble ?? 0,
          cells: features.p10?.cells || [],
        });
      } else {
        this.$worker.postMessage({ type: "meyda", payload: features });
      }

      for (let i = 0; i < featuresToGet.length; i += 1) {
        const feature = featuresToGet[i];
        this.features[feature] = features[feature];
      }
    }
  }

  setSize({ width, height }) {
    this.store.dispatch("size/setSize", { width, height });
  }

  tick(delta) {
    this.loop(delta);
  }

  startCaptureForDevice(deviceId) {
    if (this.engineMode) {
      // Camera/screen frames flow to the engine via WS-10 (M6); until then
      // capture is not started in engine mode.
      if (!this._warnedEngineCamera) {
        logger.warn("camera capture is not wired to the engine yet (WS-10)");
        this._warnedEngineCamera = true;
      }
      return undefined;
    }

    if (!this._captureState[deviceId]) {
      this._captureState[deviceId] = { running: false, generation: 0 };
    }

    const state = this._captureState[deviceId];

    // Always start a fresh generation. A loop from a previous capture may
    // still be waiting on a grabFrame promise; bumping the generation makes it
    // exit instead of running a second loop against the same camera.
    state.running = true;
    state.generation = (state.generation || 0) + 1;
    state.failures = 0;
    state.recreateAttempted = false;
    state.mutedSince = null;
    state.grabFailuresReported = false;

    const generation = state.generation;

    logger.info("starting the video capture loop", { deviceId, generation });

    const isCurrent = () =>
      state.running &&
      state.generation === generation &&
      this._captureState[deviceId] === state;

    const schedule = (delay, fn) => {
      if (delay > 0) {
        setTimeout(() => {
          if (isCurrent()) fn();
        }, delay);
        return;
      }

      requestAnimationFrame(() => {
        if (isCurrent()) fn();
      });
    };

    const captureNext = () => {
      if (!isCurrent()) {
        return;
      }

      const capture = this._imageCaptures?.[deviceId];
      const track = capture?.track;

      if (track?.muted) {
        if (!state.mutedSince) {
          state.mutedSince = Date.now();
          logger.warn("the video track is muted", {
            deviceId,
            label: track.label,
            readyState: track.readyState,
          });
        }
      } else {
        state.mutedSince = null;
      }

      const mutedForMs = state.mutedSince
        ? Date.now() - state.mutedSince
        : null;
      const action = nextVideoCaptureAction({
        capture,
        consecutiveFailures: state.failures,
        mutedForMs,
        recreateAttempted: state.recreateAttempted,
      });

      if (action === "reconnect") {
        if (!this._videoReconnectTimers[deviceId]) {
          logger.warn("the video capture needs a reconnect", {
            deviceId,
            failures: state.failures,
            mutedForMs,
            readyState: track?.readyState,
          });
        }

        this.scheduleVideoReconnect?.(deviceId, { allowMuted: true });
        schedule(VIDEO_CAPTURE_RETRY_DELAY_MS, captureNext);
        return;
      }

      if (action === "wait") {
        schedule(VIDEO_CAPTURE_RETRY_DELAY_MS, captureNext);
        return;
      }

      if (action === "recreate") {
        const recreated = this.recreateImageCapture(deviceId);

        state.recreateAttempted = true;
        state.failures = 0;

        logger.warn("recreated the image capture after grab failures", {
          deviceId,
          recreated: !!recreated,
        });

        schedule(VIDEO_CAPTURE_RETRY_DELAY_MS, captureNext);
        return;
      }

      const deviceLabel = (() => {
        try {
          const list = this.store.state.mediaStream?.video || [];
          const match = list.find((d) => d.deviceId === deviceId);
          return match?.label || deviceId;
        } catch (_) {
          return deviceId;
        }
      })();

      capture
        .grabFrame()
        .then((bitmap) => {
          state.failures = 0;
          state.recreateAttempted = false;
          state.mutedSince = null;
          state.grabFailuresReported = false;

          if (bitmap && bitmap.width && bitmap.height) {
            if (this.engineMode) {
              // WS-10 (M6): the engine will ingest frames directly.
              bitmap.close?.();
            } else {
              this.$worker.postMessage(
                {
                  type: "videoFrame",
                  payload: { deviceId, label: deviceLabel, bitmap },
                },
                [bitmap],
              );
            }
          }
        })
        .catch((error) => {
          state.failures += 1;

          if (!state.grabFailuresReported) {
            state.grabFailuresReported = true;
            logger.warn("could not grab a video frame", {
              deviceId,
              failures: state.failures,
              error: describeCaptureError(error),
              readyState: track?.readyState,
              muted: track?.muted,
            });
          }
        })
        .finally(() => {
          if (!isCurrent()) {
            return;
          }

          // Back off after a failure; otherwise go straight to the next frame.
          schedule(
            state.failures > 0 ? VIDEO_CAPTURE_RETRY_DELAY_MS : 0,
            captureNext,
          );
        });
    };

    captureNext();
  }

  recreateImageCapture(deviceId) {
    const capture = this._imageCaptures?.[deviceId];
    const track = capture?.track;

    if (!track) {
      return null;
    }

    try {
      const recreated = new ImageCapture(track);
      this._imageCaptures[deviceId] = recreated;
      return recreated;
    } catch (e) {
      logger.warn("could not recreate the image capture", {
        deviceId,
        error: describeCaptureError(e),
      });
      return null;
    }
  }

  stopCaptureForDevice(deviceId) {
    const state = this._captureState[deviceId];
    if (!state) return;

    state.running = false;
    state.generation = (state.generation || 0) + 1;

    logger.info("stopped the video capture loop", {
      deviceId,
      generation: state.generation,
    });
  }

  setSystemAudioState(payload) {
    this.store.commit("mediaStream/SET_SYSTEM_AUDIO_STATE", payload);
    this._store.commit("mediaStream/SET_SYSTEM_AUDIO_STATE", payload);
  }

  scheduleVideoReconnect(deviceId, { allowMuted = false } = {}) {
    if (
      !deviceId ||
      this._videoReconnectTimers[deviceId] ||
      this._videoReconnectInFlight[deviceId]
    ) {
      return;
    }

    logger.info("scheduling a video input reconnect", { deviceId });

    this._videoReconnectTimers[deviceId] = setTimeout(async () => {
      delete this._videoReconnectTimers[deviceId];
      this._videoReconnectInFlight[deviceId] = true;

      try {
        const selected =
          this._store.state.mediaStream.selectedVideoSources || [];

        // The camera may have been disabled while the timer was pending.
        if (!selected.includes(deviceId)) {
          return;
        }

        if (
          !shouldReconnectVideoCapture(this._imageCaptures?.[deviceId], {
            allowMuted,
          })
        ) {
          return;
        }

        await this.setupMedia({
          videoId: deviceId,
          allowMutedRestart: allowMuted,
        });
        logger.info("reconnected the video input", { deviceId });
      } catch (e) {
        logger.warn("could not reconnect the video input", {
          deviceId,
          error: e?.name ? `${e.name}: ${e.message}` : String(e),
        });
      } finally {
        delete this._videoReconnectInFlight[deviceId];
      }
    }, 1500);
  }

  async refreshVideoInputs() {
    const enumerated = await this.mediaDeviceManager.enumerateDevices();
    const available = new Set(
      (enumerated.video || []).map((device) => device.deviceId),
    );
    const selected = this._store.state.mediaStream.selectedVideoSources || [];

    for (const deviceId of selected) {
      if (
        available.has(deviceId) &&
        shouldReconnectVideoCapture(this._imageCaptures?.[deviceId], {
          allowMuted: true,
        })
      ) {
        this.scheduleVideoReconnect(deviceId, { allowMuted: true });
      }
    }
  }

  async generatePreset() {
    if (this.engineMode) {
      // The engine doesn't serialise presets until WS-13 lands (M2), so the
      // renderer builds the document from the mirror store — the same code
      // the worker runs on the authoritative store. Keep the promise-worker
      // contract (a JSON string) for the save-preset menu path.
      return JSON.stringify(await createPresetData(this._store));
    }

    return await this.$asyncWorker.postMessage({
      __async: true,
      type: "generatePreset",
    });
  }

  loadPreset(filePathToPreset) {
    if (this.engineMode) {
      // Preset application in the engine is WS-13 (M2). Mutating the mirror
      // directly would desync the engine, so load is a documented no-op here.
      logger.warn("loadPreset is not wired to the engine yet (WS-13, M2)", {
        filePathToPreset,
      });
      return;
    }

    this.$worker.postMessage({ type: "loadPreset", payload: filePathToPreset });
  }

  async enumerateFonts() {
    const localFonts = await window.queryLocalFonts();
    const fonts = [];

    for (let i = 0; i < localFonts.length; i += 1) {
      const { family, fullName, postscriptName, style } = localFonts[i];

      fonts.push({ family, fullName, postscriptName, style });

      // No need to await here, async loading is fine.
      // The user can't use fonts fonts immediately at this stage, so no need to block the thread
      document.fonts.load(`14px ${postscriptName}`, fullName);
    }

    this.store.commit("fonts/SET_LOCAL_FONTS", fonts);
  }
}

export default new ModV();
