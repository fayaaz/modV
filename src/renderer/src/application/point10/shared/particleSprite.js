// Circular sprite maps for Point10's particle scenes.
//
// THREE.Points draws square fragments unless its material has a map, and an
// untextured billboard quad is square too. These helpers build one radial
// alpha map that both paths share, so particles read as round dots.

import * as THREE from "three";
import { buildParticleAlphaMap } from "./particleAlpha.js";

export {
  PARTICLE_SPRITE_INNER,
  buildParticleAlphaMap,
  particleAlphaAt,
} from "./particleAlpha.js";

export function createParticleTexture(size = 64) {
  const texture = new THREE.DataTexture(
    buildParticleAlphaMap(size),
    size,
    size,
    THREE.RGBAFormat,
  );
  texture.name = "Point10ParticleSprite";
  texture.minFilter = THREE.LinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.needsUpdate = true;

  return texture;
}
