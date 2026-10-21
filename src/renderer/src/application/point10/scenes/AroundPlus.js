// Around Plus: the Plane9 "Around And Around" preset (Cell) ported to a
// standalone native scene - no XML, no P9Scene node graph. One inverted
// torus (the tube the camera sits inside) textured with ambientCG Concrete044C,
// 8 glow rings around the tube (unreal-bloom example chain), and the camera
// wander: MinMax pitch + noise roll on top of a fixed lookAt, torus
// spinning with the mid band. Faithful to P9Nodes' Torus build (tube phase
// + rotateX so the ring is horizontal, uv rescale) and InvertMesh winding.
//
// modV port of Point10/src/scenes/AroundPlus.js. The worker has no document,
// so the two browser TextureLoaders are replaced with the Vite ?url assets
// loaded through ../shared/textures.js; the scene keeps its own composer and
// Water rig exactly as upstream.

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { valueNoise } from "../plane9/P9Expression.js";
import { COMMON_PREAMBLE } from "../plane9/P9Glsl.js";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";
import { Water } from "three/examples/jsm/objects/Water.js";
// Phrase colour flip + CPU ROYGBIV application, shared with SmokeRoom /
// AttractorParticles / RollingFire.
import { dropColorPulse, equalizedHue } from "../shared/phraseColor.js";
import { WaveSim } from "../shared/waterWave.js";
import { loadTexture } from "../shared/textures.js";
import concreteUrl from "../textures/concrete044c_col_1k.jpg?url";
import waterNormalsUrl from "../textures/waternormals.jpg?url";

// The exact line in Water.js whose result we perturb (smoke locks this).
const WAVE_ANCHOR =
  "vec3 surfaceNormal = normalize( noise.xzy * vec3( 1.5, 1.0, 1.5 ) );";

const DEG = Math.PI / 180;

// Water fills the tube's bottom eighth: the tube spans y -1..1 (local), so
// the surface sits at -0.75. At that depth the cross-section chord is
// 2*sqrt(1-y^2) wide, so a flat annulus between those radii covers exactly
// the filled region of the curved tube.
export const WATER_LEVEL = -0.75;
export const WATER_INNER = 5 - Math.sqrt(1 - WATER_LEVEL * WATER_LEVEL);
export const WATER_OUTER = 5 + Math.sqrt(1 - WATER_LEVEL * WATER_LEVEL);

// Bass drive for the water (pure; smoke-locked): bass churns the waves
// faster, shakes the reflections harder and swells the waterline - calm in
// silence. The one-pole (dt*10) gives kicks a fast attack, slow release.
export const WATER_DRIVE = {
  speed: 0.8,
  churn: 2.0,
  distortion: 2.0,
  shake: 6.0,
  swell: 0.1,
};
export function waterUniforms(bass, smooth, dt) {
  const s = smooth + (bass - smooth) * Math.min(1, dt * 10);
  return {
    smooth: s,
    dtTime: dt * (WATER_DRIVE.speed + s * WATER_DRIVE.churn),
    distortionScale: WATER_DRIVE.distortion + s * WATER_DRIVE.shake,
    level: WATER_LEVEL + Math.min(WATER_DRIVE.swell, s * 0.3),
  };
}

// Map a world point on the water annulus to the wave sim's unit square:
// angle -> u with the seam at ring angle 0 (+X, hidden under glow ring 0;
// the sim texture wraps in u so waves cross it cleanly), radius -> v clamped
// to [0,1] (waves reflect off the tube walls).
export function annulusUv(x, z) {
  const u = Math.atan2(z, x) / (Math.PI * 2);
  return new THREE.Vector2(
    u - Math.floor(u),
    (Math.hypot(x, z) - WATER_INNER) / (WATER_OUTER - WATER_INNER),
  );
}
const WAVE_RES = 256;
// Injected into Water's fragment shader: sample the heightfield at the
// fragment's polar coords and turn its gradient into a normal bend, so light,
// reflections and refraction all move with the propagating waves.
const WAVE_GLSL = `
uniform sampler2D uWaveTex;
vec3 waveSlope(vec2 p) {
  vec2 uv = vec2(atan(p.y, p.x) / 6.2831853, (length(p) - ${WATER_INNER.toFixed(6)}) / ${(WATER_OUTER - WATER_INNER).toFixed(6)});
  vec2 d = vec2(${(1 / WAVE_RES).toFixed(8)}, 0.0);
  float h = texture2D(uWaveTex, uv).r;
  float hu = texture2D(uWaveTex, uv + d).r - h;
  float hv = texture2D(uWaveTex, uv + d.yx).r - h;
  vec2 radial = normalize(p);
  vec2 tangent = vec2(-radial.y, radial.x);
  vec2 s = tangent * hu + radial * hv;
  return vec3(s.x, 0.0, s.y) * 60.0;
}
`;

