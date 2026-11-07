/*{
  "DESCRIPTION": "Infinite laser corridor: an endlessly-tiled Menger KIFS tunnel flown straight down its central shaft, dark bars edged in hot neon laser lines receding to a vanishing point. Real-time, seamless 16s loop.",
  "CREDIT": "glslop agent (Claude)",
  "CATEGORIES": [
    "fractal",
    "raymarch",
    "neon",
    "kifs",
    "tunnel"
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
      "NAME": "glow",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.3,
      "MAX": 3.0
    },
    {
      "NAME": "hue",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "sway",
      "TYPE": "float",
      "DEFAULT": 0.5,
      "MIN": 0.0,
      "MAX": 1.5
    },
    {
      "NAME": "zoom",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.5,
      "MAX": 2.0
    },
    {
      "NAME": "iterCount",
      "TYPE": "long",
      "DEFAULT": 4,
      "MIN": 2,
      "MAX": 8,
      "VALUES": [
        2,
        3,
        4,
        5,
        6,
        7,
        8
      ],
      "LABELS": [
        "2",
        "3",
        "4",
        "5",
        "6",
        "7",
        "8"
      ]
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/5ovhttcz


#define PI  3.14159265359
#define TAU 6.28318530718

const float PERIOD = 16.0;
const int   MAXI   = 8;
const int   STEPS  = 120;

mat2 rot(float a){ float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }

vec3 pal(float t){
  return 0.5 + 0.5 * cos(TAU * (t + vec3(0.55, 0.40, 0.18)));
}

// Menger KIFS tiled INFINITELY along z (the tunnel axis) via mod, so flying +z is
// an endless corridor. trap -> color ; lineT2 -> squared dist to a fold axis (laser).
float map(vec3 p, out float trap, out float lineT2){
  vec3 z = p;
  z.z = mod(z.z + 1.0, 2.0) - 1.0;        // tile period 2 along the shaft
  float scale = 3.0;
  trap = 1.0e9; lineT2 = 1.0e9;
  for (int i = 0; i < MAXI; i++){
    if (i >= iterCount) break;
    z = abs(z);
    if (z.x < z.y) z.xy = z.yx;
    if (z.x < z.z) z.xz = z.zx;
    if (z.y < z.z) z.yz = z.zy;
    z = z * scale - vec3(2.0);
    if (z.z < -1.0) z.z += 2.0;
    trap   = min(trap, dot(z, z));
    lineT2 = min(lineT2, min(min(dot(z.xy, z.xy), dot(z.yz, z.yz)), dot(z.xz, z.xz)));
  }
  return length(max(abs(z) - vec3(1.0), 0.0)) * pow(scale, -float(iterCount));
}

void main(){
  vec2 uv = (gl_FragCoord.xy - 0.5 * RENDERSIZE.xy) / RENDERSIZE.y;

  float ph = fract(TIME * speed / PERIOD);
  float th = TAU * ph;
  float flyZ = 2.0 * ph;                   // advance exactly one tile per loop -> seamless

  // camera in the central shaft, flying +z, with a gentle banking sway
  vec3 ro = vec3(0.18 * sway * sin(th), 0.13 * sway * sin(2.0 * th), flyZ);
  vec3 ta = ro + vec3(0.10 * sway * sin(th + 1.0), 0.08 * sway * cos(th), 1.0);
  vec3 fwd = normalize(ta - ro);
  vec3 rgt = normalize(cross(vec3(0.0, 1.0, 0.0), fwd));
  vec3 upv = cross(fwd, rgt);
  float roll = 0.25 * sway * sin(th);      // gentle barrel roll
  vec3 rd = normalize((uv.x * cos(roll) - uv.y * sin(roll)) * rgt
                    + (uv.x * sin(roll) + uv.y * cos(roll)) * upv
                    + (1.4 / zoom) * fwd);

  // ---- surface raymarch down the shaft ----
  float t = 0.02;
  float tcap = 14.0;
  float trapHit = 1.0e9, lineHit2 = 1.0e9, te = 0.0;
  bool hit = false;
  for (int i = 0; i < STEPS; i++){
    vec3 p = ro + rd * t;
    float tr, l2;
    float d = map(p, tr, l2);
    if (d < 0.0006 * t){ hit = true; trapHit = tr; lineHit2 = l2; te = t; break; }
    t += d * 0.85;
    if (t > tcap) break;
  }

  vec3 col = vec3(0.0);

  if (hit){
    vec3 p = ro + rd * te;
    vec2 k = vec2(1.0, -1.0);
    float e = 0.0014, tr, l2;
    vec3 n = normalize(
      k.xyy * map(p + k.xyy * e, tr, l2) +
      k.yyx * map(p + k.yyx * e, tr, l2) +
      k.yxy * map(p + k.yxy * e, tr, l2) +
      k.xxx * map(p + k.xxx * e, tr, l2));

    // colour cycles with depth down the tunnel -> multicolour shaft
    vec3 body = pal(hue + 0.6 * sqrt(trapHit) + 0.10 * (te + flyZ));
    vec3 lig  = normalize(vec3(0.3, 0.6, 0.74));
    float dif = max(dot(n, lig), 0.0);
    float fres = pow(1.0 - max(dot(n, -rd), 0.0), 2.0);

    float laser = 1.0 / (1.0 + 120.0 * lineHit2);
    laser = pow(laser, 1.5);
    vec3 laserCol = mix(vec3(0.85, 0.96, 1.0), body + 0.3, 0.4);

    col += body * 0.03;                    // dark bars
    col += body * dif * 0.10;
    col += body * fres * 2.4 * glow;       // neon bar edges
    col += laserCol * laser * 6.5 * glow;  // hot laser lines
    col *= exp(-0.34 * t);                 // fog -> recedes to a black vanishing point
  }

  col = col / (1.0 + col);
  col = pow(col, vec3(0.76));
  col *= 1.0 - 0.26 * dot(uv, uv);

  gl_FragColor = vec4(col, 1.0);
}
