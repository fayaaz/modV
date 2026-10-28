// Circular sprite maps for Point10's particle scenes.
//
// THREE.Points draws square fragments unless its material has a map, and an
// untextured billboard quad is square too. These helpers build one radial
// alpha map that both paths share, so particles read as round dots.

// Fraction of the sprite radius that stays fully opaque; the alpha ramps down
// from here to the edge so the dot has a soft outline instead of a hard cut.
export const PARTICLE_SPRITE_INNER = 0.55;

const smoothstep = (edge0, edge1, x) => {
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));

  return t * t * (3 - 2 * t);
};

// Alpha for a normalised sprite coordinate. (0,0) is the top-left corner,
// (0.5,0.5) the centre, so the corners fall outside the unit circle.
export function particleAlphaAt(x, y, inner = PARTICLE_SPRITE_INNER) {
  const dx = x * 2 - 1;
  const dy = y * 2 - 1;
  const radius = Math.sqrt(dx * dx + dy * dy);

  return 1 - smoothstep(inner, 1, radius);
}

// White RGBA map whose alpha is the circular particle mask. Keeping the RGB
// channels white means it multiplies a particle's vertex colour unchanged.
export function buildParticleAlphaMap(size = 64) {
  const data = new Uint8Array(size * size * 4);
  const denom = Math.max(1, size - 1);

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      const alpha = Math.round(particleAlphaAt(x / denom, y / denom) * 255);

      data[i] = 255;
      data[i + 1] = 255;
      data[i + 2] = 255;
      data[i + 3] = alpha;
    }
  }

  return data;
}
