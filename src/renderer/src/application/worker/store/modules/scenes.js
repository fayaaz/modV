/* eslint-env worker */
import store from "../";
import constants, { GROUP_ENABLED } from "../../../constants";
import { v4 as uuidv4 } from "uuid";
import {
  createPhraseState,
  switchChance,
  updatePhrase,
} from "../../phrase-detector";
import {
  FADE_TRANSITION,
  NONE_TRANSITION,
  NATIVE_TRANSITION_NAMES,
} from "../../../scene-transition-defs";
import DEFAULT_SCENES from "../default-scenes";

const fs = require("fs");
const path = require("path");

/**
 * @typedef {Object} Scene
 *
 * A Scene is a snapshot of the Groups tab: every Group, its settings and its
 * ordered Modules (with their preset data).
 *
 * @property {String} id       UUID of the Scene
 * @property {String} name     Name of the Scene
 * @property {Array}  groups   Group snapshots, including their Module ids
 * @property {Object} modules  Preset data for each Module, keyed by Module id
 */

/**
 * @returns {Object}
 */
function getDefaultState() {
  return {
    scenes: [],
    defaultsSeeded: false,
    performance: {
      enabled: false,
      currentSceneId: null,
      minSeconds: 8,
      maxSeconds: 24,
      transitionSeconds: 2,
      allowedTransitions: [],
      order: "in-order",
    },
    transitioning: false,
  };
}

const state = getDefaultState();

/**
 * Phrase detection, dwell timing and crossfade bookkeeping live outside the
 * store so the per-frame tick doesn't flood the commitQueue with mutations.
 * Only meaningful changes (scene applied, performance config) are committed.
 */
let phrase = createPhraseState();
let dwell = 0;
let transition = null;
let pendingCleanup = [];

const getters = {
  getById: (state) => (sceneId) =>
    state.scenes.find((scene) => scene.id === sceneId),

  groupBySceneId: (state, getters, rootState) => (sceneId) =>
    rootState.groups.groups.find((group) => group.sceneId === sceneId),

  transitionProgress: () => {
    if (!transition) {
      return 1;
    }

    return Math.min(1, transition.elapsed / transition.duration);
  },

  currentTransitionName: () => (transition ? transition.name : null),

  transitionGroupIds: () => {
    if (!transition) {
      return null;
    }

    return {
      from: transition.from.map((item) => item.groupId),
      to: transition.to.map((item) => item.groupId),
    };
  },
};

function isGalleryGroup(group) {
  return group.name === constants.GALLERY_GROUP_NAME;
}

function isSceneEnabled(scene) {
  return scene.enabled !== false;
}

function getEnabledScenes(scenes) {
  return scenes.filter(isSceneEnabled);
}

function normalizeTransitionList(value) {
  if (!Array.isArray(value)) {
    return [];
  }

  const names = [];

  for (let i = 0, len = value.length; i < len; i++) {
    const name = value[i];

    if (typeof name === "string" && name && !names.includes(name)) {
      names.push(name);
    }
  }

  return names;
}

function normalizePerformanceData(data) {
  const normalized = { ...(data || {}) };
  const hasAllowedTransitions = Object.prototype.hasOwnProperty.call(
    normalized,
    "allowedTransitions",
  );
  const legacyTransition = normalized.transition;

  delete normalized.transition;

  const allowedTransitions = hasAllowedTransitions
    ? normalizeTransitionList(normalized.allowedTransitions)
    : [];

  if (allowedTransitions.length) {
    normalized.allowedTransitions = allowedTransitions;
  } else if (typeof legacyTransition === "string" && legacyTransition) {
    normalized.allowedTransitions = [legacyTransition];
  } else if (hasAllowedTransitions) {
    normalized.allowedTransitions = [];
  }

  return normalized;
}

