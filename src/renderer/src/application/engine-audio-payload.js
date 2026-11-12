// WS-9: the `meyda/SET_BANDS` payload the renderer's audio pump sends to the
// Rust engine.
//
// The legacy worker receives the whole Meyda `features` bag through
// `postMessage` (`type: "meyda"`), so it needs no serialisation step. The
// engine is driven over the JSON control socket, so the arrays the Rust
// `AudioFrame` stores are converted to plain number arrays here. Keeping this
// conversion in one pure function lets `tests/unit/engine-audio-payload.test.js`
// pin the wire shape without booting Electron.

/** The engine's `AudioFrame` array lengths. */
export const ENGINE_AUDIO_SAMPLE_COUNT = 1024;
export const ENGINE_AUDIO_LANE_COUNT = 12;
export const ENGINE_AUDIO_CELL_COUNT = 16;

/** Meyda scalar features the engine forwards for Input Links. */
export const ENGINE_AUDIO_SCALAR_FEATURES = [
  "rms",
  "zcr",
  "energy",
  "spectralCentroid",
  "spectralFlatness",
  "spectralSlope",
  "spectralRolloff",
  "spectralSpread",
  "spectralSkewness",
  "spectralKurtosis",
  "perceptualSpread",
  "perceptualSharpness",
];

/**
 * Convert a typed array (or plain array) to a number array.
 *
 * Absent values become an empty array; the engine treats a missing/short
 * array as "keep the zeroed tail", so an empty array is a valid silent frame.
 *
 * @param {ArrayLike<number> | null | undefined} value
 * @returns {number[]}
 */
export function toNumberArray(value) {
  if (!value || typeof value.length !== "number") {
    return [];
  }

  return Array.from(value, (entry) => {
    const number = Number(entry);
    return Number.isFinite(number) ? number : 0;
  });
}

/**
 * A finite number, falling back to `fallback` for `undefined`/`NaN`/`±∞`.
 *
 * @param {unknown} value
 * @param {number} fallback
 * @returns {number}
 */
export function finiteNumber(value, fallback = 0) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

/**
 * Build the `meyda/SET_BANDS` payload from one `loop()` feature bag.
 *
 * The shape matches `crates/modv-renderers/src/two_d/mod.rs::AudioFrame`
 * exactly (camelCase wire keys):
 *
 * | key | length | source |
 * |-----|--------|--------|
 * | `bass`/`mid`/`treble` | 1 | `features.bass/mid/treble` (dB-mapped band features) |
 * | `lanes`/`rawLanes` | 12 | `features.p10.lanes` / `features.p10.raw.lanes` |
 * | `cells`/`rawCells` | 16 | `features.p10.cells` / `features.p10.raw.cells` |
 * | `wave` | 1024 | `features.p10.wave` (-1..1) |
 * | `freqMag` | 1024 | `features.p10.freqMag` (linear magnitudes) |
 * | `beat` | 1 | `features.p10.beat` (kick envelope) |
 * | `sampleRate` | 1 | `features.p10.sampleRate` (fallback 44 100) |
 * | `byteFrequencyData` | ≤1024 | `features.byteFrequencyData` (512 today) |
 * | `byteTimeDomainData` | ≤1024 | `features.byteTimeDomainData` (512 today) |
 * | scalar features | 1 each | `features[name]` |
 *
 * @param {object | null | undefined} features the `loop()` feature bag
 * @returns {object} the engine wire payload
 */
export function buildEngineAudioPayload(features) {
  const p10 = features?.p10 || {};
  const raw = p10.raw || {};
  const payload = {
    bass: finiteNumber(features?.bass),
    mid: finiteNumber(features?.mid),
    treble: finiteNumber(features?.treble),
    lanes: toNumberArray(p10.lanes),
    cells: toNumberArray(p10.cells),
    rawLanes: toNumberArray(raw.lanes),
    rawCells: toNumberArray(raw.cells),
    beat: finiteNumber(p10.beat),
    wave: toNumberArray(p10.wave),
    freqMag: toNumberArray(p10.freqMag),
    sampleRate: finiteNumber(p10.sampleRate, 44100),
    byteFrequencyData: toNumberArray(features?.byteFrequencyData),
    byteTimeDomainData: toNumberArray(features?.byteTimeDomainData),
  };

  for (const name of ENGINE_AUDIO_SCALAR_FEATURES) {
    payload[name] = finiteNumber(features?.[name]);
  }

  return payload;
}
