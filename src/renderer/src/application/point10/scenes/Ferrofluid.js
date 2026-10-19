// Ferrofluid - the marbled-liquid look shot from above: a BOLD saturated
// colour field (yellow -> orange -> magenta) with BLACK marbled through it as
// the graphic shape, delicate cyan tracery inside the black, and glossy black
// beads beaded over the whole thing.
//
// The whole look is one heavily domain-warped fbm. The warp IS the trick - at
// warp ~1.7 the iso-contours are fat blobs, past ~3 they stretch into the
// ribbon labyrinth the reference reads as. Everything else is shading on top
// of that single field:
//   ink    -> the black ribbons (field BELOW threshold, so black is the bold
//             shape and colour is the field - not the other way round)
//   mass   -> a large-scale mask keeping the black concentrated centrally, so
//             colour dominates the edges like the reference composition
//   tracery-> zero-crossing contours of a finer field = the cyan hairlines
//   beads  -> Worley F1 cells = the round glossy droplets
//
// Audio (P9Audio bands, the plane9 semantics in AGENTS.md): bass DROPS the ink
// threshold, so a swell floods the colour field with black; it also churns the
// warp and pops the specular sheen. Beat is a transient sheen kick, mid/treble
// open the bead size and the speckle. Silence reads ~0 on every band, so the
// scene rests as a slow marbled drift - no fallback values anywhere.
import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../p10Audio.js";
import { dropColorPulse, equalizedHue } from "../shared/phraseColor.js";

const clamp01 = (v) => Math.max(0, Math.min(1, Number.isFinite(v) ? v : 0));

// The bold treatments the scene morphs between: marble ribbons, halftone
// stipple, glossy cells, wet tracery.
const LAYERS = 4;

// Pure audio -> visual levels, locked by scripts/ferrofluid-check.mjs.
export function ferrofluidLevels(bass = 0, mid = 0, high = 0, beat = 0) {
  const b = clamp01(bass),
    m = clamp01(mid),
    h = clamp01(high),
    k = clamp01(beat);
  return {
    // Warp strength - what turns noise blobs into the ribbon labyrinth.
    // Bass churns the marbling harder; a beat snaps it.
    warp: 1.2 + b * 1.2 + k * 0.6,
    // Ink threshold. LOWER = MORE black, so 0.50 (quiet) -> 0.35 (loud) is
    // the "black bulks out across the colour field" swell.
    coverage: 0.5 - b * 0.15,
    // Glossy specular pop, mostly on the transient.
    sheen: 0.3 + b * 0.45 + k * 1.0,
    // How fast the pattern flows through itself.
    flow: 0.05 + b * 0.1 + m * 0.04,
    // Bead density/scale - mid+treble tighten the Worley cells.
    droplet: 5.0 + m * 2.0 + h * 2.5,
    // How many of the beads actually show (a sparse set reads as liquid;
    // a dense one reads as noise).
    dots: clamp01(0.4 + m * 0.3 + h * 0.25),
    // Tracery + rim brightness.
    glow: clamp01(0.25 + b * 0.7 + k * 0.35),
  };
}

const GOLD_BASE = new THREE.Color(0xffd21e); // the yellow core of the field
const WARM_BASE = new THREE.Color(0xff6a12); // orange mid-field
const BG_BASE = new THREE.Color(0xff1250); // magenta outer field
const ACCENT_BASE = new THREE.Color(0x2fd4e8); // cyan tracery / rim light
// (INK lives as a GLSL const in the shader - it never changes.)

const VERT = /* glsl */ `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position, 1.0);
}
`;

