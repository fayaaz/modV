// Light Tunnel: the Plane9 "Light Tunnel" preset (author Joakim Dahl,
// CC BY-NC-SA 3.0) ported to a standalone native three.js scene - no XML, no
// P9Scene node graph.
//
// The preset is a single fullscreen tunnel fragment shader drawn into a
// RenderToTexture, bloomed, then combined with the original tunnel by a second
// shader that ACES-tonemaps bloom + tunnel. The port keeps the shader math
// verbatim and reproduces the chain with three's RenderPass ->
// UnrealBloomPass -> OutputPass composer (OutputPass' ACES tone mapping is
// the same curve as the preset's _tonemapACES).
//
// Expression port:
//   perm += (band(-1,0,0,0.025) * 3 + 0.01) * dt
//   in1.x = perm + permrand * 10
//   in1.y = min(1, band(-1,1,1,0))
// permrand is a stable, seeded per-scene random.

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { seededRand } from "../shared/random.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";

// Fullscreen quad edge length (clip space is -1..1, so a 2x2 plane fills it).
export const TUNNEL_SIZE = 2;

// The preset's Bloom1 port: Radius 5 / Threshold 0 / Smoothness 0.5 /
// Intensity 0.17. UnrealBloomPass normalises radius to 0..1, hence ~0.85.
export const BLOOM = { strength: 0.17, radius: 0.85, threshold: 0 };

// Expression1's per-scene `permrand`, fixed so the tunnel pattern is stable
// across reloads instead of jumping with Math.random().
export const PERMRAND = seededRand(9, 8);

// Guard against tab-suspend catch-up frames turning silence's 0.01 creep into
// a full pattern jump; P9 deltatime is clamped the same way by long frames.
const MAX_DT = 0.1;

// Pure Expression1: advance perm with the bass band and expose the gIn1
// uniforms (x = tunnel phase, y = clamped band 1 glow). RandomSeed=1 means the
// initial perm is 1 (see makePerm).
export function tunnelDrive(bass, mid, dt, perm) {
  const b = Math.max(0, Number.isFinite(bass) ? bass : 0);
  const m = Math.max(0, Number.isFinite(mid) ? mid : 0);
  const step = Math.max(0, Number.isFinite(dt) ? Math.min(dt, MAX_DT) : 0);
  const base = Number.isFinite(perm) ? perm : 0;
  const next = base + (b * 3 + 0.01) * step;

  return {
    perm: next,
    in1: {
      x: next + PERMRAND * 10,
      y: Math.min(1, m),
    },
  };
}

export function makePerm(seed = 1) {
  return Number.isFinite(seed) ? seed : 1;
}

export function buildTunnelFragment() {
  return `
precision highp float;
uniform vec4 gIn1;
varying vec2 vUv;
${COMMON_PREAMBLE}
void main() {
  // Inspiration taken from tunnel shader by kusma (via the Plane9 preset).
  vec2 p = -1.0 + 2.0 * vUv;
  float r = length(p);
  float a = atan(p.y, p.x) / PI2;

  float d = 0.25 / r + gIn1.x;

  vec3 uv = vec3(a + d, a - d, a + sin(d) * 0.1);
  vec3 r2 = 0.5 + cos(uv * PI * vec3(2.0, 2.0, 6.0)) * 0.5;
  float v = _turbulencefast(r2 * 5.0, 5.0) + 0.5;
  vec3 xx;
  xx.x = _turbulencefast(r2 * 5.0 + v + vec3(1.2, 4.2, 1.2), 3.0) * 0.125;
  xx.y = _turbulencefast(r2 * 5.0 + v + vec3(2.2, 3.2, 2.2), 3.0) * 0.125;
  xx.z = _turbulencefast(r2 * 5.0 + v + vec3(1.1, 2.2, 1.4), 3.0) * 0.125;
  vec3 c = mix(vec3(1.0, 0.86, 0.94), vec3(0.1, 0.1, 0.2), pow(r2.x + xx.x, 0.2)) * 1.75;
  c += mix(vec3(0.84, 0.91, 1.0), vec3(0.1, 0.1, 0.2), pow(r2.y + xx.y, 1.5)) * 0.25;
  c += mix(vec3(0.91, 0.85, 1.0), vec3(0.1, 0.2, 0.2), pow(r2.z + xx.z, 1.5)) * 0.25;

  // Glow parts
  c = mix(c, vec3(0.953, 0.047, 0.090) * vec3(500.0), smoothstep(0.2, 0.5, _fbmfast(r2 * vec3(15.0, 1.0, 1.0), 3.0)) * gIn1.y * smoothstep(0.05, 0.2, r));

  c *= smoothstep(0.01, 0.05, r);
  c *= smoothstep(1.6, 0.5, length(p)); // Vignetting
  gl_FragColor = vec4(v * c * saturate(r) * 0.125, 1.0);
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

export class LightTunnel extends BaseScene {
  _init() {
    this.label = "Light Tunnel";

    // Screen1 is a fullscreen fragment shader, so the faithful three.js
    // equivalent is a quad that writes clip space directly and ignores the
    // camera (CamPos 0 0 -2 / lookAt 0 0 1 / fov 45 are moot for it).
    this.quad = new THREE.Mesh(
      new THREE.PlaneGeometry(TUNNEL_SIZE, TUNNEL_SIZE),
      new THREE.ShaderMaterial({
        vertexShader: VERT,
        fragmentShader: buildTunnelFragment(),
        uniforms: {
          gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
        },
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.quad.frustumCulled = false;
    this.group.add(this.quad);

    // Shader2 combines bloom + tunnel and ACES-tonemaps the result. OutputPass
    // uses the renderer's tone mapping, so ACES here is the preset's curve.
    if (this.renderer) {
      this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
      this.renderer.toneMappingExposure = 1;

      const size = this.renderer.getSize(new THREE.Vector2());
      this.composer = new EffectComposer(this.renderer);
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.addPass(new RenderPass(this.group, this.camera));
      this.composer.addPass(
        new UnrealBloomPass(
          size,
          BLOOM.strength,
          BLOOM.radius,
          BLOOM.threshold,
        ),
      );
      this.composer.addPass(new OutputPass());
    }

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

    // Expression1: bass advances perm, band 1 adds the red glow.
    const drive = tunnelDrive(
      this.p9audio.band(-1, 0, 0, 0.025),
      this.p9audio.band(-1, 1, 1, 0),
      dt,
      this.perm,
    );
    this.perm = drive.perm;
    this.quad.material.uniforms.gIn1.value.set(drive.in1.x, drive.in1.y, 0, 0);
  }

  get paintsCanvas() {
    return true;
  }

  get captureTexture() {
    return this.composer?.readBuffer?.texture ?? null;
  }

  render() {
    const renderToScreen = !this._captureNextFrame;
    this._captureNextFrame = false;
    if (!this.composer) {
      super.render();
      return;
    }
    const w = this.renderer.domElement.width;
    const h = this.renderer.domElement.height;
    if (w !== this._cw || h !== this._ch) {
      this._cw = w;
      this._ch = h;
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.setSize(
        w / this.renderer.getPixelRatio(),
        h / this.renderer.getPixelRatio(),
      );
    }
    this.composer.renderToScreen = renderToScreen;
    this.composer.render();
  }

  resize(w, h) {
    this.size = { w, h };
  }
}
