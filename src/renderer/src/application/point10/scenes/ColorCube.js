// Color Cube: the Plane9 "Color Cube" preset (author Joakim Dahl,
// CC BY-NC-SA 3.0) ported to a standalone native three.js scene - no XML, no
// P9Scene node graph.
//
// The preset clones an 8x8x8 (512) grid of 0.05 cubes with CloneExpression1.
// Each clone is positioned from its 1-based grid coordinates and an audio
// `dist`, coloured by the same coordinates (r = x/8, g = y/8, b = 1-z/8), and
// lit by Shader1's directional + point lights. Expression1 advances a perm
// phase from the beat envelope and feeds CloneExpression1.In1, so every cube
// wobbles on the sine terms as the grid spreads. Rotator1 spins the whole
// RenderObject at 1.5/1/0.5 deg/s.
//
// Render chain: RenderObject1 -> RenderToTexture1 -> Bloom1 -> Shader2, where
// Shader2 is `mix(bg, capture, capture.a) + bloom`. RenderToTexture1 has a
// Render input (RenderObject1), so P9Scene captures the cubes rather than
// running the node's own fbm shader. Width/Height are 0 (= 100% of the render
// target), so the capture is full-resolution; WidthCustom 256 is ignored.
// PreviousLayer is transparent black in standalone playback (there is no layer
// beneath), which leaves the transparent background modV composites against.
//
// gl_InstanceID is replaced by three's InstancedMesh instanceMatrix. The
// preset's gKeepInst per-instance colour becomes the aColor InstancedBuffer-
// Attribute. The point light is inlined because COMMON_PREAMBLE only ships the
// directional/specular helpers.

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { preserveBloomAlpha } from "./Boxy.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";

// CloneExpression1 NumObjectsX/Y/Z and Cube1 Size.
export const GRID_SIZE = 8;
export const INSTANCE_COUNT = GRID_SIZE * GRID_SIZE * GRID_SIZE;
export const CUBE_SIZE = 0.05;

// Screen1: CamPos (0,0,-1.17), lookAt (0,0,1), fov 70, near 0.01, far 100.
export const CAMERA = {
  fov: 70,
  near: 0.01,
  far: 100,
  position: [0, 0, -1.17],
  lookAt: [0, 0, 1],
};

// Rotator1 RotSpeedX/Y/Z, in degrees per second.
export const ROTATOR = { x: 1.5, y: 1, z: 0.5 };

// Bloom1: Radius 5 / Threshold 0 / Intensity 0.5. UnrealBloomPass normalises
// radius to 0..1, hence ~0.85 (same mapping as Boxy).
export const BLOOM = { strength: 0.5, radius: 0.85, threshold: 0 };

// Clear1/Out are (0, 0, 0, 0): the scene composites transparently over the
// layers behind it instead of painting a black backdrop.
export const CLEAR_ALPHA = 0;

// Shader1's In1 is (8.781, 14.945, -3.827); Expression1 overwrites In1.x with
// the turbulence discard threshold, leaving the light position's y/z.
export const LIGHT_POSITION = [0, 14.945, -3.827];

// Shader1's Expression: in1.x = min(0.15, band(-1,2,0,0.05) * 10).
export const DISCARD_MAX = 0.15;

// Expression1 caps elapsed time at 0.05s, like the Plane9 expression runtime.
export const MAX_DT = 0.05;

// CloneExpression1's seeded axis count and its two sine scales.
export const GRID_STEP = 0.05;
export const SINE_SCALE = 0.025;

const DEG_TO_RAD = Math.PI / 180;

const finiteOr = (value, fallback) =>
  typeof value === "number" && Number.isFinite(value) ? value : fallback;

const nonNegative = (value) => Math.max(0, finiteOr(value, 0));

