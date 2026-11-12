//! Plasma — port of `modules/Plasma.js` and `modules/Plasma/plasma.frag`
//! (`gl_FragColor` flavour).
//!
//! Four interfering sine fields, scaled per prop.
//!
//! GLSL source anchors: the `.frag` body, `plasma.frag:13-25`.

#![forbid(unsafe_code)]

use crate::shader::default_shader::direct_fragment;
use crate::shader::modules::{ShaderModule, fragment_with_props};
use crate::shader::uniforms::{PropKind, ShaderProp};

// Port of plasma.frag:4-25. The `#define PI ...` maps to a typed const.
const BODY: &str = r#"const PI: f32 = 3.1415926535897932384626433832795;

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let vUv = modv_uv(position.xy);
  let time = u.u_time / props.u_timeScale.x;
  let u_scale = vec2<f32>(u.iResolution.x / props.u_scaleX.x, u.iResolution.y / props.u_scaleY.x);
  var v = 0.0;
  var c = vUv * u_scale - u_scale / 2.0;
  v += sin((c.x + time));
  v += sin((c.y + time) / 2.0);
  v += sin((c.x + c.y + time) / 2.0);
  c += u_scale / 2.0 * vec2<f32>(sin(time / 3.0), cos(time / 2.0));
  v += sin(sqrt(c.x * c.x + c.y * c.y + 1.0) + time);
  v = v / 2.0;
  let col = vec3<f32>(1.0, sin(PI * v), cos(PI * v));
  // Port of: gl_FragColor = vec4(col*.5 + .5, 1);
  return vec4<f32>(col * 0.5 + 0.5, 1.0);
}
"#;

pub(crate) fn module() -> ShaderModule {
    let props = vec![
        ShaderProp {
            name: "u_scaleX",
            kind: PropKind::Float,
            label: "Scale X",
            min: 1.0,
            max: 150.0,
            step: 1.0,
            default: 50.0,
        },
        ShaderProp {
            name: "u_scaleY",
            kind: PropKind::Float,
            label: "Scale Y",
            min: 1.0,
            max: 150.0,
            step: 1.0,
            default: 50.0,
        },
        ShaderProp {
            name: "u_timeScale",
            kind: PropKind::Float,
            label: "Time Scale",
            min: 1.0,
            max: 1000.0,
            step: 1.0,
            default: 100.0,
        },
    ];
    ShaderModule {
        name: "Plasma",
        fragment_wgsl: direct_fragment(&fragment_with_props(BODY, &props)),
        meta_author: "2xAA",
        meta_version: "0.1",
        category: None,
        audio: false,
        audio_features: &[],
        preview_with_output: false,
        props,
    }
}
