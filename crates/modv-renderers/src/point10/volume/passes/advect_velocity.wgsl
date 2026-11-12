// `advectVelocityPass` (`SmokeRoom.js:585-673`).
//
// Semi-Lagrangian velocity self-advection + thermal/ambient curl turbulence +
// radial buoyancy + the emitter's wind + the boundary damping that makes the
// grid a sealed room.

@group(0) @binding(1) var vel_src: texture_3d<f32>;
@group(0) @binding(2) var dye_src: texture_3d<f32>;
@group(0) @binding(3) var curl_tex: texture_3d<f32>;
@group(0) @binding(4) var linear_sampler: sampler;
@group(0) @binding(5) var repeat_sampler: sampler;
@group(0) @binding(6) var vel_dst: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (any(gid >= vec3<u32>(u.grid.xyz))) {
    return;
  }

  let uvw = voxel_uvw(gid);
  let dt = u.grid.w;

  // `prevPos = uvw - (vel / worldSize) * dt`.
  let vel = textureSampleLevel(vel_src, linear_sampler, uvw, 0.0).xyz;
  let prev_pos = uvw - (vel / u.world.xyz) * dt;
  var new_vel = textureSampleLevel(vel_src, linear_sampler, prev_pos, 0.0).xyz;

  let dye = textureSampleLevel(dye_src, linear_sampler, uvw, 0.0);
  let density = dye.x;
  let temperature = dye.y;
  let age = dye.z;

  // Thermal curl: drifts down/sideways with age, decays with age.
  let thermal_pos =
    uvw + vec3<f32>(0.0, -age * 0.6, age * 0.13) / u.turb2.x;
  let decay = exp(-age * u.turb.w);
  let thermal =
    textureSampleLevel(curl_tex, repeat_sampler, thermal_pos, 0.0).xyz
      * u.turb.z
      * temperature
      * decay;

  // Ambient curl: slow time-driven drift, scaled by the local density.
  let ambient_pos = uvw * 0.5
    + vec3<f32>(0.0, u.world.w * 0.25, u.world.w * 0.06) / u.turb2.x;
  let ambient = textureSampleLevel(curl_tex, repeat_sampler, ambient_pos, 0.0).xyz
    * (u.turb.z * 0.2)
    * density;

  new_vel = new_vel + (thermal + ambient) * u.world.y * dt;

  // `newVel *= max(1 - velDamping * dt, 0)`.
  new_vel = new_vel * max(1.0 - u.turb2.y * dt, 0.0);

  // Radial buoyancy: push fire/smoke outward from the emitter centre.
  let world_pos = uvw_to_world(uvw);
  let offset = world_pos - u.sphere_pos.xyz;
  let dist = length(offset);
  let sphere_radius = SPHERE_RADIUS * u.origin.w;
  let buoyancy_force =
    (temperature * u.turb.x - density * u.turb.y) * u.world.y;
  if (dist > 0.01) {
    new_vel = new_vel + (offset / dist) * buoyancy_force * dt;
  }

  // The emitter's own motion drags the surrounding medium.
  if (dist < sphere_radius) {
    let ratio = dist / max(sphere_radius, 1e-4);
    let falloff = smoothstep(0.0, 1.0, 1.0 - ratio);
    let wind_pos = uvw + vec3<f32>(0.0, u.world.w * 0.5, 0.0) / u.turb2.x;
    let wind_turbulence =
      textureSampleLevel(curl_tex, repeat_sampler, wind_pos, 0.0).xyz
        * u.turb.z
        * u.sphere_pos.w;
    let wind = (u.sphere_vel.xyz * u.emit2.x + wind_turbulence) * dt * falloff;
    new_vel = new_vel + wind;
  }

  // The grid boundary damps velocity to zero — the sealed "room".
  let edge = min(uvw, vec3<f32>(1.0) - uvw);
  let boundary = smoothstep(0.0, 0.08, min(edge.x, min(edge.y, edge.z)));
  new_vel = new_vel * boundary;

  textureStore(vel_dst, vec3<i32>(gid), vec4<f32>(new_vel, 0.0));
}
