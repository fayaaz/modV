// RenderPass — three's RenderPass / CopyShader equivalent.
//
// `UnrealBloomPass` and `OutputPass` copy a texture 1:1 with the CopyShader;
// this is that blit. `position / textureDimensions(source)` replaces the uv
// attribute the full-screen quad carries in GL.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let size = vec2<f32>(textureDimensions(source));
  return textureSample(source, source_sampler, position.xy / size);
}
