// The JS's `renderPipeline.outputNode` (`SmokeRoom.js:1270-1276`):
//
//   outColor = scenePassColor.add(bloomPass)
//   outputNode = vec4(outColor.rgb, max(outColor.r, max(outColor.g, outColor.b)))
//
// `scenePassColor` is the combined geometry + volume target (see
// `combine.wgsl`) and the bloom is the JS `UnrealBloomPass` composite, both in
// linear HDR. The alpha rule keeps empty pixels transparent so SmokeRoom /
// DvdRoom composite over the layers below as a foreground layer.

struct FinalUniforms {
  resolution: vec2<f32>,
  _pad: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: FinalUniforms;
@group(0) @binding(1) var scene_colour: texture_2d<f32>;
@group(0) @binding(2) var bloom_colour: texture_2d<f32>;
@group(0) @binding(3) var source_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let scene = textureSample(scene_colour, source_sampler, uv);
  let bloom = textureSample(bloom_colour, source_sampler, uv);
  let rgb = scene.rgb + bloom.rgb;
  return vec4<f32>(rgb, clamp(max(max(rgb.r, rgb.g), rgb.b), 0.0, 1.0));
}
