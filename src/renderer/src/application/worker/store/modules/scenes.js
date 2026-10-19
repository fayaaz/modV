/* eslint-env worker */
import store from "../";
import constants from "../../../constants";
import { v4 as uuidv4 } from "uuid";
import {
  createPhraseState,
  switchChance,
  updatePhrase,
} from "../../phrase-detector";

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
    autopilot: {
      enabled: false,
      currentSceneId: null,
      minSeconds: 8,
      maxSeconds: 24,
      transitionSeconds: 2,
      transition: "Fade",
    },
    transitioning: false,
  };
}

const state = getDefaultState();

/**
 * Phrase detection, dwell timing and crossfade bookkeeping live outside the
 * store so the per-frame tick doesn't flood the commitQueue with mutations.
 * Only meaningful changes (scene applied, autopilot config) are committed.
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
      minSeconds: state.autopilot.minSeconds,
      maxSeconds: state.autopilot.maxSeconds,
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
      data.minSeconds >= (scene.maxSeconds ?? state.autopilot.maxSeconds)
    ) {
      data.minSeconds = (scene.maxSeconds ?? state.autopilot.maxSeconds) - 1;
    }

    if (
      data.maxSeconds !== undefined &&
      data.maxSeconds <= (scene.minSeconds ?? state.autopilot.minSeconds)
    ) {
      data.maxSeconds = (scene.minSeconds ?? state.autopilot.minSeconds) + 1;
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

    commit("UPDATE_AUTOPILOT", { currentSceneId: scene.id });

    const transitionName = state.autopilot.transition || "Fade";

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

    const duration = Math.max(0.01, state.autopilot.transitionSeconds);
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

    if (state.autopilot.currentSceneId === sceneId) {
      commit("UPDATE_AUTOPILOT", { currentSceneId: null });
    }

    commit("REMOVE_SCENE", { sceneId });
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

  setAutopilot({ commit }, { enabled }) {
    dwell = 0;
    phrase = createPhraseState();

    commit("UPDATE_AUTOPILOT", { enabled });
  },

  updateAutopilot({ commit }, data) {
    commit("UPDATE_AUTOPILOT", data);
  },

  /**
   * Runs once per frame from the render loop: advances any active crossfade
   * and, when the autopilot is on, feeds the phrase detector with the bass
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

    const autopilot = state.autopilot;

    if (!autopilot.enabled || state.scenes.length < 2) {
      return;
    }

    const { bands } = store.state.meyda;
    const bass = bands ? bands.bass : 0;

    const phraseUpdate = updatePhrase(phrase, bass, dt);
    phrase = phraseUpdate.state;

    const currentScene = state.scenes.find(
      (scene) => scene.id === autopilot.currentSceneId,
    );
    const minSeconds = currentScene?.minSeconds ?? autopilot.minSeconds;
    const maxSeconds = currentScene?.maxSeconds ?? autopilot.maxSeconds;

    dwell += dt;

    const chance = switchChance(dwell, minSeconds, maxSeconds);
    const musical = phraseUpdate.flipped && Math.random() < chance;
    const forced = dwell >= maxSeconds;

    if ((musical || forced) && !transition) {
      dwell = 0;
      phrase = { ...phrase, t: 0 };

      const currentIndex = state.scenes.findIndex(
        (scene) => scene.id === autopilot.currentSceneId,
      );

      const nextIndex = (currentIndex + 1) % state.scenes.length;

      await dispatch("applyScene", { sceneId: state.scenes[nextIndex].id });
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

  UPDATE_AUTOPILOT(state, data) {
    Object.assign(state.autopilot, data);
  },

  SET_TRANSITIONING(state, transitioning) {
    state.transitioning = transitioning;
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
