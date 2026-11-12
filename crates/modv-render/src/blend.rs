//! Canvas 2D `globalCompositeOperation` — all 28 operations, W3C-exact.
//!
//! Source of truth (all anchors in this repository):
//!
//! - `src/renderer/src/util/composite-operations.js` — the 28 tokens the UI
//!   offers (15 blend modes incl. the 4 non-separable ones, 13 Porter-Duff
//!   composite modes); the JS classifies the non-separable quartet
//!   (`hue`/`saturation`/`color`/`luminosity`) under "Blend Modes".
//! - `docs/rust-hybrid.md` WS-1 "Blend modes" and WS-2 "Blend modes".
//!
//! ## One code path, by design
//!
//! The doc sketch optimises the Porter-Duff modes onto fixed-function
//! `wgpu::BlendState` factors and reserves the shader path for the
//! non-separable blend modes. This module deliberately implements **every**
//! mode as one shader code path ([`BLEND_WGSL`]) instead: the separable
//! blend functions (e.g. multiply needs the `Cb × Cs` product) and the
//! non-separable HSL functions are not expressible as blend-state factors,
//! and splitting the path early risks drifting the two implementations apart
//! on edge cases (un-premultiplication, zero-alpha backdrops). Uniform
//! correctness first; the fixed-function Porter-Duff fast path is a
//! documented follow-up once the golden-pixel suite is green.
//!
//! ## Formula reference
//!
//! W3C Compositing and Blending Level 1
//! (<https://www.w3.org/TR/compositing-1/>):
//!
//! - Porter-Duff (\u00a7 9): general equation `co = αs×Fa×Cs + αb×Fb×Cb`, with
//!   `Fa`/`Fb` per operator. Evaluated on the **premultiplied** channel
//!   values (`cs = Cs×αs`, `cb = Cb×αb`), which reduces the general equation
//!   to `co = Fa×cs + Fb×cb`, `αo = Fa×αs + Fb×αb`.
//! - Blend modes (\u00a7 10): the spec forbids blending on premultiplied
//!   values, so the shader first un-premultiplies (`Cs = cs/αs`,
//!   `Cb = cb/αb`, zero channels yield 0), applies `B(Cb, Cs)`, and
//!   recombines via the general formula
//!   `co = αs×(1-αb)×Cs + αb×(1-αs)×Cb + αs×αb×B(Cb, Cs)`,
//!   `αo = αs + αb×(1-αs)`.
//! - `normal` blends with `B = Cs`, which is algebraically identical to
//!   Porter-Duff `source-over` (canvas aliases them). They stay two distinct
//!   enum variants / two distinct mode indices; only the *behaviour* is
//!   shared.
//! - Non-separable helpers (`Lum`/`Sat`/`ClipColor`/`SetLum`/`SetSat`) use
//!   the spec's Rec. 601 luma weights `0.3 / 0.59 / 0.11`.
//!
//! ## Pixel-space contract
//!
//! Canvas 2D performs these operations on the raw (non-linearised) channel
//! values of its 8-bit backing store. The compositor therefore runs on
//! `Rgba8Unorm` textures; sRGB-encoded formats would linearise on sample and
//! re-encode on store and are reserved for the present boundary (M3). All
//! internal textures are **premultiplied** RGBA; the one exception is the
//! final sRGB swapchain, un-premultiplied at present time.
//!
//! The uniform `src_alpha_scale` multiplies the source's alpha **and its
//! premultiplied colour** before compositing — `globalAlpha` in canvas terms
//! (`drawTo.globalAlpha = module.meta.alpha` in `worker/loop.js`). The doc
//! sketch typed the uniform as `{ mode: u32 }`; it is extended here with the
//! scale so globalAlpha needs no extra pass, keeping one code path.

use std::fmt;

/// Unknown `globalCompositeOperation` token (canvas accepts only the 28 in
/// `composite-operations.js`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownBlendMode(pub String);

impl fmt::Display for UnknownBlendMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown blend mode: {:?}", self.0)
    }
}

impl std::error::Error for UnknownBlendMode {}