function getAvailableTransitionNames() {
  const names = [FADE_TRANSITION, NONE_TRANSITION, ...NATIVE_TRANSITION_NAMES];
  const registered = store.state.modules.registered || {};

  for (const module of Object.values(registered)) {
    if (
      module.props &&
      module.props.startImage &&
      module.props.endImage &&
      module.props.progress
    ) {
      names.push(module.meta.name);
    }
  }

  return names;
}

function pickTransitionName(performance) {
  const available = new Set(getAvailableTransitionNames());
  const allowed = normalizeTransitionList(
    performance.allowedTransitions,
  ).filter((name) => available.has(name));

  if (!allowed.length) {
    return FADE_TRANSITION;
  }

  return allowed[Math.floor(Math.random() * allowed.length)];
}

function pickNextScene(scenes, currentSceneId, order) {
  const enabledScenes = getEnabledScenes(scenes);

  if (!enabledScenes.length) {
    return null;
  }

  if (enabledScenes.length === 1) {
    return enabledScenes[0];
  }

  if (order === "random") {
    const pool = enabledScenes.filter((scene) => scene.id !== currentSceneId);

    return pool[Math.floor(Math.random() * pool.length)];
  }

  const currentIndex = enabledScenes.findIndex(
    (scene) => scene.id === currentSceneId,
  );
  const nextIndex =
    currentIndex === -1 ? 0 : (currentIndex + 1) % enabledScenes.length;

  return enabledScenes[nextIndex];
}

/**
 * Scenes saved before the Groups snapshot format used a single Group and a
 * flat `layers` array. Turn those into the current shape when loading.
 */
function normalizeScene(scene) {
  if (!scene || !scene.id) {
    return null;
  }

  if (Array.isArray(scene.groups)) {
    return scene;
  }

  if (!Array.isArray(scene.layers)) {
    return null;
  }

  const modules = {};

  for (let i = 0, len = scene.layers.length; i < len; i++) {
    const layer = scene.layers[i];

    if (layer && layer.$id) {
      modules[layer.$id] = layer;
    }
  }

  return {
    id: scene.id,
    name: scene.name,
    groups: [
      {
        ...(scene.group || {}),
        name: scene.name,
        modules: Object.keys(modules),
      },
    ],
    modules,
  };
}

const SCENES_STORAGE_FILENAME = "scenes.json";

function getScenesStoragePath() {
  if (!self.__userData) {
    return null;
  }

  return path.join(self.__userData, SCENES_STORAGE_FILENAME);
}

async function readStoredScenes() {
  const filePath = getScenesStoragePath();

  if (!filePath) {
    return null;
  }

  try {
    const fileBuffer = await fs.promises.readFile(filePath);
    const stored = JSON.parse(fileBuffer.toString());

    return stored && typeof stored === "object" ? stored : null;
  } catch (e) {
    if (e.code !== "ENOENT") {
      console.error("Could not read the saved Scene list", e);
    }

    return null;
  }
}

async function writeStoredScenes(state) {
  const filePath = getScenesStoragePath();

  if (!filePath) {
    return;
  }

  const performance = { ...state.performance };
  delete performance.currentSceneId;

  const payload = {
    version: 1,
    defaultsSeeded: !!state.defaultsSeeded,
    scenes: JSON.parse(JSON.stringify(state.scenes)),
    performance,
  };

  try {
    await fs.promises.mkdir(path.dirname(filePath), { recursive: true });
    await fs.promises.writeFile(filePath, JSON.stringify(payload));
  } catch (e) {
    console.error("Could not save the Scene list", e);
  }
}

/**
 * Builds a Scene snapshot from a declarative definition by briefly creating the
 * Groups and Modules in the live store, then tearing them down again. This
 * keeps the default Scenes independent of the internal Module/Group snapshot
 * shape: they are produced by the same code path as a user-saved Scene.
 */
