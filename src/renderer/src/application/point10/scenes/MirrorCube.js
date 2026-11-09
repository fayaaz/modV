// Mirror Cube: the Plane9 "Mirror Cube" preset (author Joakim Dahl, CC BY-NC-SA
// 4.0) ported to a standalone native three.js scene - no XML, no P9Scene node
// graph.
//
// A 1024x1024 RenderToTexture carries the mirror: its own fbm shader paints the
// base, Shader3 draws the inset edge frame over it, and the inner unit cube is
// drawn on top with Shader1 sampling the *stored* previous frame (StoreTexture
// feedback). The outer cube uses the same Expression rotation and samples the
// current frame, so each face shows a nested reflection of the cube inside it.
// There is no bloom and no PreviousLayer node. Unlike the preset's opaque
// black screen clear, the scene clears to alpha 0 so it composites over the
// layers beneath it.
//
// gl_InstanceID / GLSL3 are not needed: the two objects are a plain inner and
// outer BoxGeometry, and the fullscreen passes are a 2x2 PlaneGeometry.
//
// Expression1 (RandomSeed = 1):
//   perm += beat(0.1) * 0.1 * dt
//   out1 = noise2(perm, 0) * 360
//   out2 = noise2(perm, 1) * 360
//   out3 = noise2(perm, 2) * 360
// The per-scene phase is a stable seeded random (PERM_SEED), not Math.random.
// Expression2 zooms the camera with the mid band:
//   out1 = -(band(-1, 1, 1, 0.1) * 5 - 3) + 1.5

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { seededRand } from "../shared/random.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";

// RenderToTexture1 is WidthCustom/HeightCustom 1024 x 1024.
export const RTT_SIZE = 1024;

// Screen1: CamPos 0 0 z, lookAt 0 0 -1, fov 45, near 0.1, far 1000. The z
// port is Expression2's output, 4.5 at silence and 5 units closer per band.
export const CAMERA = { fov: 45, near: 0.1, far: 1000, z: 4.5 };
export const CAMERA_ZOOM_PER_BAND = 5;
// Keep the camera outside the outer cube (centre z 1.2, rotated half-diagonal
// ~0.87) so loud mid bands cannot fly through it.
export const CAMERA_MIN_Z = 2.2;

// Clear2 is the opaque screen clear in the preset; modV composites the scene
// transparently over the layers below instead.
export const CLEAR_ALPHA = 0;

// RenderToTexture1's own Shader port: _fbm(vec3(tex * 10, time * 0.1), 3).
export const FBM_OCTAVES = 3;
export const FBM_SCALE = 10;
export const FBM_TIME_SCALE = 0.1;

// RenderObject1 / Cube1 and RenderObject2 / Cube2: unit cubes at z 1 and 1.2,
// the inner one slightly oversized on X/Y.
export const INNER_POSITION = { x: 0, y: 0, z: 1 };
export const OUTER_POSITION = { x: 0, y: 0, z: 1.2 };
export const INNER_SCALE = { x: 1.01251, y: 1.01251, z: 1 };
export const CUBE_SIZE = 1;

// Guard against tab-suspend catch-up frames: P9's Expression deltatime is
// clamped by long frames, so the phase cannot jump.
export const MAX_DT = 0.1;

// Expression1's per-scene phase, fixed so the cube orientation is stable across
// reloads instead of jumping with Math.random().
export const PERM_SEED = seededRand(31, 8);

const DEG = Math.PI / 180;

const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
const finite = (value) => (Number.isFinite(value) ? value : 0);
const smooth = (t) => t * t * (3 - 2 * t);

// Plane9's value-noise (noise2 in expressions): a seeded hash over the four
// integer cell corners, smoothstep-interpolated, mapped to [-1, 1).
export function valueNoise2(x, y) {
  const ix = Math.floor(finite(x));
  const iy = Math.floor(finite(y));
  const fx = smooth(finite(x) - ix);
  const fy = smooth(finite(y) - iy);
  let value = 0;

  for (let dy = 0; dy <= 1; dy++) {
    for (let dx = 0; dx <= 1; dx++) {
      const corner = seededRand(ix + dx, iy + dy);
      value += corner * (dx ? fx : 1 - fx) * (dy ? fy : 1 - fy);
    }
  }

  return value * 2 - 1;
}

