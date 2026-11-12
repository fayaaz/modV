//! Fisheye — port of `modules/Fisheye.js` and
//! `modules/Fisheye/fisheye.frag` (`gl_FragColor` flavour).
//!
//! Inverse angular-fisheye transform ("à la Bourke"), as explained on
//! <http://paulbourke.net/miscellaneous/domefisheye/fisheye/>.
//!
//! GLSL source anchors: the `.frag` body, `fisheye.frag:17-47`.
//!
//! # Uniform-control-flow note
//!
//! The texture fetch sits inside the `else` of `if (l > 1.0)`, i.e. in
//! non-uniform control flow; implicit-derivative sampling is undefined there
//! and naga's uniformity validator rejects it. The port uses
//! `textureSampleLevel(.., 0.0)` — explicit LOD 0, which is what the GLSL
//! texture (no mipmaps) always sampled anyway.

#![forbid(unsafe_code)]

use crate::shader::default_shader::direct_fragment;
use crate::shader::modules::{ShaderModule, fragment_with_props};
use crate::shader::uniforms::{PropKind, ShaderProp};

// Port of fisheye.frag:10-46 (main body). aperture arrives in degrees.
const BODY: &str = r#"const PI: f32 = 3.1415926535;

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let vUv = modv_uv(position.xy);
  let apertureHalf = 0.5 * props.aperture.x * (PI / 180.0);

  // This factor adjusts the coordinates in the case that the aperture angle
  // is less than 180 degrees, in which case the area displayed is not the
  // entire half-sphere.
  let maxFactor = sin(apertureHalf);

  let pos = 2.0 * vUv - 1.0;

  let l = length(pos);
  var outColor = vec4<f32>(0.0, 0.0, 0.0, 1.0);
  if (l > 1.0) {
    outColor = vec4<f32>(0.0, 0.0, 0.0, 1.0);
  } else {
    let x = maxFactor * pos.x;
    let y = maxFactor * pos.y;

    let n = length(vec2<f32>(x, y));

    let z = sqrt(1.0 - n * n);

    let r = atan2(n, z) / PI;

    let phi = atan2(y, x);

    let u = r * cos(phi) + 0.5;
    let v = r * sin(phi) + 0.5;

    outColor = textureSampleLevel(u_modVCanvas, modvGroupSampler, vec2<f32>(u, v), 0.0);
  }
  return outColor;
}
"#;

pub(crate) fn module() -> ShaderModule {
    let props = vec![ShaderProp {
        name: "aperture",
        kind: PropKind::Float,
        label: "Aperture",
        min: 1.0,
        max: 360.0,
        step: 0.5,
        default: 180.0,
    }];
    ShaderModule {
        name: "Fisheye",
        fragment_wgsl: direct_fragment(&fragment_with_props(BODY, &props)),
        meta_author: "???",
        meta_version: "1.0.0",
        category: None,
        audio: false,
        audio_features: &[],
        preview_with_output: true,
        props,
    }
}