async function buildSceneSnapshot(definition) {
  const createdGroupIds = [];

  try {
    for (let i = 0, len = definition.groups.length; i < len; i++) {
      const groupDefinition = definition.groups[i];
      const group = await store.dispatch("groups/createGroup", {
        name: groupDefinition.name,
        enabled: GROUP_ENABLED,
        inherit: groupDefinition.inherit ?? true,
        inheritFrom: -1,
        clearing: groupDefinition.clearing ?? true,
        alpha: groupDefinition.alpha ?? 1,
        compositeOperation: groupDefinition.compositeOperation || "normal",
        pipeline: groupDefinition.pipeline ?? false,
      });

      createdGroupIds.push(group.id);

      const moduleDefinitions = groupDefinition.modules || [];

      for (
        let j = 0, moduleLen = moduleDefinitions.length;
        j < moduleLen;
        j++
      ) {
        const moduleDefinition = moduleDefinitions[j];

        if (!store.state.modules.registered[moduleDefinition.name]) {
          console.warn(
            `Default Scene "${definition.name}" could not find Module "${moduleDefinition.name}"`,
          );
          continue;
        }

        const module = await store.dispatch("modules/makeActiveModule", {
          moduleName: moduleDefinition.name,
          moduleMeta: {
            enabled: true,
            ...(moduleDefinition.meta || {}),
          },
        });

        if (!module) {
          continue;
        }

        store.commit("groups/ADD_MODULE_TO_GROUP", {
          moduleId: module.$id,
          groupId: group.id,
        });

        const propNames = Object.keys(moduleDefinition.props || {});

        for (let k = 0, propLen = propNames.length; k < propLen; k++) {
          const propName = propNames[k];

          if (!module.$props[propName]) {
            continue;
          }

          await store.dispatch("modules/updateProp", {
            moduleId: module.$id,
            prop: propName,
            data: moduleDefinition.props[propName],
          });
        }
      }
    }

    const groupSnapshots = [];
    const moduleIds = [];

    for (let i = 0, len = createdGroupIds.length; i < len; i++) {
      const group = store.getters["groups/getById"](createdGroupIds[i]);

      if (!group) {
        continue;
      }

      const snapshot = { ...group };
      delete snapshot.context;
      delete snapshot.sceneId;
      groupSnapshots.push(JSON.parse(JSON.stringify(snapshot)));
      moduleIds.push(...group.modules);
    }

    const presetData = await store.dispatch("modules/createPresetData");
    const moduleSnapshots = {};

    for (let i = 0, len = moduleIds.length; i < len; i++) {
      const moduleId = moduleIds[i];

      if (presetData[moduleId]) {
        moduleSnapshots[moduleId] = presetData[moduleId];
      }
    }

    return {
      id: uuidv4(),
      name: definition.name,
      enabled: definition.enabled !== false,
      minSeconds: definition.minSeconds,
      maxSeconds: definition.maxSeconds,
      groups: groupSnapshots,
      modules: moduleSnapshots,
    };
  } finally {
    for (let i = 0, len = createdGroupIds.length; i < len; i++) {
      try {
        await store.dispatch("groups/removeGroup", {
          groupId: createdGroupIds[i],
        });
      } catch (e) {
        console.error("Could not clean up a default Scene Group", e);
      }
    }
  }
}

async function cleanupPendingGroups() {
  const groupIds = pendingCleanup.splice(0);

  for (let i = 0, len = groupIds.length; i < len; i++) {
    const groupId = groupIds[i];
    const exists = store.state.groups.groups.some(
      (group) => group.id === groupId,
    );

    if (exists) {
      await store.dispatch("groups/removeGroup", { groupId });
    }
  }
}

async function completeTransition(commit) {
  if (transition) {
    for (let i = 0, len = transition.to.length; i < len; i++) {
      const item = transition.to[i];

      store.commit("groups/UPDATE_GROUP", {
        groupId: item.groupId,
        data: { alpha: item.alpha },
      });
    }

    transition = null;
  }

  await cleanupPendingGroups();
  commit("SET_TRANSITIONING", false);
}

