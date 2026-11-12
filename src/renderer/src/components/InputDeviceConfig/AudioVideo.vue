<template>
  <div
    v-infoView="{ title: iVTitle, body: iVBody, id: 'Media Input Config' }"
    v-searchTerms="{
      terms: ['audio', 'video', 'input'],
      title: 'Audio/Video Input Config',
      type: 'Panel',
    }"
    class="device-config"
  >
    <div>
      <grid class="borders">
        <c span="1..">
          <grid columns="4">
            <c span="1">Audio Input</c>
            <c span="3">
              <Select
                v-model="currentAudioSource"
                class="light"
                :disabled="switchingAudio"
              >
                <option
                  v-for="input in audioInputs"
                  :key="input.deviceId"
                  :value="input.deviceId"
                >
                  {{ input.label }}
                </option>
              </Select>
            </c>
          </grid>
        </c>

        <c span="1..">
          <grid columns="4">
            <c span="1"></c>
            <c span="3">
              <div v-if="systemAudioError" class="system-audio-error">
                {{ systemAudioError }}
              </div>
              <div v-else-if="systemAudioActive" class="system-audio-status">
                (active: {{ systemAudioLabel }})
              </div>
            </c>
          </grid>
        </c>

        <c span="1..">
          <grid columns="4">
            <c span="1">Logs</c>
            <c span="2">
              <Button class="light" @click="openLogsFolder"> Open Logs </Button>
            </c>
            <c class="log-path">{{ logDirectory || "(unknown)" }}</c>
          </grid>
        </c>

        <c span="1..">
          <grid columns="4">
            <c span="1">Audio Gain</c>
            <c span="2">
              <Range
                v-model="gainRangeValue"
                :min="minGain"
                :max="maxGain"
                :step="0.01"
              />
            </c>
            <c>({{ gainRangeValue.toFixed(2) }})</c>
          </grid>
        </c>

        <c v-if="engineMode" span="1..">
          <grid columns="4">
            <c span="1">Capture</c>
            <c span="3" class="engine-capture-hint">
              Cameras are captured by the Rust engine (GStreamer). Enabling a
              camera starts an engine source; the order below is the Webcam
              module's <code>camera</code> index.
            </c>
          </grid>
        </c>

        <c span="1..">
          <grid columns="4">
            <c span="1">Video Inputs</c>
            <c span="3">
              <grid columns="3" class="video-inputs-grid">
                <c
                  v-for="input in videoInputs"
                  :key="input.deviceId"
                  class="video-input-item"
                >
                  <grid columns="2">
                    <c style="width: 16px">
                      <Checkbox
                        class="light"
                        :model-value="
                          selectedVideoSourceIds.includes(input.deviceId)
                        "
                        :emit-boolean="true"
                        :disabled="switchingVideo"
                        @update:model-value="
                          (v) => toggleVideoDevice(input.deviceId, v)
                        "
                      />
                    </c>
                    <c>
                      <span class="video-input-label">{{ input.label }}</span>
                    </c>
                  </grid>
                </c>
              </grid>
            </c>
          </grid>
        </c>

        <c v-if="selectedVideoInputs.length === 0" span="1..">
          <grid columns="4">
            <c span="1"></c>
            <c span="3" class="video-settings-hint">
              Select a camera to choose its resolution and frame rate.
            </c>
          </grid>
        </c>

        <c
          v-for="input in selectedVideoInputs"
          :key="`settings-${input.deviceId}`"
          span="1.."
        >
          <grid columns="4" class="video-settings">
            <c span="1.." class="video-settings__title">{{ input.label }}</c>

            <c span="1">Video Resolution</c>
            <c span="2">
              <Select
                :model-value="videoResolutionValue(input.deviceId)"
                class="light"
                :disabled="switchingVideo"
                @update:model-value="
                  (value) => setVideoResolution(input.deviceId, value)
                "
              >
                <option value="default">Default</option>
                <option
                  v-for="option in videoResolutionOptions(input.deviceId)"
                  :key="`${option.width}x${option.height}`"
                  :value="`${option.width}x${option.height}`"
                >
                  {{ option.width }}×{{ option.height }}
                </option>
              </Select>
            </c>
            <c>{{ appliedResolutionText(input.deviceId) }}</c>

            <c span="1">Video Frame Rate</c>
            <c span="2">
              <Select
                :model-value="videoFrameRateValue(input.deviceId)"
                class="light"
                :disabled="switchingVideo"
                @update:model-value="
                  (value) => setVideoFrameRate(input.deviceId, value)
                "
              >
                <option value="default">Default</option>
                <option
                  v-for="option in videoFrameRateOptions(input.deviceId)"
                  :key="option.value"
                  :value="String(option.value)"
                >
                  {{ option.label }}
                </option>
              </Select>
            </c>
            <c>{{ appliedFrameRateText(input.deviceId) }}</c>

            <c
              v-if="engineMode && engineStatusText(input.deviceId)"
              span="1.."
              class="engine-source-status"
            >
              {{ engineStatusText(input.deviceId) }}
            </c>

            <c
              v-if="engineMode && engineNotice(input.deviceId)"
              span="1.."
              class="system-audio-error"
            >
              {{ engineNotice(input.deviceId) }}
            </c>
          </grid>
        </c>

        <c v-if="engineMode && engineStatusError" span="1..">
          <grid columns="4">
            <c span="1"></c>
            <c span="3" class="system-audio-error">{{ engineStatusError }}</c>
          </grid>
        </c>

        <c v-if="engineMode && engineSources.length" span="1..">
          <grid columns="4">
            <c span="1">Engine Sources</c>
            <c span="3">
              <div
                v-for="source in engineSources"
                :key="source.sourceId"
                class="engine-source"
              >
                <span class="engine-source__id">{{ source.sourceId }}</span>
                <span class="engine-source__state">
                  — {{ formatEngineSource(source) }}</span
                >
              </div>
            </c>
          </grid>
        </c>

        <c span="1..">
          <grid columns="4">
            <c span="2+3">
              <Button class="light" @click="renumerate">Re-scan Devices</Button>
            </c>
          </grid>
        </c>
      </grid>
    </div>
  </div>