/// Canvas 2D `globalCompositeOperation` values (28).
///
/// Variants are named after the spec's camel/kebab runs collapsed to
/// PascalCase; the wire token (e.g. `"color-dodge"`) comes from
/// [`BlendMode::name`]. The discriminants are stable and match the `mode`
/// uniform evaluated by [`BLEND_WGSL`]; append new modes at the end only.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum BlendMode {
    // The 15 canvas "Blend Modes" from composite-operations.js (11 separable
    // + the 4 non-separable HSL modes).
    Normal = 0,
    Multiply = 1,
    Overlay = 2,
    Darken = 3,
    Lighten = 4,
    ColorDodge = 5,
    ColorBurn = 6,
    HardLight = 7,
    SoftLight = 8,
    Difference = 9,
    Exclusion = 10,
    Hue = 11,
    Saturation = 12,
    Color = 13,
    Luminosity = 14,
    // The 13 "Composite Modes" (Porter-Duff; `clear` included).
    Clear = 15,
    Copy = 16,
    Destination = 17,
    SourceOver = 18,
    DestinationOver = 19,
    SourceIn = 20,
    DestinationIn = 21,
    SourceOut = 22,
    DestinationOut = 23,
    SourceAtop = 24,
    DestinationAtop = 25,
    Xor = 26,
    Lighter = 27,
}

impl BlendMode {
    /// Every supported token, in the order `composite-operations.js` lists
    /// them (blend modes first, then composite modes).
    pub const ALL: &[BlendMode] = &[
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::Clear,
        BlendMode::Copy,
        BlendMode::Destination,
        BlendMode::SourceOver,
        BlendMode::DestinationOver,
        BlendMode::SourceIn,
        BlendMode::DestinationIn,
        BlendMode::SourceOut,
        BlendMode::DestinationOut,
        BlendMode::SourceAtop,
        BlendMode::DestinationAtop,
        BlendMode::Xor,
        BlendMode::Lighter,
    ];

    /// Parse a canvas token (`globalCompositeOperation` is an ASCII
    /// case-insensitive enumerated attribute, so the match is too).
    ///
    /// Unknown tokens (e.g. `screen`, `plus-lighter`, the legacy `source-in`
    /// aliases, or non-strings) yield [`UnknownBlendMode`].
    pub fn from_name(name: &str) -> Result<BlendMode, UnknownBlendMode> {
        // `to_ascii_lowercase` on a borrowed str only allocates on the
        // mismatched path in newer std; an explicit match keeps it simple.
        let lowered = name.to_ascii_lowercase();
        match lowered.as_str() {
            "normal" => Ok(BlendMode::Normal),
            "multiply" => Ok(BlendMode::Multiply),
            "overlay" => Ok(BlendMode::Overlay),
            "darken" => Ok(BlendMode::Darken),
            "lighten" => Ok(BlendMode::Lighten),
            "color-dodge" => Ok(BlendMode::ColorDodge),
            "color-burn" => Ok(BlendMode::ColorBurn),
            "hard-light" => Ok(BlendMode::HardLight),
            "soft-light" => Ok(BlendMode::SoftLight),
            "difference" => Ok(BlendMode::Difference),
            "exclusion" => Ok(BlendMode::Exclusion),
            "hue" => Ok(BlendMode::Hue),
            "saturation" => Ok(BlendMode::Saturation),
            "color" => Ok(BlendMode::Color),
            "luminosity" => Ok(BlendMode::Luminosity),
            "clear" => Ok(BlendMode::Clear),
            "copy" => Ok(BlendMode::Copy),
            "destination" => Ok(BlendMode::Destination),
            "source-over" => Ok(BlendMode::SourceOver),
            "destination-over" => Ok(BlendMode::DestinationOver),
            "source-in" => Ok(BlendMode::SourceIn),
            "destination-in" => Ok(BlendMode::DestinationIn),
            "source-out" => Ok(BlendMode::SourceOut),
            "destination-out" => Ok(BlendMode::DestinationOut),
            "source-atop" => Ok(BlendMode::SourceAtop),
            "destination-atop" => Ok(BlendMode::DestinationAtop),
            "xor" => Ok(BlendMode::Xor),
            "lighter" => Ok(BlendMode::Lighter),
            _ => Err(UnknownBlendMode(name.to_owned())),
        }
    }

    /// The exact `globalCompositeOperation` token (`composite-operations.js`).
    pub fn name(&self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::ColorDodge => "color-dodge",
            BlendMode::ColorBurn => "color-burn",
            BlendMode::HardLight => "hard-light",
            BlendMode::SoftLight => "soft-light",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
            BlendMode::Clear => "clear",
            BlendMode::Copy => "copy",
            BlendMode::Destination => "destination",
            BlendMode::SourceOver => "source-over",
            BlendMode::DestinationOver => "destination-over",
            BlendMode::SourceIn => "source-in",
            BlendMode::DestinationIn => "destination-in",
            BlendMode::SourceOut => "source-out",
            BlendMode::DestinationOut => "destination-out",
            BlendMode::SourceAtop => "source-atop",
            BlendMode::DestinationAtop => "destination-atop",
            BlendMode::Xor => "xor",
            BlendMode::Lighter => "lighter",
        }
    }

    /// The `mode` uniform evaluated by [`BLEND_WGSL`].
    pub(crate) fn mode_index(self) -> u32 {
        self as u32
    }

    /// True for the 13 Porter-Duff composite operators (used by the
    /// fixed-function fast-path follow-up; not used by the shader path).
    pub fn is_porter_duff(&self) -> bool {
        (*self as u32) >= BlendMode::Clear as u32
    }
}

