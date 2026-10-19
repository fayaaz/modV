// BaseScene for the vendored WebGPU scenes (Rolling Fire, Smoke Room).
//
// Same lifecycle as ../BaseScene.js, but built on the `three/webgpu` entry
// (TSL nodes, RenderPipeline) instead of the plain WebGL entry. Both entries
// come from the same three install, so their classes are interchangeable.
//
// The `host` argument is modV-specific: the adapter owns the OffscreenCanvas
// and the WebGPURenderer, and tells the scene its pixel size (there is no
// window in the worker). `init()` replaces the upstream `onActivate()` +
// private requestAnimationFrame loop - modV's worker loop drives update() and
// render() itself.
import * as THREE from "three/webgpu";

export class BaseSceneWebGPU {
  constructor(renderer, camera, host = {}) {
    this.renderer = renderer;
    this.camera = camera;
    this.host = host;
    this.group = new THREE.Group();
    this.audio = null;
    this.time = 0;
    this.role = "standalone";
    this._init();
  }

  _init() {}

  setAudio(audio) {
    this.audio = audio;
  }

  // Builds the scene's GPU resources. Resolves once the renderer backend and
  // the scene's own passes exist; the adapter skips drawing until then.
  async init() {
    return true;
  }

  get ready() {
    return true;
  }

  // False when the scene cannot run at all (no WebGPU adapter).
  get available() {
    return true;
  }

  // eslint-disable-next-line no-unused-vars
  update(dt, audioData) {
    this.time += dt;
  }

  render() {
    this.renderer.render(this.group, this.camera);
  }

  // eslint-disable-next-line no-unused-vars
  resize(width, height) {}

  dispose() {}
}
