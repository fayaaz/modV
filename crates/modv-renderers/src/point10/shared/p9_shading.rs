//! The Plane9 **lighting + colour** helpers, in WGSL.
//!
//! Batch 1-3 only needed the noise/random/hsv half of `plane9/P9Glsl.js`
//! ([`super::p9_glsl::P9_COMMON_WGSL`]). The batch-4 cube scenes are the first
//! ones that light geometry with Plane9's Cook-Torrance helpers, so that half
//! lands here:
//!
//! | `P9Glsl.js` | Rust (WGSL) | Used by |
//! |-------------|-------------|---------|
//! | `_rand3(float)` | `p9_rand3` | Boxy |
//! | `_palette(t, brightness, contrast, a, b)` | `p9_palette` | Boxy, ColorCube |
//! | `_shade(...)` | `p9_shade` | Boxy, ColorCube (via `p9_light_directional`) |
//! | `_lightDirectional(...)` | `p9_light_directional` | Boxy, ColorCube |
//! | `_fresnelRoughness(...)` | `p9_fresnel_roughness` | Boxy |
//! | `_texturePanoramicLod(tex, v, lod)` | `p9_texture_panoramic_lod` | Boxy |
//! | GLSL `mod(x, y)` | `p9_mod` | Boxy |
//!
//! # Concatenation order
//!
//! These helpers call `p9_saturate` and read `P9_PI`/`P9_PI2`, so a scene
//! concatenates [`super::p9_glsl::P9_COMMON_WGSL`] **first**:
//!
//! ```text
//! format!("{P9_COMMON_WGSL}\n{P9_SHADING_WGSL}\n{SCENE_WGSL}")
//! ```
//!
//! # GLSL → WGSL notes
//!
//! * `p2 += dot(p2.yzx, …)` adds a *scalar* to a vector in GLSL; WGSL needs the
//!   explicit `vec3<f32>(…)` splat.
//! * GLSL's `mod(x, y)` is `x - y * floor(x / y)` (always the same sign as `y`);
//!   WGSL's `%` on floats is a truncated remainder and goes negative for a
//!   negative `x`. [`p9_rand3`]'s consumers (`Boxy`'s tunnel scroll) feed
//!   negative values, so the port keeps GLSL's definition as `p9_mod`.
//! * `acos` is undefined outside `[-1, 1]` in both languages; the panorama
//!   helper clamps `v.y` because a normalised direction can land a few ULPs
//!   outside after interpolation.
//!
//! Nothing in `P9Glsl.js`'s tone-map / voronoi / black-body / bicubic halves is
//! ported yet — no ported scene calls them.

#![forbid(unsafe_code)]

/// WGSL port of Plane9's lighting/colour helpers.
///
/// Requires [`super::p9_glsl::P9_COMMON_WGSL`] in front (for `p9_saturate`,
/// `P9_PI` and `P9_PI2`).
pub const P9_SHADING_WGSL: &str = r#"
// vec3 _rand3(float p) — the three-lane hash Boxy places its cubes with.
fn p9_rand3(p: f32) -> vec3<f32> {
  var p2 = fract(vec3<f32>(p * 5.3987, p * 5.4421, p * 6.9371));
  p2 = p2 + vec3<f32>(dot(p2.yzx, p2.xyz + vec3<f32>(21.5351, 14.3137, 15.3247)));
  return fract(vec3<f32>(p2.x * p2.z * 95.4307, p2.x * p2.y * 97.5901, p2.y * p2.z * 93.8369));
}

// GLSL mod(): the result takes the sign of the divisor, unlike WGSL's `%`.
fn p9_mod(x: f32, y: f32) -> f32 {
  return x - y * floor(x / y);
}

// vec3 _palette(float t, vec3 brightness, vec3 contrast, vec3 a, vec3 b)
fn p9_palette(
  t: f32,
  brightness: vec3<f32>,
  contrast: vec3<f32>,
  a: vec3<f32>,
  b: vec3<f32>,
) -> vec3<f32> {
  return brightness + contrast * cos(P9_PI2 * (a * t + b));
}

