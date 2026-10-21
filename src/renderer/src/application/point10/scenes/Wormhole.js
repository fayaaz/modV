// Wormhole: the Plane9 "Wormhole" preset (author Joakim Dahl, CC BY-NC-SA
// 4.0) ported to a standalone native three.js scene - no XML, no P9Scene node
// graph.
//
// The preset's RenderToTexture1 is fed by RenderObject1.Render, so P9Scene
// captures the 100k-point wormhole chain rather than the node's own noise
// shader. The captured points are bloomed (Radius 5 / Threshold 0 /
// Smoothness 0.5 / Intensity 0.1) and ACES-composited; PreviousLayer1 resolves
// to black in standalone playback, so there is no frame feedback. The port
// keeps the points shader math verbatim in a THREE.Points with additive
// ONE/ONE blending and reproduces the chain with RenderPass ->
// UnrealBloomPass -> OutputPass (OutputPass' ACES curve is the preset's
// _tonemapACES).
//
// gl_VertexID is replaced with a float attribute aId (0..99999) so the shader
// stays GLSL1 and compiles under WebGL2 without a GLSL3 migration.
//
// Expression port (RandomSeed = 1):
//   perm += min(2, band(-1, 0, 0, 0.1) * 25 + 0.25) * dt
//   in1.x = permrand * 100
//   in1.z = perm + permrand * 10
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

// MeshInstancer1 NumInstances.
export const POINT_COUNT = 100000;
// Shader1's constant point size and the Expression's phase constants.
export const POINT_SIZE = 5;
export const POINT_DIST = 300;
export const SPEED = 0.5;

// The preset's Bloom1 port. UnrealBloomPass normalises radius to 0..1, hence
// ~0.85 for Plane9's Radius 5.
export const BLOOM = { strength: 0.1, radius: 0.85, threshold: 0 };

// Screen1: CamPos 0 0 0, lookAt 0 0 -1, fov 60, near 0.01, far 50.
export const CAMERA = { fov: 60, near: 0.01, far: 50 };

// Expression1's per-scene `permrand`, fixed so the wormhole pattern is stable
// across reloads instead of jumping with Math.random(). in1.x (unused by the
// shader) still exposes it for parity with the preset.
export const PERMRAND = seededRand(21, 8);

// Pure Expression1: advance perm with the bass band and expose the gIn1
// uniforms (x = permrand * 100, y unused, z = phase).
export function wormholeDrive(bass, dt, perm) {
  const b = Math.max(0, Number.isFinite(bass) ? bass : 0);
  const step = Math.max(0, Number.isFinite(dt) ? dt : 0);
  const base = Number.isFinite(perm) ? perm : 0;
  const next = base + Math.min(2, b * 25 + 0.25) * step;

  return {
    perm: next,
    in1: {
      x: PERMRAND * 100,
      y: 0,
      z: next + PERMRAND * 10,
    },
  };
}

export function makePerm(seed = 1) {
  return Number.isFinite(seed) ? seed : 1;
}

export function buildWormholeVertexShader() {
  return `
precision highp float;
uniform vec4 gIn1;
uniform vec4 gIn3;
attribute float aId;
varying vec4 vDiffuse;
${COMMON_PREAMBLE}
void main() {
  float d = aId / 100000.0;

  vec3 nv = vec3(0.0, 0.0, 0.0);

  float speed = 0.5;
  float t = fract(d - gIn1.z * 0.01); // Current point pos in the graph
  float pointDist = 300.0;
  float tt = t * pointDist;
  float paramSet = floor(d + gIn1.z * speed);

  float k = _rand(vec2(paramSet, 0.123));
  float l = _rand(vec2(paramSet, 2.572));
  float scale = 3.0;

  float kInv = 1.0 - k;
  nv.x = (kInv * cos(tt) + (l * k) * cos((kInv / k) * tt)) * scale;
  nv.y = (kInv * sin(tt) - (l * k) * sin((kInv / k) * tt)) * scale;
  nv.z = fract(d + gIn1.z * speed + gIn3.x) * 20.0 - 20.0;

  vec3 pos = vec3(nv.x * 2.0, nv.y * 2.0, nv.z * 1.5);
  gl_Position = projectionMatrix * modelViewMatrix * vec4(pos, 1.0);
  gl_PointSize = 5.0;

  vDiffuse = vec4(_hsv2rgb(l, 0.75 + k * 0.25, 0.9), 1.0) * vec4(0.25);
  vDiffuse.a = smoothstep(0.1, 0.2, length(nv.xy)); // Fade out at center
  vDiffuse.a *= smoothstep(-20.0, -5.0, nv.z); // Fade by distance
  vDiffuse.a *= smoothstep(0.0, -0.5, nv.z); // Fade by distance
}
`;
}

export function buildWormholeFragmentShader() {
  return `
precision highp float;
varying vec4 vDiffuse;
${COMMON_PREAMBLE}
void main() {
  vec2 p = gl_PointCoord * 2.0 - 1.0;
  gl_FragColor = vec4(
    saturate(smoothstep(1.0, 0.9, length(p)) * vDiffuse.rgb) * vDiffuse.a,
    1.0
  );
}
`;
}

export class Wormhole extends BaseScene {
  _init() {
    this.label = "Wormhole";

    // One vertex per instance: the shader derives the whole wormhole from
    // aId, so position only exists to give three a draw count.
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.BufferAttribute(new Float32Array(POINT_COUNT * 3), 3),
    );
    const ids = new Float32Array(POINT_COUNT);
    for (let i = 0; i < POINT_COUNT; i++) ids[i] = i;
    geometry.setAttribute("aId", new THREE.BufferAttribute(ids, 1));

    this.material = new THREE.ShaderMaterial({
      vertexShader: buildWormholeVertexShader(),
      fragmentShader: buildWormholeFragmentShader(),
      uniforms: {
        gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
        gIn3: { value: new THREE.Vector4(0, 0, 0, 0) },
      },
      transparent: true,
      depthTest: false,
      depthWrite: false,
      blending: THREE.CustomBlending,
      blendSrc: THREE.OneFactor,
      blendDst: THREE.OneFactor,
      blendEquation: THREE.AddEquation,
    });

    this.points = new THREE.Points(geometry, this.material);
    this.points.frustumCulled = false;
    this.group.add(this.points);

    if (this.renderer) {
      this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
      this.renderer.toneMappingExposure = 1;
      this.renderer.setClearColor(0x000000, 1);

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
    this._applyCamera(this.size.w, this.size.h);
  }

  _applyCamera(w, h) {
    const cam = this.camera;
    cam.fov = CAMERA.fov;
    cam.near = CAMERA.near;
    cam.far = CAMERA.far;
    cam.aspect = w / Math.max(1, h);
    cam.position.set(0, 0, 0);
    cam.lookAt(0, 0, -1);
    cam.updateMatrixWorld();
    cam.matrixWorldInverse.copy(cam.matrixWorld).invert();
    cam.updateProjectionMatrix();
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the wormhole freezes.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const drive = wormholeDrive(
      this.p9audio.band(-1, 0, 0, 0.1),
      dt,
      this.perm,
    );
    this.perm = drive.perm;
    this.material.uniforms.gIn1.value.set(
      drive.in1.x,
      drive.in1.y,
      drive.in1.z,
      0,
    );
    this.material.uniforms.gIn3.value.set(0, 0, 0, 0);
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
    this._applyCamera(w, h);
  }
}
