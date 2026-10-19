/* eslint-env worker */

/**
 * Dependency-free port of musicvis' phrase detector (src/shared/phraseColor.js).
 *
 * A phrase boundary ("drop") is the bass band re-charging out of its recent
 * trough: the climb must be strong (peakRise past riseFrac of the level,
 * floored by the AGC-scaled floor), reach the crest region (peakFrac of the
 * session running max) and roll over (bass < prev), with rearmSec spacing the
 * flips. modV's audio-bands AGC normalizes the session peak to ~1.0, which is
 * the same scale assumption musicvis' constants were tuned against.
 */

export const DROP = {
  riseFrac: 0.4,
  peakFrac: 0.75,
  floor: 0.05,
  releaseSec: 2,
  rearmSec: 10,
};

export const createPhraseState = () => ({
  t: 0,
  low: undefined,
  phraseMax: undefined,
  prev: undefined,
  peakRise: 0,
});

/**
 * Advances the phrase state by dt seconds of bass energy.
 *
 * @param {Object} state Previous phrase state
 * @param {Number} bass  Current bass band level (0-1, AGC normalized)
 * @param {Number} dt    Elapsed time in seconds
 * @param {Object} opts  Drop detector options
 * @returns {{ state: Object, flipped: Boolean }}
 */
export const updatePhrase = (state, bass, dt, opts = DROP) => {
  const prev = state.prev === undefined ? bass : state.prev;
  const prevLow = state.low === undefined ? bass : state.low;

  // Rise is measured against the previous rest level so the release term
  // cannot eat the climb on the same step.
  const rise = bass - prevLow;

  // Strongest rise of the current ascent only; resets when the bass sinks
  // back to its trough.
  const peakRise = rise <= 0 ? 0 : Math.max(state.peakRise || 0, rise);

  const low = Math.min(
    bass,
    prevLow + (bass - prevLow) * Math.min(1, dt / opts.releaseSec),
  );

  // Session running max (never reset): near-crest is judged against the
  // track's own ceiling.
  const phraseMax = Math.max(
    state.phraseMax === undefined ? bass : state.phraseMax,
    bass,
  );

  const t = state.t + dt;

  const flipped =
    bass < prev &&
    peakRise >= Math.max(opts.riseFrac * Math.max(bass, prevLow), opts.floor) &&
    bass >= opts.peakFrac * phraseMax &&
    t >= opts.rearmSec;

  if (flipped) {
    return {
      state: { t: 0, low: bass, phraseMax, prev: bass, peakRise: rise },
      flipped: true,
    };
  }

  return {
    state: { ...state, t, low, phraseMax, prev: bass, peakRise },
    flipped: false,
  };
};

/**
 * Probability a phrase change switches scenes, ramping linearly from 0 at
 * minSeconds to 1 at maxSeconds. Past maxSeconds the next phrase change
 * always takes it.
 */
export const switchChance = (elapsed, minSeconds, maxSeconds) => {
  if (maxSeconds <= minSeconds) {
    return 1;
  }

  return Math.min(
    1,
    Math.max(0, (elapsed - minSeconds) / (maxSeconds - minSeconds)),
  );
};
