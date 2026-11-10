// Sun: the Plane9 "Sun" preset (author Joakim Dahl, CC BY-NC-SA 3.0) ported
// to a standalone native three.js scene - no XML, no P9Scene node graph.
//
// The preset renders a ridged-multifractal sun sphere plus an additive corona
// disc into RenderToTexture2, blurs a copy through a multi-mip gaussian
// (Blur1), combines the blurred and sharp sun (Shader3), tints the result
// orange (CopyTexture1), then a fullscreen god-ray shader accumulates the
// tint over 50 samples and squares the result (Shader2). The port keeps the
// shader math verbatim and reproduces the chain with explicit render targets.
// The preset's RenderToTexture Shader ports are dead when a Render input chain
// exists, so they are omitted like the other imported scenes.
//
// The original Clear2 is opaque black and composites over PreviousLayer. modV
// has no previous-layer texture for a standalone scene, so the final pass
// derives alpha from the ray brightness and composites transparently over the
// layer below (same convention as the other imported scenes).
//
// Expressions:
//   Shader1 in1.x = min(0.12, band(-1,2,0,0.1)*1 + 0.03)
//   Shader4 in1.x = min(0.5, band(-1,2,0,0.05)*20 + 0.1)

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";

export const SUN = {
  fov: 45,
  near: 0.1,
  far: 1000,
  cameraPosition: [0, 0, 3.8],
  cameraLookAt: [0, 0, -1],
  radius: 1,
  segments: 32,
  ringInner: 1,
  ringOuter: 1.1,
  ringSegments: 32,
  rays: { samples: 50, density: 1, weight: 0.2, decay: 0.96 },
  tint: [0.882, 0.439, 0.024],
  surface: { floor: 0.03, gain: 1, ceiling: 0.12, damping: 2, band: 0 },
  corona: { floor: 0.1, gain: 20, ceiling: 0.5, damping: 2, band: 0 },
  // Blur1 Width; the port's 21-tap pyramid keeps the preset's wide halo.
  blur: { width: 4, taps: 21, sigma: 1.2 },
};

export function sunSurfaceLevel(bass, scale = 1) {
  const b = Math.max(0, Number.isFinite(bass) ? bass : 0);
  const level = Math.min(
    SUN.surface.ceiling,
    b * SUN.surface.gain + SUN.surface.floor,
  );

  return level * Math.max(0, scale);
}

export function sunCoronaLevel(bass, scale = 1) {
  const b = Math.max(0, Number.isFinite(bass) ? bass : 0);
  const level = Math.min(
    SUN.corona.ceiling,
    b * SUN.corona.gain + SUN.corona.floor,
  );

  return level * Math.max(0, scale);
}

export function sunRayWeight(scale = 1) {
  return SUN.rays.weight * Math.max(0, scale);
}

const FULLSCREEN_VERTEX = `
precision highp float;
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

export function buildSunSurfaceVertex() {
  return `
precision highp float;
varying vec3 vWpos;
varying vec2 vTex;
void main() {
  vWpos = position;
  vTex = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;
}

export function buildSunSurfaceFragment() {
  return `
precision highp float;
uniform float gTime;
uniform vec4 gIn1;
varying vec3 vWpos;
varying vec2 vTex;
${COMMON_PREAMBLE}
void main() {
  float v = _ridgedmffast(vWpos*15.0 + vec3(0.0, 0.0, gTime*0.2), 3.0);
  // Sunspots
  v *= smoothstep(1.0, 0.6, _noise(vWpos*3.0 - vec3(0.0, 0.0, gTime*0.2))*0.5+0.5);
  gl_FragColor = vec4(vec3(v*gIn1.x), 1.0);
}
`;
}

export function buildSunCoronaVertex() {
  return `
precision highp float;
varying vec2 vTex;
void main() {
  vTex = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;
}

export function buildSunCoronaFragment() {
  return `
precision highp float;
uniform float gTime;
uniform vec4 gIn1;
varying vec2 vTex;
${COMMON_PREAMBLE}
void main() {
  vec2 p = vTex*vec2(2.0)-vec2(1.0);
  float v1 = _ridgedmffast(vec3(vTex*50.0, gTime*0.5), 5.0);
  float v2 = smoothstep(0.7, 1.0, _turbulencefast(vec3(vTex*10.0, gTime*0.2), 3.0)+0.4);
  float v3 = smoothstep(1.0, 0.9, length(p));
  float v = v1*v2*v3*gIn1.x;
  gl_FragColor = vec4(vec3(v), 0.0);
}
`;
}

export function buildSunBlurFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform float gW[21];
uniform vec2 gDir;
uniform vec2 gTexel;
varying vec2 vUv;
void main() {
  vec3 c = vec3(0.0);
  for (int i = 0; i < 21; i++) {
    vec2 offset = gDir * gTexel * float(i - 10);
    c += texture2D(gTexture1, vUv + offset).rgb * gW[i];
  }
  gl_FragColor = vec4(c, 1.0);
}
`;
}

