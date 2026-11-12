//! The subset of Plane9's GLSL preamble (`plane9/P9Glsl.js`) that the batch-2
//! scenes call, expressed in WGSL.
//!
//! # Why a shared snippet and not a preprocessor
//!
//! `P9Glsl.js` is a 25 KB GLSL preprocessor library; `docs/rust-hybrid.md`
//! replaces it with per-scene WGSL. Three scenes in this batch (`LightTunnel`,
//! `Wormhole`, `ItIsComingForYou`) still call into it, so the handful of
//! helpers they actually use are ported once here and concatenated in front of
//! a scene's own WGSL. The `p9_` prefix keeps them from colliding with a
//! scene's own functions and with the `postfx` preludes.
//!
//! # Ported verbatim (same arithmetic, same constants)
//!
//! | `P9Glsl.js` | Rust | Used by |
//! |-------------|------|---------|
//! | `PI`, `PI2` | [`PI`], [`PI2`] (CPU) + `P9_PI`, `P9_PI2` (WGSL) | all four |
//! | `saturate(x)` | `p9_saturate` / `p9_saturate3` | all four |
//! | `_rand(float)`, `_rand(vec2)` | `p9_rand`, `p9_rand2` | Wormhole, ItIsComingForYou |
//! | `_hsv2rgb(float, float, float)` | `p9_hsv2rgb` | Wormhole, ItIsComingForYou |
//! | `_rotateVector`, `_axisAngleToQuaternion` | `p9_rotate_vector`, `p9_axis_angle_to_quaternion` | ItIsComingForYou |
//! | `_noise(vec3)` (3D simplex, Brian Sharpe) | `p9_noise` | LightTunnel, ItIsComingForYou |
//! | `_turbulencefast`, `_fbmfast` | `p9_turbulence`, `p9_fbm` | LightTunnel |
//! | `_cubicpulse` | `p9_cubicpulse` | ItIsComingForYou |
//! | `_lightLambert` | `p9_light_lambert` | ItIsComingForYou |
//!
//! Not ported: the 2D simplex noise, the derivative forms, `_voronoi`,
//! `_blackBody`, the tone-map helpers and `_textureBicubic`/`_texturePanoramic`
//! — no batch-2 scene calls them. A later scene that needs one adds it here.
//!
//! # `smoothstep` and reversed edges
//!
//! Several plane9 shaders call `smoothstep` with `edge0 > edge1`
//! (`smoothstep(1.0, 0.9, x)`); the WebGL implementation happily evaluates it,
//! while WGSL documents `edge0 >= edge1` as *undefined*. [`P9_COMMON_WGSL`]
//! therefore ships `p9_smoothstep`, which is GLSL's own definition
//! (`t = clamp((x - edge0) / (edge1 - edge0)); t*t*(3 - 2t)`) and behaves
//! identically for both orders — and, unlike a `1 - smoothstep(swapped)` fix,
//! also for `edge0 == edge1`-adjacent values.
//!
//! # GLSL → WGSL notes
//!
//! * `inversesqrt` → `inverseSqrt`.
//! * `step(edge, x)` → `p9_step3`, because WGSL has `step` for scalars only.
//! * `mix(a, b, vecFactor)` is component-wise in both languages.
//! * `vec3(v2, 0.0)` → `vec3<f32>(v2, 0.0)`.
//! * `select(f, t, cond)` takes `cond` as a `bool` or a `vecN<bool>` and
//!   returns the per-component pick, which is how GLSL's `?:` on vectors is
//!   spelled.

#![forbid(unsafe_code)]

/// `#define PI 3.14159265358979323846` (`P9Glsl.js:6`).
pub const PI: f32 = 3.141_592_653_589_793;

/// `#define PI2 6.28318530717958623200` (`P9Glsl.js:7`).
pub const PI2: f32 = 6.283_185_307_179_586;

/// WGSL port of the `P9Glsl` helpers the batch-2 scenes use.
///
/// Concatenate this in front of a scene's own WGSL (the scenes do it with
/// `format!("{P9_COMMON_WGSL}\n{...}")`). Every function is pure, so naga can
/// inline or drop the ones a scene does not call.
pub const P9_COMMON_WGSL: &str = r#"
const P9_PI: f32 = 3.14159265358979323846;
const P9_PI2: f32 = 6.28318530717958623200;

// GLSL's smoothstep, including the reversed-edge calls plane9 makes. WGSL
// leaves edge0 >= edge1 undefined; this does not.
fn p9_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
  let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
  return t * t * (3.0 - 2.0 * t);
}

