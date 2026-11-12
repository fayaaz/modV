// The JS's `scenePassColor` (`SmokeRoom.js:1258-1263`):
//
//   sRGB = saturation(blurredVolumetric.rgb, 1.1)
//   adjustedVolumetric = vec4(sRGB, blurredVolumetric.a) * 0.5
//   scenePassColor = scenePass.max(adjustedVolumetric).add(adjustedVolumetric)
//
// i.e. the volume light is added on top of the geometry, but the geometry is
// first raised to at least the volume's level — bloom is applied to that
// combination, exactly like the JS.

struct CombineUniforms {
  resolution: vec2<f32>,
  _pad: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: CombineUniforms;
@group(0) @binding(1) var scene_tex: texture_2d<f32>;
@group(0) @binding(2) var volumetric_tex: texture_2d<f32>;
@group(0) @binding(3) var source_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let scene = textureSample(scene_tex, source_sampler, uv);
  let volumetric = textureSample(volumetric_tex, source_sampler, uv);

  // `saturation(colour, 1.1)`: luminance + (colour - luminance) * 1.1.
  let grey = vec3<f32>(dot(volumetric.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)));
  let saturated = max(grey + (volumetric.rgb - grey) * 1.1, vec3<f32>(0.0));
  let adjusted = vec4<f32>(saturated, volumetric.a) * 0.5;

  return max(scene, adjusted) + adjusted;
}
