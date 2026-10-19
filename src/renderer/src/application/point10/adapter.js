// Adapter between a vendored Point10 scene (BaseScene lifecycle:
// _init / setAudio / update(dt, audioData) / render() / resize(w, h)) and a
// modV 2d module.
//
// Each module instance gets its own OffscreenCanvas + WebGLRenderer + camera
// (the same rig musicvis builds in main.js: fov 60, near 0.1, far 1000, at
// z = 15), so a scene keeps its own object graph, its own EffectComposer
// chains and its own render targets instead of fighting modV's shared three
// renderer. The scene renders into its private canvas, which is then blitted
// onto the modV group canvas like any other 2d module.
//
// Scene instances live in this Map, keyed by module instance id, and NOT in
// module.data: modV snapshots active modules with JSON.parse(JSON.stringify())
// for scenes/presets, so anything hanging off data has to stay JSON-safe.
import * as THREE from "three";
import * as THREE_GPU from "three/webgpu";
import { Point10AudioBridge } from "./audioBridge.js";
import { RESPONSIVENESS_PROPS, readLevers } from "./props.js";

const instances = new Map();

// Every live scene holds its own GPU context (modV caps the process at 6 -
// src/main/index.js), its own canvas backing store and its own scene graph.
// Gallery hovers and removed modules would otherwise accumulate forever, so the
// adapter keeps only the most recently drawn scenes and disposes the rest.
const MAX_LIVE_INSTANCES = 4;
const INSTANCE_IDLE_MS = 3000;

function disposeInstance(instance) {
  instance.disposed = true;

  try {
    instance.scene.dispose?.();
  } catch (error) {
    console.warn("Point10 scene dispose failed:", error);
  }

  try {
    // frees the WebGL context slot; WebGPURenderer only has dispose()
    instance.renderer.forceContextLoss?.();
    instance.renderer.dispose?.();
  } catch (error) {
    console.warn("Point10 renderer dispose failed:", error);
  }

  instance.bitmap?.close?.();
  instance.bitmap = null;
  instance.renderTarget?.dispose?.();
  instance.renderTarget = null;
  instance.frameData = null;
  instance.canvas.width = 0;
  instance.canvas.height = 0;
}

function evictInstances(now) {
  if (instances.size <= MAX_LIVE_INSTANCES) {
    return;
  }

  const ids = Array.from(instances.keys());
  const idle = [];

  for (let i = 0; i < ids.length; i++) {
    const instance = instances.get(ids[i]);

    if (instance.presenting || instance.reading) {
      continue;
    }

    // only reap scenes that stopped being drawn, never live ones
    if (now - instance.lastUsed < INSTANCE_IDLE_MS) {
      continue;
    }

    idle.push({ id: ids[i], instance });
  }

  idle.sort((a, b) => a.instance.lastUsed - b.instance.lastUsed);

  for (let i = 0; i < idle.length; i++) {
    disposeInstance(idle[i].instance);
    instances.delete(idle[i].id);
  }
}

function createInstance(SceneClass, width, height) {
  const canvas = new OffscreenCanvas(width, height);
  const renderer = new THREE.WebGLRenderer({
    canvas,
    alpha: true,
    antialias: true,
    premultipliedAlpha: false,
  });

  // modV canvases are already at output resolution.
  renderer.setPixelRatio(1);
  renderer.setSize(width, height, false);
  renderer.setClearColor(0x000000, 0);

  const camera = new THREE.PerspectiveCamera(60, width / height, 0.1, 1000);
  camera.position.set(0, 0, 15);
  camera.lookAt(0, 0, 0);

  const bridge = new Point10AudioBridge();
  const scene = new SceneClass(renderer, camera);

  scene.setAudio(bridge);

  if (typeof scene.onActivate === "function") {
    scene.onActivate();
  }

  return {
    canvas,
    renderer,
    camera,
    bridge,
    scene,
    width,
    height,
    lastUsed: 0,
  };
}

// WebGPU scenes (Rolling Fire, Smoke Room) need the `three/webgpu` entry:
// their own WebGPURenderer and TSL passes. The adapter owns the
// OffscreenCanvas and the size (there is no window in the worker) and hands
// both to the scene through `host`; the scene builds its own camera.
//
// modV composites 2d modules on a 2d canvas, so the WebGPU frame has to leave
// the GPU as an ImageBitmap. `canvas.transferToImageBitmap()` does that on the
// GPU side (~0.3ms/frame); the alternative - reading the frame back into
// memory and re-uploading it - costs ~9ms/frame at 640x480 and ~22ms at
// 832x751, which is most of a 60fps budget. Transfer is the default, the
// readback is only the fallback for backends where transfer comes back empty.
// The frame is presented one draw() behind, which is invisible at 60fps.
function createWebGPUInstance(SceneClass, width, height) {
  const canvas = new OffscreenCanvas(width, height);
  const renderer = new THREE_GPU.WebGPURenderer({
    canvas,
    alpha: true,
    antialias: true,
  });

  const bridge = new Point10AudioBridge();
  const host = { width, height, canvas };
  const scene = new SceneClass(renderer, null, host);

  scene.setAudio(bridge);

  const instance = {
    canvas,
    renderer,
    bridge,
    scene,
    host,
    renderTarget: null,
    frameData: null,
    webgpu: true,
    ready: false,
    bitmap: null,
    presenting: false,
    width,
    height,
    lastUsed: 0,
  };

  scene
    .init()
    .then((ok) => {
      instance.ready = ok !== false;

      if (instance.ready === false) {
        console.warn(`Point10 scene "${SceneClass.name}" is unavailable.`);
      }
    })
    .catch((error) => {
      console.error(
        `Point10 scene "${SceneClass.name}" failed to init:`,
        error,
      );
    });

  return instance;
}

