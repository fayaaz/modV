// Hollow Cube: the Plane9 "Hollow Cube" preset (author Joakim Dahl,
// CC BY-NC-SA 4.0) ported to a standalone native three.js scene - no XML, no
// P9Scene node graph.
//
// The preset is 15000 instanced 0.01-unit cubes on the six faces of a
// 50x50x50 lattice. Shader1 positions every cube from its instance id (the
// hollow shell mapping), rotates it by a random axis-angle quaternion and
// pulses it outward along the lattice normal; the fragment is lit with
// Plane9's half-Lambert/Blinn-Phong pair. The cube chain is captured into a
// RenderToTexture, bloomed, and Shader2 composites that capture, the bloom and
// the 256x256 fbm capture. There is no PreviousLayer feedback in this port:
// modV has no layer beneath a standalone scene, so the fbm capture stands in
// for the preset's background term. Output alpha follows the cube capture, so
// the empty space stays transparent.
//
// gl_InstanceID is replaced with a float attribute aInstance (0..14999) so the
// shader stays GLSL1 and compiles under WebGL2. The InstancedMesh matrices all
// stay identity on purpose - the shader does the placement, exactly as Boxy's
// tunnel does.
//
// Expression (RandomSeed = 1):
//   perm += min(0.5, band(-1, 0, 0, 0.1) + 0.01) * 40 * dt
//   in1.x = min(1.2, band(-1, 2, 0, 0.1) * 2.2) * 0.6   // explosion
//   in1.y = min(0.55, band(-1, 1, 0, 0.1) * 4 + 0.4)    // colour
//   in1.z = min(1, band(-1, 1, 0, 0.1) * 1.5)           // rotation
//   in2.x = perm
//   in2.y = 1 + min(0.2, band(-1, 1, 0, 0.1))            // size

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { preserveBloomAlpha } from "./Boxy.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { ShaderPass } from "three/examples/jsm/postprocessing/ShaderPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";

// MeshInstancer1 NumInstances / Cube1 Size / the 50x50x50 lattice.
export const INSTANCE_COUNT = 15000;
export const LATTICE_COUNT = 50;
export const CUBE_SIZE = 0.01;

// Screen1: CamPos 0 0 2.2, lookAt 0 0 -1, fov 50, near 0.01, far 100.
export const CAMERA = {
  fov: 50,
  near: 0.01,
  far: 100,
  position: [0, 0, 2.2],
  lookAt: [0, 0, -1],
};

// Bloom1: Radius 6 / Threshold 0 / ThresholdSmoothness 0.5 / Intensity 0.51.
// UnrealBloomPass normalises radius to 0..1, hence ~0.85 for Plane9's Radius 6.
export const BLOOM = { strength: 0.51, radius: 0.85, threshold: 0 };
export const BLOOM_SMOOTHNESS = 0.5;

// Clear1 is (0, 0, 0, 0): Hollow Cube composites transparently over the layers
// beneath it. Shader2 samples the fbm background term, but the final alpha
// follows the cube capture so empty space keeps alpha 0.
export const CLEAR_ALPHA = 0;

// RenderToTexture1: a 256x256 fbm capture.
export const FBM_SIZE = 256;
export const FBM_OCTAVES = 3;

// Guard against tab-suspend catch-up frames turning the pulse into a jump;
// P9 deltatime is clamped the same way by long frames.
const MAX_DT = 0.1;

export function makePerm(seed = 1) {
  return Number.isFinite(seed) ? seed : 1;
}

const bandValue = (value) => Math.max(0, Number.isFinite(value) ? value : 0);

// Pure Expression: the bass band read at three damping strengths. The raw
// lane advances the pulse phase, the damping-2 lane explodes the shell, and
// the damping-1 lane drives colour, rotation and cube size.
export function hollowCubeDrive(permBand, levelBand, explodeBand, dt, perm) {
  const p = bandValue(permBand);
  const l = bandValue(levelBand);
  const e = bandValue(explodeBand);
  const step = Math.min(bandValue(dt), MAX_DT);
  const base = Number.isFinite(perm) ? perm : 1;
  const next = base + Math.min(0.5, p + 0.01) * 40 * step;

  return {
    perm: next,
    in1: {
      x: Math.min(1.2, e * 2.2) * 0.6,
      y: Math.min(0.55, l * 4 + 0.4),
      z: Math.min(1, l * 1.5),
    },
    in2: {
      x: next,
      y: 1 + Math.min(0.2, l),
    },
  };
}

