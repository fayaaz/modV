/*{
  "DESCRIPTION": "Infinite neon fractal maze flythrough: a solid universe with corridors carved at three nested scales so every wall is itself a smaller maze. The camera threads the lattice with rounded right-angle turns; convex edges glow, sparse wall panels are lit windows, circuit lines crawl the grooves, a coloured scout light drifts ahead. Every knob exposed.",
  "CREDIT": "CC0",
  "CATEGORIES": [
    "generator",
    "abstract",
    "3d"
  ],
  "INPUTS": [
    {
      "NAME": "speed",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "fov",
      "TYPE": "float",
      "DEFAULT": 1.39,
      "MIN": 0.4,
      "MAX": 2.0
    },
    {
      "NAME": "turns",
      "TYPE": "float",
      "DEFAULT": 1.63,
      "MIN": 0.0,
      "MAX": 2.0
    },
    {
      "NAME": "smoothness",
      "TYPE": "float",
      "DEFAULT": 0.6,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "randomness",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "corrWidth",
      "TYPE": "float",
      "DEFAULT": 0.91,
      "MIN": 0.5,
      "MAX": 1.4
    },
    {
      "NAME": "density",
      "TYPE": "float",
      "DEFAULT": 1.5,
      "MIN": 0.0,
      "MAX": 1.5
    },
    {
      "NAME": "detail",
      "TYPE": "float",
      "DEFAULT": 0.97,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "edgeGlow",
      "TYPE": "float",
      "DEFAULT": 0.1,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "circuits",
      "TYPE": "float",
      "DEFAULT": 1.67,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "windows",
      "TYPE": "float",
      "DEFAULT": 2.36,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "headlight",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "hue",
      "TYPE": "float",
      "DEFAULT": 0.58,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "hueCycle",
      "TYPE": "float",
      "DEFAULT": 0.21,
      "MIN": 0.0,
      "MAX": 0.3
    },
    {
      "NAME": "depthHue",
      "TYPE": "float",
      "DEFAULT": 1.28,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "saturate",
      "TYPE": "float",
      "DEFAULT": 1.54,
      "MIN": 0.0,
      "MAX": 2.0
    },
    {
      "NAME": "fog",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/gstbkfmm


// FRACTAL MAZE — ISF port of the Shadertoy/WebGL2 version.
// GLSL ES 1.00: constant loop bounds, gl_FragColor, RENDERSIZE/TIME.

#define TAU 6.28318530718

#define uSpeed    speed
#define uFov      fov
#define uTurns    turns
#define uSmooth   smoothness
#define uRandom   randomness
#define uWidth    corrWidth
#define uDensity  density
#define uDetail   detail
#define uEdgeGlow edgeGlow
#define uStrips   circuits
#define uWindows  windows
#define uLight    headlight
#define uHue      hue
#define uHueCycle hueCycle
#define uZHue     depthHue
#define uSat      saturate
#define uFog      fog

#define iTime TIME

const float CELL = 3.0;

// ---- hashes / palette --------------------------------------------
float hash21(vec2 p){
    p = fract(p * vec2(234.34, 435.345));
    p += dot(p, p + 34.23);
    return fract(p.x * p.y);
}

float hash31(vec3 p){
    p = fract(p * 0.1031);
    p += dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
}

vec3 palette(float h){
    return 0.5 + 0.5 * cos(TAU * (h + vec3(0.0, 0.33, 0.67)));
}

// 1D value noise for the irregular camera-path wander
float hash11(float p){ return fract(sin(p * 127.1) * 43758.5453123); }
float vnoise(float x){
    float i = floor(x), f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    return mix(hash11(i), hash11(i + 1.0), f);
}

// ---- camera path -------------------------------------------------
float stairs(float x){
    float f = fract(x);
    float hw = mix(0.14, 0.5, clamp(uSmooth, 0.0, 1.0));
    return floor(x) + smoothstep(0.5 - hw, 0.5 + hw, f);
}

vec3 pathPos(float z){
    float zc = z / CELL;
    float sgx = 1.0 * sin(zc * 0.19) + 0.7 * sin(zc * 0.47 + 2.7);
    float sgy = 0.85 * sin(zc * 0.15 + 5.0) + 0.45 * sin(zc * 0.33 + 1.0);
    float ngx = ((vnoise(zc / 3.5  + 3.0)  - 0.5) + 0.5 * (vnoise(zc / 1.575 + 9.0)  - 0.5)) * 1.60;
    float ngy = ((vnoise(zc / 3.9  + 21.0) - 0.5) + 0.5 * (vnoise(zc / 1.75  + 40.0) - 0.5)) * 1.35;
    float gx = mix(sgx, ngx, uRandom) * uTurns;
    float gy = mix(sgy, ngy, uRandom) * uTurns;
    return vec3((stairs(gx) + 0.5) * CELL, (stairs(gy) + 0.5) * CELL, z);
}

// ---- the maze ----------------------------------------------------
// Corridors reach through any cell face whose symmetric hash rolls
// open, each half-tube overshooting the shared face by w so adjacent
// cells describe identical geometry at the seam (continuous field).
float carveScale(vec3 p, float s, float prob, float w, float seed){
    vec3 id = floor(p / s);
    vec3 f = p - (id + 0.5) * s;
    float hs = 0.5 * s;
    float e = hs + w;
    float d = 1e9;

    float crx = max(abs(f.y), abs(f.z)) - w;
    if (hash31(id + vec3( 0.5, 0.0, 0.0) + seed) < prob)
        d = min(d, max(crx, max( f.x - e, -f.x - w)));
    if (hash31(id + vec3(-0.5, 0.0, 0.0) + seed) < prob)
        d = min(d, max(crx, max(-f.x - e,  f.x - w)));

    float sz = seed + 33.17;
    float crz = max(abs(f.x), abs(f.y)) - w;
    if (hash31(id + vec3(0.0, 0.0,  0.5) + sz) < prob)
        d = min(d, max(crz, max( f.z - e, -f.z - w)));
    if (hash31(id + vec3(0.0, 0.0, -0.5) + sz) < prob)
        d = min(d, max(crz, max(-f.z - e,  f.z - w)));

    float sy = seed + 71.73;
    float py = prob * 0.45;
    float cry = max(abs(f.x), abs(f.z)) - w;
    if (hash31(id + vec3(0.0,  0.5, 0.0) + sy) < py)
        d = min(d, max(cry, max( f.y - e, -f.y - w)));
    if (hash31(id + vec3(0.0, -0.5, 0.0) + sy) < py)
        d = min(d, max(cry, max(-f.y - e,  f.y - w)));

    return d;
}

float map(vec3 p){
    float w0 = 0.34 * CELL * uWidth;

    float carve = carveScale(p, CELL, 0.58 * uDensity, w0, 0.0);

    // guaranteed corridor along the camera path; scaled by 0.2 to stay
    // Lipschitz-safe near turns (keeps the raymarch from over-stepping
    // and punching sharp "blade" shards through the walls). Scaling an
    // SDF by a positive constant does not move its zero-crossing.
    vec3 pc = pathPos(p.z);
    carve = min(carve, (max(abs(p.x - pc.x), abs(p.y - pc.y)) - w0) * 0.2);

    // nested detail: the walls are themselves mazes
    if (uDetail > 0.25){
        float s1 = CELL / 3.0;
        carve = min(carve, carveScale(p, s1, 0.42 * uDensity, 0.30 * s1 * uWidth, 101.3));
    }
    if (uDetail > 0.75){
        float s2 = CELL / 9.0;
        carve = min(carve, carveScale(p, s2, 0.45 * uDensity, 0.32 * s2 * uWidth, 57.7));
    }
    return -carve;
}

vec3 calcNormal(vec3 p){
    vec2 e = vec2(0.0015, -0.0015);
    return normalize(e.xyy * map(p + e.xyy) + e.yyx * map(p + e.yyx) +
                     e.yxy * map(p + e.yxy) + e.xxx * map(p + e.xxx));
}

float calcAO(vec3 p, vec3 n){
    float occ = 0.0;
    float sca = 1.0;
    for (int i = 0; i < 5; i++){
        float h = 0.02 + 0.11 * float(i);
        occ += (h - map(p + n * h)) * sca;
        sca *= 0.72;
    }
    return clamp(1.0 - 1.4 * occ, 0.0, 1.0);
}

float edgeGlowAt(vec3 p, float h){
    float d0 = map(p);
    float s = map(p + vec3( h, 0.0, 0.0)) + map(p - vec3( h, 0.0, 0.0))
            + map(p + vec3(0.0,  h, 0.0)) + map(p - vec3(0.0,  h, 0.0))
            + map(p + vec3(0.0, 0.0,  h)) + map(p - vec3(0.0, 0.0,  h));
    return clamp((s / 6.0 - d0) / h * 4.0, 0.0, 1.0);
}

vec3 shade(vec3 ro, vec3 rd, float t, float zt, float hue0){
    vec3 p = ro + rd * t;
    vec3 n = calcNormal(p);
    float ao = calcAO(p, n);
    vec3 an = abs(n);
    float hue = hue0 + p.z * 0.012 * uZHue;
    float dfade = exp(-t * 0.06);

    float tint = hash31(floor(p / CELL) + 7.7);
    vec3 alb = mix(vec3(0.16, 0.17, 0.20), vec3(0.23, 0.21, 0.27), tint);

    float gs = 0.5;
    vec3 fr = fract(p / gs);
    vec3 dl = gs * min(fr, 1.0 - fr);
    float lm = 0.0;
    lm = max(lm, (1.0 - smoothstep(0.006, 0.028, dl.x)) * (1.0 - an.x));
    lm = max(lm, (1.0 - smoothstep(0.006, 0.028, dl.y)) * (1.0 - an.y));
    lm = max(lm, (1.0 - smoothstep(0.006, 0.028, dl.z)) * (1.0 - an.z));
    lm *= dfade;

    float lh = hash31(floor(p / gs) + 13.7);
    float circuit = lm * step(0.72, lh) * (0.7 + 0.3 * sin(iTime * 2.0 + lh * TAU * 3.0));

    vec3 wq = abs(fract(p) - 0.5);
    float inset = max(wq.x * (1.0 - an.x), max(wq.y * (1.0 - an.y), wq.z * (1.0 - an.z)));
    float wh = hash31(floor(p) + 91.7);
    float thr = 1.0 - 0.11 * uWindows;
    float wmask = (1.0 - smoothstep(0.27, 0.33, inset)) * step(thr, wh)
                * (0.65 + 0.35 * sin(iTime * 1.5 + wh * TAU * 5.0));

    float trim = edgeGlowAt(p, 0.06) * uEdgeGlow;

    vec3 lp = ro + vec3(0.0, 0.2, 0.0);
    vec3 ld = lp - p;
    float lr = max(length(ld), 1e-3);
    ld /= lr;
    float att = 1.0 / (1.0 + 0.14 * lr * lr);
    float dif = max(dot(n, ld), 0.0) * att;
    float spe = pow(max(dot(reflect(rd, n), ld), 0.0), 24.0) * att;

    vec3 lp2 = pathPos(zt + 9.0 + 3.0 * sin(iTime * 0.4));
    vec3 ld2 = lp2 - p;
    float lr2 = max(length(ld2), 1e-3);
    float att2 = 1.0 / (1.0 + 0.20 * lr2 * lr2);
    float dif2 = max(dot(n, ld2 / lr2), 0.0) * att2;

    vec3 neon = palette(hue);
    vec3 col = alb * (0.05 + 1.5 * dif * uLight) * ao;
    col += alb * palette(hue + 0.45) * dif2 * 2.2 * uLight;
    col *= 1.0 - 0.45 * lm;
    col += vec3(0.9) * spe * 0.5 * uLight * ao;
    col += neon * trim * 1.5 * (0.3 + 0.7 * ao);
    col += neon * circuit * 1.1 * uStrips;
    col += palette(hue + 0.13 + wh * 0.35) * wmask * 2.0;
    return col;
}

void main(){
    vec2 fragCoord = gl_FragCoord.xy;
    vec2 uv = (2.0 * fragCoord - RENDERSIZE.xy) / RENDERSIZE.y;

    float zt = iTime * 3.2 * uSpeed + 0.4;
    vec3 ro = pathPos(zt);
    vec3 ta = pathPos(zt + 2.4);
    vec3 fw = normalize(ta - ro);

    float lean = clamp(-0.22 * (pathPos(zt + 2.8).x - ro.x) / CELL, -0.35, 0.35);
    vec3 wup = vec3(sin(lean), cos(lean), 0.0);
    vec3 rt = normalize(cross(fw, wup));
    vec3 up = cross(rt, fw);
    float focal = 1.15 / max(uFov, 0.25);
    vec3 rd = normalize(uv.x * rt + uv.y * up + fw * focal);

    float hue0 = uHue + iTime * uHueCycle;

    const float TMAX = 48.0;
    float t = 0.0;
    float d = 0.0;
    for (int i = 0; i < 150; i++){
        d = map(ro + rd * t);
        if (d < 0.0013 * (1.0 + t * 1.2) || t > TMAX) break;
        t += d * 0.8;
    }

    bool hit = t < TMAX && d < 0.05;
    vec3 fogc = palette(hue0 + t * 0.008 * uZHue) * 0.10 + vec3(0.010, 0.012, 0.020);
    vec3 col;
    if (hit){
        col = shade(ro, rd, t, zt, hue0);
        float fga = 1.0 - exp(-t * t * 0.0014 * uFog);
        col = mix(col, fogc, fga);
    } else {
        col = fogc;
    }

    float lum = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(lum), col, uSat);
    col = 1.0 - exp(-col * 1.7);
    col = pow(col, vec3(0.4545));
    col *= 1.0 - 0.28 * dot(uv * 0.55, uv * 0.55);
    col += (hash21(fragCoord + fract(iTime)) - 0.5) / 256.0;
    gl_FragColor = vec4(col, 1.0);
}
