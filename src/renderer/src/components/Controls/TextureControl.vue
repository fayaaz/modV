<template>
  <grid columns="2">
    <c>
      <Select v-model="type" style="width: 100%">
        <option
          v-for="tt in textureTypes"
          :key="tt"
          :value="tt"
          :selected="type === tt"
        >
          {{ tt }}
        </option>
      </Select>
    </c>

    <c>
      <div v-if="type === 'group'">
        <Select
          v-model="modelCanvasId"
          style="width: 100%"
          @update:model-value="setTexture('group')"
        >
          <option
            v-for="group in groupOutputs"
            :key="group.id"
            :value="group.id"
          >
            {{ group.name }}
          </option>
        </Select>
      </div>

      <div v-if="type === 'canvas'">
        <Select
          v-model="modelCanvasId"
          style="width: 100%"
          @update:model-value="setTexture('canvas')"
        >
          <optgroup
            v-for="(outputs, group) in allOutputsByGroup"
            :key="group"
            :label="group"
          >
            <option
              v-for="output in outputs"
              :key="output.id"
              :value="output.id"
            >
              {{ output.name }}
            </option>
          </optgroup>
        </Select>
      </div>

      <div v-if="type === 'buffer'">
        <Select
          v-model="modelCanvasId"
          style="width: 100%"
          @update:model-value="setTexture('buffer')"
        >
          <option v-if="bufferOutputs.length === 0" selected value="">
            No buffers
          </option>
          <option
            v-for="output in bufferOutputs"
            :key="output.id"
            :value="output.id"
          >
            {{ output.name }}
          </option>
        </Select>
      </div>

      <div v-if="type === 'webcam'">
        <Select
          v-model="modelCanvasId"
          style="width: 100%"
          @update:model-value="setTexture('webcam')"
        >
          <option v-if="webcamOutputs.length === 0" selected value="">
            No webcam
          </option>
          <option
            v-for="output in webcamOutputs"
            :key="output.id"
            :value="output.id"
          >
            {{ output.name }}
          </option>
        </Select>
      </div>

      <div v-if="type === 'screen'">
        <Select
          v-model="modelCanvasId"
          style="width: 100%"
          @update:model-value="setTexture('screen')"
        >
          <option v-if="screenOutputs.length === 0" selected value="">
            No screen capture
          </option>
          <option
            v-for="output in screenOutputs"
            :key="output.id"
            :value="output.id"
          >
            {{ output.name }}
          </option>
        </Select>
      </div>

      <div v-if="type === 'image'">
        <Select
          v-model="modelImagePath"
          :disabled="!images"
          style="width: 100%"
          @update:model-value="setTexture('image')"
        >
          <option selected value="">No image</option>
          <option
            v-for="(image, index) in images"
            :key="index"
            :value="image.path"
          >
            {{ image.name }}
          </option>
        </Select>
      </div>

      <div v-if="type === 'video'">
        <Select
          v-model="modelVideoPath"
          :disabled="!videos"
          style="width: 100%"
          @update:model-value="setTexture('video')"
        >
          <option selected value="">No video</option>
          <option
            v-for="output in videos"
            :key="output.path"
            :value="output.path"
          >
            {{ output.name }}
          </option>
        </Select>
      </div>
    </c>

    <c span="2">
      <VideoControl
        v-if="type === 'video' && modelVideoPath && modelValue.id"
        :video-id="modelValue.id"
        :paused="modelValue.options.paused ?? false"
        :playbackrate="modelValue.options.playbackrate ?? 1"
        @pause="videoPause"
        @play="videoPlay"
        @ratechange="videoRateChange"
      />
    </c>
  </grid>
</template>

<script>
import constants from "../../application/constants";
import { VideoControl } from "./VideoControl.vue";
import { createLogger } from "../../application/logger";

const logger = createLogger("renderer:texture-control");

