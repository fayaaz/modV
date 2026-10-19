// Phrase colour logic shared by the scenes that flip ROYGBIV on musical
// phrases (RollingFire, SmokeRoom, AttractorParticles, AroundPlus). Moved
// here from RollingFire.js - it is scene-agnostic: a drop detector on the
// bass band plus the palette of hue rotations it cycles through.

// Colour change on musical phrase boundaries. TSL `hue()` rotates around the
// white axis in radians; the angles below are verified to map the default
// red/amber ramp onto each ROYGBIV fire gradient (negative = toward
// yellow/green, positive = toward blue/violet).
export const ROYGBIV_HUES = [
  0, // red (default ramp)
  -Math.PI / 12, // orange
  -Math.PI / 4, // yellow
  -(2 * Math.PI) / 3, // green
  (2 * Math.PI) / 3, // blue
  (3 * Math.PI) / 4, // indigo
  (7 * Math.PI) / 12, // violet
];

// Colour change driven by the MUSIC, not a clock and with no tempo constants:
// a "drop" is the sub band re-charging out of its recent trough, and the flip
// fires when that climb is BOTH strong and near the crest of the current
// phrase - landing on the change itself at any tempo (the sub pre-rolls the
// drop by ~1 bar; requiring the crest trims that lead with no time constant).
// Everything is scale-free fractions of the signal's own level, except one
// amplitude floor: the plane9 AGC already normalizes the session peak to ~1.0,
// so a quiet section's micro-pulses (~0.02) can never be a drop while real
// re-charges sit 0.05+, and this floor rides that global scale - it is not
// tied to any one track or BPM. A rest level follows bass down instantly and
// revives over releaseSec (a smoothing constant); rearmSec floors the spacing.
export const DROP = {
  riseFrac: 0.4,
  peakFrac: 0.75,
  floor: 0.05,
  releaseSec: 2,
  rearmSec: 10,
};
// Pick a ROYGBIV rotation different from the current hue, so every change shows.
export function pickRoygbivHue(avoid, rand = Math.random) {
  const pool = ROYGBIV_HUES.filter((h) => Math.abs(h - avoid) > 0.2);
  const choices = pool.length ? pool : ROYGBIV_HUES;
  return choices[Math.floor(rand() * choices.length)];
}
// Performance dwell: probability a phrase change switches to a new scene,
// ramping linearly 0 at minSec to 1 at maxSec (clamped) - min..max is the
// window where the switch is music-gated with growing odds; past max the
// next phrase change always takes it.
export const switchChance = (elapsed, minSec, maxSec) =>
  Math.min(1, Math.max(0, (elapsed - minSec) / (maxSec - minSec)));

// state: { hue, t, low, phraseMax, prev, peakRise } -> next state.
export const dropColorPulse = (state, bass, dt, opts = DROP, rand) => {
  const prev = state.prev === undefined ? bass : state.prev;
  const prevLow = state.low === undefined ? bass : state.low;
  // rise is measured against the PREVIOUS rest level: the release term must not
  // eat the climb on the same step (a coarse dt would swallow a real drop).
  const rise = bass - prevLow;
  // peakRise = the strongest rise of the CURRENT ascent only: it resets to 0
  // once the bass sinks back to its trough (rise <= 0), so the first drop's big
  // rise can't leak into later re-charges. The fire test uses it rather than the
  // rollover frame's rise, which the trough-release has already shrunk.
  const peakRise = rise <= 0 ? 0 : Math.max(state.peakRise ?? 0, rise);
  const low = Math.min(
    bass,
    prevLow + (bass - prevLow) * Math.min(1, dt / opts.releaseSec),
  );
  // phraseMax is the session's running sub-bass max (never reset): near-crest
  // is judged against the track's own ceiling, so the first climb of a section
  // can't fire early - it only fires once it approaches its own top. The AGC
  // normalizes the session, so this ceiling is track-adaptive, not a constant.
  const phraseMax = Math.max(state.phraseMax ?? bass, bass);
  const t = state.t + dt;
  // The crest IS the change, and the only way to know a crest without peeking
  // ahead is to see the climb roll over (bass < prev). The other gates reject
  // wobble and quiet chatter: the climb must have been strong (peakRise past
  // riseFrac of the level, floored by the AGC-scaled floor) and reach the
  // crest region (peakFrac of the phrase ceiling), plus the rearm floor.
  if (
    bass < prev &&
    peakRise >= Math.max(opts.riseFrac * Math.max(bass, prevLow), opts.floor) &&
    bass >= opts.peakFrac * phraseMax &&
    t >= opts.rearmSec
  ) {
    return {
      hue: pickRoygbivHue(state.hue, rand),
      t: 0,
      low: bass,
      phraseMax,
      prev: bass,
      peakRise: rise,
    };
  }
  return { ...state, t, low, phraseMax, prev: bass, peakRise };
};

// CPU twin of TSL's hue() (three/src/nodes/display/ColorAdjustment.js): a
// Rodrigues rotation of the RGB vector about the white axis. The ROYGBIV_HUES
// angles were verified against the fire ramp in that color space, so applying
// them here lands CPU-driven materials on the same palette the TSL shaders
// show (see AroundPlus' rings).
import * as THREE from "three";
const _hueAxis = new THREE.Vector3(0.57735, 0.57735, 0.57735);
const _hueV = new THREE.Vector3();
export function hueRotate(color, angle, target = new THREE.Color()) {
  _hueV.set(color.r, color.g, color.b);
  const cos = Math.cos(angle),
    sin = Math.sin(angle);
  const dot = _hueAxis.dot(_hueV);
  const x =
    _hueV.x * cos +
    (_hueAxis.y * _hueV.z - _hueAxis.z * _hueV.y) * sin +
    _hueAxis.x * dot * (1 - cos);
  const y =
    _hueV.y * cos +
    (_hueAxis.z * _hueV.x - _hueAxis.x * _hueV.z) * sin +
    _hueAxis.y * dot * (1 - cos);
  const z =
    _hueV.z * cos +
    (_hueAxis.x * _hueV.y - _hueAxis.y * _hueV.x) * sin +
    _hueAxis.z * dot * (1 - cos);
  return target.setRGB(Math.max(0, x), Math.max(0, y), Math.max(0, z));
}

// fireHue's luminance equalization on the CPU: every ROYGBIV state reads as
// bright as the green one (blue/violet otherwise sink to dark).
const _greenRef = new THREE.Color();
export function equalizedHue(base, angle, target = new THREE.Color()) {
  hueRotate(base, angle, target);
  hueRotate(base, ROYGBIV_HUES[4], _greenRef);
  const lr = 0.2126 * target.r + 0.7152 * target.g + 0.0722 * target.b;
  const lg = 0.2126 * _greenRef.r + 0.7152 * _greenRef.g + 0.0722 * _greenRef.b;
  const k = lg / Math.max(1e-4, lr);
  return target.setRGB(
    Math.max(0, target.r * k),
    Math.max(0, target.g * k),
    Math.max(0, target.b * k),
  );
}
