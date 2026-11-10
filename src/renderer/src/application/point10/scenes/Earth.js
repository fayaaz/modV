// Earth: the Plane9 "Earth" preset (author Joakim Dahl, CC BY-NC-SA 4.0)
// ported to a standalone native three.js scene - no XML, no P9Scene node graph.
//
// The preset is a planet sphere (day/night/clouds), an atmosphere shell, an
// inside-out sky sphere with procedural stars + Milky Way panorama, and a tiny
// 100x-bright sun, all drawn into a half-float RenderToTexture, bloomed, then
// ACES-tonemapped by a fullscreen shader. The port keeps the shader math
// verbatim and reproduces the chain with three's RenderPass ->
// UnrealBloomPass -> OutputPass composer. The preset's RenderToTexture Shader
// is dead (a Render input chain replaces it), so it is omitted like the other
// imported scenes.
//
// The port also adopts Point10's tuned overrides (P9Overrides.js): the camera
// orbits at 34 + noise*5.5 instead of 25 + noise*5 (zoomed out), and the sun's
// day/night sweep runs at time*0.04 instead of time*0.1.
//
// Expression port (permrand = RandomSeed = 1):
//   t = 15 + time*0.005
//   rot = noise2(t*0.2, 0); dist = 34 + noise2(t*2.0, 2)*5.5
//   camera = (cos(rot*PI*4)*dist, noise2(t, 1)*10, sin(rot*PI*4)*dist)
//   lookAt = noise2(t, 3.42/6.73/8.45)*5
//   sun t = 1 + 10 + time*0.04 -> (sin(-t)*100, 0, cos(-t)*100)
//   in1.x = 10 + time*0.002            (texture scroll)
//   in3.x = min(band(-1,1,1,1)*3, 0.8)+0.2  (night lights)

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { valueNoise } from "../plane9/P9Expression.js";
import { loadTexture } from "../shared/textures.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";
import dayTextureUrl from "../textures/earth.jpg?url";
import nightTextureUrl from "../textures/earthlights.jpg?url";
import cloudTextureUrl from "../textures/earthcloud.jpg?url";
import panoramaTextureUrl from "../textures/wilkyway_panorama.jpg?url";

export const EARTH = {
  // Expression RandomSeed is 1 in the preset; permrand starts at that value.
  seed: 1,
  radius: 10,
  atmosphereRadius: 10.2,
  skyRadius: 150,
  sunDistance: 100,
  sunTimeScale: 0.04,
  cameraDistance: 34,
  cameraDistanceVar: 5.5,
  fov: 60.4,
  near: 0.1,
  far: 200,
  // Bloom1: Radius 7 / Threshold 0 / Smoothness 0.5 / Intensity 0.2.
  bloom: { strength: 0.2, radius: 0.85, threshold: 0 },
};

export function earthCamera(time) {
  const t = EARTH.seed * 15 + time * 0.005;
  const rot = valueNoise(2, t * 0.2, 0);
  const dist =
    EARTH.cameraDistance + valueNoise(2, t * 2, 2) * EARTH.cameraDistanceVar;

  return {
    position: {
      x: Math.cos(rot * Math.PI * 4) * dist,
      y: valueNoise(2, t, 1) * 10,
      z: Math.sin(rot * Math.PI * 4) * dist,
    },
    lookAt: {
      x: valueNoise(2, t, 3.42) * 5,
      y: valueNoise(2, t, 6.73) * 5,
      z: valueNoise(2, t, 8.45) * 5,
    },
  };
}

export function earthSunPosition(time, speed = 1) {
  const t =
    1 + 10 * EARTH.seed + time * EARTH.sunTimeScale * Math.max(0, speed);

  return { x: Math.sin(-t) * 100, y: 0, z: Math.cos(-t) * 100 };
}

export function earthTexturePhase(time, speed = 1) {
  return EARTH.seed * 10 + time * 0.002 * Math.max(0, speed);
}

export function earthLightLevel(mid, scale = 1) {
  const m = Math.max(0, Number.isFinite(mid) ? mid : 0);

  return (Math.min(m * 3, 0.8) + 0.2) * Math.max(0, scale);
}

