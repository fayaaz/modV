// One Jacobi pressure iteration (`jacobi()` in `SmokeRoom.js:690-712`). The
// scene runs `PRESSURE_ITERATIONS` of these, alternating the pressure pair.

@group(0) @binding(1) var pressure_src: texture_3d<f32>;
@group(0) @binding(2) var divergence_src: texture_3d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var pressure_dst: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (any(gid >= vec3<u32>(u.grid.xyz))) {
    return;
  }

  let uvw = voxel_uvw(gid);
  let texel = grid_texel();
  let p_r = textureSampleLevel(pressure_src, linear_sampler, uvw + vec3<f32>(texel.x, 0.0, 0.0), 0.0).x;
  let p_l = textureSampleLevel(pressure_src, linear_sampler, uvw - vec3<f32>(texel.x, 0.0, 0.0), 0.0).x;
  let p_u = textureSampleLevel(pressure_src, linear_sampler, uvw + vec3<f32>(0.0, texel.y, 0.0), 0.0).x;
  let p_d = textureSampleLevel(pressure_src, linear_sampler, uvw - vec3<f32>(0.0, texel.y, 0.0), 0.0).x;
  let p_f = textureSampleLevel(pressure_src, linear_sampler, uvw + vec3<f32>(0.0, 0.0, texel.z), 0.0).x;
  let p_b = textureSampleLevel(pressure_src, linear_sampler, uvw - vec3<f32>(0.0, 0.0, texel.z), 0.0).x;
  let divergence = textureSampleLevel(divergence_src, linear_sampler, uvw, 0.0).x;

  let pressure = (p_r + p_l + p_u + p_d + p_f + p_b - divergence) / 6.0;
  textureStore(pressure_dst, vec3<i32>(gid), vec4<f32>(pressure, 0.0, 0.0, 0.0));
}
