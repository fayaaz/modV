export const DEFAULT_PERFORMANCE_TIMING = Object.freeze({
  minSeconds: 8,
  maxSeconds: 24,
  transitionSeconds: 2,
});

const MIN_SCENE_SECONDS = 1;
const MIN_TRANSITION_SECONDS = 0.1;

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