// The planet vertex shader carries the atmospheric scattering from the
// original planet shader (based on nicholaswoodfield.com, CC BY-SA 3.0).
export function buildEarthSurfaceVertex() {
  return `
precision highp float;
uniform vec3 gIn2;
uniform vec4 gIn1;
uniform vec4 gIn3;
varying vec3 vWpos;
varying vec2 vTex;
varying vec3 vWnormal;
varying vec4 vCol0;
varying vec4 vCol1;

const int nSamples = 2;
const float fSamples = 2.0;
const vec3 InvWaveLength = vec3(5.602, 9.478, 19.646);
const float InnerRadius = 10.0;
const float OuterRadius = 10.2;
const float KrESun = 0.02;
const float KmESun = 0.02;
const float Kr4PI = 0.031416;
const float Km4PI = 0.012566;
const float Scale = 1.0 / (10.2 - 10.0);
const float ScaleDepth = 0.25;
const float InvScaleDepth = 1.0 / 0.25;
#define ScaleOverScaleDepth (Scale / ScaleDepth)

float scale(float fCos) {
  float x = 1.0 - fCos;
  return ScaleDepth * exp(-0.00287 + x*(0.459 + x*(3.83 + x*(-6.80 + x*5.25))));
}

float getNearIntersection(vec3 v3Pos, vec3 v3Ray, float fDistance2, float fRadius2) {
  float B = 2.0 * dot(v3Pos, v3Ray);
  float C = fDistance2 - fRadius2;
  float fDet = max(0.0, B*B - 4.0 * C);
  return 0.5 * (-B - sqrt(fDet));
}

void main() {
  vec3 CamPos = cameraPosition;
  float CamHeight = length(CamPos);
  vec3 posWS = position;
  vec3 ray = posWS - CamPos;
  posWS = normalize(posWS);
  float far = length(ray);
  ray /= far;
  float near = getNearIntersection(CamPos, ray, CamHeight*CamHeight, OuterRadius*OuterRadius);
  vec3 rayStart = CamPos + (ray * near);
  far -= near;

  vec3 lightDir = normalize(gIn2 - posWS);

  float depth = exp((InnerRadius - OuterRadius) * InvScaleDepth);
  float camAngle = dot(-ray, posWS);
  float camScale = scale(camAngle);
  float lightAngle = dot(lightDir, posWS);
  float lightScale = scale(lightAngle);
  float camOffset = depth * camScale;
  float temp = (lightScale + camScale);

  float sampleLength = far / fSamples;
  float scaledLength = sampleLength * Scale;
  vec3 sampleRay = ray * sampleLength;
  vec3 samplePoint = rayStart + (sampleRay * 0.5);

  vec3 frontColor = vec3(0.0);
  vec3 attenuate = vec3(1.0);

  for (int i = 0; i < nSamples; i++) {
    float height = length(samplePoint);
    float depth2 = exp(ScaleOverScaleDepth * (InnerRadius - height));
    float scatter = (depth2 * temp) - camOffset;
    attenuate = exp(-scatter * ((InvWaveLength * Kr4PI) + Km4PI));
    frontColor += attenuate * (depth2 * scaledLength);
    samplePoint += sampleRay;
  }

  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
  vWpos = posWS;
  vWnormal = normalize(normal);
  vCol0 = vec4(frontColor * vec3((InvWaveLength * KrESun) + KmESun) * vec3(gIn3.x), 1.0);
  vCol1 = vec4(attenuate + 0.05, 1.0);
  vTex = uv + vec2(gIn1.x, 0.0);
}
`;
}

