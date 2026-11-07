/*{
  "DESCRIPTION": "An endless liminal office, generated rather than modelled. There is no level file and no mesh: every partition, every ceiling tile and every dead fluorescent tube is decided by hashing an integer cell index, so the building is unbounded in all four directions and identical for everyone who runs it. The geometry is a 2D problem wearing a 3D coat - partitions run floor to ceiling without exception, so the whole maze is a plane figure extruded and the scene distance is nothing more than min(floor, ceiling, footprint), where footprint is a 2D box SDF evaluated over the 3x3 cells around the sample. That is why a 128-step march through an infinite building costs about what a single sphere costs: the expensive dimension was never there. One cell holds one partition, with its orientation, length, lateral offset and slide read off a four-component hash of the cell id and a small chance of a square pillar instead; doorways are punched by subtracting a second, shorter box, and because that hole is height-dependent the subtraction is the only place the third dimension enters the maze at all. The camera has to be able to GO somewhere, which a hash alone will not guarantee, so one lane of cells is forced empty - a cell is skipped when its centre lies within about a cell of a smooth wandering curve x = path(z) - and the camera walks that same curve. The corridor is therefore carved by the very function that drives the walk and can never be walled off; turn wander to zero and it straightens into a single long hallway. Light is the whole subject. Troffers sit on their own lattice, 2x4 panels with a quarter of them dead and a few of them dying, and every surface gathers from the nine nearest treating each as a RECTANGLE rather than a point: the closest point on the panel is clamped to its extents, so a wall a metre below a tube gets the long soft falloff a real one does instead of a hotspot. A shadow ray is fired only at the strongest of the nine and the other eight are carried by ambient occlusion, which is what keeps a nine-light scene at the cost of a one-light scene. A second, coarse march then accumulates in-scattering from those same panels into the fog, and that is what puts the hazy halo around every fixture and the shafts down the far end of the corridor - the glow is integrated along the ray rather than smeared on afterwards, so it sits correctly in front of and behind the geometry. The surfaces are procedural too: wallpaper as a half-drop repeat of one of four motifs (a chevron stack, a damask lily, a sparse dot and a striped bead), drop-ceiling tile as a two-octave stipple with water rings, discoloured panels and the occasional missing one, and carpet as two octaves of value noise at loop-pile frequency with traffic wear and damp. None of that survives contact with a horizon unless something stands in for a mip chain, and fading by DISTANCE alone is not enough - a wall seen edge-on packs a hundred tiles into ten pixels while still an arm's length away. So each surface is told how many metres of itself one pixel covers, which is the pixel's world size at the hit distance divided by the grazing cosine, and each texture fades toward its own average as that footprint approaches its feature size. Without the cosine term the chevrons alias into broad moire arcs across every oblique wall; with it they simply go quiet. The same care is why the shadow ray is aimed along the WEIGHTED MEAN of the nine panel directions rather than at the strongest of them: the strongest one changes as you slide along a wall, the loci where two panels tie are smooth curves, and the shadow term steps across them - which paints the wall in concentric arcs. A mean moves continuously and has no seams. Then it is put on tape: the ray direction is bent by a barrel term and shoved sideways per scanline BEFORE it is ever traced, so the tracking error lives in the optics rather than being painted over them, and the frame picks up scanlines, head-switching hash along the bottom edge, dropouts, grain, edge tint and a lifted black. Where the lights have failed is decided by a slowly varying field over the plan rather than one tube at a time, so the dead ones clump: a flat per-panel probability scatters them evenly and every distance ends up equally lit, whereas clumping puts a genuinely dark room at the end of a corridor. That same field dims the ambient and the fog, so a dark zone is dark the whole way through instead of merely missing its highlights. The troffers land in the tile grid rather than across it: the tile size is a quarter of the light spacing, so a panel centred at (lid + 0.5)*LG sits on an exact multiple of the tile - a T-bar - and every fixture straddles a grid line by half a tile until x is offset by half a tile and the panel's size is taken from the spacing instead of from fixed metres, which keeps it exactly one tile by two at any setting. And sometimes there is a figure standing down the corridor - pale, featureless, its legs dissolving before they reach the floor, built to real proportions out of ten pieces with an egg-shaped head, a short neck, sloped shoulders, a chest narrowing to a waist, arms with an elbow and legs with a knee. A neck is a width pinch rather than a gap, and a capsule ending at the shoulders puts a hemispherical cap a full radius ABOVE its endpoint, so a chest of radius 0.15 ending at 1.50 bulges to the underside of the head and swallows the neck whole; stopping the chest lower and giving the shoulders their own sloped piece takes the silhouette from 0.43 m wide at the shoulders to 0.10 m at the neck to 0.18 m at the head inside twenty centimetres, and that contrast is the neck. A torso is also not round - a circular capsule as wide as a chest is equally deep and reads as a barrel, so z is squashed by 1.45 before the trunk is evaluated. Both of those lean on the same freedom: the tapered capsule is not a valid distance field wherever its radii differ, and the squash breaks the metric outright, and neither matters because the field is only ever sampled. It is never marched as a surface; the distance field does not know about it. It is integrated as a soft volume along the primary ray inside its own bounding sphere - which is also what makes it cheap to break: the sample position can simply be warped before the figure is evaluated, so horizontal slices tear sideways, a slow boil runs underneath, a scanline sweeps down it and the whole thing drops out for the odd beat, all of it updating on an 11 Hz beat rather than continuously, because a value that holds for ninety milliseconds and then jumps reads as a recording fault where a smooth wobble would only read as motion; a second, x-offset density accumulated in the same loop drives the blue channel, giving a real spatial separation rather than a coloured fringe - which is the whole difference between something you can see through and a white statue, and it costs eighteen samples over a few per cent of the frame. The walls occlude it for nothing, because the integration simply stops at whatever the marcher already hit. Rather more than half of the doorway openings are still hung with a leaf, which is a second thin box in the same footprint as the opening - one more box and no extra hash, since fract(g.z*13.7) is decorrelated enough from the g.z that already decided whether there is an opening at all. Thirty-six controls: walk speed, focal length, yaw and pitch, how much the corridor wanders and how sharply it turns, handheld sway, cell size, wall density, doorway frequency, ceiling height, light spacing, what fraction of the tubes still work, brightness, flicker, how much the outages clump into dark zones, haze, fog, a palette that crossfades classic yellow into sickly green, institutional grey and sepia dusk, the wallpaper motif and its scale, baseboard, grime, damp, blood, carpet pile, ambient occlusion, shadow, the figure, how many of them and how badly they glitch, and five tape controls for the analogue damage.",
  "CREDIT": "CC0",
  "CATEGORIES": [
    "generator",
    "raymarch",
    "liminal",
    "backrooms",
    "horror",
    "architecture",
    "procedural",
    "vhs",
    "3d"
  ],
  "INPUTS": [
    {
      "NAME": "uSpeed",
      "TYPE": "float",
      "DEFAULT": 3.0,
      "MIN": 0.0,
      "MAX": 6.0
    },
    {
      "NAME": "uLens",
      "TYPE": "float",
      "DEFAULT": 1.45,
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
      "DEFAULT": 0.0,
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
      "DEFAULT": 0.5,
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
      "DEFAULT": 4.0,
      "MIN": 2.5,
      "MAX": 8.0
    },
    {
      "NAME": "uDensity",
      "TYPE": "float",
      "DEFAULT": 0.72,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uDoors",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uRooms",
      "TYPE": "float",
      "DEFAULT": 0.3,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uHeight",
      "TYPE": "float",
      "DEFAULT": 3.05,
      "MIN": 2.2,
      "MAX": 5.0
    },
    {
      "NAME": "uLightSpace",
      "TYPE": "float",
      "DEFAULT": 2.44,
      "MIN": 1.22,
      "MAX": 6.1
    },
    {
      "NAME": "uLightOn",
      "TYPE": "float",
      "DEFAULT": 0.72,
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
      "NAME": "uFlicker",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uZones",
      "TYPE": "float",
      "DEFAULT": 0.65,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uHaze",
      "TYPE": "float",
      "DEFAULT": 0.55,
      "MIN": 0.0,
      "MAX": 1.5
    },
    {
      "NAME": "uFog",
      "TYPE": "float",
      "DEFAULT": 0.03,
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
      "NAME": "uPattern",
      "TYPE": "float",
      "DEFAULT": 3.0,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "uPatScale",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.4,
      "MAX": 2.5
    },
    {
      "NAME": "uTrim",
      "TYPE": "float",
      "DEFAULT": 0.55,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGrime",
      "TYPE": "float",
      "DEFAULT": 0.55,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uDamp",
      "TYPE": "float",
      "DEFAULT": 0.45,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uBlood",
      "TYPE": "float",
      "DEFAULT": 0.3,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uCarpet",
      "TYPE": "float",
      "DEFAULT": 0.8,
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
      "DEFAULT": 0.85,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uFigure",
      "TYPE": "float",
      "DEFAULT": 0.5,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGlitch",
      "TYPE": "float",
      "DEFAULT": 0.45,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uTrails",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGhosts",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 1.0,
      "MAX": 4.0
    },
    {
      "NAME": "uVHS",
      "TYPE": "float",
      "DEFAULT": 0.55,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uWobble",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uScan",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uGrain",
      "TYPE": "float",
      "DEFAULT": 0.28,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "uVignette",
      "TYPE": "float",
      "DEFAULT": 0.5,
      "MIN": 0.0,
      "MAX": 1.0
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "3D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/36v1cyvi


// BACKROOMS - ISF port of the Shadertoy/WebGL2 version
// (GLSL ES 1.00: gl_FragColor, TIME/RENDERSIZE).
//
// An infinite hashed office: partitions extruded from a 2D box SDF over a
// 3x3 cell neighbourhood, a corridor carved by the same curve the camera
// walks, rectangular ceiling troffers gathered nine at a time with one
// shadow ray, in-scattered haze, procedural wallpaper/tile/carpet, and a
// VHS pass applied to the ray direction rather than to the frame.

#define PI 3.14159265359
#define FAR 54.0

// A real 2 ft ceiling tile. This is the MODULE: everything on the ceiling is
// measured in it.
//
// It used to be a quarter of the light spacing, which meant the ceiling tile
// had no fixed size at all - open the light-spacing slider and the tiles grew
// with it, so the ceiling could end up gridded in metre-and-a-half squares and
// the whole room lost its sense of scale. A ceiling tile is 2 ft in every
// building this shader is pretending to be.
#define TILE 0.6096

// So the light lattice snaps to a WHOLE NUMBER of tiles, which is what lets a
// 2x4 troffer sit exactly in the grid at any spacing rather than straddling a
// T-bar. gLOF is the half-tile nudge that puts the panel on a tile centre
// across and on a tile boundary along - which parity it needs depends on
// whether the cell is an odd or an even number of tiles wide.
float gLG;
vec2  gLOF;

#define PANEL_C(lid)  ((lid + 0.5)*gLG + gLOF)
#define PANEL_E       (vec2(0.5*TILE, TILE))

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

// Every octave is ROTATED as well as scaled.
//
// Value noise is smooth but its iso-contours run along its own lattice, so
// without a rotation all four octaves share one set of axis-aligned cell edges
// and those edges stack up. On a large flat surface that reads as a quilt of
// squares - which is exactly, and entirely, what made the carpet look blocky.
// One mat2 multiply per octave buys an isotropic field instead.
float fbm(vec2 p){
    mat2 R = mat2(0.8040, 0.5946, -0.5946, 0.8040);   // 36.5 degrees
    float s = 0.0, a = 0.5;
    for (int i = 0; i < 4; i++){
        s += a*vnoise(p);
        p = R*p*2.07 + 19.3;
        a *= 0.5;
        if (a < 0.03) break;
    }
    return s;
}

// ---------------------------------------------------------------
// primitives
// ---------------------------------------------------------------
float sdBox2(vec2 p, vec2 b){ vec2 q = abs(p) - b; return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0); }

float smin(float a, float b, float k){
    float h = clamp(0.5 + 0.5*(b - a)/k, 0.0, 1.0);
    return mix(b, a, h) - k*h*(1.0 - h);
}

// A tapered capsule.
//
// This is deliberately NOT a valid distance field on the taper - it
// under-estimates wherever r1 and r2 differ, which would put acne all over a
// sphere-traced surface. It does not matter here, and that is the point: the
// figure below is never traced, only SAMPLED, so the field only has to have
// the right zero set and a sane gradient near it. Dropping the requirement to
// be a true SDF is what buys the anatomy for nine cheap primitives.
float sdCone2(vec3 p, vec3 a, vec3 b, float r1, float r2){
    vec3 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba)/dot(ba, ba), 0.0, 1.0);
    return length(pa - ba*h) - mix(r1, r2, h);
}

// A standing figure, 1.80 m and about eight and a half heads tall, matched to
// the canonical backrooms apparition.
//
// Two things decide whether it reads as a person, and the first attempt got
// both wrong.
//
// THE NECK IS A WIDTH PINCH, not a gap. It survives only if nothing else is
// wide up at that height — and a capsule ending at the shoulders puts a
// hemispherical cap a full radius ABOVE its endpoint, which swallowed the neck
// whole. So the chest stops at 1.40 and the shoulder girdle is its own sloped
// piece, leaving the silhouette to go 0.43 m across at the shoulders, 0.10 m
// at the neck and 0.18 m at the head within twenty centimetres.
//
// AND A TORSO IS NOT ROUND. A circular capsule as wide as a chest is just as
// deep, and reads as a barrel. Squashing z by 1.45 before the torso pieces are
// evaluated costs one multiply and makes it a person.
//
// It is never marched as a SURFACE. map() does not know about it at all: it is
// integrated as a soft volume along the primary ray, inside its own bounding
// sphere. That is the whole difference between something you can see through
// and a white statue, and it costs twenty-two samples over a few per cent of
// the frame. The walls still occlude it, because the integration simply stops
// at whatever the marcher already hit.
float figure(vec3 p){
    vec3 q = vec3(abs(p.x), p.y, p.z);            // mirrored, for the limbs
    vec3 s = vec3(p.x, p.y, p.z*1.45);            // flattened, for the trunk
    vec3 r = vec3(abs(p.x), p.y, p.z*1.45);       // both

    // Head: a plain round skull. Nine of them to the figure - that ratio is
    // what makes the reference read as tall rather than heroic.
    float d = length((p - vec3(0.0, 1.730, 0.002))/vec3(1.0, 1.06, 1.02)) - 0.096;

    // Neck: under half the width of the head. That RATIO is the single most
    // important number in the whole silhouette.
    d = smin(d, sdCone2(p, vec3(0.0, 1.470, 0.004), vec3(0.0, 1.645, 0.004), 0.043, 0.047), 0.028);

    // shoulders, 0.46 m across and sloping
    d = smin(d, sdCone2(r, vec3(0.020, 1.480, 0.0), vec3(0.172, 1.448, 0.0), 0.066, 0.060), 0.045);

    // Chest to hips, barely tapered: this figure is a column, not an hourglass.
    // The chest STOPS at 1.40 so that its cap cannot reach up and swallow the
    // neck, which is the trap this shape falls into every single time.
    d = smin(d, sdCone2(s, vec3(0.0, 1.400, 0.0), vec3(0.0, 1.180, 0.0), 0.145, 0.124), 0.050);
    d = smin(d, sdCone2(s, vec3(0.0, 1.180, 0.0), vec3(0.0, 0.900, 0.0), 0.124, 0.140), 0.058);

    // Arms: thin, hanging almost straight, hands to mid-thigh. Held at 0.19
    // with a 0.126 chest the armpit slot is 27 mm, so the blend has to stay
    // under that or the gap the reference shows simply closes up.
    d = smin(d, sdCone2(q, vec3(0.186, 1.448, 0.006), vec3(0.196, 1.005, 0.016), 0.043, 0.035), 0.030);
    d = smin(d, sdCone2(q, vec3(0.196, 1.005, 0.016), vec3(0.201, 0.828, 0.002), 0.035, 0.024), 0.032);
    // a hand only reads if there is a WRIST for it to be wider than
    d = smin(d, sdCone2(q, vec3(0.201, 0.820, 0.000), vec3(0.198, 0.746, -0.004), 0.034, 0.026), 0.018);

    // legs: long, touching at the hip, parting below the knee
    d = smin(d, sdCone2(q, vec3(0.086, 0.905, 0.0), vec3(0.098, 0.414, 0.0), 0.086, 0.062), 0.068);
    d = smin(d, sdCone2(q, vec3(0.098, 0.414, 0.0), vec3(0.086, 0.140, 0.0), 0.062, 0.041), 0.044);

    // Feet TURNED OUT. Pointed at the camera they foreshorten to stubs and
    // vanish; the reference stands with them splayed, which is the only reason
    // they read at all from the front.
    d = smin(d, sdCone2(q, vec3(0.088, 0.072, 0.012), vec3(0.196, 0.032, -0.056), 0.041, 0.027), 0.028);
    return d;
}

// ---------------------------------------------------------------
// the building
//
// pathX() is both the corridor and the walk: cells near it are left
// empty and the camera follows it, so there is always somewhere to
// go and never a wall in the face.
// ---------------------------------------------------------------
// The walk, and the corridor, are the same curve: x = pathX(z).
//
// uTurn is not "more wiggle" - the amplitudes are large enough that the
// SLOPE reaches about 1.1, which is a heading of 48 degrees off the axis, so
// the camera genuinely turns and heads somewhere else rather than drifting
// laterally inside one long hallway. The frequencies are incommensurate with
// the base pair and with each other, so it never comes back to the same
// heading twice.
float pathX(float z){
    return uWander*(2.6*sin(z*0.0290) + 1.35*sin(z*0.0161 + 1.7))
         + uTurn*(11.0*sin(z*0.0473 + 2.1) + 7.0*sin(z*0.0281 + 5.3)
                + 4.5*sin(z*0.0917 + 0.7));
}

// d(pathX)/dz. Needed because the lane is carved by HORIZONTAL distance from
// the curve, and a horizontal band of fixed width is a corridor that gets
// narrower the more steeply the path runs - at 48 degrees it would close to
// two thirds and the camera would start clipping walls. Widening the band by
// sqrt(1 + slope^2) keeps the corridor a constant width measured across it.
float pathSlope(float z){
    return uWander*(0.0754*cos(z*0.0290) + 0.0217*cos(z*0.0161 + 1.7))
         + uTurn*(0.5203*cos(z*0.0473 + 2.1) + 0.1967*cos(z*0.0281 + 5.3)
                + 0.4127*cos(z*0.0917 + 0.7));
}

// .x = partition footprint, .y = doorway slots, .z = the leaves hung in some
// of those slots. All three are 2D; the height of the opening and of the leaf
// is applied in map(), which is the only place the third dimension appears.
vec3 maze2(vec2 q){
    float C = uCell;
    vec2 base = floor(q/C);
    float dW = C, dD = C, dL = C;
    for (int j = 0; j < 3; j++){
        // the lane only depends on the row, so pathX is evaluated once here
        // rather than nine times below
        float rz = (base.y + float(j) - 0.5)*C;
        float px = pathX(rz);
        float pw = 0.78*C*sqrt(1.0 + pathSlope(rz)*pathSlope(rz));
        for (int i = 0; i < 3; i++){
            vec2 id = base + vec2(float(i) - 1.0, float(j) - 1.0);
            vec2 c  = (id + 0.5)*C;
            vec4 h  = hash42(id);
            vec4 g  = hash42(id + 57.31);

            // one lane of cells is held open for the camera to walk
            float lane = step(pw, abs(c.x - px));
            float on   = step(h.x, uDensity)*lane;

            float horiz = step(0.5, h.y);
            vec2 dirL = mix(vec2(0.0, 1.0), vec2(1.0, 0.0), horiz);   // along the wall
            vec2 dirT = mix(vec2(1.0, 0.0), vec2(0.0, 1.0), horiz);   // through it

            float pil = step(0.88, g.x);                              // sometimes a pillar
            float len = mix((0.62 + 0.38*h.z)*0.5*C, 0.26, pil);
            float th  = mix(0.085, 0.26, pil);
            vec2  ctr = c + dirL*((h.w - 0.5)*(C - 2.0*len))
                          + dirT*((g.y - 0.5)*0.70*(C - 2.0*th));

            // Some cells are a closed ROOM instead of a free-standing partition:
            // a rectangular ring, which is the difference of two boxes. Without
            // this the plan is only ever scattered slabs, and however far the
            // density is pushed you never end up anywhere with four walls.
            float isRoom = step(fract(h.y*31.7), uRooms);
            float rr = 0.5*C - 0.16;
            float ring = max(sdBox2(q - c, vec2(rr)), -sdBox2(q - c, vec2(rr - 0.09)));
            float slab = sdBox2(q - ctr, dirL*len + dirT*th);

            // an absent cell is pushed far away rather than branched around
            dW = min(dW, mix(slab, ring, isRoom) + (1.0 - on)*90.0);

            // A room ALWAYS gets its opening - it would otherwise be a sealed
            // box you could never be inside - and it sits in the middle of one
            // wall, which also makes the leaf's centre recoverable later.
            float dr = on*mix(step(g.z, uDoors)*(1.0 - pil), 1.0, isRoom);
            vec2 dc = mix(ctr + dirL*((g.w - 0.5)*2.0*max(len - 0.62, 0.0)),
                          c + dirT*rr, isRoom);
            vec2 de = mix(dirL*0.46 + dirT*(th + 0.40), dirL*0.46 + dirT*0.30, isRoom);
            dD = min(dD, sdBox2(q - dc, de) + (1.0 - dr)*90.0);

            // Rather more than half of those openings are still hung with a
            // leaf. It reuses q - dc, so it costs one more box and no hash:
            // fract(g.z*13.7) is decorrelated enough from g.z, which already
            // decided whether there is an opening at all.
            float lf = dr*step(fract(g.z*13.7), 0.58);
            dL = min(dL, sdBox2(q - dc, dirL*0.44 + dirT*0.022) + (1.0 - lf)*90.0);

            if (dW < -0.5*C) break;   // deep inside a partition; nothing nearer matters
        }
        if (dW < -0.5*C) break;
    }
    // a box two cells away is at least a cell distant, so clamping here is safe
    return vec3(min(dW, C), min(dD, C), min(dL, C));
}

// .x = distance, .y = material (1 carpet, 2 ceiling, 3 wall, 4 door)
vec2 map(vec3 p){
    vec2 r = vec2(p.y, 1.0);
    float dC = uHeight - p.y;
    if (dC < r.x) r = vec2(dC, 2.0);

    vec3 mz = maze2(p.xz);
    float dh = min(1.015, uHeight*0.5 - 0.11);        // door half-height, capped by the ceiling
    float door = max(mz.y, abs(p.y - dh) - dh);       // the hole is the only 3D part
    float dW = max(mz.x, -door);
    if (dW < r.x) r = vec2(dW, 3.0);

    // the leaf standing in the opening, a shade shorter than the hole it fills
    float leaf = max(mz.z, abs(p.y - dh*0.99) - dh*0.975);
    if (leaf < r.x) r = vec2(leaf, 4.0);
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
        if (o > 2.0) break;
    }
    return clamp(1.0 - 1.1*o, 0.0, 1.0);
}

float shadowRay(vec3 ro, vec3 rd, float maxt){
    float res = 1.0, t = 0.16, prev = 1e12;
    for (int i = 0; i < 28; i++){
        float h = map(ro + rd*t).x;
        // how far back along the ray the closest approach actually was
        float y = h*h/(2.0*prev);
        float d = sqrt(max(h*h - y*y, 0.0));
        res = min(res, d/max(0.0001, 7.0*(t - y)));
        prev = h;
        t += clamp(h, 0.04, 0.5);
        if (res < 0.02 || t > maxt) break;
    }
    return clamp(res, 0.0, 1.0);
}

// ---------------------------------------------------------------
// the ceiling grid of fluorescent troffers
// ---------------------------------------------------------------

// A slowly varying field over the plan - roughly one feature every 25 m - that
// decides where the lights have failed in BLOCKS rather than one tube at a
// time. A flat per-panel probability scatters the dead ones evenly and every
// distance comes out equally lit; clumping them is what puts a genuinely dark
// room at the end of a corridor. The same field dims the ambient and the fog,
// so a dark zone is dark all the way through rather than merely missing its
// highlights.
float lightZone(vec2 xz){ return smoothstep(0.20, 0.80, vnoise(xz*0.026)); }

float panelOn(vec2 lid){
    vec4 h = hash42(lid + 11.7);
    float zn = lightZone(PANEL_C(lid));
    float on = step(h.x, uLightOn*mix(1.0, 0.05 + 1.90*zn, uZones));
    float ph = h.y*97.0 + h.z*13.0;
    float dying = step(h.w, 0.35*uFlicker);
    float s = hash11(floor(TIME*20.0)*0.7331 + ph);
    float fl = mix(1.0, step(0.26, s)*(0.55 + 0.55*s), dying);
    return on*fl*(0.965 + 0.035*sin(TIME*47.0 + ph));   // and the mains hum
}

// Gathers the nine nearest panels as RECTANGLES, and hands back one direction
// to fire a single shadow ray along.
//
// That direction is the WEIGHTED MEAN of the nine, not the strongest of them.
// Picking the strongest is the obvious thing and it is wrong: the winner
// changes as you slide across a wall, and the loci where two panels tie are
// smooth curves, so the shadow term steps across them and the wall comes out
// covered in concentric arcs. A mean moves continuously and has no seams.
float lighting(vec3 p, vec3 n, out vec3 keyDir, out float keyDist){
    float LG = gLG;
    vec2 lb = floor(p.xz/LG);
    float sum = 0.0, accD = 0.0;
    vec3 acc = vec3(0.0);
    for (int j = 0; j < 3; j++){
        for (int i = 0; i < 3; i++){
            vec2 lid = lb + vec2(float(i) - 1.0, float(j) - 1.0);
            vec2 lc  = PANEL_C(lid);
            vec2 pe  = PANEL_E;
            float on = panelOn(lid);
            vec3 pl = vec3(clamp(p.x, lc.x - pe.x, lc.x + pe.x), uHeight - 0.03,
                           clamp(p.z, lc.y - pe.y, lc.y + pe.y));
            vec3 L = pl - p;
            float dl = max(length(L), 0.06);
            L /= dl;
            // A panel must be worth NOTHING by the time the window drops it.
            // The 3x3 neighbourhood only guarantees inclusion out to 1.5*LG
            // horizontally, so fade each one to zero before that and the sum
            // becomes continuous across every cell edge.
            vec2 ho = max(abs(p.xz - lc) - pe, 0.0);
            float w = on*max(dot(n, L), 0.0)*(0.30 + 0.70*max(L.y, 0.0))
                    * smoothstep(1.45*LG, 1.02*LG, length(ho))/(1.0 + 0.32*dl*dl);
            sum  += w;
            acc  += L*w;
            accD += dl*w;
            if (sum > 60.0) break;
        }
        if (sum > 60.0) break;
    }
    keyDir  = normalize(acc + n*0.001 + vec3(0.0, 0.0001, 0.0));
    keyDist = accD/max(sum, 0.0001);
    return sum;
}

// ---------------------------------------------------------------
// palettes: 0 classic yellow, 1 sickly green, 2 institutional grey,
// 3 sepia dusk. Continuous, so the slider crossfades between them.
// ---------------------------------------------------------------
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
// wallpaper: four motifs on a half-drop repeat
// ---------------------------------------------------------------
// How much of a surface one pixel covers, and therefore how much of a texture
// is safe to draw. exp(-distance) is not enough on its own: a wall seen edge-on
// packs a hundred tiles into ten pixels at arm's length. Dividing the pixel's
// world size by the grazing cosine is the whole correction, and it is exactly
// the term a mip chain would be computing.
// The transition has to be WIDE. Every texture keys its strength off foot, so
// a narrow fade puts a visible contour on the surface wherever the level
// changes - and since foot depends on dot(n, rd), those contours are conics
// about the normal, which is to say concentric rings across an oblique wall.
// Four mip levels gave four nested rings.
float mip(float feature, float foot){ return smoothstep(1.10, 0.10, foot/feature); }

// Box-filter coverage of the band |x| < r by a pixel w wide.
//
// This is the part a fixed smoothstep cannot do. A wallpaper stroke is about a
// centimetre across, so past a few metres — and immediately on any wall seen
// obliquely — it is thinner than a pixel, and a smoothstep with hard-coded
// edges keeps drawing it at full strength wherever it happens to land. That is
// moire: the pattern beats against the pixel grid and comes out as broad
// smeared arcs that read as a stretched texture. Filtering by the actual
// footprint degrades to the band's AREA FRACTION instead, so the paper simply
// goes quietly grey at the far end.
float band(float x, float r, float w){
    float h = 0.5*max(w, 1e-5);
    return clamp((min(x + h, r) - max(x - h, -r))/(2.0*h), 0.0, 1.0);
}

// the same idea for a filled region given as a signed distance
float fill(float d, float w){ return clamp(0.5 - d/max(w, 1e-5), 0.0, 1.0); }

float motifChevron(vec2 q, float w){
    float m = 0.0;
    for (int k = 0; k < 3; k++){
        float fk = float(k);
        // the arms are y = a - 1.15|x|; 0.66 renormalises that slope to a distance
        float y = (q.y + 0.30 - fk*0.215 - 1.15*(0.14 - abs(q.x)))*0.66;
        m = max(m, band(y, 0.038 - 0.006*fk, w*0.66)*band(q.x, 0.29, w));
        if (m > 0.999) break;
    }
    float dia = abs(q.x)*1.7 + abs(q.y - 0.355) - 0.085;
    m = max(m, fill(dia, w*1.7));
    // a pair of beads flanking the crown, and a fine stem below the stack
    m = max(m, fill(length((q - vec2(0.150, 0.330))*vec2(1.0, 1.15)) - 0.030, w));
    m = max(m, fill(length((q + vec2(0.150, -0.330))*vec2(1.0, 1.15)) - 0.030, w));
    m = max(m, band(q.x, 0.011, w)*band(q.y + 0.395, 0.070, w));
    return m;
}

float motifLily(vec2 q, float w){
    vec2 a = vec2(abs(q.x), q.y);
    float m = band(q.x, 0.034, w)*band(q.y - 0.02, 0.31, w);          // stem
    vec2 e = (a - vec2(0.135, 0.02))/vec2(0.130, 0.200);              // side scrolls
    m = max(m, band(length(e) - 1.0, 0.23, w/0.165)*(1.0 - band(a.x, 0.045, w)));
    m = max(m, fill(length((q - vec2(0.0,  0.31))*vec2(1.0, 0.62)) - 0.070, w));
    m = max(m, fill(length((q - vec2(0.0, -0.31))*vec2(1.0, 0.80)) - 0.052, w));
    return m;
}

float motifDot(vec2 q, float w){
    float d = abs(q.x)*1.3 + abs(q.y) - 0.105;
    float e = max(abs(q.x) - 0.030, abs(abs(q.y) - 0.30) - 0.075);
    return max(fill(d, w*1.3), fill(e, w));
}

float motifStripe(vec2 q, float w){
    float m = band(abs(q.x) - 0.34, 0.038, w);
    m = max(m, 0.55*band(abs(q.x) - 0.20, 0.055, w));
    float b = length(vec2(q.x, mod(q.y + 0.5, 0.25) - 0.125)*vec2(1.0, 1.4));
    return max(m, fill(b - 0.055, w));
}

float wallpaper(vec2 uv, float foot){
    vec2 T = vec2(0.165, 0.224)/max(uPatScale, 0.05);
    vec2 g = uv/T;
    g.x += 0.5*mod(floor(g.y), 2.0);     // half-drop, the way paper is actually hung
    vec2 q = fract(g) - 0.5;
    float w = foot/T.y;                  // one pixel, expressed in tiles
    float s = floor(uPattern + 0.5);
    float m;
    if (s < 0.5)      m = motifChevron(q, w);
    else if (s < 1.5) m = motifLily(q, w);
    else if (s < 2.5) m = motifDot(q, w);
    else              m = motifStripe(q, w);
    // a fine printed stripe down every repeat, which most of these papers have
    // and which is the last thing still legible once the motif is a pixel wide
    return max(m, 0.30*band(abs(q.x) - 0.5, 0.022, w));
}

// ---------------------------------------------------------------
void main(){
    vec2 fragCoord = gl_FragCoord.xy;
    vec2 R = RENDERSIZE.xy;
    float T = TIME;
    float t = T*uSpeed;

    vec3 cWall  = pick4(vec3(0.80, 0.71, 0.29), vec3(0.76, 0.82, 0.50), vec3(0.66, 0.64, 0.60), vec3(0.72, 0.60, 0.36), uPalette);
    vec3 cInk   = pick4(vec3(0.53, 0.45, 0.20), vec3(0.45, 0.55, 0.44), vec3(0.46, 0.46, 0.45), vec3(0.42, 0.34, 0.22), uPalette);
    vec3 cFloor = pick4(vec3(0.34, 0.27, 0.10), vec3(0.46, 0.48, 0.26), vec3(0.44, 0.42, 0.39), vec3(0.46, 0.36, 0.22), uPalette);
    vec3 cTile  = pick4(vec3(0.74, 0.69, 0.53), vec3(0.80, 0.84, 0.74), vec3(0.72, 0.72, 0.70), vec3(0.66, 0.58, 0.42), uPalette);
    vec3 cTrim  = pick4(vec3(0.84, 0.79, 0.62), vec3(0.82, 0.85, 0.76), vec3(0.75, 0.75, 0.73), vec3(0.70, 0.62, 0.46), uPalette);
    vec3 cLite  = pick4(vec3(1.00, 0.94, 0.66), vec3(0.86, 1.00, 0.82), vec3(0.92, 0.96, 1.00), vec3(1.00, 0.86, 0.56), uPalette);

    // the light lattice, snapped to a whole number of ceiling tiles
    float ltn = max(2.0, floor(uLightSpace/TILE + 0.5));
    gLG  = TILE*ltn;
    gLOF = 0.5*TILE*vec2(1.0 - mod(ltn, 2.0), mod(ltn, 2.0));

    // ---- the walk ----
    float z = t*1.15;
    vec3 ro = vec3(pathX(z) + uBob*0.020*sin(t*2.6), 1.62 + uBob*0.030*sin(t*5.2), z);

    float dz = 0.35;
    vec3 fw = normalize(vec3(pathX(z + dz) - pathX(z - dz), 0.0, 2.0*dz));
    vec3 trav = fw;                 // before yaw: where the camera is HEADING
    float yaw = uYaw + uBob*0.05*sin(t*0.83 + 1.0);
    float cy = cos(yaw), sy = sin(yaw);
    fw = normalize(vec3(fw.x*cy + fw.z*sy, 0.0, fw.z*cy - fw.x*sy));
    vec3 rt = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(rt, fw);
    fw = normalize(fw + up*(uPitch + uBob*0.020*sin(t*1.7)));
    rt = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    up = cross(rt, fw);
    float roll = uBob*0.014*sin(t*2.1 + 0.5);
    vec3 rr = rt*cos(roll) + up*sin(roll);
    up = up*cos(roll) - rt*sin(roll);
    rt = rr;

    // ---- the lens, bent before anything is traced ----
    vec2 uv = (fragCoord - 0.5*R)/R.y;
    float vhs = uVHS;
    float ln = hash11(floor(fragCoord.y) + floor(T*24.0)*17.0);
    uv.x += vhs*uWobble*(0.0035*(ln - 0.5) + 0.0040*sin(T*1.9 + fragCoord.y*0.035));
    uv *= 1.0 + vhs*0.050*dot(uv, uv);
    vec3 rd = normalize(uv.x*rt + uv.y*up + uLens*fw);

    // ---- march ----
    float tt = 0.0, tHit = FAR, mat = 0.0;
    for (int i = 0; i < 192; i++){
        vec2 m = map(ro + rd*tt);
        // The hit test has to open up with distance or a grazing ray down a
        // 40 m hall runs out of steps. It cannot open up too far, though:
        // stopping short by e is stopping short ALONG THE RAY, and on a surface
        // seen at a grazing angle that is a slide of e/cos ALONG the surface -
        // tens of centimetres, quantised, which bands every lighting term
        // computed from the hit point. Hence a tight-ish epsilon here and the
        // Newton step below rather than one loose epsilon doing both jobs.
        if (m.x < 0.0022*tt + 0.0008){ tHit = tt; mat = m.y; break; }
        tt += m.x*0.90;
        if (tt > FAR) break;
    }

    // The haze is only as bright as the tubes feeding it. Sample the outage
    // field a fixed distance along the ray, NOT at the hit point: keying it to
    // tHit made the fog colour jump across every silhouette in the frame, and
    // the fog at two metres contributes nothing anyway.
    float zf = lightZone((ro + rd*20.0).xz);
    vec3 fogCol = mix(cWall, cLite, 0.45)*(0.045 + 0.105*uBright)
                * mix(1.0, 0.10 + 1.50*zf, uZones);
    vec3 col = fogCol;

    if (mat > 0.5){
        vec3 p = ro + rd*tHit;
        vec3 n = calcNormal(p);

        // One Newton step onto the true intersection. Moving dt along the ray
        // changes the signed distance by dt*dot(n, rd), so the residual maps
        // straight onto a correction in t. Exact for a plane, one extra map(),
        // and it is what removes the contour banding on oblique walls.
        float ndv0 = min(dot(n, rd), -0.06);
        tHit += clamp(-map(p).x/ndv0, 0.0, 0.75);
        p = ro + rd*tHit;

        // Metres of surface covered by one pixel, grazing angle included.
        //
        // The grazing term is sqrt(c*c + k) rather than max(|c|, k): a hard
        // clamp has an EDGE where it engages, and on a flat wall that edge is a
        // cone about the normal - a ring painted across the surface. The
        // softened form has the same limit and no boundary at all.
        float ndv  = dot(n, rd);
        float foot = tHit/(uLens*R.y*sqrt(ndv*ndv + 0.004));
        float det  = mip(0.010, foot);      // carpet pile, ceiling stipple
        float detW = mip(0.22/max(uPatScale, 0.05), foot);   // what the filter cannot save
        float detM = mip(0.45, foot);       // the big soft stains
        vec3 alb = vec3(0.5);
        vec3 emisC = vec3(0.0);

        if (mat < 1.5){
            // ---- carpet ----
            //
            // Loop pile is fine and it has a NAP: the two octaves are stretched
            // on opposite axes, which is what stops it reading as generic noise
            // and starts it reading as something woven.
            float lp = 0.55*vnoise(p.xz*vec2(205.0, 96.0))
                     + 0.45*vnoise(p.xz*vec2(88.0, 255.0) + 11.0);

            // The pile is sub-pixel past a couple of metres, and with nothing
            // between it and the metre-scale stains the floor went flat exactly
            // where most of it is on screen. This is that missing octave.
            float mid = vnoise(p.xz*13.0 + 5.0);

            float gr  = fbm(p.xz*0.60 + 3.0);
            float mot = vnoise(p.xz*2.6 + 8.0);
            alb = cFloor*(1.0 + uCarpet*0.50*(lp - 0.5)*det);
            alb *= 1.0 + uCarpet*0.17*(mid - 0.5)*mip(0.075, foot);
            alb *= 0.86 + 0.26*gr;
            alb *= 1.0 + uCarpet*0.16*(mot - 0.5)*mip(0.38, foot);
            alb = mix(alb, alb*vec3(0.74, 0.70, 0.63), smoothstep(0.40, 1.10, gr)*uGrime*detM);
            alb *= 1.0 - 0.26*uDamp*detM*smoothstep(0.38, 1.10, fbm(p.xz*0.90 + 31.0) + 0.10*lp);

            // Blood, as SPLATTER rather than one pool per cell.
            //
            // Warping the whole plane once and then testing plain blobs is
            // what makes this affordable: the per-blob fbm the first version
            // used meant only the cell under the sample could be afforded, so
            // any blob bigger than its own cell was cut off at the boundary.
            // With the warp hoisted out, the 3x3 neighbourhood is just nine
            // hashes and nothing can clip.
            //
            // The radius is cubed, so most draws are 4-6 cm spots and only the
            // occasional one opens out into a pool.
            vec2 bw = p.xz*1.15 + 0.42*(vec2(vnoise(p.xz*2.3),
                                             vnoise(p.xz*2.3 + 19.0)) - 0.5);
            vec2 bb = floor(bw);
            float blood = 0.0;
            for (int bj = 0; bj < 3; bj++){
                for (int bi = 0; bi < 3; bi++){
                    vec2 bid = bb + vec2(float(bi) - 1.0, float(bj) - 1.0);
                    vec4 bh  = hash42(bid + 61.3);
                    float br = 0.05 + 0.30*bh.z*bh.z*bh.z;
                    vec2 bo  = (bw - (bid + 0.25 + 0.5*bh.xy))/vec2(1.0, 0.72 + 0.5*bh.w);
                    blood = max(blood, step(hash12(bid + 3.1), uBlood*0.6)
                                     * smoothstep(br, br*0.35, length(bo)));
                    if (blood > 0.99) break;
                }
                if (blood > 0.99) break;
            }
            alb = mix(alb, vec3(0.038, 0.008, 0.006)*(0.75 + 0.5*hash12(bb)),
                      blood*0.95*mip(0.055, foot));
        } else if (mat < 2.5){
            // ---- drop ceiling, and the troffers set into it ----
            float LG = gLG;
            vec2 lid = floor(p.xz/LG);
            vec2 dp  = abs(p.xz - PANEL_C(lid)) - PANEL_E;
            if (max(dp.x, dp.y) < 0.0){
                float on = panelOn(lid);
                vec2 pg = fract((p.xz - PANEL_C(lid))/(PANEL_E.x*0.25));
                float pr = smoothstep(0.0, 0.16, min(min(pg.x, 1.0 - pg.x), min(pg.y, 1.0 - pg.y)));
                pr = mix(1.0, 0.72 + 0.40*pr, det);       // prismatic diffuser
                emisC = cLite*(1.6 + 7.0*uBright)*on*pr;
                alb = cLite*0.50;
            } else {
                float TS = TILE;                           // a fixed 2 ft, not a fraction of LG
                vec2 tid = floor(p.xz/TS);
                vec2 tf  = fract(p.xz/TS);
                float grid = min(min(tf.x, 1.0 - tf.x), min(tf.y, 1.0 - tf.y));
                // a 15 mm T-bar is sub-pixel across most of a ceiling, so it gets
                // the same treatment, settling to the grid's average coverage
                float wt = foot/TS;
                float bar = 1.0 - mix(band(grid, 0.025, wt), 0.10, smoothstep(0.06, 0.30, wt));
                vec4 th = hash42(tid + 3.7);
                float st = 0.5*vnoise(p.xz*95.0) + 0.5*vnoise(p.xz*260.0);
                alb = cTile*(0.60 + 0.40*bar)*(0.86 + 0.20*th.x)*(1.0 + 0.26*(st - 0.5)*det);
                float ring = smoothstep(0.52, 0.80, fbm(p.xz*1.40 + th.yz*11.0))*uDamp*step(th.w, 0.45)*detM;
                alb = mix(alb, vec3(0.30, 0.23, 0.13), ring*0.75);
                alb *= mix(1.0, 0.05, step(0.982, th.z));  // a tile gone from the grid
                alb *= 1.0 - 0.55*smoothstep(-0.05, 0.0, max(dp.x, dp.y));   // fixture frame
            }
        } else if (mat < 3.5){
            // ---- wallpaper, baseboard, and forty years of shoulders ----
            vec2 wuv;
            if (abs(n.x) > 0.5) wuv = vec2(p.z, p.y); else wuv = vec2(p.x, p.y);
            float ink = mix(0.24, wallpaper(wuv, foot), detW);
            alb = mix(cWall, cInk, ink);
            // the tooth of the vinyl. Too fine to resolve past a couple of
            // metres, but it is what stops a near wall reading as flat paint.
            alb *= 1.0 + 0.16*(vnoise(wuv*230.0) - 0.5)*mip(0.011, foot)
                       + 0.09*(vnoise(wuv*61.0) - 0.5)*mip(0.045, foot);
            float sm = abs(fract(wuv.x/0.53 + 0.5) - 0.5);
            alb *= 1.0 - 0.09*smoothstep(0.010, 0.0, sm)*det;   // where the rolls meet
            float gr = fbm(wuv*1.30 + 7.0);
            alb *= mix(1.0, 0.62 + 0.44*gr, smoothstep(0.75, 0.02, wuv.y)*uGrime*0.80*detM);
            alb = mix(alb, alb*vec3(0.58, 0.51, 0.39), smoothstep(0.48, 1.02, gr)*uDamp*0.70*detM);
            // Vinyl cove base. uTrim is its HEIGHT, not its opacity: mixing
            // toward the trim colour BY uTrim left 45 % of the wallpaper
            // showing through it at the default, and a baseboard you can see
            // the paper through is not a baseboard. Inside the band the wall
            // covering is replaced outright; uTrim 0 removes it entirely.
            float bh  = 0.160*uTrim;
            float bOn = step(0.012, bh);
            float cov = band(wuv.y - 0.5*bh, 0.5*bh, foot)*bOn;
            vec3 baseCol = cTrim*(0.80 + 0.26*gr)
                         * (0.90 + 0.16*clamp(wuv.y/max(bh, 1e-4), 0.0, 1.0));
            alb = mix(alb, baseCol, cov);
            alb *= 1.0 - 0.42*bOn*band(wuv.y - bh, 0.010, foot);
            alb *= 1.0 + 0.30*bOn*band(wuv.y - bh + 0.016, 0.006, foot);
            alb *= 1.0 - 0.35*uGrime*band(wuv.y, 0.022, foot);
        } else {
            // ---- a door leaf, and no baseboard running across it ----
            //
            // Everything below needs the leaf's OWN horizontal coordinate, so
            // the cell's hashes are recomputed here. maze2 works them out and
            // throws them away; redoing them once per pixel is far cheaper
            // than threading a door centre out through two hundred marcher
            // steps, and it is only the shading pass that pays.
            vec2 did = floor(p.xz/uCell);
            vec2 dcc = (did + 0.5)*uCell;
            vec4 dh  = hash42(did);
            vec4 dg  = hash42(did + 57.31);
            float dHor  = step(0.5, dh.y);
            vec2  dDir  = mix(vec2(0.0, 1.0), vec2(1.0, 0.0), dHor);
            float dRoom = step(fract(dh.y*31.7), uRooms);
            float dPil  = step(0.88, dg.x);
            float dLen  = mix((0.62 + 0.38*dh.z)*0.5*uCell, 0.26, dPil);
            float ctrL  = dot(dcc, dDir) + (dh.w - 0.5)*(uCell - 2.0*dLen);
            float u = dot(p.xz, dDir)
                    - mix(ctrL + (dg.w - 0.5)*2.0*max(dLen - 0.62, 0.0),
                          dot(dcc, dDir), dRoom);
            vec2 duv = vec2(u, p.y);

            alb = cTrim*0.84;
            alb *= 1.0 + 0.12*(vnoise(duv*vec2(80.0, 6.0)) - 0.5)*mip(0.013, foot);

            // two recessed panels between three rails, which is what a plain
            // office door is: stiles 0.11, panels 0.65 across
            float sdU = sdBox2(duv - vec2(0.0, 1.50), vec2(0.325, 0.345));
            float sdL = sdBox2(duv - vec2(0.0, 0.585), vec2(0.325, 0.325));
            alb *= 1.0 - 0.06*clamp(fill(sdU + 0.012, foot) + fill(sdL + 0.012, foot), 0.0, 1.0);
            alb *= 1.0 - 0.34*clamp(band(sdU, 0.009, foot) + band(sdL, 0.009, foot), 0.0, 1.0);

            // a lever handle on the leading stile, and its rose
            float hs = mix(-1.0, 1.0, step(0.5, fract(dg.z*23.1)));
            vec2 hp = duv - vec2(hs*0.345, 1.020);
            alb = mix(alb, vec3(0.30, 0.29, 0.26),
                      0.85*clamp(fill(sdBox2(hp + vec2(hs*0.030, 0.0), vec2(0.062, 0.013)), foot)
                               + fill(length(hp) - 0.032, foot), 0.0, 1.0));

            // kick plate along the bottom, brushed
            float kick = band(duv.y - 0.105, 0.105, foot)*band(u, 0.415, foot);
            alb = mix(alb, cTrim*(0.92 + 0.22*vnoise(duv*vec2(150.0, 7.0))), kick*0.75);

            // and the shadow line down its own edge, inside the frame
            alb *= 1.0 - 0.30*(1.0 - band(u, 0.405, foot));
            alb *= 0.90 + 0.20*fbm(duv*1.6 + 21.0);
        }

        float ao = mix(1.0, calcAO(p, n), uAO);
        vec3 kd; float kdist;
        float lit = lighting(p, n, kd, kdist);
        float sh = 1.0;
        if (uShadow > 0.01) sh = mix(1.0, shadowRay(p + n*0.06, kd, kdist - 0.20), uShadow);
        vec3 diff = cLite*lit*(0.52 + 0.48*sh)*(2.7 + 4.3*uBright);
        // no GI here, but a room this small and this bright is mostly bounce:
        // a hemispheric fill plus an explicit term for light coming back off
        // the carpet, which is what puts any brightness on the ceiling at all
        float dn = 0.5 - 0.5*n.y;
        float zdim = mix(1.0, 0.25 + 1.05*lightZone(p.xz), uZones);
        vec3 amb = ao*zdim*(mix(cWall, cLite, 0.5)*(0.075 + 0.095*uBright)
                          + cFloor*cLite*(0.24 + 0.31*uBright)*dn);
        col = alb*(diff*ao*0.85 + amb) + emisC;
        // A ray straight down a straight corridor can reach FAR without ever
        // hitting anything, and then it is pure fog - while the rays either
        // side of it still hit a wall at 50 m and keep 20 % of a lit surface.
        // That is a hard dark stripe a few pixels wide at the vanishing point,
        // and it is what a "split down the centre of the scene" turned out to
        // be. Forcing the fog to complete by the far plane makes the miss and
        // the far hit agree, so there is nothing to see at the join.
        float fg = 1.0 - exp(-uFog*tHit);
        // The fade has to COMPLETE before rays start missing, not at the far
        // plane itself: at 0.72*FAR a hit at 50 m still kept 15 % of its
        // surface while the ray beside it, which reached 54 and missed, kept
        // none - and that 15 % was the stripe.
        col = mix(col, fogCol, max(fg, smoothstep(FAR*0.55, FAR*0.92, tHit)));
    }

    // ---- in-scattering from the panels: the haze around every fixture ----
    if (uHaze > 0.005){
        float LG = gLG;
        float tm = min(tHit, FAR);
        float jt = hash12(fragCoord + floor(T*30.0));
        float glow = 0.0;
        for (int i = 0; i < 18; i++){
            // Sample positions depend ONLY on i, not on where the ray landed.
            //
            // Spacing them tm/18 apart is the obvious thing and it is wrong:
            // it ties every sample's position to tHit, so the whole integral
            // lurches whenever tHit does. Down a straight corridor tHit jumps
            // several metres between the ray that reaches the far plane and
            // its neighbour that clips a wall at fifty, and the result is a
            // hard vertical stripe a few pixels wide at the vanishing point.
            //
            // Quadratic in i: fixed in world space, dense near the camera
            // where the halo needs resolution, coarse out where the fog has
            // already eaten it.
            float u  = (float(i) + jt)/18.0;
            float s  = FAR*u*u;
            if (s > tm) break;
            float ds = FAR*2.0*u/18.0;              // this sample's interval
            ds *= min(1.0, (tm - s)/max(ds, 1e-3)); // taper the truncated one
            vec3 q = ro + rd*s;

            // Nearest panel by FOLDING the lattice, and the local DENSITY of
            // working tubes rather than whether one particular fixture is lit.
            //
            // Looking the cell up with floor() and calling panelOn() on it is
            // the obvious way and it is what put a hard vertical line down the
            // middle of the frame: both the lookup and the on/off draw are
            // piecewise constant, so the glow steps at every lattice plane -
            // and the camera stands ON the plane x = 0, which projects to a
            // straight full-height line through the vanishing point. Every
            // other lattice plane is oblique and its step goes unnoticed.
            //
            // Folded, both terms are continuous everywhere. It is also cheaper:
            // eighteen hash42 calls a pixel go away.
            vec2 f  = mod(q.xz - gLOF, LG) - 0.5*LG;
            vec2 d2 = max(abs(f) - PANEL_E, 0.0);
            float dl = length(vec3(d2.x, uHeight - 0.03 - q.y, d2.y));
            float on = uLightOn*mix(1.0, 0.05 + 1.90*lightZone(q.xz), uZones);
            glow += on*ds*exp(-uFog*2.0*s)/(1.0 + 1.6*dl*dl);
        }
        col += cLite*glow*uHaze*0.34;
    }

    // ---- figures standing about the place ----
    //
    // One candidate per segment of corridor, six segments looked at, and the
    // segment SHORTENS as the count goes up - so the slider scatters them more
    // densely rather than stacking them in one spot. About three in five slots
    // are actually occupied.
    float gCount = floor(uGhosts + 0.5);
    float seg = 26.0/max(gCount, 1.0);
    float gb = floor(T*11.0);
    for (int gi = 0; gi < 6; gi++){
        float zi = floor(z/seg) + float(gi);
        float ez = (zi + 0.5)*seg + 0.42*seg*(hash11(zi*1.7 + 0.3) - 0.5);
        float dA = ez - z;
        if (dA > 40.0) break;                  // segments only get further away

        float entA = uFigure*step(hash11(zi*11.3 + 5.1), 0.6)
                   * smoothstep(34.0, 26.0, dA)*smoothstep(4.2, 6.8, dA);
        if (entA <= 0.004) continue;

        // Anywhere across the lane and out into the rooms, not lined up down
        // the middle of the corridor. The draw can land inside a partition, so
        // it is tested against the maze and pulled back into the open lane if
        // it does - one extra maze2(), and only for a slot that is visible.
        float lat = 7.0*(hash11(zi*3.9 + 2.7) - 0.5);
        vec2 fxz = vec2(pathX(ez) + lat, ez);
        if (maze2(fxz).x < 0.45) fxz.x = pathX(ez) + 1.4*(hash11(zi*5.1 + 9.4) - 0.5);

        // Everything glitchy below updates on an ~11 Hz BEAT rather than
        // continuously. That is the whole trick: a smoothly animated wobble
        // reads as motion, and a value that holds for ninety milliseconds and
        // then jumps reads as a fault in the recording.
        vec3 entP = vec3(fxz.x + uGlitch*0.09*(hash11(gb*2.3 + zi) - 0.5), 0.0, fxz.y);
        vec3 oc = ro - (entP + vec3(0.0, 0.95, 0.0));
        float fb = dot(oc, rd);
        // Three copies trail up to 3.2 offsets behind, and the bounding
        // sphere has to contain them or they get sliced off at its edge.
        // Travel gives real parallax - which means a figure dead ahead has
        // none at all and shows no trail. A lateral judder on the beat, along
        // the camera's own right vector, smears it wherever it happens to be.
        vec3 tOff = (trav*(0.17 + 0.34*uTrails)
                   + rt*(0.05 + 0.17*uTrails)*(hash11(gb*3.7 + zi) - 0.5)*2.0)
                  * (0.45 + 0.18*uSpeed);
        float bR = 1.30;
        float fh = fb*fb - dot(oc, oc) + bR*bR;
        if (fh <= 0.0) continue;

        fh = sqrt(fh);
        // The trail copies are the figure sampled at q + tOff, and tOff runs
        // away down the ray - so they are reached from positions BEFORE the
        // figure, and the interval has to grow at the NEAR end.
        //
        // It grows either way, which is the thing to watch: the sample count
        // has to grow with it or the step outruns an 8 cm arm and the limbs are
        // stepped straight over. That, and not any compositing, is what made
        // the arms disappear the moment trails were switched on.
        float t0 = max(-fb - fh - 3.4*max(dot(tOff, rd), 0.0), 0.0);
        float t1 = min(-fb + fh, tHit);        // the walls occlude it for free
        if (t1 <= t0) continue;

        // The tear has to be constant ALONG a ray.
        //
        // Computed per sample, each of the twenty-two samples passes through a
        // differently-displaced body, the integral decorrelates, and whatever
        // is thin integrates to nothing - which is where the arms went. It is
        // a displacement of the FIGURE, not of the medium, so it belongs out
        // here. Neighbouring rays still get different draws, which is what
        // makes it a tear rather than a wobble, and it now costs one set of
        // hashes instead of twenty-two.
        vec3  qm = ro + rd*(0.5*(t0 + t1)) - entP;      // mid-chord, figure space
        float sl = floor(qm.y*30.0);
        float pick = step(0.72, hash11(sl*3.1 + gb*0.37));
        float tx = 0.115*uGlitch*(hash11(sl*1.7 + gb) - 0.5)*pick;
        float bx = floor((qm.x + tx)*2.4);
        tx += 0.042*uGlitch*(hash11(sl*5.3 + bx*2.9 + gb) - 0.5)*pick;
        float ty = 0.020*uGlitch*(hash11(sl*5.9 + gb*1.7) - 0.5)*pick;
        float rip = step(0.94, hash11(gb*0.19 + 7.7 + zi));
        float bo1 = 0.010*uGlitch*(vnoise(vec2(qm.y*6.0, T*0.9)) - 0.5);
        float bo2 = 0.010*uGlitch*(vnoise(vec2(qm.y*5.0, T*0.7 + 11.0)) - 0.5);

        // a scan running down it, and holes punched through the odd slice
        float scanline = 1.0 - 0.22*uGlitch*(0.5 + 0.5*sin(qm.y*210.0 - T*7.0));
        scanline *= 1.0 - 0.55*uGlitch*step(0.972, hash11(sl*11.3 + gb*0.53));

        float ds = (t1 - t0)/32.0;
        float dens = 0.0, densB = 0.0;
        vec3  densT = vec3(0.0);        // three afterimages, oldest in .z
        for (int i = 0; i < 32; i++){
            float sT = t0 + (float(i) + 0.5)*ds;
            if (sT > t1) break;
            vec3 q = ro + rd*sT - entP;

            // the same displacement for every sample on this ray, plus the
            // shear, which only takes everything above the waist
            q.x += tx + bo1 + 0.16*uGlitch*rip*(hash11(sl*0.7 + gb) - 0.5)
                                              *step(0.0, q.y - 1.05);
            q.y += ty;
            q.z += bo2;

            // the legs dissolve toward the floor rather than ending
            float fade = smoothstep(-0.100, 0.055, q.y)*scanline*ds;
            dens  += smoothstep(0.040, -0.018, figure(q))*fade;
            // a second copy, offset, driving the blue channel: an actual
            // spatial separation instead of a coloured fringe
            densB += smoothstep(0.040, -0.018, figure(q + vec3(0.055*uGlitch, 0.0, 0.0)))*fade;
            // Three afterimages dragged back along the way you came.
            //
            // Their bands are TIGHTER than the figure's, not wider. Widening
            // them made each copy fatter than the body, so they filled the
            // armpit gaps and the space outside the arms - the negative space
            // that is the only reason a limb reads as a limb - and the whole
            // silhouette closed up into a column. Softness comes from the
            // alpha, not from thickening the shape.
            densT.x += smoothstep(0.040, -0.018, figure(q + tOff       ))*fade;
            densT.y += smoothstep(0.044, -0.016, figure(q + tOff*2.05  ))*fade;
            densT.z += smoothstep(0.048, -0.014, figure(q + tOff*3.20  ))*fade;
        }
        // This is composited BEFORE the tone map, where the carpet is already
        // sitting near 1.9 - a figure painted at 0.8 came out darker than the
        // floor and simply vanished. It has to be genuinely emissive to read as
        // pale, and the alpha is capped so the room still shows through it.
        float drop = 1.0 - 0.60*uGlitch*step(0.88, hash11(gb*0.37 + 4.1 + zi));
        float kA = entA*exp(-uFog*0.5*(t0 + t1))*0.88*drop;
        float aA = (1.0 - exp(-dens *7.4))*kA;
        float aB = (1.0 - exp(-densB*7.4))*kA;
        vec3  av = vec3(aA, mix(aA, aB, 0.5), aB);

        // The afterimages go UNDER the figure, oldest first, each dimmer than
        // the one in front of it.
        vec3 aT = (1.0 - exp(-densT*vec3(5.0, 4.4, 3.8)))*kA*uTrails
                * vec3(0.44, 0.27, 0.15);
        vec3 tc = mix(vec3(0.72, 0.78, 0.86), cLite, 0.25)*(1.5 + 1.0*uBright);
        col = mix(col, tc*0.82, aT.z);
        col = mix(col, tc*0.91, aT.y);
        col = mix(col, tc,      aT.x);
        // The silhouette is grainy in its own right, not merely sitting in a
        // grainy frame. Mean 1.0, so it costs no brightness.
        av *= 0.70 + 0.60*hash12(fragCoord*1.63 + floor(T*24.0)*vec2(7.3, 3.1));
        vec3 gc = mix(vec3(0.92, 0.94, 0.88), cLite, 0.30);
        col = mix(col, gc*(2.6 + 1.8*uBright), av);
        col += gc*av*0.30;
    }

    // ---- and then it is put on tape ----
    vec2 sc = fragCoord/R;
    col = col/(1.0 + col*0.85);
    col *= 1.0 - uScan*vhs*0.22*(0.5 + 0.5*sin(fragCoord.y*PI));

    float nz = hash12(fragCoord + floor(T*24.0)*vec2(13.7, 91.3));
    float hb = smoothstep(0.028, 0.0, sc.y);                    // head-switching noise
    col = mix(col, vec3(nz)*0.55 + col*0.40, hb*vhs*0.70);

    float drow = hash11(floor(sc.y*180.0) + floor(T*12.0)*7.3);
    float dseg = step(0.996, hash11(floor(sc.x*40.0)*3.1 + floor(sc.y*180.0)*11.7 + floor(T*12.0)*29.0));
    col += vhs*0.20*dseg*step(0.988, drow)*vec3(1.0, 0.98, 0.92); // dropouts

    col += (nz - 0.5)*uGrain*0.14;
    float r2 = dot(uv, uv);
    col.r *= 1.0 - vhs*0.10*r2;
    col.b *= 1.0 - vhs*0.16*r2;
    col *= 1.0 - uVignette*1.30*dot(sc - 0.5, sc - 0.5);
    float lum = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(lum), col, 1.46 - vhs*0.24);   // the tone map greys it; put it back
    col = col*(1.0 - vhs*0.06) + vhs*0.022;                     // lifted black

    // Half an LSB of noise before the 8-bit write. A dark room is mostly smooth
    // low-amplitude gradients, which is exactly where quantisation shows up as
    // contour bands across a wall - and the grain slider cannot be relied on
    // for this, because a viewer is entitled to set it to zero.
    col += (hash12(fragCoord + 0.5) - 0.5)/255.0;

    gl_FragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
