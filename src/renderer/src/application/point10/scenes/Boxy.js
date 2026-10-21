// Boxy: the Plane9 "Boxy" preset (author Joakim Dahl, CC BY-NC-SA 4.0) ported
// to a standalone native three.js scene - no XML, no P9Scene node graph.
//
// The preset is 1000 instanced unit cubes scattered in random directions down
// a -Z tunnel, textured with the metallic tinore material and a Stormy Days
// cubemap reflection. Shader1 positions every cube from its instance id;
// RenderToTexture1 captures that chain (it has a Render input, so P9Scene
// renders the cubes rather than the node's own noise shader), Bloom1 blurs
// the capture, and Shader2 adds bloom + cubes for the screen. PreviousLayer
// is not part of this preset, so there is no frame feedback.
//
// gl_InstanceID is replaced with a float attribute aInstance (0..999) so the
// shader stays GLSL1 and compiles under WebGL2. BoxGeometry has no tangents,
// so the fragment derives a tangent frame from the world normal + the normal
// map; that keeps the tinore normal map readable without changing the math.
//
// glTexture1Material: the preset packs R=metallic, G=roughness, B=AO into one
// sibling map. Rather than build a canvas-packed DataTexture in the worker we
// sample the three sibling maps separately and reconstruct
// `vec3(metallic.r, roughness.g, ao.r)` in the shader - identical values,
// simpler and worker-safe.
//
// Expression1 (RandomSeed = 1):
//   in1.x = band(0, 1, 0, 0)  * 20   (unused by the shader, kept for parity)
//   in1.y = band(1, 1, 0, 0)  * 20   (glow boost)
//   perm += min(2, band(-1, 1, 0, 0.04) * 20 + 0.2) * dt
//   in1.z = perm + permrand * 10     (tunnel scroll)
//   in2   = (0.3, 0, 0)              (roughness floor)
// permrand is a stable, seeded per-scene random.

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { seededRand } from "../shared/random.js";
import { loadTexture, loadImageBitmap } from "../shared/textures.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";
import diffuseUrl from "../textures/tinore_diffuse.jpg?url";
import normalUrl from "../textures/tinore_normal.jpg?url";
import roughnessUrl from "../textures/tinore_roughness.jpg?url";
import metallicUrl from "../textures/tinore_metallic.png?url";
import aoUrl from "../textures/tinore_ao.jpg?url";
import stormyPosX from "../textures/stormy/posx.jpg?url";
import stormyNegX from "../textures/stormy/negx.jpg?url";
import stormyPosY from "../textures/stormy/posy.jpg?url";
import stormyNegY from "../textures/stormy/negy.jpg?url";
import stormyPosZ from "../textures/stormy/posz.jpg?url";
import stormyNegZ from "../textures/stormy/negz.jpg?url";

// MeshInstancer1 NumInstances, Cube1 Size (1x1x1, 1x1x1 tessellation).
export const INSTANCE_COUNT = 1000;
export const CUBE_SIZE = 1;

// Screen1: CamPos 0 0 0, lookAt 0 0 -1, fov 70, near 0.01, far 100.
export const CAMERA = { fov: 70, near: 0.01, far: 100 };

// The preset's Bloom1 port: Radius 5 / Threshold 0 / Smoothness 0.5 /
// Intensity 1. UnrealBloomPass normalises radius to 0..1, hence ~0.85.
export const BLOOM = { strength: 1, radius: 0.85, threshold: 0 };

// Expression1's per-scene `permrand`, fixed so the tunnel offset is stable
// across reloads instead of jumping with Math.random().
export const PERMRAND = seededRand(23, 8);

// Expression1's In2 = (0.3, 0, 0): the shader's roughness floor.
export const ROUGHNESS_FLOOR = 0.3;

// Guard against tab-suspend catch-up frames turning the scroll into a jump;
// P9 deltatime is clamped the same way by long frames.
const MAX_DT = 0.1;

// The six Stormy Days faces in THREE.CubeTexture order (+X, -X, +Y, -Y, +Z, -Z).
const STORMY_FACES = [
  stormyPosX,
  stormyNegX,
  stormyPosY,
  stormyNegY,
  stormyPosZ,
  stormyNegZ,
];

