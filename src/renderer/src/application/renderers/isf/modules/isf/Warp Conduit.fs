/*{
  "DESCRIPTION": "Warp Conduit — a raymarched flight down an endless twisting tube. The tunnel axis snakes through space and the faceted wall is carved with rings and flutes that sweep past for parallax; a glowing filament runs down the throat. Pure SDF, GLSL ES 1.00.",
  "CREDIT": "glslop loop iter3",
  "CATEGORIES": [
    "raymarch",
    "3d",
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
      "NAME": "twist",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "glow",
      "TYPE": "color",
      "DEFAULT": [
        0.3,
        0.7,
        1.0,
        1.0
      ]
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/chwj7cww


// ---- helpers (ES 1.00 safe: no round/tanh/dFdx, constant loop bounds) ----

mat2 rot(float a){
  float c = cos(a), s = sin(a);
  return mat2(c, -s, s, c);
}

// the wandering center of the tunnel at depth z (gives parallax + a snaking path)
vec2 tunnelCenter(float z){
  return vec2(sin(z * 0.30) * 0.55, cos(z * 0.23) * 0.55);
}

// Distance field of the conduit. The camera flies down the INSIDE of a hollow tube
// whose axis snakes through space; the wall is at radius `tube` from that axis, so
// (tube - r) is the (signed) distance to the wall from inside. Rings + flutes carve
// relief into the wall. No geometry sits on the axis, so the camera never self-hits.
float mapScene(vec3 p){
  vec2 ctr = tunnelCenter(p.z);
  vec2 pc = p.xy - ctr;

  // corkscrew twist increases with depth so the ribs spiral as you fly
  pc = rot(p.z * 0.25 * twist) * pc;

  float r = length(pc);
  float ang = atan(pc.y, pc.x);

  float tube  = 1.35 + 0.12 * sin(p.z * 0.5);        // breathing radius
  float rings = 0.12 * sin(p.z * 3.0);               // grooves along the run
  float flutes = 0.06 * sin(ang * 8.0 + p.z * 1.5);  // angular ridges -> facets

  // inside the tube, distance to the wall:
  return (tube + rings + flutes) - r;
}

void main(){
  vec2 uv = (gl_FragCoord.xy - 0.5 * RENDERSIZE.xy) / RENDERSIZE.y;

  float t = TIME * speed;

  // camera rides the snaking tunnel axis
  float z = t * 2.4;
  vec3 ro = vec3(tunnelCenter(z), z);

  // forward = tangent of the axis path (so we lean into the bends)
  float dz = 0.35;
  vec3 axisAhead = vec3(tunnelCenter(z + dz), z + dz);
  vec3 fwd = normalize(axisAhead - ro);
  vec3 rgt = normalize(cross(vec3(0.0, 1.0, 0.0), fwd));
  vec3 upv = cross(fwd, rgt);

  // gentle camera roll for life
  vec2 ruv = rot(sin(t * 0.3) * 0.25) * uv;
  vec3 rd = normalize(fwd * 1.5 + rgt * ruv.x + upv * ruv.y);

  // ---- raymarch (constant loop bound) ----
  float dist = 0.0;
  float glowAcc = 0.0;     // volumetric haze accumulated along the ray
  float coreAcc = 0.0;     // glow from the bright filament on the axis
  float hit = -1.0;
  vec3 pos = ro;
  for (int i = 0; i < 96; i++){
    pos = ro + rd * dist;
    float d = mapScene(pos);

    // bright filament running down the tunnel axis (analytic glow, not geometry):
    // distance from this sample to the local axis -> inverse-square-ish falloff.
    vec2 toAxis = pos.xy - tunnelCenter(pos.z);
    float ra = length(toAxis);
    coreAcc += (0.012 / (0.10 + ra * ra)) * 0.10;

    // soft volumetric haze hugging the wall surface
    glowAcc += exp(-9.0 * abs(d)) * 0.025;

    if (d < 0.001){ hit = dist; break; }
    dist += max(d, 0.02) * 0.6;   // conservative step; map is not a strict SDF
    if (dist > 55.0) break;
  }

  vec3 glowCol = glow.rgb;
  vec3 col = vec3(0.0);

  if (hit > 0.0){
    // surface normal via central differences (no dFdx)
    vec2 e = vec2(0.0015, 0.0);
    vec3 n = normalize(vec3(
      mapScene(pos + e.xyy) - mapScene(pos - e.xyy),
      mapScene(pos + e.yxy) - mapScene(pos - e.yxy),
      mapScene(pos + e.yyx) - mapScene(pos - e.yyx)
    ));

    vec2 toAxis = pos.xy - tunnelCenter(pos.z);
    float ang = atan(toAxis.y, toAxis.x);

    // key light grazes the wall from down the tunnel: near walls fall into shadow,
    // the ring crests ahead catch the light -> high-contrast depth, not a flat wash.
    vec3 keyDir = normalize(fwd + vec3(-toAxis * 0.2, 0.0));
    float diff = clamp(dot(n, keyDir), 0.0, 1.0);
    diff = pow(diff, 1.6);                       // tighten the lit band

    // fresnel rim picks out the facet edges
    float fres = pow(1.0 - clamp(dot(n, -rd), 0.0, 1.0), 3.0);

    // emissive bands sweeping along the wall -> reads as forward motion
    float band = 0.5 + 0.5 * sin(pos.z * 5.0 - t * 7.0 + ang * 2.0);
    band = band * band;                          // sharpen the stripes

    // saturated palette: deep teal in shadow, electric cyan-white on the crests
    vec3 deep = glowCol * vec3(0.10, 0.14, 0.20);
    vec3 lit  = mix(glowCol, vec3(0.85, 0.97, 1.0), 0.45);
    col  = mix(deep, lit, diff);
    col += glowCol * fres * 1.1;                 // rim light on the facets
    col += mix(glowCol, vec3(1.0), 0.4) * band * 0.35;

    // atmospheric depth fade -> throat recedes into darkness
    float fog = exp(-hit * 0.11);
    col = mix(vec3(0.004, 0.01, 0.02), col, fog);
  } else {
    // looking straight down the open throat -> near-black for depth
    col = vec3(0.004, 0.01, 0.02);
  }

  // composite the additive glows (core filament + thin haze)
  col += glowCol * min(coreAcc, 1.0) * 0.55;
  col += glowCol * glowAcc * 0.55;

  // vignette to push depth toward the center
  float vig = 1.0 - 0.32 * dot(uv, uv);
  col *= vig;

  // filmic-ish tonemap + gamma (no fancy builtins)
  col = col / (1.0 + col);
  col = pow(col, vec3(0.4545));

  gl_FragColor = vec4(col, 1.0);
}
