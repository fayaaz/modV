// `VolumeNodeMaterial` raymarch (`SmokeRoom.js:838-954`, 16 steps).
//
// The JS renders the volume box with `side: BackSide` / `depthTest: false`
// into a half-resolution pass, then `VolumetricLightingModel` marches from the
// back face toward the camera (or camera→back face when it is farther than
// `2 * modelRadius`) with a dithered offset. This pass does the same march
// analytically: a full-screen triangle intersects the box in world space, so
// there is no depth interaction and the result is additive light only.
//
// Per sample:
//   * `getVolumeSample` — velocity-distorted dye lookup + detail simplex noise
//     + the grid edge fade;
//   * `scatteringNode` — a two-step shadow march toward the key light, Beer /
//     multi-scatter / powder transmittance and a Henyey-Greenstein phase;
//   * `scatteringEmissiveNode` — the fire ramp at the local temperature, hue
//     rotated and luminance equalised, attenuated from the key light;
//   * the JS's point-light term (`pointLightColor` with
//     `isVolume == 1`: the soft segment attenuation and the 10..15 distance
//     ramp) — precomputed on the CPU into `u.point_light`.
//
// `stepLight = scattering * 0.01 + emissive * 0.01` and
// `falloff = exp(-scattering * 0.01 * stepSize)` are the JS's `start()` body.

@group(0) @binding(1) var dye_tex: texture_3d<f32>;
@group(0) @binding(2) var vel_tex: texture_3d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

const RAYMARCH_STEPS: u32 = 16u;
const SHADOW_STEPS: u32 = 2u;
const SHADOW_STEP_SIZE: f32 = 0.35;
const HENYEY_GREENSTEIN_K: f32 = 0.079577;

struct VolumeSample {
  density: f32,
  temperature: f32,
  age: f32,
  uvw: vec3<f32>,
}

fn volume_sample(position_ray: vec3<f32>) -> VolumeSample {
  let uvw = world_to_uvw(position_ray);
  let noise_distortion =
    textureSampleLevel(vel_tex, linear_sampler, uvw, 0.0).xyz / u.world.xyz * 0.15;
  let distorted = clamp(uvw + noise_distortion, vec3<f32>(0.0), vec3<f32>(1.0));
  let sample_value = textureSampleLevel(dye_tex, linear_sampler, distorted, 0.0);

  let age = sample_value.z;
  let temperature = sample_value.y;
  let detail = snoise(position_ray * 5.5 + vec3<f32>(0.0, -age * 0.8, 0.0));
  let density = sample_value.x * (detail * 0.35 + 0.85) * edge_fade(distorted, 0.06);

  return VolumeSample(density, temperature, age, distorted);
}

fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
  let g2 = g * g;
  let denom = 1.0 + g2 - 2.0 * g * cos_theta;
  let one_minus_g2 = 1.0 - g2;
  return one_minus_g2 / pow(max(denom, 1e-4), 1.5) * HENYEY_GREENSTEIN_K;
}

// Distance from `p` to the emitter's flame segment (sphere position →
// `+flameHeight` on Y), the JS `distToSegment` that drives the point light's
// soft attenuation and distance ramp.
fn distance_to_flame(p: vec3<f32>) -> f32 {
  let a = u.sphere_pos.xyz;
  let h = vec3<f32>(0.0, u.emit2.z, 0.0);
  let v = p - a;
  let t = clamp(dot(v, h) / max(dot(h, h), 1e-4), 0.0, 1.0);
  return length(p - (a + h * t));
}

// The JS's per-sample scattering: shadow march, transmittance and phase.
fn scattering_at(
  p: vec3<f32>,
  density: f32,
  light_dir: vec3<f32>,
  view_dir: vec3<f32>,
) -> f32 {
  var shadow_sum = 0.0;
  for (var i = 0u; i < SHADOW_STEPS; i = i + 1u) {
    let step_dist = (f32(i) + 0.5) * SHADOW_STEP_SIZE;
    let shadow_pos = p + light_dir * step_dist;
    let shadow_uvw = world_to_uvw(shadow_pos);
    let shadow_density = textureSampleLevel(dye_tex, linear_sampler, shadow_uvw, 0.0).x;
    shadow_sum = shadow_sum + shadow_density * edge_fade(shadow_uvw, 0.06);
  }

  let tau = shadow_sum * SHADOW_STEP_SIZE * u.light.x;
  let beer = exp(-tau);
  let multi_scatter = exp(-tau * 0.25) * 0.5;
  let base_transmittance = mix(beer, beer + multi_scatter, u.light.z);
  let powder = 1.0 - exp(-tau * 2.0);
  let final_transmittance =
    mix(base_transmittance, base_transmittance * powder, u.light.w);
  let light_transmittance = clamp(final_transmittance + u.light.y, 0.0, 1.0);

  let cos_theta = clamp(dot(view_dir, light_dir), -1.0, 1.0);
  let phase = henyey_greenstein(cos_theta, u.light2.x);

  return density * light_transmittance * phase * 12.56637;
}