// Pure Expression1: bass and mid drive in1.x/in1.y, bass advances the tunnel
// scroll (capped at 2 units/s) and in1.z carries the seeded phase.
export function boxyDrive(bass, mid, dt, perm) {
  const b = Math.max(0, Number.isFinite(bass) ? bass : 0);
  const m = Math.max(0, Number.isFinite(mid) ? mid : 0);
  const step = Math.max(0, Number.isFinite(dt) ? Math.min(dt, MAX_DT) : 0);
  const base = Number.isFinite(perm) ? perm : 1;
  const next = base + Math.min(2, b * 20 + 0.2) * step;

  return {
    perm: next,
    in1: {
      x: b * 20,
      y: m * 20,
      z: next + PERMRAND * 10,
    },
    in2: {
      x: ROUGHNESS_FLOOR,
      y: 0,
      z: 0,
    },
  };
}

export function makePerm(seed = 1) {
  return Number.isFinite(seed) ? seed : 1;
}

export function buildBoxyVertexShader() {
  return `
precision highp float;
uniform vec4 gIn1;
attribute float aInstance;
varying vec2 vTex;
varying vec3 vWNormal;
varying vec3 vWPos;
varying vec3 vViewDir;
varying vec3 vPos;
varying vec2 vRnd;
${COMMON_PREAMBLE}
void main() {
  vec4 v = vec4(position, 1.0);
  float instanceIdFloat = aInstance;
  float d = aInstance / 1000.0;

  vec3 rnd = _rand3(d);
  vec3 rnd2 = _rand3(aInstance * 0.825931);
  vec3 wp = vec3(0.0);

  // Random direction
  vec2 r = normalize(rnd.xy * 2.0 - 1.0) * (1.0 + rnd2.x * 5.0);
  wp.xy += r;
  wp.z -= mod(d * 20.0 - gIn1.z, 20.0);

  v.xyz *= vec3(0.1) + rnd.xyz * 0.2;
  v.xyz = v.xyz + wp;
  gl_Position = projectionMatrix * modelViewMatrix * v;

  vTex = uv;
  vWNormal = normalize(mat3(modelMatrix) * normal);
  vWPos = (modelMatrix * v).xyz;
  vViewDir = cameraPosition - vWPos;
  vPos = v.xyz;
  vRnd = rnd2.yz;
}
`;
}