function resizeInstance(instance, width, height) {
  instance.width = width;
  instance.height = height;
  instance.canvas.width = width;
  instance.canvas.height = height;
  instance.renderer.setSize(width, height, false);

  const camera = instance.camera || instance.scene.camera;

  if (camera) {
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
  }

  if (instance.renderTarget) {
    instance.renderTarget.setSize(width, height);
    instance.frameData = null;
  }

  if (instance.bitmap) {
    instance.bitmap.close();
    instance.bitmap = null;
  }

  if (typeof instance.scene.resize === "function") {
    instance.scene.resize(width, height);
  }
}

function swapBitmap(instance, bitmap) {
  if (!bitmap) {
    return;
  }

  const previous = instance.bitmap;
  instance.bitmap = bitmap;

  if (previous) {
    previous.close();
  }
}

// Preferred presentation: render to the adapter canvas, hand the frame over as
// an ImageBitmap without copying it through CPU memory.
function presentFrame(instance) {
  if (instance.presenting) {
    return;
  }

  instance.presenting = true;

  Promise.resolve(instance.scene.render())
    .then(() => instance.canvas.transferToImageBitmap())
    .then((bitmap) => {
      swapBitmap(instance, bitmap);
    })
    .catch((error) => {
      if (!instance.useReadback) {
        console.warn(
          "Point10 WebGPU transferToImageBitmap failed, falling back to readback:",
          error,
        );
        instance.useReadback = true;
      }
    })
    .finally(() => {
      instance.presenting = false;
    });
}

// Fallback for backends where the canvas cannot be transferred: render into an
// explicit RenderTarget and copy it out. Buffers are reused across frames -
// allocating a fresh Uint8ClampedArray + ImageData every frame is a third of
// the cost of this path.
function readBackFrame(instance) {
  if (instance.reading) {
    return;
  }

  instance.reading = true;

  const { renderer, width, height } = instance;

  if (!instance.renderTarget || instance.renderTarget.width !== width) {
    instance.renderTarget = new THREE_GPU.RenderTarget(width, height, {
      type: THREE_GPU.UnsignedByteType,
    });
    instance.frameData = null;
  }

  if (!instance.frameData) {
    instance.frameData = new ImageData(width, height);
  }

  renderer.setRenderTarget(instance.renderTarget);
  instance.scene.render();
  renderer.setRenderTarget(null);

  renderer
    .readRenderTargetPixelsAsync(instance.renderTarget, 0, 0, width, height)
    .then((data) => {
      instance.frameData.data.set(data);

      return createImageBitmap(instance.frameData);
    })
    .then((bitmap) => {
      swapBitmap(instance, bitmap);
    })
    .catch((error) => {
      console.error("Point10 WebGPU readback failed:", error);
    })
    .finally(() => {
      instance.reading = false;
    });
}

function drawScene({
  moduleId,
  SceneClass,
  webgpu,
  canvas,
  context,
  features,
  delta,
  props,
}) {
  let instance = instances.get(moduleId);

  if (!instance) {
    try {
      instance = webgpu
        ? createWebGPUInstance(SceneClass, canvas.width, canvas.height)
        : createInstance(SceneClass, canvas.width, canvas.height);
      instances.set(moduleId, instance);
    } catch (error) {
      console.error(`Point10 scene "${moduleId}" failed to initialise:`, error);
      return;
    }
  } else if (
    instance.width !== canvas.width ||
    instance.height !== canvas.height
  ) {
    resizeInstance(instance, canvas.width, canvas.height);
  }

  instance.lastUsed = performance.now();
  evictInstances(instance.lastUsed);

  // async GPU/backend bring-up: the scene reports ready when it can draw
  if (instance.webgpu && instance.ready === false) {
    return;
  }

  const dt = Math.min(0.1, (delta || 16.7) / 1000);

  instance.bridge.update(dt, features ? features.p10 : null, readLevers(props));

  const audioData =
    instance.bridge.spectrum && instance.bridge.spectrum.length
      ? instance.bridge.spectrum
      : null;

  instance.scene.update(dt, audioData);

  if (instance.webgpu) {
    if (instance.bitmap) {
      context.drawImage(instance.bitmap, 0, 0, canvas.width, canvas.height);
    }

    if (instance.useReadback) {
      readBackFrame(instance);
    } else {
      presentFrame(instance);
    }

    return;
  }

  instance.scene.render();

  context.drawImage(instance.canvas, 0, 0, canvas.width, canvas.height);
}

function createPoint10Module({
  name,
  SceneClass,
  props = {},
  meta = {},
  webgpu = false,
}) {
  const key = name;

  return {
    meta: {
      name,
      author: "Point10",
      version: "1.0.0",
      type: "2d",
      audioFeatures: ["bass", "mid", "treble"],
      ...meta,
    },

    props: {
      ...RESPONSIVENESS_PROPS,
      ...props,
    },

    // must stay JSON-safe (see the note above)
    data: {},

    draw({ canvas, context, features, delta, props, moduleId }) {
      drawScene({
        moduleId: moduleId || key,
        SceneClass,
        webgpu,
        canvas,
        context,
        features,
        delta,
        props,
      });
    },
  };
}

export { createPoint10Module, RESPONSIVENESS_PROPS };
