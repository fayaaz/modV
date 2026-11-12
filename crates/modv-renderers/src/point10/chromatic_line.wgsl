// Chromatic Line — WGSL port of the FRAG constant in
// src/renderer/src/application/point10/scenes/ChromaticLine.js (lines 31-64).
//
// Layout note: `_pad4_0.._pad4_2` are three scalar pads, NOT `_pad4: vec3<f32>`.
// WGSL vec3<f32> carries 16-byte alignment, so a trailing vec3 pad would be
// placed at offset 80 and stretch the struct span to 96 bytes (verified with
// naga's TypeInner::Struct span). The Rust uniform block is 80 bytes, and wgpu
// rejects draws whose bound buffer is smaller than the shader-expected size, so
// the tail is flattened to three f32 pads to keep every member at exactly the
// offset the Rust ChromaticLineUniforms struct uses (resolution 0, _pad0 8,
// x_scale 16, _pad1 28, amp 32, _pad2 44, glow 48, _pad3 60, distortion 64,
// _pad4_0..2 68/72/76, struct span 80).
struct ChromaticLineUniforms {
  resolution: vec2<f32>, _pad0: vec2<f32>,
  x_scale: vec3<f32>, _pad1: f32,
  amp: vec3<f32>, _pad2: f32,
  glow: vec3<f32>, _pad3: f32,
  distortion: f32, _pad4_0: f32, _pad4_1: f32, _pad4_2: f32,
}
@group(0) @binding(0) var<uniform> u: ChromaticLineUniforms;
@group(0) @binding(1) var wave_tex: texture_2d<f32>;
@group(0) @binding(2) var wave_sampler: sampler;

// Full-screen triangle, the same trick the legacy pipeline uses for raw NDC
// output (ChromaticLine.js VERT / BufferGeometry: [-1,-1, 3,-1, -1,3]).
@vertex fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var pos = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(pos[vi], 0.0, 1.0);
}

// ChromaticLine.js FRAG waveAt(): the wave texture is sampled with REPEAT on
// the S axis; the sampler bound for binding 2 provides AddressMode::Repeat.
fn wave_at(coord: f32) -> f32 {
  return textureSample(wave_tex, wave_sampler, vec2<f32>(fract(coord), 0.5)).r * 2.0 - 1.0;
}

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let resolution = u.resolution;
  // wgpu fragment position is top-left origin; GL's gl_FragCoord is
  // bottom-left. Flip y so the scene is not mirrored relative to the
  // reference (ChromaticWave/ChromaticLine GL).
  let frag = vec2<f32>(position.x, resolution.y - position.y);
  let p = (frag * 2.0 - resolution) / min(resolution.x, resolution.y);
  let d = length(p) * u.distortion;
  let rx = p.x * (1.0 + d);
  let gx = p.x;
  let bx = p.x * (1.0 - d);
  let r = u.glow.r / abs(p.y + wave_at(rx * u.x_scale.r) * u.amp.r);
  let g = u.glow.g / abs(p.y + wave_at(gx * u.x_scale.g) * u.amp.g);
  let b = u.glow.b / abs(p.y + wave_at(bx * u.x_scale.b) * u.amp.b);
  // The line is the artwork, not a full-frame fill; the glow tail below 0.5 is
  // the background veil, so fade alpha out there and keep the bright line
  // opaque (ChromaticLine.js FRAG comment, GL lines 57-61).
  let intensity = max(r, max(g, b));
  let alpha = smoothstep(0.5, 1.0, intensity);
  return vec4<f32>(r, g, b, alpha);
}
