// GPU heightfield wave simulation, vendored-adapted from
// jeantimex/threejs-water (MIT) - a port of Evan Wallace's WebGL Water.
// The 2D wave equation on a square grid, ping-ponged between two render
// targets: height in R, vertical velocity in G. Each step:
//   acceleration = (average of 4 neighbours - height) * 2
//   velocity = velocity * damping + acceleration
//   height += velocity
// Domain-agnostic: the sim lives on the unit square; callers map their water
// geometry onto it (AroundPlus maps its annulus in polar coordinates) and
// sample `texture` for normals/distortion. Drops are cosine bumps added by
// `drop()` - AroundPlus fires them from bass attacks.

import * as THREE from "three";

const WAVE_VERT = `
varying vec2 coord;
void main() {
  coord = uv;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}`;

// One propagation step. Wrap/clamp behaviour comes from the target's wrapS/T:
// RepeatWrapping in u gives periodic boundaries, ClampToEdge reflects waves
// off walls.
const WAVE_UPDATE_FRAG = `
precision highp float;
uniform sampler2D tInput;
uniform vec2 delta;
varying vec2 coord;
void main() {
  vec4 info = texture2D(tInput, coord);
  vec2 dx = vec2(delta.x, 0.0);
  vec2 dy = vec2(0.0, delta.y);
  float ave = (texture2D(tInput, coord + dx).r + texture2D(tInput, coord - dx).r +
               texture2D(tInput, coord + dy).r + texture2D(tInput, coord - dy).r) * 0.25;
  info.g += (ave - info.r) * 2.0;
  info.g *= 0.995;
  info.r += info.g;
  gl_FragColor = info;
}`;

// Add a smooth cosine bump (a drop) to the height field.
const WAVE_DROP_FRAG = `
precision highp float;
uniform sampler2D tInput;
uniform vec2 center;
uniform float radius;
uniform float strength;
varying vec2 coord;
void main() {
  vec4 info = texture2D(tInput, coord);
  float drop = max(0.0, 1.0 - length(center - coord) / radius);
  info.r += (0.5 - cos(drop * 3.14159265) * 0.5) * strength;
  gl_FragColor = info;
}`;

export class WaveSim {
  constructor(renderer, res = 256) {
    this.renderer = renderer;
    const opts = {
      type: THREE.HalfFloatType,
      depthBuffer: false,
      stencilBuffer: false,
      // u wraps (periodic domain - waves cross the caller's angular seam),
      // v clamps (waves reflect off the caller's side walls).
      wrapS: THREE.RepeatWrapping,
      wrapT: THREE.ClampToEdgeWrapping,
    };
    this.a = new THREE.WebGLRenderTarget(res, res, opts);
    this.b = new THREE.WebGLRenderTarget(res, res, opts);
    this.scene = new THREE.Scene();
    this.cam = new THREE.Camera();
    this.quad = new THREE.Mesh(new THREE.PlaneGeometry(2, 2));
    this.scene.add(this.quad);
    this.updateMat = new THREE.ShaderMaterial({
      uniforms: {
        tInput: { value: null },
        delta: { value: new THREE.Vector2(1 / res, 1 / res) },
      },
      vertexShader: WAVE_VERT,
      fragmentShader: WAVE_UPDATE_FRAG,
    });
    this.dropMat = new THREE.ShaderMaterial({
      uniforms: {
        tInput: { value: null },
        center: { value: new THREE.Vector2() },
        radius: { value: 0.04 },
        strength: { value: 0.02 },
      },
      vertexShader: WAVE_VERT,
      fragmentShader: WAVE_DROP_FRAG,
    });
    // Rest state = all zeros.
    renderer.setRenderTarget(this.a);
    renderer.clear();
    renderer.setRenderTarget(this.b);
    renderer.clear();
    renderer.setRenderTarget(null);
  }

  _pass(mat, target, input) {
    this.quad.material = mat;
    mat.uniforms.tInput.value = input.texture;
    this.renderer.setRenderTarget(target);
    this.renderer.render(this.scene, this.cam);
    this.renderer.setRenderTarget(null);
  }

  // Disturb the field: cosine bump at (u, v) in sim space.
  drop(u, v, strength, radius = 0.04) {
    this.dropMat.uniforms.center.value.set(u, v);
    this.dropMat.uniforms.radius.value = radius;
    this.dropMat.uniforms.strength.value = strength;
    this._pass(this.dropMat, this.b, this.a);
    this._swap();
  }

  // One propagation step; call once per frame.
  step() {
    this._pass(this.updateMat, this.b, this.a);
    this._swap();
  }

  get texture() {
    return this.a.texture;
  }

  _swap() {
    [this.a, this.b] = [this.b, this.a];
  }

  dispose() {
    this.a.dispose();
    this.b.dispose();
    this.quad.geometry.dispose();
    this.updateMat.dispose();
    this.dropMat.dispose();
  }
}
