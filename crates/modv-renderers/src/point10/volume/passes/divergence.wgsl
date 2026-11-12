// `divergencePass` (`SmokeRoom.js:675-688`): central-difference divergence of
// the advected velocity field.

@group(0) @binding(1) var vel_src: texture_3d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var divergence_out: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (any(gid >= vec3<u32>(u.grid.xyz))) {
    return;
  }

  let uvw = voxel_uvw(gid);
  let texel = grid_texel();
  let v_r = textureSampleLevel(vel_src, linear_sampler, uvw + vec3<f32>(texel.x, 0.0, 0.0), 0.0).x;
  let v_l = textureSampleLevel(vel_src, linear_sampler, uvw - vec3<f32>(texel.x, 0.0, 0.0), 0.0).x;
  let v_u = textureSampleLevel(vel_src, linear_sampler, uvw + vec3<f32>(0.0, texel.y, 0.0), 0.0).y;
  let v_d = textureSampleLevel(vel_src, linear_sampler, uvw - vec3<f32>(0.0, texel.y, 0.0), 0.0).y;
  let v_f = textureSampleLevel(vel_src, linear_sampler, uvw + vec3<f32>(0.0, 0.0, texel.z), 0.0).z;
  let v_b = textureSampleLevel(vel_src, linear_sampler, uvw - vec3<f32>(0.0, 0.0, texel.z), 0.0).z;

  let divergence = (v_r - v_l + v_u - v_d + v_f - v_b) * 0.5;
  textureStore(divergence_out, vec3<i32>(gid), vec4<f32>(divergence, 0.0, 0.0, 0.0));
}