export function buildSunCopyFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
varying vec2 vUv;
void main() {
  gl_FragColor = texture2D(gTexture1, vUv);
}
`;
}

// Shader3: take the blurred sun's red channel, add the sharp sun, keep the
// sharp sun's alpha (the additive corona writes alpha 0).
export function buildSunCoronaCompositeFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform sampler2D gTexture2;
uniform sampler2D gTexture3;
uniform sampler2D gTexture4;
uniform sampler2D gTexture5;
uniform sampler2D gTexture6;
varying vec2 vUv;
void main() {
  vec4 c1 = texture2D(gTexture2, vUv) * 0.333;
  c1 += texture2D(gTexture3, vUv) * 0.267;
  c1 += texture2D(gTexture4, vUv) * 0.2;
  c1 += texture2D(gTexture5, vUv) * 0.133;
  c1 += texture2D(gTexture6, vUv) * 0.067;
  vec4 c2 = texture2D(gTexture1, vUv);
  gl_FragColor = vec4(c1.rrr*3.0+c2.rgb*0.9, c2.a);
}
`;
}

export function buildSunTintFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform vec4 gColor1;
varying vec2 vUv;
void main() {
  gl_FragColor = gColor1*texture2D(gTexture1, vUv).r;
}
`;
}

export function buildSunScreenFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform sampler2D gTexture2;
uniform vec4 gIn1;
uniform float gRayWeight;
uniform float gBrightness;
varying vec2 vUv;
void main() {
  int NUM_SAMPLES = 50;
  vec2 lightPosTex = gIn1.xy;
  float density = 1.0;
  float weight = gRayWeight;
  float decay = 0.96;

  vec2 texPos = vUv;
  vec2 deltaTexCoord = (texPos - lightPosTex.xy);
  deltaTexCoord *= 1.0 / float(NUM_SAMPLES) * density;

  vec3 c = texture2D(gTexture1, texPos).rgb;
  float illuminationDecay = 1.0;
  for (int i = 0; i < NUM_SAMPLES; i++) {
    texPos -= deltaTexCoord;
    vec3 s = texture2D(gTexture1, texPos).rgb;
    c += s*illuminationDecay * weight;
    illuminationDecay *= decay;
  }

  vec4 src = texture2D(gTexture2, vUv);
  c += src.rgb*0.8;

  vec3 outColor = c*c*gBrightness;
  gl_FragColor = vec4(outColor, clamp(max(outColor.r, max(outColor.g, outColor.b)), 0.0, 1.0));
}
`;
}

function makeFullscreenQuad(material) {
  const quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2), material);
  quad.frustumCulled = false;
  return quad;
}

