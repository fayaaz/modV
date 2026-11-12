//! The 31 Point10 effects — port of
//! `src/renderer/src/application/renderers/shader/modules/Point10Effects.js`.
//!
//! Every effect is registered with the `Point10 ` name prefix,
//! `author = "Point10"`, `version = "1.0.0"`, `previewWithOutput = true`,
//! and `category = None` (type-based Gallery fallback), exactly as the
//! `effect()` helper in `Point10Effects.js:1-11` does. None of them declare
//! props; they consume only the built-in uniforms.
//!
//! The four effects with `audioFeatures: ["bass", "mid", "treble"]`
//! (Contrast, Color Distort, RGB Shift, Digital Glitch) set
//! [`ShaderModule::audio`] and `audio_features` likewise.
//!
//! GL line references below point at `Point10Effects.js` at commit
//! 8a512b6e (anchors, not contracts).
//!
//! # Chunks
//!
//! Two shared GLSL chunks are spliced into the effects that use them,
//! matching `${noise}` / `${bayer}` interpolation in the source:
//!
//! * [`P10_NOISE_WGSL`] — `p10Hash`, `p10Hash2`, `p10Noise`, `p10Fbm`.
//!   Used by Fuzzy Pixels, Fractured, Bathroom Window, Color Distort,
//!   Old Movie, Digital Glitch. `p10Hash2` is unused by every effect but
//!   kept: it is part of the shared chunk's contract.
//! * [`P10_BAYER_WGSL`] — `p10Bayer2/4/8`. Used by Bayer 8 Matrix
//!   Dithering BW and Bayer Matrix Dithering Color.
//! * `glsl_mod_f` / `glsl_mod_v2` (from [`crate::shader::uniforms`]) —
//!   GLSL `mod` for Film Grain, Crosshatching, Blocks and 3d Blocks.
//!
//! # fragCoord
//!
//! `fragCoord` passed to `mainImage` is the wgpu `@builtin(position)`
//! (top-left origin), **not** GL's bottom-left `gl_FragCoord`. Effects that
//! pattern on `fragCoord` (Crt, Crosshatching, Bayer dithers) therefore see
//! flipped rows — the dither matrix effectively mirrors. No golden-image
//! comparison exists yet to measure that; it is noted for WS-15.
//!
//! # textureSampleLevel sites
//!
//! Sampling inside non-uniform control flow requires an explicit LOD
//! (naga's uniformity validator). The affected sites are `Crt`'s
//! `crtFetch` (out-of-bounds early return) and Fisheye's `else` branch —
//! both use `textureSampleLevel(.., 0.0)`. Explicit LOD 0 matches the
//! legacy textures, which declare no mipmaps.

#![forbid(unsafe_code)]

use crate::shader::default_shader::main_image_fragment;
use crate::shader::modules::ShaderModule;
use crate::shader::uniforms::{GLSL_MOD_F_WGSL, GLSL_MOD_V2_WGSL};

// Port of Point10Effects.js:13-46 (const noise).
const P10_NOISE_WGSL: &str = r#"fn p10Hash(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn p10Hash2(p: vec2<f32>) -> vec2<f32> {
  return fract(sin(vec2<f32>(
    dot(p, vec2<f32>(127.1, 311.7)),
    dot(p, vec2<f32>(269.5, 183.3))
  )) * 43758.5453);
}

fn p10Noise(p: vec2<f32>) -> f32 {
  let i = floor(p);
  var f = fract(p);
  f = f * f * (vec2<f32>(3.0) - 2.0 * f);
  let a = p10Hash(i);
  let b = p10Hash(i + vec2<f32>(1.0, 0.0));
  let c = p10Hash(i + vec2<f32>(0.0, 1.0));
  let d = p10Hash(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn p10Fbm(p: vec2<f32>) -> f32 {
  var sum = 0.0;
  var amp = 0.5;
  var pp = p;
  // GLSL `for (int i = 0; i < 4; i++)` — WGSL loops over var i: i32.
  for (var i = 0; i < 4; i++) {
    sum += p10Noise(pp) * amp;
    pp *= 2.13;
    amp *= 0.5;
  }
  return sum;
}

"#;

// Port of Point10Effects.js:48-61 (const bayer). The GLSL shadowed
// parameter `p = floor(p)` becomes a renamed `pf` local.
const P10_BAYER_WGSL: &str = r#"fn p10Bayer2(p: vec2<f32>) -> f32 {
  let pf = floor(p);
  return fract(pf.x / 2.0 + pf.y * pf.y * 0.75);
}

fn p10Bayer4(p: vec2<f32>) -> f32 {
  return p10Bayer2(0.5 * p) * 0.25 + p10Bayer2(p);
}

fn p10Bayer8(p: vec2<f32>) -> f32 {
  return p10Bayer4(0.5 * p) * 0.25 + p10Bayer2(p);
}

"#;

/// Spliced chunks an effect needs, in insertion order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum P10Chunk {
    /// `p10Hash`/`p10Hash2`/`p10Noise`/`p10Fbm`.
    Noise,
    /// `p10Bayer2/4/8`.
    Bayer,
    /// Scalar GLSL `mod`.
    ModF,
    /// Vec2 GLSL `mod`.
    ModV2,
}

