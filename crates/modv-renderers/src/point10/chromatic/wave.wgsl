// Chromatic Wave — WGSL port of the FRAG constant in
// src/renderer/src/application/point10/scenes/ChromaticWave.js (lines 11-45).
//
// The vertex entry (`vs_main`, the full-screen triangle) is prepended from
// crate::point10::FULLSCREEN_TRIANGLE_WGSL, matching the legacy raw-NDC
// vertex shader inherited from ChromaticLine.js (`VERT`, lines 31-36).
//
// Layout note: the tail is two scalar pads, not a vec3 — see the same note in
// line.wgsl. Member offsets: resolution 0, _pad0 8, x_scale 16, _pad1 28,
// glow 32, _pad2 44, distortion 48, time 52, _pad3 56/60, span 64.

struct ChromaticWaveUniforms {
  resolution: vec2<f32>, _pad0: vec2<f32>,
  x_scale: vec3<f32>, _pad1: f32,
  glow: vec3<f32>, _pad2: f32,
  distortion: f32, time: f32, _pad3: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: ChromaticWaveUniforms;
@group(0) @binding(1) var wave_tex: texture_2d<f32>;
@group(0) @binding(2) var wave_sampler: sampler;

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let resolution = u.resolution;
  // wgpu's fragment position is top-left origin; GL's gl_FragCoord is
  // bottom-left. Flip y so the scene matches the GL reference.
  let frag = vec2<f32>(position.x, resolution.y - position.y);
  let p = (frag * 2.0 - resolution) / min(resolution.x, resolution.y);

  let d = length(p) * u.distortion;
  let rx = p.x * (1.0 + d);
  let gx = p.x;
  let bx = p.x * (1.0 - d);

  // One waveform spans the whole screen edge to edge (gl_FragCoord.x /
  // resolution.x instead of fract(u) — no tiling, no per-tile repeat).
  let energy = textureSample(wave_tex, wave_sampler, vec2<f32>(frag.x / resolution.x, 0.5)).r;
  let mul = 0.75 + 0.5 * energy;

  // The pen's sin((u + time) * xScale) * yScale, with yScale folded into mul.
  let r = u.glow.r / abs(p.y + sin((rx + u.time) * u.x_scale.r) * mul);
  let g = u.glow.g / abs(p.y + sin((gx + u.time) * u.x_scale.g) * mul);
  let b = u.glow.b / abs(p.y + sin((bx + u.time) * u.x_scale.b) * mul);

  // The line is the artwork, not a full-frame fill: the glow tail below 0.5
  // is the background veil, so alpha fades out there and the bright line
  // stays opaque (ChromaticWave.js:41-44).
  let intensity = max(r, max(g, b));
  let alpha = smoothstep(0.5, 1.0, intensity);
  return vec4<f32>(r, g, b, alpha);
}