/// Full-screen-triangle blit implementing every [`BlendMode`].
///
/// `@group(0)` bindings: `dst_texture` + `dst_sampler` (the current canvas
/// contents, premultiplied), `src_texture` + `src_sampler` (the element being
/// drawn, premultiplied), and a uniform `{ mode: u32, src_alpha_scale: f32 }`.
/// Output is the premultiplied blend result; nothing is discarded.
///
/// The vertex stage emits the oversized triangle `(-1,-1), (3,-1), (-1,3)`
/// so the rasteriser clips it to a full-screen triangle; no vertex buffer is
/// needed. `uv.y` maps clip `y = +1` (top of the render target) to `v = 0`
/// (texture row 0), matching canvas's y-down drawImage on an upload that is
/// row-0-first.
pub const BLEND_WGSL: &str = r#"// Canvas 2D globalCompositeOperation — one code path for all 28 modes.
// Formulas: W3C Compositing and Blending Level 1 (https://www.w3.org/TR/compositing-1/).
// Porter-Duff (§9): co = Fa×cs_p + Fb×cb_p on premultiplied inputs.
// Blend modes (§10): un-premultiply, apply B(cb, cs), recombine with the
// general formula co = sa(1-da)cs + da(1-sa)cb + sa·da·B(cb, cs), ao = sa + da(1-sa).
// Non-separable HSL helpers use Rec. 601 luma weights (0.3 / 0.59 / 0.11).

struct BlendUniform {
  mode: u32,
  src_alpha_scale: f32,
}

@group(0) @binding(0) var dst_texture: texture_2d<f32>;
@group(0) @binding(1) var dst_sampler: sampler;
@group(0) @binding(2) var src_texture: texture_2d<f32>;
@group(0) @binding(3) var src_sampler: sampler;
@group(0) @binding(4) var<uniform> u: BlendUniform;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) tex_coord: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
  var out: VertexOutput = VertexOutput(vec4<f32>(0.0, 0.0, 0.0, 1.0), vec2<f32>(0.0, 0.0));
  var positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  let position = positions[index];
  out.position = vec4<f32>(position, 0.0, 1.0);
  out.tex_coord = vec2<f32>((position.x + 1.0) * 0.5, (1.0 - (position.y + 1.0) * 0.5));
  return out;
}