struct P10Effect {
    /// Registration name — the `Point10 ` prefix is part of the name, as in
    /// the JS `effect()` helper.
    name: &'static str,
    /// WGSL fragment body (the `mainImage` function and any locals/consts).
    body: &'static str,
    /// Chunks spliced before `body`.
    chunks: &'static [P10Chunk],
    /// `true` when the JS meta passes `audio = true`.
    audio: bool,
}

const P10_EFFECTS: &[P10Effect] = &[
    // Point10Effects.js:64-72 — Gray
    P10Effect {
        name: "Point10 Gray",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  c = vec4<f32>(vec3<f32>(dot(vec3<f32>(0.222, 0.707, 0.071), c.rgb)), c.a);
  return c;
}
"#,
    },
    // Point10Effects.js:74-82 — Invert
    P10Effect {
        name: "Point10 Invert",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  c = vec4<f32>(vec3<f32>(1.0) - c.rgb, c.a);
  return c;
}
"#,
    },
    // Point10Effects.js:84-96 — Posterization
    P10Effect {
        name: "Point10 Posterization",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let gamma = 0.6;
  let numColors = 4.0;
  var c2 = pow(c.rgb, vec3<f32>(gamma));
  c2 = floor(c2 * numColors) / numColors;
  c = vec4<f32>(pow(c2, vec3<f32>(1.0 / gamma)), c.a);
  return c;
}
"#,
    },
    // Point10Effects.js:98-118 — Film Grain
    P10Effect {
        name: "Point10 Film Grain",
        chunks: &[P10Chunk::ModF],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let p = vUv;
  let textureScreen = textureSample(u_modVCanvas, modvGroupSampler, p);
  let fNintensity = 0.35;
  let fSintensity = 0.35;
  let fScount = 4096.0;
  let x = p.x * p.y * u.iTime * 1000.0;
  let dx = glsl_mod_f(glsl_mod_f(x, 13.0) * glsl_mod_f(x, 123.0), 0.01);
  var result = textureScreen.rgb + textureScreen.rgb * clamp(0.1 + dx * 100.0, 0.0, 1.0);
  let sc = vec2<f32>(sin(p.y * fScount), cos(p.y * fScount));
  result += textureScreen.rgb * sc.xyx * fSintensity;
  result = textureScreen.rgb + clamp(fNintensity, 0.0, 1.0) * (result - textureScreen.rgb);
  result = vec3<f32>(dot(vec3<f32>(0.222, 0.707, 0.071), result));
  return vec4<f32>(result, 1.0);
}
"#,
    },
    // Point10Effects.js:120-130 — BlackAndWhite
    P10Effect {
        name: "Point10 BlackAndWhite",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let t = (c.r + c.g + c.b) / 3.0;
  c = vec4<f32>(vec3<f32>(1.0), c.a);
  if (t < 0.1 || t > 0.9) {
    c = vec4<f32>(vec3<f32>(0.0), c.a);
  }
  return c;
}
"#,
    },
    // Point10Effects.js:132-143 — Gray Stripes
    P10Effect {
        name: "Point10 Gray Stripes",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let aspect = u.iResolution.x / max(u.iResolution.y, 1.0);
  if (fract(vUv.x * aspect * 5.0) < 0.5) {
    c = vec4<f32>(vec3<f32>(dot(vec3<f32>(0.222, 0.707, 0.071), c.rgb)), c.a);
  }
  return c;
}
"#,
    },
    // Point10Effects.js:145-154 — Invert Gray
    P10Effect {
        name: "Point10 Invert Gray",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  c = vec4<f32>(vec3<f32>(1.0) - c.rgb, c.a);
  c = vec4<f32>(vec3<f32>(dot(vec3<f32>(0.222, 0.707, 0.071), c.rgb)), c.a);
  return c;
}
"#,
    },
    // Point10Effects.js:156-177 — Crosshatching
    P10Effect {
        name: "Point10 Crosshatching",
        chunks: &[P10Chunk::ModF],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let l = length(c.rgb);
  c = vec4<f32>(vec3<f32>(1.0), c.a);
  if (l < 1.0 && glsl_mod_f(fragCoord.x + fragCoord.y, 10.0) < 0.5) {
    c = vec4<f32>(vec3<f32>(0.0), c.a);
  }
  if (l < 0.7 && glsl_mod_f(fragCoord.x - fragCoord.y, 10.0) < 0.5) {
    c = vec4<f32>(vec3<f32>(0.0), c.a);
  }
  if (l < 0.5 && glsl_mod_f(fragCoord.x + fragCoord.y - 5.0, 10.0) < 0.5) {
    c = vec4<f32>(vec3<f32>(0.0), c.a);
  }
  if (l < 0.3 && glsl_mod_f(fragCoord.x - fragCoord.y - 5.0, 10.0) < 0.5) {
    c = vec4<f32>(vec3<f32>(0.0), c.a);
  }
  return c;
}
"#,
    },
    // Point10Effects.js:179-191 — Blocks
    P10Effect {
        name: "Point10 Blocks",
        chunks: &[P10Chunk::ModV2],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let numTiles = 128.0;
  let size = vec2<f32>(1.0) / vec2<f32>(numTiles * (u.iResolution.x / max(u.iResolution.y, 1.0)), numTiles);
  let base = vUv - glsl_mod_v2(vUv, size);
  return textureSample(u_modVCanvas, modvGroupSampler, base + size * 0.5);
}
"#,
    },
    // Point10Effects.js:193-219 — 3d Blocks
    P10Effect {
        name: "Point10 3d Blocks",
        chunks: &[P10Chunk::ModV2],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let numTiles = 70.0;
  let threshold = 0.15;
  let edgeColor = vec3<f32>(0.7);
  let size = 1.0 / numTiles;
  let base = vUv - glsl_mod_v2(vUv, vec2<f32>(size));
  let center = base + vec2<f32>(size * 0.5);
  let st = (vUv - base) / size;
  var c1 = vec4<f32>(0.0);
  var c2 = vec4<f32>(0.0);
  let inverseOffset = vec4<f32>(1.0 - edgeColor, 1.0);
  if (st.x > st.y) {
    c1 = inverseOffset;
  }
  if (st.x > 1.0 - threshold) {
    c2 = c1;
  }
  if (st.y > 1.0 - threshold) {
    c2 = c1;
  }
  let bottom = c2;
  c1 = vec4<f32>(0.0);
  c2 = vec4<f32>(0.0);
  if (st.x > st.y) {
    c1 = inverseOffset;
  }
  if (st.x < threshold) {
    c2 = c1;
  }
  if (st.y < threshold) {
    c2 = c1;
  }
  let top = c2;
  return textureSample(u_modVCanvas, modvGroupSampler, center) + top - bottom;
}
"#,
    },
    // Point10Effects.js:221-231 — Bayer 8 Matrix Dithering BW
    P10Effect {
        name: "Point10 Bayer 8 Matrix Dithering BW",
        chunks: &[P10Chunk::Bayer],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let grayscale = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
  let limit = p10Bayer8(fragCoord);
  // GLSL `grayscale < limit ? 0.0 : 1.0` → WGSL select(false, true, cond).
  let value = select(1.0, 0.0, grayscale < limit);
  return vec4<f32>(value, value, value, 1.0);
}
"#,
    },
    // Point10Effects.js:233-246 — Bayer Matrix Dithering Color
    P10Effect {
        name: "Point10 Bayer Matrix Dithering Color",
        chunks: &[P10Chunk::Bayer],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let rgb = textureSample(u_modVCanvas, modvGroupSampler, vUv).rgb;
  let limit = p10Bayer4(fragCoord);
  // GLSL `rgb.r < limit ? 0.0 : 1.0` → WGSL select(false, true, cond).
  let color = vec3<f32>(
    select(1.0, 0.0, rgb.r < limit),
    select(1.0, 0.0, rgb.g < limit),
    select(1.0, 0.0, rgb.b < limit)
  );
  return vec4<f32>(color, 1.0);
}
"#,
    },
    // Point10Effects.js:248-255 — Fuzzy Pixels
    P10Effect {
        name: "Point10 Fuzzy Pixels",
        chunks: &[P10Chunk::Noise],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let rnd = vec2<f32>(p10Hash(vUv), p10Hash(vUv + 0.1));
  return textureSample(u_modVCanvas, modvGroupSampler, vUv + rnd * 0.01);
}
"#,
    },
    // Point10Effects.js:257-268 — Fractured
    P10Effect {
        name: "Point10 Fractured",
        chunks: &[P10Chunk::Noise],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let p = vUv * 10.0;
  let cell = floor(p);
  let local = fract(p);
  let point = vec2<f32>(p10Hash(cell), p10Hash(cell + 17.0));
  let delta = normalize(local - point + 0.0001);
  return textureSample(u_modVCanvas, modvGroupSampler, fract(vUv + delta * 0.005));
}
"#,
    },
    // Point10Effects.js:270-284 — Bathroom Window
    P10Effect {
        name: "Point10 Bathroom Window",
        chunks: &[P10Chunk::Noise],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let texel = 1.0 / u.iResolution.xy;
  // `center` is unused — carried over from the GLSL for parity.
  let center = p10Fbm(vUv * 5.0);
  let ox = p10Fbm((vUv - vec2<f32>(texel.x, 0.0)) * 5.0) -
    p10Fbm((vUv + vec2<f32>(texel.x, 0.0)) * 5.0);
  let oy = p10Fbm((vUv - vec2<f32>(0.0, texel.y)) * 5.0) -
    p10Fbm((vUv + vec2<f32>(0.0, texel.y)) * 5.0);
  let offset = vec2<f32>(ox, oy);
  let c = textureSample(u_modVCanvas, modvGroupSampler, vUv + offset * 0.25);
  return vec4<f32>(c.rgb + offset.x * abs(offset.y) * 10.0, 1.0);
}
"#,
    },
    // Point10Effects.js:286-295 — Contrast (audioFeatures: bass, mid, treble)
    P10Effect {
        name: "Point10 Contrast",
        chunks: &[],
        audio: true,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  c = vec4<f32>(c.rgb - vec3<f32>(u.u_bass) * (c.rgb - 1.0) * c.rgb * (c.rgb - 0.5), c.a);
  return c;
}
"#,
    },
    // Point10Effects.js:297-312 — Color Distort (audioFeatures: bass, mid, treble)
    P10Effect {
        name: "Point10 Color Distort",
        chunks: &[P10Chunk::Noise],
        audio: true,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let scale = 0.05 / (u.iResolution.x / max(u.iResolution.y, 1.0));
  let t = u.iTime * 0.2;
  let strength1 = min(1.0, u.u_bass * 5.0);
  let strength2 = min(1.0, u.u_mid * 5.0);
  var c = vec3<f32>(0.0);
  c.r = textureSample(u_modVCanvas, modvGroupSampler, vec2<f32>(vUv.x + p10Noise(vec2<f32>(t, 0.0)) * 0.5 * strength1 * scale, vUv.y)).r;
  c.g = textureSample(u_modVCanvas, modvGroupSampler, vUv).g;
  c.b = textureSample(u_modVCanvas, modvGroupSampler, vec2<f32>(vUv.x + p10Noise(vec2<f32>(t, 2.232)) * 0.5 * strength2 * scale, vUv.y)).b;
  return vec4<f32>(c, 1.0);
}
"#,
    },
    // Point10Effects.js:314-395 — Crt
    P10Effect {
        name: "Point10 Crt",
        chunks: &[],
        audio: false,
        body: r#"const CRT_HARD_SCAN: f32 = -8.0;
const CRT_HARD_PIX: f32 = -3.0;
const CRT_MASK_DARK: f32 = 0.5;
const CRT_MASK_LIGHT: f32 = 1.5;

fn crtFetch(pos: vec2<f32>, off: vec2<f32>) -> vec3<f32> {
  let res = u.iResolution.xy / 6.0;
  let p = floor(pos * res + off) / res;
  if (max(abs(p.x - 0.5), abs(p.y - 0.5)) > 0.5) {
    return vec3<f32>(0.0);
  }
  // Out-of-bounds early return is non-uniform control flow; explicit LOD 0.
  return textureSampleLevel(u_modVCanvas, modvGroupSampler, p, 0.0).rgb;
}

fn crtDist(pos: vec2<f32>) -> vec2<f32> {
  let res = u.iResolution.xy / 6.0;
  let p = pos * res;
  return -((p - floor(p)) - vec2<f32>(0.5));
}

fn crtGaus(pos: f32, scale: f32) -> f32 {
  return exp2(scale * pos * pos);
}

fn crtHorz3(pos: vec2<f32>, off: f32) -> vec3<f32> {
  let b = crtFetch(pos, vec2<f32>(-1.0, off));
  let c = crtFetch(pos, vec2<f32>(0.0, off));
  let d = crtFetch(pos, vec2<f32>(1.0, off));
  let dpos = crtDist(pos).x;
  let wb = crtGaus(dpos - 1.0, CRT_HARD_PIX);
  let wc = crtGaus(dpos, CRT_HARD_PIX);
  let wd = crtGaus(dpos + 1.0, CRT_HARD_PIX);
  return (b * wb + c * wc + d * wd) / (wb + wc + wd);
}

fn crtHorz5(pos: vec2<f32>, off: f32) -> vec3<f32> {
  let a = crtFetch(pos, vec2<f32>(-2.0, off));
  let b = crtFetch(pos, vec2<f32>(-1.0, off));
  let c = crtFetch(pos, vec2<f32>(0.0, off));
  let d = crtFetch(pos, vec2<f32>(1.0, off));
  let e = crtFetch(pos, vec2<f32>(2.0, off));
  let dpos = crtDist(pos).x;
  let wa = crtGaus(dpos - 2.0, CRT_HARD_PIX);
  let wb = crtGaus(dpos - 1.0, CRT_HARD_PIX);
  let wc = crtGaus(dpos, CRT_HARD_PIX);
  let wd = crtGaus(dpos + 1.0, CRT_HARD_PIX);
  let we = crtGaus(dpos + 2.0, CRT_HARD_PIX);
  return (a * wa + b * wb + c * wc + d * wd + e * we) / (wa + wb + wc + wd + we);
}

fn crtScan(pos: vec2<f32>, off: f32) -> f32 {
  return crtGaus(crtDist(pos).y + off, CRT_HARD_SCAN);
}

fn crtTri(pos: vec2<f32>) -> vec3<f32> {
  let a = crtHorz3(pos, -1.0);
  let b = crtHorz5(pos, 0.0);
  let c = crtHorz3(pos, 1.0);
  let wa = crtScan(pos, -1.0);
  let wb = crtScan(pos, 0.0);
  let wc = crtScan(pos, 1.0);
  return a * wa + b * wb + c * wc;
}

fn crtMask(pos_in: vec2<f32>) -> vec3<f32> {
  var pos = pos_in;
  pos.x += pos.y * 3.0;
  var m = vec3<f32>(CRT_MASK_DARK);
  pos.x = fract(pos.x / 6.0);
  if (pos.x < 0.333) {
    m.r = CRT_MASK_LIGHT;
  } else if (pos.x < 0.666) {
    m.g = CRT_MASK_LIGHT;
  } else {
    m.b = CRT_MASK_LIGHT;
  }
  return m;
}

fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  return vec4<f32>(crtTri(vUv) * crtMask(fragCoord), 1.0);
}
"#,
    },
    // Point10Effects.js:397-428 — Cubic Lens Distortion
    P10Effect {
        name: "Point10 Cubic Lens Distortion",
        chunks: &[],
        audio: false,
        body: r#"fn cubicRand(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453 + fract(u.iTime) * 0.5);
}

