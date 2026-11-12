// Pure fire-ramp + hue maths shared by the volume raymarch (`common.wgsl`) and
// the lava/sphere emitter shaders (RollingFire's ball, SmokeRoom's orb).
//
// Every input is an argument, so a scene shader can reuse the exact JS
// `fireRamp` / `fireHue` functions without binding the volume uniform block.
// Ported from RollingFire.js `:89-100` / SmokeRoom.js `:92-96` and `:850-858`:
//
//   const fireHue = (color, adjustment) => {
//     const rotated = hue(color, adjustment);
//     const green = hue(color, ROYGBIV_HUES[4]);
//     return rotated.mul(luminance(green).div(luminance(rotated).max(0.001)));
//   };

// `ROYGBIV_HUES[4]`, the reference hue `fireHue` equalises every other state to.
const ROYGBIV_GREEN: f32 = 2.0943951; // 2 * PI / 3

// `fireRamp(t)` (`SmokeRoom.js:850-858`): black -> end -> mid -> start.
fn fire_ramp_stops(t: f32, start: vec3<f32>, mid: vec3<f32>, end: vec3<f32>) -> vec3<f32> {
  var colour = mix(vec3<f32>(0.0), end, smoothstep(0.05, 0.35, t));
  colour = mix(colour, mid, smoothstep(0.35, 0.65, t));
  return mix(colour, start, smoothstep(0.65, 1.0, t));
}

// three's `hue()`: Rodrigues rotation about the white axis, each channel
// clamped at 0 like `Color.setRGB(Math.max(0, x), ...)`.
fn hue_rotate(colour: vec3<f32>, angle: f32) -> vec3<f32> {
  let k = vec3<f32>(0.57735);
  let cos_angle = cos(angle);
  let rotated = colour * cos_angle
    + cross(k, colour) * sin(angle)
    + k * (dot(k, colour) * (1.0 - cos_angle));
  return max(rotated, vec3<f32>(0.0));
}

// three's `luminance()` (Rec. 709 weights).
fn luminance(colour: vec3<f32>) -> f32 {
  return dot(colour, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// `fireHue(colour, angle)`: rotate, then rescale so the sample keeps the green
// ROYGBIV state's luminance (each ramp stop stays equally bright).
fn fire_hue_stops(colour: vec3<f32>, angle: f32) -> vec3<f32> {
  let rotated = hue_rotate(colour, angle);
  let green = hue_rotate(colour, ROYGBIV_GREEN);
  return rotated * (luminance(green) / max(luminance(rotated), 0.001));
}

// TSL `saturation(colour, adjustment)`: `mix(luminance, colour, adj)`.
fn saturation(colour: vec3<f32>, adjustment: f32) -> vec3<f32> {
  let grey = luminance(colour);
  return max(vec3<f32>(grey) + (colour - vec3<f32>(grey)) * adjustment, vec3<f32>(0.0));
}
