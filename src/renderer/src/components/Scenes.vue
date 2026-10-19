<template>
  <div
    v-infoView="{ title: iVTitle, body: iVBody, id: 'Scenes Panel' }"
    v-searchTerms="{
      terms: ['scenes', 'autopilot', 'performance', 'phrase', 'crossfade'],
      title: 'Scenes',
      type: 'Panel',
    }"
    class="scenes"
  >
    <grid columns="6" class="scenes__autopilot">
      <c span="1..">
        <grid columns="6">
          <c span="2">Performance mode</c>
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
          <c span="2">Default min scene time</c>
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
          <c span="2">Default max scene time</c>
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
          <c span="2">Transition time</c>
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
        <grid columns="6">
          <c span="2">Transition</c>
          <c span="4">
            <Select v-model="transition" class="scenes__transitionSelect">
              <option
                v-for="name in transitionOptions"
                :key="name"
                :value="name"
              >
                {{ name }}
              </option>
            </Select>
          </c>
        </grid>
      </c>

      <c span="1..">
        <span class="scenes__status">
          <template v-if="currentScene">
            Current scene: {{ currentScene.name }}
            <template v-if="transitioning"> (transitioning)</template>
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
        <div class="scene__details">
          <div class="scene__name" @dblclick="startSceneRename(scene)">
            <TextInput
              v-if="editingSceneId === scene.id"
              ref="sceneNameInput"
              v-model="localSceneName"
              @keypress.enter="commitSceneRename(scene)"
              @blur="commitSceneRename(scene)"
              @keydown.esc="cancelSceneRename"
            />
            <span v-else>{{ scene.name }}</span>
          </div>
          <div class="scene__layers">
            {{ sceneGroupCount(scene) }} groups &middot;
            {{ sceneModuleCount(scene) }} modules
          </div>
          <div class="scene__timing">
            <span class="scene__timingLabel">Time</span>
            <Number
              :model-value="sceneMin(scene)"
              class="scene__timeInput"
              :min="1"
              :step="1"
              @update:model-value="(value) => setSceneMin(scene, value)"
            />
            <span>&ndash;</span>
            <Number
              :model-value="sceneMax(scene)"
              class="scene__timeInput"
              :min="2"
              :step="1"
              @update:model-value="(value) => setSceneMax(scene, value)"
            />
            <span>s</span>
          </div>
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
        <Button class="light" @click="nextScene">Next scene</Button>
      </c>
    </grid>
  </div>
</template>

<script>
import {
  FADE_TRANSITION,
  NONE_TRANSITION,
  NATIVE_TRANSITION_NAMES,
} from "../application/scene-transition-defs";

const { ipcRenderer } = window.electron;

export default {
  data() {
    return {
      iVTitle: "Scenes",
      iVBody:
        "Scenes are snapshots of the Groups tab: every Group, its settings and its Modules can be saved and reloaded as a whole. Performance mode switches between Scenes on musical phrase changes detected from the bass audio band, transitioning all Groups between Scenes. Scenes are saved as JSON in the current project's scene folder.",
      newSceneName: "",
      editingSceneId: null,
      localSceneName: "",
      sceneSaveTimer: null,
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

    transition: {
      get() {
        return this.autopilot.transition || FADE_TRANSITION;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updateAutopilot", {
          transition: value,
        });
      },
    },

    transitionOptions() {
      const registered = this.$modV.store.state.modules.registered;
      const isfTransitions = Object.values(registered)
        .filter(
          (module) =>
            module.props &&
            module.props.startImage &&
            module.props.endImage &&
            module.props.progress,
        )
        .map((module) => module.meta.name)
        .sort((a, b) => a.localeCompare(b));

      return [
        FADE_TRANSITION,
        NONE_TRANSITION,
        ...NATIVE_TRANSITION_NAMES,
        ...isfTransitions,
      ];
    },
  },

  mounted() {
    this.loadFromDisk();
  },

  beforeUnmount() {
    clearTimeout(this.sceneSaveTimer);
  },

  methods: {
    sceneGroupCount(scene) {
      if (Array.isArray(scene.groups)) {
        return scene.groups.length;
      }

      return Array.isArray(scene.layers) ? 1 : 0;
    },

    sceneModuleCount(scene) {
      if (Array.isArray(scene.groups)) {
        return scene.groups.reduce(
          (count, group) => count + (group.modules || []).length,
          0,
        );
      }

      return Array.isArray(scene.layers) ? scene.layers.length : 0;
    },

    sceneMin(scene) {
      return scene.minSeconds ?? this.autopilot.minSeconds;
    },

    sceneMax(scene) {
      return scene.maxSeconds ?? this.autopilot.maxSeconds;
    },

    async setSceneMin(scene, value) {
      const updatedScene = await this.$modV.store.dispatch(
        "scenes/updateSceneTiming",
        {
          sceneId: scene.id,
          minSeconds: value,
        },
      );

      if (updatedScene) {
        this.persistScene(updatedScene, 500);
      }
    },

    async setSceneMax(scene, value) {
      const updatedScene = await this.$modV.store.dispatch(
        "scenes/updateSceneTiming",
        {
          sceneId: scene.id,
          maxSeconds: value,
        },
      );

      if (updatedScene) {
        this.persistScene(updatedScene, 500);
      }
    },

    persistScene(scene, delay = 0) {
      clearTimeout(this.sceneSaveTimer);

      this.sceneSaveTimer = setTimeout(() => {
        ipcRenderer.send("save-file", {
          what: "scene",
          name: scene.name,
          fileType: "json",
          project: this.currentProject,
          payload: JSON.stringify(scene),
        });
      }, delay);
    },

    startSceneRename(scene) {
      if (this.editingSceneId === scene.id) {
        return;
      }

      this.editingSceneId = scene.id;
      this.localSceneName = scene.name;

      this.$nextTick(() => {
        const refs = this.$refs.sceneNameInput;
        const input = Array.isArray(refs) ? refs[0] : refs;

        if (input) {
          input.focus();
        }
      });
    },

    cancelSceneRename() {
      this.editingSceneId = null;
    },

    async commitSceneRename(scene) {
      if (this.editingSceneId !== scene.id) {
        return;
      }

      const oldName = scene.name;
      const newName = this.localSceneName.trim();

      this.editingSceneId = null;

      if (!newName || newName === oldName) {
        return;
      }

      const updatedScene = await this.$modV.store.dispatch(
        "scenes/updateSceneName",
        {
          sceneId: scene.id,
          name: newName,
        },
      );

      if (!updatedScene) {
        return;
      }

      this.persistScene(updatedScene);

      ipcRenderer.send("delete-scene", {
        name: oldName,
        project: this.currentProject,
      });
    },

    nextScene() {
      if (!this.scenes.length) {
        return;
      }

      const currentIndex = this.scenes.findIndex(
        (scene) => scene.id === this.autopilot.currentSceneId,
      );
      const nextIndex = (currentIndex + 1) % this.scenes.length;

      this.applyScene(this.scenes[nextIndex]);
    },

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
      clearTimeout(this.sceneSaveTimer);

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

div.scene__details {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

div.scene__timing {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  opacity: 0.8;
}

span.scene__timingLabel {
  opacity: 0.6;
}

input.scene__timeInput {
  width: 52px;
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
