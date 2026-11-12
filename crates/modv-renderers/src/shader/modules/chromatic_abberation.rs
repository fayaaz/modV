//! Chromatic Abberation — port of `modules/ChromaticAbberation.js` and
//! `modules/ChromaticAbberation/chromaticAbberation.frag` (mainImage
//! flavour). Effect from <https://www.shadertoy.com/view/MtXXDr>.
//!
//! Refracts the image through per-channel refractive indices built from the
//! three props, sampling the aperture border of the framebuffer as a lens.
//!
//! GLSL source anchors: the `.frag` body, `chromaticAbberation.frag:1-28`.
//! The JS meta keeps the misspelled registration name "Chromatic Abberation".

#![forbid(unsafe_code)]

use crate::shader::default_shader::main_image_fragment;
use crate::shader::modules::{ShaderModule, fragment_with_props};
use crate::shader::uniforms::{PropKind, ShaderProp};

// Port of chromaticAbberation.frag:8-27 (mainImage body).
// Refract indices ride the per-module prop struct; vUv is computed inline.
const BODY: &str = r#"fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  let refractiveIndex = vec3<f32>(props.rOffset.x, props.gOffset.x, props.bOffset.x);
  // [0, 1] -> [-1, 1]
  let normalizedTexCoord = vec2<f32>(2.0, 2.0) * vUv - vec2<f32>(1.0, 1.0);
  let texVec = vec3<f32>(normalizedTexCoord, 1.0);
  let normalVec = vec3<f32>(0.0, 0.0, -1.0);
  let redRefractionVec = refract(texVec, normalVec, refractiveIndex.r);
  let greenRefractionVec = refract(texVec, normalVec, refractiveIndex.g);
  let blueRefractionVec = refract(texVec, normalVec, refractiveIndex.b);
  let redTexCoord = ((redRefractionVec / redRefractionVec.z).xy + vec2<f32>(1.0)) / 2.0;
  let greenTexCoord = ((greenRefractionVec / greenRefractionVec.z).xy + vec2<f32>(1.0)) / 2.0;
  let blueTexCoord = ((blueRefractionVec / blueRefractionVec.z).xy + vec2<f32>(1.0)) / 2.0;

  return vec4<f32>(
    textureSample(u_modVCanvas, modvGroupSampler, redTexCoord).r,
    textureSample(u_modVCanvas, modvGroupSampler, greenTexCoord).g,
    textureSample(u_modVCanvas, modvGroupSampler, blueTexCoord).b,
    textureSample(u_modVCanvas, modvGroupSampler, vUv).a
  );
}
"#;

pub(crate) fn module() -> ShaderModule {
    let props = vec![
        ShaderProp {
            name: "rOffset",
            kind: PropKind::Float,
            label: "Red Offset",
            min: 1.0,
            max: 2.0,
            step: 0.001,
            default: 1.0,
        },
        ShaderProp {
            name: "gOffset",
            kind: PropKind::Float,
            label: "Green Offset",
            min: 1.0,
            max: 2.0,
            step: 0.001,
            default: 1.015,
        },
        ShaderProp {
            name: "bOffset",
            kind: PropKind::Float,
            label: "Blue Offset",
            min: 1.0,
            max: 2.0,
            step: 0.001,
            default: 1.03,
        },
    ];
    ShaderModule {
        name: "Chromatic Abberation",
        fragment_wgsl: main_image_fragment(&fragment_with_props(BODY, &props)),
        meta_author: "2xAA",
        meta_version: "1.0.0",
        category: None,
        audio: false,
        audio_features: &[],
        preview_with_output: true,
        props,
    }
}