// Around And Around's Torus1 (Radius 5, Thickness 1, 256x45) with the same
// treatment P9Nodes gives it: tube cross-section phased +90deg (the geometry
// seam lands at the tube bottom, hidden by the shader's dark term), ring
// rotated into the XZ plane (hole up, so the ring passes through the camera
// at every spin angle), uvs rescale (ScaleU on the ring, ScaleV on the
// tube), and the InvertMesh winding flip (camera sits inside the tube).
export function buildAroundTorus() {
  const R = 5,
    r = 1,
    subAxis = 256,
    subHeight = 45;
  const arc = Math.PI * 2,
    phase = Math.PI / 2;
  const positions = new Float32Array((subAxis + 1) * (subHeight + 1) * 3);
  const normals = new Float32Array((subAxis + 1) * (subHeight + 1) * 3);
  const uvs = new Float32Array((subAxis + 1) * (subHeight + 1) * 2);
  const indices = [];
  let p = 0,
    u = 0;
  for (let i = 0; i <= subHeight; i++) {
    const v = (i / subHeight) * Math.PI * 2 + phase;
    const cv = Math.cos(v),
      sv = Math.sin(v);
    const ringR = R + r * cv;
    for (let j = 0; j <= subAxis; j++) {
      const a = (j / subAxis) * arc;
      const ca = Math.cos(a),
        sa = Math.sin(a);
      positions[p] = ringR * ca;
      positions[p + 1] = ringR * sa;
      positions[p + 2] = r * sv;
      normals[p] = cv * ca;
      normals[p + 1] = cv * sa;
      normals[p + 2] = sv;
      uvs[u] = j / subAxis;
      uvs[u + 1] = i / subHeight;
      p += 3;
      u += 2;
    }
  }
  for (let j = 1; j <= subAxis; j++) {
    for (let i = 1; i <= subHeight; i++) {
      const a = i * (subAxis + 1) + (j - 1);
      const b = (i - 1) * (subAxis + 1) + (j - 1);
      const c = (i - 1) * (subAxis + 1) + j;
      const d = i * (subAxis + 1) + j;
      indices.push(a, b, d, b, c, d);
    }
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  g.setAttribute("normal", new THREE.BufferAttribute(normals, 3));
  g.setAttribute("uv", new THREE.BufferAttribute(uvs, 2));
  g.setIndex(indices);
  g.rotateX(Math.PI / 2);
  const su = 12.64,
    sv = 33.19;
  const uv = g.attributes.uv;
  for (let i = 0; i < uv.count; i++)
    uv.setXY(i, (1 - uv.getY(i)) * su, (1 - uv.getX(i)) * sv);
  const idx = [...g.index.array];
  for (let i = 0; i + 2 < idx.length; i += 3)
    [idx[i], idx[i + 2]] = [idx[i + 2], idx[i]];
  g.setIndex(idx);
  return g;
}

// The MinMax1 port: a smoothed random wander between min/max, re-rolled every
// 3..20 s (ITimeMin..ITimeMax). Integrated toward the target so the value
// lands exactly when the interval ends - no jumps. Replica of NODES.MinMax.
export function makePitchWander(min, max, itMin = 3, itMax = 20) {
  const st = { t: 0, dur: 0, dir: 1, val: null, last: null };
  // eslint-disable-next-line no-unused-vars
  return (time, dt) => {
    const now = time;
    if (st.val === null) {
      st.val = min;
      st.last = now;
      st.dur = itMin + Math.random() * Math.max(0, itMax - itMin);
      st.t = now + st.dur;
    }
    const ddt = Math.min(0.05, Math.max(0, now - st.last));
    st.last = now;
    if (now >= st.t) {
      st.dir = -st.dir;
      st.dur = itMin + Math.random() * Math.max(0, itMax - itMin);
      st.t = now + st.dur;
    }
    const target = st.dir > 0 ? max : min;
    const remain = Math.max(0.001, st.t - now);
    st.val = Math.min(
      max,
      Math.max(min, st.val + ((target - st.val) / remain) * ddt),
    );
    return st.val;
  };
}

const VERT = `
varying vec3 v_wpos;
varying vec3 v_normal;
varying vec2 v_tex;
varying vec4 v_diffuse;
void main() {
  vec4 v = vec4(position, 1.0);
  v_wpos = (modelMatrix * v).xyz;
  gl_Position = projectionMatrix * modelViewMatrix * v;
  v_normal = mat3(transpose(inverse(modelMatrix))) * normal;
  v_tex = uv;
  v_diffuse = vec4(abs(length(position) - 2.7));
}
`;

// some three builds emit `#define saturate( a ) clamp( a, 0.0, 1.0 )` in the
// ShaderMaterial prefix (the vite-bundled cache served the unguarded version):
// a real `float saturate(...)` function there macro-expands into garbage and
// the fragment fails to compile. Guard the prelude's definitions with three's
// own `#ifndef saturate` idiom - if the macro exists the calls use it (same
// clamp behavior), if not the functions compile.
const SAT_DEFS = /((?:float|vec[234])\s+saturate\([^)]* v\)\s*\{[^}]*\}\n){4}/;
const PRELUDE = COMMON_PREAMBLE.replace(
  SAT_DEFS,
  (m) => `#ifndef saturate\n${m}#endif\n`,
);

