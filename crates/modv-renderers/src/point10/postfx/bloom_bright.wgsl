// Bright-pass of UnrealBloomPass — port of three's
// LuminosityHighPassShader (examples/jsm/shaders/LuminosityHighPassShader.js).
//
// `luminance()` in three is generated from the working colour space's
// luminance coefficients (WebGLProgram.js:136-144); for linear-sRGB those are
// the Rec.709 weights below.
struct BrightUniforms {
  // 1 / bright-target size. The bright target is half the source, so the uv
  // must come from the *destination* size, not textureDimensions(source).
  inv_size: vec2<f32>,
  threshold: f32,
  smooth_width: f32,
}
@group(0) @binding(0) var<uniform> u: BrightUniforms;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

fn luminance(rgb: vec3<f32>) -> f32 {
  return dot(vec3<f32>(0.2126, 0.7152, 0.0722), rgb);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let texel = textureSample(source, source_sampler, position.xy * u.inv_size);
  let v = luminance(texel.rgb);
  // defaultColor is black with defaultOpacity 0, so the mix fades the
  // sub-threshold tail to transparent black instead of a hard cut.
  let alpha = smoothstep(u.threshold, u.threshold + u.smooth_width, v);
  return mix(vec4<f32>(0.0, 0.0, 0.0, 0.0), texel, alpha);
}