export function buildEarthSurfaceFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform sampler2D gTexture2;
uniform sampler2D gTexture3;
uniform vec3 gIn2;
uniform vec4 gIn3;
varying vec3 vWpos;
varying vec2 vTex;
varying vec3 vWnormal;
varying vec4 vCol0;
varying vec4 vCol1;
${COMMON_PREAMBLE}
void main() {
  vec3 lightDir = normalize(gIn2 - vWpos);
  vec3 normalWS = normalize(vWnormal);

  vec4 ground = texture2D(gTexture1, vTex.xy);
  vec4 clouds = texture2D(gTexture3, vTex.xy);
  float groundGrey = dot(vec3(0.222, 0.707, 0.071), ground.rgb);

  vec3 ambient = vec3(1.0, 1.0, 1.0) * 0.01;
  float ndl = dot(normalWS, lightDir);
  float NDotL = max(ndl, 0.0);
  vec3 diffuse = vec3(NDotL);

  vec4 albedo = ground + clouds;
  vec4 c = vec4((diffuse + ambient) * albedo.rgb, 1.0);
  c = vec4((vCol0 + (c * vCol1)).rgb, max(-ndl, 0.1));
  c.rgb = max(vec3(0.0, 0.0, 0.0), _brightnessSaturationContrast(c.rgb, 1.2, 2.2, 1.05));

  // The dark side: city lights tinted orange, plus a faint blue ambient.
  vec4 lights = (texture2D(gTexture2, vTex) * vec4(3.0 * gIn3.x) * vec4(1.0, 0.82, 0.2, 1.0));
  lights += (groundGrey + clouds * 0.125) * vec4(0.01, 0.01, 0.2, 1.0);
  lights *= max(-ndl, 0.0);

  // Plane9 draws the planet with SrcBlend=1/DstBlend=0 (opaque), so the
  // shader's 0.1 alpha never faded it. The adapter composites the scene
  // canvas, so keep the planet fully opaque or it blends into the layer below.
  gl_FragColor = vec4((c + lights).rgb, 1.0);
}
`;
}

// The atmosphere shell is offset outwards by OuterRadius-InnerRadius in the
// vertex shader, so it shares Sphere1's geometry with Shader5.
export function buildEarthAtmosphereVertex() {
  return `
precision highp float;
uniform vec3 gIn2;
varying vec3 vWpos;
varying vec4 vCol0;
varying vec4 vCol1;
varying vec3 vTex;

const int nSamples = 2;
const float fSamples = 2.0;
const vec3 InvWaveLength = vec3(5.602, 9.478, 19.646);
const float InnerRadius = 10.0;
const float OuterRadius = 10.2;
const float KrESun = 0.02;
const float KmESun = 0.02;
const float Kr4PI = 0.031416;
const float Km4PI = 0.012566;
const float Scale = 1.0 / (10.2 - 10.0);
const float ScaleDepth = 0.25;
const float InvScaleDepth = 1.0 / 0.25;
#define ScaleOverScaleDepth (Scale / ScaleDepth)

float scale(float fCos) {
  float x = 1.0 - fCos;
  return ScaleDepth * exp(-0.00287 + x*(0.459 + x*(3.83 + x*(-6.80 + x*5.25))));
}

float getNearIntersection(vec3 v3Pos, vec3 v3Ray, float fDistance2, float fRadius2) {
  float B = 2.0 * dot(v3Pos, v3Ray);
  float C = fDistance2 - fRadius2;
  float fDet = max(0.0, B*B - 4.0 * C);
  return 0.5 * (-B - sqrt(fDet));
}

void main() {
  vec3 pos = position + (normal * (OuterRadius - InnerRadius));
  vec3 CamPos = cameraPosition;
  vec3 posWS = pos;
  float CamHeight = length(CamPos);
  vec3 ray = posWS - CamPos;
  float far = length(ray);
  ray /= far;

  vec3 lightDir = normalize(gIn2 - posWS);
  float near = getNearIntersection(CamPos, ray, CamHeight*CamHeight, OuterRadius*OuterRadius);
  vec3 rayStart = CamPos + (ray * near);
  far -= near;

  float startAngle = dot(ray, rayStart) / OuterRadius;
  float startDepth = exp(-InvScaleDepth);
  float startOffset = startDepth * scale(startAngle);

  float sampleLength = far / fSamples;
  float scaledLength = sampleLength * Scale;
  vec3 sampleRay = ray * sampleLength;
  vec3 samplePoint = rayStart + (sampleRay * 0.5);

  vec3 frontColor = vec3(0.0);
  for (int i = 0; i < nSamples; i++) {
    float height = length(samplePoint);
    float depth = exp(ScaleOverScaleDepth * (InnerRadius - height));
    float lightAngle = dot(lightDir, samplePoint) / height;
    float cameraAngle = dot(ray, samplePoint) / height;
    float scatter = startOffset + (depth * (scale(lightAngle) - scale(cameraAngle)));
    vec3 attenuate = exp(-scatter * ((InvWaveLength * Kr4PI) + Km4PI));
    frontColor += attenuate * (depth * scaledLength);
    samplePoint += sampleRay;
  }

  gl_Position = projectionMatrix * modelViewMatrix * vec4(pos, 1.0);
  vWpos = posWS;
  vCol0 = vec4(frontColor * vec3(InvWaveLength * KrESun), 1.0);
  vCol1 = vec4(frontColor * vec3(KmESun), 1.0);
  vTex = CamPos - posWS;
}
`;
}

export function buildEarthAtmosphereFragment() {
  return `
