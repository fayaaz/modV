import { createSerialTask } from "../../serial-task";
import { getSceneRowPreview } from "../../scene-preview-data";
import {
  attachScenePreviewCanvas,
  clearScenePreview,
  createScenePreview,
  disposeScenePreview,
  freezeScenePreview,
  pruneScenePreviews,
} from "../../scene-preview";

function getDefaultState() {
  return {
    active: false,
    sceneId: null,
    rowIndex: null,
    groupName: "",
    moduleCount: 0,
    rendered: false,
    frozen: false,
  };
}

const state = getDefaultState();
const schedulePreview = createSerialTask();
let generation = 0;

const getters = {
  isActive: (state) => state.active,
};

const actions = {
  start({ commit, rootState }, payload = {}) {
    const request = ++generation;
    const { sceneId, rowIndex, canvas, width, height } = payload;
    const key = payload.key || `${sceneId}:${rowIndex}`;

    if (canvas) {
      attachScenePreviewCanvas(key, canvas, width, height);
    }

    return schedulePreview(async () => {
      if (request !== generation) {
        return null;
      }
      const scene = rootState.scenes.scenes.find((item) => item.id === sceneId);
      const row = Number(rowIndex);
      const preview =
        scene && Number.isInteger(row) && row >= 0
          ? getSceneRowPreview(scene, row)
          : null;

      if (!preview) {
        clearScenePreview(rootState.renderers, key);
        commit("CLEAR_PREVIEW");
        return null;
      }

      const result = await createScenePreview({
        key,
        sceneId,
        rowIndex: row,
        groupName: preview.group.name,
        modules: preview.modules,
        registered: rootState.modules.registered,
        renderers: rootState.renderers,
        canvas,
        width,
        height,
      });

      if (request !== generation) {
        clearScenePreview(rootState.renderers, key);
        return null;
      }

      if (!result) {
        commit("CLEAR_PREVIEW");
        return null;
      }

      commit("SET_PREVIEW", result);
      return result;
    });
  },

  stop({ commit, rootState }, payload = {}) {
    ++generation;

    return schedulePreview(async () => {
      clearScenePreview(rootState.renderers, payload.key);
      commit("CLEAR_PREVIEW");
    });
  },

  freeze({ commit, rootState }, payload = {}) {
    ++generation;

    return schedulePreview(async () => {
      const frozen = freezeScenePreview(rootState.renderers, payload.key);

      if (frozen) {
        commit("FREEZE_PREVIEW");
      }

      return frozen;
    });
  },

  prune({ commit, state }, payload = {}) {
    const keys = Array.isArray(payload.keys) ? payload.keys : [];
    const keep = new Set(keys);
    const currentKey =
      state.sceneId === null ? null : `${state.sceneId}:${state.rowIndex}`;

    pruneScenePreviews(keys);

    if (currentKey !== null && !keep.has(currentKey)) {
      commit("CLEAR_PREVIEW");
    }
  },

  dispose({ commit, rootState }) {
    ++generation;

    return schedulePreview(async () => {
      disposeScenePreview(rootState.renderers);
      commit("CLEAR_PREVIEW");
    });
  },
};

const mutations = {
  SET_PREVIEW(state, { sceneId, rowIndex, groupName, moduleCount }) {
    state.active = true;
    state.sceneId = sceneId;
    state.rowIndex = rowIndex;
    state.groupName = groupName;
    state.moduleCount = moduleCount;
    state.rendered = false;
    state.frozen = false;
  },

  SET_RENDERED(state) {
    if (state.active) {
      state.rendered = true;
      state.frozen = false;
    }
  },

  FREEZE_PREVIEW(state) {
    state.active = false;
    state.frozen = true;
  },

  CLEAR_PREVIEW(state) {
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
