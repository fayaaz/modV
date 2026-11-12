// Shared uniform block + maths for the volumetric fire/smoke simulation used
// by SmokeRoom and DvdRoom (port of `SmokeRoom.js`'s TSL compute passes and
// `VolumeNodeMaterial` raymarch).
//
// Every pass binds this block at `group(0) binding(0)`; each pass declares its
// own textures above it. The field packing mirrors `VolumeUniforms` in
// `volume/mod.rs` exactly (vec4 rows so the layout is alignment-free).
//
// Field map:
//   grid        xyz grid dims, w simulation dt (1/120 * speed)
//   world       xyz volume world size, w `uTime` (simulation seconds % 1000)
//   origin      xyz grid bottom-centre, w `uBoxScale`
//   sphere_pos  xyz emitter centre, w `uSphereSpeed`
//   sphere_vel  xyz emitter velocity, w `uMotionBoost`
//   camera      xyz camera position, w frame id (raymarch jitter)
//   cam_r/u/f   camera basis (right / up / forward)
//   fire_start  rgb `uFireStartColor`, w `uFireIntensity`
//   fire_mid    rgb `uFireMidColor`, w `uFireGlowSpread`
//   fire_end    rgb `uFireEndColor`, w `uFireHue` (radians)
//   turb        x buoyancy, y weight, z turbulence, w turbulence decay
//   turb2       x turb frequency, y velocity damping, z cooling, w dissipation
//   emit        x emit density, y emit temperature, z `uFireSize`, w emit gain
//   emit2       x wind strength, y flicker, z flame height, w unused
//   light       x shadow absorption, y shadow ambient, z multi scattering,
//               w powder strength
//   light2      x asymmetry, y unused, z unused, w unused
//   point_light rgb the volume point-light colour (already hue-equalised,
//               flickered and fade-in scaled), w unused
//   view        x tan(fov/2), y aspect, z raymarch target width,
//               w raymarch target height
//   misc        x emitter vertex count, yzw unused
//   emitter_model  the emitter's world matrix (`uSphereMatrix`)
struct VolumeUniforms {
  grid: vec4<f32>,
  world: vec4<f32>,
  origin: vec4<f32>,
  sphere_pos: vec4<f32>,
  sphere_vel: vec4<f32>,
  camera: vec4<f32>,
  cam_r: vec4<f32>,
  cam_u: vec4<f32>,
  cam_f: vec4<f32>,
  fire_start: vec4<f32>,
  fire_mid: vec4<f32>,
  fire_end: vec4<f32>,
  turb: vec4<f32>,
  turb2: vec4<f32>,
  emit: vec4<f32>,
  emit2: vec4<f32>,
  light: vec4<f32>,
  light2: vec4<f32>,
  point_light: vec4<f32>,
  view: vec4<f32>,
  misc: vec4<f32>,
  emitter_model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> u: VolumeUniforms;

// `SPHERE_RADIUS` (`SmokeRoom.js:73`): the emitter radius the sphere buoyancy
// uses, scaled by `uBoxScale` like the JS.
const SPHERE_RADIUS: f32 = 0.9;
// `uKeyLightPos` (`:1233`): the overhead sun, also the raymarch shadow target.
const KEY_LIGHT: vec3<f32> = vec3<f32>(0.0, 80.0, 0.0);
// `ROYGBIV_HUES[4]`, the reference hue `fireHue` equalises every other state to.
const ROYGBIV_GREEN: f32 = 2.0943951; // 2 * PI / 3
// `VOLUME_WORLD_SIZE_Y` is 12 in both scenes and every JS division uses it.
const WORLD_SIZE_Y: f32 = 12.0;

// Grid dimensions as floats (the uniform stores them pre-scaled).
fn grid_dims() -> vec3<f32> {
  return u.grid.xyz;
}

// `TEXEL_X/Y/Z` (`:70-72`), as a vector.
fn grid_texel() -> vec3<f32> {
  return 1.0 / u.grid.xyz;
}

// `getVoxelCoord` + `coordToUVW` (`:538-548`).
fn voxel_uvw(coord: vec3<u32>) -> vec3<f32> {
  return (vec3<f32>(coord) + 0.5) / u.grid.xyz;
}

// `worldPos = (uvw - 0.5) * world + (0, world.y / 2, 0) + gridOrigin` (`:626-630`).
fn uvw_to_world(uvw: vec3<f32>) -> vec3<f32> {
  return (uvw - 0.5) * u.world.xyz + vec3<f32>(0.0, u.world.y * 0.5, 0.0) + u.origin.xyz;
}

// The inverse of `uvw_to_world` (the raymarch's `getVolumeSample`, `:867-873`).
fn world_to_uvw(world: vec3<f32>) -> vec3<f32> {
  return (world - u.origin.xyz - vec3<f32>(0.0, u.world.y * 0.5, 0.0)) / u.world.xyz + 0.5;
}

// `smoothstep(0.0, width, min(edge))` — the boundary fade that keeps the volume
// from sampling the texture edge (`:886-889`, `:907-913`).
fn edge_fade(uvw: vec3<f32>, width: f32) -> f32 {
  let edge = min(uvw, vec3<f32>(1.0) - uvw);
  return smoothstep(0.0, width, min(edge.x, min(edge.y, edge.z)));
}

// ---------------------------------------------------------------------------
// Perlin noise — port of three's `ImprovedNoise` (`examples/jsm/math/
// ImprovedNoise.js`), the source of the JS flicker / sway / colour noise.
// ---------------------------------------------------------------------------

var<private> PERM: array<i32, 512> = array<i32, 512>(
  151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225,
  140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148,
  247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32,
  57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
  74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122,
  60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54,
  65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169,
  200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64,
  52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
  207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213,
  119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9,
  129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104,
  218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241,
  81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157,
  184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93,
  222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
  151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225,
  140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148,
  247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32,
  57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
  74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122,
  60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54,
  65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169,
  200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64,
  52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
  207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213,
  119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9,
  129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104,
  218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241,
  81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157,
  184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93,
  222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180
);

fn perlin_fade(t: f32) -> f32 {
  return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn perlin_grad(hash: i32, x: f32, y: f32, z: f32) -> f32 {
  let h = hash & 15;
  let u_ = select(y, x, h < 8);
  let v_ = select(select(z, x, h == 12 || h == 14), y, h < 4);
  let a = select(-u_, u_, (h & 1) == 0);
  let b = select(-v_, v_, (h & 2) == 0);
  return a + b;
}

fn perlin_noise(x_in: f32, y_in: f32, z_in: f32) -> f32 {
  let fx = floor(x_in);
  let fy = floor(y_in);
  let fz = floor(z_in);
  let x0 = i32(fx) & 255;
  let y0 = i32(fy) & 255;
  let z0 = i32(fz) & 255;
  let x = x_in - fx;
  let y = y_in - fy;
  let z = z_in - fz;
  let x1 = x - 1.0;
  let y1 = y - 1.0;
  let z1 = z - 1.0;
  let uu = perlin_fade(x);
  let vv = perlin_fade(y);
  let ww = perlin_fade(z);
  let a = PERM[x0] + y0;
  let aa = PERM[a] + z0;
  let ab = PERM[a + 1] + z0;
  let b = PERM[x0 + 1] + y0;
  let ba = PERM[b] + z0;
  let bb = PERM[b + 1] + z0;
  let grad_x0 = perlin_grad(PERM[aa], x, y, z);
  let grad_x1 = perlin_grad(PERM[ba], x1, y, z);
  let grad_x0_y1 = perlin_grad(PERM[ab], x, y1, z);
  let grad_x1_y1 = perlin_grad(PERM[bb], x1, y1, z);
  let lerp_x0 = mix(grad_x0, grad_x1, uu);
  let lerp_x1 = mix(grad_x0_y1, grad_x1_y1, uu);
  let lerp_y0 = mix(lerp_x0, lerp_x1, vv);
  let grad_x0_z1 = perlin_grad(PERM[aa + 1], x, y, z1);
  let grad_x1_z1 = perlin_grad(PERM[ba + 1], x1, y, z1);
  let grad_x0_y1_z1 = perlin_grad(PERM[ab + 1], x, y1, z1);
  let grad_x1_y1_z1 = perlin_grad(PERM[bb + 1], x1, y1, z1);
  let lerp_x0_z1 = mix(grad_x0_z1, grad_x1_z1, uu);
  let lerp_x1_z1 = mix(grad_x0_y1_z1, grad_x1_y1_z1, uu);
  let lerp_y1 = mix(lerp_x0_z1, lerp_x1_z1, vv);
  return mix(lerp_y0, lerp_y1, ww);
}

// ---------------------------------------------------------------------------
// Simplex noise — port of three's `snoise` / `snoiseVec3`
// (`examples/jsm/tsl/math/curlNoise.js`).
// ---------------------------------------------------------------------------

fn permute4(x: vec4<f32>) -> vec4<f32> {
  let squared = x * x * 34.0 + x;
  return squared - 289.0 * floor(squared / 289.0);
}

fn step4(edge: vec4<f32>, x: vec4<f32>) -> vec4<f32> {
  return select(vec4<f32>(0.0), vec4<f32>(1.0), x >= edge);
}

fn snoise(v: vec3<f32>) -> f32 {
  let C = vec2<f32>(1.0 / 6.0, 1.0 / 3.0);
  let D = vec4<f32>(0.0, 0.5, 1.0, 2.0);
  var i = floor(v + vec3<f32>(dot(v, C.yyy)));
  let x0 = v - i + vec3<f32>(dot(i, C.xxx));
  // `step(x0.yzx, x0.xyz)` (three's 3-wide compare), componentwise.
  let g3 = vec3<f32>(
    select(0.0, 1.0, x0.z >= x0.x),
    select(0.0, 1.0, x0.x >= x0.y),
    select(0.0, 1.0, x0.y >= x0.z),
  );
  let l3 = vec3<f32>(1.0) - g3;
  let i1 = min(g3.xyz, l3.zxy);
  let i2 = max(g3.xyz, l3.zxy);
  let x1 = x0 - i1 + C.x;
  let x2 = x0 - i2 + C.y;
  let x3 = x0 - vec3<f32>(D.yyy);
  i = i - 289.0 * floor(i / 289.0);
  let p = permute4(
    permute4(permute4(i.z + vec4<f32>(0.0, i1.z, i2.z, 1.0)) + i.y + vec4<f32>(0.0, i1.y, i2.y, 1.0))
      + i.x
      + vec4<f32>(0.0, i1.x, i2.x, 1.0),
  );
  let ns = 0.142857142857 * vec4<f32>(D.w, D.y, D.z, 0.0) - vec4<f32>(D.x, D.z, D.x, 0.0);
  let j = p - 49.0 * floor(p * ns.z * ns.z);
  let x_ = floor(j * ns.z);
  let xv = x_ * vec4<f32>(ns.x) + vec4<f32>(ns.y);
  let yv = floor(j - 7.0 * x_) * vec4<f32>(ns.x) + vec4<f32>(ns.y);
  let h = vec4<f32>(1.0) - abs(xv) - abs(yv);
  let b0 = vec4<f32>(xv.xy, yv.xy);
  let b1 = vec4<f32>(xv.zw, yv.zw);
  let sh = -step4(vec4<f32>(0.0), h);
  let a0 = b0.xzyw + (floor(b0) * 2.0 + 1.0).xzyw * sh.xxyy;
  let a1 = b1.xzyw + (floor(b1) * 2.0 + 1.0).xzyw * sh.zzww;
  let p0 = vec3<f32>(a0.xy, h.x);
  let p1 = vec3<f32>(a0.zw, h.y);
  let p2 = vec3<f32>(a1.xy, h.z);
  let p3 = vec3<f32>(a1.zw, h.w);
  let norm = inverseSqrt(vec4<f32>(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
  let n0 = p0 * norm.x;
  let n1 = p1 * norm.y;
  let n2 = p2 * norm.z;
  let n3 = p3 * norm.w;
  let m = max(vec4<f32>(0.6) - vec4<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), vec4<f32>(0.0));
  return 0.5
    + 12.0
      * dot(
        m * m * m,
        vec4<f32>(dot(n0, x0), dot(n1, x1), dot(n2, x2), dot(n3, x3)),
      );
}

fn snoise_vec3(x: vec3<f32>) -> vec3<f32> {
  return vec3<f32>(
    snoise(x * 2.0 - 1.0),
    snoise(vec3<f32>(x.y - 19.1, x.z + 33.4, x.x + 47.2)) * 2.0 - 1.0,
    snoise(vec3<f32>(x.z + 74.2, x.x - 124.5, x.y + 99.4)) * 2.0 - 1.0,
  );
}

// `mx_noise_float` stand-in: the Plane9 simplex noise the crate's other scenes
// use for the same MaterialX helper (see `shared/p9_glsl.rs`). Used by the
// emitter flicker only; the fire volume's shape comes from `snoise`.
fn mx_noise(p: vec3<f32>) -> f32 {
  return snoise(p);
}

// ---------------------------------------------------------------------------
// Colour — `fireRamp` (SmokeRoom `:850-858`), TSL `hue`/`luminance`
// (`ColorAdjustment.js`) and `fireHue` (`:92-96`).
// ---------------------------------------------------------------------------

fn fire_ramp(t: f32) -> vec3<f32> {
  var colour = mix(vec3<f32>(0.0), u.fire_end.rgb, smoothstep(0.05, 0.35, t));
  colour = mix(colour, u.fire_mid.rgb, smoothstep(0.35, 0.65, t));
  return mix(colour, u.fire_start.rgb, smoothstep(0.65, 1.0, t));
}

fn hue_rotate(colour: vec3<f32>, angle: f32) -> vec3<f32> {
  let k = vec3<f32>(0.57735);
  let cos_angle = cos(angle);
  let rotated = colour * cos_angle
    + cross(k, colour) * sin(angle)
    + k * (dot(k, colour) * (1.0 - cos_angle));
  return max(rotated, vec3<f32>(0.0));
}

fn luminance(colour: vec3<f32>) -> f32 {
  return dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn fire_hue(colour: vec3<f32>, angle: f32) -> vec3<f32> {
  let rotated = hue_rotate(colour, angle);
  let green = hue_rotate(colour, ROYGBIV_GREEN);
  return rotated * (luminance(green) / max(luminance(rotated), 0.001));
}