export function buildBoxyFragmentShader() {
  return `
precision highp float;
uniform vec4 gIn1;
uniform vec4 gIn2;
uniform sampler2D gTexture1;
uniform sampler2D gTexture1Normal;
uniform sampler2D gTexture1Metallic;
uniform sampler2D gTexture1Roughness;
uniform sampler2D gTexture1AO;
uniform samplerCube gTexture2;
uniform sampler2D uEnvPanorama;
uniform float uEnvPanoramaMode;
varying vec2 vTex;
varying vec3 vWNormal;
varying vec3 vWPos;
varying vec3 vViewDir;
varying vec3 vPos;
varying vec2 vRnd;
${COMMON_PREAMBLE}
vec3 simpleFog(vec3 orgCol, float distToCam, float density)
{
  float fogAmount = 1.0 - exp(-distToCam * density);
  return mix(orgCol, vec3(0.0), fogAmount);
}

void main()
{
  vec2 p = vTex * 2.0 - 1.0;
  vec3 viewdir = normalize(vViewDir);
  vec3 normalSurf = normalize(vWNormal);
  // BoxGeometry has no tangents, so build a stable tangent frame around the
  // interpolated world normal (the axis pick avoids the degenerate Y case).
  vec3 tangent = normalize(cross(normalSurf, abs(normalSurf.y) < 0.99 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0)));
  vec3 bitangent = cross(normalSurf, tangent);

  vec3 albedo = texture2D(gTexture1, vTex).rgb;
  vec3 normalTex = texture2D(gTexture1Normal, vTex).rgb * 2.0 - 1.0;
  // Packed material: R = metallic, G = roughness, B = AO.
  vec3 material = vec3(
    texture2D(gTexture1Metallic, vTex).r,
    texture2D(gTexture1Roughness, vTex).g,
    texture2D(gTexture1AO, vTex).r
  );
  float roughness = max(gIn2.x, material.g);
  vec3 normal = normalize(normalTex.x * tangent + normalTex.y * bitangent + normalTex.z * normalSurf);
  vec3 tintCol = _palette(0.75 + vRnd.r * 0.25, vec3(0.5), vec3(0.1), vec3(1.0, 1.0, 1.0), vec3(0.2, 0.1, 0.7));
  albedo *= tintCol;
  float glowingObject = smoothstep(0.91, 0.911, vRnd.r);

  vec3 realAlbedo = mix(albedo * material.bbb, vec3(0.0), material.r);
  vec3 realSpecular = mix(vec3(0.04), albedo, material.r);

  vec3 c = _lightDirectional(realAlbedo, realSpecular, normal, roughness, vec3(1.0) * 10.0, vec3(0.0, 0.0, -1.0), viewdir);

  // Environment (Stormy Days cubemap; posz panorama fallback when the worker
  // cannot upload ImageBitmap cube faces).
  vec3 envDir = reflect(-viewdir, normal);
  float envLod = roughness * roughness * 8.0;
  vec3 envColor = textureLod(gTexture2, envDir, envLod).rgb;
  if (uEnvPanoramaMode > 0.5) {
    envColor = _texturePanoramicLod(uEnvPanorama, envDir, envLod).rgb;
  }
  vec3 envFresnel = _fresnelRoughness(realSpecular, roughness, normal, viewdir);
  c += envColor * envFresnel * vec3(0.5);

  if (vRnd.r > 0.9)
  {
    c.rgb += _palette(vRnd.g * 0.25, vec3(0.5), vec3(0.5), vec3(1.0, 1.0, 1.0), vec3(0.2, 0.1, 0.7)) * glowingObject * (1.0 + gIn1.y);
  }
  c.rgb = simpleFog(c.rgb, length(vViewDir), 0.3);
  c.rgb *= smoothstep(0.0, 1.0, -vWPos.z);
  gl_FragColor = vec4(c, 1.0);
}
`;
}

// The Cube node: THREE's BoxGeometry already has the preset's per-face uvs
// and normals. InstancedBufferGeometry shares those attributes and adds the
// per-instance id the shader positions each cube from.
export function buildBoxyGeometry() {
  const box = new THREE.BoxGeometry(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE);
  const geometry = new THREE.InstancedBufferGeometry();

  geometry.index = box.index;
  geometry.setAttribute("position", box.attributes.position);
  geometry.setAttribute("normal", box.attributes.normal);
  geometry.setAttribute("uv", box.attributes.uv);

  const ids = new Float32Array(INSTANCE_COUNT);
  for (let i = 0; i < INSTANCE_COUNT; i++) ids[i] = i;
  geometry.setAttribute(
    "aInstance",
    new THREE.InstancedBufferAttribute(ids, 1),
  );
  geometry.instanceCount = INSTANCE_COUNT;
  return geometry;
}

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

// A 1x1 black cube keeps the samplerCube bound (and the shader complete) until
// the six Stormy Days faces finish decoding.
function placeholderCubeTexture() {
  if (typeof OffscreenCanvas === "undefined") return null;
  try {
    const canvas = new OffscreenCanvas(1, 1);
    const context = canvas.getContext("2d");
    context.fillStyle = "#000";
    context.fillRect(0, 0, 1, 1);
    const faces = [];
    for (let i = 0; i < 6; i++) {
      faces.push(canvas.transferToImageBitmap());
    }
    const cube = new THREE.CubeTexture(faces);
    cube.needsUpdate = true;
    return cube;
  } catch (error) {
    console.warn("Boxy placeholder cubemap failed:", error);
    return null;
  }
}

