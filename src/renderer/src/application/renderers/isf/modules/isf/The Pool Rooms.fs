/*{
  "DESCRIPTION": "The Poolrooms. An endless tiled natatorium in shallow water, generated rather than modelled: no level file and no mesh, every partition and every arched opening decided by hashing an integer cell index, so the building is unbounded and identical for everyone who runs it. The plan is the backrooms trick - a 2D box SDF over the 3x3 cells around the sample, extruded floor to ceiling, so a 128-step march through an infinite building costs about what a single sphere costs. The openings are what change. A doorway punched with a box is a doorway; an arch has to be a real cross-section, so the opening is built in the plane of the wall as the union of a jamb box and a disc at the springing, then extruded THROUGH the wall - a proper distance field with a bounded gradient, where a semicircular profile written as a height-dependent width would have infinite slope at the crown and tear the march apart there. The water is the subject and it is deliberately NOT marched. Seascape marches its surface because its waves are 0.6 m tall; a pool ripple is 8 mm, which is under a pixel at any distance past arm's length, so the displacement is invisible and only the NORMAL matters: the surface is an analytic plane hit plus one Newton step against the true height, and everything anyone actually sees is in the gradient. The ripple field is four rotated wave trains whose phase is warped by value noise, each moving at a speed proportional to one over the square root of its wavenumber - gravity-wave dispersion - so the long trains outrun the short ones and the field never reads as a single scrolling texture, which is the giveaway in most water. A wake radiates from the camera and decays, so wading disturbs the surface it is wading through. At the far end the ripple cannot be resolved, and it must not simply be faded to a mirror: an unresolved ripple is not a flat surface, it is a rough one, so as the pixel footprint outgrows the wavelength the geometric normal relaxes toward vertical and the specular lobe widens by exactly as much - the shimmer becomes gloss instead of aliasing into sparkle. The surface is then split by Fresnel between a reflection march and a refraction march. The refracted ray carries Beer-Lambert absorption on the way down and picks up the in-scattered pool colour on the way back, which is the whole reason shallow water over white tile is turquoise and deep water is not. Caustics are not a texture: they are the focusing term of the refracted map, computed as one over one plus depth times the Laplacian of the ripple field, so they spread with depth, tighten under a crest, and move with the water rather than under it - and the same field, attenuated by height above the waterline, is what puts the dancing light on the walls and the ceiling, which is the single image everyone has of this place. Every surface is glazed tile, filtered the way the backrooms filters its wallpaper: each point is told how many metres of itself one pixel covers - the pixel's world size at the hit distance divided by the grazing cosine - and the grout is box-filtered against that footprint, decaying to the grid's duty cycle rather than to nothing, because a periodic grid seen edge-on averages to grey and not to white. A course of accent tile runs at the waterline, there is a wet band above it that tracks the live ripple height rather than a fixed level, and the ledges that break the surface are defined relative to the water rather than to the floor, because their entire job is to be half in it. Light is 4 ft square coffers on a lattice snapped to a whole number of ceiling tiles, gathered nine at a time as RECTANGLES with one shadow ray fired along their weighted mean, and a second coarse march accumulates in-scattering into the humid air, which is what puts the shafts down the far end of the hall. Thirty-seven controls: walk, lens, yaw and pitch, corridor wander and turn, sway, cell size, wall density, closed rooms, arch frequency and width, ceiling height, ledge width, water depth, ripple, flow, caustics, clarity, light spacing, working lights, brightness, dark zones, haze, fog, four palettes from chlorine white through warm natatorium, cold blue and sickly green, tile scale, accent course, grime, damp, ambient occlusion, shadow, gloss, and four camcorder controls.",
  "CREDIT": "CC0. Plan generation, footprint filtering and the light gather follow the CC0 BACKROOMS shader; the water is original, informed by but not derived from Alexander Alekseev's 'Seascape'.",
  "CATEGORIES": [
    "generator",
    "raymarch",
    "liminal",
    "poolrooms",
    "water",
    "caustics",
    "architecture",
    "procedural",
    "3d"
  ],
  "INPUTS": [
    {
      "NAME": "uSpeed",
      "TYPE": "float",
      "DEFAULT": 2.2,
      "MIN": 0.0,
      "MAX": 5.0
    },
    {
      "NAME": "uLens",
      "TYPE": "float",
      "DEFAULT": 1.5,
      "MIN": 0.7,
      "MAX": 3.0
    },
    {
      "NAME": "uYaw",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": -1.57,
      "MAX": 1.57
    },
    {
      "NAME": "uPitch",
      "TYPE": "float",
      "DEFAULT": -0.06,
      "MIN": -0.6,
      "MAX": 0.6
    },
    {
      "NAME": "uWander",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 1.6
    },
    {
      "NAME": "uTurn",
      "TYPE": "float",
      "DEFAULT": 0.45,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uBob",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.0
    },
    {
      "NAME": "uCell",
      "TYPE": "float",
      "DEFAULT": 5.5,
      "MIN": 3.5,
      "MAX": 9.0
    },
    {
      "NAME": "uDensity",
      "TYPE": "float",
      "DEFAULT": 0.62,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uRooms",
      "TYPE": "float",
      "DEFAULT": 0.25,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uArch",
      "TYPE": "float",
      "DEFAULT": 0.6,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uArchW",
      "TYPE": "float",
      "DEFAULT": 0.66,
      "MIN": 0.4,
      "MAX": 1.1
    },
    {
      "NAME": "uHeight",
      "TYPE": "float",
      "DEFAULT": 3.6,
      "MIN": 2.4,
      "MAX": 6.0
    },
    {
      "NAME": "uLedge",
      "TYPE": "float",
      "DEFAULT": 0.26,
      "MIN": 0.0,
      "MAX": 0.6
    },
    {
      "NAME": "uWater",
      "TYPE": "float",
      "DEFAULT": 0.42,
      "MIN": 0.0,
      "MAX": 1.1
    },
    {
      "NAME": "uRipple",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.5
    },
    {
      "NAME": "uFlow",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uCaustic",
      "TYPE": "float",
      "DEFAULT": 0.8,
      "MIN": 0.0,
      "MAX": 2.0
    },
    {
      "NAME": "uClarity",
      "TYPE": "float",
      "DEFAULT": 0.55,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uLightSpace",
      "TYPE": "float",
      "DEFAULT": 3.05,
      "MIN": 1.83,
      "MAX": 7.3
    },
    {
      "NAME": "uLightOn",
      "TYPE": "float",
      "DEFAULT": 0.85,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uBright",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uZones",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uHaze",
      "TYPE": "float",
      "DEFAULT": 0.7,
      "MIN": 0.0,
      "MAX": 1.5
    },
    {
      "NAME": "uFog",
      "TYPE": "float",
      "DEFAULT": 0.035,
      "MIN": 0.0,
      "MAX": 0.2
    },
    {
      "NAME": "uPalette",
      "TYPE": "float",
      "DEFAULT": 0.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uTileScale",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.5,
      "MAX": 2.5
    },
    {
      "NAME": "uAccent",
      "TYPE": "float",
      "DEFAULT": 0.7,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGrime",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uDamp",
      "TYPE": "float",
      "DEFAULT": 0.4,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uAO",
      "TYPE": "float",
      "DEFAULT": 0.75,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uShadow",
      "TYPE": "float",
      "DEFAULT": 0.8,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGloss",
      "TYPE": "float",
      "DEFAULT": 0.8,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uVHS",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uScan",
      "TYPE": "float",
      "DEFAULT": 0.25,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGrain",
      "TYPE": "float",
      "DEFAULT": 0.2,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uVignette",
      "TYPE": "float",
      "DEFAULT": 0.45,
      "MIN": 0.0,
      "MAX": 1.0
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/yt9hscmb


// THE POOLROOMS - ISF / GLSL ES 1.00 (gl_FragColor, TIME, RENDERSIZE).
//
// An infinite tiled natatorium in ankle-to-knee-deep water. The plan and the
// filtering follow the BACKROOMS shader; the openings are true arches and the
// water is an analytic surface whose entire appearance lives in its normal.

#define PI  3.14159265359
#define FAR 46.0

// A 2 ft ceiling tile: the module everything on the ceiling is measured in.
#define CTILE 0.6096
// glazed wall tile (6 in) and pool-floor tile (9 in), before uTileScale
#define WTILE 0.1524
#define FTILE 0.2286

// The light lattice, snapped to a WHOLE NUMBER of ceiling tiles so a 2x2-tile
// coffer sits exactly in the grid instead of straddling a T-bar. The coffer is
// an EVEN number of tiles across, so its centre has to land on a tile
// BOUNDARY - which happens for free when the lattice is an even number of
// tiles and needs a half-tile nudge when it is odd.
float gLG;
vec2  gLOF;
#define PANEL_C(lid) ((lid + 0.5)*gLG + gLOF)
#define PANEL_E      (vec2(CTILE, CTILE))

float gWL;      // still-water level
float gWT;      // water clock
vec2  gCam;     // camera plan position, for the wake
float gPX;      // pixel solid angle scale, for the footprint

// palette, filled once in main
vec3 gTile, gGrout, gAcc, gPlas, gLite, gShal, gDeep;

// ---------------------------------------------------------------
// hashes and value noise (Dave Hoskins' constants)
// ---------------------------------------------------------------
float hash11(float p){ p = fract(p*0.1031); p *= p + 33.33; p *= p + p; return fract(p); }

float hash12(vec2 p){
    vec3 q = fract(vec3(p.xyx)*0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y)*q.z);
}

vec4 hash42(vec2 p){
    vec4 q = fract(vec4(p.xyxy)*vec4(0.1031, 0.1030, 0.0973, 0.1099));
    q += dot(q, q.wzxy + 33.33);
    return fract((q.xxyz + q.yzzw)*q.zywx);
}

float vnoise(vec2 p){
    vec2 i = floor(p), f = fract(p);
    f = f*f*(3.0 - 2.0*f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), f.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), f.x), f.y);
}

// Every octave rotated as well as scaled: value noise has axis-aligned
// iso-contours, and four unrotated octaves stack their cell edges into a
// visible quilt on any large flat surface.
float fbm(vec2 p){
    mat2 R = mat2(0.8040, 0.5946, -0.5946, 0.8040);
    float s = 0.0, a = 0.5;
    for (int i = 0; i < 4; i++){
        s += a*vnoise(p);
        p = R*p*2.07 + 19.3;
        a *= 0.5;
    }
    return s;
}

// ---------------------------------------------------------------
// primitives and the footprint filter
// ---------------------------------------------------------------
float sdBox2(vec2 p, vec2 b){ vec2 q = abs(p) - b; return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0); }

// Box-filter coverage of the band |x| < r by a pixel w wide. This is the part
// a fixed smoothstep cannot do: a grout line is 3 mm across, so past a couple
// of metres - and immediately on any wall seen obliquely - it is thinner than
// a pixel, and a hard-edged test keeps drawing it at full strength wherever it
// lands, which beats against the pixel grid as moire.
float band(float x, float r, float w){
    float h = 0.5*max(w, 1e-5);
    return clamp((min(x + h, r) - max(x - h, -r))/(2.0*h), 0.0, 1.0);
}

float fill(float d, float w){ return clamp(0.5 - d/max(w, 1e-5), 0.0, 1.0); }

// How much of a texture is safe to draw. The transition has to be WIDE:
// everything keys its strength off the footprint, and since the footprint
// depends on dot(n, rd), a narrow fade paints conics about the normal - which
// is to say concentric rings across every oblique wall.
float mip(float feature, float foot){ return smoothstep(1.10, 0.10, foot/feature); }

vec3 pick4(vec3 a, vec3 b, vec3 c, vec3 d, float f){
    float i = clamp(f, 0.0, 3.0);
    float n = floor(i), k = i - n;
    vec3 lo = a;
    lo = mix(lo, b, step(0.5, n)); lo = mix(lo, c, step(1.5, n)); lo = mix(lo, d, step(2.5, n));
    vec3 hi = b;
    hi = mix(hi, c, step(0.5, n)); hi = mix(hi, d, step(1.5, n));
    return mix(lo, hi, k);
}

// ---------------------------------------------------------------
// the plan
// ---------------------------------------------------------------
// The walk and the open lane are the same curve, x = pathX(z), so the corridor
// is carved by the very function that drives the camera and can never be
// walled off.
float pathX(float z){
    return uWander*(2.6*sin(z*0.0290) + 1.35*sin(z*0.0161 + 1.7))
         + uTurn*(11.0*sin(z*0.0473 + 2.1) + 7.0*sin(z*0.0281 + 5.3)
                + 4.5*sin(z*0.0917 + 0.7));
}

float pathSlope(float z){
    return uWander*(0.0754*cos(z*0.0290) + 0.0217*cos(z*0.0161 + 1.7))
         + uTurn*(0.5203*cos(z*0.0473 + 2.1) + 0.1967*cos(z*0.0281 + 5.3)
                + 0.4127*cos(z*0.0917 + 0.7));
}

// THE ARCH, as a cross-section rather than as a width.
//
// The obvious way to arch an opening is to make its half-width a function of
// height - W*sqrt(1 - u*u) above the springing - and subtract that box. It
// looks right and it destroys the march: that profile has infinite slope at
// the crown, so the field's Lipschitz constant blows up exactly there and the
// sphere tracer steps straight through the arch head.
//
// So the section is built properly, in the plane of the wall: the union of a
// jamb box and a disc at the springing line, which is a valid SDF everywhere,
// then extruded through the wall by a max against the through-thickness. Same
// silhouette, bounded gradient, no artefact at the crown.
//
//   u = along the wall, v = through it, y = height
float archSection(float u, float v, float y, float thick){
    float W = uArchW;
    float S = 1.02 + 0.25*W;                 // springing line
    float jamb = sdBox2(vec2(u, y - 0.5*S), vec2(W, 0.5*S));
    float head = length(vec2(u, y - S)) - W;
    return max(min(jamb, head), abs(v) - thick);
}

// .x = partition footprint (2D), .y = the arched opening (3D)
vec2 maze2(vec2 q, float y){
    float C = uCell;
    vec2 base = floor(q/C);
    float dW = C, dD = C;
    for (int j = 0; j < 3; j++){
        // the lane depends only on the row, so the path is evaluated once here
        float rz = (base.y + float(j) - 0.5)*C;
        float px = pathX(rz);
        float sl = pathSlope(rz);
        // A band of fixed horizontal width is a corridor that narrows as the
        // path steepens; widening it by sqrt(1 + slope^2) keeps the lane a
        // constant width measured ACROSS itself.
        float pw = 0.80*C*sqrt(1.0 + sl*sl);
        for (int i = 0; i < 3; i++){
            vec2 id = base + vec2(float(i) - 1.0, float(j) - 1.0);
            vec2 c  = (id + 0.5)*C;
            vec4 h  = hash42(id);
            vec4 g  = hash42(id + 57.31);

            float lane = step(pw, abs(c.x - px));
            float on   = step(h.x, uDensity)*lane;

            float horiz = step(0.5, h.y);
            vec2 dirL = mix(vec2(0.0, 1.0), vec2(1.0, 0.0), horiz);
            vec2 dirT = mix(vec2(1.0, 0.0), vec2(0.0, 1.0), horiz);

            float pil = step(0.90, g.x);                       // a square column
            float len = mix((0.66 + 0.34*h.z)*0.5*C, 0.32, pil);
            float th  = mix(0.105, 0.32, pil);
            vec2  ctr = c + dirL*((h.w - 0.5)*(C - 2.0*len))
                          + dirT*((g.y - 0.5)*0.70*(C - 2.0*th));

            // Some cells are a closed chamber rather than a free-standing
            // partition: a rectangular ring, the difference of two boxes.
            // Without it the plan is only ever scattered slabs and you never
            // end up anywhere with four walls around you.
            float isRoom = step(fract(h.y*31.7), uRooms);
            float rr = 0.5*C - 0.22;
            float ring = max(sdBox2(q - c, vec2(rr)), -sdBox2(q - c, vec2(rr - 0.10)));
            float slab = sdBox2(q - ctr, dirL*len + dirT*th);

            dW = min(dW, mix(slab, ring, isRoom) + (1.0 - on)*90.0);

            // A chamber ALWAYS gets its arch, or it is a sealed box nobody can
            // ever be inside; a partition gets one with probability uArch.
            float dr = on*mix(step(g.z, uArch)*(1.0 - pil), 1.0, isRoom);
            vec2 dc = mix(ctr + dirL*((g.w - 0.5)*2.0*max(len - 1.15, 0.0)),
                          c + dirT*rr, isRoom);
            vec2 rel = q - dc;
            float u = dot(rel, dirL), v = dot(rel, dirT);
            float thick = mix(th, 0.10, isRoom) + 0.55;   // cut clean through
            dD = min(dD, archSection(u, v, y, thick) + (1.0 - dr)*90.0);

            if (dW < -0.5*C) break;
        }
        if (dW < -0.5*C) break;
    }
    return vec2(min(dW, C), min(dD, C));
}

// .x = distance, .y = material (1 pool floor, 2 ceiling, 3 wall, 4 ledge)
vec2 map(vec3 p){
    vec2 r = vec2(p.y, 1.0);                       // the pool floor
    float dC = uHeight - p.y;
    if (dC < r.x) r = vec2(dC, 2.0);

    vec2 mz = maze2(p.xz, p.y);
    float dW = max(mz.x, -mz.y);                   // wall, less the arch
    if (dW < r.x) r = vec2(dW, 3.0);

    // The ledge is defined relative to the WATER, not to the floor, because
    // its whole job is to be half in it: a step you could sit on, that gives
    // the waterline something to cut across other than a flat wall.
    if (uLedge > 0.02){
        float lh = gWL + 0.20;
        float dl = max(max(mz.x - uLedge, -mz.y), p.y - lh);
        if (dl < r.x) r = vec2(dl, 4.0);
    }
    return r;
}

vec3 calcNormal(vec3 p){
    vec2 e = vec2(1.0, -1.0)*0.0018;
    return normalize(e.xyy*map(p + e.xyy).x + e.yyx*map(p + e.yyx).x
                   + e.yxy*map(p + e.yxy).x + e.xxx*map(p + e.xxx).x);
}

float calcAO(vec3 p, vec3 n){
    float o = 0.0, s = 1.0;
    for (int i = 0; i < 5; i++){
        float h = 0.02 + 0.16*float(i);
        o += (h - map(p + n*h).x)*s;
        s *= 0.72;
    }
    return clamp(1.0 - 1.05*o, 0.0, 1.0);
}

float shadowRay(vec3 ro, vec3 rd, float maxt){
    float res = 1.0, t = 0.16, prev = 1e12;
    for (int i = 0; i < 24; i++){
        float h = map(ro + rd*t).x;
        float y = h*h/(2.0*prev);
        float d = sqrt(max(h*h - y*y, 0.0));
        res = min(res, d/max(0.0001, 7.0*(t - y)));
        prev = h;
        t += clamp(h, 0.05, 0.55);
        if (res < 0.02 || t > maxt) break;
    }
    return clamp(res, 0.0, 1.0);
}

// ---------------------------------------------------------------
// the ceiling coffers
// ---------------------------------------------------------------
// A slowly varying field that fails the lights in BLOCKS rather than one at a
// time. A flat per-fixture probability scatters the dead ones evenly and every
// distance comes out equally lit; clumping is what puts a genuinely dark
// chamber at the end of a hall. The same field dims the ambient and the fog,
// so a dark zone is dark the whole way through.
float lightZone(vec2 xz){ return smoothstep(0.20, 0.80, vnoise(xz*0.026)); }

float panelOn(vec2 lid){
    vec4 h = hash42(lid + 11.7);
    float zn = lightZone(PANEL_C(lid));
    float on = step(h.x, uLightOn*mix(1.0, 0.05 + 1.90*zn, uZones));
    float ph = h.y*97.0 + h.z*13.0;
    return on*(0.965 + 0.035*sin(TIME*47.0 + ph));      // the mains hum
}

// Nine nearest coffers gathered as RECTANGLES, plus one direction to fire a
// single shadow ray along. That direction is the WEIGHTED MEAN of the nine and
// not the strongest of them: the strongest changes as you slide along a wall,
// the loci where two tie are smooth curves, and the shadow term steps across
// them - which paints the wall in concentric arcs. A mean has no seams.
float lighting(vec3 p, vec3 n, out vec3 keyDir, out float keyDist){
    float LG = gLG;
    vec2 lb = floor(p.xz/LG);
    float sum = 0.0, accD = 0.0;
    vec3 acc = vec3(0.0);
    for (int j = 0; j < 3; j++){
        for (int i = 0; i < 3; i++){
            vec2 lid = lb + vec2(float(i) - 1.0, float(j) - 1.0);
            vec2 lc  = PANEL_C(lid);
            float on = panelOn(lid);
            vec3 pl = vec3(clamp(p.x, lc.x - PANEL_E.x, lc.x + PANEL_E.x), uHeight - 0.04,
                           clamp(p.z, lc.y - PANEL_E.y, lc.y + PANEL_E.y));
            vec3 L = pl - p;
            float dl = max(length(L), 0.08);
            L /= dl;
            // A fixture must be worth NOTHING by the time the 3x3 window drops
            // it, or the sum steps at every cell edge.
            vec2 ho = max(abs(p.xz - lc) - PANEL_E, 0.0);
            float w = on*max(dot(n, L), 0.0)*(0.30 + 0.70*max(L.y, 0.0))
                    * smoothstep(1.45*LG, 1.02*LG, length(ho))/(1.0 + 0.30*dl*dl);
            sum  += w;
            acc  += L*w;
            accD += dl*w;
        }
    }
    keyDir  = normalize(acc + n*0.001 + vec3(0.0, 0.0001, 0.0));
    keyDist = accD/max(sum, 0.0001);
    return sum;
}

// ---------------------------------------------------------------
// the water
// ---------------------------------------------------------------
// Four wave trains, rotated off each other by an angle incommensurate with a
// right angle, each with its phase warped by value noise so no train is
// straight, and each moving at a speed proportional to one over the square
// root of its wavenumber.
//
// That last part is gravity-wave DISPERSION and it is the whole reason this
// does not read as a scrolling texture: in real water the long swell outruns
// the short chop, the crossings drift, and the pattern never repeats its own
// motion. Scroll every octave at one speed and the eye finds the loop
// immediately, however many octaves there are.
float rippleHi(vec2 p, float t){
    mat2 R = mat2(0.8763, 0.4818, -0.4818, 0.8763);
    float h = 0.0, a = 1.0, f = 1.0, s = 0.0;
    for (int i = 0; i < 4; i++){
        vec2 q = p*f;
        float ph = 6.2831*vnoise(q*0.33 + float(i)*7.1 + t*0.04);
        h += a*sin(q.x + 0.55*q.y + ph + t*2.6*inversesqrt(f));
        s += a;
        p = R*p; a *= 0.52; f *= 2.15;
    }
    return h/s;
}

// two octaves, for the caustic Laplacian and for anything far enough away that
// the top two octaves are already below a pixel
float rippleLo(vec2 p, float t){
    mat2 R = mat2(0.8763, 0.4818, -0.4818, 0.8763);
    float h = 0.0, a = 1.0, f = 1.0, s = 0.0;
    for (int i = 0; i < 2; i++){
        vec2 q = p*f;
        float ph = 6.2831*vnoise(q*0.33 + float(i)*7.1 + t*0.04);
        h += a*sin(q.x + 0.55*q.y + ph + t*2.6*inversesqrt(f));
        s += a;
        p = R*p; a *= 0.52; f *= 2.15;
    }
    return h/s;
}

// A wake radiating from the camera and decaying, so that wading disturbs the
// surface it is wading through. Without it the water is a beautiful field the
// viewer is not in.
float wake(vec2 xz){
    float r = length(xz - gCam);
    return sin(r*10.5 - gWT*6.0)*exp(-r*0.70)*smoothstep(0.04, 0.85, r);
}

float waterHeight(vec2 xz){
    return gWL + uRipple*(0.0130*rippleHi(xz*3.0, gWT)
                        + 0.0075*wake(xz)*clamp(uSpeed*0.4, 0.0, 1.2));
}

// CAUSTICS ARE NOT A TEXTURE. They are the Jacobian of the refracted map: for
// a shallow height field the bundle of rays through a patch of surface is
// compressed by 1/(1 + d*div), where div is the Laplacian of the height and d
// is how far the light has travelled since. So they spread and soften with
// depth, tighten under a crest, and move WITH the water instead of under it.
//
// The sample spacing widens with depth too, which is the same statement made
// twice: deep water cannot carry the fine detail of the surface above it.
float caustic(vec2 xz, float depth){
    if (uCaustic < 0.005) return 1.0;
    float e = clamp(0.07 + 0.30*depth, 0.07, 0.55);
    float h0 = rippleLo(xz*3.0, gWT);
    float lap = rippleLo((xz + vec2(e, 0.0))*3.0, gWT) + rippleLo((xz - vec2(e, 0.0))*3.0, gWT)
              + rippleLo((xz + vec2(0.0, e))*3.0, gWT) + rippleLo((xz - vec2(0.0, e))*3.0, gWT)
              - 4.0*h0;
    float foc = -lap/(e*e);                      // positive where the surface focuses
    float c = clamp(1.0 + 0.055*uRipple*(0.30 + depth)*foc, 0.04, 3.4);
    c = c*c;                                     // filaments, not gradients
    return mix(1.0, c, clamp(uCaustic, 0.0, 2.0));
}

// ---------------------------------------------------------------
// glazed tile, box-filtered
// ---------------------------------------------------------------
// A periodic grid seen edge-on averages to its DUTY CYCLE, not to nothing.
// Letting band() run to zero on its own would bleach every distant wall to
// plain white; blending toward the average coverage as the footprint outgrows
// the tile is what makes a far wall read as tile you cannot resolve.
float grout(vec2 uv, float T, float foot, out vec2 tid, out vec2 tf){
    vec2 g = uv/T;
    tid = floor(g);
    tf  = fract(g);
    vec2 d = abs(tf - 0.5);
    float e = 0.5 - max(d.x, d.y);               // distance to the nearest joint
    float w = foot/T;
    float gr = 0.034;
    return mix(band(e, gr, w), 4.0*gr, smoothstep(0.06, 0.40, w));
}

// ---------------------------------------------------------------
// surface shading. qual 1 = full (AO, shadow, fine detail), 0 = the cheap
// version used inside the reflection and refraction marches.
// ---------------------------------------------------------------
vec3 shade(vec3 p, vec3 rd, vec3 n, float mat, float foot, float qual, out vec3 emis){
    emis = vec3(0.0);
    float det  = mip(0.008, foot);
    float detT = mip(0.030, foot);
    float detM = mip(0.45,  foot);
    float wh   = waterHeight(p.xz);
    float above = p.y - wh;                       // signed height over the water
    float sub  = smoothstep(0.012, -0.012, above);

    vec3 alb = gTile;
    float gloss = 0.0;
    vec2 tid, tf;

    if (mat < 1.5){
        // ---- the pool floor ----
        float T = FTILE*uTileScale;
        float gm = grout(p.xz, T, foot, tid, tf);
        vec4 th = hash42(tid + 4.3);
        alb = gTile*(0.90 + 0.14*th.x)*(1.0 + 0.10*(vnoise(p.xz*40.0) - 0.5)*det);
        alb = mix(alb, gGrout*(0.85 + 0.3*th.y), gm);
        // silt collecting in the joints and in the corners of the plan
        float silt = fbm(p.xz*0.55 + 9.0);
        alb *= mix(1.0, 0.72 + 0.36*silt, uGrime*detM);
        alb = mix(alb, alb*vec3(0.72, 0.78, 0.70), gm*uGrime*0.55*detT);
        gloss = 0.35;
    } else if (mat < 2.5){
        // ---- ceiling: a coffer, or the tile grid it sits in ----
        vec2 lid = floor(p.xz/gLG);
        vec2 dp  = abs(p.xz - PANEL_C(lid)) - PANEL_E;
        if (max(dp.x, dp.y) < 0.0){
            float on = panelOn(lid);
            vec2 pg = fract((p.xz - PANEL_C(lid))/(CTILE*0.25));
            float pr = smoothstep(0.0, 0.16, min(min(pg.x, 1.0 - pg.x), min(pg.y, 1.0 - pg.y)));
            pr = mix(1.0, 0.74 + 0.38*pr, det);
            emis = gLite*(1.5 + 6.5*uBright)*on*pr;
            alb  = gLite*0.55;
        } else {
            float gm = grout(p.xz, CTILE, foot, tid, tf);
            vec4 th = hash42(tid + 3.7);
            alb = gPlas*(0.88 + 0.18*th.x)*(1.0 + 0.16*(vnoise(p.xz*90.0) - 0.5)*det);
            alb = mix(alb, gPlas*0.62, gm*0.8);
            // damp blooming through from whatever is above, which there is not
            float ring = smoothstep(0.54, 0.82, fbm(p.xz*1.3 + th.yz*11.0))*uDamp*step(th.w, 0.5)*detM;
            alb = mix(alb, vec3(0.32, 0.30, 0.22), ring*0.65);
            alb *= 1.0 - 0.50*smoothstep(-0.05, 0.0, max(dp.x, dp.y));
        }
    } else {
        // ---- wall or ledge: glazed tile ----
        float T = WTILE*uTileScale;
        vec2 uv;
        float top = step(0.55, abs(n.y));
        if (top > 0.5)            uv = p.xz;                      // ledge top
        else if (abs(n.x) > 0.5)  uv = vec2(p.z, p.y);
        else                      uv = vec2(p.x, p.y);
        if (mat > 3.5 && top > 0.5) T = FTILE*uTileScale;         // bigger tile on the deck

        float gm = grout(uv, T, foot, tid, tf);
        vec4 th = hash42(tid + 17.1);
        alb = gTile*(0.92 + 0.13*th.x);
        // the tooth of the glaze: too fine to resolve past a couple of metres,
        // but it is what stops a near wall reading as flat paint
        alb *= 1.0 + 0.13*(vnoise(uv*220.0) - 0.5)*mip(0.010, foot)
                   + 0.07*(vnoise(uv*55.0)  - 0.5)*mip(0.050, foot);
        // the odd tile crazed or gone
        alb *= mix(1.0, 0.55, step(0.988, th.z)*detT);

        // A course of accent tile at the waterline, which every pool built
        // before 1995 has. Two rows, filtered like everything else, and keyed
        // to the STILL level rather than the live one - the tiler did not know
        // about the ripple.
        if (top < 0.5 && uAccent > 0.01){
            float ah = gWL + 0.14;
            float cov = band(uv.y - ah, 0.16*T/WTILE*0.95, foot);
            alb = mix(alb, gAcc*(0.85 + 0.3*th.y), cov*uAccent);
        }
        alb = mix(alb, gGrout*(0.88 + 0.24*th.y), gm);

        // grime rising out of the water, and damp coming down the wall
        float gr = fbm(uv*1.25 + 7.0);
        if (top < 0.5){
            float tide = smoothstep(0.45, 0.0, abs(above - 0.05));
            alb *= mix(1.0, 0.66 + 0.42*gr, (smoothstep(1.1, 0.0, above)*0.5 + tide*0.6)*uGrime*detM);
            alb = mix(alb, alb*vec3(0.62, 0.66, 0.60), smoothstep(0.52, 1.02, gr)*uDamp*0.6*detM);
        }
        gloss = 0.55;
    }

    // Everything the water touches is WET, and wet is not a colour change, it
    // is a darkening plus a gloss. The band tracks the live ripple height, so
    // the wet line breathes with the surface instead of ruling a straight line
    // across the tile.
    float wet = sub + (1.0 - sub)*exp(-max(above, 0.0)*11.0);
    alb *= 1.0 - 0.28*wet;
    gloss = mix(gloss, 0.95, wet);

    // ---- light ----
    float ao = 1.0;
    if (qual > 0.5) ao = mix(1.0, calcAO(p, n), uAO);
    vec3 kd; float kdist;
    float lit = lighting(p, n, kd, kdist);
    float sh = 1.0;
    if (qual > 0.5 && uShadow > 0.01) sh = mix(1.0, shadowRay(p + n*0.06, kd, kdist - 0.20), uShadow);

    // Caustics, from the same field, on both sides of the surface.
    //
    // Underwater it is the light arriving through the ripple; ABOVE water it
    // is the same pattern bounced back up off it, attenuated by height, and it
    // is the single image everyone has of this place. Both come from one
    // function, because they are one phenomenon.
    float depth = max(wh - p.y, 0.0);
    float cw = 1.0;
    if (sub > 0.5){
        cw = caustic(p.xz, depth)*(0.55 + 0.45*max(n.y, 0.0));
    } else {
        float h = max(above, 0.0);
        float reach = exp(-h*0.55)*(0.30 + 0.70*(1.0 - max(n.y, 0.0)));
        cw = mix(1.0, caustic(p.xz, 0.35 + h*0.5), reach*0.85);
    }

    vec3 diff = gLite*lit*(0.52 + 0.48*sh)*(2.4 + 3.8*uBright)*cw;

    // no GI, but a white-tiled room is almost entirely bounce: a hemispheric
    // fill, plus an explicit term for light coming back off the water, which
    // is what puts any brightness on the ceiling at all
    float dn = 0.5 - 0.5*n.y;
    float zdim = mix(1.0, 0.25 + 1.05*lightZone(p.xz), uZones);
    vec3 amb = ao*zdim*(mix(gTile, gLite, 0.5)*(0.085 + 0.10*uBright)
                      + gShal*gLite*(0.22 + 0.30*uBright)*dn*mix(1.0, cw, 0.5));

    vec3 col = alb*(diff*ao*0.85 + amb);

    // and the glaze itself: a real highlight off the key direction, which is
    // most of what says "tile" rather than "plaster"
    if (uGloss > 0.01 && lit > 0.001){
        vec3 hv = normalize(kd - rd);
        float sp = pow(max(dot(n, hv), 0.0), mix(28.0, 150.0, gloss));
        col += gLite*sp*lit*sh*gloss*uGloss*(0.35 + 0.5*uBright);
    }
    return col;
}

// ---------------------------------------------------------------
void main(){
    vec2 fragCoord = gl_FragCoord.xy;
    vec2 R = RENDERSIZE.xy;
    float T = TIME;
    float t = T*uSpeed;

    gTile  = pick4(vec3(0.90, 0.93, 0.91), vec3(0.91, 0.87, 0.76), vec3(0.70, 0.77, 0.86), vec3(0.85, 0.90, 0.74), uPalette);
    gGrout = pick4(vec3(0.62, 0.66, 0.64), vec3(0.61, 0.56, 0.47), vec3(0.44, 0.50, 0.59), vec3(0.56, 0.61, 0.48), uPalette);
    gAcc   = pick4(vec3(0.09, 0.34, 0.42), vec3(0.16, 0.30, 0.29), vec3(0.05, 0.15, 0.31), vec3(0.16, 0.35, 0.20), uPalette);
    gPlas  = pick4(vec3(0.86, 0.90, 0.90), vec3(0.88, 0.84, 0.74), vec3(0.66, 0.73, 0.83), vec3(0.82, 0.88, 0.73), uPalette);
    gLite  = pick4(vec3(0.93, 0.98, 1.00), vec3(1.00, 0.95, 0.80), vec3(0.72, 0.85, 1.00), vec3(0.88, 1.00, 0.83), uPalette);
    gShal  = pick4(vec3(0.44, 0.86, 0.86), vec3(0.42, 0.76, 0.66), vec3(0.24, 0.54, 0.80), vec3(0.46, 0.84, 0.60), uPalette);
    gDeep  = pick4(vec3(0.02, 0.26, 0.34), vec3(0.05, 0.24, 0.24), vec3(0.02, 0.10, 0.26), vec3(0.05, 0.26, 0.18), uPalette);

    float ltn = max(3.0, floor(uLightSpace/CTILE + 0.5));
    gLG  = CTILE*ltn;
    // the coffer is two tiles across, so its centre must land on a tile
    // BOUNDARY: free when the lattice is even, a half-tile nudge when it is odd
    gLOF = vec2(0.5*CTILE*mod(ltn, 2.0));
    gWL  = uWater;
    gWT  = T*uFlow;

    // ---- the wade ----
    float z = t*1.05;
    vec3 ro = vec3(pathX(z) + uBob*0.026*sin(t*2.4), 1.50 + uBob*0.034*sin(t*4.8), z);
    gCam = ro.xz;

    float dz = 0.35;
    vec3 fw = normalize(vec3(pathX(z + dz) - pathX(z - dz), 0.0, 2.0*dz));
    float yaw = uYaw + uBob*0.05*sin(t*0.79 + 1.0);
    float cy = cos(yaw), sy = sin(yaw);
    fw = normalize(vec3(fw.x*cy + fw.z*sy, 0.0, fw.z*cy - fw.x*sy));
    vec3 rt = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(rt, fw);
    fw = normalize(fw + up*(uPitch + uBob*0.020*sin(t*1.6)));
    rt = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    up = cross(rt, fw);
    float roll = uBob*0.013*sin(t*2.0 + 0.5);
    vec3 rr2 = rt*cos(roll) + up*sin(roll);
    up = up*cos(roll) - rt*sin(roll);
    rt = rr2;

    // ---- the lens, bent before anything is traced ----
    vec2 uv = (fragCoord - 0.5*R)/R.y;
    float vhs = uVHS;
    float ln = hash11(floor(fragCoord.y) + floor(T*24.0)*17.0);
    uv.x += vhs*(0.0022*(ln - 0.5) + 0.0026*sin(T*1.7 + fragCoord.y*0.031));
    uv *= 1.0 + vhs*0.040*dot(uv, uv);
    vec3 rd = normalize(uv.x*rt + uv.y*up + uLens*fw);
    gPX = 1.0/(uLens*R.y);

    // ---- where the water is, BEFORE the march ----
    //
    // Solved analytically. The surface is never marched and never appears in
    // map(): its displacement is 13 mm, which is under a pixel at any distance
    // past arm's length, so a marcher would spend a hundred steps resolving
    // something nobody can see. One plane hit plus one Newton step against the
    // real height puts the intersection where it belongs, and everything the
    // viewer actually reacts to is in the NORMAL.
    float tW = 1e9;
    if (rd.y < -0.004 && ro.y > gWL){
        float tp = (gWL - ro.y)/rd.y;
        if (tp > 0.0 && tp < FAR){
            vec3 pw = ro + rd*tp;
            // a crest is reached sooner than the still plane, a trough later;
            // clamped because at grazing incidence the correction diverges
            tp += clamp((waterHeight(pw.xz) - pw.y)/rd.y, -0.45, 0.45);
            tW = max(tp, 0.0);
        }
    }
    float tMax = min(FAR, tW);

    // ---- march the room, but only as far as the water ----
    float tt = 0.0, tHit = tMax, mat = 0.0;
    for (int i = 0; i < 144; i++){
        vec2 m = map(ro + rd*tt);
        // The hit test opens up with distance or a grazing ray down a 40 m
        // hall runs out of steps - but not too far: stopping short by e along
        // the ray is a slide of e/cos ALONG a grazing surface, which bands
        // every lighting term computed from the hit point. Hence a tight-ish
        // epsilon and the Newton step below rather than one loose epsilon
        // doing both jobs.
        if (m.x < 0.0024*tt + 0.0009){ tHit = tt; mat = m.y; break; }
        tt += m.x*0.88;
        if (tt > tMax) break;
    }

    float zf = lightZone((ro + rd*18.0).xz);
    vec3 fogCol = mix(gTile, gLite, 0.55)*(0.055 + 0.115*uBright)
                * mix(1.0, 0.10 + 1.50*zf, uZones);
    vec3 col = fogCol;
    float tShow = FAR;

    if (mat > 0.5){
        vec3 p = ro + rd*tHit;
        vec3 n = calcNormal(p);
        // One Newton step onto the true intersection: moving dt along the ray
        // changes the signed distance by dt*dot(n, rd), so the residual maps
        // straight onto a correction in t. Exact for a plane, one extra map(),
        // and it is what removes the contour banding on oblique walls.
        float ndv0 = min(dot(n, rd), -0.06);
        tHit += clamp(-map(p).x/ndv0, 0.0, 0.75);
        p = ro + rd*tHit;
        n = calcNormal(p);
        float ndv = dot(n, rd);
        // The grazing term is sqrt(c*c + k) and not max(|c|, k): a hard clamp
        // has an EDGE where it engages, and on a flat wall that edge is a cone
        // about the normal - a ring painted across the surface.
        float foot = tHit*gPX/sqrt(ndv*ndv + 0.004);
        vec3 em;
        col = shade(p, rd, n, mat, foot, 1.0, em) + em;
        tShow = tHit;
    } else if (tW < FAR){
        // ---------------- the surface ----------------
        vec3 pw = ro + rd*tW;
        float foot = tW*gPX/max(-rd.y, 0.02);

        // An unresolved ripple is not a flat surface, it is a ROUGH one.
        //
        // Fading the normal to vertical with distance is right - it is the
        // only way to stop the far water aliasing into crawling sparkle - but
        // done alone it turns the far end of the hall into a mirror, which is
        // worse than the aliasing. So the same term that flattens the geometry
        // widens the specular lobe by the same amount: the shimmer stops being
        // shape and becomes gloss, which is exactly what it is.
        float res = mip(0.28, foot);
        float e = max(0.012, 0.55*foot);
        float h  = waterHeight(pw.xz);
        float hx = waterHeight(pw.xz + vec2(e, 0.0));
        float hz = waterHeight(pw.xz + vec2(0.0, e));
        vec3 n = normalize(vec3((h - hx)*res, e, (h - hz)*res));

        float ndv = max(dot(-rd, n), 0.0);
        // Schlick, with the 0.02 of water and a floor under it so the surface
        // never goes perfectly non-reflective head-on
        float F = 0.02 + 0.98*pow(1.0 - ndv, 5.0);
        F = clamp(F, 0.02, 0.96);

        // ---- reflection ----
        vec3 rr = reflect(rd, n);
        vec3 refl = fogCol;
        float rt2 = 0.05, rHit = -1.0, rMat = 0.0;
        for (int i = 0; i < 56; i++){
            vec2 m = map(pw + rr*rt2);
            if (m.x < 0.004*rt2 + 0.002){ rHit = rt2; rMat = m.y; break; }
            rt2 += m.x*0.92;
            if (rt2 > 30.0) break;
        }
        if (rMat > 0.5){
            vec3 rp = pw + rr*rHit;
            vec3 rn = calcNormal(rp);
            float rfoot = (tW + rHit)*gPX/sqrt(dot(rn, rr)*dot(rn, rr) + 0.02);
            vec3 em;
            refl = shade(rp, rr, rn, rMat, rfoot, 0.0, em) + em;
            refl = mix(refl, fogCol, 1.0 - exp(-uFog*rHit));
        }

        // ---- refraction ----
        vec3 rf = refract(rd, n, 0.7502);          // 1 / 1.333
        vec3 tran = fogCol;
        float ft2 = 0.03, fHit = 6.0, fMat = 0.0;
        for (int i = 0; i < 40; i++){
            vec2 m = map(pw + rf*ft2);
            if (m.x < 0.004*ft2 + 0.002){ fHit = ft2; fMat = m.y; break; }
            ft2 += m.x*0.92;
            if (ft2 > 12.0) break;
        }
        if (fMat > 0.5){
            vec3 fp = pw + rf*fHit;
            vec3 fn = calcNormal(fp);
            float ffoot = (tW + fHit)*gPX/sqrt(dot(fn, rf)*dot(fn, rf) + 0.02);
            vec3 em;
            tran = shade(fp, rf, fn, fMat, ffoot, 0.0, em) + em;
        }
        // Beer-Lambert down the refracted path, plus what the water itself
        // scatters back. This is the entire reason shallow water over white
        // tile is turquoise and the same water two metres deeper is not: red
        // is gone within a metre, and once the transmitted light is dark
        // enough the in-scattered colour is ALL there is.
        float sig = mix(1.20, 0.18, uClarity);
        vec3 absorb = exp(-sig*vec3(1.20, 0.29, 0.22)*min(fHit, 12.0));
        vec3 inSc = mix(gDeep, gShal, exp(-0.55*fHit))*(0.55 + 0.85*uBright)
                  * mix(1.0, 0.15 + 1.5*lightZone(pw.xz), uZones);
        tran = tran*absorb + inSc*(1.0 - absorb);

        col = mix(tran, refl, F);

        // The glint: a coffer seen in the surface at a grazing angle, smeared
        // along the ripple. Roughness comes back in here, so the far water
        // that lost its normal keeps a long soft streak instead of a mirror.
        vec3 kd; float kdist;
        float lit = lighting(pw + vec3(0.0, 0.01, 0.0), n, kd, kdist);
        vec3 hv = normalize(kd - rd);
        float sp = pow(max(dot(n, hv), 0.0), mix(60.0, 900.0, res));
        col += gLite*sp*lit*(0.5 + 0.6*uBright)*(0.35 + 0.65*F);

        tShow = tW;
    }

    if (tShow < FAR){
        float fg = 1.0 - exp(-uFog*tShow);
        // The fade has to COMPLETE before rays start missing, not at the far
        // plane itself: otherwise a hit at 44 m keeps 15 % of its surface
        // while the ray beside it, which reached 46 and missed, keeps none -
        // and that 15 % is a hard stripe at the vanishing point.
        col = mix(col, fogCol, max(fg, smoothstep(FAR*0.55, FAR*0.92, tShow)));
    }

    // ---- in-scattering: the shafts, and the humidity ----
    if (uHaze > 0.005){
        float tm = min(tShow, FAR);
        float jt = hash12(fragCoord + floor(T*30.0));
        float glow = 0.0;
        for (int i = 0; i < 18; i++){
            // Sample positions depend ONLY on i, never on where the ray
            // landed. Spacing them tm/18 apart ties the whole integral to
            // tHit, which jumps several metres between the ray that reaches
            // the far plane and its neighbour that clips a wall - a hard
            // vertical stripe at the vanishing point. Quadratic in i: fixed in
            // world space, dense near the camera where the halo needs the
            // resolution, coarse out where the fog has already eaten it.
            float u = (float(i) + jt)/18.0;
            float s = FAR*u*u;
            if (s > tm) break;
            float ds = FAR*2.0*u/18.0;
            ds *= min(1.0, (tm - s)/max(ds, 1e-3));
            vec3 q = ro + rd*s;
            // Nearest coffer by FOLDING the lattice, and the local DENSITY of
            // working lights rather than whether one particular fixture is
            // lit: floor() plus panelOn() are both piecewise constant, so the
            // glow steps at every lattice plane - and the camera stands ON the
            // plane x = 0, which projects to a straight full-height line
            // through the vanishing point.
            vec2 f  = mod(q.xz - gLOF, gLG) - 0.5*gLG;
            vec2 d2 = max(abs(f) - PANEL_E, 0.0);
            float dl = length(vec3(d2.x, uHeight - 0.04 - q.y, d2.y));
            float on = uLightOn*mix(1.0, 0.05 + 1.90*lightZone(q.xz), uZones);
            // and the air above water is thicker than the air above a carpet
            float humid = 1.0 + 0.9*exp(-max(q.y - gWL, 0.0)*0.8);
            glow += on*humid*ds*exp(-uFog*2.0*s)/(1.0 + 1.5*dl*dl);
        }
        col += gLite*glow*uHaze*0.30;
    }

    // ---- and then it is put on tape ----
    vec2 sc = fragCoord/R;
    col = col/(1.0 + col*0.80);
    col *= 1.0 - uScan*vhs*0.18*(0.5 + 0.5*sin(fragCoord.y*PI));

    float nz = hash12(fragCoord + floor(T*24.0)*vec2(13.7, 91.3));
    float hb = smoothstep(0.024, 0.0, sc.y);
    col = mix(col, vec3(nz)*0.5 + col*0.45, hb*vhs*0.6);

    float drow = hash11(floor(sc.y*180.0) + floor(T*12.0)*7.3);
    float dseg = step(0.997, hash11(floor(sc.x*40.0)*3.1 + floor(sc.y*180.0)*11.7 + floor(T*12.0)*29.0));
    col += vhs*0.16*dseg*step(0.990, drow)*vec3(1.0, 0.99, 0.95);

    col += (nz - 0.5)*uGrain*0.12;
    float r2 = dot(uv, uv);
    col.r *= 1.0 - vhs*0.08*r2;
    col.b *= 1.0 - vhs*0.13*r2;
    col *= 1.0 - uVignette*1.25*dot(sc - 0.5, sc - 0.5);
    float lum = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(lum), col, 1.38 - vhs*0.20);        // the tone map greys it; put it back
    col = col*(1.0 - vhs*0.05) + vhs*0.018;            // lifted black

    // Half an LSB before the 8-bit write. A white-tiled room is almost
    // entirely smooth low-amplitude gradients, which is exactly where
    // quantisation shows up as contours across a wall - and the grain slider
    // cannot be relied on for this, because a viewer is entitled to zero it.
    col += (hash12(fragCoord + 0.5) - 0.5)/255.0;

    gl_FragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}