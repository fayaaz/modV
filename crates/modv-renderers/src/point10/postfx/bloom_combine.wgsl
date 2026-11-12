// Additive bloom combine — the replacement for three's final
// `blendMaterial` draw into the read buffer.
//
// three sets `AdditiveBlending` + `premultipliedAlpha: true` on the copy
// material (UnrealBloomPass.js:196-205, 415-421), which is `blendFunc(ONE, ONE)`:
//     dst = src + dst            (rgb and alpha)
// wgpu cannot sample and render into the same texture, so the composer keeps a
// `base` copy of the scene, and this pass computes `base + bloom` in-shader
// while writing to the HDR target. Identical arithmetic, one less blend state.
//
// `fs_preserve_alpha` is `preserveBloomAlpha(bloomPass)` (`Boxy.js:73-86`,
// used by Wormhole): the same additive colour with `blendSrcAlpha = ZERO`,
// `blendDstAlpha = ONE`, i.e. the destination alpha survives the bloom. With a
// bloom threshold of 0 every pixel receives bloom, so the additive alpha would
// otherwise turn a transparent clear into an opaque veil and break the
// module's layer alpha.
@group(0) @binding(0) var base_texture: texture_2d<f32>;
@group(0) @binding(1) var bloom_texture: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / vec2<f32>(textureDimensions(base_texture));
  let base = textureSample(base_texture, linear_sampler, uv);
  let bloom = textureSample(bloom_texture, linear_sampler, uv);
  return vec4<f32>(base.rgb + bloom.rgb, base.a + bloom.a);
}

// preserveBloomAlpha: add the bloom RGB, keep the destination alpha.
@fragment
fn fs_preserve_alpha(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / vec2<f32>(textureDimensions(base_texture));
  let base = textureSample(base_texture, linear_sampler, uv);
  let bloom = textureSample(bloom_texture, linear_sampler, uv);
  return vec4<f32>(base.rgb + bloom.rgb, base.a);
}