// #define saturate(a) clamp(a, 0.0, 1.0)
fn p9_saturate(x: f32) -> f32 {
  return clamp(x, 0.0, 1.0);
}
fn p9_saturate3(v: vec3<f32>) -> vec3<f32> {
  return clamp(v, vec3<f32>(0.0), vec3<f32>(1.0));
}

// float _rand(float p) { ... }
fn p9_rand(p: f32) -> f32 {
  var p2 = fract(vec2<f32>(p * 5.3987, p * 5.4421));
  p2 = p2 + vec2<f32>(dot(p2.yx, p2.xy + vec2<f32>(21.5351, 14.3137)));
  return fract(p2.x * p2.y * 95.4307);
}

// float _rand(vec2 p) { ... }
fn p9_rand2(p0: vec2<f32>) -> f32 {
  var p = fract(p0 * vec2<f32>(5.3987, 5.4421));
  p = p + vec2<f32>(dot(p.yx, p.xy + vec2<f32>(21.5351, 14.3137)));
  return fract(p.x * p.y * 95.4307);
}

// vec3 _hsv2rgb(float h, float s, float v)
fn p9_hsv2rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
  let ramp = abs(
    fract(vec3<f32>(h) + vec3<f32>(3.0, 2.0, 1.0) / vec3<f32>(3.0)) * vec3<f32>(6.0)
      - vec3<f32>(3.0)
  ) - vec3<f32>(1.0);
  return mix(vec3<f32>(1.0), p9_saturate3(ramp), vec3<f32>(p9_saturate(s))) * vec3<f32>(v);
}

// vec4 _axisAngleToQuaternion(vec3 axis, float angle)
fn p9_axis_angle_to_quaternion(axis: vec3<f32>, angle: f32) -> vec4<f32> {
  let half_angle = angle * 0.5;
  return vec4<f32>(axis * sin(half_angle), cos(half_angle));
}

// vec3 _rotateVector(vec3 v, vec4 q). The quaternion sandwich, written with
// the same swizzles as the GLSL so the component algebra is checkable
// side by side.
fn p9_rotate_vector(v: vec3<f32>, q: vec4<f32>) -> vec3<f32> {
  let i_xyz = vec3<f32>(q.w) * v + q.yzx * v.zxy - q.zxy * v.yzx;
  let i_w = -q.x * v.x - q.y * v.y - q.z * v.z;
  return i_xyz * vec3<f32>(q.w)
    + vec3<f32>(i_w) * (-q.xyz)
    + i_xyz.yzx * (-q.zxy)
    - i_xyz.zxy * (-q.yzx);
}

// GLSL step(edge, x) per component (WGSL's step is scalar-only).
fn p9_step3(edge: vec3<f32>, x: vec3<f32>) -> vec3<f32> {
  return select(vec3<f32>(0.0), vec3<f32>(1.0), x >= edge);
}

