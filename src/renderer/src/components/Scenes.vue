<template>
  <div
    v-infoView="{ title: iVTitle, body: iVBody, id: 'Scenes Panel' }"
    v-searchTerms="{
      terms: ['scenes', 'autopilot', 'phrase', 'crossfade'],
      title: 'Scenes',
      type: 'Panel',
    }"
    class="scenes"
  >
    <grid columns="6" class="scenes__autopilot">
      <c span="1..">
        <grid columns="6">
          <c span="2">Autopilot</c>
          <c span="4">
            <Checkbox
              v-model="autopilotEnabled"
              class="scenes__autopilotCheckbox"
            />
          </c>
        </grid>
      </c>

      <c span="1..">
        <grid columns="6">
          <c span="2">Min dwell</c>
          <c span="4">
            <Number
              v-model="minSeconds"
              class="scenes__dwellInput"
              :min="1"
              :step="1"
            />
          </c>
        </grid>
      </c>

      <c span="1..">
        <grid columns="6">
          <c span="2">Max dwell</c>
          <c span="4">
            <Number
              v-model="maxSeconds"
              class="scenes__dwellInput"
              :min="2"
              :step="1"
            />
          </c>
        </grid>
      </c>

      <c span="1..">
        <grid columns="6">
          <c span="2">Crossfade</c>
          <c span="4">
            <Number
              v-model="transitionSeconds"
              class="scenes__dwellInput"
              :min="0.1"
              :step="0.1"
            />
          </c>
        </grid>
      </c>

      <c span="1..">
        <span class="scenes__status">
          <template v-if="currentScene">
            Current scene: {{ currentScene.name }}
            <template v-if="transitioning"> (crossfading)</template>
          </template>
          <template v-else>No scene applied</template>
        </span>
      </c>
    </grid>

    <div class="scene-container">
      <div
        v-for="scene in scenes"
        :key="scene.id"
        class="scene"
        :class="{ 'scene--current': scene.id === autopilot.currentSceneId }"
      >
        <div class="scene__name">{{ scene.name }}</div>
        <div class="scene__layers">
          {{ (scene.layers || []).length }} layers
        </div>
        <div class="scene__controls">
          <Button class="light" @click="applyScene(scene)">Apply</Button>
          <Button class="light" @click="removeScene(scene)">Delete</Button>
        </div>
      </div>
    </div>

    <grid columns="6" class="scenes__footer">
      <c span="1..">
        <TextInput
          v-model="newSceneName"
          class="scenes__nameInput"
          placeholder="New scene name"
        />
      </c>
      <c span="1..">
        <Button class="light" @click="saveCurrentAsScene">
          Save current as Scene
        </Button>
        <Button class="light" @click="loadFromDisk">Reload from disk</Button>
      </c>
    </grid>
  </div>
</template>

<script>
const { ipcRenderer } = window.electron;

export default {
  data() {
    return {
      iVTitle: "Scenes",
      iVBody:
        "Scenes are collections of Modules that can be saved and applied as a whole. The autopilot switches between Scenes on musical phrase changes detected from the bass audio band, crossfading each Scene's Group. Scenes are saved as JSON in the current project's scene folder.",
      newSceneName: "",
    };
  },

  computed: {
    scenes() {
      return this.$modV.store.state.scenes.scenes;
    },

    autopilot() {
      return this.$modV.store.state.scenes.autopilot;
    },

    transitioning() {
      return this.$modV.store.state.scenes.transitioning;
    },

    currentScene() {
      return this.scenes.find(
        (scene) => scene.id === this.autopilot.currentSceneId,
      );
    },

    currentProject() {
      return this.$modV.store.state.projects.currentProject;
    },

    autopilotEnabled: {
      get() {
        return this.autopilot.enabled;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/setAutopilot", { enabled: value });
      },
    },

    minSeconds: {
      get() {
        return this.autopilot.minSeconds;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updateAutopilot", {
          minSeconds: value,
        });
      },
    },

    maxSeconds: {
      get() {
        return this.autopilot.maxSeconds;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updateAutopilot", {
          maxSeconds: value,
        });
      },
    },

    transitionSeconds: {
      get() {
        return this.autopilot.transitionSeconds;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updateAutopilot", {
          transitionSeconds: value,
        });
      },
    },
  },

  mounted() {
    this.loadFromDisk();
  },

  methods: {
    async saveCurrentAsScene() {
      const scene = await this.$modV.store.dispatch(
        "scenes/saveCurrentAsScene",
        {
          name: this.newSceneName || undefined,
        },
      );

      if (!scene) {
        return;
      }

      ipcRenderer.send("save-file", {
        what: "scene",
        name: scene.name,
        fileType: "json",
        project: this.currentProject,
        payload: JSON.stringify(scene),
      });

      this.newSceneName = "";
    },

    applyScene(scene) {
      this.$modV.store.dispatch("scenes/applyScene", { sceneId: scene.id });
    },

    removeScene(scene) {
      this.$modV.store.dispatch("scenes/removeScene", { sceneId: scene.id });

      ipcRenderer.send("delete-scene", {
        name: scene.name,
        project: this.currentProject,
      });
    },

    loadFromDisk() {
      this.$modV.store.dispatch("scenes/loadScenesFromDisk");
    },
  },
};
</script>

<style scoped>
div.scenes {
  height: 100%;
  width: 100%;
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  box-sizing: border-box;
}

grid.scenes__autopilot {
  padding: 8px;
  grid-row-gap: 4px;
}

div.scene-container {
  flex: 1;
  overflow-y: auto;
  padding: 0 8px;
}

div.scene {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  margin-bottom: 4px;
  background: #282828;
  border-left: 3px solid transparent;
}

div.scene--current {
  border-left-color: #00c0ff;
}

div.scene__name {
  flex: 1;
  font-size: 14px;
}

div.scene__layers {
  font-size: 12px;
  opacity: 0.6;
}

div.scene__controls {
  display: flex;
  gap: 4px;
}

grid.scenes__footer {
  padding: 8px;
  grid-row-gap: 4px;
}

span.scenes__status {
  font-size: 12px;
  opacity: 0.8;
}
</style>
