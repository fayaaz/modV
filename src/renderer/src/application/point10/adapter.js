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
import { scenePropsFor } from "./sceneProps.js";
import { createFrameClock } from "./timing.js";
import { createWarmCache, registerWarmCache } from "./warm-cache.js";

const instances = new Map();

// Every live scene holds its own GPU context (src/main/index.js allows 16
// active contexts), its own canvas backing store and its own scene graph.
// Gallery hovers and removed modules would otherwise accumulate forever, so the
// adapter keeps only the most recently drawn scenes and disposes the rest.
const MAX_LIVE_INSTANCES = 4;
const INSTANCE_IDLE_MS = 3000;

// Warming a heavy Scene before it is applied keeps its shader compilation and
// texture allocation off the Scene-change path. A warmed instance still owns
// its GPU resources, so keep only a couple around.
const MAX_WARM_INSTANCES = 2;

// Bumped whenever the warm cache is dropped (performance mode off, app reset)
// so a warmup that finishes afterwards does not re-populate it.
let warmEpoch = 0;

const warmInstances = createWarmCache({
  limit: MAX_WARM_INSTANCES,
  onEvict: disposeInstance,
  onClear: () => {
    warmEpoch += 1;
  },
});

registerWarmCache(warmInstances);

// Warmups that are still compiling, keyed by Scene name. Two rows can pick the
// same Scene, and a Scene can become due before its template finishes; both
// should share one expensive init instead of building a second instance.
const warmInFlight = new Map();

function normalizeDimension(value) {
  const number = Math.round(Number(value));

  return Number.isFinite(number) && number > 0 ? number : 300;
}

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

// Live instances are released as soon as the Module goes away (during a
// crossfade that keeps the context budget in check). Only templates warmed
// ahead of time live in `warmInstances`.
function releaseInstance(instance) {
  instances.delete(instance.id);
  disposeInstance(instance);
}

function releaseIfIdle(instance) {
  if (!instance.disposeWhenIdle || instance.presenting || instance.reading) {
    return;
  }

  releaseInstance(instance);
}

/**
 * Releases the GPU context of a Scene instance as soon as its Module is
 * removed (a Scene transition completing, a Group being deleted, etc.) instead
 * of waiting for idle eviction. During a transition both Scenes are live, so
 * holding the outgoing Scene's contexts until they time out can push the app
 * over Chromium's active-WebGL-context budget.
 */
function removeSceneInstance(moduleId) {
  const instance = instances.get(moduleId);

  if (!instance) {
    return;
  }

  if (instance.presenting || instance.reading) {
    // Disposing a canvas mid-transfer can reject the in-flight frame; defer
    // until its promise settles.
    instance.disposeWhenIdle = true;
    return;
  }

  releaseInstance(instance);
}

