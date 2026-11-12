// `emitSpherePass` (`SmokeRoom.js:765-833`): one invocation per emitter vertex.
//
// The emitter geometry seeds the dye grid: each vertex's world position maps to
// a voxel that gains density + temperature, with a per-vertex noise flicker and
// the hi-hat/snare `uFireSize` pulse. SmokeRoom seeds the lava sphere, DvdRoom
// the bouncing logo — same pass, different vertex buffer.

@group(0) @binding(1) var dye_src: texture_3d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var dye_dst: texture_storage_3d<rgba16float, write>;
@group(0) @binding(4) var<storage, read> emitter: array<vec4<f32>>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let index = gid.x;
  if (index >= u32(u.misc.x)) {
    return;
  }

  let vertex_pos = emitter[index].xyz;
  let world_pos = (u.emitter_model * vec4<f32>(vertex_pos, 1.0)).xyz;

  let uvw = world_to_uvw(world_pos);
  if (any(uvw < vec3<f32>(0.0)) || any(uvw > vec3<f32>(1.0))) {
    return;
  }

  let dims = vec3<u32>(u.grid.xyz);
  let coord = min(vec3<u32>(uvw * vec3<f32>(dims)), dims - vec3<u32>(1));

  // `mx_noise_float(vertexPos * 9 + (0, -uTime * 2.5, uTime * 0.7))`.
  let flicker =
    mx_noise(
      vertex_pos * 9.0
        + vec3<f32>(0.0, -u.world.w * 2.5, u.world.w * 0.7),
    )
    * 0.5
    + 0.5;

  // `baseEmission = emitTemperature > 0 ? 1 : 0`, plus the emitter's motion.
  let base_emission = select(0.0, 1.0, u.emit.y > 0.0);
  let movement_emission = u.sphere_pos.w * u.sphere_vel.w;
  let emission_factor = base_emission + movement_emission;
  let pulse = flicker * 0.85 + 0.15;

  let density_value =
    u.emit.x * (1.0 / 120.0) * pulse * emission_factor * u.emit.z * u.emit.w;
  if (density_value <= 0.0) {
    return;
  }

  let temperature_value =
    u.emit.y * (1.0 / 120.0) * pulse * emission_factor * u.emit.z * u.emit.w;

  let current = textureSampleLevel(dye_src, linear_sampler, uvw, 0.0);
  let new_density = current.x + density_value;
  let new_temperature = clamp(current.y + temperature_value, 0.0, 12.0);
  let new_age = mix(current.z, 0.0, density_value / max(new_density, 0.001));

  textureStore(dye_dst, coord, vec4<f32>(new_density, new_temperature, new_age, 1.0));
}
