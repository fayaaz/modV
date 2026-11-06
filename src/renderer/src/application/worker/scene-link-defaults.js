// Defaults for the automatic audio links a Scene receives. The automatic BPM
// Tween deliberately runs slowly and eases in/out so props move musically
// instead of snapping. Structural geometry props are excluded per Module so a
// Scene falls back to a Group alpha (or another Module) rather than warping
// its shape on every beat.

export const DEFAULT_SCENE_BPM_TWEEN = Object.freeze({
  bpmDivision: 32,
  easing: "easeInOutSine",
});

export const AUTO_BASS_EXCLUDED_PROPS = Object.freeze({
  Twirl: Object.freeze(["radius", "amount"]),
  Kaleidoscope: Object.freeze(["sides"]),
  "Kaleidoscope Tile": Object.freeze(["sides", "size", "rotation", "angle"]),
});

export const AUTO_BPM_EXCLUDED_PROPS = Object.freeze({
  Twirl: Object.freeze(["radius", "amount"]),
  Kaleidoscope: Object.freeze(["sides"]),
  "Kaleidoscope Tile": Object.freeze(["sides", "size", "angle"]),
});

const EXCLUDED_PROPS_BY_MODE = {
  bass: AUTO_BASS_EXCLUDED_PROPS,
  bpm: AUTO_BPM_EXCLUDED_PROPS,
};

/**
 * @param {String} moduleName
 * @param {String} propName
 * @param {"bass"|"bpm"} mode
 * @returns {Boolean}
 */
export function isAutoLinkExcluded(moduleName, propName, mode) {
  const excludedByModule = EXCLUDED_PROPS_BY_MODE[mode];

  if (!excludedByModule || !moduleName) {
    return false;
  }

  const excludedProps = excludedByModule[moduleName];

  return Array.isArray(excludedProps) && excludedProps.includes(propName);
}
