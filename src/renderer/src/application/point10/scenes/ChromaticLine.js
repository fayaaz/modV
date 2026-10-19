import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";
import { P9Audio } from "../p10Audio.js";

// Full-screen chromatic line, ported from the freefrontend CodePen "GLSL
// Sample027" (pen yLgYpWM, MIT, Web_yuki1027): one quad, a fragment shader
// whose line is evaluated three times with the x input radially offset per
// channel (rx/gx/bx) so the RGB fringing shifts with distance from centre.
// The pen's sine is replaced by the audio's own waveform, kept in a ring
// buffer of ~5.9 s: each frame appends the genuinely new samples (rate*dt)
// from P9Audio.wave's tail, so old audio drifts left in real time. Each
// texel is the mean |v| of its own 23 ms sample window (the window is the
// temporal average that keeps a fixed screen point on a smooth envelope,
// never a strobe). The row lives in a 256x1 waveform texture, tiled across
// the screen (texture, not a uniform array: ANGLE/SwiftShader reject
// non-const indexing of FS uniform arrays; POT + REPEAT is legal in
// WebGL1/2). Each channel - red <- bass, green <- mids, blue <- highs -
// reads its band through P9Audio's damping-2 lane (0.12 s, the Sound Along
// A Wire read) and owns a capped amplitude min(band*10, 0.8) - the wire
// scene's displacement curve: swell on hits, settle back, flat glowing line
// in silence. The line's texel is the absolute mean |v| (biased up from
// centre); ChromaticWave derives a signed, baseline-relative undulation.
const VERT = `
attribute vec3 position;

void main() {
  gl_Position = vec4(position, 1.0);
}
`;

const FRAG = `
precision highp float;
uniform vec2 resolution;
uniform vec3 xScale;
uniform vec3 amp;
uniform vec3 glow;
uniform float distortion;
uniform sampler2D waveTex;

float waveAt(float u) {
  return texture2D(waveTex, vec2(fract(u), 0.5)).r * 2.0 - 1.0;
}

void main() {
  vec2 p = (gl_FragCoord.xy * 2.0 - resolution) / min(resolution.x, resolution.y);

  float d = length(p) * distortion;

  float rx = p.x * (1.0 + d);
  float gx = p.x;
  float bx = p.x * (1.0 - d);

  float r = glow.r / abs(p.y + waveAt(rx * xScale.r) * amp.r);
  float g = glow.g / abs(p.y + waveAt(gx * xScale.g) * amp.g);
  float b = glow.b / abs(p.y + waveAt(bx * xScale.b) * amp.b);

  gl_FragColor = vec4(r, g, b, 1.0);
}
`;

export class ChromaticLine extends BaseScene {
  _init() {
    this.label = "Chromatic Line";
    this.category = "Music";

    this.rate = 44100;
    this.SPX = 1024; // samples per texel (~23 ms)
    this.WAVE_N = 256;
    this.D = this.SPX * this.WAVE_N; // displayed horizon (~5.9 s)
    this.R = this.D + 4096; // ponytail: +4096 covers one frame's append burst
    this.ring = new Float32Array(this.R);
    this.head = 0;
    this._e = new Float32Array(this.WAVE_N);

    this.waveData = new Uint8Array(this.WAVE_N * 4);
    this.waveData.fill(255); // alpha; rgb starts 128 (= 0.0 wave)
    this.waveTex = new THREE.DataTexture(
      this.waveData,
      this.WAVE_N,
      1,
      THREE.RGBAFormat,
    );
    this.waveTex.magFilter = THREE.LinearFilter;
    this.waveTex.minFilter = THREE.LinearFilter;
    this.waveTex.wrapS = THREE.RepeatWrapping;
    this.waveTex.needsUpdate = true;
    this.uniforms = {
      resolution: { value: new THREE.Vector2(1, 1) },
      xScale: { value: new THREE.Vector3(2.5, 2.5, 2.5) },
      amp: { value: new THREE.Vector3(0, 0, 0) },
      glow: { value: new THREE.Vector3(0.05, 0.05, 0.05) },
      distortion: { value: 0.05 },
      waveTex: { value: this.waveTex },
    };

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.BufferAttribute(
        new Float32Array([-1, -1, 0, 3, -1, 0, -1, 3, 0]),
        3,
      ),
    );

