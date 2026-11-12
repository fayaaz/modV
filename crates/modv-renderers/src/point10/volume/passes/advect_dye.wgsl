// `advectDyePass` (`SmokeRoom.js:733-763`): semi-Lagrangian advection of the
// density/temperature/age field, with the dissipation and cooling rates the
// scene derives from the smoke and fire lifespans.

@group(0) @binding(1) var vel_src: texture_3d<f32>;
@group(0) @binding(2) var dye_src: texture_3d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var dye_dst: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (any(gid >= vec3<u32>(u.grid.xyz))) {
    return;
  }

  let uvw = voxel_uvw(gid);
  let dt = u.grid.w;
  let vel = textureSampleLevel(vel_src, linear_sampler, uvw, 0.0).xyz;
  let prev_pos = uvw - (vel / u.world.xyz) * dt;
  let dye = textureSampleLevel(dye_src, linear_sampler, prev_pos, 0.0);

  var density = dye.x * max(1.0 - u.turb2.w * dt, 0.0);
  var temperature = dye.y * max(1.0 - u.turb2.z * dt, 0.0);

  // Age is sampled at the nearest texel (the JS `floor(prevPos * dims)`), so a
  // parcel keeps a single age instead of interpolating between neighbours.
  let dims = vec3<f32>(u.grid.xyz);
  let nearest_uvw = (floor(prev_pos * dims) + 0.5) / dims;
  var age = textureSampleLevel(dye_src, linear_sampler, nearest_uvw, 0.0).z + dt;

  temperature = clamp(temperature, 0.0, 12.0);

  if (density <= 0.01) {
    age = 0.0;
  }

  textureStore(dye_dst, vec3<i32>(gid), vec4<f32>(density, temperature, age, 1.0));
}
