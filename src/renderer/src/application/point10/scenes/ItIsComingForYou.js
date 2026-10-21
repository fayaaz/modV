// It Is Coming For You: the Plane9 "It Is Coming For You" preset (author
// Joakim Dahl, CC BY-NC-SA 3.0) ported to a standalone native three.js scene -
// no XML, no P9Scene node graph.
//
// The preset is one MeshInstancer of 30000 additive/alpha-blended POINTS with
// a custom Shader1: every instance is a noise-driven cloud point rotated by an
// axis-angle quaternion, coloured from HSV and lit by two lamps. The points
// render into RenderToTexture1, Bloom1 blurs the capture, and Shader2 combines
// the points (gTexture2), the bloom (gTexture1) and a dot background with
// _blendScreen before saturating. Clear2's dark red is the screen background.
// The Clear1 -> Shader3/Blur/Copy/Store chain is not connected to Screen1 in
// the preset, so it is invisible and is deliberately not implemented.
//
// gl_VertexID is replaced with a float attribute aInstance (0..29999) so the
// shader stays GLSL1 and compiles under WebGL2 without a GLSL3 migration.
//
// The preset's chain maps onto three's composer:
//   RenderPass (clear dark red) -> background dots quad -> 30000 points ->
//   UnrealBloomPass (strength 1, radius 0.85, threshold 0.5) -> OutputPass
// with NoToneMapping. UnrealBloom composites its bloom additively whereas
// Shader2 screen-blends it; with the preset's dark base the difference is
// small. OutputPass still sRGB-encodes the composer's linear buffer, while
// Plane9 wrote its values straight to the frame (see the parity note below).
//
// Expression1 (RandomSeed = 1):
//   perm += (band(-1, 0, 0, 0.1) * 20 + 0.1) * dt
//   in1.x = min(band(-1, 0, 1, 1) * 2, 2) * 2   // spots
//   in1.y = min(band(-1, 0, 0, 0.1) * 5, 1) * 0.25
//   in1.z = perm + permrand * 10
//   in2   = (0.222, 0.087, 0)
// permrand is a stable, seeded per-scene random. Shader2's Expression is
// in1.x = random() every frame - the background sparkle seed (uSeed).

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
export const INSTANCE_COUNT = 30000;

// The preset's Bloom1 port: Radius 5 / Threshold 0.5 / Smoothness 0.5 /
// Intensity 1. UnrealBloomPass normalises radius to 0..1, hence ~0.85.
export const BLOOM = { strength: 1, radius: 0.85, threshold: 0.5 };

// Clear2: the screen background colour.
export const CLEAR_COLOR = { r: 0.592, g: 0.075, b: 0.075 };

// Screen1: CamPos 0 0 -2.73, CamLookAt 0 0 1, fov 45, near 0.1, far 10.
export const CAMERA = {
  fov: 45,
  near: 0.1,
  far: 10,
  position: [0, 0, -2.73],
  lookAt: [0, 0, 1],
};

// Expression1's per-scene `permrand`, fixed so the point cloud pattern is
// stable across reloads instead of jumping with Math.random().
export const PERMRAND = seededRand(18, 8);

// Expression1's In2 = (0.222, 0.087, 0): the depth cubic-pulse band.
export const IN2 = { x: 0.222, y: 0.087, z: 0 };

// Guard against tab-suspend catch-up frames turning the scroll into a jump;
// P9 deltatime is clamped the same way by long frames.
const MAX_DT = 0.1;

const nonNegative = (value) => Math.max(0, Number.isFinite(value) ? value : 0);

// Pure Expression1: bass advances the point cloud, mid opens the spot term and
// the bass band also sets the glow. in2 stays the preset's fixed band.
export function itIsComingDrive(bass, mid, dt, perm) {
  const b = nonNegative(bass);
  const m = nonNegative(mid);
  const step = Math.min(nonNegative(dt), MAX_DT);
  const base = Number.isFinite(perm) ? perm : 1;
  const next = base + (b * 20 + 0.1) * step;

  return {
    perm: next,
    in1: {
      x: Math.min(m * 2, 2) * 2,
      y: Math.min(b * 5, 1) * 0.25,
      z: next + PERMRAND * 10,
    },
    in2: { ...IN2 },
  };
}

export function makePerm(seed = 1) {
  return Number.isFinite(seed) ? seed : 1;
}

