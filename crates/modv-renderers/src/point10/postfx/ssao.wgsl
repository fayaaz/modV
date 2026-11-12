// Depth-only screen-space ambient occlusion.
//
// See `ssao.rs` for the documented substitution of three's `SSAOPass`.
// Bindings: 0 = uniforms, 1 = the scene's depth attachment.

struct SsaoUniforms {
  projection: mat4x4<f32>,
  inverse_projection: mat4x4<f32>,
  resolution: vec2<f32>,
  near: f32,
  far: f32,
  radius: f32,
  min_distance: f32,
  max_distance: f32,
  strength: f32,
  bias: f32,
  _pad: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: SsaoUniforms;
@group(0) @binding(1) var depth_map: texture_depth_2d;

const TAP_COUNT: u32 = 12u;

// Window depth -> linear view distance (the standard [0,1] depth mapping).
fn linear_depth(depth: f32) -> f32 {
  return (u.near * u.far) / (u.far - depth * (u.far - u.near));
}

// A stable per-pixel rotation so the spiral does not repeat across the frame.
fn hash12(p: vec2<f32>) -> f32 {
  let h = dot(p, vec2<f32>(12.9898, 78.233));
  return fract(sin(h) * 43758.5453);
}

fn clamp_pixel(p: vec2<i32>) -> vec2<i32> {
  return clamp(p, vec2<i32>(0), vec2<i32>(u.resolution) - vec2<i32>(1));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let pixel = vec2<i32>(position.xy);
  let centre_depth = textureLoad(depth_map, pixel, 0);
  // Nothing drawn here: leave the background unoccluded so a transparent
  // clear stays transparent through the scene's composite.
  if (centre_depth >= 1.0) {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
  }

  let centre_linear = linear_depth(centre_depth);
  // A 0.5-world-unit kernel in pixels: the vertical projection scale maps one
  // world unit at `view_z` to `proj[1][1] * height * 0.5 / view_z` pixels.
  let world_radius_px = u.radius * u.projection[1][1] * u.resolution.y * 0.5
    / max(centre_linear, 0.0001);
  let radius_px = clamp(world_radius_px, 1.0, 48.0);
  let angle_offset = hash12(position.xy) * 6.2831853;

  var occlusion = 0.0;
  for (var i = 0u; i < TAP_COUNT; i = i + 1u) {
    let t = (f32(i) + 0.5) / f32(TAP_COUNT);
    let angle = f32(i) * 2.3999632 + angle_offset;
    // `sqrt(t)` spreads the taps evenly over the disc.
    let radius = radius_px * sqrt(max(t, u.bias));
    let offset = vec2<f32>(cos(angle), sin(angle)) * radius;
    let sample_pixel = clamp_pixel(pixel + vec2<i32>(offset));
    let sample_depth = textureLoad(depth_map, sample_pixel, 0);
    // three's SSAO compares window-depth deltas against `minDistance` /
    // `maxDistance`; a positive delta means the neighbour is in front of the
    // centre pixel, i.e. it occludes it.
    let delta = centre_depth - sample_depth;
    let in_range = delta > u.min_distance && delta < u.max_distance;
    let within_kernel = abs(linear_depth(sample_depth) - centre_linear) < u.radius;
    if (in_range && within_kernel) {
      occlusion = occlusion + (1.0 - t * 0.6);
    }
  }

  let ao = clamp(1.0 - u.strength * occlusion / f32(TAP_COUNT), 0.0, 1.0);
  return vec4<f32>(ao, ao, ao, 1.0);
}
