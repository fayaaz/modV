//! Wobble — port of `modules/Wobble.js` and `modules/Wobble/wobble.frag`
//! (`gl_FragColor` flavour). From <https://getmosh.io/>.
//!
//! Sinusoidal UV wobble driven by `iGlobalTime`, scaled by the `strength`
//! and `size` props.
//!
//! GLSL source anchors: the `.frag` body, `wobble.frag:13-21`.

#![forbid(unsafe_code)]

use crate::shader::default_shader::direct_fragment;
use crate::shader::modules::{ShaderModule, fragment_with_props};
use crate::shader::uniforms::{PropKind, ShaderProp};

// Port of wobble.frag:10-21. The GLSL module-scope `float speed = 1.0;`
// becomes a typed const.
const BODY: &str = r#"const SPEED: f32 = 1.0;

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let vUv = modv_uv(position.xy);
  let p = -1.0 + 2.0 * vUv;
  // Port of: texture2D(u_modVCanvas, vUv + strength * vec2(cos(...), sin(...)))
  return textureSample(
    u_modVCanvas,
    modvGroupSampler,
    vUv + props.strength.x * vec2<f32>(
      cos(u.iGlobalTime * SPEED + length(p * props.size.x)),
      sin(u.iGlobalTime * SPEED + length(p * props.size.x))
    )
  );
}
"#;

pub(crate) fn module() -> ShaderModule {
    let props = vec![
        ShaderProp {
            name: "strength",
            kind: PropKind::Float,
            label: "Float",
            min: 0.0,
            max: 0.05,
            step: 0.001,
            default: 0.001,
        },
        ShaderProp {
            name: "size",
            kind: PropKind::Float,
            label: "Size",
            min: 1.0,
            max: 50.0,
            step: 1.0,
            default: 1.0,
        },
    ];
    ShaderModule {
        name: "Wobble",
        fragment_wgsl: direct_fragment(&fragment_with_props(BODY, &props)),
        meta_author: "2xAA",
        meta_version: "1.0.0",
        category: None,
        audio: false,
        audio_features: &[],
        preview_with_output: true,
        props,
    }
}
