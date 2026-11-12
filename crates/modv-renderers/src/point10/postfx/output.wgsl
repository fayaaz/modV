// OutputPass — port of three's examples/jsm/shaders/OutputShader.js.
//
// Applies the renderer's tone mapping and then the sRGB transfer, exactly as
// OutputShader does under the defines OutputPass sets from
// renderer.toneMapping / renderer.outputColorSpace (OutputPass.js:106-123).
// This port selects the tone map with a uniform instead of rebuilding the
// shader per setting (one pipeline per composer instead of one per mode).

struct OutputUniforms {
  exposure: f32,
  // 0 none, 1 linear, 2 reinhard, 3 cineon, 4 ACES filmic.
  mode: u32,
  // 1 when the destination format is not itself *Srgb (hardware encode off).
  srgb_encode: u32,
  _pad: f32,
}
@group(0) @binding(0) var<uniform> u: OutputUniforms;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var source_sampler: sampler;

fn saturate3(value: vec3<f32>) -> vec3<f32> {
  return clamp(value, vec3<f32>(0.0), vec3<f32>(1.0));
}

// tonemapping_pars_fragment.glsl.js, LinearToneMapping.
fn linear_tone_mapping(color: vec3<f32>, exposure: f32) -> vec3<f32> {
  return saturate3(exposure * color);
}

// ReinhardToneMapping.
fn reinhard_tone_mapping(color: vec3<f32>, exposure: f32) -> vec3<f32> {
  let scaled = color * exposure;
  return saturate3(scaled / (vec3<f32>(1.0) + scaled));
}

// CineonToneMapping (Jim Hejl / Richard Burgess-Dawson).
fn cineon_tone_mapping(color: vec3<f32>, exposure: f32) -> vec3<f32> {
  var scaled = color * exposure;
  scaled = max(vec3<f32>(0.0), scaled - 0.004);
  let mapped = (scaled * (6.2 * scaled + 0.5)) / (scaled * (6.2 * scaled + 1.7) + 0.06);
  return pow(mapped, vec3<f32>(2.2));
}

// RRTAndODTFit (ACES helper).
fn rrt_and_odt_fit(v: vec3<f32>) -> vec3<f32> {
  let a = v * (v + 0.0245786) - 0.000090537;
  let b = v * (0.983729 * v + 0.4329510) + 0.238081;
  return a / b;
}

// ACESFilmicToneMapping, including the 1/0.6 viewing-environment scale.
fn aces_filmic_tone_mapping(color: vec3<f32>, exposure: f32) -> vec3<f32> {
  // three's mat3 constructor takes columns; these are the transposed rows.
  let aces_input_a = vec3<f32>(0.59719, 0.35458, 0.04823);
  let aces_input_b = vec3<f32>(0.07600, 0.90834, 0.01566);
  let aces_input_c = vec3<f32>(0.02840, 0.13383, 0.83777);
  let aces_output_a = vec3<f32>(1.60475, -0.53108, -0.07367);
  let aces_output_b = vec3<f32>(-0.10208, 1.10813, -0.00605);
  let aces_output_c = vec3<f32>(-0.00327, -0.07276, 1.07602);

  var scaled = color * (exposure / 0.6);
  let input = vec3<f32>(
    dot(aces_input_a, scaled),
    dot(aces_input_b, scaled),
    dot(aces_input_c, scaled),
  );
  scaled = rrt_and_odt_fit(input);
  let output = vec3<f32>(
    dot(aces_output_a, scaled),
    dot(aces_output_b, scaled),
    dot(aces_output_c, scaled),
  );
  return saturate3(output);
}

// sRGBTransferOETF (colorspace_pars_fragment.glsl.js).
fn srgb_transfer_oetf(value: vec3<f32>) -> vec3<f32> {
  let low = value * 12.92;
  let high = pow(value, vec3<f32>(0.41666)) * 1.055 - vec3<f32>(0.055);
  return select(high, low, value <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let size = vec2<f32>(textureDimensions(source));
  let texel = textureSample(source, source_sampler, position.xy / size);

  var color = texel.rgb;
  switch (u.mode) {
    case 1u: { color = linear_tone_mapping(color, u.exposure); }
    case 2u: { color = reinhard_tone_mapping(color, u.exposure); }
    case 3u: { color = cineon_tone_mapping(color, u.exposure); }
    case 4u: { color = aces_filmic_tone_mapping(color, u.exposure); }
    default: { }
  }

  // Alpha is untouched by the transfer in three as well.
  if (u.srgb_encode != 0u) {
    color = srgb_transfer_oetf(color);
  }

  return vec4<f32>(color, texel.a);
}
