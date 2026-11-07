/*{
  "ISFVSN": "2",
  "DESCRIPTION": "A river canyon at low sun.",
  "CREDIT": "croze",
  "CATEGORIES": [
    "generator",
    "nature",
    "raymarching"
  ],
  "INPUTS": [
    {
      "NAME": "speed",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 4.0
    },
    {
      "NAME": "sunHeight",
      "TYPE": "float",
      "DEFAULT": 0.32,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "haze",
      "TYPE": "float",
      "DEFAULT": 0.5,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "seed",
      "TYPE": "float",
      "DEFAULT": 3.0,
      "MIN": 0.0,
      "MAX": 16.0
    }
  ],
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/ottmkit8


// -------------------------------------------------------------
//  River Canyon
//
//  Built the standard way a heightfield terrain is built, and
//  nothing more: fBm for the land, one carve for the canyon,
//  a plane for the water, and a march that steps by the gap to
//  the ground. Forest is a colour, not geometry.
//
//  Public domain (CC0).
// -------------------------------------------------------------

#ifdef GL_ES
precision highp float;
#endif

#define MAXD 220.0
#define CEIL  26.0

const mat2 M2 = mat2(0.80, 0.60, -0.60, 0.80);

vec3  gSun;
vec3  gSunCol;
float gT;
vec2  gSeed;

// ------------------------------------------------------- noise
float hash12(vec2 p){
  vec3 p3 = fract(vec3(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

float noise(vec2 p){
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash12(i);
  float b = hash12(i + vec2(1.0, 0.0));
  float c = hash12(i + vec2(0.0, 1.0));
  float d = hash12(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm(vec2 p, int oct){
  float a = 0.5;
  float s = 0.0;
  for (int i = 0; i < 6; i++){
    if (i >= oct) break;
    s += a * noise(p);
    a *= 0.5;
    p = M2 * p * 2.03;
  }
  return s;
}

// Ridged fBm: the same loop, folded, for crests.
float ridged(vec2 p, int oct){
  float a = 0.5;
  float s = 0.0;
  for (int i = 0; i < 6; i++){
    if (i >= oct) break;
    float n = 1.0 - abs(2.0 * noise(p) - 1.0);
    s += a * n * n;
    a *= 0.5;
    p = M2 * p * 2.03;
  }
  return s;
}

// ----------------------------------------------------- terrain
// Where the canyon runs, and how far a point sits from it.
float canyonZ(float x){
  return 22.0 * sin(x * 0.0115)
       + 8.0 * sin(x * 0.0271 + 1.7)
       + 3.0 * sin(x * 0.0630 + 0.6);
}

float canyonSlope(float x){
  return 22.0 * 0.0115 * cos(x * 0.0115)
       + 8.0 * 0.0271 * cos(x * 0.0271 + 1.7)
       + 3.0 * 0.0630 * cos(x * 0.0630 + 0.6);
}

// oct trades detail for speed; the marcher drops it with range.
float terrain(vec2 p, int oct){
  vec2 q = p + gSeed;

  // Uplands: ridged fBm, and nothing clever.
  float h = ridged(q * 0.020, oct) * 26.0;

  // One carve. v is 0 on the canyon floor, 1 out on the tops.
  float d = abs(q.y - canyonZ(q.x));
  float w = 8.6 + 3.4 * noise(vec2(q.x * 0.013, 4.0));
  float v = smoothstep(w, w + 5.5, d);

  // The floor is not flat: a channel on the axis with banks
  // rising away from it, so the water has somewhere to sit.
  float bed = 2.1 * smoothstep(1.0, 5.2, d);
  h = mix(bed, h + 2.8, v);
  h += 0.34 * (fbm(q * 0.22, 3) - 0.47) * (1.0 - v);

  // Detail. The base fBm's finest octave is 3.6 units across,
  // which is smooth clay at walking range, so add two bands of
  // relief and let the marcher drop them with distance.
  if (oct >= 5) h += 1.95 * (fbm(q * 0.36, 4) - 0.47)
                          * (0.60 + 1.10 * v);
  if (oct >= 6){
    h += 0.62 * (fbm(q * 1.55, 3) - 0.47) * (0.60 + 1.10 * v);
    h += 0.14 * (fbm(q * 4.80, 2) - 0.47);
  }
  return h;
}

float terrain(vec2 p){
  return terrain(p, 6);
}

// ------------------------------------------------------- march
// Step by the vertical gap to the ground, capped as a fraction
// of the distance travelled. Then bisect the crossing.
float trace(vec3 ro, vec3 rd, float tmax){
  float t = 0.6;
  float dt = 0.0;
  for (int i = 0; i < 190; i++){
    vec3 p = ro + rd * t;
    if (p.y > CEIL && rd.y > 0.0) break;
    if (t > tmax) break;
    int oct = 6;
    if (t > 60.0) oct = 5;
    if (t > 130.0) oct = 4;
    float h = terrain(p.xz, oct);
    if (p.y < h){
      float t0 = t - dt;
      float t1 = t;
      for (int j = 0; j < 6; j++){
        float tm = 0.5 * (t0 + t1);
        vec3 pm = ro + rd * tm;
        if (pm.y < terrain(pm.xz, oct)) t1 = tm;
        else t0 = tm;
      }
      return t1;
    }
    dt = clamp(0.35 * (p.y - h), 0.05 + 0.010 * t, 0.045 * t);
    t += dt;
  }
  return -1.0;
}

vec3 normalAt(vec3 p, float t){
  float e = 0.002 * t + 0.02;
  vec2 ex = vec2(e, 0.0);
  vec2 ez = vec2(0.0, e);
  return normalize(vec3(
    terrain(p.xz - ex) - terrain(p.xz + ex),
    2.0 * e,
    terrain(p.xz - ez) - terrain(p.xz + ez)));
}

float shadow(vec3 ro, vec3 rd){
  float res = 1.0;
  float t = 0.8;
  for (int i = 0; i < 26; i++){
    vec3 p = ro + rd * t;
    if (p.y > CEIL || t > 90.0) break;
    float h = p.y - terrain(p.xz, 4);
    res = min(res, 8.0 * h / t);
    if (res < 0.03) break;
    t += clamp(h, 0.6, 0.8 + 0.12 * t);
  }
  return clamp(res, 0.0, 1.0);
}

// ----------------------------------------------------------sky
// Plain fBm gives round blobs. Warping its domain with more
// fBm gives the cauliflower edge that reads as cloud.
float cloudF(vec2 q){
  vec2 w = vec2(fbm(q * 0.60, 2), fbm(q * 0.60 + 5.1, 2));
  return fbm(q + (w - 0.47) * 1.9, 5);
}

vec3 skyCol(vec3 rd){
  float y = max(rd.y, 0.0);
  float mu = max(dot(rd, gSun), 0.0);

  vec3 col = mix(vec3(0.62, 0.70, 0.80),
                 vec3(0.10, 0.30, 0.62), pow(y, 0.55));
  col = mix(col, gSunCol * vec3(1.0, 0.76, 0.54),
            exp(-7.0 * y) * (0.25 + 0.45 * mu * mu));
  col += gSunCol * 0.30 * pow(mu, 6.0);

  if (rd.y > 0.010){
    // Low deck. Sampling the density once more toward the sun
    // gives the lit edge for free.
    float t1 = 60.0 / rd.y;
    vec2 q1 = (rd * t1).xz * 0.0060
            + vec2(gT * 0.006, 0.0) + gSeed * 0.2;
    float f1 = cloudF(q1);
    float l1 = f1 - cloudF(q1 + gSun.xz * 0.17);
    l1 = clamp(0.5 + 3.4 * l1, 0.0, 1.0);
    float c1 = smoothstep(0.40, 0.64, f1);
    c1 *= smoothstep(0.010, 0.10, rd.y) * exp(-t1 * 0.0016);
    vec3 cc1 = mix(vec3(0.27, 0.31, 0.42),
                   gSunCol * 1.36 + 0.16, l1 * l1);
    col = mix(col, cc1, c1 * 0.92);

    // High deck: thinner, slower, and it lights up first.
    float t2 = 165.0 / rd.y;
    vec2 q2 = (rd * t2).xz * 0.0021
            + vec2(gT * 0.0018, 0.0) + gSeed;
    float f2 = cloudF(q2 + 21.0);
    float c2 = smoothstep(0.46, 0.72, f2);
    c2 *= smoothstep(0.03, 0.24, rd.y) * exp(-t2 * 0.0008);
    col = mix(col, gSunCol * 1.18 + 0.13, c2 * 0.55);
  }

  col += gSunCol * smoothstep(0.9994, 0.9999, mu) * 14.0;
  col += gSunCol * pow(mu, 400.0) * 2.0;
  return col;
}

vec3 fogCol(vec3 rd){
  float mu = max(dot(rd, gSun), 0.0);
  return mix(vec3(0.30, 0.40, 0.55),
             gSunCol * vec3(0.95, 0.66, 0.40), pow(mu, 3.0));
}

// -------------------------------------------------------- land
vec3 shade(vec3 p, vec3 nor, vec3 rd, float t){
  float slope = nor.y;

  // One pixel, in world units. Any texture finer than a couple
  // of these is noise, so it gets faded to its own mean.
  float px = 1.4 * t / RENDERSIZE.y;
  float fine = smoothstep(0.9, 2.4, 0.30 / px);
  float med = smoothstep(0.9, 2.4, 1.20 / px);

  float m = fbm((p.xz + gSeed) * 0.09, 4);

  // Rock.
  vec3 rock = mix(vec3(0.105, 0.098, 0.090),
                  vec3(0.255, 0.232, 0.200), m);
  rock *= 0.82 + 0.38 * mix(0.47, fbm(p.xz * 0.7, 3), med);
  rock *= 0.86 + 0.30 * mix(0.47, fbm(p.xz * 2.9, 3), fine);

  // Forest, as colour: dark conifer green, clumped, only where
  // a tree could stand and only below the treeline.
  vec3 tree = mix(vec3(0.036, 0.052, 0.030),
                  vec3(0.070, 0.098, 0.050), m);
  tree *= 0.72 + 0.56 * mix(0.47, fbm(p.xz * 1.3, 3), med);
  tree *= 0.88 + 0.24 * mix(0.47, fbm(p.xz * 4.1, 2), fine);

  float veg = smoothstep(0.55, 0.78, slope);
  veg *= smoothstep(20.0, 12.0, p.y);
  veg *= smoothstep(0.34, 0.58, fbm((p.xz + gSeed) * 0.05, 3));
  vec3 alb = mix(rock, tree, veg);

  // Gravel where the river has been.
  float bank = smoothstep(2.6, 0.9, p.y);
  bank *= smoothstep(0.80, 0.94, slope);
  alb = mix(alb, vec3(0.190, 0.176, 0.156), bank * 0.75);

  // Snow on the tops, off the steepest faces.
  float snow = smoothstep(21.0, 25.0, p.y);
  snow *= smoothstep(0.68, 0.88, slope);
  alb = mix(alb, vec3(0.70, 0.73, 0.79), snow);

  // Light: sun, sky dome, and a warm bounce off the ground.
  float sh = shadow(p + nor * 0.05, gSun);
  float dif = clamp(dot(nor, gSun), 0.0, 1.0);
  float sky = clamp(0.5 + 0.5 * nor.y, 0.0, 1.0);
  float bnc = clamp(0.30 - 0.70 * nor.y, 0.0, 1.0);

  vec3 lin = gSunCol * 2.6 * dif * sh;
  lin += vec3(0.21, 0.30, 0.47) * sky * 0.62;
  lin += vec3(0.32, 0.23, 0.14) * bnc * 0.30;
  lin += vec3(0.045, 0.052, 0.065);

  return alb * lin;
}

vec3 aces(vec3 x){
  vec3 a = x * (2.51 * x + 0.03);
  vec3 b = x * (2.43 * x + 0.59) + 0.14;
  return clamp(a / b, 0.0, 1.0);
}

void main(){
  gT = TIME * speed;
  gSeed = vec2(seed * 31.7, seed * 17.3);

  float el = mix(0.06, 0.55, sunHeight);
  float az = 1.1 + 0.9 * sin(gT * 0.010);
  gSun = normalize(vec3(cos(az), el, sin(az)));
  gSunCol = mix(vec3(1.00, 0.42, 0.14), vec3(1.00, 0.94, 0.84),
                smoothstep(0.05, 0.45, gSun.y));

  // Camera: along the canyon at a height that is a plain
  // function of time. Nothing samples the ground, so there is
  // nothing for it to inherit.
  // The terrain reads its noise at p + gSeed, so the camera
  // has to be placed in that same frame or it ends up out on
  // the tops instead of in the canyon.
  float ax = 30.0 + gT * 1.7;
  float qx = ax + gSeed.x;
  float cz = canyonZ(qx) - gSeed.y;
  float hgt = 3.9 + 0.8 * sin(gT * 0.019);
  vec3 ro = vec3(ax, hgt, cz);

  // Aim at the canyon axis some way ahead. The river then runs
  // out of the centre of frame no matter how the walls bend.
  float la = 30.0 + 8.0 * sin(gT * 0.011);
  float qa = qx + la;
  float ty = 0.9 + 0.9 * sin(gT * 0.027);
  vec3 ta = vec3(ax + la, ty, canyonZ(qa) - gSeed.y);

  vec3 cw = normalize(ta - ro);
  vec3 cu = normalize(cross(cw, vec3(0.0, 1.0, 0.0)));
  vec3 cv = cross(cu, cw);

  vec2 sp = isf_FragNormCoord * 2.0 - 1.0;
  sp.x *= RENDERSIZE.x / RENDERSIZE.y;
  vec3 rd = normalize(sp.x * cu + sp.y * cv + 1.5 * cw);

  vec3 col;
  float t = trace(ro, rd, MAXD);

  // Water: one plane. Whatever the terrain does below it is
  // simply not seen.
  float wy = 0.85;
  float tw = -1.0;
  if (rd.y < -0.0005) tw = (wy - ro.y) / rd.y;

  if (tw > 0.0 && (t < 0.0 || tw < t)){
    vec3 p = ro + rd * tw;
    float dmp = 1.0 / (1.0 + tw * tw * 0.02);

    // Ripples travel downstream, at three scales.
    float f = gT * 2.2;
    float a1 = noise(vec2(p.x * 9.0 - f, p.z * 9.0));
    float a2 = noise(vec2(p.x * 19.0 - f * 1.7, p.z * 17.0));
    float a3 = noise(vec2(p.x * 37.0 - f * 2.6, p.z * 33.0));
    float b1 = noise(vec2(p.z * 9.0 + f * 0.4, p.x * 9.0));
    float b2 = noise(vec2(p.z * 19.0 - f * 0.9, p.x * 17.0));
    float nx = (a1 - 0.5) * 0.34 + (a2 - 0.5) * 0.19
             + (a3 - 0.5) * 0.09;
    float nz = (b1 - 0.5) * 0.28 + (b2 - 0.5) * 0.15;
    vec3 wn = normalize(vec3(nx * dmp, 1.0, nz * dmp));

    // How deep the bed lies under the plane.
    float dep = clamp(wy - terrain(p.xz, 5), 0.0, 2.0);

    vec3 rr = normalize(reflect(rd, wn));
    rr.y = max(rr.y, 0.015);
    rr = normalize(rr);

    // Reflect the canyon, not just the sky.
    vec3 refl = skyCol(rr);
    float tr = trace(p + vec3(0.0, 0.03, 0.0), rr, 70.0);
    if (tr > 0.0){
      vec3 rp = p + rr * tr;
      vec3 rc = shade(rp, normalAt(rp, tr), rr, tr);
      refl = mix(rc, fogCol(rr), 1.0 - exp(-tr * 0.011));
    }

    float fr = 1.0 - clamp(dot(-rd, wn), 0.0, 1.0);
    fr = 0.03 + 0.97 * pow(fr, 5.0);

    // Shallows show their bed; the channel goes green-black.
    vec3 body = mix(vec3(0.062, 0.098, 0.090),
                    vec3(0.011, 0.030, 0.036),
                    smoothstep(0.06, 1.10, dep));
    col = mix(body, refl, fr);

    float g = clamp(dot(rr, gSun), 0.0, 1.0);
    col += gSunCol * pow(g, 220.0) * 3.2 * dmp;

    // White water where it runs thin over the bed.
    float foam = smoothstep(0.30, 0.02, dep);
    foam *= smoothstep(0.30, 0.62, fbm(vec2(p.x * 5.0 - f * 0.6,
                                            p.z * 5.0), 3));
    col = mix(col, vec3(0.60, 0.65, 0.68), foam * 0.60 * dmp);
    t = tw;
  } else if (t > 0.0){
    vec3 p = ro + rd * t;
    col = shade(p, normalAt(p, t), rd, t);
  } else {
    col = skyCol(rd);
    t = -1.0;
  }

  if (t > 0.0){
    float d = 0.0011 + haze * 0.0034;
    float fa = 1.0 - exp(-t * d);
    col = mix(col, fogCol(rd), fa);
  }

  col = aces(col * 1.12);
  col = pow(col, vec3(0.4545));
  vec2 v = isf_FragNormCoord - 0.5;
  col *= 1.0 - 0.32 * dot(v, v);
  col += (hash12(isf_FragNormCoord * RENDERSIZE
                 + fract(TIME)) - 0.5) / 240.0;

  gl_FragColor = vec4(col, 1.0);
}