const FRAG = /* glsl */ `
precision highp float;
uniform vec2 uResolution;
uniform float uTime;
uniform float uWarp;
uniform float uCoverage;
uniform float uSheen;
uniform float uFlow;
uniform float uDroplet;
uniform float uDots;
uniform float uGlow;
uniform float uLayer;
uniform vec3 uGold;
uniform vec3 uWarm;
uniform vec3 uBg;
uniform vec3 uAccent;
varying vec2 vUv;

// The black liquid. A GLSL const, not a uniform: it never changes.
const vec3 INK = vec3(0.016, 0.016, 0.039);

// Ashima simplex noise (MIT), prefixed to avoid clashing with the P9 prelude.
vec3 ffPermute(vec3 x) { return mod(((x * 34.0) + 1.0) * x, 289.0); }

float ffSnoise(vec2 v) {
  const vec4 C = vec4(0.211324865405187, 0.366025403784439,
                     -0.577350269189626, 0.024390243902439);
  vec2 i  = floor(v + dot(v, C.yy));
  vec2 x0 = v - i + dot(i, C.xx);
  vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod(i, 289.0);
  vec3 p = ffPermute(ffPermute(i.y + vec3(0.0, i1.y, 1.0))
                     + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
  m = m * m; m = m * m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 a0 = x - floor(x + 0.5);
  m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
  vec3 g;
  g.x  = a0.x * x0.x + h.x * x0.y;
  g.yz = a0.yz * x12.xz + h.yz * x12.yw;
  return 130.0 * dot(m, g);
}

// Rotate+scale per octave: without the rotation the lattice shows up as a
// grid of blobs in the marbling.
mat2 FF_ROT = mat2(1.62, 1.18, -1.18, 1.62);

float ffFbm(vec2 p, int oct) {
  float v = 0.0, a = 0.5;
  for (int i = 0; i < 4; i++) {
    if (i >= oct) break;
    v += a * ffSnoise(p);
    p = FF_ROT * p;
    a *= 0.5;
  }
  return v;
}

// Double domain warp -> the master marbled field, 0..1.
float ffField(vec2 p, float t, int oct) {
  vec2 fl = vec2(0.0, 1.0) * t;
  vec2 q = vec2(ffFbm(p + fl, oct),
                ffFbm(p + vec2(5.2, 1.3) - fl * 0.6, oct));
  vec2 r = vec2(ffFbm(p + uWarp * q + vec2(1.7, 9.2), oct),
                ffFbm(p + uWarp * q + vec2(8.3, 2.8), oct));
  return ffFbm(p + uWarp * r, oct) * 0.5 + 0.5;
}

vec2 ffHash2(vec2 p) {
  p = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
  return fract(sin(p) * 43758.5453);
}

float smin(float a, float b, float k) {
  float h = clamp(b / a, 0.0, 1.0);
  return mix(b, a, h) - k * h * (1.0 - h);
}

float smin2(float a, float b, float k) {
  float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
  return mix(b, a, h) - k * h * (1.0 - h);
}

float gauss(float x, float c, float w) {
  float u = (x - c) / w;
  return exp(-u * u);
}

float ffMound(vec2 p) { return ffSnoise(p * 1.05 + vec2(4.4, 8.8)) * 0.5 + 0.5; }

// Treatment tent weights (set in main, read by ffHeight).
float wR, wS, wD, wT, nrm;

// Mitosis field, taken from the shader.gallery example: four mirror-symmetric
// kernels per cell, eased split/refuse phases, smooth-min necking, and a snap
// ring after each pinch. This is the "blobs warping" part - the cell is not a
// static noise blob, it elongates, necks, and splits.
float gR, gK;

float ffMitosisField(vec2 p, float t) {
  float S = 0.18 * (3.5 / max(uDroplet, 1.0));
  gR = 0.62 * S;
  gK = 0.42;

  vec2 q = p / S + vec2(ffSnoise(p + 1.7), ffSnoise(p + 9.2)) * 0.55
         + vec2(0.0, t * 0.15);
  vec2 cell = floor(q), f = fract(q);
  float d = 1e9;

  for (int j = -1; j <= 1; j++) {
    for (int i = -1; i <= 1; i++) {
      vec2 hh = ffHash2(cell + vec2(float(i), float(j)));
      float ph = fract(t * 0.12 + hh.y * 0.7 + hh.x * 0.3);
      float sA = smoothstep(0.04, 0.17, ph) - smoothstep(0.83, 0.96, ph);
      float sB = smoothstep(0.30, 0.43, ph) - smoothstep(0.57, 0.70, ph);
      float envA = gauss(ph, 0.17, 0.05) + gauss(ph, 0.83, 0.05);
      float envB = gauss(ph, 0.43, 0.05) + gauss(ph, 0.57, 0.05);
      float wt = t * 6.2831853;
      float wobX = sin(wt * 1.7) * 0.10 * uWarp * envA;
      float wobY = sin(wt * 1.9 + 1.1) * 0.10 * uWarp * envB;

      float ox = sA * 1.02 + wobX;
      float oy = sB * 1.02 + wobY;
      float spread = clamp((sA + sB) / 1.6, 0.0, 1.0);
      float R = 0.62 - 0.18 * spread;

      vec2 r = vec2(float(i), float(j)) + hh - f;
      vec2 c0 = hh + vec2(-ox, -oy);
      vec2 c1 = hh + vec2( ox, -oy);
      vec2 c2 = hh + vec2(-ox,  oy);
      vec2 c3 = hh + vec2( ox,  oy);

      float a = smin2(length(r - c0) - R, length(r - c1) - R, gK);
      float b = smin2(length(r - c2) - R, length(r - c3) - R, gK);
      d = smin(d, smin2(a, b, gK), 0.15);
    }
  }

  return d * S;
}

float ffBlobs(vec2 p, float t) {
  float h = clamp(-ffMitosisField(p, t) / max(gR, 1e-4), 0.0, 1.0);
  return h * (0.40 + 0.80 * ffMound(p));
}

// Height of the liquid at p under the current treatment mix. This is the
// single source of truth: the black mask AND the normal both come from it, so
// black appears exactly where the liquid is actually raised. That is what
// makes the frame read as a poured 3D surface instead of a printed pattern -
// the previous version shaded a mask it had no idea was raised.
float ffHeight(vec2 p, float t) {
  float g = ffField(p, t, 2);
  float hR = clamp((g - uCoverage) * 3.2, 0.0, 1.0);
  float dF = ffSnoise(p * 30.0 + vec2(7.3, 2.9)) * 0.5 + 0.5;
  float dS = clamp(dF * 0.7 + g * 0.5, 0.0, 1.0);
  float sTh = mix(0.74, 0.50, uDots);
  float hS = clamp((dS - sTh) * 3.0, 0.0, 1.0) * 0.30;
  // uDots lowers the rise gate (more of the bumps actually show).
  float hD = clamp((ffBlobs(p, t) - mix(0.35, 0.10, uDots)) * 2.0, 0.0, 1.0);
  return (hR * (wR + wT) + hS * wS + hD * wD) / nrm;
}

void main() {
  vec2 uv = (gl_FragCoord.xy * 2.0 - uResolution) / uResolution.y;
  // 0.5, not 1.x: the reference ribbons are fat (~2% of the frame across)
  // separated by broad colour channels - a higher base frequency turns the
  // same math into busy thin filaments.
  vec2 p = uv * 0.5;
  float t = uTime * uFlow;
  // Regional swell: a slow low-frequency mound so whole areas of the liquid
  // sit much higher than others. Multiplies every relief term below.
  float mound = ffMound(p);
  float liftH = 0.60 + 1.70 * mound;

  // ---- bold marbled colour patches --------------------------------------
  // The colour regions are themselves marbled and bounded by the black
  // ribbons. A flat radial wash reads as a background, not as liquid, so the
  // palette is picked by a SECOND (coarser, slower) warped field - the two
  // fields disagreeing is what gives distinct colour zones with black borders.
  float cp = ffField(p * 0.62 + vec2(21.0, 5.0), t * 0.6, 2);
  vec3   fieldCol = mix(uGold, uWarm, smoothstep(0.34, 0.62, cp));
  fieldCol = mix(fieldCol, uBg, smoothstep(0.56, 0.86, cp));
  fieldCol *= 0.45;

  // Tent weights over uLayer: integer = one treatment, fractional = the morph
  // between two. They always sum to 1, so the composite stays balanced. Set
  // before the height taps because ffHeight reads them.
  wR = max(0.0, 1.0 - abs(uLayer - 0.0));
  wS = max(0.0, 1.0 - abs(uLayer - 1.0));
  wD = max(0.0, 1.0 - abs(uLayer - 2.0));
  wT = max(0.0, 1.0 - abs(uLayer - 3.0));
  nrm = max(wR + wS + wD + wT, 1e-4);

  // ---- the surface: height, then light it --------------------------------
  // 4 height taps (the point, two for the gradient, one toward the light for
  // the cast shadow). Everything below is shading on that one height field,
  // so the black, the tilt and the shadow all agree about where the liquid is
  // actually raised - which is the whole 3D read.
  float e = 0.012;
  float h0 = ffHeight(p, t);
  float hx = ffHeight(p + vec2(e, 0.0), t);
  float hy = ffHeight(p + vec2(0.0, e), t);
  vec3 Ld = normalize(vec3(-0.55, 0.75, 0.40));
  // Cast shadow: a raised point blocks the light for the ground behind it, and
  // the taller it is the farther the block reaches. This directional shadow is
  // the strongest "sitting above the surface" cue there is.
  float hs = ffHeight(p + Ld.xy * (0.02 + 0.12 * h0), t);
  float ao = 1.0 - smoothstep(0.0, 0.30, (h0 + hx + hy) / 3.0 - h0);   // crevices darker
  float sh = smoothstep(0.0, 0.02, h0 - hs);                            // blocked from the light
  vec3 n = normalize(vec3((h0 - hx) / e * liftH, (h0 - hy) / e * liftH, 1.0));
  float diff = max(dot(n, Ld), 0.0);
  // Tilted view, not straight down. Looking straight down the plane makes every
  // blob top face the camera, so fresnel is 0 there and the top stays flat
  // black - only the grazing edges lit up, which reads as a ring around each
  // blob. A tilted V is what puts the reflection on the tops.
  vec3 V = normalize(vec3(0.30, -0.50, 0.80));
  vec3 R = reflect(-Ld, n);
  float spec = pow(max(dot(R, V), 0.0), 50.0);
  float fres = pow(1.0 - max(dot(n, V), 0.0), 2.0);

  // Black = raised. The mask is the height itself, so the bulk of the liquid
  // reads as solid mass and the low ground between domes stays colour.
  float rad = length(uv);
  float feat = clamp(smoothstep(0.10, 0.34, h0), 0.0, 1.0);
  feat *= mix(0.72, 1.0, smoothstep(1.5, 0.4, rad));
  float domeOnly = wD / nrm;
  float tracAmt = wT / nrm;

  // Glossy black. The reference reads as 3D because the black REFLECTS its
  // surroundings. A cheap gradient environment (warm above, deep magenta below)
  // sampled by the reflected normal carries the whole reflection - an env-map
  // texture is the upgrade path, but for a top-down view a two-colour ramp is
  // the same information. Peaks catch more of it, so the tall spots read as
  // tall.
  float peak = smoothstep(0.25, 1.0, h0);
  vec3 env = mix(uBg, uGold, smoothstep(-0.5, 0.7, R.y));
  vec3 base = mix(fieldCol * (0.35 + 0.35 * diff) * (0.70 + 0.30 * ao),
                  INK * (0.25 + 1.10 * diff) * ao, feat);
  base *= 0.62 + 0.38 * sh;
  vec3 outCol = base
              + env * fres * (0.25 + 0.45 * uGlow) * (0.35 + 0.65 * peak) * feat
              + vec3(1.0, 0.98, 0.92) * spec * uSheen * feat * (0.30 + 0.85 * peak);
  // A little reflection on the flat field too, so it reads as wet, not painted.
  outCol += env * fres * (0.20 + 0.35 * uGlow) * 0.18 * (1.0 - feat);
  // One warm bleed on the active feature's edge.
  float fEdge = feat * (1.0 - feat) * 4.0;
  outCol += uWarm * fEdge * 0.20;

  // Cyan tracery, only during the wet-tracery treatment.
  float trac = ffFbm(p * 11.0 + vec2(3.3, 7.7) + uWarp * 0.2, 3);
  float lines = 1.0 - smoothstep(0.0, 0.075, abs(trac));
  outCol += uAccent * lines * feat * tracAmt * (0.45 + 0.75 * uGlow);

  // Beaded rim, only while the domes dominate. Kept weak: a strong accent ring
  // around each blob reads as a colony outline, not as a lit edge.
  float beadN = ffSnoise(p * uDroplet * 8.0 + 3.3) * 0.5 + 0.5;
  float dEdge = h0 * (1.0 - h0) * 4.0 * domeOnly;
  outCol += uAccent * pow(dEdge, 1.6) * (0.40 + 0.60 * smoothstep(0.35, 0.65, beadN))
          * (0.20 + 0.25 * uGlow);

  // ---- vignette + grain --------------------------------------------------
  outCol *= 1.0 - 0.28 * smoothstep(0.6, 1.8, rad);
  outCol += (fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453) - 0.5) * 0.02;

  gl_FragColor = vec4(max(outCol, 0.0), 1.0);
}
`;

