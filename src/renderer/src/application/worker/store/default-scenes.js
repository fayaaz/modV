// Ready-to-go music reactive Scenes. Each Scene is composed of three Groups so
// the visual is layered the same way musicvis builds a look: a Background
// visual, a Foreground overlay and an Effects group that inherits the previous
// Groups and runs a post effect over them.
//
// `props` here are applied on top of each Module's registered defaults when the
// Scene snapshot is generated on first run. `meta` is merged into the Module's
// meta (alpha, blend mode, ...).
//
// `introducedVersion` lets existing installs receive only the Scenes added
// since their last seeding pass. Version 1 is the original six Scenes.
export const LEGACY_DEFAULTS_VERSION = 1;
export const DEFAULTS_VERSION = 2;

const sceneKey = (name) =>
  String(name || "")
    .trim()
    .toLowerCase();

const getIntroducedVersion = (scene) =>
  Number.isFinite(scene.introducedVersion)
    ? scene.introducedVersion
    : LEGACY_DEFAULTS_VERSION;

export function getDefaultSceneNamesUpTo(version) {
  const maxVersion = Number.isFinite(version) ? version : 0;

  return DEFAULT_SCENES.filter(
    (scene) => getIntroducedVersion(scene) <= maxVersion,
  ).map((scene) => scene.name);
}

export function getScenesToSeed({
  defaultsVersion = 0,
  seededNames = [],
  existingNames = [],
} = {}) {
  const version = Number.isFinite(defaultsVersion) ? defaultsVersion : 0;
  const seeded = new Set((seededNames || []).map(sceneKey));
  const existing = new Set((existingNames || []).map(sceneKey));

  return DEFAULT_SCENES.filter((scene) => {
    const key = sceneKey(scene.name);

    return (
      getIntroducedVersion(scene) > version &&
      !seeded.has(key) &&
      !existing.has(key)
    );
  });
}