export function buildItIsComingVertexShader() {
  return `
precision highp float;
uniform vec4 gIn1;
uniform vec2 gTargetSize;
uniform float gTime;
attribute float aInstance;
varying vec4 vDiffuse;
varying vec3 vWPos;
varying vec3 vWNormal;
varying vec3 vViewDir;
varying float vViewDepth;
${COMMON_PREAMBLE}
void main() {
  vec4 iPosition = vec4(position, 1.0);
  float t = gIn1.z*0.1;
  vec4 v = iPosition;
  float spacing = 2.0;
  float instanceIDFloat = aInstance;
  float d = instanceIDFloat / 30000.0;
  float aspect = gTargetSize.x / gTargetSize.y;

  vec4 rotation = vec4(0.0, 0.0, 1.0, instanceIDFloat * 0.01 + gIn1.z);
  vec4 qRotation = _axisAngleToQuaternion(rotation.xyz, rotation.w);

  vec3 woldPos;
  woldPos.x = _noise(vec3(d*spacing + t, 0.12, 0.0));
  woldPos.y = _noise(vec3(d*spacing + t, 1.23, 0.0));
  woldPos.z = 0.0;
  gl_PointSize = 50.0 + (_rand(vec2(d*spacing))*10.0);
  vec2 offset = vec2(sin(d*731871.0), sin(d*8234981.0))*0.1;
  v.xy = offset;
  vec3 normal = normalize(v.xyz);
  v.xyz = _rotateVector(v.xyz, qRotation);
  normal = _rotateVector(normal, qRotation);
  v.xyz += woldPos*vec3(aspect, 1.0, 1.0);
  gl_Position = (projectionMatrix * modelViewMatrix) * v;

  vDiffuse.rgb = _hsv2rgb(d*0.001, 0.8, 0.8)*2.0*smoothstep(0.985, 1.0, _rand(vec2(d)))*gIn1.xxx;
  vDiffuse.rgb += smoothstep(0.99, 1.0, sin(d*20.0 + gTime*4.0)*0.5 + 0.5)*gIn1.y;
  vDiffuse.rgb *= smoothstep(0.991, 0.99, d);
  vDiffuse.a = d;

  // three's view z is negative; the preset's depth ring needs the positive
  // distance from the camera.
  vViewDepth = -(modelViewMatrix * vec4(v.x, v.y, d, 1.0)).z;
  vWPos = (modelMatrix * v).xyz;
  vWNormal = normalize(mat3(normalMatrix) * normal);
  vViewDir = normalize(cameraPosition - vWPos);
}
`;
}

export function buildItIsComingFragmentShader() {
  return `
precision highp float;
uniform vec4 gIn2;
uniform float gZNear;
uniform float gZFar;
varying vec4 vDiffuse;
varying vec3 vWPos;
varying vec3 vWNormal;
varying vec3 vViewDir;
varying float vViewDepth;
${COMMON_PREAMBLE}
void main() {
  vec2 p = gl_PointCoord*vec2(2.0)-vec2(1.0);

  vec3 normal = normalize(vWNormal);
  vec3 viewDir = normalize(vViewDir);
  vec4 diffuseCol = vDiffuse*0.1;
  vec3 c = vDiffuse.xyz;

  vec3 lightDir = normalize(vec3(0.0, 1.0, 0.0)-vWPos);
  c += _lightLambert(normal, lightDir, vec3(0.5, 0.8, 1.0))*0.025;

  lightDir = normalize(vec3(0.0, -1.0, 1.0)-vWPos);
  c += _lightLambert(normal, lightDir, vec3(0.9, 0.5, 0.5))*0.05;

  float depth = (vViewDepth - gZNear)/(gZFar - gZNear);
  float blurAmount = 1.0 - _cubicpulse(depth, gIn2.x, gIn2.y);
  float ringSize = 0.5;
  blurAmount = clamp(blurAmount*0.4, 0.0, ringSize);
  float blur = smoothstep(ringSize + blurAmount, ringSize - blurAmount, length(p));
  gl_FragColor = vec4(c*5.0, blur);
}
`;
}

const BACKGROUND_VERT = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

// Shader2's dot background, without the (unconnected) RenderRect2 chain:
//   bg = vec3(0.15 + _rand(uv*100 + seed)*0.025), vignetted and squared,
//   then screen-blended over the preset's dark red.
export function buildBackgroundFragmentShader() {
  return `
precision highp float;
uniform float uSeed;
varying vec2 vUv;
${COMMON_PREAMBLE}
void main() {
  vec2 p = vUv*vec2(2.0)-vec2(1.0);
  vec3 bg = vec3(0.15 + _rand(vUv*100.0 + uSeed)*0.025);
  bg *= smoothstep(1.5, 0.0, length(p));
  bg = bg*bg;
  vec3 c = _blendScreen(vec3(0.592, 0.075, 0.075), bg);
  gl_FragColor = vec4(saturate(c), 1.0);
}
`;
}