export function buildAroundFragment() {
  return `
precision highp float;
uniform vec4 gIn2;
uniform sampler2D uMap;
varying vec3 v_wpos;
varying vec3 v_normal;
varying vec2 v_tex;
varying vec4 v_diffuse;
${PRELUDE}
void main() {
  // Concrete044C albedo (ambientCG, CC0); the rescaled uvs tile it around the tube.
  vec3 col = texture2D(uMap, v_tex).rgb * v_diffuse.rgb;

  // Large scale variations
  col *= vec3(0.5 + _noise(v_tex * 0.2) * 0.5);

  // Line - gIn2.x is band*50. ponytail: Reinhard soft knee (asymptote 6,
  // half-point at band 0.1) instead of a clamp - the hard cap saturated at
  // band 0.08, making bass all-or-nothing. This keeps every level distinct
  // (slope 1.2 at silence, still rising at band 0.5) while bounding the
  // side-wall bloom feed to ~6x threshold.
  float flash = gIn2.x * 6.0 / (gIn2.x + 5.0);
  col = mix(col, col + vec3(flash), pow(1.0 - dot(abs(v_normal), vec3(0.0, 1.0, 0.0)), 8.0));

  // Dark top bottom
  float x = dot(abs(v_normal), normalize(vec3(1.0, 0.0, 1.0)));
  col *= vec3(x * x);

  gl_FragColor = vec4(col, 1.0);
}
`;
}

function placeholderTexture(data, width = 1, height = 1) {
  const texture = new THREE.DataTexture(
    new Uint8Array(data),
    width,
    height,
    THREE.RGBAFormat,
  );
  texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
  texture.needsUpdate = true;
  return texture;
}