function evictInstances(now) {
  if (instances.size <= MAX_LIVE_INSTANCES) {
    return;
  }

  const ids = Array.from(instances.keys());
  const idle = [];

  for (let i = 0; i < ids.length; i++) {
    const instance = instances.get(ids[i]);

    if (instance.presenting || instance.reading || instance.warming) {
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

// Gallery previews are shown one at a time (GalleryItem focus enables a single
// scene), but every preview owns a WebGL context. Letting previews accumulate
// force-loses the shared ISF/three contexts - ISF effects then silently stop
// rendering. Keep at most one gallery preview alive: the next hovered scene
// recycles it.
function evictGalleryInstances(keepId) {
  const ids = Array.from(instances.keys());

  for (let i = 0; i < ids.length; i++) {
    const id = ids[i];

    if (id === keepId) {
      continue;
    }

    const instance = instances.get(id);

    if (!instance.gallery || instance.presenting || instance.reading) {
      continue;
    }

    disposeInstance(instance);
    instances.delete(id);
  }
}

function createWebGLInstance(SceneClass, width, height) {
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
    frameClock: createFrameClock(),
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
    frameClock: createFrameClock(),
    webgpu: true,
    ready: false,
    readyPromise: null,
    bitmap: null,
    presenting: false,
    width,
    height,
    lastUsed: 0,
  };

  // The backend and the scene's own passes come up asynchronously. Prewarming
  // awaits this promise so a Scene change can hold the incoming Group's fade
  // until the first frame is actually drawable.
  instance.readyPromise = Promise.resolve()
    .then(() => scene.init())
    .then((ok) => {
      instance.ready = ok !== false;

      if (instance.ready === false) {
        console.warn(`Point10 scene "${SceneClass.name}" is unavailable.`);
      }

      return instance;
    })
    .catch((error) => {
      console.error(
        `Point10 scene "${SceneClass.name}" failed to init:`,
        error,
      );

      instance.ready = false;
      return instance;
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
      releaseIfIdle(instance);
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
      releaseIfIdle(instance);
    });
}

function createTrackedInstance({
  moduleId,
  SceneClass,
  webgpu,
  width,
  height,
  isGallery,
  warmKey = null,
}) {
  const instance = webgpu
    ? createWebGPUInstance(SceneClass, width, height)
    : createWebGLInstance(SceneClass, width, height);

  instance.id = moduleId;
  instance.SceneClass = SceneClass;
  instance.warmKey = warmKey || null;
  instance.gallery = !!isGallery;
  // Eagerly created instances must not look like they have been idle since
  // time zero: eviction would reap a prewarmed Scene before its first draw.
  instance.lastUsed = performance.now();
  instances.set(moduleId, instance);

  return instance;
}

function applyWarmupProps(instance, props, applyProps) {
  if (typeof applyProps === "function") {
    applyProps(instance.scene, props || {});
  }
}

// Compiles the scene's shaders and presents one frame before it is faded in.
// WebGPU scenes also wait for `init()` here, so the incoming Group is
// drawable when the crossfade starts instead of popping in mid-fade.
async function warmupInstance(instance, props, applyProps) {
  if (instance.webgpu) {
    await instance.readyPromise;

    if (instance.ready === false) {
      return instance;
    }
  }

  try {
    applyWarmupProps(instance, props, applyProps);
    instance.scene.update(0, null);
    await Promise.resolve(instance.scene.render());

    if (instance.webgpu) {
      try {
        swapBitmap(instance, instance.canvas.transferToImageBitmap());
      } catch (error) {
        // drawScene retries and permanently switches to readback if transfer
        // is not supported on this backend.
      }
    }
  } catch (error) {
    console.warn("Point10 scene prewarm failed:", error);
  }

  return instance;
}

async function prewarmSceneInstance({
  moduleId,
  SceneClass,
  webgpu,
  width = 300,
  height = 300,
  props,
  applyProps,
  isGallery = false,
  warmKey = null,
  warmOnly = false,
}) {
  if (!moduleId) {
    return null;
  }

  // Join an in-flight warmup for the same Scene rather than starting another.
  if (warmKey && warmInFlight.has(warmKey)) {
    const pending = warmInFlight.get(warmKey);

    if (warmOnly) {
      return pending;
    }

    await pending;
  }

  let instance = instances.get(moduleId);

  if (instance && instance.disposed) {
    instances.delete(moduleId);
    instance = null;
  }

  // Adopt a template warmed ahead of time for this Scene. It is only valid for
  // the same Scene class; a stale entry under the same name is discarded.
  if (!instance && warmKey) {
    const warm = warmInstances.take(warmKey);

    if (warm && !warm.disposed && warm.SceneClass === SceneClass) {
      instance = warm;
      instance.id = moduleId;
      instance.gallery = !!isGallery;
      instance.parked = false;
      instance.lastUsed = performance.now();
      instances.set(moduleId, instance);
    } else if (warm) {
      disposeInstance(warm);
    }
  }

  if (!instance) {
    try {
      instance = createTrackedInstance({
        moduleId,
        SceneClass,
        webgpu,
        width,
        height,
        isGallery,
        warmKey,
      });
    } catch (error) {
      console.error(`Point10 scene "${moduleId}" failed to prewarm:`, error);
      return null;
    }
  }

  const epoch = warmEpoch;

  const run = async () => {
    // Do not let idle eviction reap a Scene while it is still compiling.
    instance.warming = true;

    try {
      if (instance.width !== width || instance.height !== height) {
        resizeInstance(instance, width, height);
      }

      const warmed = await warmupInstance(instance, props, applyProps);

      // A template warmed for a future Module is not drawn yet, so park it
      // until a live Module adopts it. If the cache was dropped while it was
      // compiling, release it instead of leaving a stale warmed Scene behind.
      if (warmOnly && warmKey && warmed && !warmed.disposed) {
        instances.delete(warmed.id);

        if (epoch !== warmEpoch) {
          disposeInstance(warmed);
        } else {
          warmed.id = warmKey;
          warmed.parked = true;
          warmInstances.set(warmKey, warmed);
        }
      }

      return warmed;
    } finally {
      instance.warming = false;
    }
  };

  if (warmOnly && warmKey) {
    const pending = run();
    warmInFlight.set(warmKey, pending);

    try {
      return await pending;
    } finally {
      warmInFlight.delete(warmKey);
    }
  }

  return run();
}

function drawScene({
  moduleId,
  SceneClass,
  webgpu,
  canvas,
  context,
  features,
  props,
  applyProps,
  isGallery,
  warmKey = null,
}) {
  let instance = instances.get(moduleId);

  // A deferred removal may have released the instance after it was fetched.
  if (instance && instance.disposed) {
    instances.delete(moduleId);
    instance = null;
  }

  if (isGallery) {
    evictGalleryInstances(moduleId);
  }

  if (!instance) {
    try {
      instance = createTrackedInstance({
        moduleId,
        SceneClass,
        webgpu,
        width: canvas.width,
        height: canvas.height,
        isGallery,
        warmKey,
      });
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

  // `delta` is the absolute requestAnimationFrame timestamp, not elapsed
  // frame time (see renderers/2d/index.js), so the instance keeps its own
  // clock and advances by real seconds.
  const now = performance.now();
  instance.lastUsed = now;
  evictInstances(now);

  // async GPU/backend bring-up: the scene reports ready when it can draw
  if (instance.webgpu && instance.ready === false) {
    return;
  }

  const dt = instance.frameClock(now);

  // Scene-specific module inputs (smoke density, lifespans, ...) are applied
  // every frame so a Scene built after the first draw still receives them.
  if (typeof applyProps === "function") {
    applyProps(instance.scene, props || {});
  }

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
  applyProps = null,
}) {
  const key = name;
  // Every imported scene gets its module inputs from the central registry (see
  // sceneProps.js), so a new scene only has to register its schema there.
  const sceneConfig = scenePropsFor(name);
  const sceneProps = { ...sceneConfig.props, ...props };
  const applySceneProps = applyProps || sceneConfig.apply || null;

  return {
    meta: {
      name,
      author: "Point10",
      version: "1.0.0",
      type: "2d",
      category: "3d-scenes",
      audioFeatures: ["bass", "mid", "treble"],
      ...meta,
    },

    // Not serialised (unlike `meta`): the worker uses this to choose which
    // scenes are worth warming ahead of time. WebGPU scenes are the heavy ones
    // (large simulation textures and pipeline compilation); WebGL scenes stay
    // lazy so they do not consume the shared WebGL context budget.
    webgpu,

    props: {
      ...RESPONSIVENESS_PROPS,
      ...sceneProps,
    },

    // must stay JSON-safe (see the note above)
    data: {},

    // Called by the 2d renderer when the Module becomes active, before a
    // Scene crossfade can start. Creating (and initialising) the instance
    // here is what stops Point10 Scenes from popping in mid-fade.
    prewarm({ module, width, height, warmOnly = false } = {}) {
      return prewarmSceneInstance({
        moduleId: (module && module.$id) || key,
        SceneClass,
        webgpu,
        width: normalizeDimension(width),
        height: normalizeDimension(height),
        props: module && module.props,
        applyProps: applySceneProps,
        isGallery: !!(module && module.meta && module.meta.isGallery),
        warmKey: key,
        warmOnly,
      });
    },

    draw({ canvas, context, features, props, moduleId, isGallery }) {
      drawScene({
        moduleId: moduleId || key,
        SceneClass,
        webgpu,
        canvas,
        context,
        features,
        props,
        applyProps: applySceneProps,
        isGallery,
        warmKey: key,
      });
    },
  };
}

export {
  createPoint10Module,
  prewarmSceneInstance,
  removeSceneInstance,
  RESPONSIVENESS_PROPS,
};
