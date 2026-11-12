//! MattiasCRT — port of `modules/MattiasCRT-2.0.js` and
//! `modules/MattiasCRT/mattiasCrt.frag` (`gl_FragColor` flavour).
//!
//! From <https://www.shadertoy.com/view/Ms23DR>; loosely based on
//! post-processing by Iñigo Quílez (CC-BY-NC-SA 3.0).
//!
//! GLSL source anchors: the `.frag` body, `mattiasCrt.frag:11-62`.
//! The JS registration name is "MattiasCRT" — the file is `MattiasCRT-2.0.js`
//! and the version metadata is 1.0.0.
//!
//! `fragCoord` is the wgpu top-left fragment position (the GL reference
//! uses bottom-left `gl_FragCoord`); the scanline parity pattern observed
//! through `mod(fragCoord.x, 2.0)` therefore flips by one column.

#![forbid(unsafe_code)]

use crate::shader::default_shader::direct_fragment;
use crate::shader::modules::{ShaderModule, fragment_with_props};

// Port of mattiasCrt.frag:11-62 (body). The scalar GLSL `mod` helper is
// spliced in from the shared const module.
const BODY: &str = r#"fn curve_uv(uv_in: vec2<f32>) -> vec2<f32> {
  var uv = (uv_in - 0.5) * 2.0;
  uv *= 1.1;
  uv.x *= 1.0 + pow((abs(uv.y) / 5.0), 2.0);
  uv.y *= 1.0 + pow((abs(uv.x) / 4.0), 2.0);
  uv = (uv / 2.0) + 0.5;
  uv = uv * 0.92 + 0.04;
  return uv;
}

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let vUv = modv_uv(position.xy);

  var uv = vUv;
  uv = curve_uv(uv);
  // The GLSL `oricol` capture is intentionally dropped: its only consumer
  // (`col = mix(col, oricol, comp)`) was already commented out upstream.
  var col: vec3<f32>;
  let x = sin(0.3 * u.u_delta + uv.y * 21.0) * sin(0.7 * u.u_delta + uv.y * 29.0) * sin(0.3 + 0.33 * u.u_delta + uv.y * 31.0) * 0.0017;

  col.r = textureSample(u_modVCanvas, modvGroupSampler, vec2<f32>(x + uv.x + 0.001, uv.y + 0.001)).x + 0.05;
  col.g = textureSample(u_modVCanvas, modvGroupSampler, vec2<f32>(x + uv.x + 0.000, uv.y - 0.002)).y + 0.05;
  col.b = textureSample(u_modVCanvas, modvGroupSampler, vec2<f32>(x + uv.x - 0.002, uv.y + 0.000)).z + 0.05;
  col.r += 0.08 * textureSample(u_modVCanvas, modvGroupSampler, 0.75 * vec2<f32>(x + 0.025, -0.027) + vec2<f32>(uv.x + 0.001, uv.y + 0.001)).x;
  col.g += 0.05 * textureSample(u_modVCanvas, modvGroupSampler, 0.75 * vec2<f32>(x + -0.022, -0.02) + vec2<f32>(uv.x + 0.000, uv.y - 0.002)).y;
  col.b += 0.08 * textureSample(u_modVCanvas, modvGroupSampler, 0.75 * vec2<f32>(x + -0.02, -0.018) + vec2<f32>(uv.x - 0.002, uv.y + 0.000)).z;

  col = clamp(col * 0.6 + 0.4 * col * col * 1.0, vec3<f32>(0.0), vec3<f32>(1.0));

  let vig = (0.0 + 1.0 * 16.0 * uv.x * uv.y * (1.0 - uv.x) * (1.0 - uv.y));
  col *= vec3<f32>(pow(vig, 0.3));

  col *= vec3<f32>(0.95, 1.05, 0.95);
  col *= 2.8;

  let scans = clamp(0.35 + 0.35 * sin(3.5 * u.u_delta + uv.y * u.iResolution.y * 1.5), 0.0, 1.0);

  let s = pow(scans, 1.7);
  col = col * vec3<f32>(0.4 + 0.7 * s);

  col *= 1.0 + 0.01 * sin(110.0 * u.u_delta);
  if (uv.x < 0.0 || uv.x > 1.0) {
    col *= 0.0;
  }
  if (uv.y < 0.0 || uv.y > 1.0) {
    col *= 0.0;
  }

  // fragCoord.x in the GL reference is the bottom-left-origin gl_FragCoord;
  // wgpu's position is top-left (see the module docs).
  col *= 1.0 - 0.65 * vec3<f32>(clamp((glsl_mod_f(position.x, 2.0) - 1.0) * 2.0, 0.0, 1.0));

  // The GLSL cross-fade (`col = mix(col, oricol, comp)`, with its
  // `smoothstep(0.1, 0.9, sin(u_delta))` term) was already commented out
  // upstream; not carried into the port.
  // Port of: gl_FragColor = vec4(col, 1.0);
  return vec4<f32>(col, 1.0);
}
"#;

pub(crate) fn module() -> ShaderModule {
    ShaderModule {
        name: "MattiasCRT",
        fragment_wgsl: direct_fragment(&fragment_with_props(
            &format!("{}\n{BODY}", crate::shader::uniforms::GLSL_MOD_F_WGSL),
            &[],
        )),
        meta_author: "Mattias",
        meta_version: "1.0.0",
        category: None,
        audio: false,
        audio_features: &[],
        preview_with_output: true,
        props: Vec::new(),
    }
}