// Pure Expression1: the beat advances the phase and each axis reads noise2.
// spinScale is the modV input that scales the beat-driven rotation.
export function mirrorCubeDrive(beat, dt, perm, spinScale = 1) {
  const b = clamp(finite(beat), 0, 1);
  const step = clamp(finite(dt), 0, MAX_DT);
  const base = Number.isFinite(perm) ? perm : PERM_SEED;
  const next = base + b * 0.1 * step;
  const scale = Number.isFinite(spinScale) ? spinScale : 1;

  return {
    perm: next,
    rotation: {
      x: valueNoise2(next, 0) * 360 * scale,
      y: valueNoise2(next, 1) * 360 * scale,
      z: valueNoise2(next, 2) * 360 * scale,
    },
  };
}

export function makePerm(seed = PERM_SEED) {
  return Number.isFinite(seed) ? seed : PERM_SEED;
}

// Pure Expression2: camera z = -(band * 5 - 3) + 1.5, kept outside the cube.
export function mirrorCameraZ(midBand) {
  const b = clamp(finite(midBand), 0, 1);
  return clamp(CAMERA.z - b * CAMERA_ZOOM_PER_BAND, CAMERA_MIN_Z, CAMERA.z);
}

function buildFullscreenVertexShader() {
  return `
precision highp float;
varying vec2 vTex;
void main() {
  vTex = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;
}

export function buildMirrorCubeFbmShader() {
  return `
precision highp float;
uniform float gTime;
varying vec2 vTex;
${COMMON_PREAMBLE}
void main() {
  vec3 v = vec3(_fbm(vec3(vTex.xy * vec2(${FBM_SCALE.toFixed(1)}), gTime * ${FBM_TIME_SCALE.toFixed(1)}), ${FBM_OCTAVES.toFixed(1)}));
  gl_FragColor = vec4(v, 1.0);
}
`;
}

export function buildMirrorCubeFrameShader() {
  return `
precision highp float;
varying vec2 vTex;
void main() {
  vec2 p = vTex * vec2(2.0) - vec2(1.0);
  // Shader3 writes smoothstep(0.96, 0.95, |p|) with reversed edges. GLSL leaves
  // that undefined, so spell out the same ramp (1 inside 0.95, 0 past 0.96)
  // with standard edges.
  float band = (1.0 - smoothstep(0.95, 0.96, abs(p.x))) * (1.0 - smoothstep(0.95, 0.96, abs(p.y)));
  float inner = smoothstep(0.04, 0.05, abs(p.x)) * smoothstep(0.04, 0.05, abs(p.y));
  float v = band * inner;
  gl_FragColor = vec4(vec3(v), 1.0);
}
`;
}

export function buildMirrorCubeInnerVertexShader() {
  return `
precision highp float;
varying vec2 vTex;
varying vec3 vNormal;
void main() {
  vTex = uv;
  vNormal = normalize(mat3(modelMatrix) * normal);
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;
}

export function buildMirrorCubeInnerFragmentShader() {
  return `
precision highp float;
uniform sampler2D gTexture1;
varying vec2 vTex;
varying vec3 vNormal;
void main() {
  vec3 normal = normalize(vNormal);
  float v = dot(vec3(0.0, 1.0, 0.0), normal) * 0.5 + 0.5;
  gl_FragColor = vec4(texture2D(gTexture1, vTex).rgb * 0.5 * v, 1.0);
}
`;
}

export function buildMirrorCubeOuterVertexShader() {
  return `
precision highp float;
varying vec2 vTex;
void main() {
  vTex = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;
}

export function buildMirrorCubeOuterFragmentShader() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform float gBrightness;
varying vec2 vTex;
void main() {
  vec4 c = texture2D(gTexture1, vTex);
  gl_FragColor = vec4(c.rgb * gBrightness, c.a);
}
`;
}

function makeRenderTarget(size) {
  const target = new THREE.WebGLRenderTarget(size, size, {
    type: THREE.UnsignedByteType,
  });
  target.texture.wrapS = target.texture.wrapT = THREE.RepeatWrapping;
  target.texture.minFilter = THREE.LinearFilter;
  target.texture.magFilter = THREE.LinearFilter;
  target.texture.generateMipmaps = false;
  return target;
}

export class MirrorCube extends BaseScene {
  _init() {
    this.label = "Mirror Cube";
    this.role = "standalone";

    // Parent can wire inputs for these later (sceneProps stays untouched).
    this.spinScale = 1;
    this.brightnessScale = 1;

    this.innerMaterial = new THREE.ShaderMaterial({
      vertexShader: buildMirrorCubeInnerVertexShader(),
      fragmentShader: buildMirrorCubeInnerFragmentShader(),
      uniforms: {
        gTexture1: { value: null },
      },
      depthTest: true,
      depthWrite: true,
      side: THREE.FrontSide,
    });

    this.outerMaterial = new THREE.ShaderMaterial({
      vertexShader: buildMirrorCubeOuterVertexShader(),
      fragmentShader: buildMirrorCubeOuterFragmentShader(),
      uniforms: {
        gTexture1: { value: null },
        gBrightness: { value: 1 },
      },
      depthTest: true,
      depthWrite: true,
      side: THREE.FrontSide,
    });

    this.fbmMaterial = new THREE.ShaderMaterial({
      vertexShader: buildFullscreenVertexShader(),
      fragmentShader: buildMirrorCubeFbmShader(),
      uniforms: {
        gTime: { value: 0 },
      },
      depthTest: false,
      depthWrite: false,
      side: THREE.FrontSide,
    });

    this.frameMaterial = new THREE.ShaderMaterial({
      vertexShader: buildFullscreenVertexShader(),
      fragmentShader: buildMirrorCubeFrameShader(),
      depthTest: false,
      depthWrite: false,
      side: THREE.FrontSide,
    });

    this.innerMesh = new THREE.Mesh(
      new THREE.BoxGeometry(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
      this.innerMaterial,
    );
    this.innerMesh.position.set(
      INNER_POSITION.x,
      INNER_POSITION.y,
      INNER_POSITION.z,
    );
    this.innerMesh.scale.set(INNER_SCALE.x, INNER_SCALE.y, INNER_SCALE.z);
    this.innerMesh.frustumCulled = false;

    this.outerMesh = new THREE.Mesh(
      new THREE.BoxGeometry(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
      this.outerMaterial,
    );
    this.outerMesh.position.set(
      OUTER_POSITION.x,
      OUTER_POSITION.y,
      OUTER_POSITION.z,
    );
    this.outerMesh.frustumCulled = false;
    this.group.add(this.outerMesh);

    // The inner cube renders only into the RTT, never the screen chain.
    this.rttScene = new THREE.Group();
    this.rttScene.add(this.innerMesh);

    this._quadScene = new THREE.Group();
    this._quad = new THREE.Mesh(
      new THREE.PlaneGeometry(2, 2),
      this.fbmMaterial,
    );
    this._quad.frustumCulled = false;
    this._quadScene.add(this._quad);
    this._ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
    this._ortho.updateProjectionMatrix();

    this.rttCamera = new THREE.PerspectiveCamera(
      CAMERA.fov,
      1,
      CAMERA.near,
      CAMERA.far,
    );

    this._rttA = null;
    this._rttB = null;
    this._prevTarget = null;
    this._curTarget = null;

    if (this.renderer) {
      this._rttA = makeRenderTarget(RTT_SIZE);
      this._rttB = makeRenderTarget(RTT_SIZE);
      this._prevTarget = this._rttA;
      this._curTarget = this._rttB;
    }

    if (this.renderer) {
      this.renderer.setClearColor(0x000000, CLEAR_ALPHA);
      this.renderer.toneMapping = THREE.NoToneMapping;

      this.composer = new EffectComposer(this.renderer);
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.addPass(
        new RenderPass(this.group, this.camera, null, null, CLEAR_ALPHA),
      );
      this.composer.addPass(new OutputPass());
    }

    this.p9audio = new P9Audio(this.audio);
    this.perm = makePerm();
    this._cameraZ = CAMERA.z;
    this.size = { w: 2, h: 2 };
    if (this.renderer) {
      const s = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: s.x, h: s.y };
    }
    this._applyCamera(this.size.w, this.size.h, this._cameraZ);
  }

  _applyCamera(w, h, z = CAMERA.z) {
    this._cameraZ = z;

    const apply = (camera, aspect) => {
      if (!camera) return;
      camera.fov = CAMERA.fov;
      camera.near = CAMERA.near;
      camera.far = CAMERA.far;
      camera.aspect = aspect;
      camera.position.set(0, 0, z);
      camera.lookAt(0, 0, -1);
      camera.updateMatrixWorld();
      camera.matrixWorldInverse.copy(camera.matrixWorld).invert();
      camera.updateProjectionMatrix();
    };

    apply(this.camera, w / Math.max(1, h));
    apply(this.rttCamera, 1);
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the cube stops responding.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const drive = mirrorCubeDrive(
      this.p9audio.beat(0.1),
      dt,
      this.perm,
      this.spinScale,
    );
    this.perm = drive.perm;

    this.innerMesh.rotation.set(
      drive.rotation.x * DEG,
      drive.rotation.y * DEG,
      drive.rotation.z * DEG,
    );
    this.outerMesh.rotation.copy(this.innerMesh.rotation);

    this._applyCamera(
      this.size.w,
      this.size.h,
      mirrorCameraZ(this.p9audio.band(-1, 1, 1, 0.1)),
    );

    this.outerMaterial.uniforms.gBrightness.value = this.brightnessScale;
    this.fbmMaterial.uniforms.gTime.value = this.time;
  }

  get paintsCanvas() {
    return true;
  }

  get captureTexture() {
    return this.composer?.readBuffer?.texture ?? null;
  }

  _renderFeedback() {
    const renderer = this.renderer;
    const previousTarget = renderer.getRenderTarget();
    const previousAutoClear = renderer.autoClear;
    const previousClearColor = renderer.getClearColor(new THREE.Color());
    const previousClearAlpha = renderer.getClearAlpha();

    renderer.autoClear = false;
    renderer.setRenderTarget(this._curTarget);
    renderer.setClearColor(0x000000, 1);
    renderer.clear(true, true, false);

    this._quad.material = this.fbmMaterial;
    this.fbmMaterial.uniforms.gTime.value = this.time;
    renderer.render(this._quadScene, this._ortho);

    this._quad.material = this.frameMaterial;
    renderer.render(this._quadScene, this._ortho);

    this.innerMaterial.uniforms.gTexture1.value = this._prevTarget.texture;
    renderer.render(this.rttScene, this.rttCamera);

    renderer.autoClear = previousAutoClear;
    renderer.setClearColor(previousClearColor, previousClearAlpha);
    renderer.setRenderTarget(previousTarget);
  }

  _swapFeedback() {
    const next = this._prevTarget;
    this._prevTarget = this._curTarget;
    this._curTarget = next;
  }

  render() {
    if (!this.renderer || !this.composer || !this._curTarget) {
      super.render();
      return;
    }

    this._renderFeedback();
    this.outerMaterial.uniforms.gTexture1.value = this._curTarget.texture;

    const renderToScreen = !this._captureNextFrame;
    this._captureNextFrame = false;

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

    this._swapFeedback();
  }

  resize(w, h) {
    this.size = { w, h };
    this._applyCamera(w, h, this._cameraZ);
  }
}
