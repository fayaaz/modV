// `computeCurlNoisePass` (`SmokeRoom.js:551-583`), run once at init.
//
// Stores a divergence-free xyz noise field sampled by the advection passes.
// The JS writes `vec3(x, y, z) * 5.0` (it inlines the curl rather than using
// the addon's `1 / (2e)` divisor), so the texture is 5x the raw curl.

@group(0) @binding(1) var curl_out: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (any(gid >= vec3<u32>(u.grid.xyz))) {
    return;
  }

  let uvw = voxel_uvw(gid);
  let freq = u.turb2.x;
  let e = 0.1 / freq;
  let p = uvw * vec3<f32>(u.world.x / u.world.y, 1.0, u.world.z / u.world.y);

  let px0 = snoise_vec3((p - vec3<f32>(e, 0.0, 0.0)) * freq);
  let px1 = snoise_vec3((p + vec3<f32>(e, 0.0, 0.0)) * freq);
  let py0 = snoise_vec3((p - vec3<f32>(0.0, e, 0.0)) * freq);
  let py1 = snoise_vec3((p + vec3<f32>(0.0, e, 0.0)) * freq);
  let pz0 = snoise_vec3((p - vec3<f32>(0.0, 0.0, e)) * freq);
  let pz1 = snoise_vec3((p + vec3<f32>(0.0, 0.0, e)) * freq);

  let x = py1.z - py0.z - pz1.y + pz0.y;
  let y = pz1.x - pz0.x - px1.z + px0.z;
  let z = px1.y - px0.y - py1.x + py0.x;

  let noise_value = vec3<f32>(x, y, z) * 5.0;
  textureStore(curl_out, vec3<i32>(gid), vec4<f32>(noise_value, 0.0));
}
