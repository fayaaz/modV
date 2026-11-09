// Sound Cube: the Plane9 "Sound Cube" preset (author Joakim Dahl, CC
// BY-NC-SA 4.0) ported to a standalone native three.js scene - no XML, no
// P9Scene node graph.
//
// The preset is one rotating unit cube seen through a camera at z = 5 (fov 45)
// with Shader1 drawing a bright ring on each face. RenderToTexture1 has a
// Render input (RenderRect2 -> RenderObject1), so P9Scene captures that chain
// instead of the node's own fbm shader: the 256x256 target holds the cube plus
// a blurred, zoomed copy of its previous frame. StoreTexture1 snapshots the
// target between frames, Blur1 blurs the snapshot (Dir 2 = both axes, Width 4)
// and Shader3 adds it back with alpha 0 - a classic ping-pong feedback trail.
// RenderRect1 then composites Shader2 to the screen: PreviousLayer is the
// layer beneath the scene (transparent black in standalone playback), c is the
// feedback target, and the output is mix(bg, c, c.a) + c.
//
// The screen clean is (0, 0, 0, 0) and the briefing needs the scene to stay
// transparent on empty space, so the composite derives its alpha from the
// visible glow rather than copying the preset's opaque black clear.
//
// Expression1 (RandomSeed = 1):
//   out1 = 0.3 + band(-1, 1, 0, (noise1(time) * 0.5 + 0.5) * 0.1) * 8
// out1 feeds Vector2, the RenderObject's uniform scale, so the cube pulses
// with the smoothed bass band.

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";

// Cube1 Size (1x1x1, 1x1x1 tessellation).
export const CUBE_SIZE = 1;

// Screen1: CamPos 0 0 5, lookAt 0 0 -1, fov 45, near 0.1, far 100.
export const CAMERA = { fov: 45, near: 0.1, far: 100 };

// Clear1 is (0, 0, 0, 0): the cube composites transparently over the layers
// behind it instead of an opaque black backdrop.
export const CLEAR_ALPHA = 0;

// RenderToTexture1: WidthCustom / HeightCustom 256, CreateMipMaps true.
export const RTT_SIZE = 256;

// Shader3 samples the feedback at (p * 0.95) * 0.5 + 0.5 and writes
// vec4(c.rgb * 0.8, 0.0).
export const TRAIL_ZOOM = 0.95;
export const TRAIL_BRIGHTNESS = 0.8;

// Blur1: Dir 2 (both axes), Width 4, Brightness 1.
export const BLUR = { dir: 2, width: 4, brightness: 1 };

// Rotator1: degrees per second on each axis (P9 applies .Rot as degrees).
export const ROTATOR = { x: 1.5, y: 1, z: 0.5 };

// Expression1's silence floor and bass gain.
export const SCALE_BASE = 0.3;
export const SCALE_AUDIO = 8;

const DEG = Math.PI / 180;

// Guard against tab-suspend catch-up frames turning the rotator into a jump;
// P9 clamps its deltatime the same way.
const MAX_DT = 0.1;

// Fullscreen quad edge length (clip space is -1..1, so a 2x2 plane fills it).
const QUAD_SIZE = 2;

const QUAD_VERT = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}
`;

function number01(value, fallback) {
  return Number.isFinite(value) ? Math.max(0, value) : fallback;
}

// Pure Expression1: the smoothed bass band drives the cube's uniform scale.
export function soundCubeDrive(bass, pulseScale = 1) {
  const band = number01(bass, 0);
  const pulse = number01(pulseScale, 1);
  const expression = SCALE_BASE + band * SCALE_AUDIO;

  return {
    band,
    expression,
    scale: expression * pulse,
  };
}

// Pure feedback parameters for Shader3: the zoom is fixed by the preset and
// trailScale scales how much of the previous frame survives.
export function soundCubeFeedback(trailScale = 1) {
  return {
    zoom: TRAIL_ZOOM,
    brightness: TRAIL_BRIGHTNESS * number01(trailScale, 1),
  };
}

// Pure Rotator: advances euler angles (radians) from the preset's degree-per
// second speeds.
export function advanceSoundCubeRotation(rotation, dt, spinScale = 1) {
  const base = rotation ?? {};
  const step = Math.max(0, Number.isFinite(dt) ? Math.min(dt, MAX_DT) : 0);
  const spin = number01(spinScale, 1);
  const angle = (value, speed) =>
    (Number.isFinite(value) ? value : 0) + speed * DEG * step * spin;

  return {
    x: angle(base.x, ROTATOR.x),
    y: angle(base.y, ROTATOR.y),
    z: angle(base.z, ROTATOR.z),
  };
}

export function buildSoundCubeVertexShader() {
  return `