export class Sun extends BaseScene {
  _init() {
    this.label = "Sun";
    this.role = "standalone";
    this.surfaceSpeed = 1;
    this.brightnessScale = 1;
    this.coronaScale = 1;
    this.raysScale = 1;
    this.size = { w: 2, h: 2 };
    this._targetSize = { w: 0, h: 0 };

    this.surfaceMaterial = new THREE.ShaderMaterial({
      vertexShader: buildSunSurfaceVertex(),
      fragmentShader: buildSunSurfaceFragment(),
      uniforms: {
        gTime: { value: 0 },
        gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
      },
      side: THREE.FrontSide,
      depthTest: false,
      depthWrite: false,
    });

    this.coronaMaterial = new THREE.ShaderMaterial({
      vertexShader: buildSunCoronaVertex(),
      fragmentShader: buildSunCoronaFragment(),
      uniforms: {
        gTime: { value: 0 },
        gIn1: { value: new THREE.Vector4(0, 0, 0, 0) },
      },
      side: THREE.DoubleSide,
      transparent: true,
      depthTest: false,
      depthWrite: false,
      blending: THREE.CustomBlending,
      blendEquation: THREE.AddEquation,
      blendSrc: THREE.OneFactor,
      blendDst: THREE.OneFactor,
      blendEquationAlpha: THREE.AddEquation,
      blendSrcAlpha: THREE.OneFactor,
      blendDstAlpha: THREE.OneFactor,
    });

    this.sun = new THREE.Mesh(
      new THREE.SphereGeometry(SUN.radius, SUN.segments, SUN.segments),
      this.surfaceMaterial,
    );
    this.group.add(this.sun);

    this.corona = new THREE.Mesh(
      new THREE.RingGeometry(SUN.ringInner, SUN.ringOuter, SUN.ringSegments, 1),
      this.coronaMaterial,
    );
    this.group.add(this.corona);

    this._copyMaterial = new THREE.ShaderMaterial({
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: buildSunCopyFragment(),
      uniforms: {
        gTexture1: { value: null },
      },
      depthTest: false,
      depthWrite: false,
    });

    const weights = new Float32Array(SUN.blur.taps);
    let sum = 0;
    for (let i = 0; i < SUN.blur.taps; i++) {
      const d = i - (SUN.blur.taps - 1) / 2;
      weights[i] = Math.exp((-0.5 * d * d) / (SUN.blur.sigma * SUN.blur.sigma));
      sum += weights[i];
    }
    for (let i = 0; i < SUN.blur.taps; i++) weights[i] /= sum;

    this.blurMaterial = new THREE.ShaderMaterial({
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: buildSunBlurFragment(),
      uniforms: {
        gTexture1: { value: null },
        gW: { value: weights },
        gDir: { value: new THREE.Vector2(1, 0) },
        gTexel: { value: new THREE.Vector2() },
      },
      depthTest: false,
      depthWrite: false,
    });

    this.coronaCompositeMaterial = new THREE.ShaderMaterial({
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: buildSunCoronaCompositeFragment(),
      uniforms: {
        gTexture1: { value: null },
        gTexture2: { value: null },
        gTexture3: { value: null },
        gTexture4: { value: null },
        gTexture5: { value: null },
        gTexture6: { value: null },
      },
      depthTest: false,
      depthWrite: false,
    });

    this.tintMaterial = new THREE.ShaderMaterial({
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: buildSunTintFragment(),
      uniforms: {
        gTexture1: { value: null },
        gColor1: {
          value: new THREE.Vector4(...SUN.tint, 1),
        },
      },
      depthTest: false,
      depthWrite: false,
    });

    this.screenMaterial = new THREE.ShaderMaterial({
      vertexShader: FULLSCREEN_VERTEX,
      fragmentShader: buildSunScreenFragment(),
      uniforms: {
        gTexture1: { value: null },
        gTexture2: { value: null },
        gIn1: { value: new THREE.Vector4(0.5, 0.5, 0, 0) },
        gRayWeight: { value: SUN.rays.weight },
        gBrightness: { value: 1 },
      },
      depthTest: false,
      depthWrite: false,
    });

    this.quad = makeFullscreenQuad(this._copyMaterial);
    this.quadScene = new THREE.Scene();
    this.quadScene.add(this.quad);
    this.quadCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);

    this._applyCamera();

