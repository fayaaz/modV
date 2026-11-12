// Damped afterimage feedback — port of three's `AfterimageShader.frag`.
//
// Bindings follow the prelude provided by the pass:
//   @group(0) @binding(0) uniforms, 1 = new frame, 2 = history, 3 = sampler.

struct AfterimageUniforms {
  // x = damp, yzw padding.
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: AfterimageUniforms;
@group(0) @binding(1) var new_frame: texture_2d<f32>;
@group(0) @binding(2) var old_frame: texture_2d<f32>;
@group(0) @binding(3) var feedback_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let dimensions = vec2<f32>(textureDimensions(new_frame));
  let uv = position.xy / dimensions;
  let new_sample = textureSample(new_frame, feedback_sampler, uv);
  let old = textureSample(old_frame, feedback_sampler, uv);
  // `texelOld *= damp * when_gt(texelOld, 0.1)` — three only keeps history
  // above 0.1 so the black background never smears.
  let damped = old * u.params.x * select(vec4<f32>(0.0), vec4<f32>(1.0), old > vec4<f32>(0.1));
  return max(new_sample, damped);
}
