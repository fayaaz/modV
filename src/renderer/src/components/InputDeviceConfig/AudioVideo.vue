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

        <c span="1..">
          <grid columns="4">
            <c span="1">Video Resolution</c>
            <c span="2">
              <Select
                v-model="videoResolution"
                class="light"
                :disabled="switchingVideo"
              >
                <option value="default">Default</option>
                <option value="640x480">480p</option>
                <option value="1280x720">720p</option>
                <option value="1920x1080">1080p</option>
              </Select>
            </c>
            <c>{{ appliedResolutionText }}</c>
          </grid>
        </c>

        <c span="1..">
          <grid columns="4">
            <c span="1">Video Frame Rate</c>
            <c span="2">
              <Select
                v-model="videoFrameRate"
                class="light"
                :disabled="switchingVideo"
              >
                <option value="default">Default</option>
                <option value="30">30</option>
                <option value="60">60</option>
              </Select>
            </c>
            <c>{{ appliedFrameRateText }}</c>
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
export default {
  data() {
    return {
      iVTitle: "Media Input Config",
      iVBody: `Configure your audio and video inputs here. Click "Re-scan Devices" to scan for new sources.`,
      switchingAudio: false,
      switchingVideo: false,
      gainRangeValue: 1,
    };
  },

  computed: {
    audioInputs() {
      const audioInputs = Object.values(
        this.$modV.store.state.mediaStream.audio,
      );

      return audioInputs.sort((a, b) => a.label.localeCompare(b.label));
    },

    videoInputs() {
      const videoInputs = Object.values(
        this.$modV.store.state.mediaStream.video,
      );

      return videoInputs.sort((a, b) => a.label.localeCompare(b.label));
    },

    currentAudioSource: {
      get() {
        return this.$modV.store.state.mediaStream.currentAudioSource;
      },

      async set(value) {
        this.switchingAudio = true;
        await this.$modV.setupMedia({ audioId: value });
        this.switchingAudio = false;
      },
    },

    selectedVideoSourceIds() {
      return (
        this.$modV.store.state.mediaStream.selectedVideoSources ||
        (this.$modV.store.state.mediaStream.currentVideoSource
          ? [this.$modV.store.state.mediaStream.currentVideoSource]
          : [])
      );
    },

    videoResolution: {
      get() {
        const { width, height } =
          this.$modV.store.state.mediaStream.videoResolution || {};

        if (width && height) {
          return `${width}x${height}`;
        }

        return "default";
      },

      async set(value) {
        if (this.switchingVideo) {
          return;
        }

        const [width, height] = value.split("x");

        const videoResolution = {
          width: Number(width) || null,
          height: Number(height) || null,
        };

        this.switchingVideo = true;

        this.$modV.store.commit(
          "mediaStream/SET_VIDEO_RESOLUTION",
          videoResolution,
        );
        this.$modV._store.commit(
          "mediaStream/SET_VIDEO_RESOLUTION",
          videoResolution,
        );

        await this.applyVideoSettings();

        this.switchingVideo = false;
      },
    },

    videoFrameRate: {
      get() {
        const { videoFrameRate } = this.$modV.store.state.mediaStream;

        return videoFrameRate ? String(videoFrameRate) : "default";
      },

      async set(value) {
        if (this.switchingVideo) {
          return;
        }

        const frameRate = Number(value) || null;

        this.switchingVideo = true;

        this.$modV.store.commit("mediaStream/SET_VIDEO_FRAME_RATE", {
          frameRate,
        });
        this.$modV._store.commit("mediaStream/SET_VIDEO_FRAME_RATE", {
          frameRate,
        });

        await this.applyVideoSettings();

        this.switchingVideo = false;
      },
    },

    appliedResolutionText() {
      const { width, height } =
        this.$modV.store.state.mediaStream.appliedVideoSettings || {};

      if (!width || !height) {
        return "";
      }

      return `(active: ${width}x${height})`;
    },

    appliedFrameRateText() {
      const { frameRate } =
        this.$modV.store.state.mediaStream.appliedVideoSettings || {};

      if (!frameRate) {
        return "";
      }

      return `(active: ${frameRate}fps)`;
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
  },

  methods: {
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

    toggleVideoDevice(deviceId, enabled) {
      this.switchingVideo = true;

      if (enabled) {
        this.$modV.setupMedia({ videoId: deviceId });
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
      }

      this.switchingVideo = false;
    },

    async applyVideoSettings() {
      const { selectedVideoSources, videoResolution, videoFrameRate } =
        this.$modV.store.state.mediaStream;

      const deviceIds = [...(selectedVideoSources || [])];

      for (let i = 0; i < deviceIds.length; i++) {
        const deviceId = deviceIds[i];

        // setupMedia re-uses a live capture, so drop it first to force the
        // new resolution / frame rate constraints to be applied
        this.stopVideoDevice(deviceId);

        await this.$modV.setupMedia({
          videoId: deviceId,
          videoResolution,
          videoFrameRate,
        });
      }
    },

    renumerate() {
      this.$modV.enumerateDevices();
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
</style>