export class Boxy extends BaseScene {
  _init() {
    this.label = "Boxy";
    this.role = "standalone";

    const placeholderCube = placeholderCubeTexture();
    this.material = new THREE.ShaderMaterial({
      vertexShader: buildBoxyVertexShader(),
      fragmentShader: buildBoxyFragmentShader(),
      uniforms: {
        gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
        gIn2: { value: new THREE.Vector4(ROUGHNESS_FLOOR, 0, 0, 0) },
        gTexture1: { value: placeholderTexture([128, 128, 128, 255]) },
        gTexture1Normal: { value: placeholderTexture([128, 128, 255, 255]) },
        gTexture1Metallic: { value: placeholderTexture([0, 0, 0, 255]) },
        gTexture1Roughness: { value: placeholderTexture([0, 255, 0, 255]) },
        gTexture1AO: { value: placeholderTexture([255, 255, 255, 255]) },
        gTexture2: { value: placeholderCube },
        uEnvPanorama: { value: placeholderTexture([0, 0, 0, 255]) },
        uEnvPanoramaMode: { value: 0 },
      },
      depthTest: true,
      depthWrite: true,
      side: THREE.FrontSide,
    });

    this.mesh = new THREE.Mesh(buildBoxyGeometry(), this.material);
    this.mesh.frustumCulled = false;
    this.group.add(this.mesh);

    this._loadMaterialMaps();
    this._loadEnvironment();

    // Clear1 is (0, 0, 0, 0): transparent, like the preset. Boxy is a
    // full-screen standalone scene, so modV composites it over black.
    if (this.renderer) {
      this.renderer.setClearColor(0x000000, 0);
      // Shader2 only applies _tolinear (no filmic/ACES curve), so leave the
      // renderer's tone mapping at NoToneMapping and let OutputPass do the
      // linear -> sRGB presentation step.
      this.renderer.toneMapping = THREE.NoToneMapping;

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

  _loadMaterialMaps() {
    const uniforms = this.material.uniforms;
    const maxAniso = this.renderer?.capabilities?.getMaxAnisotropy?.() ?? 8;
    const load = (url, key, srgb) =>
      loadTexture(url)
        .then((texture) => {
          texture.colorSpace = srgb ? THREE.SRGBColorSpace : THREE.NoColorSpace;
          texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
          texture.anisotropy = maxAniso;
          texture.needsUpdate = true;
          uniforms[key].value = texture;
        })
        .catch((error) => {
          console.warn(`Boxy texture "${key}" failed:`, error);
        });

    load(diffuseUrl, "gTexture1", true);
    load(normalUrl, "gTexture1Normal", false);
    load(metallicUrl, "gTexture1Metallic", false);
    load(roughnessUrl, "gTexture1Roughness", false);
    load(aoUrl, "gTexture1AO", false);
  }

  async _loadEnvironment() {
    const uniforms = this.material.uniforms;

    // Load posz as a panorama up front: it is both a useful fallback and, when
    // the cube upload works, an unused sampler. uEnvPanoramaMode stays 0.
    loadTexture(stormyPosZ)
      .then((texture) => {
        texture.colorSpace = THREE.SRGBColorSpace;
        texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
        texture.needsUpdate = true;
        uniforms.uEnvPanorama.value = texture;
      })
      .catch((error) => {
        console.warn("Boxy panorama fallback failed:", error);
      });

    try {
      const images = await Promise.all(STORMY_FACES.map(loadImageBitmap));
      if (images.some((image) => !image || !image.width || !image.height)) {
        throw new Error("empty Stormy Days face");
      }
      // three uploads an ImageBitmap array as a cube face set; needsUpdate
      // schedules the six texImage2D calls on the next render.
      const cube = new THREE.CubeTexture(images);
      cube.colorSpace = THREE.SRGBColorSpace;
      cube.needsUpdate = true;
      uniforms.gTexture2.value = cube;
    } catch (error) {
      // If the worker cannot build/upload ImageBitmap cube faces, keep the
      // environment: sample the +Z face through an equirect lookup instead.
      console.warn(
        "Boxy cubemap failed, falling back to the posz panorama:",
        error,
      );
      uniforms.uEnvPanoramaMode.value = 1;
    }
  }

  _applyCamera(w, h) {
    const cam = this.camera;
    if (!cam) return;
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
    // audio or every band reads 0 and the tunnel stops scrolling.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const drive = boxyDrive(
      this.p9audio.band(0, 1, 0, 0),
      this.p9audio.band(1, 1, 0, 0),
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
