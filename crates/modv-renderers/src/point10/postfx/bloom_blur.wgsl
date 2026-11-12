// Separable gaussian blur of UnrealBloomPass — port of
// `_getSeparableBlurMaterial` (UnrealBloomPass.js:531-604).
//
// The JS builds the kernel on the CPU:
//   sigma = kernelRadius / 3
//   coefficients[i] = 0.39894 * exp(-0.5 * i * i / (sigma * sigma)) / sigma
// then merges adjacent taps into single bilinear fetches (pairs of
// `(i, i + 1)` with the weighted offset), which halves the sample count without
// changing the result. Both halves of that maths live in Rust
// (`unreal_bloom_pass::kernel_pairs`); this shader just consumes the merged
// `(offset, weight)` list.
//
// WGSL uniform arrays need 16-byte strides, so each tap is a vec4 with the
// offset in `.x` and the weight in `.y`.
struct BlurUniforms {
  // 1 / this level's size. Each level downsamples the previous one, so the uv
  // must come from the destination size (the source is larger for mips 1..5).
  inv_size: vec2<f32>,
  center_weight: f32,
  pairs: u32,
  direction: vec2<f32>,
  _pad: vec2<f32>,
  taps: array<vec4<f32>, 11>,
}
@group(0) @binding(0) var<uniform> u: BlurUniforms;
@group(0) @binding(1) var color_texture: texture_2d<f32>;
@group(0) @binding(2) var color_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy * u.inv_size;
  var sum = textureSample(color_texture, color_sampler, uv).rgb * u.center_weight;

  for (var i = 0u; i < u.pairs; i = i + 1u) {
    let tap = u.taps[i];
    let offset = u.direction * u.inv_size * tap.x;
    let sample_a = textureSample(color_texture, color_sampler, uv + offset).rgb;
    let sample_b = textureSample(color_texture, color_sampler, uv - offset).rgb;
    sum = sum + (sample_a + sample_b) * tap.y;
  }

  return vec4<f32>(sum, 1.0);
}