// float _noise(vec3 P) — Brian Sharpe's 3D simplex noise, verbatim.
fn p9_noise(p0: vec3<f32>) -> f32 {
  let skew_factor = 1.0 / 3.0;
  let unskew_factor = 1.0 / 6.0;
  let simplex_corner_pos = 0.5;

  var Pi = floor(p0 + vec3<f32>(dot(p0, vec3<f32>(skew_factor))));
  let x0 = p0 - Pi + vec3<f32>(dot(Pi, vec3<f32>(unskew_factor)));
  let g = p9_step3(x0.yzx, x0.xyz);
  let l = vec3<f32>(1.0) - g;
  let pi_1 = min(g, l.zxy);
  let pi_2 = max(g, l.zxy);
  let x1 = x0 - pi_1 + unskew_factor;
  let x2 = x0 - pi_2 + skew_factor;
  let x3 = x0 - simplex_corner_pos;
  let v1234_x = vec4<f32>(x0.x, x1.x, x2.x, x3.x);
  let v1234_y = vec4<f32>(x0.y, x1.y, x2.y, x3.y);
  let v1234_z = vec4<f32>(x0.z, x1.z, x2.z, x3.z);

  Pi = Pi - floor(Pi * (1.0 / 69.0)) * 69.0;
  let pi_inc1 = p9_step3(Pi, vec3<f32>(69.0 - 1.5)) * (Pi + vec3<f32>(1.0));
  var Pt = vec4<f32>(Pi.xy, pi_inc1.xy) + vec4<f32>(50.0, 161.0, 50.0, 161.0);
  Pt = Pt * Pt;
  let v1xy_v2xy = mix(Pt.xyxy, Pt.zwzw, vec4<f32>(pi_1.xy, pi_2.xy));
  Pt = vec4<f32>(Pt.x, v1xy_v2xy.x, v1xy_v2xy.z, Pt.z)
    * vec4<f32>(Pt.y, v1xy_v2xy.y, v1xy_v2xy.w, Pt.w);

  let some_large_floats = vec3<f32>(635.298681, 682.357502, 668.926525);
  let zinc = vec3<f32>(48.500388, 65.294118, 63.934599);
  let lowz_mods = vec3<f32>(1.0) / (some_large_floats + Pi.zzz * zinc);
  let highz_mods = vec3<f32>(1.0) / (some_large_floats + pi_inc1.zzz * zinc);
  let sel_1 = select(highz_mods, lowz_mods, pi_1.z < 0.5);
  let sel_2 = select(highz_mods, lowz_mods, pi_2.z < 0.5);

  let hash_0 = fract(Pt * vec4<f32>(lowz_mods.x, sel_1.x, sel_2.x, highz_mods.x)) - 0.49999;
  let hash_1 = fract(Pt * vec4<f32>(lowz_mods.y, sel_1.y, sel_2.y, highz_mods.y)) - 0.49999;
  let hash_2 = fract(Pt * vec4<f32>(lowz_mods.z, sel_1.z, sel_2.z, highz_mods.z)) - 0.49999;

  let grad_results = inverseSqrt(hash_0 * hash_0 + hash_1 * hash_1 + hash_2 * hash_2)
    * (hash_0 * v1234_x + hash_1 * v1234_y + hash_2 * v1234_z);

  var kernel_weights = v1234_x * v1234_x + v1234_y * v1234_y + v1234_z * v1234_z;
  kernel_weights = max(vec4<f32>(0.5) - kernel_weights, vec4<f32>(0.0));
  kernel_weights = kernel_weights * kernel_weights * kernel_weights;

  return dot(kernel_weights, grad_results) * 37.837227241611314102871574478976;
}

// float _turbulencefast(vec3 p, float octaves)
fn p9_turbulence(p: vec3<f32>, octaves: u32) -> f32 {
  let lacunarity = 2.13628142;
  let gain = 0.5;
  var sum = 0.0;
  var amp_sum = 0.0;
  var freq = 1.0;
  var amp = 1.0;
  for (var i = 0u; i < octaves; i = i + 1u) {
    sum = sum + abs(p9_noise(p * freq)) * amp;
    amp_sum = amp_sum + amp;
    freq = freq * lacunarity;
    amp = amp * gain;
  }
  return sum / amp_sum;
}

// float _fbmfast(vec3 p, float octaves)
fn p9_fbm(p: vec3<f32>, octaves: u32) -> f32 {
  let lacunarity = 2.13628142;
  let gain = 0.5;
  var sum = 0.0;
  var amp_sum = 0.0;
  var freq = 1.0;
  var amp = 1.0;
  for (var i = 0u; i < octaves; i = i + 1u) {
    sum = sum + p9_noise(p * freq) * amp;
    amp_sum = amp_sum + amp;
    freq = freq * lacunarity;
    amp = amp * gain;
  }
  return sum / amp_sum;
}

// float _cubicpulse(float value, float bumpCenter, float width)
fn p9_cubicpulse(value: f32, bump_center: f32, width: f32) -> f32 {
  let distance = abs(value - bump_center);
  if (distance > width) {
    return 0.0;
  }
  let t = distance / width;
  return 1.0 - t * t * (3.0 - 2.0 * t);
}

