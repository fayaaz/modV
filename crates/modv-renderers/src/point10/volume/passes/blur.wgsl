// `gaussianBlur(volumetricPass, uniform(0.5), 1)` (`SmokeRoom.js:1256`).
//
// three's GaussianBlurNode runs at sigma 0.5, which is effectively a 3-tap
// kernel (the ±2 taps have a weight of 3e-4). This single pass applies the
// separable kernel in both directions over the half-resolution volumetric
// target: centre 1, axial exp(-0.5 / sigma^2) = 0.135335, corners its square.

struct BlurUniforms {
  resolution: vec2<f32>,
  texel: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: BlurUniforms;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

const AXIAL: f32 = 0.1353352832366127;
const CORNER: f32 = 0.01831563888873418;
const NORMALISER: f32 = 1.0 / 1.6145258466968264;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let o = u.texel;

  var total = textureSample(source, source_sampler, uv).rgb;
  let horizontal =
    textureSample(source, source_sampler, uv + vec2<f32>(o.x, 0.0)).rgb
      + textureSample(source, source_sampler, uv - vec2<f32>(o.x, 0.0)).rgb;
  let vertical =
    textureSample(source, source_sampler, uv + vec2<f32>(0.0, o.y)).rgb
      + textureSample(source, source_sampler, uv - vec2<f32>(0.0, o.y)).rgb;
  total = total + (horizontal + vertical) * AXIAL;

  let corners =
    textureSample(source, source_sampler, uv + vec2<f32>(o.x, o.y)).rgb
      + textureSample(source, source_sampler, uv + vec2<f32>(-o.x, o.y)).rgb
      + textureSample(source, source_sampler, uv + vec2<f32>(o.x, -o.y)).rgb
      + textureSample(source, source_sampler, uv + vec2<f32>(-o.x, -o.y)).rgb;
  total = total + corners * CORNER;

  return vec4<f32>(total * NORMALISER, 1.0);
}
