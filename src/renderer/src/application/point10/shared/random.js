// Deterministic [0,1) hash used by the Point10 scenes that need repeatable
// "random" choices (AttractorParticles' colour drop pulses). Vendored from
// Point10's plane9/P9Expression.js so the scenes do not pull in the whole
// expression parser.
export function seededRand(a = 0, b = 0) {
  let h = (a | 0) * 374761393 + (b | 0) * 668265263;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}