fn lum(c: vec3<f32>) -> f32 {
  return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn sat(c: vec3<f32>) -> f32 {
  return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

fn clip_color(c: vec3<f32>) -> vec3<f32> {
  let l = lum(c);
  let n = min(min(c.r, c.g), c.b);
  let x = max(max(c.r, c.g), c.b);
  var out = c;
  if (n < 0.0) {
    let d = l - n;
    if (d != 0.0) {
      out = vec3<f32>(l) + (out - vec3<f32>(l)) * (l / d);
    }
  }
  if (x > 1.0) {
    let d = x - l;
    if (d != 0.0) {
      out = vec3<f32>(l) + (out - vec3<f32>(l)) * ((1.0 - l) / d);
    }
  }
  return out;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
  let d = l - lum(c);
  return clip_color(c + vec3<f32>(d, d, d));
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
  let mx = max(max(c.r, c.g), c.b);
  let mn = min(min(c.r, c.g), c.b);
  if (mx <= mn) {
    return vec3<f32>(0.0, 0.0, 0.0);
  }
  let md = c.r + c.g + c.b - mx - mn;
  let new_md = (md - mn) * s / (mx - mn);
  // mx/mn come out of max()/min() so the `==` comparisons are exact.
  var out: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);
  out.r = select(select(new_md, 0.0, c.r == mn), s, c.r == mx);
  out.g = select(select(new_md, 0.0, c.g == mn), s, c.g == mx);
  out.b = select(select(new_md, 0.0, c.b == mn), s, c.b == mx);
  return out;
}

fn color_dodge(b: f32, s: f32) -> f32 {
  if (b == 0.0) { return 0.0; }
  if (s == 1.0) { return 1.0; }
  return min(1.0, b / (1.0 - s));
}

fn color_burn(b: f32, s: f32) -> f32 {
  if (b == 1.0) { return 1.0; }
  if (s == 0.0) { return 0.0; }
  return 1.0 - min(1.0, (1.0 - b) / s);
}

fn hard_light(b: f32, s: f32) -> f32 {
  if (s <= 0.5) {
    return b * (2.0 * s);
  }
  // Screen(Cb, 2×Cs - 1)
  return 1.0 - (1.0 - b) * (1.0 - (2.0 * s - 1.0));
}

fn soft_light(b: f32, s: f32) -> f32 {
  if (s <= 0.5) {
    return b - (1.0 - 2.0 * s) * b * (1.0 - b);
  }
  var d: f32 = 0.0;
  if (b <= 0.25) {
    d = ((16.0 * b - 12.0) * b + 4.0) * b;
  } else {
    d = sqrt(b);
  }
  return b + (2.0 * s - 1.0) * (d - b);
}

// B(cb, cs) for the 15 canvas blend modes (cases 0..=14).
fn blend(cb: vec3<f32>, cs: vec3<f32>, mode: u32) -> vec3<f32> {
  switch (mode) {
    case 0u: { return cs; }                            // normal
    case 1u: { return cb * cs; }                       // multiply
    case 2u: {                                         // overlay = HardLight swapped (B = HardLight(Cs, Cb))
      var out: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);
      out.r = hard_light(cs.r, cb.r);
      out.g = hard_light(cs.g, cb.g);
      out.b = hard_light(cs.b, cb.b);
      return out;
    }
    case 3u: { return min(cb, cs); }                   // darken
    case 4u: { return max(cb, cs); }                   // lighten
    case 5u: {                                         // color-dodge
      return vec3<f32>(color_dodge(cb.r, cs.r), color_dodge(cb.g, cs.g), color_dodge(cb.b, cs.b));
    }
    case 6u: {                                         // color-burn
      return vec3<f32>(color_burn(cb.r, cs.r), color_burn(cb.g, cs.g), color_burn(cb.b, cs.b));
    }
    case 7u: {                                         // hard-light
      return vec3<f32>(hard_light(cb.r, cs.r), hard_light(cb.g, cs.g), hard_light(cb.b, cs.b));
    }
    case 8u: {                                         // soft-light
      return vec3<f32>(soft_light(cb.r, cs.r), soft_light(cb.g, cs.g), soft_light(cb.b, cs.b));
    }
    case 9u: { return abs(cb - cs); }                  // difference
    case 10u: { return cb + cs - 2.0 * cb * cs; }      // exclusion
    case 11u: { return set_lum(set_sat(cs, sat(cb)), lum(cb)); }  // hue
    case 12u: { return set_lum(set_sat(cb, sat(cs)), lum(cb)); }  // saturation
    case 13u: { return set_lum(cs, lum(cb)); }                    // color
    case 14u: { return set_lum(cb, lum(cs)); }                    // luminosity
    default: { return cs; }
  }
}

struct FragmentOutput {
  @location(0) color: vec4<f32>,
}