export class Ferrofluid extends BaseScene {
  _init() {
    this.label = "Ferrofluid";
    this.category = "Other";
    this.p9audio = new P9Audio(this.audio);
    this._bass = 0;
    this._mid = 0;
    this._high = 0;
    this._beat = 0;
    this._phrase = { hue: 0, t: 0 };
    this._phraseHue = 0;
    // Which bold treatment is on screen (0..LAYERS-1) and where we are
    // morphing to. A phrase change advances the treatment and the shader
    // interpolates the masks - the looks replace each other rather than
    // stacking up.
    this._layer = 0;
    this._layerTarget = 0;
    this._accent = new THREE.Color();
    this._gold = new THREE.Color();
    this._warm = new THREE.Color();
    this._deep = new THREE.Color();
    this._rw = 0;
    this._rh = 0;
    const lv = ferrofluidLevels();
    this.uniforms = {
      uResolution: { value: new THREE.Vector2(1, 1) },
      uTime: { value: 0 },
      uWarp: { value: lv.warp },
      uCoverage: { value: lv.coverage },
      uSheen: { value: lv.sheen },
      uFlow: { value: lv.flow },
      uDroplet: { value: lv.droplet },
      uDots: { value: lv.dots },
      uGlow: { value: lv.glow },
      uLayer: { value: 0 },
      uGold: { value: new THREE.Color().copy(GOLD_BASE) },
      uWarm: { value: new THREE.Color().copy(WARM_BASE) },
      uBg: { value: new THREE.Color().copy(BG_BASE) },
      uAccent: { value: new THREE.Color().copy(ACCENT_BASE) },
    };
    this.mesh = new THREE.Mesh(
      new THREE.PlaneGeometry(2, 2),
      new THREE.ShaderMaterial({
        vertexShader: VERT,
        fragmentShader: FRAG,
        uniforms: this.uniforms,
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.mesh.frustumCulled = false;
    this.group.add(this.mesh);
  }

  setAudio(audio) {
    super.setAudio(audio);
    this.p9audio = new P9Audio(audio);
  }

  update(dt) {
    super.update(dt);
    this.p9audio?.update(dt);
    // hasAudio gates every read: no stream means the frame rests, and a
    // live-but-quiet stream reads ~0 (never a fallback).
    const live = !!this.p9audio?.hasAudio;
    const target = (band, prev, rate) => {
      const v = live ? clamp01(this.p9audio.band(-1, 1, band)) : 0;
      return prev + (v - prev) * Math.min(1, dt * rate);
    };
    this._bass = target(0, this._bass, 6);
    this._mid = target(1, this._mid, 10);
    this._high = target(2, this._high, 12);
    const rawBeat = live ? clamp01(this.p9audio.beat()) : 0;
    this._beat += (rawBeat - this._beat) * Math.min(1, dt * 18);

    const lv = ferrofluidLevels(this._bass, this._mid, this._high, this._beat);
    const u = this.uniforms;
    u.uTime.value = this.time;
    u.uWarp.value = lv.warp;
    u.uCoverage.value = lv.coverage;
    u.uSheen.value = lv.sheen;
    u.uFlow.value = lv.flow;
    u.uDroplet.value = lv.droplet;
    u.uDots.value = lv.dots;
    u.uGlow.value = lv.glow;

    // Phrase hue drives the WHOLE palette, not just the accent: the source
    // swings yellow -> magenta -> teal over the track, so gold/warm/bg all
    // rotate together by one angle while black stays black. That is what
    // makes the field read as bold colour that changes per section rather
    // than one fixed tint. The same phrase change advances the treatment.
    const wasHue = this._phrase.hue;
    this._phrase = dropColorPulse(this._phrase, this._bass, dt);
    if (this._phrase.hue !== wasHue) {
      this._layerTarget = (this._layerTarget + 1) % LAYERS;
    }
    // Shortest way round the ring (layers are evenly spaced 0..3).
    let ld = this._layerTarget - this._layer;
    if (ld > LAYERS / 2) ld -= LAYERS;
    else if (ld < -LAYERS / 2) ld += LAYERS;
    this._layer += ld * Math.min(1, dt * 0.8);
    u.uLayer.value = this._layer;

    let delta = this._phrase.hue - this._phraseHue;
    delta = Math.atan2(Math.sin(delta), Math.cos(delta));
    this._phraseHue += delta * Math.min(1, dt * 0.55);
    const lift = 0.8 + lv.glow * 0.45;
    equalizedHue(GOLD_BASE, this._phraseHue, this._gold);
    equalizedHue(WARM_BASE, this._phraseHue, this._warm);
    equalizedHue(BG_BASE, this._phraseHue, this._deep);
    equalizedHue(ACCENT_BASE, this._phraseHue, this._accent);
    u.uGold.value.copy(this._gold).multiplyScalar(lift);
    u.uWarm.value.copy(this._warm).multiplyScalar(lift);
    u.uBg.value.copy(this._deep).multiplyScalar(0.8 + lv.glow * 0.55);
    u.uAccent.value.copy(this._accent);

    this._syncResolution();
  }

  _syncResolution() {
    const canvas = this.renderer?.domElement;
    const w = canvas?.width || 1;
    const h = canvas?.height || 1;
    if (w !== this._rw || h !== this._rh) {
      this._rw = w;
      this._rh = h;
      this.uniforms.uResolution.value.set(w, h);
    }
  }

  resize() {
    this._rw = 0;
    this._rh = 0;
    this._syncResolution();
  }

  render() {
    if (!this.renderer) return;
    this._syncResolution();
    super.render();
  }
}
