/*{
  "DESCRIPTION": "Digital rain in log-polar space: glyph columns spiral inward toward an event horizon, shearing into light as they are crushed past the resolution limit. shear blends rigid shells (crisp glyphs) into continuous frame drag (smeared streaks).",
  "CREDIT": "Claude, from ChatGPT's Procedural Matrix Digital Rain",
  "CATEGORIES": [
    "Generator"
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
      "NAME": "density",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.5,
      "MAX": 3.0
    },
    {
      "NAME": "shear",
      "LABEL": "shear (0 = crisp glyphs, 1 = smeared streaks)",
      "TYPE": "float",
      "DEFAULT": 0.35,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "swirl",
      "TYPE": "float",
      "DEFAULT": 0.6,
      "MIN": 0.0,
      "MAX": 4.0
    },
    {
      "NAME": "horizon",
      "TYPE": "float",
      "DEFAULT": 0.13,
      "MIN": 0.02,
      "MAX": 0.4
    },
    {
      "NAME": "beaming",
      "LABEL": "doppler beaming",
      "TYPE": "float",
      "DEFAULT": 0.45,
      "MIN": 0.0,
      "MAX": 1.0
    },
    {
      "NAME": "stars",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.0,
      "MAX": 2.0
    },
    {
      "NAME": "glow",
      "TYPE": "float",
      "DEFAULT": 1.2,
      "MIN": 0.0,
      "MAX": 3.0
    },
    {
      "NAME": "brightness",
      "TYPE": "float",
      "DEFAULT": 1.0,
      "MIN": 0.2,
      "MAX": 3.0
    }
  ],
  "ISFVSN": "2",
  "MODV_CATEGORY": "2D Scenes"
}*/
// License: CC0-1.0 (public domain). Source: https://glslop.com/shader/kxldgnkx


#define TAU 6.2831853

float hash(float n)
{
    return fract(sin(n)*43758.5453123);
}

// 5x7 procedural glyph
float glyph(vec2 uv,float id)
{
    uv*=vec2(5.0,7.0);

    vec2 cell=floor(uv);

    float n=hash(dot(cell,vec2(17.0,31.0))+id*83.0);

    return step(0.55,n);
}

// sparse background stars, sampled on a plane behind the hole
float starField(vec2 sp)
{
    vec2 c=floor(sp);

    vec2 f=fract(sp)-0.5;

    float h=hash(dot(c,vec2(41.0,289.0)));

    // only a few cells hold a star
    float present=step(0.972,h);

    float twinkle=.7+.3*sin(TIME*2.0+h*90.0);

    return present*exp(-dot(f,f)*55.0)*twinkle;
}

void main()
{
    vec2 p=isf_FragNormCoord-0.5;

    p.x*=RENDERSIZE.x/RENDERSIZE.y;

    p*=2.0;

    float r=length(p);

    // untwisted angle — stars and beaming key off this so they don't spin
    // with the rain
    float a0=atan(p.y,p.x);

    //------------------------------------
    // log-polar grid: spokes x shells.
    // cols must stay integral so floor/fract wrap cleanly at the atan seam.
    //------------------------------------

    float cols=floor(110.0*density);

    float rows=64.0;

    float rowScale=14.0;

    // log(r) -> -inf at the center, so negating makes v grow inward
    float v=-log(max(r,1e-3))*rowScale;

    float cellV=floor(v);

    //------------------------------------
    // frame dragging — the twist tightens as you fall in.
    //
    // Keyed to r, the twist shears each glyph across several columns over its
    // own height: fluid, but the text dissolves. Keyed to the shell, rotation
    // is rigid (cellV has no angular dependence) so glyphs stay square and the
    // drag steps between shells instead. shear blends the two.
    //------------------------------------

    float shellR=exp(-(cellV+0.5)/rowScale);

    float twist=mix(swirl/max(shellR,0.05),swirl/max(r,0.05),shear);

    float a=a0+twist+TIME*0.05;

    float u=(a/TAU)*cols;

    float column=mod(floor(u),cols);

    vec2 local=fract(vec2(u,v));

    //------------------------------------
    // randomize each spoke
    //------------------------------------

    float rnd=hash(column);

    float speedMul=mix(.5,2.2,rnd);

    float t=TIME*speed*speedMul;

    //------------------------------------
    // heads fall inward; trail streams outward behind them
    //------------------------------------

    float headV=mod(t*8.0+rnd*100.0,rows);

    float trail=mod(headV-cellV+rows,rows);

    //------------------------------------
    // character
    //------------------------------------

    float charID=floor(mod(cellV+floor(t*7.0)+rnd*100.0,96.0));

    float pix=glyph(local,charID);

    //------------------------------------
    // a cell's arc length shrinks with r. Past the sampling limit, dissolve
    // the glyph into a solid streak rather than letting it alias to noise.
    //------------------------------------

    float cellPx=r*(TAU/cols)*RENDERSIZE.y*0.5;

    float detail=smoothstep(1.5,5.0,cellPx);

    pix=mix(1.0,pix,detail);

    //------------------------------------
    // fade trail
    //------------------------------------

    float fade=exp(-trail*0.10);

    float leader=smoothstep(1.5,0.0,trail);

    float flicker=.6+.4*sin(TIME*20.0+column*3.1+cellV);

    //------------------------------------
    // blueshift toward the horizon
    //------------------------------------

    vec3 far=vec3(0.05,1.0,0.15);

    vec3 near=vec3(0.75,1.0,0.95);

    vec3 tint=mix(near,far,smoothstep(horizon,0.75,r));

    vec3 color=tint*pix*fade*flicker*brightness;

    color+=vec3(1.0)*leader*pix;

    color+=tint*leader*glow;

    //------------------------------------
    // doppler beaming — the limb rotating toward us runs hot
    //------------------------------------

    color*=1.0+beaming*cos(a0-2.2)*smoothstep(1.1,horizon,r);

    //------------------------------------
    // lensed starfield: deflection grows as light passes nearer the hole,
    // smearing the field into a ring rather than a flat backdrop
    //------------------------------------

    float rl=r+0.05/max(r,0.13);

    float s=starField(vec2(cos(a0),sin(a0))*rl*17.0);

    color+=vec3(0.55,0.75,1.0)*s*stars*(1.0-smoothstep(0.2,0.9,r)*0.5);

    //------------------------------------
    // accretion bloom hugging the horizon
    //------------------------------------

    color+=near*exp(-abs(r-horizon*1.6)*9.0)*0.18*glow;

    //------------------------------------
    // photon ring + the horizon itself
    //------------------------------------

    float ring=exp(-abs(r-horizon*1.05)*70.0);

    color+=vec3(0.7,1.0,0.85)*ring*1.6*glow;

    color*=smoothstep(horizon,horizon*1.12,r);

    //------------------------------------
    // vignette the outer field
    //------------------------------------

    color*=1.0-smoothstep(0.75,1.45,r);

    gl_FragColor=vec4(color,1.0);
}
