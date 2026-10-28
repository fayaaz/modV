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
  // Every checked webcam gets its own capture and output
  selectedVideoSources: [],

  // Requested capture settings per deviceId. The global videoResolution /
  // videoFrameRate values are the legacy single-camera setting and act as a
  // fallback for cameras that have no per-device entry yet.
  videoSettings: {},

  videoResolution: {
    width: null,
    height: null,
  },

  videoFrameRate: null,

  // Modes each camera reports through getCapabilities()
  videoCapabilities: {},

  // Settings each camera actually gave us
  appliedVideoSettings: {},
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
    state.currentVideoSource = videoId || null;

    if (videoId && !state.selectedVideoSources.includes(videoId)) {
      state.selectedVideoSources.push(videoId);
    }
  },
  SET_SELECTED_VIDEO_SOURCES(state, { videoIds }) {
    state.selectedVideoSources = Array.isArray(videoIds)
      ? [...new Set(videoIds.filter(Boolean))]
      : [];

    if (!state.selectedVideoSources.includes(state.currentVideoSource)) {
      state.currentVideoSource = state.selectedVideoSources[0] || null;
    }
  },
  REMOVE_SELECTED_VIDEO_SOURCE(state, { videoId }) {
    state.selectedVideoSources = state.selectedVideoSources.filter(
      (id) => id !== videoId,
    );
    if (state.currentVideoSource === videoId) {
      state.currentVideoSource = state.selectedVideoSources[0] || null;
    }
  },

  SET_VIDEO_SETTINGS(state, { deviceId, resolution, frameRate }) {
    if (!deviceId) {
      return;
    }

    const current = state.videoSettings[deviceId] || {};
    const next = { ...current };

    if (resolution !== undefined) {
      next.resolution = {
        width: resolution?.width || null,
        height: resolution?.height || null,
      };
    }

    if (frameRate !== undefined) {
      next.frameRate = frameRate || null;
    }

    state.videoSettings = {
      ...state.videoSettings,
      [deviceId]: next,
    };
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

  SET_VIDEO_CAPABILITIES(state, { deviceId, resolutions, frameRates }) {
    if (!deviceId) {
      return;
    }

    state.videoCapabilities = {
      ...state.videoCapabilities,
      [deviceId]: {
        resolutions: Array.isArray(resolutions) ? resolutions : [],
        frameRates: Array.isArray(frameRates) ? frameRates : [],
      },
    };
  },

  SET_APPLIED_VIDEO_SETTINGS(state, { deviceId, width, height, frameRate }) {
    if (!deviceId) {
      return;
    }

    state.appliedVideoSettings = {
      ...state.appliedVideoSettings,
      [deviceId]: {
        width: width || null,
        height: height || null,
        frameRate: frameRate || null,
      },
    };
  },
};

export default {
  namespaced: true,
  state,
  mutations,
};