// `interleavedGradientNoise(screenCoordinate)` (three's
// `InterleavedGradientNoise`): the ray-start dither that kills banding.
fn interleaved_gradient_noise(p: vec2<f32>) -> f32 {
  let magic = dot(p, vec2<f32>(0.06711056, 0.00583715));
  return fract(52.9829189 * fract(magic));
}

// Ray / axis-aligned box (centred at the origin), `x` = entry, `y` = exit.
fn intersect_box(ro: vec3<f32>, rd: vec3<f32>, half_extents: vec3<f32>) -> vec2<f32> {
  var t_min = -1e30;
  var t_max = 1e30;
  for (var axis = 0; axis < 3; axis = axis + 1) {
    let direction = rd[axis];
    let origin = ro[axis];
    if (abs(direction) < 1e-8) {
      if (abs(origin) > half_extents[axis]) {
        return vec2<f32>(-1.0, -1.0);
      }
      continue;
    }
    let inv = 1.0 / direction;
    var t0 = (-half_extents[axis] - origin) * inv;
    var t1 = (half_extents[axis] - origin) * inv;
    if (t0 > t1) {
      let saved = t0;
      t0 = t1;
      t1 = saved;
    }
    t_min = max(t_min, t0);
    t_max = min(t_max, t1);
  }
  return vec2<f32>(t_min, t_max);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let resolution = max(u.view.zw, vec2<f32>(1.0));
  let uv = position.xy / resolution;
  let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);

  let tan_half_fov = u.view.x;
  let ray_dir = normalize(
    u.cam_f.xyz
      + u.cam_r.xyz * (ndc.x * tan_half_fov * u.view.y)
      + u.cam_u.xyz * (ndc.y * tan_half_fov),
  );
  let ro = u.camera.xyz;

  let half_extents = u.world.xyz * 0.5;
  let hit = intersect_box(ro, ray_dir, half_extents);
  let t_enter = max(hit.x, 0.0);
  let t_exit = hit.y;
  if (t_exit <= t_enter) {
    return vec4<f32>(0.0);
  }

  // The JS marches from whichever end is farther than `2 * modelRadius`.
  let model_radius = length(half_extents);
  let front_to_back = t_exit > model_radius * 2.0;
  let step_size = t_exit / f32(RAYMARCH_STEPS);

  let offset = fract(
    interleaved_gradient_noise(position.xy)
      + u.camera.w * 0.618033988749895,
  );
  var dist_travelled = offset * step_size;

  var colour = vec3<f32>(0.0);
  var transmittance = vec3<f32>(1.0);

  for (var i = 0u; i < RAYMARCH_STEPS; i = i + 1u) {
    let distance_from_camera = select(t_exit - dist_travelled, dist_travelled, front_to_back);
    let p = ro + ray_dir * distance_from_camera;

    let sample_value = volume_sample(p);
    let local_light_dir = normalize(KEY_LIGHT - p);
    let local_view_dir = normalize(ro - p);
    let scattering = scattering_at(p, sample_value.density, local_light_dir, local_view_dir);

    // The JS point light: soft segment attenuation over the default inverse
    // square, and the 10..15 distance ramp (`SmokeRoom.js:1089-1199`).
    let flame_distance = distance_to_flame(p);
    let light_distance = length(p - u.sphere_pos.xyz);
    let attenuation_correction =
      (light_distance * light_distance) / (flame_distance * flame_distance + 1.44);
    let distance_scale = mix(
      10.0,
      15.0,
      smoothstep(0.0, 1.0, clamp(flame_distance / 10.0, 0.0, 1.0)),
    );
    let light_colour = u.point_light.rgb * attenuation_correction * distance_scale;
    let scattering_density = light_colour * scattering;

    let fire_power = 6.0 - u.fire_mid.w;
    let fire = fire_hue(
      fire_ramp(clamp(sample_value.temperature, 0.0, 1.0))
        * pow(max(sample_value.temperature, 0.0), fire_power)
        * u.fire_start.w,
      u.fire_end.w,
    );
    let light_delta = p - KEY_LIGHT;
    let attenuation = 400.0 / max(dot(light_delta, light_delta), 1e-3);
    let emissive = fire * (sample_value.density + 0.15) * attenuation;

    let step_light = scattering_density * 0.01 + emissive * 0.01;
    let falloff = exp(-scattering_density * 0.01 * step_size);

    if (front_to_back) {
      colour = colour + step_light * transmittance * step_size;
    } else {
      colour = colour * falloff + step_light * step_size;
    }
    transmittance = transmittance * falloff;
    dist_travelled = dist_travelled + step_size;
  }

  return vec4<f32>(colour, 1.0);
}