precision highp float;
uniform vec4 gColor;
uniform vec4 gColor1;
varying vec2 vTex;
varying vec4 vDiffuse;
void main() {
  vTex = uv;
  vDiffuse = gColor * gColor1;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;
}

export function buildSoundCubeFragmentShader() {
  return `
precision highp float;
varying vec2 vTex;
varying vec4 vDiffuse;
${COMMON_PREAMBLE}
void main() {
  vec2 p = vTex * 2.0 - 1.0;
  float v = smoothstep(1.0, 0.5, abs(p.x)) * smoothstep(1.0, 0.5, abs(p.y));
  float inner = smoothstep(0.4, 0.6, length(p));
  gl_FragColor = vec4(vec3(v * inner) * vDiffuse.rgb, 1.0);
}
`;
}

export function buildSoundCubeCompositeFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform sampler2D gTexture2;
uniform float gBrightness;
varying vec2 vUv;
${COMMON_PREAMBLE}
void main() {
  vec4 bg = texture2D(gTexture2, vUv);
  vec4 c = texture2D(gTexture1, vUv);
  vec4 col = mix(bg, c, c.a) + c;
  col.rgb *= gBrightness;

  // Keep the additively-composited glow transparent on empty space:
  // the preset's screen clear is opaque black, which would hide the layers
  // beneath this scene.
  float alpha = clamp(max(max(col.r, col.g), col.b), 0.0, 1.0);
  gl_FragColor = vec4(col.rgb, alpha);
}
`;
}

export function buildSoundCubeTrailFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform float gTrailScale;
varying vec2 vUv;
void main() {
  vec2 p = vUv * 2.0 - 1.0;
  vec4 c = texture2D(gTexture1, (p * 0.95) * 0.5 + 0.5);
  gl_FragColor = vec4(c.rgb * 0.8 * gTrailScale, 0.0);
}
`;
}

export function buildSoundCubeBlurFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
uniform vec2 gDir;
uniform vec2 gTexel;
uniform float gWidth;
varying vec2 vUv;
void main() {
  float sigma = max(0.5, gWidth * 0.5);
  vec3 sum = vec3(0.0);
  float total = 0.0;
  for (int i = -4; i <= 4; i++) {
    float fi = float(i);
    float weight = exp(-0.5 * fi * fi / (sigma * sigma));
    sum += texture2D(gTexture1, vUv + gDir * gTexel * fi).rgb * weight;
    total += weight;
  }
  gl_FragColor = vec4(sum / total, 1.0);
}
`;
}

function buildCopyFragment() {
  return `
