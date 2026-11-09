<template>
  <div
    v-infoView="{ title: iVTitle, body: iVBody, id: 'Scenes Panel' }"
    v-searchTerms="{
      terms: [
        'scenes',
        'performance',
        'phrase',
        'crossfade',
        'grid',
        'rows',
        'layers',
        'clips',
      ],
      title: 'Scenes',
      type: 'Panel',
    }"
    class="scenes"
  >
    <grid columns="6" class="scenes__performance">
      <c span="1..">
        <grid columns="6">
          <c span="2">Performance mode</c>
          <c span="4">
            <Button
              class="light scenes__performanceButton"
              @click="togglePerformance"
            >
              {{
                performanceEnabled ? "Stop performance" : "Start performance"
              }}
            </Button>
            <Checkbox
              v-model="performanceEnabled"
              class="scenes__performanceCheckbox"
            />
          </c>
        </grid>
      </c>

      <c span="1..">
        <grid columns="6">
          <c span="2">Scene order</c>
          <c span="4">
            <Select v-model="sceneOrder" class="scenes__orderSelect">
              <option value="in-order">In order</option>
              <option value="random">Random</option>
            </Select>
          </c>
        </grid>
      </c>

      <c span="1..">
        <grid columns="6">
          <c span="2">Default min cell time</c>
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
          <c span="2">Default max cell time</c>
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
          <c span="2">Allowed transitions</c>
          <c span="4">
            <div class="scenes__transitionList">
              <div
                v-for="name in transitionOptions"
                :key="name"
                class="scenes__transitionOption"
                @click="setTransitionAllowed(name, !isTransitionAllowed(name))"
              >
                <Checkbox
                  :model-value="isTransitionAllowed(name)"
                  emit-boolean
                  @click.stop
                  @update:model-value="
                    (allowed) => setTransitionAllowed(name, allowed)
                  "
                />
                <span>{{ name }}</span>
              </div>
            </div>
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

    <div v-if="scenes.length" ref="gridWrapper" class="scenes__gridWrapper">
      <div class="scenes__grid" :style="{ gridTemplateColumns }">
        <div class="scenes__gridCorner">
          <span>Layers</span>
          <div class="scenes__gridCornerButtons">
            <button
              class="light scenes__addRowButton"
              title="Add a new empty Scene column"
              @click="addColumn"
            >
              + Column
            </button>
            <button
              class="light scenes__addRowButton"
              title="Add a new row"
              @click="addRow"
            >
              + Row
            </button>
          </div>
        </div>
        <div
          v-for="scene in scenes"
          :key="`header-${scene.id}`"
          class="scenes__gridHeader"
          :data-scene-id="scene.id"
          :class="{
            'scenes__gridHeader--active': sceneColumnActive(scene),
            'scenes__gridHeader--disabled': scene.enabled === false,
            'scenes__gridHeader--dragging': dragSceneId === scene.id,
            'scenes__gridHeader--drop':
              dragSceneId !== null && sceneDropTargetId === scene.id,
          }"
          :draggable="editingSceneId !== scene.id"
          :title="`Apply ${scene.name}. Double-click the name to rename. Drag to reorder. Right-click for more options.`"
          @click="onSceneHeaderClick(scene, $event)"
          @dragstart="onSceneDragStart(scene, $event)"
          @dragend="onSceneDragEnd"
          @dragover="onSceneDragOver(scene, $event)"
          @drop="onSceneDrop(scene, $event)"
          v-contextMenu="() => SceneHeaderContextMenu({ scene })"
        >
          <TextInput
            v-if="editingSceneId === scene.id"
            ref="sceneNameInput"
            v-model="localSceneName"
            class="scenes__sceneNameInput"
            @click.stop
            @keypress.enter="commitSceneRename(scene)"
            @blur="commitSceneRename(scene)"
            @keydown.esc="cancelSceneRename"
          />
          <span
            v-else
            class="scenes__gridHeaderName"
            @dblclick.stop="onSceneHeaderDblClick(scene)"
          >
            {{ scene.name }}
          </span>
          <label class="scenes__gridHeaderEnabled" @click.stop>
            <Checkbox
              :model-value="scene.enabled !== false"
              @click.stop
              @update:model-value="(value) => setSceneEnabled(scene, value)"
            />
          </label>
        </div>

        <template v-for="row in rows" :key="`row-${row.index}`">
          <div
            class="scenes__gridRowLabel"
            :class="{ 'scenes__gridRowLabel--dragging': dragRowIndex === row.index }"
            :draggable="editingRowIndex !== row.index"
            :title="row.label"
            @dragstart="onRowDragStart(row.index, $event)"
            @dragover.prevent
            @drop="onRowDrop(row.index, $event)"
          >
            <TextInput
              v-if="editingRowIndex === row.index"
              ref="rowNameInput"
              v-model="localRowName"
              class="scenes__rowNameInput"
              @keypress.enter="commitRowRename(row.index)"
              @blur="commitRowRename(row.index)"
              @keydown.esc="cancelRowRename"
            />
            <template v-else>
              <span class="scenes__gridRowLabelText" @dblclick="startRowRename(row.index)">
                {{ row.label }}
              </span>
              <button
                class="light scenes__rowDeleteButton"
                title="Delete row"
                @click="removeRow(row.index)"
              >
                &times;
              </button>
            </template>
          </div>
          <div
            v-for="scene in scenes"
            :key="`cell-${scene.id}-${row.index}`"
            class="scenes__gridCell"
            :data-scene-id="scene.id"
            :data-row-index="row.index"
            :class="{
              'scenes__gridCell--empty': isRowCellEmpty(scene, row.index),
              'scenes__gridCell--active': isRowActive(scene, row),
              'scenes__gridCell--transitioning': isRowTransitioning(scene, row),
              'scenes__gridCell--disabled': scene.enabled === false,
              'scenes__gridCell--paste':
                this.cellDragTarget &&
                this.cellDragTarget.sceneId === scene.id &&
                this.cellDragTarget.rowIndex === row.index,
            }"
            :draggable="true"
            v-contextMenu="() => sceneCellContextMenu(scene, row.index)"
            @click="
              isRowCellEmpty(scene, row.index)
                ? null
                : applySceneRow(scene, row.index)
            "
            @dragstart="onCellDragStart(scene, row.index, $event)"
            @dragover="onCellDragOver(scene, row.index, $event)"
            @drop="onCellDrop(scene, row.index, $event)"
            @dragend="onCellDragEnd"
            @mouseenter="showScenePreview(scene, row.index, $event)"
            @mouseleave="hideScenePreview"
          >
            <template v-if="!isRowCellEmpty(scene, row.index)">
              <span class="scenes__gridCellCount">
                {{ sceneGroupModuleCount(scene, sceneGroupForRow(scene, row.index)) }}
                module{{
                  sceneGroupModuleCount(scene, sceneGroupForRow(scene, row.index)) ===
                  1
                    ? ""
                    : "s"
                }}
              </span>
              <span class="scenes__gridCellModule">
                {{
                  sceneGroupModuleName(
                    scene,
                    sceneGroupForRow(scene, row.index),
                  )
                }}
              </span>
            </template>
            <span v-else class="scenes__gridCellPlaceholder">Empty</span>

            <div
              v-if="isRowTransitioning(scene, row)"
              class="scenes__gridProgress"
            >
              <div
                class="scenes__gridProgressBar"
                :style="{ width: `${Math.round(row.progress * 100)}%` }"
              />
            </div>

            <canvas
              class="scenes__gridPreviewCanvas"
              :class="{
                'scenes__gridPreviewCanvas--visible':
                  previewFrames[`${scene.id}:${row.index}`],
              }"
              width="320"
              height="180"
              aria-hidden="true"
            ></canvas>
          </div>
        </template>
      </div>
    </div>
    <p v-else class="scenes__empty">
      No Scenes yet. Save the current setup as a Scene to fill the grid.
    </p>

    <Dialog
      v-if="editGroupId"
      :title="editTitle"
      @close="closeSceneCellEditor"
    >
      <div class="scenes__groupEditor">
        <div class="scenes__editorLayout">
          <div class="scenes__editorMain">
            <Group v-if="editGroup" :group-id="editGroupId" />

            <grid columns="6" class="scenes__addModuleRow">
              <c span="4">
                <Select v-model="moduleToAdd">
                  <option value="">Add a module…</option>
                  <option
                    v-for="name in moduleOptions"
                    :key="name"
                    :value="name"
                  >
                    {{ name }}
                  </option>
                </Select>
              </c>
              <c span="2">
                <Button
                  class="light"
                  :disabled="!moduleToAdd"
                  @click="addModuleToEditedGroup"
                >
                  Add module
                </Button>
              </c>
            </grid>

            <ModuleInspector
              v-if="editFocusedModuleId"
              :module-id="editFocusedModuleId"
            />
          </div>

          <aside class="scenes__modulePanel">
            <div class="scenes__modulePanelTitle">Modules</div>
            <p class="scenes__modulePanelHint">
              Drag a module into the Group.
            </p>
            <Container
              behaviour="copy"
              group-name="modules"
              :get-child-payload="modulePanelPayload"
              class="scenes__modulePanelList"
            >
              <Draggable v-for="name in moduleOptions" :key="name">
                <div class="scenes__modulePanelItem" :data-module-name="name">
                  {{ name }}
                </div>
              </Draggable>
            </Container>
          </aside>
        </div>
      </div>
    </Dialog>

    <details class="scenes__manage">
      <summary>Manage Scenes</summary>

      <div class="scene-container">
        <div
          v-for="scene in scenes"
          :key="scene.id"
          class="scene"
          :class="{
            'scene--current': scene.id === performance.currentSceneId,
            'scene--disabled': scene.enabled === false,
          }"
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
              <Button
                v-if="scene.timingOverride"
                class="light scene__defaultsButton"
                title="Use the default cell times"
                @click="resetSceneTiming(scene)"
              >
                Defaults
              </Button>
            </div>
          </div>
          <div class="scene__controls">
            <label class="scene__enabled">
              <Checkbox
                :model-value="scene.enabled !== false"
                @update:model-value="(value) => setSceneEnabled(scene, value)"
              />
              <span>Enabled</span>
            </label>
            <Button class="light" @click="applyScene(scene)">Apply</Button>
            <Button class="light" @click="removeScene(scene)">Delete</Button>
          </div>
        </div>
      </div>
    </details>

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
        <Button class="light" @click="exportPlaylist">
          Export Playlist
        </Button>
        <Button class="light" @click="importPlaylist">
          Import Playlist
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
import { resolveSceneTiming } from "../application/worker/scene-timing";
import {
  getRowGroup,
  isRowCellEmpty as isSceneRowCellEmpty,
} from "../application/worker/scene-rows";
import { SceneHeaderContextMenu } from "../menus/context/sceneHeaderContextMenu.js";
import { SceneCellContextMenu } from "../menus/context/sceneCellContextMenu.js";
import { Container, Draggable } from "vue3-smooth-dnd";
import Dialog from "./Dialog.vue";
import Group from "./Group.vue";
import ModuleInspector from "./ModuleInspector.vue";