// MeshInstancer's hollow-cube mapping: instance id -> one of the six faces of
// an n x n x n cell grid (the shader's `o` in [0, n]).
export function hollowCubePos(i, count = INSTANCE_COUNT) {
  const n = Math.max(1, Math.round(Math.sqrt(count / 6)));
  const perFace = n * n;
  const face = Math.floor(i / perFace);
  const j = i % perFace;
  const gx = j % n;
  const gy = Math.floor(j / n);
  const o =
    face === 0
      ? [0, gx, gy]
      : face === 1
        ? [n, gx, gy]
        : face === 2
          ? [gx, 0, gy]
          : face === 3
            ? [gx, n, gy]
            : face === 4
              ? [gx, gy, 0]
              : [gx, gy, n];

  return { x: o[0] / n, y: o[1] / n, z: o[2] / n };
}

export function buildHollowCubeVertexShader() {
  return `
precision highp float;
uniform vec4 gIn1;
uniform vec4 gIn2;
attribute float aInstance;
varying vec4 vDiffuse;
varying vec3 vWNormal;
varying vec3 vWPos;
varying vec3 vViewDir;
${COMMON_PREAMBLE}
void main() {
  float t = gIn2.x * 0.1;
  vec4 v = vec4(position, 1.0);
  float d = 0.0001;
  float dt = aInstance / 15000.0;
  float instanceIdFloat = aInstance;
  float objCount = 50.0;

  vec3 o = vec3(0.0);
  vec2 index = vec2(
    mod(instanceIdFloat, objCount),
    mod(instanceIdFloat / objCount, objCount)
  );

  if (instanceIdFloat < objCount * objCount) {
    o = vec3(0.0, index.x, index.y);
  } else if (instanceIdFloat < objCount * objCount * 2.0) {
    o = vec3(objCount, index.x, index.y);
  } else if (instanceIdFloat < objCount * objCount * 3.0) {
    o = vec3(index.x, 0.0, index.y);
  } else if (instanceIdFloat < objCount * objCount * 4.0) {
    o = vec3(index.x, objCount, index.y);
  } else if (instanceIdFloat < objCount * objCount * 5.0) {
    o = vec3(index.x, index.y, 0.0);
  } else {
    o = vec3(index.x, index.y, objCount);
  }

  vec3 p = o / 50.0;
  vec3 up = (p - vec3(0.5, 0.5, 0.5)) * 0.5;
  float pulse = _cubicpulse(mod(p.x + p.y + p.z + t, 1.5), 0.6, 0.4);
  float xx = 1.0;

  if (o.x >= 1.0 && o.x < objCount - 1.0 &&
      o.y >= 1.0 && o.y < objCount - 1.0 &&
      o.z >= 1.0 && o.z < objCount - 1.0) {
    pulse = 0.0;
    xx = 0.0;
  }

  vec4 rotation = vec4(
    normalize(_rand3(vec2(instanceIdFloat))),
    dt + gIn2.x
  );
  vec4 qRotation = _axisAngleToQuaternion(
    rotation.xyz,
    pulse * 6.0 * gIn1.z
  );
  v.xyz = _rotateVector(v.xyz, qRotation);

  vec3 nv;
  nv.x = (_noise(vec3(d * 50.0, 0.0, 0.0)) * 0.5 + 0.5) * sign(up.x);
  nv.y = (_noise(vec3(d * 50.0, 1.0, 0.0)) * 0.5 + 0.5) * sign(up.y);
  nv.z = (_noise(vec3(d * 50.0, 2.0, 0.0)) * 0.5 + 0.5) * sign(up.z);

  v.xyz += nv * pulse * gIn1.x;
  v.xyz += up * gIn2.y * xx;

  gl_Position = projectionMatrix * modelViewMatrix * v;

  vDiffuse = vec4(_hsv2rgb(0.0, 0.9, 0.8), 1.0) *
    (0.1 + 0.9 * _rand(vec2(instanceIdFloat)));
  vWNormal = normalize(mat3(modelMatrix) * _rotateVector(normal, qRotation));
  vWPos = (modelMatrix * v).xyz;
  vViewDir = cameraPosition - vWPos;
}
`;
}