// vec4 _texturePanoramicLod(sampler2D tex, vec3 v, float lod)
//
// The equirectangular lookup the Plane9 graph uses for a 2D panorama: +x is
// the seam, +y is the zenith.
//
// The GLSL source is `vec2((1 + atan(v.x, -v.z) / PI) * 0.5,
// 1 - acos(v.y) / PI)` — its v assumes three's default `flipY = true`, which
// puts the image's top row at v = 1. [`super::assets`] uploads images
// unflipped (row 0 = top = v 0), so the port inverts v: the zenith lands on the
// image's top row in both implementations.
fn p9_panoramic_uv(v: vec3<f32>) -> vec2<f32> {
  let up = clamp(v.y, -1.0, 1.0);
  return vec2<f32>(
    (1.0 + atan2(v.x, -v.z) / P9_PI) * 0.5,
    acos(up) / P9_PI,
  );
}

fn p9_texture_panoramic_lod(
  tex: texture_2d<f32>,
  samp: sampler,
  v: vec3<f32>,
  lod: f32,
) -> vec4<f32> {
  return textureSampleLevel(tex, samp, p9_panoramic_uv(v), lod);
}

// vec3 _fresnelRoughness(vec3 specularColor, float roughness, vec3 h, vec3 viewDir)
fn p9_fresnel_roughness(
  specular_color: vec3<f32>,
  roughness: f32,
  h: vec3<f32>,
  view_dir: vec3<f32>,
) -> vec3<f32> {
  return specular_color
    + (max(vec3<f32>(1.0 - (roughness * roughness)), specular_color) - specular_color)
      * pow(1.0 - p9_saturate(dot(view_dir, h)), 5.0);
}

// vec3 _shade(diffuseAlbedo, specularAlbedo, normal, roughness, lightColor,
//             lightDir, viewDir) — GGX specular + Lambert diffuse, plane9's
// distribution/visibility pair.
fn p9_shade(
  diffuse_albedo: vec3<f32>,
  specular_albedo: vec3<f32>,
  normal: vec3<f32>,
  roughness: f32,
  light_color: vec3<f32>,
  light_dir: vec3<f32>,
  view_dir: vec3<f32>,
) -> vec3<f32> {
  let h = normalize(view_dir + light_dir);
  let n_dot_h = p9_saturate(dot(normal, h));
  let n_dot_l = p9_saturate(dot(normal, light_dir));
  let n_dot_v = p9_saturate(dot(normal, view_dir)) + 0.0001;
  let a = max(0.001, roughness * roughness);
  let a2 = a * a;
  let denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
  let d = a2 / (P9_PI * denominator * denominator);
  let v1i = 1.0 / (n_dot_l + sqrt(a + (1.0 - a) * n_dot_l * n_dot_l));
  let v1o = 1.0 / (n_dot_v + sqrt(a + (1.0 - a) * n_dot_v * n_dot_v));
  let vis = v1i * v1o;
  let specular = d * vis;
  let fresnel = specular_albedo
    + (vec3<f32>(1.0) - specular_albedo) * pow(1.0 - p9_saturate(dot(light_dir, h)), 5.0);
  return (diffuse_albedo * (1.0 / P9_PI) + specular * fresnel) * n_dot_l * light_color;
}

