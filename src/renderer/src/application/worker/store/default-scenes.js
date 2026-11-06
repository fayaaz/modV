// Ready-to-go music reactive Scenes. Each Scene is composed of three Groups so
// the visual is layered the same way musicvis builds a look: a Background
// visual, a Foreground overlay and an Effects group that inherits the previous
// Groups and runs a post effect over them.
//
// `props` here are applied on top of each Module's registered defaults when the
// Scene snapshot is generated on first run. `meta` is merged into the Module's
// meta (alpha, blend mode, ...).
//
// A Scene may also declare `inputLinks` to pin a control to audio: each entry
// names a Group and optionally one of its Modules plus a `control` (alpha,
// enabled, compositeOperation or a numeric prop), a `feature` (bass, mid, ...)
// and an optional min/max range. These are created before the automatic
// bass/BPM links, which then skip the pinned controls.
//
// `introducedVersion` lets existing installs receive only the Scenes added
// since their last seeding pass. Version 1 is the original six Scenes,
// version 2 the second batch, version 3 the glitch/Kaleidoscope batch,
// version 4 the varied Point10 scene batch and version 5 the ASCII/filter
// variety batch.
export const LEGACY_DEFAULTS_VERSION = 1;
export const DEFAULTS_VERSION = 5;

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
            name: "Point10 Waveform",
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
    name: "Chromatic Sphere",
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
            name: "Point10 Chromatic Wave",
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
            name: "Point10 Waveform",
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
            name: "Point10 Mycelium",
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
            name: "Vignette",
            props: {
              intensity: 20,
              extension: 0.35,
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
            name: "Point10 Waveform",
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
            name: "Point10 Waveform",
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
            name: "Bloom",
            props: {
              blurAmount: 10,
              intensity: 0.45,
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
            name: "Point10 Waveform",
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
            name: "Point10 Waveform",
            props: {
              gain: 1.2,
              midGain: 1.3,
              smoothing: 0.04,
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
            name: "Point10 Chromatic Wave",
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
            name: "Point10 Waveform",
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

  {
    name: "Fractured Kaleidoscope",
    introducedVersion: 3,
    minSeconds: 6,
    maxSeconds: 16,
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
              gain: 1.15,
              bassGain: 1.5,
              midGain: 1.1,
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
            name: "Point10 Digital Glitch",
            props: {},
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
              sides: 8,
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
    name: "Prism Reliquary",
    introducedVersion: 3,
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
            name: "Point10 Spiral Particles",
            props: {
              gain: 1.15,
              bassGain: 1.2,
              trebleGain: 1.4,
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
            name: "Point10 RGB Shift",
            props: {},
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
            name: "Kaleidoscope Tile",
            props: {
              size: 0.6,
              sides: 12,
              rotation: 0.125,
              angle: 0.05,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 5,
              intensity: 0.45,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Broken Mirror Ball",
    introducedVersion: 3,
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
            name: "Point10 Mycelium",
            props: {
              gain: 1.25,
              bassGain: 1.45,
              midGain: 1.15,
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
            name: "Point10 Digital Glitch",
            props: {},
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
            name: "Kaleidoscope",
            props: {
              sides: 5,
            },
          },
          {
            name: "Point10 Contrast",
            props: {},
          },
        ],
      },
    ],
  },

  {
    name: "Twirl Reactor",
    introducedVersion: 3,
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
            name: "Point10 Color Distort",
            props: {},
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
            name: "Twirl",
            props: {
              radius: 0.9,
              amount: 2.4,
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
    name: "Spiral Static",
    introducedVersion: 3,
    minSeconds: 7,
    maxSeconds: 19,
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
            name: "Spiral",
            props: {
              rotation: 0.08,
              count: 9,
              width: 0.09,
              softness: 0.35,
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
            name: "Point10 Digital Glitch",
            props: {},
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 4,
              intensity: 0.35,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Ultimate Drift",
    introducedVersion: 3,
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
            name: "Point10 Water Ripple",
            props: {
              gain: 1.2,
              bassGain: 1.35,
              midGain: 1.1,
              smoothing: 0.16,
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
            name: "UltimateSpiral",
            props: {
              rate: 10,
              density: 48,
              sectors: 32,
              shape: 24,
              smooth: 0.08,
              vanishingpoint: 0.5,
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
            name: "Point10 RGB Shift",
            props: {},
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 6,
              intensity: 0.4,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Roto Zoom Riot",
    introducedVersion: 3,
    minSeconds: 6,
    maxSeconds: 15,
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
              bassGain: 1.55,
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
            name: "Point10 Digital Glitch",
            props: {},
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
            name: "rotozoomer",
            props: {
              rotationSpeed: 55,
              zoomScale: 4,
              timeScale: 1.5,
            },
          },
          {
            name: "Point10 Contrast",
            props: {},
          },
        ],
      },
    ],
  },

  {
    name: "Glitch Bloom Cathedral",
    introducedVersion: 3,
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
            name: "Point10 RGB Shift",
            props: {},
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
            name: "Kaleidoscope Tile",
            props: {
              size: 0.45,
              sides: 8,
              rotation: 0.2,
              angle: 0.1,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 8,
              intensity: 0.55,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Twirl Grid",
    introducedVersion: 3,
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
            name: "Point10 Mycelium",
            props: {
              gain: 1.25,
              bassGain: 1.55,
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
            name: "Polygon",
            props: {
              intensity: 14,
              shapeSize: 70,
              strokeWeight: 2,
              fill: false,
              rotateToggle: true,
              rotateSpeed: 3.5,
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
            name: "Twirl",
            props: {
              radius: 0.6,
              amount: 1.8,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Point10 Color Distort",
            props: {},
          },
        ],
      },
    ],
  },

  {
    name: "Hex Bloom",
    introducedVersion: 3,
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
            name: "Concentrics",
            props: {
              rms: true,
              intensity: 10,
              spacing: 4,
              objectDistance: 60,
              strokeWeight: 2,
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
            name: "Kaleidoscope",
            props: {
              sides: 10,
            },
          },
          {
            name: "Point10 Color Distort",
            props: {},
          },
        ],
      },
    ],
  },

  {
    name: "VHS Reliquary",
    introducedVersion: 3,
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
            name: "Point10 Particle Field",
            props: {
              gain: 1.15,
              bassGain: 1.2,
              trebleGain: 1.4,
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
            name: "Point10 Digital Glitch",
            props: {},
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
            name: "Kaleidoscope Tile",
            props: {
              size: 0.7,
              sides: 5,
              rotation: 0.05,
              angle: 0.15,
            },
          },
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
        ],
      },
    ],
  },

  {
    name: "Twirled Feedback",
    introducedVersion: 3,
    minSeconds: 11,
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
            name: "Point10 Chromatic Line",
            props: {
              gain: 1.2,
              midGain: 1.35,
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
            name: "Point10 Color Distort",
            props: {},
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
            name: "Circular Feedback Mask",
            props: {
              maskRadius: 0.2,
              feedbackRate: 0.04,
              twirlAmount: 0.12,
              fadeRate: 0.02,
              centerFeedback: 0,
              feedbackCenter: [0.5, 0.5],
              styleMode: 2,
            },
          },
          {
            name: "Twirl",
            props: {
              radius: 0.5,
              amount: 1.2,
              center: [0.5, 0.5],
            },
          },
        ],
      },
    ],
  },

  {
    name: "It Is Coming",
    introducedVersion: 4,
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
            name: "Point10 It Is Coming For You",
            props: {
              speed: 1,
              glow: 1.2,
              bloom: 1.1,
              gain: 1.15,
              bassGain: 1.5,
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
            name: "Point10 RGB Shift",
            props: {},
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
              blurAmount: 7,
              intensity: 0.55,
            },
          },
          {
            name: "Vignette",
            props: {
              intensity: 20,
              extension: 0.32,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 RGB Shift",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.75,
      },
    ],
  },

  {
    name: "It Comes At Night",
    introducedVersion: 4,
    minSeconds: 9,
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
            name: "Point10 It Is Coming For You",
            props: {
              speed: 0.8,
              glow: 0.9,
              bloom: 0.9,
              gain: 1.2,
              bassGain: 1.35,
              midGain: 1.15,
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
            name: "Point10 Digital Glitch",
            props: {},
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
            name: "Kaleidoscope",
            props: {
              sides: 7,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 6,
              intensity: 0.5,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Background",
        module: "Point10 It Is Coming For You",
        control: "alpha",
        feature: "bass",
        min: 0.25,
        max: 1,
      },
    ],
  },

  {
    name: "Around Plus Aurora",
    introducedVersion: 4,
    minSeconds: 11,
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
            name: "Point10 Around Plus",
            props: {
              spinSpeed: 1.2,
              ringBrightness: 1.3,
              waterChurn: 1.1,
              gain: 1.2,
              bassGain: 1.4,
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
              intensity: 10,
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
            name: "Ripples",
            props: {
              level: 3,
              offset: 0.08,
              x_smear: 0.1,
              y_smear: 0.2,
              center: [0.5, 0.5],
              mode: 0,
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
    inputLinks: [
      {
        group: "Foreground",
        control: "alpha",
        feature: "bass",
        min: 0.15,
        max: 0.7,
      },
    ],
  },

  {
    name: "Around Water Garden",
    introducedVersion: 4,
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
            name: "Point10 Around Plus",
            props: {
              spinSpeed: 0.9,
              ringBrightness: 1.1,
              waterChurn: 1.4,
              gain: 1.15,
              midGain: 1.3,
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
              u_ringWidth: 0.04,
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
            name: "Twirl",
            props: {
              radius: 0.8,
              amount: 1.6,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Hue-Saturation",
            props: {
              hue: 0.08,
              saturation: 0.2,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Bands",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.65,
      },
    ],
  },

  {
    name: "Attractor Bloom",
    introducedVersion: 4,
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
            name: "Point10 Attractor Particles",
            props: {
              cameraDistance: 11,
              particleSize: 1.1,
              brightness: 1.2,
              maxSpeed: 1.1,
              gain: 1.25,
              bassGain: 1.45,
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
              height: 1.4,
              speed: 1.1,
              opacity: 0.8,
              gain: 1.2,
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
            name: "Bloom",
            props: {
              blurAmount: 8,
              intensity: 0.55,
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
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Waveform",
        control: "alpha",
        feature: "treble",
        min: 0.2,
        max: 0.8,
      },
    ],
  },

  {
    name: "Attractor Void",
    introducedVersion: 4,
    minSeconds: 11,
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
            name: "Point10 Attractor Particles",
            props: {
              cameraDistance: 16,
              particleSize: 1.6,
              brightness: 1.1,
              maxSpeed: 0.8,
              gain: 1.2,
              bassGain: 1.2,
              trebleGain: 1.4,
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
            name: "UltimateSpiral",
            props: {
              rate: 12,
              density: 40,
              sectors: 28,
              shape: 22,
              smooth: 0.1,
              vanishingpoint: 0.5,
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
            name: "Twirl",
            props: {
              radius: 0.7,
              amount: 2.2,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Chromatic Abberation",
            props: {
              rOffset: 1.002,
              gOffset: 1.01,
              bOffset: 1.02,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "UltimateSpiral",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.6,
      },
    ],
  },

  {
    name: "Boxy Boulevard",
    introducedVersion: 4,
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
            name: "Point10 Boxy",
            props: {
              scrollSpeed: 1.2,
              glow: 1.1,
              roughness: 0.35,
              bloom: 1.1,
              gain: 1.25,
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
            name: "Point10 Waveform",
            props: {
              height: 2,
              gain: 1.2,
              bassGain: 1.2,
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
            name: "Point10 Crt",
            props: {},
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 5,
              intensity: 0.4,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Waveform",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.7,
      },
    ],
  },

  {
    name: "Boxy Arcade",
    introducedVersion: 4,
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
            name: "Point10 Boxy",
            props: {
              scrollSpeed: 1.6,
              glow: 1.3,
              roughness: 0.2,
              bloom: 1.2,
              gain: 1.3,
              bassGain: 1.55,
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
            name: "Point10 Digital Glitch",
            props: {},
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
            name: "Point10 Old Movie",
            props: {},
          },
          {
            name: "Posterize",
            props: {
              gamma: 1.1,
              numColors: 8,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Digital Glitch",
        control: "alpha",
        feature: "bass",
        min: 0.05,
        max: 0.65,
      },
    ],
  },

  {
    name: "DVD Afterglow",
    introducedVersion: 4,
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
            name: "Point10 DVD Room",
            props: {
              smokeDensity: 20,
              fireIntensity: 20,
              smokeLifespan: 3,
              fireLifespan: 1.2,
              simulationSpeed: 1.1,
              gain: 1.15,
              bassGain: 1.3,
              midGain: 1.1,
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
              distortion: 0.08,
              amplitude: 1.2,
              glow: 1.2,
              gain: 1.2,
              midGain: 1.35,
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
            name: "Bloom",
            props: {
              blurAmount: 8,
              intensity: 0.6,
            },
          },
          {
            name: "Hue-Saturation",
            props: {
              hue: 0.06,
              saturation: 0.2,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Chromatic Line",
        control: "alpha",
        feature: "bass",
        min: 0.15,
        max: 0.8,
      },
    ],
  },

  {
    name: "DVD Nightmare",
    introducedVersion: 4,
    minSeconds: 9,
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
            name: "Point10 DVD Room",
            props: {
              smokeDensity: 26,
              fireIntensity: 32,
              smokeLifespan: 4,
              fireLifespan: 1.5,
              simulationSpeed: 1.3,
              gain: 1.2,
              bassGain: 1.45,
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
            name: "Point10 RGB Shift",
            props: {},
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
              grainLevel: 0.2,
              scanFollow: true,
              analogDistort: 1.6,
              bleedAmount: 1,
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
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 RGB Shift",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.7,
      },
    ],
  },

  {
    name: "Light Tunnel Runner",
    introducedVersion: 4,
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
            name: "Point10 Light Tunnel",
            props: {
              scrollSpeed: 1.3,
              glow: 1.2,
              bloom: 1.1,
              gain: 1.25,
              bassGain: 1.45,
              trebleGain: 1.1,
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
              particleSize: 0.18,
              speed: 1.2,
              opacity: 0.8,
              gain: 1.1,
              trebleGain: 1.4,
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
            name: "Chroma Zoom",
            props: {
              master_zoom: 1.06,
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
              intensity: 0.5,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Particle Field",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.7,
      },
    ],
  },

  {
    name: "Light Tunnel Choir",
    introducedVersion: 4,
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
            name: "Point10 Light Tunnel",
            props: {
              scrollSpeed: 0.9,
              glow: 1.4,
              bloom: 1.2,
              gain: 1.15,
              midGain: 1.3,
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
              height: 1.6,
              speed: 0.8,
              opacity: 0.8,
              gain: 1.2,
              midGain: 1.2,
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
            name: "Kaleidoscope Tile",
            props: {
              size: 0.55,
              sides: 10,
              rotation: 0.15,
              angle: 0.08,
            },
          },
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
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Waveform",
        control: "alpha",
        feature: "bass",
        min: 0.2,
        max: 0.85,
      },
    ],
  },

  {
    name: "Square Tunnel Signal",
    introducedVersion: 4,
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
            name: "Point10 Square Tunnel",
            props: {
              scrollSpeed: 1.2,
              brightness: 1.2,
              gain: 1.2,
              bassGain: 1.5,
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
            name: "Waveform",
            props: {
              strokeWeight: 2,
              maxHeight: 65,
              maxWidth: 0.85,
              windowing: "hanning",
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
            name: "Slit Scan",
            props: {
              spacing: 0.85,
              line_width: 0.3,
              angle: 0.18,
              shift: 0.45,
            },
          },
          {
            name: "Posterize",
            props: {
              gamma: 1.15,
              numColors: 8,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Waveform",
        control: "alpha",
        feature: "bass",
        min: 0.15,
        max: 0.8,
      },
    ],
  },

  {
    name: "Wormhole Descent",
    introducedVersion: 4,
    minSeconds: 9,
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
            name: "Point10 Wormhole",
            props: {
              warpSpeed: 1.2,
              bloom: 1.1,
              gain: 1.3,
              bassGain: 1.55,
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
            name: "Point10 Spiral Particles",
            props: {
              spinSpeed: 1.2,
              particleSize: 1.1,
              turbulence: 0.25,
              gain: 1.15,
              trebleGain: 1.4,
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
              radius: 0.85,
              amount: 2.1,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 7,
              intensity: 0.5,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Spiral Particles",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.7,
      },
    ],
  },

  {
    name: "Wormhole Cathedral",
    introducedVersion: 4,
    minSeconds: 10,
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
            name: "Point10 Wormhole",
            props: {
              warpSpeed: 0.9,
              bloom: 1.3,
              gain: 1.2,
              midGain: 1.3,
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
            name: "Point10 Chromatic Wave",
            props: {
              distortion: 0.06,
              amplitude: 1.3,
              glow: 1.2,
              gain: 1.2,
              midGain: 1.3,
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
            name: "Kaleidoscope",
            props: {
              sides: 12,
            },
          },
          {
            name: "Mirror",
            props: {
              horizontal: true,
              vertical: false,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 9,
              intensity: 0.55,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Background",
        control: "alpha",
        feature: "bass",
        min: 0.35,
        max: 1,
      },
    ],
  },

  {
    name: "ASCII Sunbeam",
    introducedVersion: 5,
    minSeconds: 9,
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
            name: "Point10 Mycelium",
            props: {
              growthSpeed: 1.1,
              glow: 1.2,
              gain: 1.2,
              bassGain: 1.4,
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
              amplitude: 1.2,
              glow: 1.2,
              gain: 1.15,
              bassGain: 1.35,
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
            name: "ASCII Art",
            props: {
              amount: 0.35,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 6,
              intensity: 0.5,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Chromatic Line",
        control: "alpha",
        feature: "bass",
        min: 0.15,
        max: 0.75,
      },
    ],
  },

  {
    name: "Terminal Bloom",
    introducedVersion: 5,
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
            name: "Point10 Ball Of Fire",
            props: {
              fireIntensity: 26,
              simulationSpeed: 1,
              gain: 1.2,
              bassGain: 1.45,
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
              thickness: 2,
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
            name: "ASCII Art",
            props: {
              amount: 0.5,
            },
          },
          {
            name: "Hue-Saturation",
            props: {
              hue: 0.08,
              saturation: 0.3,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Background",
        control: "alpha",
        feature: "bass",
        min: 0.3,
        max: 1,
      },
    ],
  },

  {
    name: "Halftone Harbor",
    introducedVersion: 5,
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
            name: "Point10 Water Ripple",
            props: {
              speed: 0.8,
              distortion: 0.06,
              gain: 1.15,
              midGain: 1.3,
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
              maxHeight: 60,
              maxWidth: 0.85,
              windowing: "hanning",
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
            name: "CMYK Halftone",
            props: {
              gridSize: 45,
              smoothing: 0.15,
            },
          },
          {
            name: "Pixellate",
            props: {
              cell_size: 0.05,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Crosshatch Cipher",
    introducedVersion: 5,
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
              scrollSpeed: 1,
              brightness: 1.1,
              gain: 1.2,
              bassGain: 1.4,
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
              intensity: 9,
              spacing: 3,
              objectDistance: 55,
              strokeWeight: 2,
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
            name: "Edges",
            props: {
              intensity: 40,
              threshold: 0.1,
              sobel: true,
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
    inputLinks: [
      {
        group: "Foreground",
        module: "Concentrics",
        control: "alpha",
        feature: "bass",
        min: 0.15,
        max: 0.8,
      },
    ],
  },

  {
    name: "Dither District",
    introducedVersion: 5,
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
            name: "Point10 Cube Floor",
            props: {
              height: 4,
              glow: 1.2,
              gain: 1.2,
              bassGain: 1.45,
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
              intensity: 6,
              shapeSize: 120,
              strokeWeight: 3,
              rotateToggle: true,
              rotateSpeed: 2,
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
            name: "Dither-Bayer",
            props: {
              colorize: 0,
              matrixMode: 2,
            },
          },
          {
            name: "Dot Screen",
            props: {
              angle: 0.3,
              scale: 4,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Polygon",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.7,
      },
    ],
  },

  {
    name: "Mosaic Meridian",
    introducedVersion: 5,
    minSeconds: 11,
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
            name: "Point10 Chromatic Wave",
            props: {
              distortion: 0.06,
              amplitude: 1.2,
              glow: 1.2,
              gain: 1.2,
              midGain: 1.35,
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
              u_ringWidth: 0.05,
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
            name: "Color Posterize",
            props: {
              levels: 12,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 8,
              intensity: 0.55,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Hexagon Halo",
    introducedVersion: 5,
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
            name: "Point10 Spiral Particles",
            props: {
              spinSpeed: 1.1,
              particleSize: 1.1,
              turbulence: 0.2,
              gain: 1.15,
              trebleGain: 1.35,
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
              particleSize: 0.16,
              speed: 1.1,
              opacity: 0.8,
              gain: 1.1,
              trebleGain: 1.3,
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
            name: "Radial Replicate",
            props: {
              numberOfDivisions: 12,
              postRotateAngle: 0,
            },
          },
          {
            name: "Vignette",
            props: {
              intensity: 20,
              extension: 0.35,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Background",
        module: "Point10 Spiral Particles",
        control: "alpha",
        feature: "bass",
        min: 0.25,
        max: 1,
      },
    ],
  },

  {
    name: "Circuit Ritual",
    introducedVersion: 5,
    minSeconds: 9,
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
            name: "Point10 Smoke Room",
            props: {
              smokeDensity: 24,
              smokeLifespan: 4,
              simulationSpeed: 1.2,
              gain: 1.15,
              bassGain: 1.4,
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
              height: 1.5,
              speed: 1,
              opacity: 0.8,
              gain: 1.2,
              midGain: 1.25,
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
            name: "RGB Halftone",
            props: {
              gridSize: 50,
              smoothing: 0.12,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 7,
              intensity: 0.5,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Waveform",
        control: "alpha",
        feature: "bass",
        min: 0.15,
        max: 0.8,
      },
    ],
  },

  {
    name: "Etched Aurora",
    introducedVersion: 5,
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
            name: "Point10 Rolling Fire",
            props: {
              fireIntensity: 24,
              simulationSpeed: 0.9,
              gain: 1.2,
              bassGain: 1.4,
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
              amplitude: 1.1,
              glow: 1.3,
              gain: 1.2,
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
            name: "Emboss",
            props: {
              intensity: 2,
              colorize: 0.5,
              brightness: 0.5,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 6,
              intensity: 0.5,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 Chromatic Line",
        control: "alpha",
        feature: "bass",
        min: 0.15,
        max: 0.8,
      },
    ],
  },

  {
    name: "Brickwork Beat",
    introducedVersion: 5,
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
            name: "Point10 Light Tunnel",
            props: {
              scrollSpeed: 1.1,
              glow: 1.2,
              bloom: 1.1,
              gain: 1.2,
              bassGain: 1.4,
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
            name: "Line",
            props: {
              linesToShow: 60,
              spacing: 2,
              lineWidth: 2,
              speed: 1,
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
            name: "Color Invert",
            props: {},
          },
          {
            name: "Edge Trace",
            props: {
              intensity: 3,
              spread: 5,
            },
          },
          {
            name: "Vignette",
            props: {
              intensity: 16,
              extension: 0.28,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        control: "alpha",
        feature: "bass",
        min: 0.2,
        max: 0.85,
      },
    ],
  },
];

export default DEFAULT_SCENES;
