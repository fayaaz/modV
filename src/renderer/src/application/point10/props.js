// Responsiveness levers every imported Point10 scene gets. They are applied to
// the band signal by Point10AudioBridge (see audioBridge.js), not inside the
// vendored scenes, so the scene files stay byte-for-byte portable.
const RESPONSIVENESS_PROPS = {
  gain: {
    label: "Gain",
    type: "float",
    min: 0,
    max: 4,
    step: 0.01,
    default: 1,
  },

  smoothing: {
    label: "Smoothing",
    type: "float",
    min: 0,
    max: 1,
    step: 0.01,
    default: 0,
  },

  bassGain: {
    label: "Bass Gain",
    type: "float",
    min: 0,
    max: 4,
    step: 0.01,
    default: 1,
  },

  midGain: {
    label: "Mid Gain",
    type: "float",
    min: 0,
    max: 4,
    step: 0.01,
    default: 1,
  },

  trebleGain: {
    label: "Treble Gain",
    type: "float",
    min: 0,
    max: 4,
    step: 0.01,
    default: 1,
  },

  useAgc: {
    label: "Auto Gain (AGC)",
    type: "bool",
    default: true,
  },
};

function readLevers(props = {}) {
  const num = (value, fallback, min, max) => {
    const n = typeof value === "number" && isFinite(value) ? value : fallback;

    return n < min ? min : n > max ? max : n;
  };

  return {
    gain: num(props.gain, 1, 0, 4),
    smoothing: num(props.smoothing, 0, 0, 1),
    bassGain: num(props.bassGain, 1, 0, 4),
    midGain: num(props.midGain, 1, 0, 4),
    trebleGain: num(props.trebleGain, 1, 0, 4),
    useAgc: props.useAgc !== false,
  };
}

export { RESPONSIVENESS_PROPS, readLevers };