// vec3 _lightDirectional(diffuseAlbedo, specularAlbedo, normal, roughness,
//                        lightColor, lightDir, viewDir)
fn p9_light_directional(
  diffuse_albedo: vec3<f32>,
  specular_albedo: vec3<f32>,
  normal: vec3<f32>,
  roughness: f32,
  light_color: vec3<f32>,
  light_dir: vec3<f32>,
  view_dir: vec3<f32>,
) -> vec3<f32> {
  var lighting = vec3<f32>(0.0);
  let n_dot_l = p9_saturate(dot(normal, light_dir));
  if (n_dot_l > 0.0) {
    lighting = p9_shade(
      diffuse_albedo,
      specular_albedo,
      normal,
      roughness,
      light_color,
      light_dir,
      view_dir,
    );
  }
  return max(lighting, vec3<f32>(0.0));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::shared::p9_glsl::P9_COMMON_WGSL;

    /// A program that calls every helper, so naga type-checks each body.
    fn composed_program() -> String {
        format!(
            "{P9_COMMON_WGSL}\n{P9_SHADING_WGSL}\n\
             @group(0) @binding(0) var pano: texture_2d<f32>;\n\
             @group(0) @binding(1) var pano_sampler: sampler;\n\
             @fragment\n\
             fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {{\n\
               let p = vec3<f32>(position.x * 0.01, position.y * 0.01, 0.5);\n\
               let n = normalize(p);\n\
               let albedo = p9_rand3(p.x);\n\
               let tint = p9_palette(p.x, vec3<f32>(0.5), vec3<f32>(0.1), vec3<f32>(1.0), vec3<f32>(0.2, 0.1, 0.7));\n\
               var c = p9_light_directional(albedo, tint, n, 0.4, vec3<f32>(10.0), vec3<f32>(0.0, 0.0, -1.0), n);\n\
               c = c + p9_texture_panoramic_lod(pano, pano_sampler, n, p9_mod(p.x, 20.0)).rgb;\n\
               c = c + p9_fresnel_roughness(tint, 0.4, n, n) * albedo;\n\
               return vec4<f32>(p9_saturate3(c), 1.0);\n\
             }}\n"
        )
    }

    #[test]
    fn shading_wgsl_parses_and_validates() {
        let program = composed_program();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("P9_SHADING_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("P9_SHADING_WGSL must validate");
    }

    #[test]
    fn shading_wgsl_keeps_the_ported_formulas() {
        for needle in [
            "fn p9_rand3(p: f32) -> vec3<f32>",
            "p * 5.3987, p * 5.4421, p * 6.9371",
            "p2.x * p2.z * 95.4307, p2.x * p2.y * 97.5901, p2.y * p2.z * 93.8369",
            "fn p9_mod(x: f32, y: f32) -> f32",
            "x - y * floor(x / y)",
            "brightness + contrast * cos(P9_PI2 * (a * t + b))",
            "(1.0 + atan2(v.x, -v.z) / P9_PI) * 0.5",
            "acos(up) / P9_PI",
            "max(vec3<f32>(1.0 - (roughness * roughness)), specular_color)",
            "let a = max(0.001, roughness * roughness);",
            "(diffuse_albedo * (1.0 / P9_PI) + specular * fresnel) * n_dot_l * light_color",
            "fn p9_light_directional(",
        ] {
            assert!(P9_SHADING_WGSL.contains(needle), "lost {needle}");
        }
    }

    #[test]
    fn glsl_mod_keeps_the_divisor_sign() {
        // GLSL `mod(-3, 20)` = 17, WGSL `%` would give -3.
        let glsl_mod = |x: f32, y: f32| x - y * (x / y).floor();
        assert_eq!(glsl_mod(-3.0, 20.0), 17.0);
        assert_eq!(glsl_mod(23.0, 20.0), 3.0);
        assert_eq!(glsl_mod(0.0, 20.0), 0.0);
        assert_eq!(glsl_mod(20.0, 20.0), 0.0);
    }

    #[test]
    fn panoramic_uv_puts_the_seam_and_the_zenith_where_plane9_does() {
        let uv = |v: [f32; 3]| {
            let y = v[1].clamp(-1.0, 1.0);
            [
                (1.0 + v[0].atan2(-v[2]) / std::f32::consts::PI) * 0.5,
                y.acos() / std::f32::consts::PI,
            ]
        };

        // Forward (-z) is the centre of the panorama.
        let forward = uv([0.0, 0.0, -1.0]);
        assert!((forward[0] - 0.5).abs() < 1e-5);
        assert!((forward[1] - 0.5).abs() < 1e-5);

        // The zenith is the image's top row (v = 0 for an unflipped upload).
        assert!(uv([0.0, 1.0, 0.0])[1].abs() < 1e-5);
        assert!((uv([0.0, -1.0, 0.0])[1] - 1.0).abs() < 1e-5);

        // +x and -x are the two edges.
        assert!((uv([1.0, 0.0, 0.0])[0] - 0.75).abs() < 1e-5);
        assert!((uv([-1.0, 0.0, 0.0])[0] - 0.25).abs() < 1e-5);
    }
}