    if (this.renderer) {
      this.renderer.setClearColor(0x000000, 0);
      this.renderer.toneMapping = THREE.NoToneMapping;
      const size = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: size.x, h: size.y };
    }

    this.p9audio = new P9Audio(this.audio);
  }

  _applyCamera() {
    if (!this.camera) return;
    this.camera.fov = SUN.fov;
    this.camera.near = SUN.near;
    this.camera.far = SUN.far;
    this.camera.aspect = this.size.w / Math.max(1, this.size.h);
    this.camera.position.set(...SUN.cameraPosition);
    this.camera.lookAt(...SUN.cameraLookAt);
    this.camera.updateProjectionMatrix();
    this.camera.updateMatrixWorld();
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the sun freezes.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    const bass = this.p9audio.band(
      -1,
      SUN.surface.damping,
      SUN.surface.band,
      0.1,
    );
    const surface = sunSurfaceLevel(bass, 1);
    const corona = sunCoronaLevel(
      this.p9audio.band(-1, SUN.corona.damping, SUN.corona.band, 0.05),
      this.coronaScale,
    );

    this.surfaceMaterial.uniforms.gIn1.value.x = surface;
    this.coronaMaterial.uniforms.gIn1.value.x = corona;

    const time = this.time * this.surfaceSpeed;
    this.surfaceMaterial.uniforms.gTime.value = time;
    this.coronaMaterial.uniforms.gTime.value = time;

    this.screenMaterial.uniforms.gRayWeight.value = sunRayWeight(
      this.raysScale,
    );
    this.screenMaterial.uniforms.gBrightness.value = this.brightnessScale;
  }

  _ensureTargets(width, height) {
    if (
      this._targetSize.w === width &&
      this._targetSize.h === height &&
      this.sunTarget
    ) {
      return;
    }

    for (const target of [
      this.sunTarget,
      this.coronaTarget,
      this.tintTarget,
      this._blurAux,
      ...(this._mips ?? []),
    ]) {
      target?.dispose();
    }

    const halfW = Math.max(2, width >> 1);
    const halfH = Math.max(2, height >> 1);
    this.sunTarget = new THREE.WebGLRenderTarget(width, height, {
      type: THREE.HalfFloatType,
      depthBuffer: false,
    });
    this.coronaTarget = new THREE.WebGLRenderTarget(width, height, {
      type: THREE.HalfFloatType,
      depthBuffer: false,
    });
    this.tintTarget = new THREE.WebGLRenderTarget(width, height, {
      type: THREE.HalfFloatType,
      depthBuffer: false,
    });
    this._blurAux = new THREE.WebGLRenderTarget(halfW, halfH, {
      type: THREE.HalfFloatType,
      depthBuffer: false,
    });
    this._mips = [0, 1, 2, 3, 4].map(
      (i) =>
        new THREE.WebGLRenderTarget(
          Math.max(2, halfW >> i),
          Math.max(2, halfH >> i),
          {
            type: THREE.HalfFloatType,
            depthBuffer: false,
          },
        ),
    );

    this.coronaCompositeMaterial.uniforms.gTexture1.value =
      this.sunTarget.texture;
    this.coronaCompositeMaterial.uniforms.gTexture2.value =
      this._mips[0].texture;
    this.coronaCompositeMaterial.uniforms.gTexture3.value =
      this._mips[1].texture;
    this.coronaCompositeMaterial.uniforms.gTexture4.value =
      this._mips[2].texture;
    this.coronaCompositeMaterial.uniforms.gTexture5.value =
      this._mips[3].texture;
    this.coronaCompositeMaterial.uniforms.gTexture6.value =
      this._mips[4].texture;
    this.tintMaterial.uniforms.gTexture1.value = this.coronaTarget.texture;
    this.screenMaterial.uniforms.gTexture1.value = this.tintTarget.texture;
    this.screenMaterial.uniforms.gTexture2.value = this.coronaTarget.texture;

    this._targetSize = { w: width, h: height };
  }

  _pass(material, target) {
    this.quad.material = material;
    this.renderer.setRenderTarget(target);
    this.renderer.render(this.quadScene, this.quadCamera);
  }

  _blurPyramid() {
    this._copyMaterial.uniforms.gTexture1.value = this.sunTarget.texture;
    this._pass(this._copyMaterial, this._mips[0]);

    const blurInto = (from, to) => {
      const spread = from.width / to.width;

      this.blurMaterial.uniforms.gTexture1.value = from.texture;
      this.blurMaterial.uniforms.gDir.value.set(1, 0);
      this.blurMaterial.uniforms.gTexel.value.set(spread / to.width, 0);
      this._pass(this.blurMaterial, this._blurAux);

      this.blurMaterial.uniforms.gTexture1.value = this._blurAux.texture;
      this.blurMaterial.uniforms.gDir.value.set(0, 1);
      this.blurMaterial.uniforms.gTexel.value.set(0, spread / to.height);
      this._pass(this.blurMaterial, to);
    };

    for (let i = 0; i < 4; i++) blurInto(this._mips[i], this._mips[i + 1]);
  }

  render() {
    if (!this.renderer || !this.camera) return;

    const width = this.renderer.domElement.width;
    const height = this.renderer.domElement.height;
    if (width !== this.size.w || height !== this.size.h) {
      this.size = { w: width, h: height };
      this.camera.aspect = width / Math.max(1, height);
      this.camera.updateProjectionMatrix();
    }
    this._ensureTargets(width, height);

    this.renderer.setRenderTarget(this.sunTarget);
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.clear(true, true, false);
    this.renderer.render(this.group, this.camera);

    this._blurPyramid();
    this._pass(this.coronaCompositeMaterial, this.coronaTarget);
    this._pass(this.tintMaterial, this.tintTarget);
    this._pass(this.screenMaterial, null);
  }

  resize(w, h) {
    this.size = { w, h };
    if (this.camera) {
      this.camera.aspect = w / Math.max(1, h);
      this.camera.updateProjectionMatrix();
    }
  }

  dispose() {
    for (const target of [
      this.sunTarget,
      this.coronaTarget,
      this.tintTarget,
      this._blurAux,
      ...(this._mips ?? []),
    ]) {
      target?.dispose();
    }
    this._mips = null;
    for (const material of [
      this.surfaceMaterial,
      this.coronaMaterial,
      this._copyMaterial,
      this.blurMaterial,
      this.coronaCompositeMaterial,
      this.tintMaterial,
      this.screenMaterial,
    ]) {
      material?.dispose();
    }
    this.sun?.geometry?.dispose();
    this.corona?.geometry?.dispose();
    this.quad?.geometry?.dispose();
  }
}
