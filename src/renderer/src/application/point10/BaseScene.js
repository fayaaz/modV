import * as THREE from "three";

export class BaseScene {
  constructor(renderer, camera) {
    this.renderer = renderer;
    this.camera = camera;
    this.group = new THREE.Group();
    this.audio = null;
    this.time = 0;
    // Composition role: 'standalone' (plays as the main scene), 'background'
    // (under-layer) or 'foreground' (over-layer) - exactly one. Custom scenes
    // default to standalone (they were built as full-screen visuals, not
    // transparent layers).
    this.role = "standalone";
    this._init();
  }

  _init() {}

  setAudio(audio) {
    this.audio = audio;
  }

  // Overlay canvas the scene draws to outside the shared renderer (WebGPU
  // overlays). null = pure WebGL scene: PostFX captures via the shared
  // renderer. hybrid: the canvas composites OVER the shared renderer's output;
  // otherwise the snapshot replaces it (pure WebGPU scene).
  get postOverlay() {
    return null;
  }

  get transitionReady() {
    return true;
  }

  get available() {
    return true;
  }

  setTransitionHidden(hidden) {
    this._transitionHidden = !!hidden;
    if (hidden) {
      const overlay = this.postOverlay;
      if (overlay) overlay.canvas.style.display = "none";
    }
  }

  // True when the scene paints through its own EffectComposer and exposes its
  // final composer texture for PostFX and transition capture.
  get paintsCanvas() {
    return false;
  }

  requestCanvasCapture() {
    this._captureNextFrame = true;
  }

  get captureTexture() {
    return null;
  }

  // eslint-disable-next-line no-unused-vars
  update(dt, audioData) {
    this.time += dt;
  }

  render() {
    this.renderer.render(this.group, this.camera);
  }

  // eslint-disable-next-line no-unused-vars
  resize(w, h) {}

  dispose() {
    // ponytail: scene switches must NOT destroy the object graph. Scenes are
    // constructed once at startup and re-activated on every revisit -
    // destroying meshes here left a revisited scene empty (37 children -> 0,
    // rendered black and left the previous scene's frame on the canvas).
    // Releasing GPU resources is only needed at app teardown
    // (SceneManager.dispose on page exit), not on a switch. Keep the graph so
    // re-visits work.
  }
}