// Pure CloneExpression1 position for one 1-based grid coordinate. `dist` is the
// audio spread, `perm` the Expression1 phase feeding In1.
function writePosition(out, x, y, z, dist, perm) {
  const spread = 1 + dist;
  const wobble = dist * SINE_SCALE;

  out.x = x * spread * GRID_STEP + Math.sin(x - y - z + perm) * wobble;
  out.y = y * spread * GRID_STEP + Math.sin(x + y + z + perm) * wobble;
  out.z =
    z * spread * GRID_STEP +
    Math.sin((x + 1) * (y + 1) * (z + 1) + perm) * wobble;

  return out;
}

export function clonePosition(x, y, z, dist, perm) {
  return writePosition(
    { x: 0, y: 0, z: 0 },
    x,
    y,
    z,
    finiteOr(dist, 0),
    finiteOr(perm, 0),
  );
}

// CloneExpression1 colour: r = x/nx, g = y/ny, b = 1-z/nz.
export function cloneColor(
  x,
  y,
  z,
  nx = GRID_SIZE,
  ny = GRID_SIZE,
  nz = GRID_SIZE,
) {
  return {
    r: x / nx,
    g: y / ny,
    b: 1 - z / nz,
  };
}

/**
 * Builds the 512 clone positions and colours. Coordinates are 1-based, matching
 * P9Nodes' CloneExpression. Center=true then subtracts the bounding-box
 * midpoint, so the grid sits on the origin. Positions/colours can be supplied
 * so the render loop reuses its typed arrays.
 */
export function computeCloneGrid(
  perm,
  dist,
  positions = new Float32Array(INSTANCE_COUNT * 3),
  colors = new Float32Array(INSTANCE_COUNT * 3),
  nx = GRID_SIZE,
  ny = GRID_SIZE,
  nz = GRID_SIZE,
) {
  const count = nx * ny * nz;
  const p = finiteOr(perm, 0);
  const d = finiteOr(dist, 0);
  const scratch = { x: 0, y: 0, z: 0 };
  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;

  for (let i = 0; i < count; i++) {
    const z = Math.floor(i / (nx * ny)) + 1;
    const row = i % (nx * ny);
    const x = (row % nx) + 1;
    const y = Math.floor(row / nx) + 1;

    writePosition(scratch, x, y, z, d, p);

    const o = i * 3;
    positions[o] = scratch.x;
    positions[o + 1] = scratch.y;
    positions[o + 2] = scratch.z;

    colors[o] = x / nx;
    colors[o + 1] = y / ny;
    colors[o + 2] = 1 - z / nz;

    if (scratch.x < minX) minX = scratch.x;
    if (scratch.y < minY) minY = scratch.y;
    if (scratch.z < minZ) minZ = scratch.z;
    if (scratch.x > maxX) maxX = scratch.x;
    if (scratch.y > maxY) maxY = scratch.y;
    if (scratch.z > maxZ) maxZ = scratch.z;
  }

  const cx = (minX + maxX) * 0.5;
  const cy = (minY + maxY) * 0.5;
  const cz = (minZ + maxZ) * 0.5;

  for (let i = 0; i < count; i++) {
    const o = i * 3;
    positions[o] -= cx;
    positions[o + 1] -= cy;
    positions[o + 2] -= cz;
  }

  return {
    positions,
    colors,
    count,
    nx,
    ny,
    nz,
    center: { x: cx, y: cy, z: cz },
  };
}

export function makePerm(seed = 1) {
  return finiteOr(seed, 1);
}

/**
 * Derived from Expression1 (perm/beat), CloneExpression1 (dist) and Shader1's
 * Expression (turbulence discard). `turbulenceScale` scales the beat-driven
 * perm advance and the noise discard threshold.
 */
export function colorCubeDrive(
  bass,
  bassSmooth,
  beat,
  dt,
  perm,
  turbulenceScale = 1,
) {
  const b = nonNegative(bass);
  const bs = nonNegative(bassSmooth);
  const be = Math.min(1, nonNegative(beat));
  const step = Math.min(nonNegative(dt), MAX_DT);
  const turbulence = finiteOr(turbulenceScale, 1);
  const scale = Math.max(0, turbulence);
  const base = finiteOr(perm, 1);
  const next = base + (be * 50 * scale + 0.01) * step;
  const dist = (0.3 + b) * bs * 20;
  const discard = Math.min(DISCARD_MAX, bs * 10) * scale;

  return {
    perm: next,
    dist,
    discard,
    in1: {
      x: discard,
      y: LIGHT_POSITION[1],
      z: LIGHT_POSITION[2],
    },
  };
}