export class AroundPlus extends BaseScene {
  _init() {
    this.label = "Around Plus";
    // The Plane9 "Around And Around" preset is structured as a thin
    // horizontal torus (Radius 5 x Thickness 1) with the camera sitting inside
    // the tube at radius 5 - the wall the preset's voronoi bricks covered,
    // now the Concrete044C albedo. Gray 1x1 placeholder until the jpg lands;
    // the worker loads it through a Vite ?url asset instead of TextureLoader.
    this.torus = new THREE.Mesh(
      buildAroundTorus(),
      new THREE.ShaderMaterial({
        vertexShader: VERT,
        fragmentShader: buildAroundFragment(),
        uniforms: {
          gIn2: { value: new THREE.Vector4(0, 0, 0, 0) },
          uMap: { value: placeholderTexture([128, 128, 128, 255]) },
        },
      }),
    );
    const maxAniso = this.renderer?.capabilities?.getMaxAnisotropy?.() ?? 8;
    loadTexture(concreteUrl)
      .then((tex) => {
        tex.colorSpace = THREE.SRGBColorSpace;
        // uvs are rescaled to 12.64 x 33.19 - sampling past 1 needs repeat
        tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
        tex.anisotropy = maxAniso;
        this.torus.material.uniforms.uMap.value = tex;
      })
      .catch((error) => {
        console.warn("AroundPlus concrete texture failed:", error);
      });
    this.torus.frustumCulled = false;
    this.group.add(this.torus);

    // 8 glow rings around the tube (the unreal_bloom example's look). Ring 0
    // sits on the texture seam - the u wrap lands at ring angle 0 (+X axis,
    // where j=0 meets j=subAxis) - and the rest follow every 45deg. Children
    // of the torus so they ride its spin and keep covering that seam.
    this.ringMat = new THREE.MeshBasicMaterial({ toneMapped: false });
    const ringGeo = new THREE.TorusGeometry(1.06, 0.07, 12, 96);
    this.rings = [];
    for (let k = 0; k < 8; k++) {
      const a = (k / 8) * Math.PI * 2;
      const ring = new THREE.Mesh(ringGeo, this.ringMat);
      ring.position.set(Math.cos(a) * 5, 0, Math.sin(a) * 5);
      ring.rotation.y = -a;
      this.torus.add(ring);
      this.rings.push(ring);
    }
    // Fire-amber base pushed past 1.0 so only the rings (not the concrete
    // wall) cross the bloom threshold; hue comes from the phrase flip below.
    // ponytail: 1.8 + strength 0.9 is the white-out tune - rings at 2.2 with
    // strength 1.5 blew out together with the line flash; raise only together.
    this._ringBase = new THREE.Color(0xffe68c).multiplyScalar(1.8);
    equalizedHue(this._ringBase, 0, this.ringMat.color);

    // Phrase colour state: RollingFire's drop detector + eased hue.
    this._pulse = { hue: 0, t: 0 };
    this._ringHue = 0;

    // Water in the tube's bottom eighth (threejs-water's look: Fresnel
    // reflections over scrolling normal-map waves). The annulus is exact for
    // the filled region; a child of the torus so it rides the spin (the spin
    // axis is Y, so the level stays put). The normal map loads from a Vite
    // asset; a flat placeholder keeps the shader valid until it arrives.
    this._waterGeo = new THREE.RingGeometry(WATER_INNER, WATER_OUTER, 128, 4);
    this._waterGeo.rotateX(-Math.PI / 2);
    this._wave = null;
    if (this.renderer) {
      this.water = new Water(this._waterGeo, {
        textureWidth: 512,
        textureHeight: 512,
        waterNormals: placeholderTexture([128, 128, 255, 255]),
        sunDirection: new THREE.Vector3(0, 1, 0),
        waterColor: 0x001e0f,
        distortionScale: WATER_DRIVE.distortion,
        alpha: 0.6,
      });
      // Water.js never sets transparent itself - without it the alpha
      // uniform is ignored and the surface stays opaque.
      this.water.material.transparent = true;
      loadTexture(waterNormalsUrl)
        .then((tex) => {
          tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
          this.water.material.uniforms.normalSampler.value = tex;
        })
        .catch((error) => {
          console.warn("AroundPlus water normals failed:", error);
        });
      // Inject the wave heightfield: its gradient bends reflections,
      // refraction and specular (see WAVE_GLSL). The uniform value is
      // re-pointed at the current ping-pong texture every frame in update().
      this._wave = new WaveSim(this.renderer, WAVE_RES);
      this.water.material.onBeforeCompile = (shader) => {
        shader.uniforms.uWaveTex = { value: this._wave.texture };
        this._waveShader = shader;
        shader.fragmentShader =
          WAVE_GLSL +
          shader.fragmentShader.replace(
            WAVE_ANCHOR,
            "vec3 surfaceNormal = normalize( noise.xzy * vec3( 1.5, 1.0, 1.5 ) + waveSlope(worldPosition.xz) );",
          );
      };
      this.water.position.y = WATER_LEVEL;
      this.torus.add(this.water);
    }
    this._bassSm = 0;
    this._wkPrev = 0;
    this._wkArmed = true;

    // The example's chain: RenderPass -> UnrealBloomPass -> OutputPass.
    // Threshold 1.0 keeps the concrete wall out of the bloom.
    if (this.renderer) {
      this.composer = new EffectComposer(this.renderer);
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.addPass(new RenderPass(this.group, this.camera));
      this.composer.addPass(
        new UnrealBloomPass(
          this.renderer.getSize(new THREE.Vector2()),
          0.9,
          0.4,
          1.0,
        ),
      );
      this.composer.addPass(new OutputPass());
    }

    this.p9audio = new P9Audio(this.audio);
    this.perm = 1; // Expression1 state starts at RandomSeed=1
    this._wander = makePitchWander(-10, 10);
    this.size = { w: 2, h: 2 };
    if (this.renderer) {
      const s = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: s.x, h: s.y };
    }
  }

  setAudio(audio) {
    super.setAudio(audio);
    // fit P9Scene: _init built p9audio against a null engine, so it must be
    // (re)created with the real audio or bands read 0 and the glow/spin die.
    this.p9audio = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9audio.update(dt);
    // Shader1 expression: in2.x = band(-1,1,0,0)*50 -> the "Line" flash.
    this.torus.material.uniforms.gIn2.value.set(
      this.p9audio.band(-1, 1, 0) * 50,
      0,
      0,
      0,
    );
    // Expression1: perm += min(1, band(-1,1,1,0.05)*10+0.1)*dt*20; out1=perm
    // -> RenderObject1.Rot.y (degrees).
    this.perm += Math.min(1, this.p9audio.band(-1, 1, 1) * 10 + 0.1) * dt * 20;
    this.torus.rotation.y = this.perm * DEG;

    // Ring colour on musical phrases: the bass drop detector flips the
    // ROYGBIV hue at the crest (same constants as RollingFire), then the
    // displayed hue eases there along the shortest arc.
    this._pulse = dropColorPulse(this._pulse, this.p9audio.bands[0], dt);
    let dh = this._pulse.hue - this._ringHue;
    dh = Math.atan2(Math.sin(dh), Math.cos(dh)); // shortest hue arc (hue is 2π-periodic)
    this._ringHue += dh * Math.min(1, dt * 1.2);
    equalizedHue(this._ringBase, this._ringHue, this.ringMat.color);

    // Water vibrates with the bass: faster churn, shakier reflections and a
    // slight waterline swell (waterUniforms; calm in silence).
    const bass = this.p9audio.bands[0];
    if (this.water) {
      const w = waterUniforms(bass, this._bassSm, dt);
      this._bassSm = w.smooth;
      this.water.material.uniforms.time.value += w.dtTime;
      this.water.material.uniforms.distortionScale.value = w.distortionScale;
      this.water.position.y = w.level;
    }

    // Kicks disturb the heightfield: fire on the bass ATTACK (a sharp rise,
    // never its absolute level), drop size rides the jump.
    // The sim then propagates the wavefronts around the annulus for free.
    const jump = bass - this._wkPrev;
    if (jump <= 0) this._wkArmed = true;
    if (this._wkArmed && jump > 0.06) {
      this._wave?.drop(
        Math.random(),
        0.3 + Math.random() * 0.4,
        Math.min(0.05, jump * 0.07),
      );
      this._wkArmed = false;
    }
    this._wkPrev = bass;
    if (this._wave) {
      this._wave.step();
      if (this._waveShader)
        this._waveShader.uniforms.uWaveTex.value = this._wave.texture;
    }

    // Screen1 camera: CamPos -5 0 0, look at 0 0 -1 (relative), then the
    // MinMax pitch (Vector2.x) and the roll from Expression2
    // (out1 = noise1(time*0.01)*5, Vector2.z).
    const pitch = this._wander(this.time, dt);
    this.pitch = pitch;
    const roll = valueNoise(1, this.time * 0.01) * 5;
    let sw = this.size.w,
      sh = this.size.h;
    if (sw <= 2 && sh <= 2 && this.renderer) {
      const buf = this.renderer.getDrawingBufferSize(new THREE.Vector2());
      sw = buf.x;
      sh = buf.y;
    }
    const cam = this.camera;
    cam.fov = 60.5;
    cam.near = 0.1;
    cam.far = 1000;
    cam.aspect = sw / Math.max(1, sh);
    cam.position.set(-5, 0, 0);
    cam.lookAt(-5, 0, -1);
    cam.rotateX(pitch * DEG);
    cam.rotateY(0);
    cam.rotateZ(roll * DEG);
    cam.updateMatrixWorld();
    cam.matrixWorldInverse.copy(cam.matrixWorld).invert();
    cam.updateProjectionMatrix();
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
