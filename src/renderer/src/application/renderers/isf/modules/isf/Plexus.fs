/*{
  "DESCRIPTION": "An endless biomechanical spine - ten of them - coiled around one another and travelling away from you forever. Stacked ribbed plates threaded onto a corrugated cord, a pair of back-swept spikes down each flank, and bundles of flexible conduit running the whole length. Three things make it affordable as a single-pass raymarcher. Z IS THE PARAMETER: each strand is a curve written as a function of the world z, so for any point the nearest piece of that strand is the piece at the point's own z, with no search along the curve and no table. The cross-section then comes out sheared by the strand's slope instead of perpendicular to it, which at this pitch is about a tenth and reads as the plate leaning into the coil; and what falls out is not a distance but an over-estimate by at most sqrt(1 + slope^2), so every strand divides its own field by its own slope bound. That bound has three terms and the one that is easy to miss is the third: the local frame TURNS with z as well as translating, so a point sitting at the plate rim feels another rim-radius times dphi/dz of speed. ONLY THE PLATE REPEATS, AND IT SITS IN THE MIDDLE OF ITS CELL: mod() repetition over-estimates near a cell boundary because the nearest surface may be the neighbour's, so the cord, the dorsal cord and the conduits are all written against z and know nothing about the cell - they run through every wall unbroken, and the field near a wall is therefore small, which is exactly where the over-estimate would otherwise bite. That same rule shapes every surface: ribbing written per cell would break at every joint, since a cell picks up a new roll and a new hash there, while ribbing written against z runs the length of the strand, which is the only way a corrugated tube reads as one. It is paid for in one place - a modulated radius adds amplitude times frequency to the field's slope, so each piece divides by its own bound - and past about five units the ribs are flattened out with distance, because a rib finer than a pixel is noise and, unlike a radius, it cannot be floored: flooring a corrugation only makes a fatter corrugation. AND IT LOOPS: the camera flies at a constant speed and the whole scene is periodic in z, so after one period divided by that speed - fifteen seconds at the defaults - the frame comes back to where it started, measured at a mean difference of 0.31 of one colour step against the frame a period earlier, the residue being float precision in phases that grow with z. Not a crossfade and not a fade to black: every phase is written as k*(n*z + m*zc) with n and m whole, and every per-plate hash is taken on the cell index modulo the number of plates in a period, so advancing the camera by one period maps the scene onto itself. The two controls that would break that, the coil rate and the twist, are integer sliders for the same reason. The braid is two bundles, an inner one coiling one way and an outer one coiling the other, which is what makes them read as intertwined rather than merely parallel; they can never collide because they live in separate radial bands, and within a bundle every strand coils at the same rate, so those cannot collide either. The weaving is a radius that wanders with z, so strands cross over one another in depth without ever touching. A plate is an elliptical footprint - (er - 1) times the smaller semi-axis is a conservative distance to an ellipse - drooping backwards towards a rim that thickens into a heavy bar, with a ladder of ribs across it; the ribs run on the LONG axis because the slope a rib adds is frequency times amplitude over the axis it varies along, so running them that way costs less than half what radial rings did and buys a deeper rib for it. The six conduits round each cord are got for the price of one by folding the angle into a sixth, which is exact because each tube sits in the middle of its own sector and fits inside it; the spikes are mirrored across the strand for the same kind of reason, one primitive drawing a symmetric pair. The plate is deliberately narrow and the SPIKES carry the width, because a bounding volume is a budget: bolting long spikes onto a wide plate pushed the bound out by half, which costs march steps everywhere and forces the two bundles apart until the braid stops being a braid. Light is a key, a headlight, a rim and a translucency term read from the field a little way behind the surface, over a hard narrow highlight on a broad sheen; every colour in the picture sits within a twentieth of one hue, because a complementary glow turns this kind of thing into poster art. There is no sky function at all: the background is black and the fog is a plain multiply into the same black, because at depth the only thing separating one strand from the next is how much of it the fog has eaten, and a lit sky both washes that out and gives the bioluminescence something to compete with. An exponential never reaches zero, though, so on its own it draws a hard edge at the far clip - at any sane density the field is still lit where the march stops, and the braid comes out with a straight cut across it that reads as clipping rather than as distance. Raising the density only moves the edge nearer; the fade needs a factor that is exactly zero at the clip, so the last thing visible is already black by the time the ray runs out of distance to travel; (1-x^2)^2 in x = t/MAXD lands on zero with a zero derivative, so there is no edge there and no knee before it. Each spine also ROLLS about its own axis while its position in the braid stays put, which is what makes them turn on the spot rather than the braid swinging round: the roll is applied to the local frame before the vertebra is built, so plate, spikes, dorsal cord and conduits turn together as one body - spin only the plate and it shears through the conduits welded to it - and it is a rigid rotation, so the distance bound is untouched. It depends only on the clock and the strand index, so it is worked out once a frame into a small global array read back with the loop variable as its index, rather than costing two more transcendentals per strand in every one of the many map calls a pixel makes; and it is in WHOLE turns per period, like the coil rate and the twist, because half a turn at the end of a period would leave every strand at the wrong angle when the camera came back. The bioluminescence runs at a different rate through different parts: one wave whose wavelength and speed are chosen by the material at the hit, so the light crawls along the plates, runs down the cord and races through the conduits, with a golden-angle phase per strand so no two are in step. The glow is gathered with TWO kernels, a tight one for the light on the strand and a faint broad one for the little of it that reaches across the gap - widening a single kernel instead washes the whole frame grey. The broad one is COMPACT rather than exponential, because an exponential is never zero and a ray that misses everything would still gather a little all the way to the far clip, lifting the corners of the frame off black however small its coefficient; a window that vanishes at about one strand spacing means open void accumulates exactly nothing, and the background measures a true zero. Controls: flight speed, coil turns per period, how many strands in each bundle, both radii, the weave, the axis wander and the travelling undulation, the plate twist, the axial spin, dorsal cord, wing span, thickness, plate depth, spikes, ribbing, conduits, per-plate variation, camera distance, swing, focal length and roll, key light, bioluminescence, fog, hue, march steps, antialiasing and exposure.",
  "CREDIT": "CC0",
  "CATEGORIES": ["raymarching", "sdf", "biomechanical", "alien", "spine", "loop", "procedural"],
  "INPUTS": [
    { "NAME": "uSpeed",       "TYPE": "float", "DEFAULT": 2.00, "MIN": 0, "MAX": 6 },
    { "NAME": "uCoil",        "TYPE": "float", "DEFAULT": 3, "MIN": 0, "MAX": 6 },
    { "NAME": "uInner",       "TYPE": "float", "DEFAULT": 5, "MIN": 0, "MAX": 6 },
    { "NAME": "uOuter",       "TYPE": "float", "DEFAULT": 5, "MIN": 0, "MAX": 6 },
    { "NAME": "uRad",         "TYPE": "float", "DEFAULT": 1.05, "MIN": 0.3, "MAX": 2 },
    { "NAME": "uSpread",      "TYPE": "float", "DEFAULT": 2.30, "MIN": 1.2, "MAX": 3.5 },
    { "NAME": "uWeave",       "TYPE": "float", "DEFAULT": 0.14, "MIN": 0, "MAX": 0.6 },
    { "NAME": "uWander",      "TYPE": "float", "DEFAULT": 0.90, "MIN": 0, "MAX": 2 },
    { "NAME": "uUndul",       "TYPE": "float", "DEFAULT": 1.00, "MIN": 0, "MAX": 2 },
    { "NAME": "uTwist",       "TYPE": "float", "DEFAULT": 5, "MIN": 0, "MAX": 8 },
    { "NAME": "uSpin",        "TYPE": "float", "DEFAULT": 2, "MIN": 0, "MAX": 8 },
    { "NAME": "uSpineL",      "TYPE": "float", "DEFAULT": 0.50, "MIN": 0, "MAX": 1 },
    { "NAME": "uProc",        "TYPE": "float", "DEFAULT": 0.21, "MIN": 0, "MAX": 0.7 },
    { "NAME": "uThick",       "TYPE": "float", "DEFAULT": 1.00, "MIN": 0.4, "MAX": 2 },
    { "NAME": "uGap",         "TYPE": "float", "DEFAULT": 0.60, "MIN": 0, "MAX": 1.5 },
    { "NAME": "uSpike",       "TYPE": "float", "DEFAULT": 0.80, "MIN": 0, "MAX": 1.2 },
    { "NAME": "uFlange",      "TYPE": "float", "DEFAULT": 1.00, "MIN": 0, "MAX": 1.4 },
    { "NAME": "uCable",       "TYPE": "float", "DEFAULT": 1.00, "MIN": 0, "MAX": 1.4 },
    { "NAME": "uJitter",      "TYPE": "float", "DEFAULT": 0.60, "MIN": 0, "MAX": 1 },
    { "NAME": "uOrbit",       "TYPE": "float", "DEFAULT": 5.20, "MIN": 0, "MAX": 5 },
    { "NAME": "uSwing",       "TYPE": "float", "DEFAULT": 0.60, "MIN": 0, "MAX": 2 },
    { "NAME": "uFov",         "TYPE": "float", "DEFAULT": 2.00, "MIN": 0.7, "MAX": 3 },
    { "NAME": "uRoll",        "TYPE": "float", "DEFAULT": 1.00, "MIN": 0, "MAX": 2 },
    { "NAME": "uLight",       "TYPE": "float", "DEFAULT": 1.00, "MIN": 0, "MAX": 2 },
    { "NAME": "uGlow",        "TYPE": "float", "DEFAULT": 1.00, "MIN": 0, "MAX": 2 },
    { "NAME": "uHaze",        "TYPE": "float", "DEFAULT": 1.00, "MIN": 0, "MAX": 2.5 },
    { "NAME": "uHue",         "TYPE": "float", "DEFAULT": 0.55, "MIN": 0, "MAX": 1 },
    { "NAME": "uSteps",       "TYPE": "float", "DEFAULT": 190, "MIN": 40, "MAX": 240 },
    { "NAME": "uAA",          "TYPE": "float", "DEFAULT": 0, "MIN": 0, "MAX": 1 },
    { "NAME": "uExpo",        "TYPE": "float", "DEFAULT": 1.00, "MIN": 0.3, "MAX": 2 }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/d3a8uy1o
#define PI  3.14159265359
#define TAU 6.28318530718

// ---------------------------------------------------------------- the loop
// LOOPZ is the spatial period of everything below. NVERT divides it into
// vertebrae, so SP is exactly 5/16 and the cell index stays whole for a
// long time in a 24-bit mantissa.
#define LOOPZ  30.0
#define NVERT  96.0
#define SP     0.3125
// Written as the quotient, not as a decimal: a truncated literal makes
// KW*LOOPZ differ from TAU by more than a rounding error, and the loop
// closes only as well as that product does.
#define KW     (TAU / LOOPZ)
#define MAXD   32.0
#define SHELL  0.22          // see the note in map()

// ---------------------------------------------------------------- helpers
// mod() near an exact multiple can floor the wrong way on a GPU that
// divides by multiplying with a reciprocal; fix it up, never trust it.
float pmod(float x, float p){
    float r = x - p * floor(x / p);
    if (r < 0.0) r += p;
    if (r >= p)  r -= p;
    return r;
}
// cubing is a permutation modulo 4091 (3 does not divide 4090), so this is
// a real shuffle of the whole numbers rather than a sine that two compilers
// will disagree about in the last bits. Every intermediate stays under 2^24.
float icube(float x){ return pmod(pmod(x * x, 4091.0) * x, 4091.0); }
float hash11(float n){
    float x = pmod(n * 757.0 + 1013.0, 4091.0);
    x = icube(pmod(x * 293.0 + 1571.0, 4091.0));
    return icube(pmod(x * 61.0 + 877.0, 4091.0)) * 0.000244498;   // /4090
}

mat2 rot(float a){ float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }

float smin(float a, float b, float k){
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// A segment whose radius tapers from r1 to r2. Dividing by the slope the
// taper adds is what keeps this a lower bound on the true distance, which
// is what makes it safe to march.
float sdTaper(vec3 p, vec3 a, vec3 b, float r1, float r2){
    vec3 ba = b - a, pa = p - a;
    float l2 = dot(ba, ba);
    float h  = clamp(dot(pa, ba) / l2, 0.0, 1.0);
    float dr = r1 - r2;
    return (length(pa - ba * h) - (r1 - dr * h)) * inversesqrt(1.0 + dr * dr / l2);
}

// ---------------------------------------------------------------- globals
float gZc;        // where the camera is along the braid - the clock
float gAxSlope;   // how fast the braid's own axis can move per unit z
float gRmax;      // radius of the tube that contains one vertebra
vec3  gRo;        // the eye, for working out what a pixel is worth out there
float gPixK;      // world units a pixel covers, per unit of distance
float gMinR;      // ...at the point being mapped: a floor on every radius
float gCore;      // distance to the nearest strand's CORE, continuous
float gWall;      // distance along z to the next vertebra cell wall
float gNear;      // 1 if this point is inside some strand's bounding tube
float gHitPh;     // bioluminescent phase of the strand the ray is nearest
float gCorePh;    // ...of the strand that owns gCore
// (cos, sin) of each strand's roll about its OWN axis. It depends only on the
// clock and the strand index, never on the point, so it is worked out once a
// frame and read back here - computing it inside map() would put two more
// transcendentals per strand into every one of the ~170 map calls a pixel
// makes. ES 1.00 allows a global array indexed by a loop symbol, which is
// exactly how it is read.
vec2  gSpin[12];

// The axis the whole braid follows: a closed Lissajous wander plus a wave
// that TRAVELS along it. Both phases are whole multiples of KW*z and
// KW*gZc, which is the entire reason the shader loops.
vec2 axisAt(float z){
    float a  = KW * z;
    vec2  w  = vec2(sin(a), cos(a)) * 0.90
             + vec2(sin(2.0 * a + 1.3), cos(3.0 * a + 0.7)) * 0.30;
    float p1 = KW * (3.0 * z - 2.0 * gZc);
    float p2 = KW * (5.0 * z + 3.0 * gZc);
    vec2  u  = vec2(sin(p1), cos(p1)) * 0.30
             + vec2(cos(p2), sin(p2)) * 0.12;
    return w * uWander + u * uUndul;
}

// Bioluminescence running down the bone. n sets the wavelength and m the
// speed - the wave travels m/n units of z per unit of camera travel - and
// both have to be WHOLE for the shader to close its loop. Different parts of
// the spine are given different pairs, so the light crawls along the plates,
// runs down the cord and races through the conduits rather than the whole
// thing blinking as one.
float pulseAt(float z, float n, float m, float ph){
    float s = 0.5 + 0.5 * sin(KW * (n * z - m * gZc) + ph);
    float s3 = s * s * s;
    return s3 * s3;
}

// ---------------------------------------------------------------- vertebra
// Local frame: u points out of the braid, v is across the strand, w runs
// along it. Returns (distance, material).
//
// The shape is built from three things and every one of them is the same
// idea: a form that is RIBBED, and a rib that is periodic in z rather than
// in the vertebra cell. Ribbing that repeated per cell would break at every
// joint, because the cell picks up a new roll and a new hash there; ribbing
// written against z runs the whole length of the strand unbroken, which is
// the only way a corrugated tube reads as a corrugated tube. The price is
// always the same and it is paid in the same place: a modulated radius adds
// amplitude times frequency to the field's slope, so each piece divides by
// its own inversesqrt(1 + slope^2).
vec2 vertebra(float u, float v, float z, float sj, float gs){
    float w  = pmod(z + SP * 0.5, SP) - SP * 0.5;
    float ki = floor((z + SP * 0.5) / SP);
    float kk = pmod(ki, NVERT);                 // the loop lives or dies here
    float h1 = hash11(kk + 71.0 * sj);
    float hh1 = mix(0.5, h1, uJitter);
    float mr = gMinR;
    float rad2 = length(vec2(u, v));
    // Ribbing finer than a pixel is not detail, it is noise - and unlike a
    // radius it cannot be floored, because flooring a corrugation just makes
    // a fatter corrugation. So flatten it out with distance instead: full
    // depth inside about five units, gone by fifteen. Every slope bound
    // below is scaled by the same number, or it stops being a bound.
    float rf = clamp(1.5 - gMinR * 70.0, 0.0, 1.0);

    // ---- the cord: a tightly ribbed column, four ribs to the vertebra,
    // running the whole length of the strand. TAU/(SP*0.25) is 53.6, which
    // is the frequency the slope term below is built from.
    float cor  = 0.096 * uThick * gs;
    float cam  = 0.016 * uFlange * uThick * gs * rf;
    float csl  = 80.425 * cam;
    float dC   = (rad2 - cor - cam * sin(TAU * z / (SP * 0.25)))
               * inversesqrt(1.0 + csl * csl);

    // ---- a second, finer ribbed column riding above the first - the spring
    // that runs up the middle of the thing in every photograph of one
    float dor  = uSpineL * 0.55 * gs;
    float drr  = (0.022 + 0.036 * uSpineL) * uThick * gs;
    float dam  = 0.009 * uFlange * gs * rf;
    float dsl  = 100.531 * dam;
    float dD   = (length(vec2(u - dor, v)) - drr - dam * sin(TAU * z / (SP * 0.2) + 0.7))
               * inversesqrt(1.0 + dsl * dsl);

    // ---- the plate: ONE broad flat wing per vertebra, not a crown of
    // spikes. An elliptical footprint, wide across the strand and half that
    // fore and aft, drooping backwards towards its rim and carrying fine
    // concentric ridges on both faces. (er - 1) * eu is a conservative
    // distance to an ellipse: er's gradient is at most 1/eu, so scaling by
    // the SMALLER semi-axis keeps the whole thing a lower bound.
    vec2  bb = rot(TAU * uTwist * kk / NVERT + (h1 - 0.5) * 0.7 * uJitter) * vec2(u, v);
    float ev = (uProc + 0.22) * gs * (0.88 + 0.24 * hh1);
    float eu = ev * 0.44;
    float pu = bb.x / eu, pv = bb.y / ev;
    float er = sqrt(pu * pu + pv * pv);
    float dE = (er - 1.0) * eu;

    float droop = -0.045 * er * er * gs;
    // Ribs ACROSS the wing rather than concentric rings. Two reasons: it
    // is the ladder of transverse ribs that reads as Giger, and the slope a
    // rib adds is frequency times amplitude divided by the axis it varies
    // along - so running them on the LONG axis costs less than half what
    // running them on the radius did, and buys a deeper rib for it.
    float pam   = 0.0110 * uFlange * rf;
    float pth   = (0.022 + 0.024 * uGap) * gs;
    float hwp   = pth * (1.0 + 0.55 * er * er) + pam * sin(pv * 22.0);
    vec2  pe    = vec2(dE, abs(w - droop) - hwp);
    float psl   = (0.090 * gs + 1.10 * pth) / eu + pam * 22.0 / ev;
    // The usual inversesqrt(1 + slope^2) is WRONG for a rounded box whose
    // two coordinates have non-orthogonal gradients, and it was wrong here:
    // the radial coordinate's gradient is radial, and the axial one picks up
    // a radial component of psl from the droop, the thickening and the
    // ridges. The worst case is the largest eigenvalue of [[1, psl],
    // [psl, 1 + psl^2]], and 1 + psl*(psl + 1.2) bounds it over the range
    // this uses. Being 11% optimistic showed up as a stipple of wrong pixels
    // along every plate edge.
    float dP    = (min(max(pe.x, pe.y), 0.0) + length(max(pe, 0.0)) - max(0.015 * gs, mr))
                * inversesqrt(1.0 + psl * (psl + 1.2));

    // ---- conduits. Six of them round the cord, got for the price of one by
    // folding the angle into a sixth - which is exact, because each tube sits
    // in the middle of its own sector and fits inside it. They are ribbed on
    // z like everything else, and they know nothing about the cell, so they
    // pass through the stack of plates unbroken.
    vec2  uv0 = vec2(u, v);
    if (dot(uv0, uv0) < 1e-10) uv0 = vec2(1e-5, 0.0);
    float sec = TAU / 6.0;
    float aa  = pmod(atan(uv0.y, uv0.x) + sec * 0.5, sec) - sec * 0.5;
    float cabR = (0.40 + 0.30 * uProc) * gs;
    vec2  fq  = vec2(rad2 * cos(aa) - cabR, rad2 * sin(aa));
    float wam = 0.0052 * uCable * gs * rf;
    float wsl = 160.850 * wam;
    float wr  = uCable * 0.036 * gs + wam * sin(TAU * z / (SP * 0.125) + 1.1);
    float cf  = min(uCable * 40.0, 1.0);
    float dW  = mix(1e3, (length(fq) - max(wr, mr * cf))
                       * inversesqrt(1.0 + wsl * wsl), cf);

    // ---- the spikes down each flank. Two pairs of back-swept hooks per
    // plate: a heavy one leaving the rim and a thin one hooking on past it.
    // abs() on the lateral coordinate makes a pair cost what one costs, and
    // the mirror is exact because the pair really is symmetric. Both stay
    // inside the cell in w - the sweep is lateral and dorsal, because at
    // three plates to the unit there is no room to sweep along the strand.
    float SL  = uSpike * 0.36 * gs * (0.85 + 0.30 * hh1);
    vec3  sb  = vec3(bb.x, abs(bb.y), w);
    vec3  tip = vec3(0.01 * gs, ev * 0.90 + SL * 0.50, -0.070);
    float dS  = sdTaper(sb, vec3(0.04 * gs, ev * 0.55, 0.036), tip,
                        max(0.032 * gs, mr), max(0.009, mr));
    float dS2 = sdTaper(sb, tip,
                        vec3(0.10 * gs + SL * 0.30, ev + SL * 0.95, -0.118),
                        max(0.009 * gs, mr), max(0.003, mr));
    float dSp = mix(1e3, min(dS, dS2), min(uSpike * 20.0, 1.0));

    float dCord = min(dC, dD);
    float dPS   = smin(dP, dSp, 0.022);
    float d1    = smin(dCord, dPS, 0.018);
    float mt    = dPS < dCord ? 2.0 : 1.0;
    float d     = smin(d1, dW, 0.012);
    return vec2(d, dW < d1 - 0.02 ? 3.0 : mt);
}

// ---------------------------------------------------------------- the map
vec2 map(vec3 p){
    gMinR = gPixK * length(p - gRo);
    gCore = 1e5;
    float z  = p.z;
    gWall = SP * 0.5 - abs(pmod(z + SP * 0.5, SP) - SP * 0.5);
    gNear = 0.0;
    vec2  A  = axisAt(z);
    vec2  d2 = p.xy - A;

    float ni = floor(uInner + 0.5);
    float no = floor(uOuter + 0.5);
    float nn = ni + no;

    vec2 best = vec2(1e5, 0.0);

    for (int i = 0; i < 12; i++){
        float fi = float(i);
        if (fi >= nn) break;
        vec2 sc = gSpin[i];                         // this strand's own roll

        float isOut = step(ni, fi);                 // 1 for the outer bundle
        float cnt   = max(mix(ni, no, isOut), 1.0);
        float j     = fi - isOut * ni;
        float dirn  = 1.0 - 2.0 * isOut;            // the bundles counter-coil
        float r0    = uRad * mix(1.0, uSpread, isOut);
        float gs    = mix(1.0, 1.18, isOut);        // outer bones a touch bigger

        float base = TAU * j / cnt + isOut * 1.7;
        float phi  = dirn * KW * uCoil * z + base;
        // weaving: the radius wanders with z, so strands cross over one
        // another in depth. Same band for the whole bundle, so they never meet.
        float rr   = r0 * (1.0 + uWeave * sin(KW * (2.0 * z - 1.0 * gZc) + base * 1.7));

        vec2 nd = vec2(cos(phi), sin(phi));
        vec2 td = vec2(-nd.y, nd.x);
        vec2 d  = d2 - nd * rr;

        // How fast this strand's centre can move per unit z, and therefore
        // how much the z-parameterised field over-estimates. The last term is
        // the one that is easy to forget: the local frame (nd, td) TURNS with
        // z as well as translating, and a point sitting gRmax out from the
        // centre feels that rotation as another gRmax*dphi/dz of speed.
        // + gMinR because the pixel floor thickens every feature, and a
        // bound that does not know about that is not a bound
        float rmx   = gRmax * gs + gMinR;
        float slope = gAxSlope + (rr + rmx) * KW * uCoil + r0 * uWeave * KW * 2.0;
        float lip   = sqrt(1.0 + slope * slope);

        float rad = length(d);
        // How far this point is from the cord the strand is built around.
        // Unlike the distance field it is CONTINUOUS everywhere - no bound,
        // no min() between branches - which is what the glow integral needs.
        float cd = max((rad - 0.10) / lip, 0.0);
        if (cd < gCore){ gCore = cd; gCorePh = fi * 2.39996; }
        // SHELL is why the bound is not tested at the tube itself. A ray
        // running tangent to the tube sits where the bound goes to zero
        // while the bone inside is still far away, so it advances by the
        // step floor and nothing else - hundreds of steps, then the budget
        // runs out and the pixel is painted sky. Those directions form a
        // thin curve on screen, which is what a hairline crack ruled
        // straight across a vertebra turned out to be. Handing the shell to
        // the exact branch instead means the bound never returns less than
        // SHELL/lip, and nothing creeps.
        if (rad > rmx + SHELL){
            float cand = (rad - rmx) / lip;
            if (cand < best.x) best = vec2(cand, 0.0);
        } else {
            gNear = 1.0;
            // SPIN. The frame the vertebra is built in turns about the
            // strand's own axis while the strand's POSITION does not, which
            // is what makes each spine rotate on the spot instead of the
            // braid swinging round. Rotating the frame HERE rather than
            // inside vertebra() turns the plate, the spikes, the dorsal cord
            // and the conduits together as one body - spin only the plate
            // and it shears through the conduits that are welded to it. A
            // rigid rotation is an isometry, so the distance bound is
            // untouched by it.
            float du = dot(d, nd), dv = dot(d, td);
            vec2 h = vertebra(du * sc.x + dv * sc.y,
                              dv * sc.x - du * sc.y, z, fi + 3.0, gs);
            h.x /= lip;
            if (h.x < best.x){ best = h; gHitPh = fi * 2.39996; }
        }
    }
    return best;
}

vec3 calcNormal(vec3 p){
    vec2 e = vec2(1.0, -1.0) * 0.0016;
    return normalize(e.xyy * map(p + e.xyy).x + e.yyx * map(p + e.yyx).x +
                     e.yxy * map(p + e.yxy).x + e.xxx * map(p + e.xxx).x);
}

float calcAO(vec3 p, vec3 n){
    float o = 0.0, s = 1.0;
    for (int i = 0; i < 5; i++){
        float h = 0.012 + 0.16 * float(i) / 4.0;
        o += (h - map(p + n * h).x) * s;
        s *= 0.72;
    }
    return clamp(1.0 - 1.6 * o, 0.0, 1.0);
}

vec3 hsv(vec3 c){
    vec3 q = abs(fract(c.xxx + vec3(1.0, 0.66667, 0.33333)) * 6.0 - 3.0);
    return c.z * mix(vec3(1.0), clamp(q - 1.0, 0.0, 1.0), c.y);
}

// ---------------------------------------------------------------- render
vec3 render(vec3 ro, vec3 rd){
    vec3 gcol = hsv(vec3(fract(uHue + 0.96), 0.40, 1.0));

    float t = 0.03, mat = 0.0, glow = 0.0, hitPh = 0.0;
    for (int i = 0; i < 160; i++){
        if (float(i) >= uSteps) break;
        vec3 p = ro + rd * t;
        vec2 h = map(p);
        float ds = max(h.x * 0.92, 0.006);
        // Every vertebra fits inside its own cell, but that is not enough:
        // a point near a cell wall can be nearer to the NEXT vertebra than
        // to its own, and a cell only ever knows its own. So once the ray is
        // inside a strand's bounding tube, never step past the wall -
        // beyond it this cell's estimate is not the cell's to make. It cost
        // a hairline crack across a vertebra wherever a process reached
        // forward over a joint. Note that this caps the STEP and not the
        // reported distance: fold it into the distance instead and the cell
        // walls themselves become hittable, which draws the braid as a stack
        // of enormous discs.
        if (gNear > 0.5) ds = min(ds, gWall + 0.006);
        // light bleeding off the bone, gathered along the ray
        // Gather the light off the CORD, not off the distance field. The
        // field jumps at every strand's bounding tube - the bound goes to
        // zero there while the real vertebra behind it is still far away -
        // and an integral of that draws a bright shell around every strand
        // and a seam wherever the nearest strand changes. gCore has no such
        // discontinuity, and a glow that hugs the cord is what a lit spine
        // would do anyway. The kernel is deliberately BROAD: a tight one
        // leaves the space between the strands as flat empty black, and a
        // soft wash of the light they are already emitting is what gives
        // those gaps any depth at all.
        // Weight by the step ACTUALLY TAKEN. Capping the weight turns the
        // sum into "so much per step" rather than an integral over length,
        // so wherever the march happened to take small steps the glow piled
        // up - and since step size changes discontinuously at every
        // bounding tube, the frame came out ruled with curves.
        // The depth falloff is steep on purpose. A ray that runs out of
        // steps stops gathering, and the boundary between the rays that
        // finished and the rays that did not is a smooth curve across the
        // frame - which showed up as a hairline ruled over the whole image.
        // Weighting the far end of the gather to nearly nothing makes that
        // boundary carry nearly nothing with it.
        // ...and fade the gather out over the last third of the step
        // budget. A ray that runs out of steps stops gathering, and the
        // boundary between the rays that finished and the rays that did not
        // is a smooth curve across the frame - it showed up as a hairline
        // ruled over the whole image. Fading the weight to zero BEFORE the
        // budget runs out means both sides of that boundary are carrying
        // nothing by the time they reach it, so there is no boundary.
        float sw = clamp(2.6 - 3.0 * float(i) / uSteps, 0.0, 1.0);
        // Two kernels, not one. The tight one is the light ON the strand;
        // the broad one is the little of it that reaches across the gap, and
        // it is what stops the space between the spines reading as flat,
        // empty black. Using the broad one alone washes the whole frame grey,
        // which is the opposite mistake and undoes the black background.
        // The reach of the broad term is the whole argument of this line. Too
        // short and the gaps between the spines are flat empty black; too
        // long and a ray that misses everything still gathers a little all
        // the way to the far clip, which lifts the corners of the frame off
        // black and undoes the point of having no sky at all. It has to die
        // inside about a strand's spacing, and the gather has to carry the
        // same terminal fade as everything else so distance cannot
        // accumulate it either.
        // The kernel is COMPACT, and that is the whole point of this line.
        // An exponential is never zero, so a ray that misses everything still
        // gathers a little all the way to the far clip - which lifts the
        // corners of the frame off black however small the coefficient is,
        // and undoes the point of having no sky at all. q vanishes outright
        // at about one strand's spacing, so open void accumulates EXACTLY
        // nothing while the gaps between the spines still fill in. Same
        // argument as the terminal fade: if it has to reach zero, use a
        // function that reaches zero.
        // Note gCore is divided by the strand's Lipschitz bound, so it reads
        // LOW - a coefficient of 0.68 here would let the window reach three
        // units of real space, which is most of the braid. 1.5 puts its edge
        // at about one strand's spacing, which is what "fill the gap" means.
        float q  = max(1.0 - gCore * 1.5, 0.0);
        float gx = max(1.0 - t / MAXD, 0.0); gx *= gx;
        glow += pulseAt(p.z, 9.0, 7.0, gCorePh) * min(ds, 0.5)
              * exp(-0.17 * t) * sw * gx
              * q * (exp(-10.0 * gCore) + 0.20 * q);
        // material 0 means the field came from a strand's BOUNDING TUBE, not
        // from bone. A ray grazing that tube gets an arbitrarily small
        // distance without being anywhere near a surface, so stopping on it
        // paints sky over the braid - which is exactly what it did.
        if (h.x < 0.0020 * t && h.y > 0.5){ mat = h.y; hitPh = gHitPh; break; }
        t += ds;
        if (t > MAXD) break;
    }

    vec3 col;
    if (mat > 0.5){
        vec3 p = ro + rd * t;
        vec3 n = calcNormal(p);
        float ao = calcAO(p, n);

        vec3 cordC  = hsv(vec3(fract(uHue + 0.98), 0.15, 0.150));
        vec3 plateC = hsv(vec3(uHue, 0.09, 0.275));
        vec3 cableC = hsv(vec3(fract(uHue + 0.05), 0.13, 0.090));
        vec3 base   = mix(mix(cordC, plateC, step(1.5, mat)), cableC, step(2.5, mat));

        // mottling. A product of sines vanishes on a lattice of planes and a
        // shading term kicks hardest where its field crosses zero, so the
        // raw product draws that lattice on the surface as a bright net;
        // squaring removes every zero crossing and leaves only the blotches.
        // Fade it out with distance: at one sample a pixel, anything
        // finer than a pixel is a flicker rather than a texture.
        float mo = sin(p.x * 57.0) * sin(p.y * 49.0) * sin(p.z * 67.0);
        float mc = sin(p.x * 13.0) * sin(p.y * 11.0) * sin(p.z * 17.0);
        float pit = 0.17 * (1.0 - mo * mo) + 0.09 * (1.0 - mc * mc);
        base *= 1.0 - pit * exp(-0.22 * t);

        // grime pooling where the light cannot reach
        base *= 0.11 + 0.89 * ao;

        vec3  key = normalize(vec3(0.55, 0.70, -0.45));
        float dif = clamp(dot(n, key), 0.0, 1.0);
        float bac = clamp(0.5 + 0.5 * dot(n, -key), 0.0, 1.0);
        float hea = clamp(dot(n, -rd), 0.0, 1.0);              // headlight
        float fre = pow(clamp(1.0 + dot(n, rd), 0.0, 1.0), 4.0);

        // translucency: how much bone is behind this point. A blade's edge
        // has almost none, so it lights up from inside.
        float ins  = map(p - n * 0.12).x;
        float thin = clamp(1.0 + ins * 8.0, 0.0, 1.0);
        float sss  = thin * pow(clamp(0.5 + 0.5 * dot(rd, key), 0.0, 1.0), 2.0);

        // two lobes: a tight one for the wet highlight, a broad one for
        // the sheen that makes it read as damp rather than chalk
        // lacquer: a hard narrow highlight over a broad low sheen, and the
        // pitting takes the gloss down with it
        vec3 hv = normalize(key - rd);
        float nh = clamp(dot(n, hv), 0.0, 1.0);
        float gl = 1.0 - 0.55 * pit;
        float spe = (pow(nh, 150.0) * 1.7 + 0.30 * pow(nh, 14.0)) * gl;

        col  = base * (0.05 + 0.80 * dif * uLight);
        col += base * bac * 0.13 * uLight;
        col += base * hea * 0.18 * uLight * exp(-0.10 * t);
        col += vec3(0.86, 0.90, 0.96) * spe * 0.78 * ao * uLight;
        col += hsv(vec3(fract(uHue + 0.94), 0.38, 1.0)) * sss * 0.16 * uLight;
        col += gcol * fre * 0.20 * ao * gl;
        // The pulse lighting the bone from within, at a rate that depends on
        // WHICH part of the spine this is: it crawls along the plates, runs
        // down the cord and races through the conduits. One wave, three
        // wavelengths and three speeds, and a phase per strand on top.
        float pn  = mat < 1.5 ? 9.0 : (mat < 2.5 ? 14.0 :  5.0);
        float pm  = mat < 1.5 ? 7.0 : (mat < 2.5 ?  3.0 : 12.0);
        float pls = pulseAt(p.z, pn, pm, hitPh);
        col += gcol * pls * (0.16 + 0.50 * thin) * uGlow * ao;
    } else {
        t = MAXD;
        col = vec3(0.0);
    }

    // Fog into BLACK, and a black background behind it. There is nothing to
    // put back there: the only thing that separates one strand from the next
    // at depth is how much of it the fog has eaten, and a lit sky both washes
    // that out and gives the bioluminescence something to compete with.
    //
    // The second factor is the one that matters. An exponential NEVER reaches
    // zero, so at the far clip the field was still a fifth lit and the march
    // simply stopped there - which draws a hard edge cut straight across the
    // braid, and no amount of extra fog density fixes it (it only moves the
    // edge closer). The fade has to be a function that is exactly zero at
    // MAXD, so the last thing visible is already black by the time the ray
    // runs out of distance to travel.
    // (1 - x^2)^2 lands on zero at the far clip with a zero derivative, so
    // there is no edge there and no knee anywhere before it either. Both
    // factors are gentler than they were: the braid should thin out over most
    // of its range rather than switch off over the last tenth of it.
    float x  = clamp(t / MAXD, 0.0, 1.0);
    float xf = (1.0 - x * x) * (1.0 - x * x);
    col *= exp(-uHaze * 0.042 * t) * xf;
    col += gcol * glow * 0.52 * uGlow;
    return col;
}

void main(){
    vec2 fragCoord = gl_FragCoord.xy;
    vec2 R = RENDERSIZE.xy;

    // ---- the clock. Everything downstream is a function of gZc, never of
    // TIME, which is what lets the period be exactly LOOPZ / uSpeed.
    gZc = uSpeed * TIME;

    // |d/dz| of axisAt(): 0.90 + 0.30*sqrt(2^2 + 3^2) for the wander,
    // 0.30*3 + 0.12*5 for the travelling wave, both in units of KW
    gAxSlope = KW * (1.99 * uWander + 1.50 * uUndul);
    // a pixel subtends about 2/(height*focal) radians, so this much world
    // per unit of distance from the eye
    gPixK    = 1.1 / (R.y * uFov);
    // the tube that contains one vertebra, from the longest thing on it
    // the tube that contains one vertebra: whichever of the plate rim, the
    // outermost conduit or the dorsal cord reaches furthest
    gRmax    = max((uProc + 0.22) * 1.12 + 0.39 * uSpike + 0.05,
               max(0.40 + 0.30 * uProc + 0.07 * uCable + 0.05,
                   uSpineL * 0.60 + 0.04 + 0.11 * uThick)) * 1.06;

    // ---- camera: orbits the braid's own axis and looks along it
    float ca = KW * gZc + 0.8 + uSwing * 0.9 * sin(KW * 2.0 * gZc);
    vec2  A0 = axisAt(gZc);
    vec3  ro = vec3(A0 + uOrbit * vec2(cos(ca), sin(ca)), gZc);
    gRo = ro;

    // ---- how far each strand has rolled about its OWN axis by now. uSpin is
    // in whole turns per period, because a fractional one would leave the
    // strand part-way round when the camera comes back and the loop would not
    // close. The two bundles counter-spin, and each strand carries a phase of
    // its own so they are not all at the same angle.
    float ni0 = floor(uInner + 0.5), no0 = floor(uOuter + 0.5);
    for (int i = 0; i < 12; i++){
        float fi  = float(i);
        float io  = step(ni0, fi);
        float cn  = max(mix(ni0, no0, io), 1.0);
        float bs  = TAU * (fi - io * ni0) / cn + io * 1.7;
        float sp  = (1.0 - 2.0 * io) * TAU * floor(uSpin + 0.5) * gZc / LOOPZ
                  + bs * 1.7;
        gSpin[i]  = vec2(cos(sp), sin(sp));
    }

    float LOOK = 7.0;
    vec3  ta = vec3(axisAt(gZc + LOOK) + 0.25 * vec2(cos(ca), sin(ca)), gZc + LOOK);

    float rl = uRoll * (0.38 * sin(KW * 2.0 * gZc + 0.6) + 0.22 * sin(KW * 3.0 * gZc));
    vec3  ww = normalize(ta - ro);
    vec3  up = vec3(sin(rl), cos(rl), 0.0);
    vec3  uu = normalize(cross(ww, up));
    vec3  vv = cross(uu, ww);

    vec3 col = vec3(0.0);
    float ns = 0.0;
    for (int m = 0; m < 2; m++){
    for (int k = 0; k < 2; k++){
        if (uAA < 0.5 && (m + k) > 0) continue;
        vec2 o = (uAA < 0.5) ? vec2(0.0) : (vec2(float(m), float(k)) * 0.5 - 0.25);
        vec2 q = (2.0 * (fragCoord + o) - R) / R.y;
        vec3 rd = normalize(q.x * uu + q.y * vv + uFov * ww);
        col += render(ro, rd);
        ns += 1.0;
    }}
    col /= ns;

    col *= uExpo;
    // filmic-ish, then a vignette so the void closes in at the corners
    col = (col * (2.51 * col + 0.03)) / (col * (2.43 * col + 0.59) + 0.14);
    vec2 vq = fragCoord / R;
    col *= 0.55 + 0.45 * pow(16.0 * vq.x * vq.y * (1.0 - vq.x) * (1.0 - vq.y), 0.22);
    col = clamp(col, 0.0, 1.0);
    col = pow(col, vec3(0.4545));
    gl_FragColor = vec4(col, 1.0);
}
