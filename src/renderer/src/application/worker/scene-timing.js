export const DEFAULT_PERFORMANCE_TIMING = Object.freeze({
  minSeconds: 8,
  maxSeconds: 24,
  transitionSeconds: 2,
});

const MIN_SCENE_SECONDS = 1;
export const MIN_TRANSITION_SECONDS = 0.1;

function toFiniteNumber(value) {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }

  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);

    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

function roundTenth(value) {
  return Math.round(value * 10) / 10;
}

function normalizeCellBounds(minSeconds, maxSeconds) {
  const minValue = toFiniteNumber(minSeconds);
  const maxValue = toFiniteNumber(maxSeconds);
  const min =
    minValue === null ? DEFAULT_PERFORMANCE_TIMING.minSeconds : minValue;
  const max =
    maxValue === null ? DEFAULT_PERFORMANCE_TIMING.maxSeconds : maxValue;

  return {
    min,
    max: Math.max(min, max),
  };
}

/**
 * Picks the dwell target for a single performance cell. The target is the
 * cell's own maximum time: it is chosen once when the cell becomes active and
 * stays fixed until that cell changes, so rows advance independently.
 */
export function pickCellDwell({
  minSeconds,
  maxSeconds,
  random = Math.random,
} = {}) {
  const { min, max } = normalizeCellBounds(minSeconds, maxSeconds);
  const roll = Number(typeof random === "function" ? random() : Number.NaN);
  const clampedRoll = Number.isFinite(roll)
    ? Math.min(1, Math.max(0, roll))
    : 0;

  return clampCellDwell(min + clampedRoll * (max - min), min, max);
}

export function clampCellDwell(value, minSeconds, maxSeconds) {
  const { min, max } = normalizeCellBounds(minSeconds, maxSeconds);
  const numericValue = toFiniteNumber(value);

  // A missing/invalid target means "no earlier deadline than the maximum",
  // which keeps the legacy maxSeconds behaviour.
  if (numericValue === null) {
    return max;
  }

  return Math.min(max, Math.max(min, numericValue));
}

export function normalizeTransitionDuration(value) {
  const transition = toFiniteNumber(value);

  return transition === null
    ? DEFAULT_PERFORMANCE_TIMING.transitionSeconds
    : Math.max(MIN_TRANSITION_SECONDS, roundTenth(transition));
}

export function normalizePerformanceTiming(performance = {}) {
  const source = performance || {};
  const min = toFiniteNumber(source.minSeconds);
  const max = toFiniteNumber(source.maxSeconds);
  const transition = toFiniteNumber(source.transitionSeconds);

  const minSeconds =
    min === null
      ? DEFAULT_PERFORMANCE_TIMING.minSeconds
      : Math.max(MIN_SCENE_SECONDS, roundTenth(min));

  let maxSeconds =
    max === null
      ? Math.max(minSeconds + 1, DEFAULT_PERFORMANCE_TIMING.maxSeconds)
      : Math.max(MIN_SCENE_SECONDS + 1, roundTenth(max));

  if (maxSeconds <= minSeconds) {
    maxSeconds = roundTenth(minSeconds + 1);
  }

  const transitionSeconds =
    transition === null
      ? DEFAULT_PERFORMANCE_TIMING.transitionSeconds
      : Math.max(MIN_TRANSITION_SECONDS, roundTenth(transition));

  return {
    ...source,
    minSeconds,
    maxSeconds,
    transitionSeconds,
  };
}

export function normalizeSceneTiming(scene = {}, performance = {}) {
  const globalTiming = normalizePerformanceTiming(performance);
  const source = scene || {};
  const timingOverride = source.timingOverride === true;
  const min = timingOverride ? toFiniteNumber(source.minSeconds) : null;
  const max = timingOverride ? toFiniteNumber(source.maxSeconds) : null;

  const minSeconds =
    min === null
      ? globalTiming.minSeconds
      : Math.max(MIN_SCENE_SECONDS, roundTenth(min));

  let maxSeconds =
    max === null
      ? globalTiming.maxSeconds
      : Math.max(MIN_SCENE_SECONDS + 1, roundTenth(max));

  if (maxSeconds <= minSeconds) {
    maxSeconds = roundTenth(minSeconds + 1);
  }

  return {
    minSeconds,
    maxSeconds,
    timingOverride,
  };
}

export function resolveSceneTiming(scene, performance) {
  return normalizeSceneTiming(scene, performance);
}