    const material = new THREE.RawShaderMaterial({
      vertexShader: VERT,
      fragmentShader: FRAG,
      uniforms: this.uniforms,
      side: THREE.DoubleSide,
    });

    this.mesh = new THREE.Mesh(geometry, material);
    this.mesh.frustumCulled = false; // raw NDC output, camera-independent
    this.group.add(this.mesh);
  }

  // Texel byte (0..255) from the window's mean |v| (0..1). Absolute for the
  // line: sits at centre in silence, lifts with level.
  texelValue(e) {
    return 0.5 + 0.5 * e;
  }

  texelByte(e, ebar) {
    return 128 + ((this.texelValue(e, ebar) * 127) | 0);
  }

  // Band-driven uniforms. The line uses them for amp (wave displacement
  // curve), glow and xScale; subclasses swap in their own mapping.
  _audioUniforms(b0, b1, b2) {
    const u = this.uniforms;
    u.amp.value.set(
      Math.min(b0 * 10, 0.8),
      Math.min(b1 * 10, 0.8),
      Math.min(b2 * 10, 0.8),
    );
    u.glow.value.set(0.05 + b0 * 0.08, 0.05 + b1 * 0.08, 0.05 + b2 * 0.08);
    u.xScale.value.set(2.5 + b0 * 0.8, 2.5 + b1 * 0.8, 2.5 + b2 * 0.8);
  }

  setAudio(audio) {
    super.setAudio(audio);
    this.p9 = new P9Audio(audio);
    this.rate = audio?.ctx?.sampleRate || 44100;
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9?.update(dt);

    // Scroll: append the genuinely new samples of this frame to the ring, so
    // the waveform advances in real time and old audio drifts left. k caps at
    // the ring horizon p9.wave holds (1024 samples, ~23 ms) - a frame gap
    // beyond that (sub-24 fps) drops samples instead of re-reading them.
    if (this.p9?.hasAudio) {
      const k = Math.min(Math.round(this.rate * dt), this.p9.wave.length);
      if (k > 0) {
        const w = this.p9.wave;
        for (let m = 0; m < k; m++) {
          this.ring[(this.head + m) % this.R] = w[w.length - k + m];
        }
        this.head = (this.head + k) % this.R;
      }
    }

    // Each texel = mean |v| of its own 23 ms window (window starts at that
    // texel's ring position - one window per texel, the spatial scroll).
    const d = this.waveData,
      N = this.WAVE_N,
      S = this.SPX,
      D = this.D,
      R = this.R;
    const e = this._e;
    let ebar = 0;
    for (let i = 0; i < N; i++) {
      let m = (this.head - D + i * S) % R;
      if (m < 0) m += R;
      let sum = 0;
      for (let j = 0; j < S; j++, m++) {
        if (m === R) m = 0;
        const v = this.ring[m];
        sum += v < 0 ? -v : v;
      }
      const q = sum / S; // 0..1 mean |v| of this window
      e[i] = q;
      ebar += q;
    }
    ebar /= N;
    for (let i = 0; i < N; i++) {
      const q = this.texelByte(e[i], ebar);
      const o = i * 4;
      d[o] = d[o + 1] = d[o + 2] = q;
    }
    this.waveTex.needsUpdate = true;

    // Red <- bass, green <- mids, blue <- highs, through the damping-2 lane
    // (0.12 s low-pass - the wire scene's band read; no per-scene one-pole).
    // amp follows the wire's min(band*10, 0.8) displacement curve: swell on
    // hits, settle back, 0 in silence where only the idle glow line shows.
    const p9 = this.p9;
    const b0 = p9 ? p9.band(-1, 2, 0) : 0;
    const b1 = p9 ? p9.band(-1, 2, 1) : 0;
    const b2 = p9 ? p9.band(-1, 2, 2) : 0;
    this._audioUniforms(b0, b1, b2);
  }

  // gl_FragCoord is in drawing-buffer pixels; re-sync on canvas resize.
  _syncResolution() {
    const w = this.renderer.domElement.width;
    const h = this.renderer.domElement.height;
    if (w !== this._rw || h !== this._rh) {
      this._rw = w;
      this._rh = h;
      this.uniforms.resolution.value.set(w, h);
    }
  }

  render() {
    this._syncResolution();
    super.render();
  }
}