const DEFAULT_SCENES = [
  {
    name: "Mycelium Bloom",
    introducedVersion: 1,
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
    introducedVersion: 1,
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
    introducedVersion: 1,
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
    introducedVersion: 1,
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
    introducedVersion: 1,
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
    introducedVersion: 1,
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

  {
    name: "Burning Mycelium Circle",
    introducedVersion: 2,
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
              smoothing: 0.12,
              bassGain: 1.5,
              midGain: 1.1,
              trebleGain: 0.8,
              useAgc: true,
            },
          },
          {
            name: "Circular Feedback Mask",
            props: {
              maskRadius: 0.15180484047380588,
              feedbackRate: 0.029966065660119057,
              twirlAmount: 0,
              fadeRate: 0,
              centerFeedback: 0,
              feedbackCenter: [0.5, 0.5],
              styleMode: 2,
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
              smoothing: 0.08,
              bassGain: 1.2,
              midGain: 1,
              trebleGain: 1,
              useAgc: true,
            },
            meta: { enabled: false, compositeOperation: "screen" },
          },
          {
            name: "Point10 Smoke Room",
            props: {
              gain: 1,
              smoothing: 0,
              bassGain: 1,
              midGain: 1,
              trebleGain: 1,
              useAgc: true,
            },
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
    name: "Chromatic Cascade",
    introducedVersion: 2,
    minSeconds: 8,
    maxSeconds: 21,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Chromatic Wave",
            props: {
              gain: 1.25,
              bassGain: 1.45,
              midGain: 1.15,
              trebleGain: 0.9,
              smoothing: 0.07,
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
              gain: 1.15,
              midGain: 1.35,
              smoothing: 0.04,
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
            name: "Bloom",
            props: {
              blurAmount: 5,
              intensity: 0.45,
            },
          },
          {
            name: "Chromatic Abberation",
            props: {
              rOffset: 1.002,
              gOffset: 1.012,
              bOffset: 1.024,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Kaleidoscope Cathedral",
    introducedVersion: 2,
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
            name: "Point10 Geometric Tunnel",
            props: {
              gain: 1.2,
              bassGain: 1.4,
              trebleGain: 1.1,
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
            name: "Point10 Spiral Particles",
            props: {
              gain: 1.15,
              bassGain: 1.1,
              trebleGain: 1.4,
              smoothing: 0.05,
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
            name: "Kaleidoscope",
            props: {
              sides: 6,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 4,
              intensity: 0.4,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Neon Grid",
    introducedVersion: 2,
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
            name: "Point10 Cube Floor",
            props: {
              gain: 1.25,
              bassGain: 1.5,
              midGain: 1.1,
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
            name: "Bar",
            props: {
              rotation: 45,
              thickness: 18,
              strobe: 0,
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
            name: "Neon",
            props: {
              intensity: 6,
              gain: 0.8,
            },
          },
          {
            name: "Vignette",
            props: {
              intensity: 22,
              extension: 0.35,
            },
          },
        ],
      },
    ],
  },

  {
    name: "VHS Afterglow",
    introducedVersion: 2,
    minSeconds: 9,
    maxSeconds: 23,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Water Ripple",
            props: {
              gain: 1.15,
              bassGain: 1.3,
              midGain: 1.1,
              smoothing: 0.18,
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
            name: "Bar",
            props: {
              rotation: 0,
              thickness: 26,
              strobe: 0,
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
            name: "VHS Glitch",
            props: {
              autoScan: true,
              xScanline: 0.45,
              xScanline2: 0.55,
              yScanline: 0.1,
              xScanlineSize: 0.5,
              xScanlineSize2: 0.2,
              yScanlineAmount: 0.2,
              grainLevel: 0.15,
              scanFollow: true,
              analogDistort: 1.4,
              bleedAmount: 0.9,
            },
          },
          {
            name: "VVMotionBlur 3.0",
            props: {
              blurAmount: 0.22,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Slit Scan Horizon",
    introducedVersion: 2,
    minSeconds: 10,
    maxSeconds: 25,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Chromatic Wave",
            props: {
              gain: 1.2,
              bassGain: 1.4,
              midGain: 1.1,
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
            name: "Waveform",
            props: {
              strokeWeight: 2,
              maxHeight: 70,
              maxWidth: 0.85,
              windowing: "hanning",
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
            name: "Slit Scan",
            props: {
              spacing: 0.8,
              line_width: 0.28,
              angle: 0.2,
              shift: 0.45,
            },
          },
          {
            name: "Hue-Saturation",
            props: {
              hue: 0.1,
              saturation: 0.2,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Mirror Chamber",
    introducedVersion: 2,
    minSeconds: 12,
    maxSeconds: 27,
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
              gain: 1.2,
              bassGain: 1.4,
              midGain: 1.15,
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
              gain: 1.1,
              midGain: 1.3,
              trebleGain: 1.1,
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
            name: "Mirror",
            props: {
              horizontal: true,
              vertical: true,
            },
          },
          {
            name: "Dirty Lens",
            props: {
              scale: 14,
              brightness: 0.35,
              brightnessCurve: 1.2,
              radius: 0.55,
              noiseSeed: 0.2,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Solar Flare",
    introducedVersion: 2,
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
            name: "Point10 Ball Of Fire",
            props: {
              gain: 1.15,
              bassGain: 1.6,
              midGain: 1.1,
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
            name: "Concentrics",
            props: {
              rms: true,
              intensity: 8,
              spacing: 3,
              objectDistance: 55,
              strokeWeight: 2,
            },
            meta: { alpha: 0.55, compositeOperation: "screen" },
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
            name: "Solarize",
            props: {
              centerBrightness: 0.6,
              powerCurve: 1.2,
              colorize: 0.2,
              inverse: true,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 8,
              intensity: 0.65,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Water Ripple Dreams",
    introducedVersion: 2,
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
            name: "Point10 Water Ripple",
            props: {
              gain: 1.25,
              bassGain: 1.5,
              midGain: 1.2,
              smoothing: 0.14,
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
            name: "Polygon",
            props: {
              intensity: 12,
              shapeSize: 90,
              strokeWeight: 2,
              fill: false,
              rotateToggle: true,
              rotateSpeed: 2.5,
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
            name: "Ripples",
            props: {
              level: 4,
              offset: 0.1,
              x_smear: 0.1,
              y_smear: 0.2,
              center: [0.5, 0.5],
              mode: 0,
            },
          },
          {
            name: "Hue-Saturation",
            props: {
              hue: -0.08,
              saturation: 0.25,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Cube Floor Drive",
    introducedVersion: 2,
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
            name: "Point10 Cube Floor",
            props: {
              gain: 1.3,
              bassGain: 1.6,
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
              midGain: 1.25,
              smoothing: 0.04,
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
            name: "Chroma Zoom",
            props: {
              master_zoom: 1.05,
              red_zoom: 1.01,
              green_zoom: 1.02,
              blue_zoom: 1.03,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 6,
              intensity: 0.45,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Spiral Particles",
    introducedVersion: 2,
    minSeconds: 10,
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
            name: "Point10 Spiral Particles",
            props: {
              gain: 1.2,
              bassGain: 1.2,
              trebleGain: 1.5,
              smoothing: 0.04,
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
              smoothing: 0.06,
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
            name: "Twirl",
            props: {
              radius: 0.8,
              amount: 1.5,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Vignette",
            props: {
              intensity: 18,
              extension: 0.3,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Rolling Fire Bloom",
    introducedVersion: 2,
    minSeconds: 10,
    maxSeconds: 25,
    groups: [
      {
        name: "Background",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Rolling Fire",
            props: {
              gain: 1.2,
              bassGain: 1.5,
              midGain: 1.05,
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
            name: "Bands",
            props: {
              u_ringWidth: 0.03,
            },
            meta: { alpha: 0.5, compositeOperation: "screen" },
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
              blurAmount: 10,
              intensity: 0.7,
            },
          },
          {
            name: "Posterize",
            props: {
              gamma: 1.1,
              numColors: 10,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Spectral Bloom",
    introducedVersion: 2,
    minSeconds: 8,
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
            name: "Point10 Spectrum Bars",
            props: {
              gain: 1.1,
              midGain: 1.4,
              trebleGain: 1.2,
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
            name: "Point10 Disco Ball",
            props: {
              gain: 1.2,
              bassGain: 1.4,
              smoothing: 0.06,
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
            name: "Glow",
            props: {
              intensity: 0.6,
            },
          },
          {
            name: "Color Controls",
            props: {
              bright: 0.02,
              contrast: 1.05,
              hue: 0.05,
              saturation: 1.3,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Twirl Tunnel",
    introducedVersion: 2,
    minSeconds: 9,
    maxSeconds: 23,
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
              gain: 1.1,
              bassGain: 1.2,
              trebleGain: 1.3,
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
            name: "Point10 Chromatic Line",
            props: {
              gain: 1.2,
              midGain: 1.35,
              smoothing: 0.04,
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
            name: "Twirl",
            props: {
              radius: 1,
              amount: 2,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Hue-Saturation",
            props: {
              hue: 0.08,
              saturation: 0.15,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Strobe Riot",
    introducedVersion: 2,
    minSeconds: 7,
    maxSeconds: 18,
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
              bassGain: 1.45,
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
              gain: 1.2,
              trebleGain: 1.4,
              smoothing: 0.05,
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
            name: "Strobe",
            props: {
              strobeRate: 0.12,
              strobeMode: 0,
              strobeColor: [1, 1, 1, 1],
            },
          },
          {
            name: "Dot Screen",
            props: {
              sharpness: 2,
              angle: 0.25,
              scale: 1.2,
              colorize: 0.2,
              center: [0.5, 0.5],
            },
          },
        ],
      },
    ],
  },
];

export default DEFAULT_SCENES;