export function buildColorCubeVertexShader() {
  return `
precision highp float;
uniform vec4 gIn1;
attribute vec3 aColor;
varying vec4 vDiffuse;
varying vec3 vWNormal;
varying vec3 vViewDir;
varying vec3 vWorldPos;
${COMMON_PREAMBLE}
void main() {
  vec4 local = vec4(position, 1.0);
  vec4 world = modelMatrix * instanceMatrix * local;

  vDiffuse = vec4(aColor, 1.0);
  vWNormal = normalize(mat3(modelMatrix) * mat3(instanceMatrix) * normal);
  vViewDir = normalize(cameraPosition - world.xyz);
  vWorldPos = world.xyz;

  gl_Position = projectionMatrix * modelViewMatrix * instanceMatrix * local;
}
`;
}

export function buildColorCubeFragmentShader() {
  return `
precision highp float;
uniform vec4 gIn1;
uniform vec4 gIn3;
uniform float uBrightness;
varying vec4 vDiffuse;
varying vec3 vWNormal;
varying vec3 vViewDir;
varying vec3 vWorldPos;
${COMMON_PREAMBLE}
void main()
{
  vec3 normal = normalize(vWNormal);
  vec3 viewdir = normalize(vViewDir);
  vec3 lightPosition = gIn1.xyz;
  vec3 albedoColor = vDiffuse.rgb;
  float roughness = 0.4;
  float metallic = 0.0;

  if (_turbulencefast(vWorldPos * 2.0, 5.0) < gIn1.x)
    discard;

  vec3 realAlbedo = albedoColor - albedoColor * vec3(metallic);
  vec3 realSpecularColor = mix(vec3(0.04), albedoColor, metallic);

  vec3 light = _lightDirectional(
    realAlbedo,
    realSpecularColor,
    normal,
    roughness,
    vec3(1.0) * 2.0 * uBrightness,
    normalize(lightPosition - vWorldPos),
    viewdir
  );

  // Point light at the origin: _lightPoint from Plane9's shader library (the
  // shared preamble only ships the directional helper), colour (1,0.5,0.3)*50,
  // radius 100.
  vec3 pixelToLight = -vWorldPos;
  float lightDist = length(pixelToLight);
  vec3 pointDir = pixelToLight / lightDist;
  float attenuation = pow(saturate(1.0 - pow(lightDist / 100.0, 4.0)), 2.0) / ((lightDist * lightDist) + 1.0);
  light += _lightDirectional(
    realAlbedo,
    realSpecularColor,
    normal,
    roughness,
    vec3(1.0, 0.5, 0.3) * 50.0 * uBrightness,
    pointDir,
    viewdir
  ) * attenuation;

  vec4 c = vec4(light + realAlbedo * gIn3.x, 1.0);
  gl_FragColor = vec4(c.rgb, 1.0);
}
`;
}