fn cubicBlur(p: vec2<f32>) -> vec3<f32> {
  let powers = pow(abs(p - 0.5), vec2<f32>(2.0));
  let n = cubicRand(p) * 0.004 * smoothstep(0.05, 0.45, powers.x + powers.y) * 5.5;
  let xy1 = p + n;
  let xy2 = p - n;
  return ((textureSample(u_modVCanvas, modvGroupSampler, xy1) +
    textureSample(u_modVCanvas, modvGroupSampler, xy2) +
    textureSample(u_modVCanvas, modvGroupSampler, vec2<f32>(xy1.x, xy2.y)) +
    textureSample(u_modVCanvas, modvGroupSampler, vec2<f32>(xy2.x, xy1.y))) / 4.0).rgb;
}

fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let p = vUv * 2.0 - 1.0;
  let rescaleTerm = 0.9 * (vUv - 0.5);
  let c = vec3<f32>(
    cubicBlur(1.0675 * rescaleTerm + 0.5).r,
    cubicBlur(1.0450 * rescaleTerm + 0.5).g,
    cubicBlur(1.0225 * rescaleTerm + 0.5).b
  );
  let shaded = c * (0.5 + 0.8 * clamp(1.0 - dot(p, p), 0.0, 1.0));
  return vec4<f32>(shaded, 1.0);
}
"#,
    },
    // Point10Effects.js:430-469 — Hexagons
    P10Effect {
        name: "Point10 Hexagons",
        chunks: &[],
        audio: false,
        body: r#"fn downsample36(p: vec2<f32>) -> vec4<f32> {
  let size = u.iResolution.xy;
  let ts = 1.0 / size;
  let ts2 = ts * 2.0;
  let uv = p / size;
  var c = textureSample(u_modVCanvas, modvGroupSampler, uv) * 0.5;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(ts.x, ts.y)) * 0.5;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(-ts.x, ts.y)) * 0.5;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(ts.x, -ts.y)) * 0.5;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(-ts.x, -ts.y)) * 0.5;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(0.0, ts2.y)) * 0.25;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(-ts2.x, 0.0)) * 0.25;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(ts2.x, 0.0)) * 0.25;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(0.0, -ts2.y)) * 0.25;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(ts2.x, ts2.y)) * 0.125;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(-ts2.x, ts2.y)) * 0.125;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(ts2.x, -ts2.y)) * 0.125;
  c += textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(-ts2.x, -ts2.y)) * 0.125;
  return c * 0.25;
}

fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let sqrt3 = 1.7320508;
  let halfSqrt3 = 0.8660254;
  let size = 10.0;
  let grid = vec2<f32>(size, sqrt3 * size);
  let base = u.iResolution.xy * 0.5;
  let fc = fragCoord;
  let po1 = floor((fc - base) / grid + 0.5) * grid + base - fc;
  let dist1 = length(po1);
  let base2 = base + size * vec2<f32>(0.5, halfSqrt3);
  let po2 = floor((fc - base2) / grid + 0.5) * grid + base2 - fc;
  let dist2 = length(po2);
  // GLSL `dist1 < dist2 ? po1 : po2` → WGSL select(false, true, cond).
  let sp = select(po2, po1, dist1 < dist2);
  return downsample36(sp + fc);
}
"#,
    },
    // Point10Effects.js:471-489 — Led Display
    P10Effect {
        name: "Point10 Led Display",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let size = u.iResolution.xy;
  let ledSize = 1.0 / size;
  let ledCenter = floor(vUv * size) / size + ledSize * 0.5;
  var c = textureSample(u_modVCanvas, modvGroupSampler, ledCenter);
  let luminance = (c.r + c.g + c.b) / 3.0;
  if (luminance > 0.01) {
    let ratios = c.rgb / luminance;
    let stepSize = 1.0 / 100.0;
    let factor = stepSize * floor(luminance / stepSize);
    c = vec4<f32>(ratios * factor * 1.5, c.a);
  }
  let v = length((ledCenter - vUv) / (ledSize * 0.5));
  return mix(c, vec4<f32>(0.0, 0.0, 0.0, 1.0), v);
}
"#,
    },
    // Point10Effects.js:491-508 — Led Display Curved
    P10Effect {
        name: "Point10 Led Display Curved",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let p = vUv * 2.0 - 1.0;
  let bend = vUv + p * vec2<f32>(p.x * p.x, p.y * p.y) * 0.04;
  let size = u.iResolution.xy;
  let ledSize = 1.0 / size;
  let ledCenter = floor(bend * size) / size + ledSize * 0.5;
  var c = textureSample(u_modVCanvas, modvGroupSampler, ledCenter);
  let lpos = (ledCenter - bend) / (ledSize * 0.5);
  let v = 1.0 - smoothstep(
    0.1,
    1.0,
    lpos.x * lpos.x + lpos.y * lpos.y - lpos.x * lpos.x * lpos.y * lpos.y
  );
  c = mix(vec4<f32>(0.0, 0.0, 0.0, 1.0), c, v);
  c = mix(vec4<f32>(0.0, 0.0, 0.0, 1.0), c, pow(1.0 - abs(p.y), 1.8));
  return c;
}
"#,
    },
    // Point10Effects.js:510-549 — Limit Hue
    P10Effect {
        name: "Point10 Limit Hue",
        chunks: &[],
        audio: false,
        body: r#"fn minChannel(v: vec3<f32>) -> f32 {
  return min(min(v.x, v.y), v.z);
}