export default {
  components: {
    VideoControl,
  },
  props: {
    modelValue: { type: undefined },
  },
  emits: ["update:modelValue"],

  data() {
    return {
      textureTypes: [
        "inherit",
        "group",
        "canvas",
        "buffer",
        "webcam",
        "image",
        "video",
        "screen",
      ],
      type: "",
      modelImagePath: "",
      modelVideoPath: "",
      modelCanvasId: "",
      // WS-10: the engine source this control started for its video file, so a
      // file switch/unmount can stop exactly that pipeline.
      engineVideoSourceId: null,
    };
  },

  computed: {
    engineMode() {
      return this.$modV?.engineMode === true;
    },
    auxillaries() {
      return this.$modV.store.state.outputs.auxillary;
    },

    groupOutputs() {
      return Object.values(this.auxillaries).filter(
        (auxillary) =>
          auxillary.group === "group" &&
          auxillary.name !== constants.GALLERY_GROUP_NAME,
      );
    },

    allOutputsByGroup() {
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

    webcamOutputs() {
      return Object.values(this.auxillaries).filter(
        (auxillary) => auxillary.group === "input",
      );
    },

    screenOutputs() {
      return Object.values(this.auxillaries).filter(
        (auxillary) => auxillary.group === "screen",
      );
    },

    bufferOutputs() {
      return Object.values(this.auxillaries).filter(
        (auxillary) => auxillary.group === "buffer-modules",
      );
    },

    images() {
      return (
        this.$modV.store.state.media.media[
          this.$modV.store.state.projects.currentProject
        ].image ?? false
      );
    },

    videos() {
      return (
        this.$modV.store.state.media.media[
          this.$modV.store.state.projects.currentProject
        ].video ?? false
      );
    },
  },

  watch: {
    type(value) {
      this.setTexture(value);
    },
  },

  created() {
    this.type =
      this.modelValue?.type && this.modelValue?.type.length
        ? this.modelValue.type
        : this.textureTypes[0];
    this.modelImagePath =
      (this.modelValue?.type === "image" && this.modelValue?.options?.path) ||
      "";
    this.modelVideoPath =
      (this.modelValue?.type === "video" && this.modelValue?.options?.path) ||
      "";
    this.modelCanvasId = this.modelValue?.options?.id || "";

    // An already-applied video texture (preset load / panel reopen): make sure
    // the engine has a source for it.
    if (this.engineMode && this.type === "video" && this.modelVideoPath) {
      void this.applyEngineVideoSource();
    }
  },

  beforeUnmount() {
    void this.stopEngineVideoSource();
  },

  methods: {
    setTexture(type) {
      const textureDefinition = { type, options: {} };
      if (type === "webcam" || type === "screen" || type === "buffer") {
        // Treat webcam, screen, and buffer as canvas sources internally
        textureDefinition.type = "canvas";
      }
      if (type === "image") {
        if (!this.modelImagePath) {
          return;
        }

        textureDefinition.options.path = this.modelImagePath;
      }

      if (type === "video") {
        if (this.modelVideoPath === this.modelValue?.options?.path) {
          return;
        }

        textureDefinition.options.paused = false;
        textureDefinition.options.playbackrate = 1;
        textureDefinition.options.path = this.modelVideoPath;

        // WS-10: the engine decodes the file (GStreamer) — never a WebCodecs
        // decoder. The old source is stopped before the new one starts.
        if (this.engineMode) {
          void this.applyEngineVideoSource();
        }
      } else if (this.engineMode && this.engineVideoSourceId) {
        // Switched away from the video texture: release its pipeline.
        void this.stopEngineVideoSource();
      }

      if (
        type === "canvas" ||
        type === "group" ||
        type === "webcam" ||
        type === "screen" ||
        type === "buffer"
      ) {
        if (!this.modelCanvasId) {
          return;
        }

        textureDefinition.options.id = this.modelCanvasId;
      }

      this.$emit("update:modelValue", textureDefinition);
    },

    updateTextureDefinition(updatedValues) {
      this.$emit("update:modelValue", {
        ...this.modelValue,
        ...updatedValues,
      });
    },

    videoPause() {
      this.updateTextureDefinition({
        options: { ...this.modelValue.options, paused: true },
      });
    },

    videoPlay() {
      this.updateTextureDefinition({
        options: { ...this.modelValue.options, paused: false },
      });
    },

    videoRateChange(playbackrate) {
      this.updateTextureDefinition({
        options: { ...this.modelValue.options, playbackrate },
      });
    },

    // -- WS-10 engine video files -------------------------------------------

    /**
     * Start the engine's GStreamer source for the selected video file.
     *
     * The media library stores a media-directory-relative path
     * (`default/video/clip.mp4`); the engine wants an absolute path or a URI
     * (`engine-media.js#toEngineMediaPath`). The previous source for this
     * control is stopped first, and no WebCodecs decoder/`<video>` element is
     * ever created.
     */
    async applyEngineVideoSource() {
      const engineMedia = this.$modV.engineMedia;

      if (!engineMedia?.isEnabled?.() || !this.modelVideoPath) {
        return;
      }

      try {
        const response = await engineMedia.startVideoFile({
          path: this.modelVideoPath,
          mediaPath: this.$modV.store.state.media?.path,
          previousSourceId: this.engineVideoSourceId,
          // 0 = engine default (1280×720@30; the engine normalises).
          loop: this.modelValue?.options?.loop !== false,
        });

        this.engineVideoSourceId = response?.sourceId || null;
      } catch (e) {
        logger.warn("could not start the engine video file", {
          path: this.modelVideoPath,
          error: e?.message || String(e),
        });
      }
    },

    async stopEngineVideoSource() {
      const sourceId = this.engineVideoSourceId;

      if (!sourceId) {
        return;
      }

      this.engineVideoSourceId = null;

      try {
        await this.$modV.engineMedia?.stopSource?.(sourceId);
      } catch (e) {
        logger.warn("could not stop the engine video file", {
          sourceId,
          error: e?.message || String(e),
        });
      }
    },
  },
};
</script>

<style scoped>
grid {
  width: 100%;
}
</style>
