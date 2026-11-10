// Ready-to-go music reactive Scenes. Most are layered the same way musicvis
// builds a look: a Background visual, a Foreground overlay and an Effects group
// that inherits the previous Groups and runs a post effect over them. A Scene is
// not required to have all three (or even more than one) Group, and a filter can
// be chained in any Group - for example a Bloom or Chromatic Abberation inside
// the Foreground group after its generator.
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
// version 2 the second batch, version 3 the glitch/twirl batch,
// version 4 the varied Point10 scene batch, version 5 the ASCII/filter
// variety batch, version 6 the new digital scene batch, version 7 replaces
// the removed Cube Floor visuals and adds Circular Mountain, and version 8
// retires seven Scenes (see REMOVED_DEFAULT_SCENES), renames and updates
// several Scenes from the author's live configuration, turns Twirl Tunnel
// into Golden Spiral Tunnel and adds Rave.exe, and version 9 trims the busiest
// effect stacks, keeps Twirl only in It Twirls At Night, drops RGB Halftone and
// adds single-Group (Background-only, Foreground-only, Effects-only) Scenes.
// Version 9 also rebuilds the trimmed Scenes in place (see
// REFRESHED_DEFAULT_SCENES) for installs that seeded the version 8 set, and
// version 10 treats Bands as a 2D shape, moves it out of Effects rows, removes
// Radial Replicate, moves RGB Shift/Digital Glitch into Effects and retires
// Roto Zoom Riot. Version 11 rebuilds those version 10 edits in place for
// installs that already saved the version 10 defaults before the edits landed.
export const LEGACY_DEFAULTS_VERSION = 1;
export const DEFAULTS_VERSION = 11;

// The Cube Floor module was removed in version 7. Existing installs may still
// hold default Scenes that reference it, so the migration below rebuilds those
// Scenes from their current definitions (or replaces the old name entirely).
export const REMOVED_DEFAULT_SCENE_MODULES = ["Point10 Cube Floor"];

// Default Scenes retired without a replacement. Installs upgrading drop these
// Scenes, but only when they were seeded from the defaults. Version 8 retires
// the first seven; version 10 retires Roto Zoom Riot (reapplied in version 11).
export const REMOVED_DEFAULT_SCENES = [
  "Ferrofluid Pulse",
  "Kaleidoscope Cathedral",
  "Slit Scan Horizon",
  "Spiral Particles",
  "Spectral Bloom",
  "Fractured Kaleidoscope",
  "Broken Mirror Ball",
  "Roto Zoom Riot",
];

const RENAMED_DEFAULT_SCENES = [
  {
    oldName: "Cube Floor Drive",
    newName: "Waveform Drive",
    requiresRemovedModule: true,
  },
  {
    oldName: "VHS Afterglow",
    newName: "Digital Black Hole",
  },
  {
    oldName: "It Comes At Night",
    newName: "It Twirls At Night",
  },
  {
    oldName: "Neon Grid",
    newName: "Smoke Tunnel",
  },
  {
    oldName: "Chromatic Sphere",
    newName: "Chromatic Waveform",
  },
  {
    oldName: "Twirl Tunnel",
    newName: "Golden Spiral Tunnel",
  },
  // Version 9 drops Twirl from every default except It Twirls At Night, so the
  // Scenes that were built around it get names that describe their new look.
  {
    oldName: "Twirl Reactor",
    newName: "Reactor Glow",
  },
  {
    oldName: "Twirl Grid",
    newName: "Polygon Grid",
  },
  {
    oldName: "Twirled Feedback",
    newName: "Feedback Bloom",
  },
];