precision highp float;
uniform vec3 gIn2;
uniform float uAtmosphere;
varying vec3 vWpos;
varying vec4 vCol0;
varying vec4 vCol1;
varying vec3 vTex;

const float G = -0.95;

void main() {
  vec3 lightDir = normalize(gIn2 - vWpos);
  float fCos = dot(lightDir, vTex) / length(vTex);
  float fRayleighPhase = 0.75 * (1.0 + fCos*fCos);
  float fMiePhase = 1.5 * ((1.0 - (G*G)) / (2.0 + (G*G))) * (1.0 + fCos*fCos) / pow(1.0 + (G*G) - 2.0*G*fCos, 1.5);
  vec4 c = fRayleighPhase * vCol0 + fMiePhase * vCol1;
  c *= uAtmosphere;
  gl_FragColor = vec4(clamp(c.rgb, 0.0, 100.0), clamp(max(c.r, max(c.g, c.b)), 0.0, 1.0));
}
`;
}

export function buildEarthSkyVertex() {
  return `
precision highp float;
varying vec3 vWorldpos;
varying vec3 vWnormal;
varying vec2 vTex;

void main() {
  vWorldpos = position;
  vWnormal = normalize(normal);
  vTex = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;
}

export function buildEarthSkyFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform vec4 gColor1;
varying vec3 vWorldpos;
varying vec3 vWnormal;
varying vec2 vTex;
${COMMON_PREAMBLE}

float sampleTexture(vec3 p) {
  float stars = smoothstep(0.7, 1.0, _noise(p*1.5));
  stars += smoothstep(0.8, 1.0, _noise(p*0.75))*1.5;
  return stars;
}

const int MAX_SAMPLES = 2;

vec3 sampleTextureWithFilter(vec3 uvw, float detail) {
  vec3 ddx_uvw = uvw + dFdx(uvw);
  vec3 ddy_uvw = uvw + dFdy(uvw);

  int sx = 1 + int(clamp(detail*length(ddx_uvw-uvw), 0.0, float(MAX_SAMPLES-1)));
  int sy = 1 + int(clamp(detail*length(ddy_uvw-uvw), 0.0, float(MAX_SAMPLES-1)));
  vec3 no = vec3(0.0);
  for (int j = 0; j < MAX_SAMPLES; j++)
  for (int i = 0; i < MAX_SAMPLES; i++) {
    if (j < sy && i < sx) {
      vec2 st = vec2(float(i), float(j)) / vec2(float(sx), float(sy));
      no += vec3(sampleTexture(uvw + st.x * (ddx_uvw-uvw) + st.y*(ddy_uvw-uvw)));
    }
  }
  return no / float(sx*sy);
}