export class ItIsComingForYou extends BaseScene {
  _init() {
    this.label = "It Is Coming For You";

    // One vertex per instance: the shader derives the whole cloud from
    // aInstance, so position only exists to give three a draw count.
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.BufferAttribute(new Float32Array(INSTANCE_COUNT * 3), 3),
    );
    const ids = new Float32Array(INSTANCE_COUNT);
    for (let i = 0; i < INSTANCE_COUNT; i++) ids[i] = i;
    geometry.setAttribute("aInstance", new THREE.BufferAttribute(ids, 1));

    this.material = new THREE.ShaderMaterial({
      vertexShader: buildItIsComingVertexShader(),
      fragmentShader: buildItIsComingFragmentShader(),
      uniforms: {
        gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
        gIn2: { value: new THREE.Vector4(IN2.x, IN2.y, IN2.z, 0) },
        gTime: { value: 0 },
        gTargetSize: { value: new THREE.Vector2(2, 2) },
        gZNear: { value: CAMERA.near },
        gZFar: { value: CAMERA.far },
      },
      transparent: true,
      depthTest: false,
      depthWrite: false,
      blending: THREE.NormalBlending,
    });

    this.points = new THREE.Points(geometry, this.material);
    this.points.frustumCulled = false;
    this.points.renderOrder = 1;
    this.group.add(this.points);

    // Fullscreen clip-space quad: the dot background is screen content, not a
    // world object, so it ignores the camera exactly like Shader2.
    this.background = new THREE.Mesh(
      new THREE.PlaneGeometry(2, 2),
      new THREE.ShaderMaterial({
        vertexShader: BACKGROUND_VERT,
        fragmentShader: buildBackgroundFragmentShader(),
        uniforms: {
          uSeed: { value: 0 },
        },
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.background.frustumCulled = false;
    this.background.renderOrder = -1;
    this.group.add(this.background);

    this.p9audio = new P9Audio(this.audio);
    this.perm = makePerm(1);
    this.size = { w: 2, h: 2 };
    if (this.renderer) {
      const s = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: s.x, h: s.y };
    }
    this._applyCamera(this.size.w, this.size.h);

    if (this.renderer) {
      // OutputPass tone-maps with the renderer's setting; NoToneMapping keeps
      // the preset's direct colour output (the saturate lives in the shaders).
      this.renderer.toneMapping = THREE.NoToneMapping;
      this.renderer.toneMappingExposure = 1;
      this.renderer.setClearColor(
        new THREE.Color(CLEAR_COLOR.r, CLEAR_COLOR.g, CLEAR_COLOR.b),
        1,
      );

      const size = this.renderer.getSize(new THREE.Vector2());
      this.composer = new EffectComposer(this.renderer);
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.addPass(
        new RenderPass(
          this.group,
          this.camera,
          null,
          new THREE.Color(CLEAR_COLOR.r, CLEAR_COLOR.g, CLEAR_COLOR.b),
          1,
        ),
      );
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
  }

  _applyCamera(w, h) {
    const cam = this.camera;
    if (!cam) return;
    cam.fov = CAMERA.fov;
    cam.near = CAMERA.near;
    cam.far = CAMERA.far;
    cam.aspect = w / Math.max(1, h);
    cam.position.set(...CAMERA.position);
    cam.lookAt(...CAMERA.lookAt);
    cam.updateMatrixWorld();
    cam.matrixWorldInverse.copy(cam.matrixWorld).invert();
    cam.updateProjectionMatrix();
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the cloud freezes.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const drive = itIsComingDrive(
      this.p9audio.band(-1, 0, 0, 0.1),
      this.p9audio.band(-1, 0, 1, 1),
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
    this.material.uniforms.gIn2.value.set(
      drive.in2.x,
      drive.in2.y,
      drive.in2.z,
      0,
    );
    this.material.uniforms.gTime.value = this.time;
    this.material.uniforms.gTargetSize.value.set(this.size.w, this.size.h);

    // Shader2's Expression: in1.x = random() every frame (sparkle seed).
    this.background.material.uniforms.uSeed.value = Math.random();
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
