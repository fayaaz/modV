import { FADE_TRANSITION } from "../scene-transition-defs";

/**
 * Picks the crossfade style for a Scene change.
 *
 * While performance mode is running every change is a straight crossfade:
 * shader wipes and instant "None" switches can be jarring between grid cells,
 * so the allowed-transitions list only applies to manual Scene changes made
 * while performance mode is stopped.
 *
 * @param {Object} args
 * @param {Boolean} args.performanceEnabled     whether performance mode is running
 * @param {Array}   args.allowedTransitions     names the user allowed
 * @param {Array}   args.availableTransitionNames names that actually exist
 * @param {Function} [args.random]              RNG for picking at random
 * @returns {String} transition name
 */
export function pickTransitionName({
  performanceEnabled,
  allowedTransitions,
  availableTransitionNames,
  random = Math.random,
}) {
  if (performanceEnabled) {
    return FADE_TRANSITION;
  }

  const available = new Set(availableTransitionNames);
  const allowed = allowedTransitions.filter((name) => available.has(name));

  if (!allowed.length) {
    return FADE_TRANSITION;
  }

  return allowed[Math.floor(random() * allowed.length)];
}