export class ColorCube extends BaseScene {
  _init() {
    this.label = "Color Cube";
    this.role = "standalone";

    this.material = new THREE.ShaderMaterial({
      vertexShader: buildColorCubeVertexShader(),
      fragmentShader: buildColorCubeFragmentShader(),
      uniforms: {
        gIn1: {
          value: new THREE.Vector4(0, LIGHT_POSITION[1], LIGHT_POSITION[2], 0),
        },
        gIn3: { value: new THREE.Vector4(0, 10, 0, 0) },
        uBrightness: { value: 1 },
      },
      depthTest: true,
      depthWrite: true,
      side: THREE.FrontSide,
    });

    this.geometry = new THREE.BoxGeometry(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE);
    this._positions = new Float32Array(INSTANCE_COUNT * 3);
    this._colors = new Float32Array(INSTANCE_COUNT * 3);
    this._colorAttr = new THREE.InstancedBufferAttribute(this._colors, 3);
    this.geometry.setAttribute("aColor", this._colorAttr);
    this._dummy = new THREE.Object3D();

    this.mesh = new THREE.InstancedMesh(
      this.geometry,
      this.material,
      INSTANCE_COUNT,
    );
    this.mesh.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    this.mesh.frustumCulled = false;
    this.group.add(this.mesh);

    this.p9audio = new P9Audio(this.audio);
    this.perm = makePerm(1);
    this.spinTime = 0;
    this.spinScale = 1;
    this.turbulenceScale = 1;
    this.bloomScale = 1;
    this.brightnessScale = 1;
    this.size = { w: 2, h: 2 };

    if (this.renderer) {
      this.renderer.setClearColor(0x000000, CLEAR_ALPHA);
      // Shader2 only composes the capture + bloom; no filmic/ACES curve.
      this.renderer.toneMapping = THREE.NoToneMapping;

      const size = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: size.x, h: size.y };

      this.composer = new EffectComposer(this.renderer);
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.addPass(
        new RenderPass(this.group, this.camera, null, null, CLEAR_ALPHA),
      );
      this._bloomPass = preserveBloomAlpha(
        new UnrealBloomPass(
          size,
          BLOOM.strength,
          BLOOM.radius,
          BLOOM.threshold,
        ),
      );
      this.composer.addPass(this._bloomPass);
      this.composer.addPass(new OutputPass());
    }

    this._writeInstances(this.perm, 0);
    this._applyCamera(this.size.w, this.size.h);
  }

  _writeInstances(perm, dist) {
    const grid = computeCloneGrid(perm, dist, this._positions, this._colors);

    for (let i = 0; i < grid.count; i++) {
      const o = i * 3;
      this._dummy.position.set(
        grid.positions[o],
        grid.positions[o + 1],
        grid.positions[o + 2],
      );
      this._dummy.rotation.set(0, 0, 0);
      this._dummy.scale.set(1, 1, 1);
      this._dummy.updateMatrix();
      this.mesh.setMatrixAt(i, this._dummy.matrix);
    }

    this.mesh.instanceMatrix.needsUpdate = true;
    this._colorAttr.needsUpdate = true;
    return grid;
  }

  _applyCamera(w, h) {
    const cam = this.camera;
    if (!cam) return;
    cam.fov = CAMERA.fov;
    cam.near = CAMERA.near;
    cam.far = CAMERA.far;
    cam.aspect = w / Math.max(1, h);
    cam.position.set(
      CAMERA.position[0],
      CAMERA.position[1],
      CAMERA.position[2],
    );
    cam.lookAt(CAMERA.lookAt[0], CAMERA.lookAt[1], CAMERA.lookAt[2]);
    cam.updateMatrixWorld();
    cam.matrixWorldInverse.copy(cam.matrixWorld).invert();
    cam.updateProjectionMatrix();
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the grid never responds.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const drive = colorCubeDrive(
      this.p9audio.band(-1, 1, 0, 0),
      this.p9audio.band(-1, 2, 0, 0.05),
      this.p9audio.beat(0.05),
      dt,
      this.perm,
      this.turbulenceScale,
    );
    this.perm = drive.perm;

    this.material.uniforms.gIn1.value.set(
      drive.in1.x,
      drive.in1.y,
      drive.in1.z,
      0,
    );
    this.material.uniforms.uBrightness.value = finiteOr(
      this.brightnessScale,
      1,
    );

    const step = nonNegative(dt);
    this.spinTime += step * finiteOr(this.spinScale, 1);

    this.mesh.rotation.set(
      this.spinTime * ROTATOR.x * DEG_TO_RAD,
      this.spinTime * ROTATOR.y * DEG_TO_RAD,
      this.spinTime * ROTATOR.z * DEG_TO_RAD,
    );

    this._writeInstances(this.perm, drive.dist);

    if (this._bloomPass) {
      this._bloomPass.strength = BLOOM.strength * finiteOr(this.bloomScale, 1);
    }
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
