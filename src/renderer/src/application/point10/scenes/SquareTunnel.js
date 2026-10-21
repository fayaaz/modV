// Square Tunnel: the Plane9 "Square Tunnel" preset (author Joakim Dahl,
// CC BY-NC-SA 4.0) ported to a standalone native three.js scene - no XML, no
// P9Scene node graph.
//
// The preset is a single fullscreen fragment shader fed by checkerboard_diffuse
// and a black opaque Clear. The shader builds a squircle radius (power 15),
// turns it into polar texture coordinates and samples the checkerboard with
// repeat wrapping, so the square tube recedes forever. There is no bloom or
// composer in the preset chain, so BaseScene's single render is enough.
//
// Expression port (RandomSeed = 1):
//   perm += (band(-1, 0, 0, 0.1) * 20 + 0.01) * dt
//   t = perm + permrand * 10
//   in1.x = beat(0) * 2
//   in1.y = t
// permrand is a stable, seeded per-scene random.

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { seededRand } from "../shared/random.js";
import { loadTexture } from "../shared/textures.js";
import checkerUrl from "../textures/checkerboard_diffuse.png?url";

// Fullscreen quad edge length (clip space is -1..1, so a 2x2 plane fills it).
export const TUNNEL_SIZE = 2;

// Expression1's per-scene `permrand`, fixed so the checkerboard phase is
// stable across reloads instead of jumping with Math.random().
export const PERMRAND = seededRand(1, 0);

// Guard against tab-suspend catch-up frames turning silence's 0.01 creep into
// a full pattern jump; P9 deltatime is clamped the same way by long frames.
const MAX_DT = 0.1;

// Pure Expression1: advance perm with the bass band, expose gIn1.
// in1.x = beat * 2 (the shader's overall brightness term),
// in1.y = perm + permrand * 10 (the checkerboard scroll phase).
export function squareTunnelDrive(bass, beat, dt, perm) {
  const b = Math.max(0, Number.isFinite(bass) ? bass : 0);
  const bt = Math.max(0, Math.min(1, Number.isFinite(beat) ? beat : 0));
  const step = Math.max(0, Number.isFinite(dt) ? Math.min(dt, MAX_DT) : 0);
  const base = Number.isFinite(perm) ? perm : 1;
  const next = base + (b * 20 + 0.01) * step;

  return {
    perm: next,
    in1: {
      x: bt * 2,
      y: next + PERMRAND * 10,
    },
  };
}

export function makePerm(seed = 1) {
  return Number.isFinite(seed) ? seed : 1;
}

export function buildSquareTunnelFragment() {
  return `
precision highp float;
uniform vec4 gIn1;
uniform sampler2D gTexture1;
varying vec2 vUv;
${COMMON_PREAMBLE}
void main() {
  // Shader1, verbatim: power-15 squircle radius -> polar texture lookup.
  vec2 p = -1.0 + 2.0 * vUv;
  vec2 uv;
  float a = atan(p.y, p.x);

  float power = 15.0;
  float r = pow(pow(p.x * p.x, power) + pow(p.y * p.y, power), 1.0 / (2.0 * power));
  uv.x = 0.3 * gIn1.y + 0.5 / r;
  uv.y = a / PI;

  vec3 col = texture(gTexture1, uv * 3.0).xyz;
  gl_FragColor = vec4(col * r * (0.5 + gIn1.x) * length(p), 1.0);
}
`;
}

const VERT = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

function placeholderTexture(data) {
  const texture = new THREE.DataTexture(
    new Uint8Array(data),
    1,
    1,
    THREE.RGBAFormat,
  );
  texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
  texture.needsUpdate = true;
  return texture;
}

export class SquareTunnel extends BaseScene {
  _init() {
    this.label = "Square Tunnel";

    // Screen1 is a fullscreen fragment shader, so the faithful three.js
    // equivalent is a quad that writes clip space directly and ignores the
    // camera (CamPos 0 0 -2 / lookAt 0 0 1 / fov 45 are moot for it).
    this.quad = new THREE.Mesh(
      new THREE.PlaneGeometry(TUNNEL_SIZE, TUNNEL_SIZE),
      new THREE.ShaderMaterial({
        vertexShader: VERT,
        fragmentShader: buildSquareTunnelFragment(),
        uniforms: {
          gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
          gTexture1: { value: placeholderTexture([128, 128, 128, 255]) },
        },
        depthTest: false,
        depthWrite: false,
        // Shader1 CullMode 2: the fullscreen quad is double-sided.
        side: THREE.DoubleSide,
      }),
    );
    this.quad.frustumCulled = false;
    this.group.add(this.quad);

    // FileTexture1: data/checkerboard.png. The uv*3 sampling walks past 1, so
    // the checkerboard needs repeat wrapping. Neutral 1x1 placeholder keeps
    // the shader valid until the Vite asset resolves.
    const maxAniso = this.renderer?.capabilities?.getMaxAnisotropy?.() ?? 8;
    loadTexture(checkerUrl)
      .then((texture) => {
        texture.colorSpace = THREE.SRGBColorSpace;
        texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
        texture.anisotropy = maxAniso;
        this.quad.material.uniforms.gTexture1.value = texture;
      })
      .catch((error) => {
        console.warn("SquareTunnel checkerboard texture failed:", error);
      });

    this.p9audio = new P9Audio(this.audio);
    this.perm = makePerm(1);
    this.size = { w: 2, h: 2 };
    if (this.renderer) {
      const s = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: s.x, h: s.y };
    }
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the tunnel freezes.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    // Expression1: bass advances the scroll phase, beat flashes the tube.
    const drive = squareTunnelDrive(
      this.p9audio.band(-1, 0, 0, 0.1),
      this.p9audio.beat(0),
      dt,
      this.perm,
    );
    this.perm = drive.perm;
    this.quad.material.uniforms.gIn1.value.set(drive.in1.x, drive.in1.y, 0, 0);
  }

  render() {
    if (!this.renderer) return;
    // Clear1: black, opaque. The quad covers the frame, but the clear keeps
    // the scene opaque at the edges/resize instead of transparent.
    this.renderer.setClearColor(0x000000, 1);
    this.renderer.render(this.group, this.camera);
  }

  resize(w, h) {
    this.size = { w, h };
  }
}
