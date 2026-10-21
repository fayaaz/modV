const state = {
  audio: [],
  video: [],

  currentAudioSource: null,
  currentVideoSource: null,

  // State of the system audio (loopback) capture
  systemAudio: {
    supported: false,
    active: false,
    label: null,
    error: null,
  },
  // Multi-select support: list of selected video deviceIds
  selectedVideoSources: [],

  // Requested webcam capture settings (null = let the device decide)
  videoResolution: {
    width: null,
    height: null,
  },

  videoFrameRate: null,

  // Settings the camera actually gave us
  appliedVideoSettings: {
    width: null,
    height: null,
    frameRate: null,
  },
};

const mutations = {
  ADD_AUDIO_SOURCE(state, { source }) {
    state.audio.push(source);
  },

  ADD_VIDEO_SOURCE(state, { source }) {
    state.video.push(source);
  },

  CLEAR_AUDIO_SOURCES(state) {
    state.audio = [];
  },

  CLEAR_VIDEO_SOURCES(state) {
    state.video = [];
  },

  SET_CURRENT_AUDIO_SOURCE(state, { audioId }) {
    state.currentAudioSource = audioId;
  },

  SET_SYSTEM_AUDIO_STATE(state, payload) {
    state.systemAudio = {
      ...state.systemAudio,
      ...payload,
    };
  },

  SET_CURRENT_VIDEO_SOURCE(state, { videoId }) {
    state.currentVideoSource = videoId;
    if (videoId && !state.selectedVideoSources.includes(videoId)) {
      state.selectedVideoSources.push(videoId);
    }
  },
  SET_SELECTED_VIDEO_SOURCES(state, { videoIds }) {
    state.selectedVideoSources = Array.isArray(videoIds) ? videoIds : [];
  },
  REMOVE_SELECTED_VIDEO_SOURCE(state, { videoId }) {
    state.selectedVideoSources = state.selectedVideoSources.filter(
      (id) => id !== videoId,
    );
    if (state.currentVideoSource === videoId) {
      state.currentVideoSource = null;
    }
  },

  SET_VIDEO_RESOLUTION(state, { width, height }) {
    state.videoResolution = {
      width: width || null,
      height: height || null,
    };
  },

  SET_VIDEO_FRAME_RATE(state, { frameRate }) {
    state.videoFrameRate = frameRate || null;
  },

  SET_APPLIED_VIDEO_SETTINGS(state, { width, height, frameRate }) {
    state.appliedVideoSettings = {
      width: width || null,
      height: height || null,
      frameRate: frameRate || null,
    };
  },
};

export default {
  namespaced: true,
  state,
  mutations,
};