export function buildHollowCubeFragmentShader() {
  return `
precision highp float;
uniform vec4 gIn1;
varying vec4 vDiffuse;
varying vec3 vWNormal;
varying vec3 vWPos;
varying vec3 vViewDir;
${COMMON_PREAMBLE}
vec3 _lightHalfLambert(vec3 normal, vec3 lightDir, vec3 diffuseCol) {
  return (dot(lightDir, normal) * 0.5 + 0.5) * diffuseCol;
}
vec3 _lightBlinnPhongHalfLambert(
  vec3 normal,
  vec3 lightDir,
  vec3 viewDir,
  vec3 diffuseCol,
  vec3 specularCol,
  float specularHardness
) {
  vec3 halfVec = normalize(lightDir + viewDir);
  float specular =
    pow(saturate(dot(normal, halfVec)), specularHardness) *
    ((specularHardness + 4.0) / 8.0);
  return _lightHalfLambert(normal, lightDir, diffuseCol) + specularCol * specular;
}

void main() {
  vec3 normal = normalize(vWNormal);
  vec3 viewDir = normalize(vViewDir);
  float specularHardness = 10.0;

  vec3 c = vDiffuse.rgb * gIn1.y;

  vec3 lightDir = normalize(vec3(0.0, 1.0, 0.0) - vWPos);
  c += _lightBlinnPhongHalfLambert(
    normal,
    lightDir,
    viewDir,
    vec3(0.5, 0.8, 1.0),
    vec3(0.5, 0.8, 1.0) * 1.15,
    specularHardness
  ) * 0.04;

  lightDir = normalize(vec3(0.0, -1.0, 0.0) - vWPos);
  c += _lightBlinnPhongHalfLambert(
    normal,
    lightDir,
    viewDir,
    vec3(0.8, 0.0, 0.0),
    vec3(1.0, 1.0, 1.0) * 0.15,
    specularHardness
  ) * 0.025;

  gl_FragColor = vec4(max(vec3(0.0), c), 1.0);
}
`;
}

export function buildHollowCubeFbmFragment() {
  return `
precision highp float;
uniform float gTime;
varying vec2 vUv;
${COMMON_PREAMBLE}
void main() {
  vec3 v = vec3(_fbm(vec3(vUv.xy * vec2(10.0), gTime * 0.1), 3.0));
  gl_FragColor = vec4(v, 1.0);
}
`;
}

export function buildHollowCubeShader2Fragment() {
  return `
precision highp float;
uniform sampler2D tCube;
uniform sampler2D tBloom;
uniform sampler2D tFbm;
varying vec2 vUv;
${COMMON_PREAMBLE}
void main() {
  vec4 c1 = texture2D(tBloom, vUv);
  vec4 c2 = texture2D(tCube, vUv);
  vec4 bg = texture2D(tFbm, vUv);

  vec3 col = mix(bg.rgb, _tonemapACES(c2.rgb), c2.a) +
    _tonemapACES(c1.rgb * c1.rgb);

  gl_FragColor = vec4(col, c2.a);
}
`;
}

const FULLSCREEN_VERTEX = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

const COPY_FRAGMENT = `
precision highp float;
uniform sampler2D source;
varying vec2 vUv;
void main() {
  gl_FragColor = texture2D(source, vUv);
}
`;

// The Cube node: BoxGeometry already has the preset's per-face uvs and
// normals. The instance id attribute is what the shader positions each cube
// from; the InstancedMesh matrices stay identity.
export function buildHollowCubeGeometry() {
  const geometry = new THREE.BoxGeometry(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE);
  const ids = new Float32Array(INSTANCE_COUNT);

  for (let i = 0; i < INSTANCE_COUNT; i++) ids[i] = i;

  geometry.setAttribute(
    "aInstance",
    new THREE.InstancedBufferAttribute(ids, 1),
  );

  return geometry;
}

export class HollowCube extends BaseScene {
  _init() {
    this.label = "Hollow Cube";
    this.role = "standalone";

    this.spinScale = 1;
    this.explodeScale = 1;
    this.colorScale = 1;
    this.bloomScale = 1;

    this.material = new THREE.ShaderMaterial({
      vertexShader: buildHollowCubeVertexShader(),
      fragmentShader: buildHollowCubeFragmentShader(),
      uniforms: {
        gIn1: { value: new THREE.Vector4(0, 1.1, 0, 0) },
        gIn2: { value: new THREE.Vector4(0, 1, 0, 0) },
      },
      // Shader1: DoubleSide, opaque, depth-tested.
      depthTest: true,
      depthWrite: true,
      side: THREE.DoubleSide,
    });

    this.mesh = new THREE.InstancedMesh(
      buildHollowCubeGeometry(),
      this.material,
      INSTANCE_COUNT,
    );
    this.mesh.frustumCulled = false;
    const identity = new THREE.Matrix4();

    for (let i = 0; i < INSTANCE_COUNT; i++) {
      this.mesh.setMatrixAt(i, identity);
    }

    this.mesh.instanceMatrix.needsUpdate = true;
    this.group.add(this.mesh);

    this.p9audio = new P9Audio(this.audio);
    this.perm = makePerm(1);
    this.size = { w: 2, h: 2 };

    if (this.renderer) {
      const s = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: s.x, h: s.y };
    }

    this._applyCamera(this.size.w, this.size.h);