fn maxChannel(v: vec3<f32>) -> f32 {
  return max(max(v.x, v.y), v.z);
}

fn rgbToHsv(rgb: vec3<f32>) -> vec3<f32> {
  var hsv = vec3<f32>(0.0);
  let minValue = minChannel(rgb);
  let maxValue = maxChannel(rgb);
  let delta = maxValue - minValue;
  hsv.z = maxValue;
  if (delta != 0.0) {
    hsv.y = delta / maxValue;
    let delRgb = (((vec3<f32>(maxValue) - rgb) / 6.0) + (delta / 2.0)) / delta;
    if (rgb.x == maxValue) {
      hsv.x = delRgb.z - delRgb.y;
    } else if (rgb.y == maxValue) {
      hsv.x = 1.0 / 3.0 + delRgb.x - delRgb.z;
    } else {
      hsv.x = 2.0 / 3.0 + delRgb.y - delRgb.x;
    }
    if (hsv.x < 0.0) {
      hsv.x += 1.0;
    }
    if (hsv.x > 1.0) {
      hsv.x -= 1.0;
    }
  }
  return hsv;
}

fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let minHue = fract(u.iTime * 0.075);
  let maxHue = fract(minHue + 0.2);
  let hsv = rgbToHsv(c.rgb);
  if (maxHue > minHue) {
    if (hsv.x < minHue || hsv.x > maxHue) {
      c = vec4<f32>(vec3<f32>(dot(vec3<f32>(0.222, 0.707, 0.071), c.rgb)), c.a);
    }
  } else if (hsv.x > maxHue && hsv.x < minHue) {
    c = vec4<f32>(vec3<f32>(dot(vec3<f32>(0.222, 0.707, 0.071), c.rgb)), c.a);
  }
  return c;
}
"#,
    },
    // Point10Effects.js:551-565 — Old Movie
    P10Effect {
        name: "Point10 Old Movie",
        chunks: &[P10Chunk::Noise],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let gray = dot(vec3<f32>(0.3, 0.59, 0.11), c.rgb);
  // The GLSL `muted` capture is intentionally dropped (unused there too);
  // `sepia` is the one the source actually keeps.
  let sepia = mix(vec3<f32>(0.1, 0.05, 0.0), vec3<f32>(1.0, 0.9, 0.5), gray);
  c = vec4<f32>(sepia, c.a);
  let s = vUv * vec2<f32>(3.0, 80.0) + vec2<f32>(u.iTime * 0.03, u.iTime * 0.03);
  var scratch = p10Hash(floor(s * vec2<f32>(80.0, 1.0)));
  scratch = smoothstep(0.985, 1.0, scratch);
  return c + vec4<f32>(vec3<f32>(scratch), 0.0);
}
"#,
    },
    // Point10Effects.js:567-578 — Embos Merge
    P10Effect {
        name: "Point10 Embos Merge",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  var c = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let texel = 1.0 / u.iResolution.xy;
  let emboss = textureSample(u_modVCanvas, modvGroupSampler, vUv - texel).rgb * 2.0 -
    textureSample(u_modVCanvas, modvGroupSampler, vUv + texel).rgb * 2.0;
  c = vec4<f32>(c.rgb + vec3<f32>((emboss.r + emboss.g + emboss.b) / 3.0), c.a);
  return c;
}
"#,
    },
    // Point10Effects.js:580-591 — Embos
    P10Effect {
        name: "Point10 Embos",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let texel = 1.0 / u.iResolution.xy;
  let emboss = textureSample(u_modVCanvas, modvGroupSampler, vUv - texel).rgb * 2.0 -
    textureSample(u_modVCanvas, modvGroupSampler, vUv + texel).rgb * 2.0;
  let value = (emboss.r + emboss.g + emboss.b) / 3.0;
  let base = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  return vec4<f32>(base.rgb + vec3<f32>(value) + vec3<f32>(value) * 0.6, 1.0);
}
"#,
    },
    // Point10Effects.js:593-601 — Pixelation
    P10Effect {
        name: "Point10 Pixelation",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let cells = max(vec2<f32>(1.0), floor(u.iResolution.xy / 8.0));
  let uv = (floor(vUv * cells) + 0.5) / cells;
  return textureSample(u_modVCanvas, modvGroupSampler, uv);
}
"#,
    },
    // Point10Effects.js:603-616 — RGB Shift (audioFeatures: bass, mid, treble)
    P10Effect {
        name: "Point10 RGB Shift",
        chunks: &[],
        audio: true,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let amount = 0.0015 + min(0.007, u.u_mid * 0.01);
  let offset = vec2<f32>(cos(amount * 1200.0), sin(amount * 900.0)) / u.iResolution.xy * 4.0;
  let center = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let red = textureSample(u_modVCanvas, modvGroupSampler, vUv + offset).r;
  let blue = textureSample(u_modVCanvas, modvGroupSampler, vUv - offset).b;
  return vec4<f32>(red, center.g, blue, center.a);
}
"#,
    },
    // Point10Effects.js:618-638 — Digital Glitch (audioFeatures: bass, mid, treble)
    P10Effect {
        name: "Point10 Digital Glitch",
        chunks: &[P10Chunk::Noise],
        audio: true,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let amount = min(0.09, 0.012 + f32(u.u_kick) * 0.035 + u.u_bass * 0.02);
  let frame = floor(u.iTime * 14.0);
  let row = floor(vUv.y * 32.0);
  let burst = step(0.82, p10Hash(vec2<f32>(row, frame)));
  var uv = vUv;
  uv.x += (p10Hash(vec2<f32>(row, frame + 1.0)) - 0.5) * amount * burst;
  var color = textureSample(u_modVCanvas, modvGroupSampler, uv);
  let split = amount * (0.35 + p10Hash(vec2<f32>(row, frame + 2.0)) * 0.65);
  color.r = textureSample(u_modVCanvas, modvGroupSampler, uv + vec2<f32>(split, 0.0)).r;
  color.b = textureSample(u_modVCanvas, modvGroupSampler, uv - vec2<f32>(split, 0.0)).b;
  return color;
}
"#,
    },
    // Point10Effects.js:640-649 — Vignette
    P10Effect {
        name: "Point10 Vignette",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let color = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let p = vUv * 2.0 - 1.0;
  let edge = smoothstep(0.35, 1.45, dot(p, p));
  return vec4<f32>(color.rgb * (1.0 - edge * 0.72), color.a);
}
"#,
    },
    // Point10Effects.js:651-674 — Bloom
    P10Effect {
        name: "Point10 Bloom",
        chunks: &[],
        audio: false,
        body: r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let base = textureSample(u_modVCanvas, modvGroupSampler, vUv);
  let texel = 1.0 / u.iResolution.xy;
  var glow = vec3<f32>(0.0);
  var weight = 0.0;
  for (var x = -2; x <= 2; x++) {
    for (var y = -2; y <= 2; y++) {
      let offset = vec2<f32>(f32(x), f32(y)) * texel * 3.0;
      let sampleColor = textureSample(u_modVCanvas, modvGroupSampler, vUv + offset).rgb;
      let brightness = max(sampleColor.r, max(sampleColor.g, sampleColor.b));
      let w = smoothstep(0.35, 0.9, brightness) / (1.0 + f32(x * x + y * y));
      glow += sampleColor * w;
      weight += w;
    }
  }
  glow /= max(weight, 0.0001);
  return vec4<f32>(base.rgb + glow * 0.72, 1.0);
}
"#,
    },
];

/// All 31 Point10 effect modules, in `Point10Effects.js` registration order.
pub(crate) fn modules() -> Vec<ShaderModule> {
    P10_EFFECTS
        .iter()
        .map(|effect| {
            let mut source = String::new();
            for chunk in effect.chunks {
                match chunk {
                    P10Chunk::Noise => source.push_str(P10_NOISE_WGSL),
                    P10Chunk::Bayer => source.push_str(P10_BAYER_WGSL),
                    P10Chunk::ModF => source.push_str(GLSL_MOD_F_WGSL),
                    P10Chunk::ModV2 => source.push_str(GLSL_MOD_V2_WGSL),
                }
            }
            source.push_str(effect.body);
            ShaderModule {
                name: effect.name,
                fragment_wgsl: main_image_fragment(&source),
                meta_author: "Point10",
                meta_version: "1.0.0",
                category: None,
                audio: effect.audio,
                audio_features: if effect.audio {
                    &["bass", "mid", "treble"]
                } else {
                    &[]
                },
                preview_with_output: true,
                props: Vec::new(),
            }
        })
        .collect()
}