const actions = {
  /**
   * Snapshots every Group in the Groups tab, including each Group's ordered
   * Modules and their preset data, into a new Scene.
   */
  async saveCurrentAsScene({ commit, state }, args = {}) {
    const name = args.name || `Scene ${state.scenes.length + 1}`;

    await completeTransition(commit);

    const groups = store.state.groups.groups
      .filter((group) => !isGalleryGroup(group))
      .map((group) => {
        const snapshot = { ...group };
        delete snapshot.context;
        delete snapshot.sceneId;
        return snapshot;
      });

    const modules = await store.dispatch("modules/createPresetData");

    const scene = {
      id: uuidv4(),
      name,
      enabled: true,
      minSeconds: state.performance.minSeconds,
      maxSeconds: state.performance.maxSeconds,
      groups: JSON.parse(JSON.stringify(groups)),
      modules: JSON.parse(JSON.stringify(modules)),
    };

    commit("ADD_SCENE", scene);

    return JSON.parse(JSON.stringify(scene));
  },

  updateSceneName({ commit, state }, { sceneId, name }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);
    const newName = (name || "").trim();

    if (!scene || !newName) {
      return null;
    }

    const duplicate = state.scenes.some(
      (other) =>
        other.id !== sceneId &&
        other.name.toLowerCase() === newName.toLowerCase(),
    );

    if (duplicate) {
      return null;
    }

    commit("UPDATE_SCENE", { sceneId, data: { name: newName } });

    return JSON.parse(JSON.stringify(scene));
  },

  updateSceneTiming({ commit, state }, { sceneId, minSeconds, maxSeconds }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (!scene) {
      return null;
    }

    const data = {};

    if (Number.isFinite(minSeconds)) {
      data.minSeconds = Math.max(1, Math.round(minSeconds * 10) / 10);
    }

    if (Number.isFinite(maxSeconds)) {
      data.maxSeconds = Math.max(2, Math.round(maxSeconds * 10) / 10);
    }

    if (
      data.minSeconds !== undefined &&
      data.minSeconds >= (scene.maxSeconds ?? state.performance.maxSeconds)
    ) {
      data.minSeconds = (scene.maxSeconds ?? state.performance.maxSeconds) - 1;
    }

    if (
      data.maxSeconds !== undefined &&
      data.maxSeconds <= (scene.minSeconds ?? state.performance.minSeconds)
    ) {
      data.maxSeconds = (scene.minSeconds ?? state.performance.minSeconds) + 1;
    }

    commit("UPDATE_SCENE", { sceneId, data });

    return JSON.parse(JSON.stringify(scene));
  },

  /**
   * Applies a Scene by rebuilding all of its Groups and Modules, then
   * crossfading the previous Groups out and the new Groups in. This replaces
   * the Groups tab with the saved snapshot.
   */
  async applyScene({ commit, state }, { sceneId }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (!scene) {
      return;
    }

    const normalizedScene = normalizeScene(scene);

    if (!normalizedScene) {
      return;
    }

    // Finish any in-flight crossfade first so nothing is left orphaned.
    await completeTransition(commit);

    const oldGroups = store.state.groups.groups.filter(
      (group) => !isGalleryGroup(group),
    );
    const groupSnapshots = normalizedScene.groups || [];
    const moduleSnapshots = normalizedScene.modules || {};
    const createdGroups = [];
    const groupIdMap = {};

    for (let i = 0, len = groupSnapshots.length; i < len; i++) {
      const snapshot = groupSnapshots[i];
      const oldGroupId = snapshot.id;
      const moduleIds = snapshot.modules || [];

      const groupData = { ...snapshot };
      delete groupData.id;
      delete groupData.modules;
      delete groupData.context;
      delete groupData.sceneId;
      delete groupData.alphaInputId;
      delete groupData.enabledInputId;
      delete groupData.clearingInputId;
      delete groupData.inheritInputId;
      delete groupData.compositeOperationInputId;
      delete groupData.pipelineInputId;

      const targetAlpha = snapshot.alpha ?? 1;

      const group = await store.dispatch("groups/createGroup", {
        ...groupData,
        alpha: 0,
        sceneId: scene.id,
      });

      if (oldGroupId) {
        groupIdMap[oldGroupId] = group.id;
      }

      createdGroups.push({ groupId: group.id, alpha: targetAlpha });

      for (
        let j = 0, moduleIdsLength = moduleIds.length;
        j < moduleIdsLength;
        j++
      ) {
        const moduleId = moduleIds[j];
        const moduleSnapshot = moduleSnapshots[moduleId];

        if (!moduleSnapshot) {
          continue;
        }

        const module = await store.dispatch("modules/makeActiveModule", {
          moduleName: moduleSnapshot.$moduleName,
          existingModule: moduleSnapshot,
          generateNewIds: true,
        });

        if (!module) {
          continue;
        }

        store.commit("groups/ADD_MODULE_TO_GROUP", {
          moduleId: module.$id,
          groupId: group.id,
        });
      }
    }

    // Remap inheritFrom now that every Group has its new id, so Groups that
    // appear later in the snapshot still resolve correctly.
    for (let i = 0, len = groupSnapshots.length; i < len; i++) {
      const snapshot = groupSnapshots[i];

      if (typeof snapshot.inheritFrom !== "string") {
        continue;
      }

      const groupId = groupIdMap[snapshot.id];

      if (groupId) {
        store.commit("groups/UPDATE_GROUP", {
          groupId,
          data: { inheritFrom: groupIdMap[snapshot.inheritFrom] ?? -1 },
        });
      }
    }

    commit("UPDATE_PERFORMANCE", { currentSceneId: scene.id });

    const transitionName = pickTransitionName(state.performance);

    const applyTargetAlphas = () => {
      for (let i = 0, len = createdGroups.length; i < len; i++) {
        const { groupId, alpha } = createdGroups[i];

        store.commit("groups/UPDATE_GROUP", {
          groupId,
          data: { alpha },
        });
      }
    };

    if (!oldGroups.length) {
      applyTargetAlphas();
      return;
    }

    if (transitionName === "None") {
      applyTargetAlphas();

      const oldGroupIds = oldGroups.map((group) => group.id);

      for (let i = 0, len = oldGroupIds.length; i < len; i++) {
        await store.dispatch("groups/removeGroup", {
          groupId: oldGroupIds[i],
        });
      }

      return;
    }

    const duration = Math.max(0.01, state.performance.transitionSeconds);
    const shader = transitionName !== "Fade";

    // Shader transitions composite the two Scenes themselves, so the Groups
    // need their final opacity straight away.
    if (shader) {
      applyTargetAlphas();
    }

    transition = {
      name: transitionName,
      shader,
      from: oldGroups.map((group) => ({
        groupId: group.id,
        alpha: group.alpha,
      })),
      to: createdGroups,
      elapsed: 0,
      duration,
    };

    pendingCleanup.push(...oldGroups.map((group) => group.id));
    commit("SET_TRANSITIONING", true);
  },

  async removeScene({ commit, state }, { sceneId }) {
    await completeTransition(commit);

    const groups = store.state.groups.groups.filter(
      (group) => group.sceneId === sceneId,
    );

    for (let i = 0, len = groups.length; i < len; i++) {
      await store.dispatch("groups/removeGroup", {
        groupId: groups[i].id,
      });
    }

    if (state.performance.currentSceneId === sceneId) {
      commit("UPDATE_PERFORMANCE", { currentSceneId: null });
    }

    commit("REMOVE_SCENE", { sceneId });
  },

  /**
   * Restores the Scene list that was persisted to the app's user data folder
   * by a previous session.
   */
  async hydrateScenes({ commit, state }) {
    const stored = await readStoredScenes();

    if (!stored) {
      return;
    }

    if (Array.isArray(stored.scenes)) {
      for (let i = 0, len = stored.scenes.length; i < len; i++) {
        const scene = normalizeScene(stored.scenes[i]);

        if (scene) {
          commit("ADD_SCENE", scene);
        }
      }
    }

    const storedPerformance =
      stored.performance && typeof stored.performance === "object"
        ? stored.performance
        : stored.autopilot && typeof stored.autopilot === "object"
          ? stored.autopilot
          : null;

    if (storedPerformance) {
      commit("UPDATE_PERFORMANCE", {
        ...normalizePerformanceData(storedPerformance),
        currentSceneId: state.performance.currentSceneId,
      });
    }

    if (stored.defaultsSeeded) {
      commit("SET_DEFAULTS_SEEDED", true);
    }
  },

  /**
   * Seeds the ready-to-go music reactive Scenes on first run. The list is
   * layered into Background / Foreground / Effects Groups, built with the same
   * snapshot path as a user-saved Scene.
   */
  async ensureDefaultScenes({ commit, dispatch, state }) {
    if (state.defaultsSeeded) {
      return;
    }

    commit("SET_DEFAULTS_SEEDED", true);

    if (!state.scenes.length) {
      for (let i = 0, len = DEFAULT_SCENES.length; i < len; i++) {
        try {
          const scene = await buildSceneSnapshot(DEFAULT_SCENES[i]);

          if (scene) {
            commit("ADD_SCENE", scene);
          }
        } catch (e) {
          console.error(
            `Could not create default Scene "${DEFAULT_SCENES[i].name}"`,
            e,
          );
        }
      }
    }

    await dispatch("persistScenes");
  },

  async persistScenes({ state }) {
    await writeStoredScenes(state);
  },

  /**
   * Reads saved Scene JSON files from the current project's scene folder.
   * The Electron main process (media manager watcher) discovers the files;
   * this reads their contents, mirroring how presets are loaded.
   */
  async loadScenesFromDisk({ commit, rootState }) {
    const { media, path: mediaPath } = rootState.media;
    const project = rootState.projects.currentProject;

    if (!mediaPath || !media[project] || !media[project].scene) {
      return;
    }

    const items = Object.values(media[project].scene);

    for (let i = 0, len = items.length; i < len; i++) {
      const item = items[i];

      try {
        const fileBuffer = await fs.promises.readFile(
          path.join(mediaPath, item.path),
        );
        const scene = normalizeScene(JSON.parse(fileBuffer.toString()));

        if (scene) {
          commit("ADD_SCENE", scene);
        }
      } catch (e) {
        console.error(`Could not load scene "${item.name}"`, e);
      }
    }
  },

  setPerformanceEnabled({ commit }, { enabled }) {
    dwell = 0;
    phrase = createPhraseState();

    commit("UPDATE_PERFORMANCE", { enabled });
  },

  async startPerformance({ commit, state, dispatch }) {
    dwell = 0;
    phrase = createPhraseState();

    commit("UPDATE_PERFORMANCE", { enabled: true });

    const enabledScenes = getEnabledScenes(state.scenes);

    if (!enabledScenes.length) {
      return;
    }

    const currentSceneEnabled = enabledScenes.some(
      (scene) => scene.id === state.performance.currentSceneId,
    );

    if (!currentSceneEnabled) {
      await dispatch("nextScene");
    }
  },

  stopPerformance({ commit }) {
    dwell = 0;
    commit("UPDATE_PERFORMANCE", { enabled: false });
  },

  async nextScene({ state, dispatch }) {
    const scene = pickNextScene(
      state.scenes,
      state.performance.currentSceneId,
      state.performance.order,
    );

    if (!scene) {
      return;
    }

    await dispatch("applyScene", { sceneId: scene.id });
  },

  setSceneEnabled({ commit, state }, { sceneId, enabled }) {
    const scene = state.scenes.find((item) => item.id === sceneId);

    if (!scene) {
      return null;
    }

    commit("UPDATE_SCENE", {
      sceneId,
      data: { enabled: !!enabled },
    });

    return JSON.parse(JSON.stringify(scene));
  },

  updatePerformance({ commit }, data) {
    commit("UPDATE_PERFORMANCE", normalizePerformanceData(data));
  },

  /**
   * Runs once per frame from the render loop: advances any active crossfade
   * and, when performance mode is on, feeds the phrase detector with the bass
   * band and switches scenes on phrase changes (or when dwell passes
   * maxSeconds).
   *
   * With no audio the bass band sits at 0, so the detector can never flip;
   * the dwell timer alone cycles scenes at maxSeconds.
   */
  async tick({ commit, state, dispatch }, { delta }) {
    const dt = Math.min(0.1, (delta || 0) / 1000);

    if (transition) {
      transition.elapsed += dt;

      const progress = Math.min(1, transition.elapsed / transition.duration);

      if (!transition.shader) {
        const eased = progress * progress * (3 - 2 * progress);

        for (let i = 0, len = transition.to.length; i < len; i++) {
          const item = transition.to[i];

          store.commit("groups/UPDATE_GROUP", {
            groupId: item.groupId,
            data: { alpha: item.alpha * eased },
          });
        }

        for (let i = 0, len = transition.from.length; i < len; i++) {
          const item = transition.from[i];

          store.commit("groups/UPDATE_GROUP", {
            groupId: item.groupId,
            data: { alpha: item.alpha * (1 - eased) },
          });
        }
      }

      if (progress >= 1) {
        for (let i = 0, len = transition.to.length; i < len; i++) {
          const item = transition.to[i];

          store.commit("groups/UPDATE_GROUP", {
            groupId: item.groupId,
            data: { alpha: item.alpha },
          });
        }

        transition = null;

        await cleanupPendingGroups();
        commit("SET_TRANSITIONING", false);
      }
    }

    const performance = state.performance;
    const enabledScenes = getEnabledScenes(state.scenes);

    if (!performance.enabled || !enabledScenes.length) {
      return;
    }

    const currentScene = state.scenes.find(
      (scene) => scene.id === performance.currentSceneId,
    );

    if (!currentScene || !isSceneEnabled(currentScene)) {
      if (!transition) {
        await dispatch("nextScene");
      }

      return;
    }

    if (enabledScenes.length < 2) {
      return;
    }

    const { bands } = store.state.meyda;
    const bass = bands ? bands.bass : 0;

    const phraseUpdate = updatePhrase(phrase, bass, dt);
    phrase = phraseUpdate.state;

    const minSeconds = currentScene?.minSeconds ?? performance.minSeconds;
    const maxSeconds = currentScene?.maxSeconds ?? performance.maxSeconds;

    dwell += dt;

    const chance = switchChance(dwell, minSeconds, maxSeconds);
    const musical = phraseUpdate.flipped && Math.random() < chance;
    const forced = dwell >= maxSeconds;

    if ((musical || forced) && !transition) {
      dwell = 0;
      phrase = { ...phrase, t: 0 };

      await dispatch("nextScene");
    }
  },
};

const mutations = {
  ADD_SCENE(state, scene) {
    const index = state.scenes.findIndex((item) => item.id === scene.id);

    if (index > -1) {
      state.scenes.splice(index, 1, scene);
      return;
    }

    state.scenes.push(scene);
  },

  REMOVE_SCENE(state, { sceneId }) {
    const index = state.scenes.findIndex((scene) => scene.id === sceneId);

    if (index > -1) {
      state.scenes.splice(index, 1);
    }
  },

  UPDATE_SCENE(state, { sceneId, data }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (scene) {
      Object.assign(scene, data);
    }
  },

  UPDATE_PERFORMANCE(state, data) {
    Object.assign(state.performance, data);
  },

  SET_TRANSITIONING(state, transitioning) {
    state.transitioning = transitioning;
  },

  SET_DEFAULTS_SEEDED(state, defaultsSeeded) {
    state.defaultsSeeded = defaultsSeeded;
  },

  RESET_STATE(state) {
    Object.assign(state, getDefaultState());
  },
};

export default {
  namespaced: true,
  state,
  getters,
  actions,
  mutations,
};