// Version 9 also trims existing Scenes and version 10 moves Bands out of the
// Effects rows, removes Radial Replicate and moves RGB Shift/Digital Glitch
// into Effects. Installs that seeded the previous defaults set get these
// rebuilt in place (same id/name) so the refinement reaches existing libraries,
// while older installs keep their historical versions. Version 11 re-runs this
// refresh for libraries that saved version 10 before those edits landed.
export const REFRESHED_DEFAULT_SCENES = [
  "Mycelium Bloom",
  "Chromatic Waveform",
  "Geometric Tunnel",
  "Burning Mycelium Circle",
  "Chromatic Cascade",
  "Smoke Tunnel",
  "Digital Black Hole",
  "Mirror Chamber",
  "Solar Flare",
  "Water Ripple Dreams",
  "Waveform Drive",
  "Rolling Fire Bloom",
  "Strobe Riot",
  "Prism Reliquary",
  "Spiral Static",
  "Ultimate Drift",
  "Glitch Bloom Cathedral",
  "Hex Bloom",
  "VHS Reliquary",
  "It Is Coming",
  "It Twirls At Night",
  "Around Plus Aurora",
  "Around Water Garden",
  "Attractor Bloom",
  "Attractor Void",
  "Boxy Boulevard",
  "Boxy Arcade",
  "DVD Afterglow",
  "DVD Nightmare",
  "Light Tunnel Runner",
  "Light Tunnel Choir",
  "Square Tunnel Signal",
  "Wormhole Descent",
  "Wormhole Cathedral",
  "ASCII Sunbeam",
  "Terminal Bloom",
  "Halftone Harbor",
  "Crosshatch Cipher",
  "Dither District",
  "Mosaic Meridian",
  "Circuit Ritual",
  "Hexagon Halo",
  "Etched Aurora",
  "Brickwork Beat",
  "Digital Crystal Tunnel",
  "MBOX Light Tunnel",
  "Rave.exe",
  "Soft Bands",
  "Quiet Bloom",
];

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

function sceneUsesRemovedModule(scene, removedModuleNames) {
  const modules =
    scene && scene.modules && typeof scene.modules === "object"
      ? Object.values(scene.modules)
      : [];

  return modules.some((module) => {
    if (!module || typeof module !== "object") {
      return false;
    }

    const name =
      module.$moduleName || (module.meta && module.meta.name) || null;

    return !!name && removedModuleNames.has(name);
  });
}

function getDefaultDefinition(name) {
  const key = sceneKey(name);

  return DEFAULT_SCENES.find((scene) => sceneKey(scene.name) === key) || null;
}

/**
 * Finds default Scenes that must be rebuilt because they still reference a
 * removed Module. Only Scenes that match a current default name are touched,
 * and only while their serialized Modules still contain a removed name, so a
 * user's edits to a migrated Scene are preserved.
 *
 * The returned `replacements` keep the existing Scene id/name while swapping
 * in the current definition, `removals` are old defaults that no longer exist
 * (a rename) and `additions` are the new definitions to seed in their place.
 */
/**
 * Finds seeded default Scenes that were retired without a replacement (see
 * REMOVED_DEFAULT_SCENES). Only Scenes seeded from the defaults are returned;
 * user Scenes that happen to share a name are preserved. Returns nothing once
 * the install is on the new defaults version.
 */
export function getDefaultScenesToRemove({
  defaultsVersion = 0,
  scenes = [],
  seededNames,
  removedNames = REMOVED_DEFAULT_SCENES,
} = {}) {
  const version = Number.isFinite(Number(defaultsVersion))
    ? Number(defaultsVersion)
    : 0;
  const result = [];

  if (version >= DEFAULTS_VERSION) {
    return result;
  }

  const names = new Set((removedNames || []).map(sceneKey));

  if (!names.size) {
    return result;
  }

  const seeded =
    seededNames === undefined
      ? null
      : new Set((seededNames || []).map(sceneKey));
  const seen = new Set();

  for (const scene of scenes || []) {
    if (!scene || !scene.name) {
      continue;
    }

    const key = sceneKey(scene.name);

    if (!names.has(key) || seen.has(key)) {
      continue;
    }

    if (seeded !== null && !seeded.has(key)) {
      continue;
    }

    seen.add(key);
    result.push({ id: scene.id, name: scene.name });
  }

  return result;
}

