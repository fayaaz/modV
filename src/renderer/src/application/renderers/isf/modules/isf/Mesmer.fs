/*{
  "DESCRIPTION": "A black-and-white op-art engine built to be stared at. Everything is computed in log-polar space, where a straight line becomes a logarithmic spiral and simply scrolling the radius is an endless zoom with no seam to repeat. Four harmonics lie over one another - the spiral itself, concentric rings marching outward, radial wedges, and a pair of gratings set a hair out of alignment so they beat against each other into moire - and the sum is squared up into hard black and white. A crossed sine domain warp drags the whole plane around, so the pattern swims rather than merely turning. Every harmonic knows its own screen-space frequency and fades itself toward grey as it approaches one cycle per pixel, which is what keeps the infinite zoom clean instead of boiling into aliasing at the vanishing point. Twenty-four sliders: speed, zoom rate (negative falls inward), spin, radial tightness and arm count, the four layer amounts, warp, breathing, a hardness dial from pure sine to hard square edge, contrast, invert, vignette and grain, plus four more optical effects — Fraser twisted-cord hatching that makes concentric bands read as one continuous spiral, a cafe-wall row offset that sets straight bands leaning, a roving lens that magnifies whatever it wanders over, and a 45-degree halftone dot screen that rebuilds the tone the way it would come off a press. Four more push it further out: a kaleidoscope fold that mirrors the angle into N sectors and turns the spiral into a mandala, self-similar fractal octaves so every band grows bands of its own all the way down, a turbulent fbm warp that makes the plane boil rather than merely sway, and travelling rings of inverted polarity so the whole image keeps turning itself inside out.",
  "CREDIT": "CC0",
  "CATEGORIES": [
    "generator",
    "op-art",
    "black-and-white",
    "hypnotic",
    "spiral",
    "moire",
    "2d"
  ],
  "INPUTS": [
    {
      "NAME": "uSpeed",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uZoom",
      "TYPE": "float",
      "DEFAULT": 0.55,
      "MIN": -2.0,
      "MAX": 2.0
    },
    {
      "NAME": "uSpin",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": -2.0,
      "MAX": 2.0
    },
    {
      "NAME": "uTight",
      "TYPE": "float",
      "DEFAULT": 6.0,
      "MIN": 0.5,
      "MAX": 14.0
    },
    {
      "NAME": "uArms",
      "TYPE": "float",
      "DEFAULT": 5.0,
      "MIN": 0.0,
      "MAX": 16.0
    },
    {
      "NAME": "uSpiral",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uRings",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uWedges",
      "TYPE": "float",
      "DEFAULT": 0.25,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uMoire",
      "TYPE": "float",
      "DEFAULT": 0.12,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uWarp",
      "TYPE": "float",
      "DEFAULT": 0.12,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uPulse",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uSharp",
      "TYPE": "float",
      "DEFAULT": 0.72,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uContrast",
      "TYPE": "float",
      "DEFAULT": 0.55,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uInvert",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uVignette",
      "TYPE": "float",
      "DEFAULT": 0.4,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGrain",
      "TYPE": "float",
      "DEFAULT": 0.12,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uFraser",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uCafe",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uLens",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uHalftone",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uKaleido",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 16.0
    },
    {
      "NAME": "uFractal",
      "TYPE": "float",
      "DEFAULT": 0.45,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uTurb",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uFlip",
      "TYPE": "float",
      "DEFAULT": 0.4,
      "MIN": 0.0,
      "MAX": 1.0
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "2D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/vdstz3q9


// OCULAR - ISF port of the Shadertoy/WebGL2 version
// (GLSL ES 1.00: gl_FragColor, TIME/RENDERSIZE).

#define PI 3.14159265359

float hash12(vec2 p){
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

float vnoise2(vec2 p){
    vec2 i = floor(p), f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

float fbm2(vec2 p){
    float s = 0.0, a = 0.5;
    for (int i = 0; i < 4; i++){
        s += a * vnoise2(p);
        p *= 2.03;
        a *= 0.5;
    }
    return s;
}

// Cycles per pixel for a harmonic whose phase gradient has the given
// magnitude (radians per screen unit). px is screen units per pixel.
float cpp(float gradMag, float px){ return gradMag * px * (0.5 / PI); }

// A wave that squares up as `sharp` rises, but softens back toward a
// sine — and then toward flat grey — as it approaches Nyquist. This is
// what keeps the infinite zoom from turning into a boiling mess.
float wave(float phase, float fpp, float sharp){
    float gain = mix(1.0, 16.0, sharp) / (1.0 + fpp * 30.0);
    float s = clamp(sin(phase) * gain, -1.0, 1.0);
    return s * smoothstep(0.62, 0.16, fpp);      // fade out past Nyquist
}

void main(){
    vec2 fragCoord = gl_FragCoord.xy;
    vec2  R  = RENDERSIZE;
    vec2  uv = (fragCoord - 0.5 * R) / R.y;
    float px = 1.0 / R.y;
    float t  = TIME * uSpeed;

    // breathing — the whole field swells and contracts
    float breathe = 1.0 + 0.16 * uPulse * sin(t * 0.63);
    uv *= breathe;
    px *= breathe;

    // domain warp: two crossed sine fields dragging the plane around.
    // This is what turns a static pattern into something that swims.
    if (uWarp > 0.001){
        float w = 0.14 * uWarp;
        uv += w * vec2(sin(uv.y * 4.7 - t * 0.87), cos(uv.x * 4.3 + t * 0.79));
    }

    // turbulence: fbm dragging the plane around on top of the clean sine warp.
    // The sine warp is orderly and periodic; this one is not, and that is what
    // makes the pattern boil rather than merely sway.
    if (uTurb > 0.001){
        vec2 q1 = vec2(fbm2(uv * 2.1 + vec2(0.0, t * 0.13)),
                       fbm2(uv * 2.1 + vec2(5.2, -t * 0.11)));
        vec2 q2 = vec2(fbm2(uv * 4.7 + q1 * 1.4 - t * 0.07),
                       fbm2(uv * 4.7 + q1 * 1.4 + vec2(3.1, t * 0.09)));
        uv += uTurb * 0.30 * (q2 - 0.5);
    }

    // a roving lens — pulls the plane toward its own centre, which reads as
    // local magnification wandering across the field
    if (uLens > 0.001){
        vec2  lc = vec2(0.34 * sin(t * 0.37), 0.30 * cos(t * 0.29));
        vec2  d  = uv - lc;
        float dd = dot(d, d);
        uv -= d * uLens * 0.55 * exp(-dd * 11.0);
    }

    float r  = max(length(uv), 1e-4);
    float lr = log(r) + t * uZoom;              // endless zoom, seam-free
    float an = atan(uv.y, uv.x) + t * uSpin;    // rotation

    // kaleidoscope — mirror the angle into N sectors. The spiral stops being
    // a spiral and becomes a mandala that folds in on itself.
    if (uKaleido > 0.5){
        float sec = 2.0 * PI / floor(uKaleido);
        an = abs(mod(an, sec) - sec * 0.5);
    }

    // ---- ONE binary field, whose phase the other layers ripple ----
    // Summing several hard square waves and then thresholding does not work:
    // the sum lands on discrete plateaus and the contrast stage turns those
    // into blocky cells bounded by ring arcs and wedge rays. Feeding the
    // layers into the phase instead keeps a single clean edge that undulates.
    float kR = uTight;                 // radial frequency
    float kA = uArms * uSpiral;        // angular frequency — 0 gives plain rings
    float phase = kR * lr + kA * an;

    // radial ripple — the arms pump in and out as it zooms
    if (uRings > 0.001)
        phase += uRings * 1.15 * sin(uTight * 1.6 * lr - t * 1.1);

    // angular ripple — scallops the edges of every arm
    if (uWedges > 0.001)
        phase += uWedges * 0.85 * sin(uArms * 2.0 * an);

    // moire ripple — two fine gratings beating, dragged through the phase
    if (uMoire > 0.001){
        float ga = t * 0.19;
        float f  = 320.0;
        float fm = cpp(f, px);
        float g1 = wave(dot(uv, vec2(cos(ga),        sin(ga)))        * f, fm, 0.0);
        float g2 = wave(dot(uv, vec2(cos(ga + 0.45), sin(ga + 0.45))) * f, fm, 0.0);
        phase += uMoire * 2.2 * g1 * g2;
    }

    // cafe wall — shove alternate radial bands a half-cell round. Straight
    // concentric bands stop looking concentric and start to lean.
    if (uCafe > 0.001){
        float row = floor(uTight * 1.6 * lr);
        phase += uCafe * 1.6 * (mod(row, 2.0) - 0.5);
    }

    // Fraser — twisted cord. Fine oblique hatching laid along the bands at 45
    // degrees in log-polar space. It is what makes a ring of concentric
    // circles read as one continuous spiral even when it is not.
    float kF = 9.0;
    float ampF = uFraser * 1.15;
    if (uFraser > 0.001)
        phase += ampF * sin(kF * (an + lr) - t * 0.6);

    // fractal octaves — the same spiral again at two and four times the
    // frequency, so every band grows bands of its own. Self-similar all the
    // way down, which pairs with the endless zoom: you never reach a bottom.
    float base = kR * lr + kA * an;
    if (uFractal > 0.001){
        phase += uFractal * 0.75 * sin(2.0 * base - t * 0.5);
        phase += uFractal * 0.34 * sin(4.0 * base + t * 0.8);
    }

    // the ripples steepen the phase, so inflate the Nyquist estimate to match.
    // The cord contributes amplitude x frequency, NOT its raw frequency —
    // using the latter over-estimates badly and fades the cord to grey.
    float grad = sqrt(kR * kR + kA * kA) / r;
    grad += ampF * kF * 1.41 / r;                        // twisted cord
    // The octaves' worst-case gradient (all derivatives aligned) is ~2.9x the
    // base, but summing worst cases makes the AA far too eager and washes the
    // whole mid-field to grey. An RMS-ish half of that tracks reality.
    grad += uFractal * 1.30 * sqrt(kR * kR + kA * kA) / r;
    float fpp  = cpp(grad, px) * (1.0 + 1.5 * uRings + 1.2 * uWedges + 1.5 * uCafe);
    float v = wave(phase, fpp, uSharp);

    // polarity waves — rings of inversion travelling outward, so the image
    // keeps turning itself inside out. The slider sets how much of each cycle
    // inverts, NOT how strongly: blending toward -1 would just dim the field
    // to grey. Inside a band it is always a full flip.
    if (uFlip > 0.001){
        float fw  = sin(lr * 2.1 - t * 1.7);
        float thr = 1.0 - 2.0 * uFlip;
        v *= 1.0 - 2.0 * smoothstep(thr + 0.05, thr - 0.05, fw);
    }

    // ---- square it up into black and white ----
    v = clamp(v * mix(1.0, 5.0, uContrast), -1.0, 1.0);
    float lum = 0.5 + 0.5 * v;

    // halftone — rebuild the tone out of dots on a 45-degree screen, the way
    // it would come off a press. This has to read the RAW sine, not the
    // squared-up field: that one is already saturated to black and white, so
    // there is no continuous tone left for the dot size to track and you get
    // ragged edges instead of dots.
    if (uHalftone > 0.001){
        float tone = sin(phase) * smoothstep(0.62, 0.16, fpp);
        float ha = 0.7854;                                  // 45 degrees
        vec2  hp = vec2(fragCoord.x * cos(ha) - fragCoord.y * sin(ha),
                        fragCoord.x * sin(ha) + fragCoord.y * cos(ha)) * 0.34;
        float dot2 = sin(hp.x) * sin(hp.y);                 // the dot lattice
        float dotted = smoothstep(-0.08, 0.08, tone * 1.15 - dot2 * 0.92);
        lum = mix(lum, dotted, uHalftone);
    }

    lum = mix(lum, 1.0 - lum, uInvert);

    // a dark pupil at the vanishing point, so the eye has somewhere to fall
    lum *= smoothstep(0.0, 0.055, r);

    vec3 col = vec3(lum);

    // vignette
    vec2 q = fragCoord / R;
    float vig = pow(16.0 * q.x * q.y * (1.0 - q.x) * (1.0 - q.y), 0.28);
    col *= mix(1.0, vig, uVignette);

    // grain
    col += (hash12(fragCoord + fract(TIME) * 311.0) - 0.5) * uGrain * 0.10;

    gl_FragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