    if (this.renderer) this._initPipeline();
  }

  _initPipeline() {
    const renderer = this.renderer;

    // Shader2 writes the preset's values directly (it already applies ACES),
    // so leave three's tone mapping off and let OutputPass do linear -> sRGB.
    renderer.toneMapping = THREE.NoToneMapping;
    renderer.setClearColor(0x000000, CLEAR_ALPHA);

    const size = renderer.getSize(new THREE.Vector2());

    this._cubeTarget = new THREE.WebGLRenderTarget(
      Math.max(2, size.x),
      Math.max(2, size.y),
      { type: THREE.HalfFloatType },
    );
    this._fbmTarget = new THREE.WebGLRenderTarget(FBM_SIZE, FBM_SIZE, {
      type: THREE.UnsignedByteType,
    });

    this._fbmMaterial = new THREE.ShaderMaterial({
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: buildHollowCubeFbmFragment(),
      uniforms: {
        gTime: { value: 0 },
      },
      depthTest: false,
      depthWrite: false,
    });
    this._fbmQuad = new THREE.Mesh(
      new THREE.PlaneGeometry(2, 2),
      this._fbmMaterial,
    );
    this._fbmQuad.frustumCulled = false;
    this._fbmScene = new THREE.Scene();
    this._fbmScene.add(this._fbmQuad);
    this._ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);

    // Bloom reads a copy of the cube capture, so its bloom-only texture stays
    // separate from c2 for Shader2's `ACES(c1*c1)` term.
    this._copyPass = new ShaderPass({
      uniforms: { source: { value: null } },
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: COPY_FRAGMENT,
    });
    this._bloomPass = preserveBloomAlpha(
      new UnrealBloomPass(size, BLOOM.strength, BLOOM.radius, BLOOM.threshold),
    );
    this._bloomComposer = new EffectComposer(renderer);
    this._bloomComposer.addPass(this._copyPass);
    this._bloomComposer.addPass(this._bloomPass);

    this._finalPass = new ShaderPass({
      uniforms: {
        tCube: { value: null },
        tBloom: { value: null },
        tFbm: { value: null },
      },
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: buildHollowCubeShader2Fragment(),
    });
    this.composer = new EffectComposer(renderer);
    this.composer.addPass(this._finalPass);
    this.composer.addPass(new OutputPass());
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
    // audio or every band reads 0 and the shell stops reacting.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const drive = hollowCubeDrive(
      this.p9audio.band(-1, 0, 0, 0.1),
      this.p9audio.band(-1, 1, 0, 0.1),
      this.p9audio.band(-1, 2, 0, 0.1),
      dt,
      this.perm,
    );
    this.perm = drive.perm;
    this.material.uniforms.gIn1.value.set(
      drive.in1.x * this.explodeScale,
      drive.in1.y * this.colorScale,
      drive.in1.z * this.spinScale,
      0,
    );
    this.material.uniforms.gIn2.value.set(drive.in2.x, drive.in2.y, 0, 0);

    if (this._fbmMaterial) {
      this._fbmMaterial.uniforms.gTime.value = this.time;
    }

    if (this._bloomPass) {
      this._bloomPass.strength = BLOOM.strength * this.bloomScale;
    }
  }

  _syncSize() {
    const w = this.renderer.domElement.width;
    const h = this.renderer.domElement.height;

    if (w === this._cw && h === this._ch) return;

    this._cw = w;
    this._ch = h;
    this._cubeTarget.setSize(w, h);
    this._bloomComposer.setPixelRatio(this.renderer.getPixelRatio());
    this._bloomComposer.setSize(
      w / this.renderer.getPixelRatio(),
      h / this.renderer.getPixelRatio(),
    );
    this.composer.setPixelRatio(this.renderer.getPixelRatio());
    this.composer.setSize(
      w / this.renderer.getPixelRatio(),
      h / this.renderer.getPixelRatio(),
    );
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

    this._syncSize();

    const renderer = this.renderer;
    const previousTarget = renderer.getRenderTarget();

    renderer.setRenderTarget(this._cubeTarget);
    renderer.setClearColor(0x000000, CLEAR_ALPHA);
    renderer.clear(true, true, false);
    renderer.render(this.group, this.camera);

    renderer.setRenderTarget(this._fbmTarget);
    renderer.setClearColor(0x000000, 0);
    renderer.clear(true, true, false);
    renderer.render(this._fbmScene, this._ortho);

    renderer.setRenderTarget(previousTarget);

    this._copyPass.uniforms.source.value = this._cubeTarget.texture;
    this._bloomComposer.render();

    this._finalPass.uniforms.tCube.value = this._cubeTarget.texture;
    this._finalPass.uniforms.tBloom.value =
      this._bloomPass.renderTargetsHorizontal[0].texture;
    this._finalPass.uniforms.tFbm.value = this._fbmTarget.texture;

    this.composer.renderToScreen = renderToScreen;
    this.composer.render();
  }

  resize(w, h) {
    this.size = { w, h };
    this._applyCamera(w, h);

    if (this.renderer) {
      this._cw = 0;
      this._ch = 0;
      this._syncSize();
    }
  }
}