const { ipcRenderer } = window.electron;

export default {
  components: {
    Container,
    Draggable,
    // eslint-disable-next-line vue/no-reserved-component-names
    Dialog,
    Group,
    ModuleInspector,
  },

  data() {
    return {
      iVTitle: "Scenes",
      iVBody:
        "Scenes are full snapshots of the app: every Group, Module, Input Link, Expression, Tween and MIDI device is saved using the same JSON format as a preset. The grid shows Scenes as columns and Group layers as rows: click a column header to apply a whole Scene, or a cell to swap just that row's layer. Performance mode runs through the enabled Scenes, in order or randomly, switching on musical phrase changes detected from the bass audio band and crossfading the Groups between Scenes. Each cell independently waits a random time between the default min and max cell times before it advances, so rows change on their own schedule rather than all at once. Ctrl-drag a grid cell onto another Scene to copy its Group exactly. A Scene JSON can be opened like a preset, and all Scenes can be exported and imported as a playlist JSON.",
      newSceneName: "",
      editingSceneId: null,
      localSceneName: "",
      editingRowIndex: null,
      localRowName: "",
      dragRowIndex: null,
      dragSceneId: null,
      sceneDropTargetId: null,
      sceneClickTimer: null,
      cellDragSource: null,
      cellDragTarget: null,
      editSceneId: null,
      editRowIndex: null,
      editGroupId: null,
      moduleToAdd: "",
      sceneSaveTimer: null,
      previewKey: null,
      previewFrames: {},
    };
  },

  computed: {
    scenes() {
      return this.$modV.store.state.scenes.scenes;
    },

    rows() {
      return this.$modV.store.getters["scenes/performanceRows"];
    },

    scenePreviewState() {
      return this.$modV.store.state.scenePreview;
    },

    gridTemplateColumns() {
      return `120px repeat(${this.scenes.length}, minmax(140px, 1fr))`;
    },

    performance() {
      return this.$modV.store.state.scenes.performance;
    },

    rowNames() {
      return Array.isArray(this.performance.rowNames)
        ? this.performance.rowNames
        : [];
    },

    editGroup() {
      if (!this.editGroupId) {
        return null;
      }

      return (
        this.$modV.store.state.groups.groups.find(
          (group) => group.id === this.editGroupId,
        ) || null
      );
    },

    editFocusedModuleId() {
      const focused = this.$store.state.uiModules.focused;

      if (!focused || !this.editGroup) {
        return null;
      }

      return (this.editGroup.modules || []).includes(focused) ? focused : null;
    },

    editTitle() {
      const scene = this.scenes.find((item) => item.id === this.editSceneId);

      return `${scene ? scene.name : "Scene"} · ${
        this.editGroup ? this.editGroup.name : "Group"
      }`;
    },

    moduleOptions() {
      return Object.keys(this.$modV.store.state.modules.registered).sort(
        (a, b) => a.localeCompare(b),
      );
    },

    transitioning() {
      return this.$modV.store.state.scenes.transitioning;
    },

    currentScene() {
      return this.scenes.find(
        (scene) => scene.id === this.performance.currentSceneId,
      );
    },

    currentProject() {
      return this.$modV.store.state.projects.currentProject;
    },

    performanceEnabled: {
      get() {
        return this.performance.enabled;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/setPerformanceEnabled", {
          enabled: value,
        });
      },
    },

    sceneOrder: {
      get() {
        return this.performance.order || "in-order";
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updatePerformance", { order: value });
      },
    },

    minSeconds: {
      get() {
        return this.performance.minSeconds;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updatePerformance", {
          minSeconds: value,
        });
      },
    },

    maxSeconds: {
      get() {
        return this.performance.maxSeconds;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updatePerformance", {
          maxSeconds: value,
        });
      },
    },

    transitionSeconds: {
      get() {
        return this.performance.transitionSeconds;
      },

      set(value) {
        this.$modV.store.dispatch("scenes/updatePerformance", {
          transitionSeconds: value,
        });
      },
    },

    allowedTransitions() {
      const allowed = Array.isArray(this.performance.allowedTransitions)
        ? this.performance.allowedTransitions
        : [];
      const available = allowed.filter((name) =>
        this.transitionOptions.includes(name),
      );

      return available.length ? available : [FADE_TRANSITION];
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

  watch: {
    scenePreviewState: {
      deep: true,
      handler(state) {
        if (
          state &&
          state.active &&
          state.rendered &&
          state.sceneId !== null &&
          state.rowIndex !== null
        ) {
          this.previewFrames[`${state.sceneId}:${state.rowIndex}`] = true;
        }
      },
    },

    scenes() {
      this.pruneScenePreviews();
    },
  },

  created() {
    this.transferredPreviewCanvases = new WeakSet();
  },

  mounted() {
    this.loadFromDisk();
  },

  beforeUnmount() {
    clearTimeout(this.sceneSaveTimer);
    clearTimeout(this.sceneClickTimer);
    this.$modV.store.dispatch("scenePreview/dispose");
  },

  methods: {
    SceneHeaderContextMenu,

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

    applySceneRow(scene, rowIndex) {
      this.$modV.store.dispatch("scenes/applySceneRow", {
        sceneId: scene.id,
        rowIndex,
      });
    },

    sceneCellContextMenu(scene, rowIndex) {
      return SceneCellContextMenu({
        scene,
        rowIndex,
        group: this.sceneGroupForRow(scene, rowIndex),
        editEnabled: !this.performanceEnabled,
        clearEnabled: !this.isRowCellEmpty(scene, rowIndex),
        onEdit: () => this.openSceneCellEditor(scene, rowIndex),
        onClear: () => this.clearSceneCell(scene, rowIndex),
      });
    },

    async openSceneCellEditor(scene, rowIndex) {
      if (this.performanceEnabled) {
        return;
      }

      const active = this.$modV.store.getters["scenes/activeRow"](rowIndex);
      let groupId =
        active && active.sceneId === scene.id ? active.groupId : null;

      if (!groupId && this.isRowCellEmpty(scene, rowIndex)) {
        const created = await this.$modV.store.dispatch(
          "scenes/createSceneCellGroup",
          { sceneId: scene.id, rowIndex },
        );

        groupId = created && created.groupId;
      } else if (!groupId) {
        const result = await this.$modV.store.dispatch(
          "scenes/applySceneRow",
          {
            sceneId: scene.id,
            rowIndex,
          },
        );

        groupId = result && result.groupId;
      }

      if (!groupId) {
        return;
      }

      this.editSceneId = scene.id;
      this.editRowIndex = rowIndex;
      this.editGroupId = groupId;
      this.moduleToAdd = "";
    },

    async closeSceneCellEditor() {
      const sceneId = this.editSceneId;
      const rowIndex = this.editRowIndex;

      this.editSceneId = null;
      this.editRowIndex = null;
      this.editGroupId = null;
      this.moduleToAdd = "";

      if (!sceneId || rowIndex === null) {
        return;
      }

      const updated = await this.$modV.store.dispatch(
        "scenes/captureSceneRowToScene",
        { sceneId, rowIndex },
      );

      if (updated) {
        this.persistScene(updated);
      }
    },

    modulePanelPayload(index) {
      return {
        moduleName: this.moduleOptions[index],
        collection: "gallery",
      };
    },

    async addModuleToEditedGroup() {
      const moduleName = this.moduleToAdd;
      const groupId = this.editGroupId;

      if (!moduleName || !groupId) {
        return;
      }

      const module = await this.$modV.store.dispatch(
        "modules/makeActiveModule",
        { moduleName },
      );

      if (!module) {
        return;
      }

      this.$modV.store.commit("groups/ADD_MODULE_TO_GROUP", {
        moduleId: module.$id,
        groupId,
      });
      this.$store.commit("uiModules/SET_FOCUSED", module.$id);
      this.moduleToAdd = "";
    },

    async clearSceneCell(scene, rowIndex) {
      const group = this.sceneGroupForRow(scene, rowIndex);
      const groupName = (group && group.name) || `Row ${rowIndex + 1}`;
      const confirmed = window.confirm(
        `Remove all modules from "${groupName}" in "${scene.name}"?`,
      );

      if (!confirmed) {
        return;
      }

      const updated = await this.$modV.store.dispatch(
        "scenes/clearSceneCell",
        { sceneId: scene.id, rowIndex },
      );

      if (updated) {
        this.persistScene(updated);
        this.$store.commit("uiModules/SET_FOCUSED", null);
      }
    },

    scenePreviewSize(cell) {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      let width = Math.max(1, Math.round(cell.offsetWidth * dpr));
      let height = Math.max(1, Math.round(cell.offsetHeight * dpr));
      const scale = Math.min(1, 480 / width, 270 / height);

      width = Math.round(width * scale);
      height = Math.round(height * scale);

      return { width, height };
    },

    showScenePreview(scene, rowIndex, event) {
      if (this.isRowCellEmpty(scene, rowIndex)) {
        this.hideScenePreview();
        return;
      }

      const key = `${scene.id}:${rowIndex}`;
      const state = this.scenePreviewState;

      if (
        this.previewKey === key &&
        state.active &&
        state.sceneId === scene.id &&
        state.rowIndex === rowIndex
      ) {
        return;
      }

      const cell = event.currentTarget;
      const canvas = cell.querySelector(".scenes__gridPreviewCanvas");

      if (!canvas || !canvas.transferControlToOffscreen) {
        return;
      }

      const size = this.scenePreviewSize(cell);
      const payload = {
        key,
        sceneId: scene.id,
        rowIndex,
        width: size.width,
        height: size.height,
      };
      let transfer;

      if (!this.transferredPreviewCanvases.has(canvas)) {
        const offscreen = canvas.transferControlToOffscreen();
        this.transferredPreviewCanvases.add(canvas);
        payload.canvas = offscreen;
        transfer = [offscreen];
      }

      delete this.previewFrames[key];
      this.previewKey = key;
      this.$modV.store.dispatch("scenePreview/start", payload, transfer);
    },

    hideScenePreview() {
      const key = this.previewKey;

      if (key === null) {
        return;
      }

      this.previewKey = null;

      const state = this.scenePreviewState;
      const stateKey =
        state.sceneId === null ? null : `${state.sceneId}:${state.rowIndex}`;

      if (stateKey === key && state.rendered) {
        this.$modV.store.dispatch("scenePreview/freeze", { key });
        return;
      }

      this.$modV.store.dispatch("scenePreview/stop", { key });
    },

    pruneScenePreviews() {
      const valid = new Set();

      for (let i = 0; i < this.scenes.length; i++) {
        for (let j = 0; j < this.rows.length; j++) {
          valid.add(`${this.scenes[i].id}:${this.rows[j].index}`);
        }
      }

      for (const key of Object.keys(this.previewFrames)) {
        if (!valid.has(key)) {
          delete this.previewFrames[key];
        }
      }

      const state = this.scenePreviewState;

      if (state.sceneId !== null) {
        const activeKey = `${state.sceneId}:${state.rowIndex}`;

        if (!valid.has(activeKey) && this.previewKey === activeKey) {
          this.previewKey = null;
        }
      }

      this.$modV.store.dispatch("scenePreview/prune", {
        keys: Array.from(valid),
      });
    },

    isRowActive(scene, row) {
      return !!row && !!row.active && row.active.sceneId === scene.id;
    },

    sceneColumnActive(scene) {
      return this.rows.some((row) => this.isRowActive(scene, row));
    },

    onSceneHeaderClick(scene, event) {
      if (this.editingSceneId === scene.id || (event && event.detail > 1)) {
        return;
      }

      clearTimeout(this.sceneClickTimer);
      this.sceneClickTimer = setTimeout(() => {
        this.sceneClickTimer = null;
        this.applyScene(scene);
      }, 220);
    },

    onSceneHeaderDblClick(scene) {
      clearTimeout(this.sceneClickTimer);
      this.sceneClickTimer = null;
      this.startSceneRename(scene);
    },

    onSceneDragStart(scene, event) {
      if (this.editingSceneId === scene.id) {
        return;
      }

      clearTimeout(this.sceneClickTimer);
      this.sceneClickTimer = null;
      this.dragSceneId = scene.id;
      this.sceneDropTargetId = null;
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", `scene:${scene.id}`);
    },

    onSceneDragOver(scene, event) {
      if (!this.dragSceneId || this.dragSceneId === scene.id) {
        return;
      }

      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      this.sceneDropTargetId = scene.id;
    },

    async onSceneDrop(scene, event) {
      const payload = event.dataTransfer.getData("text/plain");
      const sceneId = payload.startsWith("scene:") ? payload.slice(6) : "";

      this.dragSceneId = null;
      this.sceneDropTargetId = null;

      if (!sceneId || sceneId === scene.id) {
        return;
      }

      event.preventDefault();

      const toIndex = this.scenes.findIndex((item) => item.id === scene.id);

      if (toIndex === -1) {
        return;
      }

      await this.$modV.store.dispatch("scenes/moveScene", {
        sceneId,
        toIndex,
      });
    },

    onSceneDragEnd() {
      this.dragSceneId = null;
      this.sceneDropTargetId = null;
    },

    sceneGroupForRow(scene, rowIndex) {
      return getRowGroup(scene, rowIndex, this.rowNames);
    },

    sceneGroupModuleCount(scene, group) {
      if (!group) {
        return 0;
      }

      const modules = group.modules;

      if (Array.isArray(modules)) {
        return modules.length;
      }

      if (modules && typeof modules === "object") {
        return Object.keys(modules).length;
      }

      return 0;
    },

    sceneGroupModuleName(scene, group) {
      if (!group) {
        return "";
      }

      const modules = group.modules;
      let moduleId = null;

      if (Array.isArray(modules)) {
        moduleId = modules[0];
      } else if (modules && typeof modules === "object") {
        moduleId = Object.keys(modules)[0];
      }

      if (!moduleId) {
        return "";
      }

      const module = scene.modules && scene.modules[moduleId];

      if (!module) {
        return "";
      }

      return (module.meta && module.meta.name) || module.$moduleName || "";
    },

    isRowCellEmpty(scene, rowIndex) {
      return isSceneRowCellEmpty(scene, rowIndex, this.rowNames);
    },

    startRowRename(rowIndex) {
      if (this.editingRowIndex === rowIndex) {
        return;
      }

      this.editingRowIndex = rowIndex;
      this.localRowName = this.rowNames[rowIndex] || "";

      this.$nextTick(() => {
        const refs = this.$refs.rowNameInput;
        const input = Array.isArray(refs) ? refs[0] : refs;

        if (input) {
          input.focus();
        }
      });
    },

    cancelRowRename() {
      this.editingRowIndex = null;
    },

    async commitRowRename(rowIndex) {
      if (this.editingRowIndex !== rowIndex) {
        return;
      }

      this.editingRowIndex = null;

      const name = (this.localRowName || "").trim();

      if (name === (this.rowNames[rowIndex] || "").trim()) {
        return;
      }

      const names = this.rowNames.slice();
      names[rowIndex] = name;

      await this.$modV.store.dispatch("scenes/setRowNames", { rowNames: names });
    },

    async addColumn() {
      const scene = await this.$modV.store.dispatch("scenes/addEmptyScene");

      if (!scene) {
        return;
      }

      await this.$nextTick();

      const wrapper = this.$refs.gridWrapper;

      if (wrapper) {
        wrapper.scrollLeft = wrapper.scrollWidth;
      }
    },

    async addRow() {
      const names = this.rowNames.concat("");
      await this.$modV.store.dispatch("scenes/setRowNames", { rowNames: names });
      this.startRowRename(names.length - 1);
    },

    async removeRow(rowIndex) {
      const names = this.rowNames.slice();

      names.splice(rowIndex, 1);
      await this.$modV.store.dispatch("scenes/setRowNames", { rowNames: names });
    },

    onRowDragStart(rowIndex, event) {
      this.dragRowIndex = rowIndex;
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", String(rowIndex));
    },

    async onRowDrop(rowIndex, event) {
      // A cell copy carries an empty text/plain payload, which would read as
      // row 0, and a Scene column carries "scene:<id>"; ignore row reordering
      // while either kind of drag is in progress.
      if (this.cellDragSource || this.dragSceneId !== null) {
        return;
      }

      const fromIndex = Number(event.dataTransfer.getData("text/plain"));
      this.dragRowIndex = null;

      if (!Number.isInteger(fromIndex) || fromIndex < 0) {
        return;
      }

      if (fromIndex === rowIndex) {
        return;
      }

      const names = this.rowNames.slice();
      const [moved] = names.splice(fromIndex, 1);
      names.splice(rowIndex, 0, moved);

      await this.$modV.store.dispatch("scenes/setRowNames", { rowNames: names });
    },

    onCellDragStart(scene, rowIndex, event) {
      // Only Ctrl+drag copies a cell; plain drags are ignored so the click-to-
      // apply behaviour and hover previews keep working as before.
      if (!event.ctrlKey && !event.metaKey) {
        // Cancel the drag entirely, otherwise a click with a little pointer
        // movement starts a data-less drag and swallows the click-to-apply.
        event.preventDefault();
        return;
      }

      event.dataTransfer.effectAllowed = "copy";
      // Chromium only starts a drag once some data is set, and the empty
      // payload keeps row-label drags (which put an index in text/plain)
      // from being mistaken for a cell drag.
      event.dataTransfer.setData("text/plain", "");
      this.cellDragSource = { sceneId: scene.id, rowIndex };
      this.cellDragTarget = null;
    },

    onCellDragEnd() {
      this.cellDragSource = null;
      this.cellDragTarget = null;
    },

    onCellDragOver(scene, rowIndex, event) {
      if (
        !this.cellDragSource ||
        (this.cellDragSource.sceneId === scene.id &&
          this.cellDragSource.rowIndex === rowIndex)
      ) {
        return;
      }

      // Without preventDefault the browser never fires `drop`; the highlight
      // is our own state, so the UI looks right while the drop is rejected.
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
      this.cellDragTarget = { sceneId: scene.id, rowIndex };
    },

    async onCellDrop(scene, rowIndex, event) {
      const source = this.cellDragSource;
      this.cellDragSource = null;
      this.cellDragTarget = null;

      if (!source) {
        return;
      }

      event.preventDefault();

      if (source.sceneId === scene.id && source.rowIndex === rowIndex) {
        return;
      }

      const updated = await this.$modV.store.dispatch(
        "scenes/copySceneCellToScene",
        {
          fromSceneId: source.sceneId,
          fromRowIndex: source.rowIndex,
          toSceneId: scene.id,
          toRowIndex: rowIndex,
        },
      );

      if (updated) {
        this.persistScene(updated);
      }
    },

    isRowTransitioning(scene, row) {
      return this.isRowActive(scene, row) && row.progress < 1;
    },

    sceneMin(scene) {
      return resolveSceneTiming(scene, this.performance).minSeconds;
    },

    sceneMax(scene) {
      return resolveSceneTiming(scene, this.performance).maxSeconds;
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

    async resetSceneTiming(scene) {
      const updatedScene = await this.$modV.store.dispatch(
        "scenes/resetSceneTiming",
        { sceneId: scene.id },
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

    isTransitionAllowed(name) {
      return this.allowedTransitions.includes(name);
    },

    setTransitionAllowed(name, allowed) {
      const current = this.allowedTransitions;
      let next = allowed
        ? [...new Set([...current, name])]
        : current.filter((transitionName) => transitionName !== name);

      if (!next.length) {
        next = [FADE_TRANSITION];
      }

      this.$modV.store.dispatch("scenes/updatePerformance", {
        allowedTransitions: next,
      });
    },

    nextScene() {
      this.$modV.store.dispatch("scenes/nextScene");
    },

    togglePerformance() {
      this.$modV.store.dispatch(
        this.performanceEnabled
          ? "scenes/stopPerformance"
          : "scenes/startPerformance",
      );
    },

    async setSceneEnabled(scene, enabled) {
      const updatedScene = await this.$modV.store.dispatch(
        "scenes/setSceneEnabled",
        { sceneId: scene.id, enabled },
      );

      if (updatedScene) {
        this.persistScene(updatedScene, 300);
      }
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

    async exportPlaylist() {
      const playlist = await this.$modV.store.dispatch(
        "scenes/exportPlaylist",
      );
      const result = await ipcRenderer.invoke(
        "export-scenes-playlist",
        playlist,
      );

      if (result && result.error) {
        console.error("Could not export the Scenes playlist", result.error);
      }
    },

    async importPlaylist() {
      const result = await ipcRenderer.invoke("import-scenes-playlist");

      if (!result || result.canceled) {
        return;
      }

      if (result.error) {
        console.error("Could not import the Scenes playlist", result.error);
        return;
      }

      await this.$modV.store.dispatch("scenes/importPlaylist", {
        playlist: result.playlist,
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

grid.scenes__performance {
  padding: 8px;
  grid-row-gap: 4px;
}

div.scenes__gridWrapper {
  padding: 0 8px 8px;
  overflow-x: auto;
}

div.scenes__grid {
  display: grid;
  gap: 3px;
  align-items: stretch;
  min-width: max-content;
  position: relative;
}

div.scenes__gridCorner,
div.scenes__gridHeader,
div.scenes__gridRowLabel,
div.scenes__gridCell {
  box-sizing: border-box;
  padding: 6px 8px;
  background: #282828;
  border-left: 3px solid transparent;
}

div.scenes__gridCorner,
div.scenes__gridRowLabel {
  position: sticky;
  left: 0;
  z-index: 2;
  font-size: 12px;
  font-weight: 600;
  opacity: 0.85;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

div.scenes__gridCorner {
  z-index: 3;
  background: #303030;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

div.scenes__gridHeader {
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  user-select: none;
  background: #303030;
}

div.scenes__gridHeader:hover {
  background: #3a3a3a;
}

div.scenes__gridHeader--active {
  border-left-color: #00c0ff;
}

div.scenes__gridHeader--disabled {
  opacity: 0.45;
}

div.scenes__gridHeader--dragging {
  opacity: 0.5;
}

div.scenes__gridHeader--drop {
  background: #3f4a56;
  box-shadow: inset 2px 0 0 #00c0ff;
}

span.scenes__gridHeaderName {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

label.scenes__gridHeaderEnabled {
  display: flex;
  align-items: center;
  cursor: pointer;
}

div.scenes__gridRowLabel {
  display: flex;
  align-items: center;
  gap: 4px;
  background: #303030;
}

span.scenes__gridRowLabelText {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

div.scenes__gridRowLabel--dragging {
  opacity: 0.5;
}

button.scenes__rowDeleteButton {
  opacity: 0;
  padding: 0 4px;
  font-size: 11px;
  line-height: 1;
  flex-shrink: 0;
}

div.scenes__gridRowLabel:hover button.scenes__rowDeleteButton {
  opacity: 0.7;
}

button.scenes__rowDeleteButton:hover {
  opacity: 1;
}

input.scenes__rowNameInput {
  width: 100%;
  min-width: 0;
}

div.scenes__gridCorner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
  flex-wrap: wrap;
}

div.scenes__gridCornerButtons {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

button.scenes__addRowButton {
  font-size: 11px;
  padding: 2px 6px;
  flex-shrink: 0;
}

div.scenes__gridCell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  aspect-ratio: 16 / 9;
  min-height: 42px;
  cursor: pointer;
  position: relative;
  overflow: hidden;
}

div.scenes__gridCell:hover {
  background: #303030;
}

div.scenes__gridCell--empty {
  cursor: default;
  background: #222;
}

div.scenes__gridCell--empty:hover {
  background: #222;
}

div.scenes__gridCell--disabled {
  opacity: 0.45;
}

div.scenes__gridCell--active {
  border-left-color: #00c0ff;
  background: #2d3539;
}

div.scenes__gridCell--transitioning {
  outline: 1px solid rgba(0, 192, 255, 0.45);
  outline-offset: -1px;
}

div.scenes__gridCell--paste {
  outline: 2px dashed rgba(0, 192, 255, 0.8);
  outline-offset: -2px;
}

span.scenes__gridCellCount {
  font-size: 11px;
  opacity: 0.6;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

span.scenes__gridCellModule {
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

span.scenes__gridCellPlaceholder {
  font-size: 12px;
  font-style: italic;
  opacity: 0.35;
}

div.scenes__gridProgress {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 2px;
  background: rgba(0, 192, 255, 0.18);
}

div.scenes__gridProgressBar {
  height: 100%;
  background: #00c0ff;
  transition: width 80ms linear;
}

canvas.scenes__gridPreviewCanvas {
  position: absolute;
  inset: 0;
  z-index: 1;
  display: block;
  width: 100%;
  height: 100%;
  background: #000;
  pointer-events: none;
  visibility: hidden;
  opacity: 0;
  transition: opacity 120ms linear;
}

canvas.scenes__gridPreviewCanvas--visible {
  visibility: visible;
  opacity: 1;
}

p.scenes__empty {
  margin: 0;
  padding: 4px 8px 12px;
  font-size: 12px;
  opacity: 0.6;
}

details.scenes__manage {
  padding: 0 8px;
}

details.scenes__manage > summary {
  cursor: pointer;
  padding: 6px 0;
  font-size: 12px;
  font-weight: 600;
  opacity: 0.8;
  user-select: none;
}

details.scenes__manage > summary:hover {
  opacity: 1;
}

div.scene-container {
  flex: 1;
  overflow-y: auto;
  padding: 0;
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

div.scene--disabled {
  opacity: 0.45;
}

label.scene__enabled {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  opacity: 0.85;
  user-select: none;
}

button.scenes__performanceButton {
  margin-right: 8px;
}

select.scenes__orderSelect {
  width: 100%;
}

div.scenes__transitionList {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  max-height: 96px;
  overflow-y: auto;
}

div.scenes__transitionOption {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  opacity: 0.85;
  cursor: pointer;
  user-select: none;
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

button.scene__defaultsButton {
  font-size: 11px;
  padding: 2px 6px;
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

div.scenes__groupEditor {
  min-width: 680px;
  max-height: 70vh;
  overflow-y: auto;
  overflow-x: hidden;
}

div.scenes__editorLayout {
  display: flex;
  gap: 12px;
  align-items: flex-start;
}

div.scenes__editorMain {
  flex: 1;
  min-width: 0;
}

aside.scenes__modulePanel {
  width: 180px;
  flex: 0 0 180px;
  display: flex;
  flex-direction: column;
  max-height: 60vh;
}

div.scenes__modulePanelTitle {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 4px;
}

p.scenes__modulePanelHint {
  font-size: 11px;
  opacity: 0.7;
  margin: 0 0 6px;
}

div.scenes__modulePanelList {
  overflow-y: auto;
  min-height: 60px;
}

div.scenes__modulePanelItem {
  padding: 4px 6px;
  margin-bottom: 4px;
  background: #303030;
  cursor: grab;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

div.scenes__modulePanelItem:hover {
  background: #3a3a3a;
}

input.scenes__sceneNameInput {
  width: 100%;
  min-width: 0;
}

grid.scenes__addModuleRow {
  padding: 8px 0;
  grid-row-gap: 4px;
}
</style>
