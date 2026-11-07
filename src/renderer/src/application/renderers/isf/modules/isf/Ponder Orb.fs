/*{
  "DESCRIPTION": "A scrying orb of nebula and starlight over a strange glowing landscape, ringed by drifting alien glyphs and script. The glass is an analytic sphere with animated thin-film iridescence, an adjustable index of refraction, and a reflection of the ground in its lower shell. Inside: a churning spiral-armed nebula whose regions burn through a shifting magenta/violet/rose/amber palette with electric cyan wisps, dark absorbing dust lanes, and two layers of tiny 4-pointed stars — with sliders for animation speed, domain warp and turbulence. A fringe of writhing tentacle fingers waves along the orb's rim, magical smoke boils off it, wispy ribbons orbit it, and sixteen alien glyphs drift around it on a spherical orbit, built from ring, spine, track, radial and script families with procedurally generated lettering. Fine atmospheric dust hangs in the air. The terrain carries glowing carved spirals, emissive veins, and its own relief / detail / colour controls. The camera orbits on a slider, and hue rotates independently for orb, ground, effects and the whole frame.",
  "CREDIT": "CC0",
  "CATEGORIES": [
    "generator",
    "raymarch",
    "glass",
    "scifi"
  ],
  "INPUTS": [
    {
      "NAME": "uNebula",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uSpiral",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uNebSpeed",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 4.0
    },
    {
      "NAME": "uWarp",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uTurb",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uColour",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.0
    },
    {
      "NAME": "uContrast",
      "TYPE": "float",
      "DEFAULT": 1.55,
      "MIN": 0.5,
      "MAX": 3.0
    },
    {
      "NAME": "uOrbit",
      "TYPE": "float",
      "DEFAULT": 0.15,
      "MIN": -2.0,
      "MAX": 2.0
    },
    {
      "NAME": "uDark",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uStars",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uDust",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uSwirl",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uGlow",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uIOR",
      "TYPE": "float",
      "DEFAULT": 1.42,
      "MIN": 1.0,
      "MAX": 2.6
    },
    {
      "NAME": "uIrid",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uSmoke",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uTentacle",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uSkyNeb",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uTrails",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uReflect",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uAtmos",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uSymbols",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uMotes",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uRelief",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uDetail",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uGndSpiral",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uGroundCol",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.0
    },
    {
      "NAME": "uSheen",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uHueNeb",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uHueGround",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uHueFx",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uHue",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uVignette",
      "TYPE": "float",
      "DEFAULT": 0.65,
      "MIN": 0.0,
      "MAX": 1.5
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/if4o5iee


// CRYSTAL BALL — ISF port of the Shadertoy/WebGL2 version
// (GLSL ES 1.00: fixed loop bounds, gl_FragColor, TIME/RENDERSIZE).
// Generated from crystal-ball.glsl by isf_convert.py — edit that file
// and re-run the converter rather than hand-editing this one.

#define TAU 6.28318530718

vec3 hueShift(vec3 c, float a){
    vec3 k = vec3(0.57735);
    float ca = cos(a * TAU);
    float sa = sin(a * TAU);
    return c * ca + cross(k, c) * sa + k * dot(k, c) * (1.0 - ca);
}

float hash11(float n){ return fract(sin(n * 127.1) * 43758.5453); }

float hash31(vec3 p){
    p = fract(p * 0.3183099 + vec3(0.71, 0.113, 0.419));
    p += dot(p, p.yzx + 19.19);
    return fract(p.x * p.y * p.z * 95.4337);
}

float noise3(vec3 p){
    vec3 i = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(hash31(i + vec3(0.0,0.0,0.0)), hash31(i + vec3(1.0,0.0,0.0)), f.x),
                   mix(hash31(i + vec3(0.0,1.0,0.0)), hash31(i + vec3(1.0,1.0,0.0)), f.x), f.y),
               mix(mix(hash31(i + vec3(0.0,0.0,1.0)), hash31(i + vec3(1.0,0.0,1.0)), f.x),
                   mix(hash31(i + vec3(0.0,1.0,1.0)), hash31(i + vec3(1.0,1.0,1.0)), f.x), f.y), f.z);
}

float fbm3(vec3 p){
    float a = 0.5, s = 0.0;
    for (int i = 0; i < 5; i++){
        s += a * noise3(p);
        p = p * 2.02 + vec3(1.7, 9.2, 4.3);
        a *= 0.5;
    }
    return s;
}

mat2 rot(float a){ float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }

float sdSeg(vec2 p, vec2 b){
    float h = clamp(dot(p, b) / dot(b, b), 0.0, 1.0);
    return length(p - b * h);
}

// distance to an archimedean spiral arm r = b*(theta + phase)
float spiralArm(vec2 q, float b, float phase){
    float r = length(q);
    float a = atan(q.y, q.x) + phase;
    float n = floor((r / b - a) / TAU + 0.5);
    return abs(r - b * (a + TAU * n));
}

const vec3  BALL_C = vec3(0.0, 0.72, 0.0);
const float BALL_R = 1.0;

// thin-film interference: shifting spectral sheen over the glass
vec3 iridescence(float f, float phase){
    return 0.5 + 0.5 * cos(TAU * (vec3(0.0, 0.33, 0.67) + f * 3.2 + phase));
}

// Contrast for the orb's interior: pull everything away from a low
// pivot, so the voids sink toward black while the cores punch up.
// Applied to the nebula only — the rim, film and reflection are
// added afterwards and keep their own levels.
vec3 contrastify(vec3 c, float k){
    const float piv = 0.15;
    vec3 o = max(piv + (c - piv) * k, vec3(0.0));
    // a touch of S-curve so the midtones separate too
    return mix(o, o * o * (3.0 - 2.0 * min(o, vec3(1.0))), clamp(k - 1.0, 0.0, 1.0) * 0.35);
}

// nebula palette: violet -> magenta -> rose -> amber, with cool
// blues at the far end. Cosine palette, so it loops seamlessly.
vec3 nebPal(float t){
    vec3 c = vec3(0.46, 0.13, 0.62) +
             vec3(0.40, 0.20, 0.36) * cos(TAU * (vec3(1.0, 0.92, 0.80) * t + vec3(0.00, 0.18, 0.60)));
    // push saturation back up — a plain cosine palette reads washed out
    return mix(vec3(dot(c, vec3(0.299, 0.587, 0.114))), c, 1.45);
}

// ---- ground: a low strange terrain the orb rests in --------------
// spiral figures carved into the ground, like glowing crop circles
float groundSpiral(vec2 q){
    float r = length(q);
    float spir = 0.0;
    spir += exp(-pow(spiralArm(q - vec2( 1.95, -0.55), 0.115, 0.0) * 13.0, 2.0));
    spir += exp(-pow(spiralArm(q - vec2(-2.30,  0.85), 0.140, 2.1) * 11.0, 2.0));
    spir += exp(-pow(spiralArm(q - vec2( 0.35,  2.60), 0.100, 4.3) * 15.0, 2.0));
    spir += exp(-pow(spiralArm(q - vec2(-1.10, -2.30), 0.130, 1.2) * 12.0, 2.0));
    // a wide spiral swirling around the orb itself
    spir += exp(-pow(spiralArm(q, 0.320, TIME * 0.05) * 15.0, 2.0)) *
            smoothstep(0.9, 1.6, r) * 0.8;
    return clamp(spir, 0.0, 1.0) * smoothstep(4.6, 1.2, r);
}

float groundH(vec2 q){
    float r = length(q);
    // the orb sits in a shallow depression
    float sag = -0.30 * exp(-r * r * 2.2);
    float relief = (fbm3(vec3(q * 0.62, 0.0)) - 0.5) * 0.62 * uRelief;
    float detail = (fbm3(vec3(q * 2.8, 7.3)) - 0.5) * 0.15 * uDetail;
    // flatten far out so the skyline stays smooth and low
    float outer = smoothstep(5.5, 1.5, r);
    return sag + (relief + detail) * outer + groundSpiral(q) * 0.075 * uGndSpiral;
}

vec3 groundN(vec2 q){
    vec2 e = vec2(0.020, 0.0);
    float h = groundH(q);
    return normalize(vec3(h - groundH(q + e.xy), e.x * 1.6, h - groundH(q + e.yx)));
}

// ---- the nebula inside the glass ---------------------------------
vec3 nebula(vec3 ro, vec3 rd, float tIn, float tOut, float seed){
    vec3 acc = vec3(0.0);
    float trans = 1.0;
    float dt = (tOut - tIn) / 40.0;
    float t = tIn + dt * hash11(seed * 13.7) * 0.9;
    float nt = TIME * max(uNebSpeed, 0.0);
    float spin = nt * 0.11 * uSwirl;

    for (int i = 0; i < 40; i++){
      if (trans >= 0.02){
        vec3 p = ro + rd * t;
        vec3 q = p - BALL_C;
        q.xz = rot(spin) * q.xz;
        q.xy = rot(spin * 0.6) * q.xy;
        vec3 w = q * 3.4 + vec3(0.0, -nt * 0.16 * uSwirl, 0.0);
        // the warp field itself evolves, so the cloud churns rather
        // than merely sliding past the camera
        vec3 warp = vec3(fbm3(w * 0.55 + 11.3 + nt * 0.22),
                         fbm3(w * 0.55 + 27.1 - nt * 0.17),
                         fbm3(w * 0.55 + 41.7 + nt * 0.13)) - 0.5;
        vec3 wq = w + warp * (1.9 * uWarp);
        // fine turbulence: a cheap sinusoidal curl over the warp
        wq += vec3(sin(wq.y * 3.6 + nt * 0.85),
                   cos(wq.z * 3.1 - nt * 0.67),
                   sin(wq.x * 4.2 + nt * 1.05)) * 0.22 * uTurb;
        float f = fbm3(wq);
        float d = pow(max(f - 0.42, 0.0) * 3.2, 1.7);
        d += pow(max(fbm3(wq * 2.3 + 5.0) - 0.48, 0.0) * 3.0, 2.2) * 0.45;
        float rq = length(q);

        // --- galactic spiral arms winding through the volume ---
        float az = atan(q.z, q.x);
        float rxz = max(length(q.xz), 0.04);
        float armF = 0.5 + 0.5 * cos(2.0 * az - 5.5 * log(rxz) + nt * 0.30);
        float arms = mix(1.0, 0.24 + 2.10 * pow(armF, 2.0), clamp(uSpiral, 0.0, 2.5));
        // a tighter counter-winding set so the arms interleave
        float armF2 = 0.5 + 0.5 * cos(3.0 * az + 9.0 * log(rxz) - nt * 0.22);
        arms *= mix(1.0, 0.55 + 1.05 * pow(armF2, 3.0), clamp(uSpiral, 0.0, 1.5) * 0.7);
        // vortex filaments curling toward the core
        float vor = 0.5 + 0.5 * cos(az * 5.0 + rq * 14.0 - nt * 0.55);
        arms *= mix(1.0, 0.70 + 0.75 * vor, clamp(uSpiral, 0.0, 1.5) * 0.5);
        // flatten the arms into a disc so it reads as a galaxy
        arms *= mix(1.0, exp(-pow(q.y * 2.6, 2.0)) * 1.5 + 0.25, clamp(uSpiral, 0.0, 1.0) * 0.75);
        d *= arms;

        // --- broad faint halo layer (reuses the warp field: free) ---
        float halo = pow(max(warp.x + 0.5 - 0.30, 0.0) * 1.7, 2.0) * 0.5;
        d += halo * 0.55;

        d *= smoothstep(0.03, 0.42, rq) * smoothstep(1.02, 0.70, rq);
        d *= uNebula * 3.4;

        // --- dark dust lanes: absorb without emitting ---
        float dark = smoothstep(0.46, 0.78, noise3(w * 0.85 + 31.0 + nt * 0.14));
        dark *= smoothstep(1.02, 0.72, rq) * uDark;

        if (d > 0.001){
            float core = smoothstep(0.35, 1.1, d);

            // Regional colour drift: different parts of the cloud burn
            // different hues and the whole palette rotates slowly.
            // `warp.y` is already computed above, so this costs nothing.
            float region = warp.y + 0.5;
            float tcol = region * 0.95 + nt * 0.045 + core * 0.20 + rq * 0.35;
            vec3 c = mix(vec3(0.34, 0.06, 0.66), nebPal(tcol), clamp(uColour, 0.0, 2.0));

            // electric cyan accent wisps threading the cloud
            float acF = noise3(wq * 1.55 + vec3(0.0, nt * 0.26, 41.0));
            c = mix(c, vec3(0.22, 0.74, 1.0),
                    smoothstep(0.60, 0.93, acF) * 0.60 * clamp(uColour, 0.0, 2.0));

            // hot cores warm toward rose-gold, but stay tinted
            c = mix(c, vec3(1.0, 0.72, 0.66), core * core * 0.55);
            // deeper indigo where the dust lanes lie over the clouds
            c = mix(c, vec3(0.10, 0.02, 0.24), dark * 0.75);
            c = hueShift(c, uHueNeb);
            float a = clamp(d * dt * 2.0, 0.0, 1.0);
            acc += c * a * trans * (0.45 + 1.15 * core) * (1.0 - dark * 0.65);
            trans *= 1.0 - a * 0.85;
        }
        // the lanes shadow whatever lies behind them
        trans *= 1.0 - clamp(dark * dt * 1.6, 0.0, 1.0);

        // --- star layer 1: the main field ---
        // Keep the falloff inside its own cell (wider clips at the
        // borders and shows cube artifacts) and the twinkle strictly
        // positive (negative subtracts light -> black dots).
        vec3 sc = floor(q * 15.0);
        float sh = hash31(sc + seed);
        if (sh > 0.80){
            vec3 sp = (sc + 0.5 + (vec3(hash31(sc + 1.3), hash31(sc + 5.7), hash31(sc + 9.1)) - 0.5) * 0.7) / 15.0;
            vec3 e = q - sp;
            float sd = length(e);
            float tw = 0.30 + 0.70 * (0.5 + 0.5 * sin(TIME * 2.6 + sh * TAU * 7.0));
            // 4-pointed star: a tight core plus two crossed spikes.
            // Spikes lie along world X and Y so they project as a cross.
            float core = exp(-sd * sd * 46000.0);
            float sx = exp(-(e.y * e.y + e.z * e.z) * 260000.0 - e.x * e.x * 3400.0);
            float sy = exp(-(e.x * e.x + e.z * e.z) * 260000.0 - e.y * e.y * 3400.0);
            vec3 stc = mix(vec3(1.0, 0.92, 1.0), vec3(0.80, 0.88, 1.0), hash11(sh * 31.7));
            acc += hueShift(stc, uHueNeb * 0.5) * (core + 0.55 * (sx + sy)) * tw * trans * uStars * 3.4;
        }

        // --- star layer 2: a fine dust of tiny 4-pointed sparks ---
        vec3 sc2 = floor(q * 30.0 + 3.7);
        float sh2 = hash31(sc2 + seed * 2.3);
        if (sh2 > 0.80){
            vec3 sp2 = (sc2 + 0.5 + (vec3(hash31(sc2 + 2.7), hash31(sc2 + 6.1), hash31(sc2 + 11.3)) - 0.5) * 0.7 - 3.7) / 30.0;
            vec3 e2 = q - sp2;
            float sd2 = length(e2);
            float tw2 = 0.20 + 0.80 * (0.5 + 0.5 * sin(TIME * 3.7 + sh2 * TAU * 11.0));
            float c2s = exp(-sd2 * sd2 * 150000.0);
            float x2 = exp(-(e2.y * e2.y + e2.z * e2.z) * 900000.0 - e2.x * e2.x * 11000.0);
            float y2 = exp(-(e2.x * e2.x + e2.z * e2.z) * 900000.0 - e2.y * e2.y * 11000.0);
            acc += vec3(0.92, 0.90, 1.0) * (c2s + 0.5 * (x2 + y2)) * tw2 * trans * uDust * 3.0;
        }
        t += dt;
      }
    }

    // pulsing heart: a slow breath of light at the orb's core
    acc += hueShift(vec3(0.85, 0.35, 1.0), uHueNeb) * (0.35 + 0.25 * sin(TIME * 0.9)) * 0.22 * uNebula;
    return acc;
}

// ---- alien script floating around the orb -------------------------
// one character of an invented alphabet: a stem plus a hash-chosen
// set of bars, diagonals and dots
float alienLetter(vec2 g, float h){
    float d = sdSeg(g - vec2(0.0, 0.46), vec2(0.0, -0.92));          // stem
    d = min(d, fract(h *  7.0) > 0.45 ? sdSeg(g - vec2(-0.30,  0.46), vec2(0.60, 0.0)) : 1e3);
    d = min(d, fract(h * 13.0) > 0.50 ? sdSeg(g - vec2(-0.30, -0.46), vec2(0.60, 0.0)) : 1e3);
    d = min(d, fract(h * 23.0) > 0.55 ? sdSeg(g - vec2(-0.28,  0.02), vec2(0.56, 0.0)) : 1e3);
    d = min(d, fract(h * 41.0) > 0.50 ? sdSeg(g - vec2(0.0, 0.10), vec2(0.34, 0.34))   : 1e3);
    d = min(d, fract(h * 59.0) > 0.60 ? sdSeg(g - vec2(0.0, -0.12), vec2(-0.34, -0.30)): 1e3);
    float m = smoothstep(0.115, 0.04, d);
    if (fract(h * 67.0) > 0.55)
        m = max(m, smoothstep(0.05, -0.01, length(g - vec2(0.34, -0.34)) - 0.085));
    if (fract(h * 89.0) > 0.65)
        m = max(m, smoothstep(0.05, -0.01, length(g - vec2(-0.34, 0.40)) - 0.075));
    return m;
}

// a short line of alien text
float alienWord(vec2 g, float h){
    float m = 0.0;
    float n = 3.0 + floor(fract(h * 31.0) * 2.0);      // 3 or 4 characters
    for (int i = 0; i < 4; i++){
        if (float(i) < n){
            float off = (float(i) - (n - 1.0) * 0.5) * 0.52;
            m = max(m, alienLetter((g - vec2(off, 0.0)) * vec2(2.4, 1.5),
                                   hash11(h * 17.0 + float(i) * 5.3)));
        }
    }
    return m;
}

// g is local sprite space (~[-1,1]); returns coverage 0..1
// --- minimal alien sigils ------------------------------------------
// A stroked arc, used as both a primary form and an accent.
float arcSeg(vec2 g, float r, float th, float ac, float aw){
    float a = atan(g.y, g.x);
    float da = abs(mod(a - ac + 3.14159, TAU) - 3.14159);
    float band = smoothstep(th + 0.030, th - 0.008, abs(length(g) - r));
    return band * smoothstep(aw, aw - 0.18, da);
}

// Deliberately sparse: one primary element plus a few small accents,
// all thin-stroked. Variety comes from the combinatorics rather than
// from piling on detail — and the small code keeps the ANGLE compiler
// happy (a long chain of bespoke glyphs wedges it).
float alienGlyph(vec2 g, float id){
    float h1 = hash11(id * 2.90 + 1.3);
    float h2 = hash11(id * 4.30 + 5.7);
    float h3 = hash11(id * 7.70 + 2.9);
    float h4 = hash11(id * 11.9 + 8.1);
    float m = 0.0;
    float d = 1e3;

    // orient the whole mark
    if (h4 > 0.5) g.x = -g.x;
    g = rot((h3 - 0.5) * 1.6) * g;

    // ---- primary form ----
    if (h1 < 0.26){
        // a single long stroke through the centre, on a quantised angle
        float a = floor(h2 * 4.0) * 0.7854 + 0.3927;
        vec2 dir = vec2(cos(a), sin(a)) * (0.62 + 0.26 * h3);
        d = min(d, sdSeg(g + dir, dir * 2.0));
        // occasionally crossed by a short bar
        if (h4 > 0.62){
            vec2 pd = vec2(-dir.y, dir.x) * 0.30;
            d = min(d, sdSeg(g + pd, pd * 2.0));
        }
        // a pair of hatch ticks stepping along the stroke
        if (h2 > 0.35){
            vec2 u = normalize(dir);
            vec2 nd = vec2(-u.y, u.x);
            d = min(d, sdSeg(g - u * 0.34 + nd * 0.085, nd * 0.17));
            d = min(d, sdSeg(g + u * 0.30 + nd * 0.065, nd * 0.13));
        }
    } else if (h1 < 0.50){
        // a bare ring, sometimes broken, over a concentric inner trace
        float r = 0.32 + 0.24 * h2;
        if (h3 > 0.45){
            m = max(m, arcSeg(g, r, 0.018, h2 * TAU, 1.9 + 0.7 * h4));
        } else {
            m = max(m, smoothstep(0.040, 0.014, abs(length(g) - r)));
        }
        if (h4 > 0.40)
            m = max(m, smoothstep(0.030, 0.011, abs(length(g) - r * 0.46)));
        // a short radial spur breaking out of the ring
        if (h2 > 0.55){
            vec2 sd = vec2(cos(h4 * TAU), sin(h4 * TAU));
            d = min(d, sdSeg(g - sd * r, sd * 0.20));
        }
    } else if (h1 < 0.74){
        // a coiled spiral, sometimes double-armed
        float rr = length(g);
        float b = 0.052 + 0.038 * h2;
        float lim = smoothstep(0.70, 0.54, rr) * step(0.045, rr);
        m = max(m, smoothstep(0.040, 0.013, spiralArm(g, b, h3 * TAU)) * lim);
        if (h4 > 0.52)
            m = max(m, smoothstep(0.040, 0.013, spiralArm(g, b, h3 * TAU + 3.14159)) * lim);
        // a bead riding the outer end of the coil
        float ea = h3 * TAU + 4.0;
        vec2 ep = (0.60 + 0.06 * h2) * vec2(cos(ea), sin(ea));
        m = max(m, smoothstep(0.032, -0.008, length(g - ep) - 0.032));
    } else {
        // an open arc paired with a chord and a counter-arc
        float r = 0.44 + 0.20 * h2;
        m = max(m, arcSeg(g, r, 0.018, h3 * TAU, 1.0 + 0.7 * h4));
        float a = h3 * TAU;
        vec2 e0 = r * vec2(cos(a - 1.0), sin(a - 1.0));
        vec2 e1 = r * vec2(cos(a + 1.0), sin(a + 1.0));
        if (h2 > 0.5) d = min(d, sdSeg(g - e0, e1 - e0));
        m = max(m, arcSeg(g, r * 0.60, 0.013, a + 3.14159, 0.45 + 0.45 * h2));
        // beads capping the arc ends
        if (h4 > 0.5){
            m = max(m, smoothstep(0.032, -0.008, length(g - e0) - 0.030));
            m = max(m, smoothstep(0.032, -0.008, length(g - e1) - 0.030));
        }
    }

    // ---- a small mark at the centre ----
    if (h3 > 0.58)      m = max(m, smoothstep(0.030, -0.008, length(g) - 0.042));
    else if (h3 < 0.24) m = max(m, smoothstep(0.028, 0.010, abs(length(g) - 0.105)));

    // ---- two to four small accents ----
    float na = 2.0 + floor(h2 * 3.0);
    for (int i = 0; i < 4; i++){
        if (float(i) < na){
            float fi = float(i);
            float ha = hash11(id * 3.17 + fi * 7.31);
            float hb = hash11(id * 5.11 + fi * 3.77);
            float ang = ha * TAU;
            float rad = 0.58 + 0.34 * hb;
            vec2 c = rad * vec2(cos(ang), sin(ang));
            if (hb < 0.40){
                // a bead
                m = max(m, smoothstep(0.040, -0.010, length(g - c) - (0.028 + 0.026 * ha)));
            } else if (hb < 0.62){
                // a short radial tick
                vec2 dir = normalize(c) * (0.10 + 0.10 * ha);
                d = min(d, sdSeg(g - c, dir));
            } else if (hb < 0.80){
                // a tiny coil hanging off the mark
                vec2 lg = (g - c) * 3.6;
                float lr = length(lg);
                m = max(m, smoothstep(0.10, 0.035, spiralArm(lg, 0.16, ha * TAU)) *
                           smoothstep(0.85, 0.62, lr) * step(0.10, lr));
            } else {
                // a tiny counter-arc
                m = max(m, arcSeg(g, rad, 0.014, ang, 0.22 + 0.20 * ha));
            }
        }
    }

    // an occasional fragment of an outer orbit ring
    if (h4 > 0.56)
        m = max(m, arcSeg(g, 1.00, 0.013, h1 * TAU, 0.32 + 0.38 * h2) * 0.85);

    m = max(m, smoothstep(0.044, 0.015, d));      // thin strokes
    return clamp(m, 0.0, 1.0);
}


// ---- writhing tentacle fringe around the orb's perimeter ---------
// Screen-space polar: `ang` is the angle around the projected orb and
// `rimD` the distance outside its surface, so each finger is a wavy
// tapering line rooted at the rim. Neighbouring cells are sampled so
// fingers can cross without popping.
float tentacleFringe(float ang, float rimD, float t){
    float N = 54.0;
    float cell = floor(ang / TAU * N + 0.5);
    float acc = 0.0;
    for (int k = 0; k < 3; k++){
        float idx = cell + float(k) - 1.0;
        float hh = hash11(idx * 7.31);
        float hs = hash11(idx * 3.17 + 4.2);
        float L = 0.050 + 0.115 * hh;
        if (rimD < L){
            float u = rimD / L;
            float ph = hh * TAU * 3.0;
            // more sway toward the tip, two frequencies so it writhes
            float sway = sin(t * (0.9 + 1.9 * hs) + ph + rimD * 11.0) * 0.075 * u
                       + sin(t * (0.5 + 0.7 * hh) + ph * 1.7) * 0.032 * u;
            float aC = idx / N * TAU + sway;
            float da = abs(mod(ang - aC + 3.14159, TAU) - 3.14159);
            float sArc = da * (1.0 + rimD);
            float w = 0.0125 * (1.0 - u * 0.85) + 0.0015;
            acc = max(acc, smoothstep(w, w * 0.15, sArc) * (1.0 - u * u));
        }
    }
    return acc;
}

// fine dust hanging in the air, lit by the orb
vec3 atmosDust(vec3 ro, vec3 rt, vec3 up, vec3 fw, vec2 uv, float tSolid){
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 20; i++){
        float fi = float(i);
        float h1 = hash11(fi * 2.13 + 0.7);
        float h2 = hash11(fi * 4.71 + 3.3);
        float h3 = hash11(fi * 6.37 + 9.1);
        float ph = fract(h1 + TIME * (0.004 + 0.012 * h2));
        float ang = h1 * TAU + TIME * 0.02 * (h3 - 0.5);
        float rad = 0.8 + 3.4 * h2;
        vec3 P = vec3(cos(ang) * rad, -0.30 + ph * 3.2, sin(ang) * rad * 0.8);
        P.x += sin(TIME * 0.25 + h3 * TAU) * 0.25;
        P.z += cos(TIME * 0.21 + h1 * TAU) * 0.25;
        vec3 v = P - ro;
        float z = dot(v, fw);
        if (z > 0.3 && z < tSolid){
            vec2 sp = vec2(dot(v, rt), dot(v, up)) * (1.70 / z);
            float dd = length(uv - sp);
            float sz = 0.0016 + 0.0018 * h3;
            float tw = 0.35 + 0.65 * (0.5 + 0.5 * sin(TIME * 0.9 + h1 * TAU * 4.0));
            float fade = smoothstep(0.0, 0.10, ph) * smoothstep(1.0, 0.86, ph);
            acc += vec3(0.80, 0.72, 1.00) * exp(-dd * dd / (sz * sz)) * tw * fade * 0.45;
        }
    }
    return acc;
}

// Glyphs and motes are drawn as PROJECTED SPRITES, not sampled from
// a 3D field — depth-sampling small features renders them as
// rectangular slabs or smeared blobs.
vec3 floatingSymbols(vec3 ro, vec3 rt, vec3 up, vec3 fw, vec2 uv, float tSolid){
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 26; i++){
        float fi = float(i);
        float hs = hash11(fi * 7.3 + 1.1);
        float h2 = hash11(fi * 3.1 + 5.9);
        float h3 = hash11(fi * 11.9 + 2.7);
        float ang = fi / 26.0 * TAU + TIME * 0.13 + hs * 0.6;
        // Place on a SPHERE around the orb, not a squashed ring — a
        // ring flattened in z dips inside the glass and clips.
        float rad = 1.40 + 0.46 * h2;
        float phi = (h3 - 0.5) * 1.5 + sin(TIME * 0.5 + hs * TAU) * 0.10;
        vec3 P = BALL_C + rad * vec3(cos(ang) * cos(phi), sin(phi), sin(ang) * cos(phi));
        vec3 v = P - ro;
        float z = dot(v, fw);
        if (z > 0.2 && z < tSolid){
            vec2 sp = vec2(dot(v, rt), dot(v, up)) * (1.70 / z);
            vec2 g = (uv - sp) * (z * 0.85) * 14.0;
            g = rot(sin(TIME * 0.4 + hs * TAU) * 0.25) * g;    // slow tilt
            // unique id per instance — quantising to a handful of ids
            // made the same glyph show up several times at once
            float m = alienGlyph(g, fi * 1.37 + 0.41);
            float lit = 0.30 + 0.70 * (0.5 + 0.5 * sin(TIME * 1.3 + hs * TAU));
            vec3 sc = mix(vec3(0.95, 0.45, 1.0), vec3(0.55, 0.85, 1.0), h2);
            acc += sc * m * lit * 1.05;
            // keep the bloom tight, or each sigil smears into a blob
            acc += sc * exp(-dot(g, g) * 1.6) * lit * 0.07;
        }
    }
    return acc;
}

vec3 airMotes(vec3 ro, vec3 rt, vec3 up, vec3 fw, vec2 uv, float tSolid){
    vec3 acc = vec3(0.0);
    for (int i = 0; i < 26; i++){
        float fi = float(i);
        float h1 = hash11(fi * 1.7 + 0.3);
        float h2 = hash11(fi * 3.9 + 2.1);
        float h3 = hash11(fi * 5.3 + 7.7);
        float ph = fract(h1 + TIME * (0.020 + 0.045 * h2));
        float ang = h1 * TAU + TIME * 0.10 * (h3 - 0.5);
        float rad = 1.20 + 1.50 * h2;
        vec3 P = vec3(cos(ang) * rad, -0.25 + ph * 2.30, sin(ang) * rad * 0.55);
        P.x += sin(TIME * 0.6 + h3 * TAU) * 0.10;

        vec3 v = P - ro;
        float z = dot(v, fw);
        if (z > 0.2 && z < tSolid){
            vec2 sp = vec2(dot(v, rt), dot(v, up)) * (1.70 / z);
            float d = length(uv - sp);
            float sz = 0.0035 + 0.0035 * h3;
            float tw = 0.25 + 0.75 * (0.5 + 0.5 * sin(TIME * 2.0 + h1 * TAU * 5.0));
            float fade = smoothstep(0.0, 0.12, ph) * smoothstep(1.0, 0.72, ph);
            acc += vec3(0.95, 0.58, 1.0) * exp(-d * d / (sz * sz)) * tw * fade * 0.9;
            acc += vec3(0.70, 0.30, 0.95) * exp(-d * d / (sz * sz * 26.0)) * tw * fade * 0.16;
        }
    }
    return acc;
}

void main(){
    vec2 res = RENDERSIZE.xy;
    vec2 fragCoord = gl_FragCoord.xy;
    vec2 uv = (fragCoord - 0.5 * res) / res.y;

    // ---- camera: orbits the orb at uOrbit turns-ish per minute ----
    float oa = TIME * uOrbit * 0.35;
    vec3 ro = BALL_C + vec3(sin(oa) * 5.10, 0.58, cos(oa) * 5.10);
    vec3 ta = vec3(0.0, 0.56, 0.0);
    vec3 fw = normalize(ta - ro);
    vec3 rt = normalize(cross(vec3(0.0, 1.0, 0.0), fw));
    vec3 up = cross(fw, rt);
    vec3 rd = normalize(uv.x * rt + uv.y * up + 1.70 * fw);

    // ---- background: near-black room, barely lit by the orb ----
    float bgr = length(uv * vec2(1.0, 1.15) - vec2(0.0, 0.05));
    vec3 col = mix(vec3(0.035, 0.006, 0.055), vec3(0.004, 0.001, 0.010),
                   smoothstep(0.15, 1.05, bgr));
    col += vec3(0.18, 0.03, 0.26) * exp(-bgr * 3.4) * 0.22 * uGlow;

    // near-black nebula banks drifting behind the orb. Sampled on the
    // ray direction so they sit at infinity and parallax correctly as
    // the camera orbits.
    {
        vec3 sd = rd * 2.4 + vec3(0.0, TIME * 0.013, TIME * 0.020);
        float n1 = fbm3(sd);
        float n2 = noise3(rd * 5.6 + vec3(TIME * 0.028, 0.0, 11.0));
        float cl = smoothstep(0.46, 0.94, n1 + n2 * 0.34);
        float wisp = smoothstep(0.62, 0.98, n1 * 1.15 + n2 * 0.2);
        vec3 cc = mix(vec3(0.014, 0.004, 0.026), vec3(0.060, 0.016, 0.098), cl);
        col += cc * cl * 1.15 * uSkyNeb;
        col += vec3(0.085, 0.030, 0.135) * wisp * 0.55 * uSkyNeb;
    }

    vec3 bgCol = col;                                    // for horizon blending

    // ---- analytic sphere intersection ----
    vec3 oc = ro - BALL_C;
    float b = dot(oc, rd);
    float c2 = dot(oc, oc) - BALL_R * BALL_R;
    float disc = b * b - c2;
    float tBall = disc > 0.0 ? -b - sqrt(disc) : 1e9;

    // ---- ground heightfield march ----
    float tGround = 1e9;
    {
        float t = 0.6;
        for (int i = 0; i < 96; i++){
            if (tGround > 1e8 && t <= 14.0){
                vec3 p = ro + rd * t;
                float h = p.y - groundH(p.xz);
                if (h < 0.0015) tGround = t;
                else t += max(h * 0.5, 0.022);
            }
        }
    }

    bool hitBall   = tBall < tGround;
    bool hitGround = tGround < 1e8 && !hitBall;

    // ---- ground shading ----
    if (hitGround){
        vec3 p = ro + rd * tGround;
        vec3 n = groundN(p.xz);
        vec3 toBall = BALL_C - p;
        float dist = length(toBall);
        vec3 l = toBall / dist;

        float diff = max(dot(n, l), 0.0);
        float fall = 1.8 / (1.0 + dist * dist * 1.15);
        vec3 lit = mix(vec3(0.42, 0.10, 0.60), vec3(0.85, 0.38, 0.95),
                       smoothstep(2.2, 0.6, dist));

        // regional colour variation across the terrain
        float cv = fbm3(vec3(p.xz * 0.42, 3.1));
        float cv2 = fbm3(vec3(p.xz * 0.9 + 21.0, 8.4));
        // keep the variation inside the violet family, or the terrain
        // drifts olive and fights the mood
        vec3 tint = mix(vec3(0.26, 0.06, 0.38), vec3(0.12, 0.10, 0.40), smoothstep(0.35, 0.72, cv));
        tint = mix(tint, vec3(0.34, 0.08, 0.30), smoothstep(0.55, 0.85, cv2));
        vec3 flat_ = vec3(0.24, 0.06, 0.36);
        vec3 base = mix(flat_, tint, clamp(uGroundCol, 0.0, 2.0)) *
                    (0.65 + 0.5 * fbm3(vec3(p.xz * 5.5, 1.7)));

        col = base * 0.22 + base * lit * diff * fall * 4.2 * uGlow;

        // --- sheen: a velvet-ish response rather than a flat ramp ---
        vec3 vdir = -rd;
        vec3 hv = normalize(l + vdir);
        float ndv = max(dot(n, vdir), 0.0);
        float ndl = max(dot(n, l), 0.0);
        float ndh = max(dot(n, hv), 0.0);

        // retroreflective rim: sharp at grazing, near-zero head-on
        float rimS = pow(1.0 - ndv, 5.0);
        // Ashikhmin-style sheen lobe — the horizon scatter of fibres
        float sinTh = sqrt(max(1.0 - ndh * ndh, 0.0));
        float lobe = pow(sinTh, 9.0) * (2.0 + 9.0) / (2.0 * 3.14159);
        // break the highlight up so it doesn't read as a smooth gradient
        float brk = 0.55 + 0.45 * fbm3(vec3(p.xz * 7.5, 4.2));
        float brk2 = 0.65 + 0.35 * fbm3(vec3(p.xz * 21.0, 9.6));

        col += vec3(0.75, 0.35, 0.95) * rimS * (0.15 + 0.85 * ndl) * brk * 0.70 * uSheen;
        col += vec3(0.85, 0.55, 1.00) * lobe * ndl * brk2 * 0.26 * uSheen;
        // tight GGX glint on the crests
        float a2 = 0.30 * 0.30;
        float dgg = ndh * ndh * (a2 - 1.0) + 1.0;
        col += vec3(0.95, 0.72, 1.0) * (a2 / (3.14159 * dgg * dgg)) * ndl * 0.028 * uSheen;

        // faint emissive veins glowing in the rock
        float vein = smoothstep(0.72, 0.95, fbm3(vec3(p.xz * 1.7 + vec2(0.0, TIME * 0.03), 5.5)));
        col += hueShift(vec3(0.40, 0.85, 0.95), uHueGround) * vein * 0.22 * uGroundCol;

        // the carved spirals glow, so the figures actually read
        float gs = groundSpiral(p.xz);
        float gsPulse = 0.55 + 0.45 * sin(TIME * 0.8 - length(p.xz) * 1.6);
        col += hueShift(vec3(0.55, 0.90, 1.0), uHueGround) *
               smoothstep(0.45, 0.98, gs) * gsPulse * 0.30 * uGndSpiral;

        // contact shadow where the orb sits
        float sh = smoothstep(0.60, 1.7, length(p.xz));
        col *= 0.16 + 0.84 * sh;
        col += vec3(0.85, 0.30, 0.95) * exp(-length(p.xz) * 1.9) * 0.22 * uGlow;

        col = hueShift(col, uHueGround);

        // blend fully to the background colour — anything darker
        // leaves a visible horizon band across the frame
        float fog = 1.0 - exp(-max(tGround - 3.7, 0.0) * 0.55);
        col = mix(col, bgCol, fog);
    }

    // ---- the crystal ball ----
    if (hitBall){
        vec3 p = ro + rd * tBall;
        vec3 n = normalize(p - BALL_C);
        float fres = pow(1.0 - max(dot(n, -rd), 0.0), 3.0);

        // refract into the glass, then march the interior volume
        vec3 rr = refract(rd, n, 1.0 / max(uIOR, 1.0));
        float b2 = dot(p - BALL_C, rr);
        float exitT = -b2 + sqrt(max(b2 * b2 - (dot(p - BALL_C, p - BALL_C) - BALL_R * BALL_R), 0.0));
        vec3 inner = nebula(p, rr, 0.02, exitT, 1.0);

        col = contrastify(inner, max(uContrast, 0.0));
        col *= mix(vec3(1.0), vec3(0.72, 0.55, 0.95), fres * 0.8);
        col += vec3(0.30, 0.10, 0.45) * fres * 0.55;

        // animated iridescence over the shell
        vec3 iri = iridescence(fres, TIME * 0.13 + n.y * 1.4 + n.x * 0.6);
        col += iri * fres * 0.55 * uIrid;

        // bright rim, tinted by the film
        float rim = pow(1.0 - max(dot(n, -rd), 0.0), 9.0);
        col += mix(vec3(0.72, 1.00, 0.92), iri, 0.55 * uIrid) * rim * 1.5;

        // --- glass reflection: the ground bounces in the lower shell
        vec3 refl = reflect(rd, n);
        vec3 rcol;
        if (refl.y < -0.02){
            float tg = (-0.05 - p.y) / refl.y;
            vec3 gp = p + refl * tg;
            float gr = length(gp.xz);
            vec3 gb = vec3(0.24, 0.06, 0.36) * (0.55 + 0.75 * fbm3(vec3(gp.xz * 2.2, 1.7)));
            rcol = gb * (0.35 + 2.2 * exp(-gr * 0.45));
            rcol += vec3(0.85, 0.30, 0.95) * exp(-gr * 1.1) * 0.55;
            rcol += hueShift(vec3(0.55, 0.90, 1.0), uHueGround) *
                    exp(-pow(spiralArm(gp.xz, 0.320, TIME * 0.05) * 9.0, 2.0)) * 0.35 * uGndSpiral;
            rcol = hueShift(rcol, uHueGround);
            rcol *= exp(-max(gr - 2.0, 0.0) * 0.35);
        } else {
            rcol = vec3(0.06, 0.012, 0.11);
        }
        // schlick-ish weight: strongest at grazing and on the underside
        float rw = (0.06 + 0.94 * fres) * (0.30 + 0.70 * smoothstep(0.35, -0.8, n.y));
        col += rcol * rw * 1.30 * uReflect;

        // a single small specular hot spot keeps the glass reading
        col += vec3(1.0) * pow(max(dot(reflect(rd, n), normalize(vec3(-0.55, 0.75, 0.45))), 0.0), 90.0) * 0.7;
        col += vec3(0.55, 0.18, 0.62) * smoothstep(0.1, -0.9, n.y) * fres * 0.5;
    }

    float tSolid = min(tBall, tGround);

    // ---- atmosphere, smoke and trails around the orb -------------
    {
        // closest approach of this ray to the orb centre, and the
        // screen-space angle around it — the basis for the corona
        float tc = max(dot(BALL_C - ro, rd), 0.0);
        vec3 cp = ro + rd * tc - BALL_C;
        float dc = length(cp);
        float rimD = dc - BALL_R;                        // >0 outside the orb
        float ang = atan(dot(cp, up), dot(cp, rt));

        float halo = exp(-max(rimD, 0.0) * 5.5);
        col += vec3(0.55, 0.14, 0.75) * halo * 0.16 * uGlow;

        if (rimD > 0.0 && tc < tSolid){
            vec3 fx = vec3(0.0);

            // magical smoke boiling off the perimeter: fbm in polar
            // coords, scrolling outward and slowly rotating
            // tendrils, not haze: a high threshold keeps it wispy and
            // a steep falloff keeps it hugging the glass
            float sm = fbm3(vec3(cos(ang) * 2.6, sin(ang) * 2.6,
                                 rimD * 3.2 - TIME * 0.34));
            sm += 0.5 * fbm3(vec3(cos(ang) * 5.2, sin(ang) * 5.2,
                                  rimD * 4.6 - TIME * 0.52 + 9.1));
            sm = smoothstep(0.72, 1.12, sm);
            sm *= exp(-rimD * 11.0) * smoothstep(0.0, 0.02, rimD);
            fx += mix(vec3(0.60, 0.20, 0.95), vec3(0.30, 0.75, 0.95),
                      0.5 + 0.5 * sin(TIME * 0.4 + ang)) * sm * 2.4 * uSmoke;

            // writhing tentacle fingers rooted on the rim
            float tf = tentacleFringe(ang, rimD, TIME);
            fx += mix(vec3(0.85, 0.35, 1.0), vec3(0.40, 0.85, 1.0),
                      0.5 + 0.5 * sin(ang * 3.0 + TIME * 0.5)) * tf * 1.35 * uTentacle;

            // wispy trails: thin wavering ribbons orbiting the orb
            for (int k = 0; k < 3; k++){
                float fk = float(k);
                float r0 = 0.045 + 0.065 * fk;
                float wob = 0.026 * sin(ang * 2.0 + TIME * (0.7 + 0.25 * fk) + fk * 2.1)
                          + 0.016 * sin(ang * 5.0 - TIME * 0.9 + fk);
                float band = exp(-pow((rimD - (r0 + wob)) * 52.0, 2.0));
                // break the ribbon into wisps
                float br = fbm3(vec3(cos(ang) * 3.2, sin(ang) * 3.2,
                                     TIME * 0.25 + fk * 4.0));
                band *= smoothstep(0.46, 0.86, br);        // wispier breakup
                fx += mix(vec3(0.85, 0.45, 1.0), vec3(0.45, 0.90, 1.0), fk / 2.0)
                      * band * 0.95 * uTrails;
            }

            col += hueShift(fx, uHueFx);
        }
    }

    // ---- floating sigils and drifting motes ----------------------
    col += hueShift(floatingSymbols(ro, rt, up, fw, uv, tSolid), uHueFx) * uSymbols;

    // low mist creeping over the ground
    {
        float mistY = 0.02;
        float tm = (mistY - ro.y) / min(rd.y, -1e-3);
        if (tm > 0.0 && tm < tSolid){
            vec3 mp = ro + rd * tm;
            float m = fbm3(vec3(mp.xz * 0.85, TIME * 0.05));
            m = smoothstep(0.42, 0.82, m);
            m *= exp(-max(length(mp.xz) - 0.6, 0.0) * 0.55);
            m *= smoothstep(7.0, 2.5, tm);
            col += hueShift(vec3(0.42, 0.14, 0.60), uHueFx) * m * 0.42 * uMotes;
        }
    }

    col += hueShift(airMotes(ro, rt, up, fw, uv, tSolid), uHueFx) * uMotes;
    col += hueShift(atmosDust(ro, rt, up, fw, uv, tSolid), uHueFx) * uAtmos;

    // ---- finish ----
    col = hueShift(col, uHue);
    vec2 vc = (fragCoord / res - 0.5) * vec2(res.x / res.y, 1.0);
    col *= 1.0 - uVignette * 1.05 * dot(vc, vc);
    col = pow(max(col, vec3(0.0)), vec3(0.86));
    col = col / (1.0 + col * 0.30);                       // gentle rolloff
    gl_FragColor = vec4(col, 1.0);
}