void main() {
  vec3 pp = vWorldpos * 0.01;
  vec3 c;
  float cloud = smoothstep(0.6, 0.0, _fbmfast(pp*3.0, 3.0)*0.8+0.5)*0.25;

  float stars = sampleTextureWithFilter(vWorldpos, 8.0).x;
  stars *= clamp(1.0-cloud, 0.0, 1.0);

  c.rgb = vec3(stars*0.2) + cloud*gColor1.rgb*0.05;
  c.rgb += _texturePanoramic(gTexture1, normalize(vWnormal)).rgb*0.05;

  // The preset clears to an opaque black sky; imported scenes composite over
  // the layer below, so the alpha follows the star/cloud brightness instead.
  gl_FragColor = vec4(c, clamp(max(c.r, max(c.g, c.b)), 0.0, 1.0));
}
`;
}

function placeholderTexture(rgba) {
  const texture = new THREE.DataTexture(
    new Uint8Array(rgba),
    1,
    1,
    THREE.RGBAFormat,
  );
  texture.needsUpdate = true;
  return texture;
}

function configureTexture(texture, maxAnisotropy) {
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.RepeatWrapping;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.magFilter = THREE.LinearFilter;
  texture.anisotropy = maxAnisotropy;
  texture.needsUpdate = true;
  return texture;
}

export class Earth extends BaseScene {
  _init() {
    this.label = "Earth";
    this.role = "standalone";
    this.spinScale = 1;
    this.sunScale = 1;
    this.lightsScale = 1;
    this.atmosphereScale = 1;
    this.brightnessScale = 1;
    this.size = { w: 2, h: 2 };
    this._cw = 0;
    this._ch = 0;

    this.surfaceMaterial = new THREE.ShaderMaterial({
      vertexShader: buildEarthSurfaceVertex(),
      fragmentShader: buildEarthSurfaceFragment(),
      uniforms: {
        gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
        gIn2: { value: new THREE.Vector3() },
        gIn3: { value: new THREE.Vector4(0, 0, 0, 0) },
        gTexture1: { value: placeholderTexture([128, 128, 128, 255]) },
        gTexture2: { value: placeholderTexture([0, 0, 0, 255]) },
        gTexture3: { value: placeholderTexture([0, 0, 0, 255]) },
      },
      side: THREE.FrontSide,
      depthTest: true,
      depthWrite: true,
    });

    this.atmosphereMaterial = new THREE.ShaderMaterial({
      vertexShader: buildEarthAtmosphereVertex(),
      fragmentShader: buildEarthAtmosphereFragment(),
      uniforms: {
        gIn2: { value: new THREE.Vector3() },
        uAtmosphere: { value: 1 },
      },
      side: THREE.BackSide,
      transparent: true,
      depthTest: true,
      depthWrite: false,
    });

    this.skyMaterial = new THREE.ShaderMaterial({
      vertexShader: buildEarthSkyVertex(),
      fragmentShader: buildEarthSkyFragment(),
      uniforms: {
        gColor1: { value: new THREE.Vector4(0.051, 0.133, 0.337, 1) },
        gTexture1: { value: placeholderTexture([0, 0, 0, 255]) },
      },
      side: THREE.BackSide,
      transparent: true,
      depthTest: true,
      depthWrite: false,
    });

    this.sunMaterial = new THREE.ShaderMaterial({
      vertexShader: `