@fragment
fn fs_main(input: VertexOutput) -> FragmentOutput {
  var out: FragmentOutput = FragmentOutput(vec4<f32>(0.0, 0.0, 0.0, 0.0));
  let src = textureSample(src_texture, src_sampler, input.tex_coord);
  let dst = textureSample(dst_texture, dst_sampler, input.tex_coord);

  // globalAlpha: scale the source's premultiplied colour and its alpha.
  let scale = clamp(u.src_alpha_scale, 0.0, 1.0);
  let sa = clamp(src.a * scale, 0.0, 1.0);
  let cs_p = src.rgb * scale;
  let da = clamp(dst.a, 0.0, 1.0);
  let cb_p = dst.rgb; // internal textures are premultiplied

  let m = u.mode;
  var co: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);
  var ao: f32 = 0.0;

  if (m >= 15u) {
    // Porter-Duff on the premultiplied channels (§9).
    var fa: f32 = 1.0;
    var fb: f32 = 1.0 - sa;
    switch (m) {
      case 15u: { fa = 0.0; fb = 0.0; }                // clear
      case 16u: { fa = 1.0; fb = 0.0; }                // copy
      case 17u: { fa = 0.0; fb = 1.0; }                // destination
      case 18u: { fa = 1.0; fb = 1.0 - sa; }           // source-over
      case 19u: { fa = 1.0 - da; fb = 1.0; }           // destination-over
      case 20u: { fa = da; fb = 0.0; }                 // source-in
      case 21u: { fa = 0.0; fb = sa; }                 // destination-in
      case 22u: { fa = 1.0 - da; fb = 0.0; }           // source-out
      case 23u: { fa = 0.0; fb = 1.0 - sa; }           // destination-out
      case 24u: { fa = da; fb = 1.0 - sa; }            // source-atop
      case 25u: { fa = 1.0 - da; fb = sa; }            // destination-atop
      case 26u: { fa = 1.0 - da; fb = 1.0 - sa; }      // xor
      case 27u: { fa = 1.0; fb = 1.0; }                // lighter
      default: { fa = 1.0; fb = 1.0 - sa; }
    }
    co = fa * cs_p + fb * cb_p;
    ao = fa * sa + fb * da;
  } else {
    // Blend modes (§10): the spec forbids blending premultiplied values,
    // so un-premultiply first, then recombine with the general formula.
    var cs: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);
    var cb: vec3<f32> = vec3<f32>(0.0, 0.0, 0.0);
    if (sa > 0.0) { cs = cs_p / sa; }
    if (da > 0.0) { cb = cb_p / da; }
    let blended = blend(cb, cs, m);
    co = sa * (1.0 - da) * cs + da * (1.0 - sa) * cb + sa * da * blended;
    ao = sa + da * (1.0 - sa);
  }

  // Canvas clamps to the output bit depth; UNORM would clamp anyway, but
  // clamping here keeps `lighter` (Fa = Fb = 1) well-defined too.
  out.color = vec4<f32>(clamp(co, vec3<f32>(0.0), vec3<f32>(1.0)), clamp(ao, 0.0, 1.0));
  return out;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens offered by `src/renderer/src/util/composite-operations.js`
    /// (source checkout; the two menus concatenate to the 28 below).
    const JS_TOKENS: &[&str] = &[
        "normal",
        "multiply",
        "overlay",
        "darken",
        "lighten",
        "color-dodge",
        "color-burn",
        "hard-light",
        "soft-light",
        "difference",
        "exclusion",
        "hue",
        "saturation",
        "color",
        "luminosity", // "Blend Modes"
        "clear",
        "copy",
        "destination",
        "source-over",
        "destination-over",
        "source-in",
        "destination-in",
        "source-out",
        "destination-out",
        "source-atop",
        "destination-atop",
        "xor",
        "lighter", // "Composite Modes"
    ];

    #[test]
    fn all_contains_exactly_the_js_tokens() {
        let mut from_enum: Vec<&str> = BlendMode::ALL.iter().map(BlendMode::name).collect();
        from_enum.sort_unstable();
        let mut from_js = JS_TOKENS.to_vec();
        from_js.sort_unstable();
        assert_eq!(from_enum, from_js);
        assert_eq!(BlendMode::ALL.len(), 28);
    }

    #[test]
    fn mode_indices_are_unique() {
        let mut indices: Vec<u32> = BlendMode::ALL.iter().map(|mode| *mode as u32).collect();
        indices.sort_unstable();
        let mut expected: Vec<u32> = (0..28).collect();
        expected.dedup();
        assert_eq!(indices, expected);
    }

    #[test]
    fn blend_mode_parse_all() {
        for mode in BlendMode::ALL {
            let parsed = BlendMode::from_name(mode.name())
                .unwrap_or_else(|e| panic!("{} must parse: {e}", mode.name()));
            assert_eq!(&parsed, mode);
        }
    }

    #[test]
    fn from_name_is_case_insensitive() {
        assert_eq!(BlendMode::from_name("Normal"), Ok(BlendMode::Normal));
        assert_eq!(
            BlendMode::from_name("SOURCE-OVER"),
            Ok(BlendMode::SourceOver)
        );
        assert_eq!(
            BlendMode::from_name("Color-Dodge"),
            Ok(BlendMode::ColorDodge)
        );
    }

    #[test]
    fn unknown_tokens_are_rejected() {
        // `screen` is a CSS blend mode but is NOT one of the 28 canvas
        // operations in composite-operations.js; it must not silently map.
        assert!(BlendMode::from_name("screen").is_err());
        assert!(BlendMode::from_name("plus-lighter").is_err());
        assert!(BlendMode::from_name("").is_err());
        assert!(BlendMode::from_name("normal ").is_err());
        assert_eq!(
            BlendMode::from_name("nope").unwrap_err(),
            UnknownBlendMode("nope".to_owned())
        );
    }

    #[test]
    fn normal_and_source_over_are_distinct_variants_same_names_behaviour() {
        assert_ne!(BlendMode::Normal, BlendMode::SourceOver);
        assert_ne!(BlendMode::Normal.name(), BlendMode::SourceOver.name());
    }
}
