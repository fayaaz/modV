import store from "./worker/store";
import { createLogger } from "./logger";
import {
  SYSTEM_AUDIO_SOURCE_ID,
  SYSTEM_AUDIO_SOURCE_LABEL,
} from "./system-audio";
import {
  cameraCapabilities,
  engineCameraDevices,
  isEngineMediaEnabled,
} from "./engine-media";

const logger = createLogger("renderer:media-devices");

class MediaDeviceManager {
  constructor() {
    this._store = store;
    this.audioSources = [];
    this.videoSources = [];
    // WS-10: with MODV_ENGINE=1 the engine enumerates cameras (GStreamer /
    // V4L2) and `navigator.mediaDevices.enumerateDevices()` must not be used
    // for video — Chromium's opaque device ids are not what the engine captures
    // from. Audio still comes from the browser (WS-9 is unchanged).
    this.engineMedia = isEngineMediaEnabled();

    // Listen for device changes. Only meaningful for the browser's own device
    // table; engine cameras are re-listed on demand (Re-scan Devices) and
    // reconciled from `media/status`.
    if (!this.engineMedia && navigator.mediaDevices) {
      navigator.mediaDevices.ondevicechange = () => this.enumerateDevices();
    }
  }

  /**
   * Enumerates available media devices and updates the store.
   * @returns {Promise<{audio: Array, video: Array}>}
   */
  async enumerateDevices() {
    const sources = {
      audio: [
        {
          deviceId: SYSTEM_AUDIO_SOURCE_ID,
          label: SYSTEM_AUDIO_SOURCE_LABEL,
          kind: "audioinput",
          isSystemAudio: true,
        },
      ],
      video: [],
    };

    if (this.engineMedia) {
      sources.video = await this.enumerateEngineCameras();
    }

    try {
      const devices = await navigator.mediaDevices.enumerateDevices();

      for (const device of devices) {
        if (device.kind === "audioinput") {
          sources.audio.push(device);
        } else if (device.kind === "videoinput" && !this.engineMedia) {
          // Engine mode: the engine's camera list is authoritative, so the
          // browser's (unusable) video devices are ignored.
          sources.video.push(device);
        }
      }
    } catch (e) {
      // The device list is unavailable (no audio backend, permission denied,
      // ...). The system audio entry stays so it can still be picked.
      logger.error("could not enumerate the media devices", e);
    }

    this.audioSources = sources.audio;
    this.videoSources = sources.video;

    // Update Vuex store
    this._store.commit("mediaStream/CLEAR_AUDIO_SOURCES");
    this._store.commit("mediaStream/CLEAR_VIDEO_SOURCES");

    for (const source of this.audioSources) {
      this._store.commit("mediaStream/ADD_AUDIO_SOURCE", { source });
    }

    for (const source of this.videoSources) {
      this._store.commit("mediaStream/ADD_VIDEO_SOURCE", { source });
    }

    return sources;
  }

  /**
   * Engine cameras as `mediaStream.video` devices (WS-10).
   *
   * The engine's caps list is also written to `mediaStream.videoCapabilities`
   * so `AudioVideo.vue` offers exactly the modes the camera/engine advertises
   * instead of the legacy `getCapabilities()` report.
   *
   * @returns {Promise<Array>}
   */
  async enumerateEngineCameras() {
    const engine = typeof window !== "undefined" ? window.engine : undefined;

    if (typeof engine?.listCameras !== "function") {
      return [];
    }

    try {
      const devices = engineCameraDevices(await engine.listCameras());

      this._engineCameraListFailed = false;

      for (const device of devices) {
        const { resolutions, frameRates } = cameraCapabilities(device.caps);

        this._store.commit("mediaStream/SET_VIDEO_CAPABILITIES", {
          deviceId: device.deviceId,
          resolutions,
          frameRates,
        });
      }

      return devices;
    } catch (e) {
      // One warning per failure streak: `enumerateDevices` runs on every audio
      // switch too, and a stopped engine would otherwise fill the log.
      if (!this._engineCameraListFailed) {
        this._engineCameraListFailed = true;
        logger.error("could not list the engine cameras", {
          error: e?.message || String(e),
        });
      }

      return [];
    }
  }

  /**
   * Selects an audio device by its ID.
   * @param {string} deviceId
   */
  selectAudioDevice(deviceId) {
    this._store.commit("mediaStream/SET_CURRENT_AUDIO_SOURCE", {
      audioId: deviceId,
    });
  }

  /**
   * Selects a video device by its ID.
   * @param {string} deviceId
   */
  selectVideoDevice(deviceId) {
    this._store.commit("mediaStream/SET_CURRENT_VIDEO_SOURCE", {
      videoId: deviceId,
    });
  }

  /**
   * Gets the current audio source ID from the store.
   * @returns {string|null}
   */
  get currentAudioSourceId() {
    return this._store.state.mediaStream.currentAudioSource;
  }

  /**
   * Gets the current video source ID from the store.
   * @returns {string|null}
   */
  get currentVideoSourceId() {
    return this._store.state.mediaStream.currentVideoSource;
  }
}

export default MediaDeviceManager;