varying vec2 vTex;
void main() {
  vTex = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`,
      fragmentShader: `
precision highp float;
void main() {
  gl_FragColor = vec4(100.0, 100.0, 100.0, 1.0);
}
`,
      side: THREE.FrontSide,
      depthTest: true,
      depthWrite: true,
    });

    this.planet = new THREE.Mesh(
      new THREE.SphereGeometry(EARTH.radius, 200, 200),
      this.surfaceMaterial,
    );
    this.planet.renderOrder = 0;
    this.group.add(this.planet);

    this.sun = new THREE.Mesh(
      new THREE.SphereGeometry(1, 32, 32),
      this.sunMaterial,
    );
    this.sun.renderOrder = 1;
    this.group.add(this.sun);

    this.sky = new THREE.Mesh(
      new THREE.SphereGeometry(EARTH.skyRadius, 64, 64),
      this.skyMaterial,
    );
    this.sky.renderOrder = 2;
    this.group.add(this.sky);

    this.atmosphere = new THREE.Mesh(
      new THREE.SphereGeometry(EARTH.radius, 200, 200),
      this.atmosphereMaterial,
    );
    this.atmosphere.renderOrder = 3;
    this.group.add(this.atmosphere);

    this._applyCamera();

    if (this.renderer) {
      this.renderer.setClearColor(0x000000, 0);
      this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
      this.renderer.toneMappingExposure = 1;
      // The preset's final shader is _tonemapACES only; OutputPass would
      // otherwise add an sRGB encode on top of it.
      this.renderer.outputColorSpace = THREE.LinearSRGBColorSpace;

      const size = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: size.x, h: size.y };
      const target = new THREE.WebGLRenderTarget(1, 1, {
        type: THREE.HalfFloatType,
      });
      this.composer = new EffectComposer(this.renderer, target);
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.addPass(
        new RenderPass(this.group, this.camera, null, null, 0),
      );
      this._bloomPass = new UnrealBloomPass(
        size,
        EARTH.bloom.strength,
        EARTH.bloom.radius,
        EARTH.bloom.threshold,
      );
      this.composer.addPass(this._bloomPass);
      this.composer.addPass(new OutputPass());

      const anisotropy = this.renderer.capabilities.getMaxAnisotropy();
      this._loadTexture(dayTextureUrl, "gTexture1", anisotropy);
      this._loadTexture(nightTextureUrl, "gTexture2", anisotropy);
      this._loadTexture(cloudTextureUrl, "gTexture3", anisotropy);
      this._loadTexture(panoramaTextureUrl, "gTexture1", anisotropy, true);
    }

    this.p9audio = new P9Audio(this.audio);
  }

  _loadTexture(url, uniform, anisotropy, sky = false) {
    const material = sky ? this.skyMaterial : this.surfaceMaterial;
    loadTexture(url)
      .then((texture) => {
        material.uniforms[uniform].value = configureTexture(
          texture,
          anisotropy,
        );
      })
      .catch((error) => {
        console.warn("Earth texture unavailable:", error);
      });
  }

  _applyCamera() {
    if (!this.camera) return;
    this.camera.fov = EARTH.fov;
    this.camera.near = EARTH.near;
    this.camera.far = EARTH.far;
    this.camera.aspect = this.size.w / Math.max(1, this.size.h);
    this.camera.updateProjectionMatrix();
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the camera/sun/night lights freeze.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const time = this.time;
    const drive = earthCamera(time);
    const sun = earthSunPosition(time, this.sunScale);
    const phase = earthTexturePhase(time, this.spinScale);
    const lights = earthLightLevel(
      this.p9audio.band(-1, 1, 1, 1),
      this.lightsScale,
    );

    if (this.camera) {
      this.camera.position.set(
        drive.position.x,
        drive.position.y,
        drive.position.z,
      );
      this.camera.lookAt(drive.lookAt.x, drive.lookAt.y, drive.lookAt.z);
      this.camera.updateMatrixWorld();
      this.camera.matrixWorldInverse.copy(this.camera.matrixWorld).invert();
    }

    this.sun.position.set(sun.x, sun.y, sun.z);
    this.surfaceMaterial.uniforms.gIn1.value.x = phase;
    this.surfaceMaterial.uniforms.gIn2.value.set(sun.x, sun.y, sun.z);
    this.surfaceMaterial.uniforms.gIn3.value.x = lights;
    this.atmosphereMaterial.uniforms.gIn2.value.set(sun.x, sun.y, sun.z);
    this.atmosphereMaterial.uniforms.uAtmosphere.value = this.atmosphereScale;

    if (this.renderer) {
      this.renderer.toneMappingExposure = this.brightnessScale;
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
      if (this.camera) {
        this.camera.aspect = w / Math.max(1, h);
        this.camera.updateProjectionMatrix();
      }
    }
    this.composer.renderToScreen = renderToScreen;
    this.composer.render();
  }

  resize(w, h) {
    this.size = { w, h };
    if (this.camera) {
      this.camera.aspect = w / Math.max(1, h);
      this.camera.updateProjectionMatrix();
    }
  }

  dispose() {
    this.composer?.dispose();
    this.composer = null;
    for (const material of [
      this.surfaceMaterial,
      this.atmosphereMaterial,
      this.skyMaterial,
      this.sunMaterial,
    ]) {
      material?.dispose();
    }
    for (const mesh of [this.planet, this.sun, this.sky, this.atmosphere]) {
      mesh?.geometry?.dispose();
    }
  }
}
