// Mip composite of UnrealBloomPass — port of `_getCompositeMaterial`
// (UnrealBloomPass.js:606-676).
//
// bloomFactors = [1.0, 0.8, 0.6, 0.4, 0.2] and the tint is white for every mip
// (`UnrealBloomPass.js:190-193`), so each tap carries `lerpBloomFactor(factor)`
// in `.x` and the tint in `.yzw`.
struct CompositeUniforms {
  bloom_strength: f32,
  bloom_radius: f32,
  // Global multiplier. three's classic UnrealBloomPass applies 3.0 "for
  // backwards compatibility with the previous alpha-based intensity"; the
  // TSL `bloom()` node (BloomNode, used by the WebGPU scenes) composites with
  // no multiplier. Keeping it a uniform lets both effects share this pass.
  bloom_scale: f32,
  _pad: f32,
  taps: array<vec4<f32>, 5>,
}
@group(0) @binding(0) var<uniform> u: CompositeUniforms;
@group(0) @binding(1) var blur1: texture_2d<f32>;
@group(0) @binding(2) var blur2: texture_2d<f32>;
@group(0) @binding(3) var blur3: texture_2d<f32>;
@group(0) @binding(4) var blur4: texture_2d<f32>;
@group(0) @binding(5) var blur5: texture_2d<f32>;
@group(0) @binding(6) var blur_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  // The composite target and blur1 are both the first mip's size.
  let uv = position.xy / vec2<f32>(textureDimensions(blur1));

  var bloom = vec3<f32>(0.0);
  let t1 = u.taps[0];
  let t2 = u.taps[1];
  let t3 = u.taps[2];
  let t4 = u.taps[3];
  let t5 = u.taps[4];
  bloom = bloom + t1.x * t1.yzw * textureSample(blur1, blur_sampler, uv).rgb;
  bloom = bloom + t2.x * t2.yzw * textureSample(blur2, blur_sampler, uv).rgb;
  bloom = bloom + t3.x * t3.yzw * textureSample(blur3, blur_sampler, uv).rgb;
  bloom = bloom + t4.x * t4.yzw * textureSample(blur4, blur_sampler, uv).rgb;
  bloom = bloom + t5.x * t5.yzw * textureSample(blur5, blur_sampler, uv).rgb;
  bloom = u.bloom_scale * u.bloom_strength * bloom;

  let bloom_alpha = max(bloom.r, max(bloom.g, bloom.b));
  return vec4<f32>(bloom, bloom_alpha);
}
