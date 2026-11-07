import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mockState = vi.hoisted(() => ({
  webglRenderers: 0,
  webgpuRenderers: 0,
  forceLost: 0,
  disposedRenderers: 0,
}));

vi.mock("three", () => {
  class PerspectiveCamera {
    constructor() {
      this.position = { set: vi.fn() };
      this.aspect = 1;
    }

    lookAt() {}
    updateProjectionMatrix() {}
  }

  class WebGLRenderer {
    constructor() {
      mockState.webglRenderers += 1;
    }

    setPixelRatio() {}
    setSize() {}
    setClearColor() {}
    forceContextLoss() {
      mockState.forceLost += 1;
    }
    dispose() {
      mockState.disposedRenderers += 1;
    }
  }

  return { PerspectiveCamera, WebGLRenderer };
});

vi.mock("three/webgpu", () => {
  class WebGPURenderer {
    constructor() {
      mockState.webgpuRenderers += 1;
    }

    setPixelRatio() {}
    setSize() {}
    forceContextLoss() {
      mockState.forceLost += 1;
    }
    dispose() {
      mockState.disposedRenderers += 1;
    }
  }

  class RenderTarget {
    constructor(width, height) {
      this.width = width;
      this.height = height;
    }

    setSize(width, height) {
      this.width = width;
      this.height = height;
    }

    dispose() {}
  }

  return {
    WebGPURenderer,
    RenderTarget,
    UnsignedByteType: 1,
  };
});

class FakeOffscreenCanvas {
  constructor(width, height) {
    this.width = width;
    this.height = height;
  }

  transferToImageBitmap() {
    return { close: vi.fn() };
  }
}

vi.stubGlobal("OffscreenCanvas", FakeOffscreenCanvas);

const { createPoint10Module, removeSceneInstance } = await import(
  "../../src/renderer/src/application/point10/adapter.js"
);

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function makeContext() {
  return { drawImage: vi.fn() };
}

function makeModule(id) {
  return { $id: id, props: {}, meta: {} };
}

beforeEach(() => {
  mockState.webglRenderers = 0;
  mockState.webgpuRenderers = 0;
  mockState.forceLost = 0;
  mockState.disposedRenderers = 0;
});

afterEach(async () => {
  // Let deferred WebGPU presentation settle so removal is synchronous.
  await flush();
});

describe("Point10 adapter prewarm", () => {
  it("waits for a WebGPU scene's async init before resolving", async () => {
    let releaseInit;
    const initGate = new Promise((resolve) => {
      releaseInit = resolve;
    });
    const records = { scenes: [], updates: 0, renders: 0, disposed: 0 };

    class SlowScene {
      constructor() {
        records.scenes.push(this);
        this.initialized = false;
      }

      setAudio() {}
      init() {
        return initGate.then(() => {
          this.initialized = true;
          return true;
        });
      }
      update() {
        records.updates += 1;
      }
      render() {
        records.renders += 1;
      }
      dispose() {
        records.disposed += 1;
      }
    }

    const definition = createPoint10Module({
      name: "Fake Slow WebGPU Scene",
      SceneClass: SlowScene,
      webgpu: true,
    });

    let settled = false;
    const warming = definition
      .prewarm({
        module: makeModule("slow-webgpu"),
        width: 320,
        height: 180,
      })
      .then(() => {
        settled = true;
      });

    await Promise.resolve();
    expect(settled).toBe(false);
    expect(records.updates).toBe(0);

    releaseInit();
    await warming;

    expect(settled).toBe(true);
    expect(records.scenes).toHaveLength(1);
    expect(records.scenes[0].initialized).toBe(true);
    expect(records.updates).toBe(1);
    expect(records.renders).toBe(1);
    expect(mockState.webgpuRenderers).toBe(1);

    removeSceneInstance("slow-webgpu");
    expect(records.disposed).toBe(1);
  });

  it("reuses the prewarmed WebGPU instance and bitmap on draw", async () => {
    const records = { scenes: [], renders: 0, disposed: 0 };

    class ReadyScene {
      constructor() {
        records.scenes.push(this);
      }

      setAudio() {}
      init() {
        return Promise.resolve(true);
      }
      update() {}
      render() {
        records.renders += 1;
      }
      dispose() {
        records.disposed += 1;
      }
    }

    const definition = createPoint10Module({
      name: "Fake Ready WebGPU Scene",
      SceneClass: ReadyScene,
      webgpu: true,
    });
    const module = makeModule("ready-webgpu");

    await definition.prewarm({ module, width: 320, height: 180 });

    const context = makeContext();
    definition.draw({
      canvas: { width: 320, height: 180 },
      context,
      features: null,
      props: {},
      moduleId: "ready-webgpu",
      isGallery: false,
    });

    expect(records.scenes).toHaveLength(1);
    expect(mockState.webgpuRenderers).toBe(1);
    // The warmed bitmap is presented on the very first live frame.
    expect(context.drawImage).toHaveBeenCalledTimes(1);

    await flush();
    removeSceneInstance("ready-webgpu");
    expect(records.disposed).toBe(1);
  });

  it("warms up WebGL scenes with a first render and reuses the instance", async () => {
    const records = { scenes: [], renders: 0, disposed: 0 };

    class GlScene {
      constructor() {
        records.scenes.push(this);
      }

      setAudio() {}
      update() {}
      render() {
        records.renders += 1;
      }
      dispose() {
        records.disposed += 1;
      }
    }

    const definition = createPoint10Module({
      name: "Fake WebGL Scene",
      SceneClass: GlScene,
      webgpu: false,
    });
    const module = makeModule("ready-webgl");

    await definition.prewarm({ module, width: 320, height: 180 });

    expect(records.scenes).toHaveLength(1);
    expect(records.renders).toBe(1);
    expect(mockState.webglRenderers).toBe(1);

    const context = makeContext();
    definition.draw({
      canvas: { width: 320, height: 180 },
      context,
      features: null,
      props: {},
      moduleId: "ready-webgl",
      isGallery: false,
    });

    expect(records.scenes).toHaveLength(1);
    expect(mockState.webglRenderers).toBe(1);
    expect(records.renders).toBe(2);
    expect(context.drawImage).toHaveBeenCalledTimes(1);

    removeSceneInstance("ready-webgl");
    expect(records.disposed).toBe(1);
  });
});