precision highp float;
uniform sampler2D gTexture1;
varying vec2 vUv;
void main() {
  gl_FragColor = texture2D(gTexture1, vUv);
}
`;
}

function placeholderTexture(data) {
  const texture = new THREE.DataTexture(
    new Uint8Array(data),
    1,
    1,
    THREE.RGBAFormat,
  );
  texture.needsUpdate = true;
  return texture;
}

function makeTarget(width, height, type = THREE.UnsignedByteType) {
  const target = new THREE.WebGLRenderTarget(width, height, {
    type,
    depthBuffer: false,
    stencilBuffer: false,
  });
  target.texture.minFilter = THREE.LinearFilter;
  target.texture.magFilter = THREE.LinearFilter;
  target.texture.wrapS = THREE.ClampToEdgeWrapping;
  target.texture.wrapT = THREE.ClampToEdgeWrapping;
  return target;
}

export class SoundCube extends BaseScene {
  _init() {
    this.label = "Sound Cube";
    this.role = "standalone";

    this.cubeMaterial = new THREE.ShaderMaterial({
      vertexShader: buildSoundCubeVertexShader(),
      fragmentShader: buildSoundCubeFragmentShader(),
      uniforms: {
        gColor: { value: new THREE.Vector4(1, 1, 1, 1) },
        gColor1: { value: new THREE.Vector4(1, 1, 1, 1) },
      },
      depthTest: false,
      depthWrite: false,
      side: THREE.FrontSide,
      // Shader1: SrcBlend 1 / DstBlend 0 = overwrite the target.
      blending: THREE.NoBlending,
    });

    this.cubeMesh = new THREE.Mesh(
      new THREE.BoxGeometry(CUBE_SIZE, CUBE_SIZE, CUBE_SIZE),
      this.cubeMaterial,
    );
    this.cubeMesh.frustumCulled = false;

    this.cubeGroup = new THREE.Group();
    this.cubeGroup.add(this.cubeMesh);
    this.group.add(this.cubeGroup);

    this._previousLayerTexture = placeholderTexture([0, 0, 0, 0]);
    this._ortho = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
    this._quadScene = new THREE.Scene();
    this._quad = new THREE.Mesh(
      new THREE.PlaneGeometry(QUAD_SIZE, QUAD_SIZE),
      this.cubeMaterial,
    );
    this._quad.frustumCulled = false;
    this._quadScene.add(this._quad);

    this.blurMaterial = new THREE.ShaderMaterial({
      vertexShader: QUAD_VERT,
      fragmentShader: buildSoundCubeBlurFragment(),
      uniforms: {
        gTexture1: { value: this._previousLayerTexture },
        gDir: { value: new THREE.Vector2(1, 0) },
        gTexel: { value: new THREE.Vector2(1 / RTT_SIZE, 0) },
        gWidth: { value: BLUR.width },
      },
      depthTest: false,
      depthWrite: false,
      blending: THREE.NoBlending,
    });

    this.trailMaterial = new THREE.ShaderMaterial({
      vertexShader: QUAD_VERT,
      fragmentShader: buildSoundCubeTrailFragment(),
      uniforms: {
        gTexture1: { value: this._previousLayerTexture },
        gTrailScale: { value: TRAIL_BRIGHTNESS },
      },
      transparent: true,
      depthTest: false,
      depthWrite: false,
      blending: THREE.CustomBlending,
      blendEquation: THREE.AddEquation,
      blendSrc: THREE.OneFactor,
      blendDst: THREE.OneFactor,
      blendEquationAlpha: THREE.AddEquation,
      blendSrcAlpha: THREE.ZeroFactor,
      blendDstAlpha: THREE.OneFactor,
    });

    this.compositeMaterial = new THREE.ShaderMaterial({
      vertexShader: QUAD_VERT,
      fragmentShader: buildSoundCubeCompositeFragment(),
      uniforms: {
        gTexture1: { value: this._previousLayerTexture },
        gTexture2: { value: this._previousLayerTexture },
        gBrightness: { value: 1 },
      },
      depthTest: false,
      depthWrite: false,
      blending: THREE.NoBlending,
    });

    this.copyMaterial = new THREE.ShaderMaterial({
      vertexShader: QUAD_VERT,
      fragmentShader: buildCopyFragment(),
      uniforms: {
        gTexture1: { value: this._previousLayerTexture },
      },
      depthTest: false,
      depthWrite: false,
      blending: THREE.NoBlending,
    });

    if (this.renderer) {
      this.renderer.setClearColor(0x000000, CLEAR_ALPHA);
      this.renderer.toneMapping = THREE.NoToneMapping;

      // RenderToTexture1 Format 5 (RG16F): keep the feedback in float so the
      // additive trail does not clamp at 1.0 as it accumulates.
      this._readRT = makeTarget(RTT_SIZE, RTT_SIZE, THREE.HalfFloatType);
      this._writeRT = makeTarget(RTT_SIZE, RTT_SIZE, THREE.HalfFloatType);
      this._blurA = makeTarget(RTT_SIZE, RTT_SIZE, THREE.HalfFloatType);
      this._blurB = makeTarget(RTT_SIZE, RTT_SIZE, THREE.HalfFloatType);
      const size = this.renderer.getSize(new THREE.Vector2());
      this._finalRT = makeTarget(
        Math.max(1, Math.round(size.x)),
        Math.max(1, Math.round(size.y)),
      );
      this._clearTargets();
    }

    this.p9audio = new P9Audio(this.audio);
    this.rotation = { x: 0, y: 0, z: 0 };

    // Parent inputs (sceneProps wires these later). All default to the
    // preset's own strength.
    this.spinScale = 1;
    this.pulseScale = 1;
    this.trailScale = 1;
    this.brightnessScale = 1;

    this.size = { w: 2, h: 2 };
    if (this.renderer) {
      const s = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: s.x, h: s.y };
    }
    this._applyCamera(this.size.w, this.size.h);
  }

  _clearTargets() {
    const renderer = this.renderer;
    if (!renderer) return;
    const previous = renderer.getRenderTarget();
    const alpha = renderer.getClearAlpha();
    const color = renderer.getClearColor(new THREE.Color());

    renderer.setClearColor(0x000000, 0);
    for (const target of [this._readRT, this._writeRT]) {
      renderer.setRenderTarget(target);
      renderer.clear(true, false, false);
    }
    renderer.setRenderTarget(previous);
    renderer.setClearColor(color, alpha);
  }

  _applyCamera(w, h) {
    const cam = this.camera;
    if (!cam) return;
    cam.fov = CAMERA.fov;
    cam.near = CAMERA.near;
    cam.far = CAMERA.far;
    cam.aspect = w / Math.max(1, h);
    cam.position.set(0, 0, 5);
    cam.lookAt(0, 0, 0);
    cam.updateMatrixWorld();
    cam.matrixWorldInverse.copy(cam.matrixWorld).invert();
    cam.updateProjectionMatrix();
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9audio against a null engine; recreate it with the real
    // audio or every band reads 0 and the cube stops pulsing.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);

    // Expression1: smoothed bass drives the cube's uniform scale.
    const drive = soundCubeDrive(
      this.p9audio.band(-1, 1, 0, 0),
      this.pulseScale,
    );
    this.cubeGroup.scale.set(drive.scale, drive.scale, drive.scale);

    // Rotator1: degrees per second around each axis.
    this.rotation = advanceSoundCubeRotation(this.rotation, dt, this.spinScale);
    this.cubeGroup.rotation.set(
      this.rotation.x,
      this.rotation.y,
      this.rotation.z,
    );

    const feedback = soundCubeFeedback(this.trailScale);
    this.trailMaterial.uniforms.gTrailScale.value = feedback.brightness;
    this.compositeMaterial.uniforms.gBrightness.value = Number.isFinite(
      this.brightnessScale,
    )
      ? Math.max(0, this.brightnessScale)
      : 1;
  }

  get paintsCanvas() {
    return true;
  }

  get captureTexture() {
    return this._finalRT?.texture ?? null;
  }

  _renderFeedback() {
    const renderer = this.renderer;
    if (!renderer) return;

    // Blur the previous frame (StoreTexture1 -> Blur1, Dir 2 = H then V).
    renderer.autoClear = false;

    this.blurMaterial.uniforms.gTexture1.value = this._readRT.texture;
    this.blurMaterial.uniforms.gDir.value.set(1, 0);
    this.blurMaterial.uniforms.gTexel.value.set(1 / RTT_SIZE, 0);
    this._quad.material = this.blurMaterial;
    renderer.setRenderTarget(this._blurA);
    renderer.render(this._quadScene, this._ortho);

    this.blurMaterial.uniforms.gTexture1.value = this._blurA.texture;
    this.blurMaterial.uniforms.gDir.value.set(0, 1);
    this.blurMaterial.uniforms.gTexel.value.set(0, 1 / RTT_SIZE);
    renderer.setRenderTarget(this._blurB);
    renderer.render(this._quadScene, this._ortho);

    // RenderObject1 -> RenderToTexture1: clear and draw the cube.
    renderer.setRenderTarget(this._writeRT);
    renderer.setClearColor(0x000000, 0);
    renderer.clear(true, true, false);
    renderer.render(this.cubeGroup, this.camera);

    // Shader3: add the blurred, zoomed previous frame with alpha 0. The
    // separate blur targets are what make this a ping-pong loop instead of
    // reading the target currently being written.
    this.trailMaterial.uniforms.gTexture1.value = this._blurB.texture;
    this._quad.material = this.trailMaterial;
    renderer.render(this._quadScene, this._ortho);

    renderer.autoClear = true;

    // Swap: _readRT now holds this frame and is the next frame's source.
    const previous = this._readRT;
    this._readRT = this._writeRT;
    this._writeRT = previous;
  }

  _renderComposite() {
    const renderer = this.renderer;
    if (!renderer) return;

    const w = Math.max(1, Math.round(this.size.w));
    const h = Math.max(1, Math.round(this.size.h));
    if (this._finalRT.width !== w || this._finalRT.height !== h) {
      this._finalRT.setSize(w, h);
    }

    // Shader2: mix(bg, c, c.a) + c.
    this.compositeMaterial.uniforms.gTexture1.value = this._readRT.texture;
    this.compositeMaterial.uniforms.gTexture2.value =
      this._previousLayerTexture;
    this._quad.material = this.compositeMaterial;

    renderer.autoClear = false;
    renderer.setRenderTarget(this._finalRT);
    renderer.setClearColor(0x000000, 0);
    renderer.clear(true, false, false);
    renderer.render(this._quadScene, this._ortho);
    renderer.setRenderTarget(null);
    renderer.autoClear = true;
  }

  _present() {
    const renderer = this.renderer;
    if (!renderer || !this._finalRT) return;

    this.copyMaterial.uniforms.gTexture1.value = this._finalRT.texture;
    this._quad.material = this.copyMaterial;
    renderer.setRenderTarget(null);
    renderer.setClearColor(0x000000, CLEAR_ALPHA);
    renderer.autoClear = true;
    renderer.render(this._quadScene, this._ortho);
  }

  render() {
    if (!this.renderer) {
      super.render();
      return;
    }

    const capture = this._captureNextFrame;
    this._captureNextFrame = false;

    this._renderFeedback();
    this._renderComposite();
    if (!capture) this._present();
  }

  resize(w, h) {
    this.size = { w, h };
    this._applyCamera(w, h);
    if (this._finalRT) {
      this._finalRT.setSize(
        Math.max(1, Math.round(w)),
        Math.max(1, Math.round(h)),
      );
    }
  }
}