</template>

<script>
import { createLogger } from "../../application/logger";
import { isSystemAudioSource } from "../../application/system-audio";
import {
  buildFrameRateOptions,
  buildResolutionOptions,
} from "../../application/video-capabilities";
import {
  cameraSourceId,
  ENGINE_MEDIA_STATUS_POLL_MS,
  formatDeviceStatus,
  formatMediaSourceStatus,
  orderEngineCameraInputs,
} from "../../application/engine-media";

const logger = createLogger("renderer:audio-input");

export default {
  data() {
    return {
      iVTitle: "Media Input Config",
      iVBody: `Configure your audio and video inputs here. Click "Re-scan Devices" to scan for new sources.`,
      switchingAudio: false,
      switchingVideo: false,
      gainRangeValue: 1,
      audioSelectValue: null,
      logDirectory: "",
      // WS-10: engine media sources and their last known state (~1 Hz poll).
      engineSources: [],
      engineStatusError: "",
      enginePollTimer: null,
    };
  },

  computed: {
    engineMode() {
      return this.$modV?.engineMode === true;
    },
    audioInputs() {
      const audioInputs = Object.values(
        this.$modV.store.state.mediaStream.audio,
      );

      return audioInputs.sort((a, b) => {
        if (!!a.isSystemAudio !== !!b.isSystemAudio) {
          return a.isSystemAudio ? -1 : 1;
        }

        return a.label.localeCompare(b.label);
      });
    },

    videoInputs() {
      const videoInputs = Object.values(
        this.$modV.store.state.mediaStream.video,
      );

      // Engine mode: the checked cameras lead, in the order the engine indexes
      // them, so the panel's "order below" really is the Webcam module's
      // `camera` index. The legacy list stays label-sorted.
      if (this.engineMode) {
        return orderEngineCameraInputs(
          videoInputs,
          this.selectedVideoSourceIds,
        );
      }

      return videoInputs.sort((a, b) => a.label.localeCompare(b.label));
    },

    currentAudioSource: {
      get() {
        if (this.audioSelectValue !== null) {
          return this.audioSelectValue;
        }

        return this.$modV.store.state.mediaStream.currentAudioSource;
      },

      async set(value) {
        const previous = this.$modV.store.state.mediaStream.currentAudioSource;

        this.switchingAudio = true;
        this.audioSelectValue = value;

        logger.info("the audio input was changed", { previous, value });

        try {
          if (isSystemAudioSource(value)) {
            const captured = await this.$modV.systemAudioCapture.capture();

            await this.$modV.setupMedia({
              audioId: value,
              audioStream: captured.stream,
            });

            this.setSystemAudioState({
              supported: true,
              active: true,
              label: captured.label,
              error: null,
            });

            logger.info("the system audio is now the audio input", {
              backend: captured.backend,
              label: captured.label,
            });
          } else {
            await this.$modV.setupMedia({ audioId: value });

            this.setSystemAudioState({
              active: false,
              label: null,
              error: null,
            });
          }
        } catch (e) {
          logger.error("the audio input could not be changed", {
            previous,
            value,
            error: e?.message || String(e),
            stack: e?.stack,
          });

          this.setSystemAudioState({
            supported: true,
            active: false,
            label: null,
            error: e?.message || String(e),
          });

          this.revertAudioSource(previous);
        } finally {
          this.audioSelectValue = null;
          this.switchingAudio = false;
        }
      },
    },

    systemAudioState() {
      return this.$modV.store.state.mediaStream.systemAudio || {};
    },

    systemAudioActive() {
      return !!this.systemAudioState.active;
    },

    systemAudioLabel() {
      return this.systemAudioState.label || "";
    },

    systemAudioError() {
      return this.systemAudioState.error || "";
    },

    selectedVideoSourceIds() {
      return (
        this.$modV.store.state.mediaStream.selectedVideoSources ||
        (this.$modV.store.state.mediaStream.currentVideoSource
          ? [this.$modV.store.state.mediaStream.currentVideoSource]
          : [])
      );
    },

    selectedVideoInputs() {
      return this.videoInputs.filter((input) =>
        this.selectedVideoSourceIds.includes(input.deviceId),
      );
    },

    maxGain() {
      const value = this.$modV.gainNode?.gain.maxValue
        .toPrecision(1 + 2)
        .split("e")[0];

      return Number(value);
    },

    minGain() {
      return 0;
    },
  },

  watch: {
    gainRangeValue(value) {
      this.$modV.gainNode.gain.value = value;
    },
  },

  created() {
    const value = this.$modV.gainNode?.gain.value ?? 1;
    this.gainRangeValue = Number(value);

    this.loadLogDirectory();

    if (this.engineMode) {
      this.startEngineStatusPolling();
    }
  },

  beforeUnmount() {
    this.stopEngineStatusPolling();
  },

  methods: {
    async loadLogDirectory() {
      try {
        this.logDirectory =
          await window.electron.ipcRenderer.invoke("get-logs-folder");
      } catch (e) {
        logger.warn("could not read the log directory", {
          message: e?.message,
        });
      }
    },

    async openLogsFolder() {
      try {
        const directory =
          await window.electron.ipcRenderer.invoke("open-logs-folder");

        this.logDirectory = directory;
      } catch (e) {
        logger.error("could not open the log directory", {
          message: e?.message,
        });
      }
    },

    setSystemAudioState(payload) {
      this.$modV.store.commit("mediaStream/SET_SYSTEM_AUDIO_STATE", payload);
      this.$modV._store.commit("mediaStream/SET_SYSTEM_AUDIO_STATE", payload);
    },

    // -- WS-10 engine capture ------------------------------------------------

    startEngineStatusPolling() {
      this.stopEngineStatusPolling();
      this.enginePollTimer = setInterval(() => {
        void this.pollEngineStatus();
      }, ENGINE_MEDIA_STATUS_POLL_MS);
      void this.pollEngineStatus();
    },

    stopEngineStatusPolling() {
      if (this.enginePollTimer !== null) {
        clearInterval(this.enginePollTimer);
        this.enginePollTimer = null;
      }
    },

    // One `media/status` poll: refreshes the per-source state line, and lets
    // the controller stop cameras that disappeared and restart dead ones (with
    // a backoff, so a busy device is not hammered).
    async pollEngineStatus() {
      const engineMedia = this.$modV.engineMedia;

      if (!engineMedia?.isEnabled?.()) {
        return;
      }

      const availableIds = Object.values(
        this.$modV.store.state.mediaStream.video || {},
      )
        .map((device) => device?.deviceId)
        .filter(Boolean);

      try {
        const result = await engineMedia.pollStatus({
          selectedIds: this.selectedVideoSourceIds,
          availableIds,
        });

        this.engineSources = Array.isArray(result.sources)
          ? result.sources
          : [];
        this.engineStatusError = result.error || "";
      } catch (e) {
        this.engineStatusError = e?.message || String(e);
      }
    },

    engineStatusText(deviceId) {
      return formatDeviceStatus({ sources: this.engineSources }, deviceId);
    },

    formatEngineSource(source) {
      return formatMediaSourceStatus(source);
    },

    // One message per source, recorded by the controller, so a failing camera
    // does not append a notice on every poll.
    engineNotice(deviceId) {
      const sourceId = cameraSourceId(deviceId);

      return sourceId
        ? this.$modV.engineMedia?.noticeFor?.(sourceId) || ""
        : "";
    },

    commitSelectedVideoSources(videoIds) {
      const payload = { videoIds: [...videoIds] };

      this.$modV.store.commit(
        "mediaStream/SET_SELECTED_VIDEO_SOURCES",
        payload,
      );
      this.$modV._store.commit(
        "mediaStream/SET_SELECTED_VIDEO_SOURCES",
        payload,
      );
    },

    revertAudioSource(previous) {
      if (this.$modV.store.state.mediaStream.currentAudioSource !== previous) {
        this.$modV.store.commit("mediaStream/SET_CURRENT_AUDIO_SOURCE", {
          audioId: previous,
        });
        this.$modV._store.commit("mediaStream/SET_CURRENT_AUDIO_SOURCE", {
          audioId: previous,
        });
      }
    },

    stopVideoDevice(deviceId) {
      const imageCapture = this.$modV._imageCaptures?.[deviceId];

      if (imageCapture) {
        try {
          imageCapture.track?.stop?.();
        } catch (e) {
          // the track may already be stopped
        }
        delete this.$modV._imageCaptures[deviceId];
      }

      const videoStream = this.$modV.videoStreams?.[deviceId];

      if (videoStream) {
        try {
          videoStream.pause();
        } catch (e) {
          // the stream may already be detached
        }

        videoStream.srcObject = null;
        delete this.$modV.videoStreams[deviceId];
      }

      // Stop the capture loop for this device
      this.$modV.stopCaptureForDevice?.(deviceId);
    },

    getDeviceVideoSettings(deviceId) {
      const mediaStream = this.$modV.store.state.mediaStream;
      const perDevice = mediaStream.videoSettings?.[deviceId];

      const resolution =
        perDevice && perDevice.resolution !== undefined
          ? perDevice.resolution
          : mediaStream.videoResolution || { width: null, height: null };

      const frameRate =
        perDevice && perDevice.frameRate !== undefined
          ? perDevice.frameRate
          : (mediaStream.videoFrameRate ?? null);

      return { resolution, frameRate };
    },

    videoCapabilitiesFor(deviceId) {
      return (
        this.$modV.store.state.mediaStream.videoCapabilities?.[deviceId] || {}
      );
    },

    videoResolutionValue(deviceId) {
      const { resolution } = this.getDeviceVideoSettings(deviceId);

      return resolution?.width && resolution?.height
        ? `${resolution.width}x${resolution.height}`
        : "default";
    },

    videoFrameRateValue(deviceId) {
      const { frameRate } = this.getDeviceVideoSettings(deviceId);

      return frameRate ? String(frameRate) : "default";
    },

    videoResolutionOptions(deviceId) {
      const { resolution } = this.getDeviceVideoSettings(deviceId);
      const capabilities = this.videoCapabilitiesFor(deviceId);
      const options = Array.isArray(capabilities.resolutions)
        ? capabilities.resolutions.map((mode) => ({ ...mode }))
        : [];

      if (!options.length) {
        return buildResolutionOptions({}, resolution || {});
      }

      if (resolution?.width && resolution?.height) {
        const current = {
          width: resolution.width,
          height: resolution.height,
        };

        if (!options.some((mode) => this.sameResolution(mode, current))) {
          options.push(current);
        }
      }

      return options.sort((a, b) => a.width * a.height - b.width * b.height);
    },

    videoFrameRateOptions(deviceId) {
      const { frameRate } = this.getDeviceVideoSettings(deviceId);
      const capabilities = this.videoCapabilitiesFor(deviceId);
      const options = Array.isArray(capabilities.frameRates)
        ? capabilities.frameRates.map((option) => ({ ...option }))
        : [];

      if (!options.length) {
        return buildFrameRateOptions({}, { frameRate });
      }

      if (frameRate && !options.some((option) => option.value === frameRate)) {
        options.push({ value: frameRate, label: `${frameRate} fps` });
      }

      return options.sort((a, b) => a.value - b.value);
    },

    sameResolution(a, b) {
      return a.width === b.width && a.height === b.height;
    },

    appliedResolutionText(deviceId) {
      const { width, height } =
        this.$modV.store.state.mediaStream.appliedVideoSettings?.[deviceId] ||
        {};

      if (!width || !height) {
        return "";
      }

      return `(active: ${width}x${height})`;
    },

    appliedFrameRateText(deviceId) {
      const { frameRate } =
        this.$modV.store.state.mediaStream.appliedVideoSettings?.[deviceId] ||
        {};

      if (!frameRate) {
        return "";
      }

      return `(active: ${frameRate}fps)`;
    },

    async setVideoResolution(deviceId, value) {
      const [width, height] = value.split("x");
      const resolution =
        value === "default"
          ? { width: null, height: null }
          : {
              width: Number(width) || null,
              height: Number(height) || null,
            };

      await this.updateVideoSettings(deviceId, { resolution });
    },

    async setVideoFrameRate(deviceId, value) {
      const frameRate = value === "default" ? null : Number(value) || null;

      await this.updateVideoSettings(deviceId, { frameRate });
    },

    async updateVideoSettings(deviceId, patch) {
      if (this.switchingVideo) {
        return;
      }

      this.switchingVideo = true;

      const payload = { deviceId, ...patch };

      this.$modV.store.commit("mediaStream/SET_VIDEO_SETTINGS", payload);
      this.$modV._store.commit("mediaStream/SET_VIDEO_SETTINGS", payload);

      try {
        await this.applyVideoSettings(deviceId);
      } finally {
        this.switchingVideo = false;
      }
    },

    syncSelectedVideoSources() {
      const videoIds = [
        ...(this.$modV.store.state.mediaStream.selectedVideoSources || []),
      ];
      const payload = { videoIds };

      this.$modV.store.commit(
        "mediaStream/SET_SELECTED_VIDEO_SOURCES",
        payload,
      );
      this.$modV._store.commit(
        "mediaStream/SET_SELECTED_VIDEO_SOURCES",
        payload,
      );
    },

    async toggleVideoDevice(deviceId, enabled) {
      if (this.switchingVideo) {
        return;
      }

      logger.info("the video input was toggled", { deviceId, enabled });

      this.switchingVideo = true;

      try {
        if (enabled) {
          const { resolution, frameRate } =
            this.getDeviceVideoSettings(deviceId);

          if (this.engineMode) {
            // The engine's 1-based camera index is the position in the saved
            // selection, so add the camera to the order *before* starting it.
            const selected = [...this.selectedVideoSourceIds];
            const alreadySelected = selected.includes(deviceId);

            if (!alreadySelected) {
              selected.push(deviceId);
              this.commitSelectedVideoSources(selected);
            }

            try {
              await this.$modV.setupMedia({
                videoId: deviceId,
                videoResolution: resolution,
                videoFrameRate: frameRate,
              });
            } catch (e) {
              if (!alreadySelected) {
                this.commitSelectedVideoSources(
                  selected.filter((id) => id !== deviceId),
                );
              }

              throw e;
            }

            void this.pollEngineStatus();
          } else {
            await this.$modV.setupMedia({
              videoId: deviceId,
              videoResolution: resolution,
              videoFrameRate: frameRate,
            });

            this.syncSelectedVideoSources();
          }
        } else {
          // Stop and remove this device stream if present
          this.stopVideoDevice(deviceId);

          // Inform worker to remove aux for this device if one exists
          this.$modV.store.dispatch("outputs/removeWebcamOutputForDevice", {
            deviceId,
          });
          this.$modV.store.commit("mediaStream/REMOVE_SELECTED_VIDEO_SOURCE", {
            videoId: deviceId,
          });
          this.$modV._store.commit("mediaStream/REMOVE_SELECTED_VIDEO_SOURCE", {
            videoId: deviceId,
          });
        }
      } catch (e) {
        logger.error("the video input could not be changed", {
          deviceId,
          enabled,
          error: e?.message || String(e),
        });
      } finally {
        this.switchingVideo = false;
      }
    },

    recordAppliedVideoSettings(deviceId, track) {
      let settings = {};

      try {
        settings = track.getSettings?.() || {};
      } catch (e) {
        // The track may have ended while the constraints were applied.
      }

      this.$modV._store.commit("mediaStream/SET_APPLIED_VIDEO_SETTINGS", {
        deviceId,
        width: settings.width,
        height: settings.height,
        frameRate: settings.frameRate ? Math.round(settings.frameRate) : null,
      });
    },

    async applyVideoSettings(deviceId) {
      const { selectedVideoSources } = this.$modV.store.state.mediaStream;

      const deviceIds = deviceId
        ? [deviceId]
        : [...(selectedVideoSources || [])];

      for (let i = 0; i < deviceIds.length; i++) {
        const id = deviceIds[i];
        const { resolution, frameRate } = this.getDeviceVideoSettings(id);
        const track = this.$modV._imageCaptures?.[id]?.track;

        // Apply constraints to the live track. Stopping and re-opening the
        // same USB camera can deadlock the renderer, and this also avoids a
        // visible frame drop while switching modes.
        if (track && typeof track.applyConstraints === "function") {
          const constraints = {
            frameRate: { ideal: frameRate || 60 },
          };

          if (resolution?.width && resolution?.height) {
            constraints.width = { ideal: resolution.width };
            constraints.height = { ideal: resolution.height };
          }

          try {
            await track.applyConstraints(constraints);
            this.recordAppliedVideoSettings(id, track);
            logger.info("applied the video settings in place", {
              deviceId: id,
              resolution,
              frameRate,
            });
            continue;
          } catch (e) {
            logger.warn("could not apply the video settings in place", {
              deviceId: id,
              error: e?.message || String(e),
            });
          }
        }

        // No live track (for example the camera was removed): fall back to a
        // fresh capture with the requested constraints.
        logger.warn("restarting the video input for new settings", {
          deviceId: id,
          hadLiveTrack: !!track,
          resolution,
          frameRate,
        });

        this.stopVideoDevice(id);

        await this.$modV.setupMedia({
          videoId: id,
          videoResolution: resolution,
          videoFrameRate: frameRate,
        });
      }
    },

    renumerate() {
      // In engine mode a re-scan must also re-list the engine's cameras and
      // reconcile the running sources; `refreshVideoInputs` does both.
      if (typeof this.$modV.refreshVideoInputs === "function") {
        void this.$modV.refreshVideoInputs();
        return;
      }

      void this.$modV.mediaDeviceManager?.enumerateDevices?.();
    },
  },
};
</script>

<style scoped>
.device-config input,
.device-config textarea,
.device-config .select {
  max-width: 120px !important;
}

.device-config .range-control {
  max-width: 240px !important;
}

.system-audio-status {
  color: #8f8;
}

.system-audio-error {
  color: #f88;
}

.video-settings {
  margin-bottom: 6px;
}

.video-settings__title {
  font-weight: bold;
  margin-bottom: 2px;
}

.video-settings-hint {
  color: #9a9a9a;
}

.engine-capture-hint {
  color: #9a9a9a;
  font-style: italic;
}

.engine-source-status {
  color: #8f8;
}

.engine-source {
  font-family: monospace;
  font-size: 11px;
}

.engine-source__id {
  color: var(--foreground-color);
}

.engine-source__state {
  color: #9a9a9a;
}
</style>
