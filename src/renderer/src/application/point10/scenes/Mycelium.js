import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";
import { P9Audio } from "../p10Audio.js";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { dropColorPulse, hueRotate } from "../shared/phraseColor.js";

// Mycelium Strand - hyphal colony spreading from the screen centre across dark
// substrate (Sceneideas #1), fragment shader ported from shader.gallery/mycelium
// (MIT, E. T. Carter). The reference runs on fixed params; here the colony is
// music-driven: growth speed <- bass, wander <- bass+beat, twigs <- mid, runner
// density <- high, bioluminescence <- mid/high+beat, and a kick gates a narrow
// pulse band sweeping out along the strands. New generations keep seeding every wave
// (floor(t)) - the repeating strands. Bands via the damping-1 lane (0.05 s,
// responsive like the Hollow Cube explosion read).
const VERT = `
attribute vec3 position;

void main() {
  gl_Position = vec4(position, 1.0);
}
`;

const FRAG = `
precision highp float;

uniform vec2  u_resolution;
uniform float u_t;       // wave clock (integrated growth, one wave per 32 s)
uniform float u_tp;      // pulse clock (same integrated clock in seconds)
uniform float u_scale;   // runner density knob
uniform float u_twigs;   // lateral twig density knob
uniform float u_glow;    // bioluminescence, driven by mid/high + beat
uniform float u_wander;  // curve/wander knob
uniform float u_beat;    // beat envelope 0..1
uniform float u_pixelRatio;
uniform vec3  u_c0;
uniform vec3  u_c1;
uniform vec3  u_c2;
uniform vec3  u_c3;
uniform vec3  u_fogColor;
uniform float u_fogDensity;

const float TAU = 6.28318530718;
const vec2  ORIGIN = vec2(0.0, 0.0);       // colony origin, screen centre
const float RMIN = 0.05;                   // radius where the centre starts
const float RMAX = 1.25;                   // radius past the far corners

float hash21(vec2 p) {
  p = fract(p * vec2(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

float vnoise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash21(i);
  float b = hash21(i + vec2(1.0, 0.0));
  float c = hash21(i + vec2(0.0, 1.0));
  float d = hash21(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// value noise whose x lattice wraps every px cells, so an angular coordinate
// can span the atan branch cut with matching tile edges
float vnoiseX(vec2 p, float px) {
  vec2 i = floor(p), f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float x0 = mod(i.x, px), x1 = mod(i.x + 1.0, px);
  float a = hash21(vec2(x0, i.y));
  float b = hash21(vec2(x1, i.y));
  float c = hash21(vec2(x0, i.y + 1.0));
  float d = hash21(vec2(x1, i.y + 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

const mat2 M2 = mat2(0.80, 0.60, -0.60, 0.80);

float fbm(vec2 p) {
  float a = 0.5, s = 0.0;
  for (int i = 0; i < 3; i++) {
    s += a * vnoise(p);
    p = M2 * p * 2.03 + vec2(11.7, 5.3);
    a *= 0.5;
  }
  return s * 1.14;
}

vec2 dirOf(float a) { return vec2(cos(a), sin(a)); }

// where the growth front of generation g sits, in radius, for the line n of
// family f: a ragged margin, each hypha a little ahead or behind its fellows
float frontOf(float tg, float n, float f, float g) {
  float j = hash21(vec2(n * 1.7 + f * 13.1, g * 7.3)) - 0.5;
  return RMIN + (RMAX - RMIN + 0.45) * smoothstep(0.0, 0.85, tg) + j * 0.35;
}

// per-line angular offset in spacing units: the bend that peels a child line
// off its parent near its start (half a spacing, decaying outward) plus a
// small fixed jitter that only takes hold once the bend has relaxed, so no
// line ever crosses its own index boundary
float offsetOf(float r, float rs, float sgn, float jit) {
  float b = sgn * 0.49 * exp(-max(r - rs, 0.0) / 0.14);
  return b + jit * (1.0 - abs(b) / 0.49);
}
// radius where child line n of family f gives up: many hyphae simply stop
float lifeOf(float rs, float n, float f, float g) {
  float h = hash21(vec2(n * 3.1 + f * 7.7, g * 5.9 + 2.0));
  return rs + 0.25 + 2.4 * h * h;
}
// start radius of family f in generation g: where its lines branch off
float startOf(float f, float g) {
  if (f < 0.5) return 0.0;
  return RMIN + 0.08 + f * 0.18 + (hash21(vec2(g, f)) - 0.5) * 0.12;
}

// one generation of the colony at warped point pw. Accumulates core coverage
// of the nearest line, its brightness, the halo and the tip glow.
void colony(vec2 pw, float g, float tg, float s0, float px,
            inout float core, inout float bright, inout float halo, inout float tip) {
  vec2  d  = pw - ORIGIN;
  float r  = max(length(d), 1e-4);
  // wrap the angle so the radial tiling is periodic: an exact cell count means
  // no partial tile and no line cut along the atan branch (the seam line)
  float th = mod(atan(d.y, d.x), TAU);
  float alpha = 1.0 - smoothstep(1.0, 1.85, tg);
  if (alpha <= 0.0) return;
  float s = s0;
  for (int f = 0; f < 5; f++) {
    float ff  = float(f);
    float rsf = startOf(ff, g);
    // integer cell count around the circle, then an exact spacing: the first
    // and last tiles keep full width and meet seamlessly
    float N   = max(1.0, floor(TAU / s + 0.5));
    float ss  = TAU / N;
    float u0  = th / ss;
    float n   = floor(u0);
    float hn  = hash21(vec2(n, ff + g * 3.0));
    float sgn = (f == 0) ? 0.0 : (hn < 0.5 ? 1.0 : -1.0);
    float jit = (hash21(vec2(n * 1.3, ff * 2.1 + g)) - 0.5) * 0.30;
    float F   = fract(u0 + offsetOf(r, rsf, sgn, jit)) - 0.5;
    // perpendicular distance: |F| over the gradient magnitude of u
    float db  = -sgn * 0.49 / 0.14 * exp(-max(r - rsf, 0.0) / 0.14);
    float gu  = sqrt(1.0 / (ss * ss * r * r) + db * db);
    float dist = abs(F) / gu;
    // visibility: born at its start radius, alive up to the growth front or
    // until the hypha gives up on its own
    float front = min(frontOf(tg, n, ff, g), lifeOf(rsf, n, ff, g));
    float born  = smoothstep(rsf - 0.02, rsf + 0.02, r);
    float alive = smoothstep(front + 0.008, front - 0.008, r);
    float vis   = born * alive * alpha;
    // width: leaders thickest, each family finer, tapering to the tip
    float w = 0.0021 * pow(0.80, ff) * (1.0 - 0.4 * smoothstep(front - 0.5, front, r));
    float c = smoothstep(w + px, w - px * 0.6, dist) * vis;
    // the body brightens toward its tip where the cytoplasm is busiest
    float age = exp(-(front - r) * 1.1);
    float bb  = 0.55 + 0.45 * age;
    bright = mix(bright, bb, step(core, c) * step(0.001, c));
    core  = max(core, c);
    halo += vis * exp(-dist * dist * 3500.0) * (0.3 + 0.7 * age) * pow(0.85, ff);
    // apex: swollen bright tip at the end of every living line
    vec2  tipv = vec2(dist * 1.6, (r - front) * 0.9);
    float td   = dot(tipv, tipv);
    tip += alpha * born * exp(-td * 30000.0) * pow(0.9, ff);
    s *= 0.5;
  }
}

// one twig segment; see the twig layer below
void seg(vec2 q, vec2 a, vec2 b, float s0, float len, float w, float reach, float alpha,
         inout float minD, inout float minW, inout float minA, inout float halo, inout float tip) {
  float frac = clamp((reach - s0) / len, 0.0, 1.0);
  if (frac <= 0.0) return;
  vec2  ab = (b - a) * frac;
  vec2  aq = q - a;
  float h  = clamp(dot(aq, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
  float dd = length(aq - ab * h);
  float ww = w * (1.0 - 0.45 * (s0 + len * frac * h));
  if (dd < minD) { minD = dd; minW = ww; minA = alpha; }
  halo += alpha * exp(-dd * dd * 500.0);
  if (frac < 1.0) {
    vec2 tp = q - (a + ab);
    tip += alpha * exp(-dot(tp, tp) * 2500.0);
  }
}

// lateral twigs of generation g: short branching polylines rooted on the
// finest runner line of their cell and growing outward behind the front
void twigs(vec2 pw, float g, float tg, float s0, float cell, float density,
           inout float minD, inout float minW, inout float minA, inout float halo, inout float tip) {
  float alpha = 1.0 - smoothstep(1.0, 1.85, tg);
  if (alpha <= 0.0) return;
  vec2 q  = pw / cell;
  vec2 ci = floor(q);
  for (int y = -1; y <= 1; y++) {
    for (int x = -1; x <= 1; x++) {
      vec2  id = ci + vec2(float(x), float(y));
      vec2  gs = id + vec2(g * 5.3, g * 2.1);
      float h0 = hash21(gs);
      float h1 = hash21(gs + 11.1);
      float h2 = hash21(gs + 23.7);
      float h3 = hash21(gs + 37.3);
      if (h3 > density) continue;
      // find the finest family born at this cell and its nearest line, then
      // put the root exactly on that line, bend and jitter included
      vec2  pc = (id + 0.5) * cell;
      vec2  d  = pc - ORIGIN;
      float r  = max(length(d) + (h0 - 0.5) * 0.6 * cell, 1e-4);
      float th = mod(atan(d.y, d.x), TAU);
      float s  = s0;
      float ff = 0.0;
      float rsf = startOf(0.0, g);
      for (int f = 1; f < 5; f++) {
        float rt = startOf(float(f), g);
        if (r > rt + 0.05) { s *= 0.5; ff = float(f); rsf = rt; }
      }
      float N   = max(1.0, floor(TAU / s + 0.5));
      float ss  = TAU / N;
      float n   = floor(th / ss);
      float hn  = hash21(vec2(n, ff + g * 3.0));
      float sgn = (ff < 0.5) ? 0.0 : (hn < 0.5 ? 1.0 : -1.0);
      float jit = (hash21(vec2(n * 1.3, ff * 2.1 + g)) - 0.5) * 0.30;
      float lineTh = (n + 0.5 - offsetOf(r, rsf, sgn, jit)) * ss;
      vec2  root = ORIGIN + dirOf(lineTh) * r;
      // keep the root within reach of the cell so nothing clips at borders
      root = clamp(root / cell, id - 0.35, id + 1.35);
      float front = min(frontOf(tg, n, ff, g), lifeOf(rsf, n, ff, g));
      // grows once the front has passed its root; none past a dead end
      float reach = smoothstep(0.0, 0.45, front - r) * 0.8;
      // outward lateral: off the runner at an acute angle, then wavering on
      float sb = h1 < 0.5 ? 1.0 : -1.0;
      float a0 = lineTh + sb * (0.45 + 0.5 * h2);
      float a1 = a0 - sb * (0.2 + 0.5 * h0);
      vec2  P1 = root + dirOf(a0) * 0.36;
      vec2  P2 = P1 + dirOf(a1) * 0.30;
      vec2  B1 = P1 + dirOf(a0 + sb * (0.5 + 0.4 * h3)) * 0.20;
      float w  = 0.012 * (0.8 + 0.4 * h2);
      seg(q, root, P1, 0.00, 0.34, w,        reach, alpha, minD, minW, minA, halo, tip);
      seg(q, P1,   P2, 0.34, 0.28, w * 0.75, reach, alpha, minD, minW, minA, halo, tip);
      seg(q, P1,   B1, 0.34, 0.18, w * 0.55, reach, alpha, minD, minW, minA, halo, tip);
    }
  }
}

void main() {
  vec2  uv     = gl_FragCoord.xy / u_resolution.xy;
  float aspect = u_resolution.x / u_resolution.y;
  vec2  p      = vec2((uv.x - 0.5) * aspect, uv.y - 0.5);
  float t      = u_t;                       // music-driven wave clock
  float tp     = u_tp;
  float s0     = 0.17 / max(clamp(u_scale, 0.3, 2.0), 0.3);
  float glow   = clamp(u_glow, 0.0, 2.0);
  float wander = clamp(u_wander, 0.0, 2.0);
  float dens   = clamp(u_twigs, 0.0, 1.0);
  float px     = 1.0 / u_resolution.y;      // frame units per device px

  float gen = floor(t);
  float tgA = t - gen;          // the growing wave
  float tgB = tgA + 1.0;        // the previous wave, dimming

  // ---- ground: dark damp substrate with pulp mottle and grit
  // (vec2 offset with a non-integer y so the value-noise lattice never aligns
  // with device rows - an integer +4.0 offset laid a 1px crease at p.y = 1/3)
  float mott = fbm(p * 3.0 + vec2(4.0, 4.173));
  float grit = hash21(floor(gl_FragCoord.xy / max(u_pixelRatio, 1.0) * 0.5));
  vec3  soil = mix(vec3(0.030, 0.027, 0.025), u_c3 * 0.11 + u_c0 * 0.03, 0.75);
  soil *= 0.70 + 0.60 * mott;
  soil += (grit - 0.5) * 0.012;
  float colonyF = fbm(p * 0.9 + vec2(1.0, 5.0) + tp * 0.006);
  vec3  hazeCol = mix(u_c2, u_c0, smoothstep(0.35, 0.65, fbm(p * 0.6 + 13.0)));
  soil += hazeCol * 0.04 * glow * smoothstep(0.35, 0.8, colonyF);

  // ---- fine fuzz: the felt of finest hyphae the colony lays down, streaks
  // aligned with the radial flow, very faint
  vec2  dp   = p - ORIGIN;
  float rp   = max(length(dp), 1e-4);
  float thp  = atan(dp.y, dp.x);
  float fz1  = vnoiseX(vec2((thp / TAU + 0.5) * 90.0, rp * 5.0) + 7.0, 90.0);
  float fz2  = vnoiseX(vec2((thp / TAU + 0.5) * 170.0, rp * 9.0) + 19.0, 170.0);
  float fmat = smoothstep(0.60, 0.92, fz1) * 0.6 + smoothstep(0.66, 0.94, fz2) * 0.4;
  fmat *= 0.4 + 0.6 * smoothstep(0.30, 0.75, colonyF);
  vec3  matCol = mix(u_c2, u_c1, 0.4) * 0.14 * glow + vec3(0.04);
  vec3  col = soil + matCol * fmat;

  // ---- the two waves, each in its own warped domain
  vec2 warpA = vec2(fbm(p * 1.4 + vec2(3.1, 7.7) + gen * 4.7),
                    fbm(p * 1.4 + vec2(9.2, 1.4) + gen * 2.9)) - 0.5;
  vec2 warpB = vec2(fbm(p * 1.4 + vec2(3.1, 7.7) + (gen - 1.0) * 4.7),
                    fbm(p * 1.4 + vec2(9.2, 1.4) + (gen - 1.0) * 2.9)) - 0.5;
  // a fine wobble so the threads are never ruler-straight
  vec2 wob = vec2(vnoise(p * 9.0 + 41.0), vnoise(p * 9.0 + 77.0)) - 0.5;
  vec2 pwA = p + (warpA * 0.30 + wob * 0.014) * wander;
  vec2 pwB = p + (warpB * 0.30 + wob * 0.014) * wander;

  // previous wave first: it sits beneath
  float coreB = 0.0, brB = 0.0, haloB = 0.0, tipB = 0.0;
  colony(pwB, gen - 1.0, tgB, s0, px, coreB, brB, haloB, tipB);
  float coreA = 0.0, brA = 0.0, haloA = 0.0, tipA = 0.0;
  colony(pwA, gen, tgA, s0, px, coreA, brA, haloA, tipA);

  // twigs on both waves
  float cell = 0.13;
  float tD = 10.0, tW = 0.02, tA = 0.0, tHalo = 0.0, tTip = 0.0;
  twigs(pwB, gen - 1.0, tgB, s0, cell, dens, tD, tW, tA, tHalo, tTip);
  twigs(pwA, gen, tgA, s0, cell, dens, tD, tW, tA, tHalo, tTip);
  float pxC = px / cell;
  float twigCore = smoothstep(tW + pxC, tW - pxC * 0.5, tD) * tA;

  // a narrow band of light sweeps outward along the threads; the kick gates it
  float rA = max(length(pwA - ORIGIN), 1e-4);
  float pulseSpeed = 1.6 + u_beat * 1.2;
  float pulsePos = mod(tp * pulseSpeed, RMAX);
  float pulseDelta = rA - pulsePos;
  float pulse = exp(-pulseDelta * pulseDelta * 1200.0) * u_beat;

  // ---- colour. Halo of bioluminescence, then the pale hyphal bodies
  vec3 haloCol = mix(u_c2, u_c1, smoothstep(0.3, 0.7, colonyF));
  float haloAll = haloB * 0.5 + haloA + tHalo * 0.6;
  col += haloCol * 0.22 * glow * min(haloAll, 2.5);

  // old wave: dimmer, cooler, sunk into the ground
  vec3 bodyB = mix(vec3(0.60, 0.60, 0.57), u_c2, 0.45) * (0.6 + 0.3 * brB);
  col = mix(col, bodyB, coreB * 0.9);
  col = mix(col, bodyB * 0.9, twigCore * 0.8);
  // new wave: pale translucent body lit from within by the pulse
  vec3 bodyA = mix(vec3(0.90, 0.88, 0.82), u_c2, 0.28) * (0.55 + 0.45 * brA);
  bodyA *= 0.70 + 0.25 * pulse * glow;
  bodyA += u_c0 * 0.12 * pulse * glow;
  col = mix(col, bodyA, coreA);
  col = mix(col, bodyA, twigCore * step(0.001, tA) * 0.9);

  // apex glow: swollen bright tips along the advancing margin, flaring on beat
  vec3 tipCol = mix(u_c1, vec3(1.0), 0.45);
  col += tipCol * min(tipA + tipB * 0.3 + tTip * 0.5, 1.4) * (0.35 + 0.45 * glow + u_beat * 0.2);

  // depth fog sinks the far strands into the dark; a kick lights the fog it
  // passes through - wider than the strand band, like light scattered by haze
  float fogDepth = max(rp - 0.15, 0.0) + fbm(p * 2.0 + 17.0) * 0.12;
  float fogAmount = 1.0 - exp(-fogDepth * u_fogDensity);
  col = mix(col, u_fogColor, fogAmount);
  float fogPulse = exp(-pulseDelta * pulseDelta * 280.0) * u_beat;
  col += mix(u_c1, vec3(1.0), 0.35) * fogPulse * fogAmount * 1.8;

  gl_FragColor = vec4(col, 1.0);
}
`;

