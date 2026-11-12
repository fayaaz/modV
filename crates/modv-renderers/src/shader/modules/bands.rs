//! Bands — port of `modules/Bands.js` (inline GLSL, `gl_FragColor` flavour).
//!
//! Ring radii come from the AGC-normalised `u_bass`/`u_mid`/`u_treble`
//! bands; the ring width is the module's one prop, `u_ringWidth` (default
//! 0.05, min 0.01, max 0.5, step 0.01). `meta.category` is `2d-scenes` —
//! every other shader module lands in Effects by type fallback.
//!
//! GLSL source anchors: fragmentShader, `modules/Bands.js:11-33`.

#![forbid(unsafe_code)]

use crate::shader::default_shader::direct_fragment;
use crate::shader::modules::{ShaderModule, fragment_with_props};
use crate::shader::uniforms::{PropKind, ShaderProp};

// Port of Bands.js:14-32 (fragmentShader body).
// GLSL's module-scope `ring(radius, width)` reads the varying `vUv` from
// outer scope; WGSL passes it in explicitly. The `ModuleProps` declaration
// is prepended by `fragment_with_props`.
const BODY: &str = r#"fn ring(uv: vec2<f32>, radius: f32, width: f32) -> f32 {
  let d = length(uv - vec2<f32>(0.5)) * 2.0;
  return 1.0 - smoothstep(0.0, width, abs(d - radius));
}

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let vUv = modv_uv(position.xy);
  let color = vec3<f32>(
    ring(vUv, u.u_bass, props.u_ringWidth.x),
    ring(vUv, u.u_mid, props.u_ringWidth.x),
    ring(vUv, u.u_treble, props.u_ringWidth.x)
  );
  // Port of: gl_FragColor = vec4(color, 1.0);
  return vec4<f32>(color, 1.0);
}
"#;

pub(crate) fn module() -> ShaderModule {
    let props = vec![ShaderProp {
        name: "u_ringWidth",
        kind: PropKind::Float,
        label: "Ring Width",
        min: 0.01,
        max: 0.5,
        step: 0.01,
        default: 0.05,
    }];
    ShaderModule {
        name: "Bands",
        fragment_wgsl: direct_fragment(&fragment_with_props(BODY, &props)),
        meta_author: "modV",
        meta_version: "0.1",
        category: Some("2d-scenes"),
        audio: true,
        audio_features: &["bass", "mid", "treble"],
        preview_with_output: false,
        props,
    }
}