export function getDefaultScenesToReplace({
  defaultsVersion = 0,
  scenes = [],
  removedModuleNames = REMOVED_DEFAULT_SCENE_MODULES,
  seededNames,
  refreshNames = REFRESHED_DEFAULT_SCENES,
} = {}) {
  const version = Number.isFinite(Number(defaultsVersion))
    ? Number(defaultsVersion)
    : 0;
  const result = { replacements: [], removals: [], additions: [] };

  if (version >= DEFAULTS_VERSION) {
    return result;
  }

  const removed = new Set((removedModuleNames || []).filter(Boolean));
  const seeded =
    seededNames === undefined
      ? null
      : new Set((seededNames || []).map(sceneKey));
  const isSeeded = (key) => seeded === null || seeded.has(key);

  const existingByKey = new Map();

  for (const scene of scenes || []) {
    if (!scene || !scene.name) {
      continue;
    }

    const key = sceneKey(scene.name);

    if (!existingByKey.has(key)) {
      existingByKey.set(key, scene);
    }
  }

  const consumedKeys = new Set();

  for (const rename of RENAMED_DEFAULT_SCENES) {
    const key = sceneKey(rename.oldName);
    const existing = existingByKey.get(key);

    if (!existing || !isSeeded(key)) {
      continue;
    }

    if (
      rename.requiresRemovedModule &&
      (!removed.size || !sceneUsesRemovedModule(existing, removed))
    ) {
      continue;
    }

    const definition = getDefaultDefinition(rename.newName);

    if (!definition) {
      continue;
    }

    result.removals.push({ id: existing.id, name: existing.name });
    result.additions.push(definition);
    consumedKeys.add(key);
  }

  if (removed.size) {
    for (const definition of DEFAULT_SCENES) {
      const key = sceneKey(definition.name);

      if (consumedKeys.has(key)) {
        continue;
      }

      const existing = existingByKey.get(key);

      if (!existing || !sceneUsesRemovedModule(existing, removed)) {
        continue;
      }

      result.replacements.push({
        id: existing.id,
        name: existing.name,
        definition,
      });
    }
  }

  // Rebuild the Scenes whose definitions changed in this version. Only
  // installs that seeded the immediately previous defaults set are refreshed,
  // so a long-neglected library is not silently rewritten. Existing ids and
  // names are kept, and removed-module/rename work already won.
  const refresh = new Set((refreshNames || []).map(sceneKey));

  if (version >= DEFAULTS_VERSION - 1 && refresh.size) {
    const replacedKeys = new Set(
      result.replacements.map((replacement) => sceneKey(replacement.name)),
    );
    const removedKeys = new Set(
      result.removals.map((removal) => sceneKey(removal.name)),
    );

    for (const definition of DEFAULT_SCENES) {
      const key = sceneKey(definition.name);

      if (
        !refresh.has(key) ||
        replacedKeys.has(key) ||
        removedKeys.has(key) ||
        consumedKeys.has(key)
      ) {
        continue;
      }

      const existing = existingByKey.get(key);

      if (!existing || !isSeeded(key)) {
        continue;
      }

      result.replacements.push({
        id: existing.id,
        name: existing.name,
        definition,
      });
      replacedKeys.add(key);
    }
  }

  return result;
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
        modules: [],
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
    name: "Chromatic Waveform",
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
        modules: [],
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
        modules: [],
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
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
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
    name: "Smoke Tunnel",
    introducedVersion: 2,
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
            name: "Point10 Light Tunnel",
            props: {
              gain: 1.25,
              smoothing: 0.08,
              bassGain: 1.5,
              midGain: 1.1,
              trebleGain: 1,
              useAgc: true,
              scrollSpeed: 1.2,
              glow: 1.2,
              bloom: 1.1,
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
            name: "Point10 Smoke Room",
            props: {
              gain: 1,
              smoothing: 0,
              bassGain: 1,
              midGain: 1,
              trebleGain: 1,
              useAgc: true,
              smokeDensity: 28,
              fireIntensity: 40,
              smokeLifespan: 4.05,
              fireLifespan: 1.3,
              simulationSpeed: 1.2,
              boxScale: 0.9,
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
            name: "Neon",
            props: {
              intensity: 3.5999999999999996,
              gain: 0.9847759065022574,
              neonColor: [1, 0.4, 0.64, 1],
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Effects",
        module: "Neon",
        control: "intensity",
        feature: "bass",
        min: 3.5999999999999996,
        max: 8.399999999999999,
      },
      {
        group: "Effects",
        module: "Neon",
        control: "gain",
        source: "tween",
        min: 0.6000000000000001,
        max: 1,
      },
    ],
  },

  {
    name: "Digital Black Hole",
    introducedVersion: 2,
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
            name: "Point10 Water Ripple",
            props: {
              gain: 1.15,
              smoothing: 0.18,
              bassGain: 1.3,
              midGain: 1.1,
              trebleGain: 1,
              useAgc: true,
              speed: 1,
              distortion: 1,
              radius: 1,
              brightness: 1,
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
            name: "Singularity Rain",
            props: {
              speed: 0,
              density: 1,
              shear: 0.35,
              swirl: 0.5642799587694213,
              horizon: 0.02,
              beaming: 0.2375,
              stars: 1,
              glow: 0.3252663394070569,
              brightness: 1,
            },
            meta: {
              alpha: 0.607261398057038,
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
            name: "VHS Glitch",
            props: {
              autoScan: true,
              xScanline: 0.22455150927394874,
              xScanline2: 0.55,
              yScanline: 0.1,
              xScanlineSize: 0,
              xScanlineSize2: 0.175,
              yScanlineAmount: 0,
              grainLevel: 1.65,
              scanFollow: true,
              analogDistort: 0.5,
              bleedAmount: 0.9772282066010674,
              bleedDistort: 0.7125,
              bleedRange: 1.3,
              colorBleedL: [0.8, 0, 0.4, 1],
              colorBleedC: [0, 0.5, 0.75, 1],
              colorBleedR: [0.8, 0, 0.4, 1],
            },
          },
          // A gentle motion blur rounds the VHS scanline jitter without adding
          // a busy second look.
          {
            name: "VVMotionBlur 3.0",
            props: {
              blurAmount: 0.22,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Singularity Rain",
        control: "alpha",
        feature: "bass",
        min: 0,
        max: 1,
      },
      {
        group: "Foreground",
        module: "Singularity Rain",
        control: "glow",
        feature: "mid",
        min: 0,
        max: 1,
        smoothing: 0.3486748374187093,
      },
      {
        group: "Effects",
        module: "VHS Glitch",
        control: "xScanline",
        source: "tween",
        min: 0,
        max: 1,
        bpmDivision: 128,
      },
      {
        group: "Effects",
        module: "VHS Glitch",
        control: "bleedAmount",
        feature: "bass",
        min: 0.54,
        max: 1.26,
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
        modules: [],
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
        ],
      },
    ],
  },

  {
    name: "Waveform Drive",
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
        modules: [],
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
        ],
      },
    ],
  },

  {
    name: "Golden Spiral Tunnel",
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
            name: "LogTransWarpSpiral",
            props: {
              rate: 0.25,
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
        modules: [],
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
              // A beat-length flash at low alpha reads as a pulse instead of a
              // hard flicker; the tween below keeps the period in time with
              // the detected BPM so it flashes on the beat rather than at a
              // fixed, unrelated rate.
              strobeRate: 0.5,
              strobeMode: 0,
              strobeColor: [1, 1, 1, 1],
            },
            meta: { alpha: 0.16 },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Effects",
        module: "Strobe",
        control: "strobeRate",
        source: "tween",
        min: 0.48,
        max: 0.56,
        bpmDivision: 16,
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
        name: "Foreground",
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
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Color Posterize",
            props: {
              levels: 10,
            },
          },
          {
            name: "Point10 RGB Shift",
            props: {},
            meta: { alpha: 0.75, compositeOperation: "screen" },
          },
        ],
      },
    ],
  },

  {
    name: "Reactor Glow",
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
        name: "Foreground",
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
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Circular Screen",
            props: {
              sharpness: 4,
              offset: 0.15,
              scale: 1.25,
              colorize: 0.55,
              center: [0.5, 0.5],
            },
          },
          {
            name: "Point10 RGB Shift",
            props: {},
            meta: { alpha: 0.7, compositeOperation: "screen" },
          },
        ],
      },
    ],
  },

  {
    name: "Polygon Grid",
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
        modules: [],
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
        name: "Foreground",
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
            name: "Point10 Digital Glitch",
            props: {},
            meta: { alpha: 0.6, compositeOperation: "screen" },
          },
        ],
      },
    ],
  },

  {
    name: "Feedback Bloom",
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
        name: "Foreground",
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
            name: "Point10 RGB Shift",
            props: {},
            meta: { alpha: 0.6, compositeOperation: "screen" },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Effects",
        module: "Point10 RGB Shift",
        control: "alpha",
        feature: "bass",
        min: 0.1,
        max: 0.75,
      },
    ],
  },

  {
    name: "It Twirls At Night",
    introducedVersion: 4,
    minSeconds: 8,
    maxSeconds: 24,
    groups: [
      {
        name: "Foreground",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 It Is Coming For You",
            props: {
              gain: 1.2,
              smoothing: 0,
              bassGain: 1.35,
              midGain: 1.15,
              trebleGain: 1,
              useAgc: true,
              speed: 0.8,
              glow: 0.9,
              bloom: 0.9,
            },
            meta: {
              alpha: 0.7446732251408736,
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
            name: "Twirl",
            props: {
              radius: 0.75,
              amount: -2.4,
              center: [0.5, 0.5],
            },
            meta: {
              alpha: 0.616,
            },
          },
          {
            name: "Bloom",
            props: {
              blurAmount: 5.710711967387224,
              intensity: 0.5638257200751327,
            },
          },
          {
            name: "Point10 Digital Glitch",
            meta: {
              compositeOperation: "screen",
              alpha: 0.65,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "Point10 It Is Coming For You",
        control: "alpha",
        feature: "bass",
        min: 0.25,
        max: 1,
      },
      {
        group: "Effects",
        module: "Bloom",
        control: "blurAmount",
        source: "tween",
        min: 3.5999999999999996,
        max: 8.4,
      },
      {
        group: "Effects",
        module: "Bloom",
        control: "intensity",
        feature: "bass",
        min: 0.3,
        max: 0.7,
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
        modules: [],
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
        ],
      },
    ],
    inputLinks: [],
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
        modules: [],
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
        ],
      },
    ],
    inputLinks: [],
  },

  {
    name: "Boxy Arcade",
    introducedVersion: 4,
    minSeconds: 8,
    maxSeconds: 20,
    groups: [
      {
        name: "Foreground",
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
            name: "Point10 Digital Glitch",
            props: {},
            meta: { alpha: 0.6, compositeOperation: "screen" },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Effects",
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
        name: "Foreground",
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
            name: "Point10 RGB Shift",
            props: {},
            meta: { alpha: 0.65, compositeOperation: "screen" },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Effects",
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
        modules: [],
      },
      {
        name: "Effects",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Posterize",
            props: {
              gamma: 1.2,
              numColors: 7,
            },
          },
        ],
      },
    ],
    inputLinks: [],
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
        modules: [],
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
        ],
      },
    ],
    inputLinks: [],
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
            name: "Mirror",
            props: {
              horizontal: true,
              vertical: false,
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
        modules: [],
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
        modules: [],
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
            name: "Point10 Chromatic Wave",
            props: {
              distortion: 0.06,
              amplitude: 1.2,
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
        modules: [],
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
        modules: [],
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
              intensity: 0.5,
            },
          },
        ],
      },
    ],
    inputLinks: [],
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

  {
    name: "Digital Crystal Tunnel",
    introducedVersion: 6,
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
            name: "digital-crystal-tunnel",
            props: {
              modifier: 1.25,
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
            name: "Point10 Geometric Tunnel",
            props: {
              gain: 1.2,
              bassGain: 1.4,
              midGain: 1.1,
              smoothing: 0.08,
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
            name: "Chromatic Abberation",
            props: {
              rOffset: 1,
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
        module: "Point10 Geometric Tunnel",
        control: "alpha",
        feature: "treble",
        min: 0.2,
        max: 0.8,
      },
    ],
  },

  {
    name: "MBOX Light Tunnel",
    introducedVersion: 6,
    minSeconds: 10,
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
            name: "MBOX3",
            props: {
              SCALE: -1.8,
              // Near zero keeps the fractal threshold sensitive to the shape
              // changes instead of only the loudest bass hits.
              dthresh: 0.05,
              fixedRad2: 2,
              foldingLimit: 1.1,
              minRad2: 0.6,
              dhue: 0.25,
              // A constant, calmer speed: linking this to bass made the
              // fractal race through its detail and look twitchy.
              speed: 24,
              rotation: 0,
              e0: 3.2,
              x: 1,
              y: 0,
              z: 0,
              type: 0,
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
            name: "Point10 Light Tunnel",
            props: {
              scrollSpeed: 0.9,
              glow: 1.2,
              bloom: 1,
              gain: 1,
              bassGain: 1.05,
              trebleGain: 0.6,
              smoothing: 0.35,
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
        module: "Point10 Light Tunnel",
        control: "alpha",
        feature: "bass",
        min: 0.35,
        max: 0.75,
      },
      {
        group: "Background",
        module: "MBOX3",
        control: "SCALE",
        feature: "bass",
        min: -2.6,
        max: -1,
      },
      {
        group: "Background",
        module: "MBOX3",
        control: "dhue",
        source: "tween",
        min: 0.1,
        max: 0.9,
      },
    ],
  },

  {
    name: "Circular Mountain",
    introducedVersion: 7,
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
            name: "fractal-mountain-tunnel",
            props: {
              speed: 0.626,
              rotationSpeed: 0.4,
              hue: 0.1125,
              saturation: 0.075,
              brightness: 1,
              transitionSpeed: 1,
            },
          },
        ],
      },
      {
        name: "Foreground",
        inherit: true,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Circular Screen",
            props: {
              sharpness: 2.7,
              offset: 0,
              scale: 0.35,
              colorize: 0,
              center: [0.5, 0.5],
            },
            meta: { compositeOperation: "normal" },
          },
          {
            name: "Corner Color Tint",
            props: {
              color1: [1, 0, 0, 1],
              color2: [0, 1, 0, 1],
              color3: [0, 0, 1, 1],
              color4: [1, 1, 1, 1],
              rotationAngle: 0.1257,
            },
            meta: { alpha: 0.2488, compositeOperation: "normal" },
          },
          {
            name: "badtv",
            props: {
              noiseLevel: 0.4,
              distortion1: 0.6,
              distortion2: 5,
              speed: 0.3,
              scroll: 0,
              scanLineThickness: 1,
              scanLineIntensity: 0.0349,
              scanLineOffset: 0,
            },
            meta: { compositeOperation: "normal" },
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
              intensity: 0.35,
            },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Background",
        module: "fractal-mountain-tunnel",
        control: "speed",
        feature: "bass",
        min: 0,
        max: 1,
      },
      {
        group: "Foreground",
        module: "Corner Color Tint",
        control: "alpha",
        feature: "rms",
        min: 0,
        max: 2,
      },
      {
        group: "Foreground",
        module: "Corner Color Tint",
        control: "rotationAngle",
        feature: "rms",
        min: 0,
        max: 1,
      },
      {
        group: "Foreground",
        module: "badtv",
        control: "scanLineIntensity",
        feature: "treble",
        min: 0,
        max: 1,
      },
    ],
  },
  {
    name: "Rave.exe",
    introducedVersion: 8,
    minSeconds: 10,
    maxSeconds: 24,
    groups: [
      {
        name: "Foreground",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "rave.exe",
            props: {},
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
              intensity: 0.4,
            },
          },
          {
            name: "Point10 Digital Glitch",
            props: {},
            meta: { alpha: 0.5, compositeOperation: "screen" },
          },
        ],
      },
    ],
    inputLinks: [
      {
        group: "Foreground",
        module: "rave.exe",
        control: "kick",
        feature: "bass",
        min: 0,
        max: 1,
      },
    ],
  },

  // Version 9 adds deliberately sparse Scenes. A Scene does not need a
  // Background, Foreground and Effects layer stack - a single Group is just as
  // valid, and an effect can live in any Group (a filter chained after a
  // generator in the Foreground works exactly like it does in Effects).
  {
    name: "Still Water",
    introducedVersion: 9,
    minSeconds: 16,
    maxSeconds: 34,
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
              gain: 1.05,
              bassGain: 1.1,
              midGain: 0.9,
              trebleGain: 0.8,
              smoothing: 0.35,
              useAgc: true,
              speed: 0.55,
              distortion: 0.5,
              radius: 1,
              brightness: 0.9,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Ember Drift",
    introducedVersion: 9,
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
            // Ball Of Fire and DVD Room are deliberately background-free, so
            // they cannot be the only layer of a Background-only Scene.
            // Mycelium paints its own opaque frame and reads as warm embers.
            name: "Point10 Mycelium",
            props: {
              gain: 1.05,
              bassGain: 1.15,
              midGain: 0.95,
              trebleGain: 0.8,
              smoothing: 0.3,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Lone Chrome",
    introducedVersion: 9,
    minSeconds: 14,
    maxSeconds: 30,
    groups: [
      {
        name: "Foreground",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Chromatic Line",
            props: {
              gain: 1.05,
              bassGain: 1.05,
              midGain: 1.1,
              trebleGain: 1,
              smoothing: 0.22,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Silk Ribbon",
    introducedVersion: 9,
    minSeconds: 14,
    maxSeconds: 30,
    groups: [
      {
        name: "Foreground",
        inherit: false,
        clearing: true,
        alpha: 1,
        compositeOperation: "normal",
        modules: [
          {
            name: "Point10 Chromatic Wave",
            props: {
              gain: 1.05,
              midGain: 1.1,
              trebleGain: 0.9,
              smoothing: 0.28,
            },
          },
          // An effect chained inside the Foreground group: the filter reads the
          // generator drawn just before it.
          {
            name: "Chromatic Abberation",
            props: {
              rOffset: 1,
              gOffset: 1.006,
              bOffset: 1.012,
            },
            meta: { alpha: 0.8 },
          },
        ],
      },
    ],
  },

  {
    name: "Soft Bands",
    introducedVersion: 9,
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
            // Bands is a 2D shape and draws its own spectrum, so it belongs in
            // the Background row rather than the Effects row.
            name: "Bands",
            props: {
              u_ringWidth: 0.06,
            },
          },
        ],
      },
    ],
  },

  {
    name: "Quiet Bloom",
    introducedVersion: 9,
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
            name: "Bands",
            props: {
              u_ringWidth: 0.05,
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
              intensity: 0.35,
            },
          },
        ],
      },
    ],
  },
];

export default DEFAULT_SCENES;