export class Mycelium extends BaseScene {
  _init() {
    this.label = "Mycelium";
    this.category = "Music";

    this.growth = 0; // integrated wave clock (seconds at speed 1)
    this.speed = 0.25;
    this._phrase = { hue: 0, t: 0, low: 0, phraseMax: 0, prev: 0, peakRise: 0 };
    this._phraseHue = 0;
    this._baseColors = [
      new THREE.Color(0.231, 0.51, 0.965),
      new THREE.Color(0.659, 0.333, 0.969),
      new THREE.Color(0.133, 0.827, 0.933),
      new THREE.Color(0.957, 0.247, 0.369),
    ];
    this._fogBase = new THREE.Color(0.02, 0.028, 0.05);

    this.uniforms = {
      u_resolution: { value: new THREE.Vector2(1, 1) },
      u_t: { value: 0.22 },
      u_tp: { value: 0 },
      u_scale: { value: 0.7 },
      u_twigs: { value: 0.35 },
      u_glow: { value: 0.15 },
      u_wander: { value: 0.6 },
      u_beat: { value: 0 },
      u_pixelRatio: { value: 1 },
      u_c0: { value: new THREE.Color() },
      u_c1: { value: new THREE.Color() },
      u_c2: { value: new THREE.Color() },
      u_c3: { value: new THREE.Color() },
      u_fogColor: { value: new THREE.Color() },
      u_fogDensity: { value: 0.25 },
    };
    for (let i = 0; i < 4; i++)
      this.uniforms[`u_c${i}`].value.copy(this._baseColors[i]);
    this.uniforms.u_fogColor.value.copy(this._fogBase);

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.BufferAttribute(
        new Float32Array([-1, -1, 0, 3, -1, 0, -1, 3, 0]),
        3,
      ),
    );

