import { ChromaticLine } from "./ChromaticLine.js";

// Chromatic Wave: the CodePen "GLSL Sample027" look restored - three
// chromatically-fringed sine waves (red/green/blue radial offset, the pen's
// sin((u + time) * xScale) * yScale with yScale folded into mul). In
// silence they are clean sines; the audio's ring-buffer energy (see
// ChromaticLine: each texel = mean |v| of its 23 ms window) multiplies the
// sine subtly in-shader - mul = 0.75 + 0.5*energy, so a loud passage swells
// the wave by ~2/3 and the energy scrolls leftward in real time.
const FRAG = `
precision highp float;
uniform vec2 resolution;
uniform float time;
uniform vec3 xScale;
uniform vec3 glow;
uniform float distortion;
uniform sampler2D waveTex;

void main() {
  vec2 p = (gl_FragCoord.xy * 2.0 - resolution) / min(resolution.x, resolution.y);

  float d = length(p) * distortion;

  float rx = p.x * (1.0 + d);
  float gx = p.x;
  float bx = p.x * (1.0 - d);

  // One waveform spans the whole screen edge to edge (gl_FragCoord.x /
  // resolution.x instead of fract(u) - no tiling, no per-tile repeat).
  float energy = texture2D(waveTex, vec2(gl_FragCoord.x / resolution.x, 0.5)).r;
  float mul = 0.5 + 0.25 * energy;

  float r = glow.r / abs(p.y + sin((rx + time) * xScale.r) * mul);
  float g = glow.g / abs(p.y + sin((gx + time) * xScale.g) * mul);
  float b = glow.b / abs(p.y + sin((bx + time) * xScale.b) * mul);

  gl_FragColor = vec4(r, g, b, 1.0);
}
`;

export class ChromaticWave extends ChromaticLine {
  _init() {
    super._init();
    this.label = "Chromatic Wave";
    this.mesh.material.fragmentShader = FRAG;
    this.mesh.material.needsUpdate = true;
    delete this.uniforms.amp;
    this.uniforms.time = { value: 0 };
    this.uniforms.xScale.value.set(1.0, 1.0, 1.0);
  }

  // Plain energy (0..1): the shader's mul uses it directly.
  texelByte(e) {
    return (e * 255) | 0;
  }

  _audioUniforms(b0, b1, b2) {
    const u = this.uniforms;
    u.time.value = this.time;
    // bloom is the music driver: each channel's glow swells with its band
    // (red <- bass, green <- mids, blue <- highs). Wave speed stays nearly
    // constant - the 0.05 xScale factor only nudges the pitch, so audio
    // no longer visibly speeds/slows the scroll.
    u.glow.value.set(0.05 + b0 * 0.3, 0.05 + b1 * 0.3, 0.05 + b2 * 0.3);
    u.xScale.value.set(1.0 + b0 * 0.05, 1.0 + b1 * 0.05, 1.0 + b2 * 0.05);
  }
}
