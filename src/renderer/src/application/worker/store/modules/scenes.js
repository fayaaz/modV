/* eslint-env worker */
import store from "../";
import { GROUP_ENABLED } from "../../../constants";
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
 * @property {String} id       UUID of the Scene
 * @property {String} name     Name of the Scene
 * @property {Array}  layers   Preset data for each Module in the Scene
 * @property {Object} group    Group settings the Scene draws into
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
};

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
    store.commit("groups/UPDATE_GROUP", {
      groupId: transition.toGroupId,
      data: { alpha: 1 },
    });

    transition = null;
  }

  await cleanupPendingGroups();
  commit("SET_TRANSITIONING", false);
}

const actions = {
  /**
   * Snapshots the currently active Modules (preset-style) into a new Scene.
   */
  async saveCurrentAsScene({ commit, state, rootState }, args = {}) {
    const name = args.name || `Scene ${state.scenes.length + 1}`;

    const layers = Object.values(
      await store.dispatch("modules/createPresetData"),
    );

    const currentGroup = state.autopilot.currentSceneId
      ? rootState.groups.groups.find(
          (group) => group.sceneId === state.autopilot.currentSceneId,
        )
      : undefined;

    const group = {
      name,
      alpha: 1,
      enabled: GROUP_ENABLED,
      clearing: currentGroup ? currentGroup.clearing : true,
      inherit: currentGroup ? currentGroup.inherit : false,
      inheritFrom: currentGroup ? currentGroup.inheritFrom : -1,
      pipeline: currentGroup ? currentGroup.pipeline : false,
      compositeOperation: currentGroup
        ? currentGroup.compositeOperation
        : "normal",
    };

    const scene = {
      id: uuidv4(),
      name,
      layers,
      group,
    };

    commit("ADD_SCENE", scene);

    return JSON.parse(JSON.stringify(scene));
  },

  /**
   * Applies a Scene: its Modules are loaded into a dedicated Group (tagged
   * with sceneId) and crossfaded against the previously applied Scene Group.
   * Groups belonging to the user (not tagged with sceneId) are left alone.
   */
  async applyScene({ commit, state, rootState }, { sceneId }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (!scene) {
      return;
    }

    // Finish any in-flight crossfade first so nothing is left orphaned.
    await completeTransition(commit);

    let previousGroup = state.autopilot.currentSceneId
      ? rootState.groups.groups.find(
          (group) => group.sceneId === state.autopilot.currentSceneId,
        )
      : undefined;

    // Re-applying a Scene rebuilds its Group from scratch.
    const existingTarget = rootState.groups.groups.find(
      (group) => group.sceneId === scene.id,
    );

    if (existingTarget) {
      await store.dispatch("groups/removeGroup", {
        groupId: existingTarget.id,
      });

      if (previousGroup && previousGroup.id === existingTarget.id) {
        previousGroup = undefined;
      }
    }

    const group = await store.dispatch("groups/createGroup", {
      ...scene.group,
      name: scene.name,
      sceneId: scene.id,
      alpha: 0,
      enabled: GROUP_ENABLED,
    });

    const layers = scene.layers || [];

    for (let i = 0, len = layers.length; i < len; i++) {
      const layer = layers[i];

      const module = await store.dispatch("modules/makeActiveModule", {
        moduleName: layer.$moduleName,
        existingModule: layer,
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

    commit("UPDATE_AUTOPILOT", { currentSceneId: scene.id });

    if (previousGroup) {
      const duration = Math.max(0.01, state.autopilot.transitionSeconds);

      transition = {
        fromGroupId: previousGroup.id,
        toGroupId: group.id,
        fromAlpha: previousGroup.alpha,
        elapsed: 0,
        duration,
      };

      pendingCleanup.push(previousGroup.id);
      commit("SET_TRANSITIONING", true);
    } else {
      store.commit("groups/UPDATE_GROUP", {
        groupId: group.id,
        data: { alpha: 1 },
      });
    }
  },

  async removeScene({ commit, state }, { sceneId }) {
    const group = store.state.groups.groups.find(
      (group) => group.sceneId === sceneId,
    );

    if (group) {
      if (transition) {
        if (transition.toGroupId === group.id) {
          store.commit("groups/UPDATE_GROUP", {
            groupId: transition.fromGroupId,
            data: { alpha: 1 },
          });
        }

        transition = null;
        commit("SET_TRANSITIONING", false);
      }

      pendingCleanup = pendingCleanup.filter((id) => id !== group.id);

      await store.dispatch("groups/removeGroup", { groupId: group.id });
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
        const scene = JSON.parse(fileBuffer.toString());

        if (scene && scene.id && Array.isArray(scene.layers)) {
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
      const eased = progress * progress * (3 - 2 * progress);

      store.commit("groups/UPDATE_GROUP", {
        groupId: transition.toGroupId,
        data: { alpha: eased },
      });

      store.commit("groups/UPDATE_GROUP", {
        groupId: transition.fromGroupId,
        data: { alpha: transition.fromAlpha * (1 - eased) },
      });

      if (progress >= 1) {
        store.commit("groups/UPDATE_GROUP", {
          groupId: transition.toGroupId,
          data: { alpha: 1 },
        });

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

    dwell += dt;

    const chance = switchChance(
      dwell,
      autopilot.minSeconds,
      autopilot.maxSeconds,
    );
    const musical = phraseUpdate.flipped && Math.random() < chance;
    const forced = dwell >= autopilot.maxSeconds;

    if (musical || forced) {
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