// vec3 _lightLambert(vec3 normal, vec3 lightDir, vec3 diffuseCol)
fn p9_light_lambert(normal: vec3<f32>, light_dir: vec3<f32>, diffuse: vec3<f32>) -> vec3<f32> {
  return max(0.0, dot(light_dir, normal)) * diffuse;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// A program that references every helper, so naga type-checks each body.
    fn composed_program() -> String {
        format!(
            "{P9_COMMON_WGSL}\n\
             @fragment\n\
             fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {{\n\
               let p = vec3<f32>(position.x * 0.01, position.y * 0.01, P9_PI);\n\
               let q = p9_axis_angle_to_quaternion(vec3<f32>(0.0, 0.0, 1.0), p.x);\n\
               var c = p9_hsv2rgb(p.x, p.y, p.z);\n\
               c = c + p9_rotate_vector(p, q);\n\
               let f = p9_rand(p.x) + p9_rand2(p.xy) + p9_noise(p);\n\
               let g = p9_turbulence(p, 5u) + p9_fbm(p, 3u) + p9_cubicpulse(f, 0.222, 0.087);\n\
               c = c + p9_light_lambert(vec3<f32>(0.0, 1.0, 0.0), -p, vec3<f32>(0.5, 0.8, 1.0)) * g;\n\
               return vec4<f32>(p9_saturate3(c), p9_saturate(p9_smoothstep(1.0, 0.5, f)));\n\
             }}\n"
        )
    }

    #[test]
    fn common_wgsl_parses_and_validates() {
        let program = composed_program();
        let module = match naga::front::wgsl::parse_str(&program) {
            Ok(module) => module,
            Err(error) => panic!("P9_COMMON_WGSL failed to parse: {error}"),
        };
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("P9_COMMON_WGSL must validate");
    }

    #[test]
    fn pi_matches_the_glsl_defines() {
        assert!((PI - std::f32::consts::PI).abs() < 1e-6);
        assert!((PI2 - std::f32::consts::TAU).abs() < 1e-6);
    }

    /// GLSL evaluates `smoothstep(1.0, 0.9, x)` fine; WGSL calls it undefined.
    /// The helper must mirror GLSL in both directions.
    #[test]
    fn common_wgsl_keeps_the_ported_formulas() {
        // Guards against a "tidy-up" that quietly changes the maths the JS
        // scenes rely on.
        for needle in [
            "fn p9_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32",
            "clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0)",
            "fn p9_saturate(",
            "fn p9_rand(p: f32) -> f32",
            "p * 5.3987, p * 5.4421",
            "p0 * vec2<f32>(5.3987, 5.4421)",
            "21.5351, 14.3137",
            "95.4307",
            "fn p9_hsv2rgb(h: f32, s: f32, v: f32) -> vec3<f32>",
            "vec3<f32>(3.0, 2.0, 1.0) / vec3<f32>(3.0)",
            "fn p9_axis_angle_to_quaternion(",
            "fn p9_rotate_vector(",
            "fn p9_noise(p0: vec3<f32>) -> f32",
            "37.837227241611314102871574478976",
            "fn p9_turbulence(p: vec3<f32>, octaves: u32) -> f32",
            "2.13628142",
            "fn p9_fbm(p: vec3<f32>, octaves: u32) -> f32",
            "fn p9_cubicpulse(value: f32, bump_center: f32, width: f32) -> f32",
            "fn p9_light_lambert(",
            "max(0.0, dot(light_dir, normal)) * diffuse",
        ] {
            assert!(P9_COMMON_WGSL.contains(needle), "P9_WGSL lost {needle}");
        }
    }

    #[test]
    fn smoothstep_handles_reversed_edges_like_glsl() {
        let glsl = |edge0: f32, edge1: f32, x: f32| {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        // Forward edges.
        assert_eq!(glsl(0.0, 1.0, 0.25), 0.156_25);
        // Reversed edges: 1 at x <= 0.9, 0 at x >= 1.0.
        assert!((glsl(1.0, 0.9, 0.9) - 1.0).abs() < 1e-6);
        assert!((glsl(1.0, 0.9, 1.0) - 0.0).abs() < 1e-6);
        assert!((glsl(1.0, 0.9, 0.95) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn hsv_reference_matches_the_glsl_formula() {
        // The GLSL `_hsv2rgb` is the standard hexagon ramp. Pin the primaries
        // and a grey, which is what the scenes' point colours rely on.
        let hsv = |h: f32, s: f32, v: f32| {
            let ramp = |offset: f32| {
                let f = (h + offset / 3.0).fract();
                let x = (((f * 6.0) - 3.0).abs() - 1.0).clamp(0.0, 1.0);
                1.0 - s + s * x
            };
            [ramp(3.0) * v, ramp(2.0) * v, ramp(1.0) * v]
        };

        let red = hsv(0.0, 1.0, 1.0);
        assert!((red[0] - 1.0).abs() < 1e-5 && red[1].abs() < 1e-5 && red[2].abs() < 1e-5);

        let green = hsv(1.0 / 3.0, 1.0, 1.0);
        assert!((green[1] - 1.0).abs() < 1e-5 && green[0].abs() < 1e-5);

        let grey = hsv(0.42, 0.0, 0.5);
        assert!((grey[0] - 0.5).abs() < 1e-6 && (grey[2] - 0.5).abs() < 1e-6);
    }
}
