// Ready-to-go music reactive Scenes. Each Scene is composed of three Groups so
// the visual is layered the same way musicvis builds a look: a Background
// visual, a Foreground overlay and an Effects group that inherits the previous
// Groups and runs a post effect over them.
//
// `props` here are applied on top of each Module's registered defaults when the
// Scene snapshot is generated on first run. `meta` is merged into the Module's
// meta (alpha, blend mode, ...).
const DEFAULT_SCENES = [
  {
    name: "Mycelium Bloom",
    minSeconds: 10,
    maxSeconds: 26,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Mycelium",
            props: {
              gain: 1.1,
              bassGain: 1.5,
              midGain: 1.1,
              trebleGain: 0.8,
              smoothing: 0.12,
            },
          },
        ],
      },
      {
        name: "Foreground",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "screen",
        modules: [
          {
            name: "Point10 Spectrum Bars",
            props: {
              gain: 1.2,
              bassGain: 1.2,
              smoothing: 0.08,
            },
            meta: { alpha: 0.85, compositeOperation: "screen" },
          },
        ],
      },
      {
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Bloom",
            props: {
              blurAmount: 6,
              intensity: 0.55,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Disco Ball",
    minSeconds: 8,
    maxSeconds: 20,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Disco Ball",
            props: {
              gain: 1.25,
              bassGain: 1.4,
              smoothing: 0.06,
            },
          },
        ],
      },
      {
        name: "Foreground",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "screen",
        modules: [
          {
            name: "Point10 Spectrum Bars",
            props: {
              gain: 1.15,
              midGain: 1.3,
            },
            meta: { alpha: 0.7, compositeOperation: "screen" },
          },
        ],
      },
      {
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Chromatic Abberation",
            props: {
              rOffset: 1,
              gOffset: 1.012,
              bOffset: 1.024,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Ferrofluid Pulse",
    minSeconds: 12,
    maxSeconds: 28,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Ferrofluid",
            props: {
              gain: 1.3,
              bassGain: 1.6,
              midGain: 1.2,
              smoothing: 0.1,
            },
          },
        ],
      },
      {
        name: "Foreground",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "screen",
        modules: [
          {
            name: "Bands",
            props: {
              u_ringWidth: 0.045,
            },
            meta: { alpha: 0.6, compositeOperation: "screen" },
          },
        ],
      },
      {
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Bloom",
            props: {
              blurAmount: 4,
              intensity: 0.65,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Ring of Fire",
    minSeconds: 9,
    maxSeconds: 22,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Ring Of Fire",
            props: {
              gain: 1.2,
              bassGain: 1.5,
              smoothing: 0.08,
            },
          },
        ],
      },
      {
        name: "Foreground",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "screen",
        modules: [
          {
            name: "Point10 Particle Field",
            props: {
              gain: 1.1,
              trebleGain: 1.4,
            },
            meta: { alpha: 0.65, compositeOperation: "screen" },
          },
        ],
      },
      {
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "VVMotionBlur 3.0",
            props: {
              blurAmount: 0.35,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Smoke Room",
    minSeconds: 14,
    maxSeconds: 30,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Smoke Room",
            props: {
              gain: 1.15,
              bassGain: 1.35,
              midGain: 1.1,
              smoothing: 0.15,
            },
          },
        ],
      },
      {
        name: "Foreground",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "screen",
        modules: [
          {
            name: "Point10 Chromatic Line",
            props: {
              gain: 1.25,
              midGain: 1.4,
            },
            meta: { alpha: 0.8, compositeOperation: "screen" },
          },
        ],
      },
      {
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Dirty Lens",
            props: {
              scale: 12,
              brightness: 0.35,
              brightnessCurve: 1.2,
              radius: 0.6,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Geometric Tunnel",
    minSeconds: 8,
    maxSeconds: 24,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Geometric Tunnel",
            props: {
              gain: 1.2,
              bassGain: 1.4,
              trebleGain: 1.1,
              smoothing: 0.05,
            },
          },
        ],
      },
      {
        name: "Foreground",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "screen",
        modules: [
          {
            name: "Point10 Waveform",
            props: {
              gain: 1.2,
              midGain: 1.2,
            },
            meta: { alpha: 0.75, compositeOperation: "screen" },
          },
        ],
      },
      {
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Bloom",
            props: {
              blurAmount: 8,
              intensity: 0.5,
            },
          },
        ],
      },
    ],
  },
];

export default DEFAULT_SCENES;
