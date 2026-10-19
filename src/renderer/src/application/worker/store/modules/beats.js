const state = {
  bpm: 0,
  bpmSource: "beatdetektor",
  kick: false,
  bpmSources: ["beatdetektor", "midi", "tap"],
};

const actions = {
  setBpm({ commit, state }, { bpm, source }) {
    if (!source) {
      throw new Error("Setting the BPM requires a source to be given");
    }

    if (source === state.bpmSource) {
      commit("SET_BPM", { bpm });
    }
  },
};

const mutations = {
  SET_BPM(state, { bpm }) {
    state.bpm = bpm;
  },

  SET_KICK(state, { kick }) {
    state.kick = kick;
  },

  ADD_BPM_SOURCE(state, { source }) {
    if (!state.bpmSources.includes(source)) {
      state.bpmSources.push(source);
    }
  },

  SET_BPM_SOURCE(state, { source }) {
    if (!state.bpmSources.includes(source)) {
      console.warn(
        `beats: "${source}" is not a valid bpm source, ignoring SET_BPM_SOURCE`,
      );
      return;
    }

    state.bpmSource = source;
  },
};

export default {
  namespaced: true,
  state,
  mutations,
  actions,
};