    const material = new THREE.RawShaderMaterial({
      vertexShader: VERT,
      fragmentShader: FRAG,
      uniforms: this.uniforms,
      side: THREE.DoubleSide,
    });

    this.mesh = new THREE.Mesh(geometry, material);
    this.mesh.frustumCulled = false; // raw NDC output, camera-independent
    this.group.add(this.mesh);

    if (this.renderer) {
      this.composer = new EffectComposer(this.renderer);
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.addPass(new RenderPass(this.group, this.camera));
      this.bloomPass = new UnrealBloomPass(
        this.renderer.getSize(new THREE.Vector2()),
        0.5,
        0.7,
        0.25,
      );
      this.composer.addPass(this.bloomPass);
      // ponytail: no OutputPass - this shader is authored display-referred
      // (the old direct-to-screen look); OutputPass would treat its dark values
      // as linear and gamma-lift the frame to a washed-out white.
    }
  }

  get paintsCanvas() {
    return true;
  }

  get captureTexture() {
    return this.composer?.readBuffer?.texture ?? null;
  }

  // Band-driven uniforms. Growth speed <- bass, wander <- bass+beat, twigs <-
  // mid, runner density <- high, bioluminescence <- mid/high + beat, pulse
  // speed/amplitude <- beat. Floors keep a slow crawl and a faint haze in
  // silence (the reference's idle look); caps stop the nonlinear mapping from
  // saturating.
  _audioUniforms(b0, b1, b2, beat) {
    const u = this.uniforms;
    this.speed = Math.min(0.25 + b0 * 3 + beat * 0.5, 3);
    u.u_glow.value = Math.min(0.15 + (b1 + b2) * 2 + beat * 0.5, 2);
    u.u_beat.value = beat;
    u.u_wander.value = Math.min(0.6 + b0 * 1.2 + beat * 0.4, 2);
    u.u_twigs.value = Math.min(0.35 + b1 * 1.2 + b2 * 0.8, 1);
    u.u_scale.value = Math.min(0.7 + b2 * 1.2, 2);
    u.u_fogDensity.value = Math.min(
      0.25 + u.u_glow.value * 0.22 + beat * 0.15,
      0.7,
    );
    if (this.bloomPass)
      this.bloomPass.strength = Math.min(
        0.5 + u.u_glow.value * 0.35 + beat * 0.25,
        1.4,
      );
  }

  setAudio(audio) {
    super.setAudio(audio);
    this.p9 = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9?.update(dt);
    const p9 = this.p9;
    const b0 = p9 ? p9.band(-1, 1, 0) : 0;
    const b1 = p9 ? p9.band(-1, 1, 1) : 0;
    const b2 = p9 ? p9.band(-1, 1, 2) : 0;
    const beat = p9 ? p9.beat() : 0;
    this._audioUniforms(b0, b1, b2, beat);

    this._phrase = dropColorPulse(this._phrase, b0, dt);
    let dh = this._phrase.hue - this._phraseHue;
    dh = Math.atan2(Math.sin(dh), Math.cos(dh));
    this._phraseHue += dh * Math.min(1, dt * 0.55);
    for (let i = 0; i < 4; i++)
      hueRotate(
        this._baseColors[i],
        this._phraseHue,
        this.uniforms[`u_c${i}`].value,
      );
    hueRotate(this._fogBase, this._phraseHue, this.uniforms.u_fogColor.value);

    // Integrated clock: a per-frame speed would make u_time*spd jump when
    // the band moves; integrating keeps the wave clock monotonic and smooth.
    this.growth += dt * this.speed;
    this.uniforms.u_t.value = this.growth / 32 + 0.22;
    this.uniforms.u_tp.value = this.growth;
  }

  // gl_FragCoord is in drawing-buffer pixels; re-sync on canvas resize.
  _syncResolution() {
    const w = this.renderer.domElement.width;
    const h = this.renderer.domElement.height;
    if (w !== this._rw || h !== this._rh) {
      this._rw = w;
      this._rh = h;
      this.uniforms.u_resolution.value.set(w, h);
    }
    const pr = this.renderer.getPixelRatio?.() ?? 1;
    if (pr !== this.uniforms.u_pixelRatio.value)
      this.uniforms.u_pixelRatio.value = pr;
  }

  render() {
    this._syncResolution();
    if (!this.composer) {
      super.render();
      return;
    }

    const renderToScreen = !this._captureNextFrame;
    this._captureNextFrame = false;
    const w = this.renderer.domElement.width;
    const h = this.renderer.domElement.height;
    if (w !== this._cw || h !== this._ch) {
      this._cw = w;
      this._ch = h;
      const pr = this.renderer.getPixelRatio();
      this.composer.setPixelRatio(pr);
      this.composer.setSize(w / pr, h / pr);
    }
    this.composer.renderToScreen = renderToScreen;
    this.composer.render();
  }

  resize(w, h) {
    if (!this.composer) return;
    this.composer.setSize(w, h);
  }
}
