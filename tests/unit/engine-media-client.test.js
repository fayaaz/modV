/**
 * Renderer client for the engine media channels (WS-10).
 *
 * `window.engine` is the preload surface; `EngineClient` adds the same
 * guarantees the preview methods have: a clear error when the surface is
 * missing, a timeout so a wedged engine cannot hang the Audio/Video panel, and
 * an error shape that `mapEngineMediaError` understands after
 * `ipcRenderer.invoke` has flattened the engine's `media_*` code into text.
 */

import { afterEach, describe, expect, it } from "vitest";

import EngineClient, {
  withTimeout,
} from "../../src/renderer/src/application/engine-client.js";
import { mapEngineMediaError } from "../../src/renderer/src/application/engine-media.js";

const CALLS = [];

function fakeEngine(overrides = {}) {
  CALLS.length = 0;

  return {
    isEnabled: () => true,
    listCameras: async () => {
      CALLS.push({ method: "listCameras" });
      return { cameras: [{ id: "/dev/video0", name: "Cam", caps: [] }] };
    },
    startEngineCamera: async (request) => {
      CALLS.push({ method: "startEngineCamera", request });
      return { sourceId: request.sourceId, index: request.index };
    },
    startEngineFile: async (request) => {
      CALLS.push({ method: "startEngineFile", request });
      return { sourceId: request.sourceId, index: 2 };
    },
    startEngineScreen: async (request) => {
      CALLS.push({ method: "startEngineScreen", request });
      return { sourceId: request.sourceId, index: 3 };
    },
    stopEngineSource: async (sourceId) => {
      CALLS.push({ method: "stopEngineSource", sourceId });
      return { sourceId };
    },
    engineMediaStatus: async () => {
      CALLS.push({ method: "engineMediaStatus" });
      return {
        sources: [
          {
            sourceId: "camera:/dev/video0",
            kind: "camera",
            index: 1,
            state: "running",
            width: 1280,
            height: 720,
            frames: 42,
            dropped: 0,
            lastError: null,
          },
        ],
      };
    },
    ...overrides,
  };
}

function clientFor(engine) {
  globalThis.window = { engine };

  return new EngineClient();
}

afterEach(() => {
  delete globalThis.window;
});

describe("withTimeout", () => {
  it("passes a fast result through", async () => {
    await expect(withTimeout(Promise.resolve("ok"), 50)).resolves.toBe("ok");
  });

  it("rejects with the requested code when the promise hangs", async () => {
    const error = await withTimeout(new Promise(() => {}), 5, {
      message: "camera start timed out",
      code: "media_start_failed",
    }).catch((reason) => reason);

    expect(error).toBeInstanceOf(Error);
    expect(error.code).toBe("media_start_failed");
    expect(error.message).toBe("media_start_failed: camera start timed out");
  });

  it("ignores a non-positive timeout", async () => {
    await expect(withTimeout(Promise.resolve(1), 0)).resolves.toBe(1);
  });
});

describe("EngineClient media methods", () => {
  it("lists engine cameras as raw DTOs", async () => {
    const client = clientFor(fakeEngine());

    await expect(client.listEngineCameras()).resolves.toEqual([
      { id: "/dev/video0", name: "Cam", caps: [] },
    ]);
  });

  it("accepts a bare array from listCameras", async () => {
    const client = clientFor(
      fakeEngine({ listCameras: async () => [{ id: "cam" }] }),
    );

    await expect(client.listEngineCameras()).resolves.toEqual([{ id: "cam" }]);
  });

  it("forwards the camera/file/screen start requests", async () => {
    const client = clientFor(fakeEngine());

    const camera = await client.startEngineCamera({
      sourceId: "camera:/dev/video0",
      cameraId: "/dev/video0",
      index: 1,
      width: 1280,
      height: 720,
      fps: 60,
    });
    const file = await client.startEngineFile({
      sourceId: "video:texture-1",
      path: "/media/a.mp4",
      loop: true,
    });
    const screen = await client.startEngineScreen({
      sourceId: "screen:42",
      nodeId: 42,
    });

    expect(camera).toEqual({ sourceId: "camera:/dev/video0", index: 1 });
    expect(file).toEqual({ sourceId: "video:texture-1", index: 2 });
    expect(screen).toEqual({ sourceId: "screen:42", index: 3 });
    expect(CALLS.map((call) => call.method)).toEqual([
      "startEngineCamera",
      "startEngineFile",
      "startEngineScreen",
    ]);
  });

  it("stops a source and skips an empty id", async () => {
    const client = clientFor(fakeEngine());

    await expect(
      client.stopEngineSource("camera:/dev/video0"),
    ).resolves.toEqual({ sourceId: "camera:/dev/video0" });
    await expect(client.stopEngineSource("")).resolves.toBeNull();
    expect(CALLS).toHaveLength(1);
  });

  it("normalises the status reply", async () => {
    const client = clientFor(fakeEngine());

    const status = await client.engineMediaStatus();

    expect(status.error).toBeNull();
    expect(status.sources).toHaveLength(1);
    expect(status.sources[0]).toMatchObject({
      sourceId: "camera:/dev/video0",
      state: "running",
      width: 1280,
      height: 720,
    });
  });

  it("explains a missing preload surface instead of throwing a TypeError", async () => {
    const client = clientFor(fakeEngine({ startEngineCamera: undefined }));

    const error = await client
      .startEngineCamera({ sourceId: "camera:x", cameraId: "x" })
      .catch((reason) => reason);

    expect(mapEngineMediaError(error)).toMatchObject({
      code: "media_unavailable",
    });
  });

  it("keeps the engine's media_* code in the rejection", async () => {
    const client = clientFor(
      fakeEngine({
        startEngineCamera: async () => {
          throw new Error("media_start_failed: device busy");
        },
      }),
    );

    const error = await client
      .startEngineCamera({ sourceId: "camera:x", cameraId: "x" })
      .catch((reason) => reason);

    expect(mapEngineMediaError(error)).toMatchObject({
      code: "media_start_failed",
      message: "device busy",
    });
  });

  it("resolves a failed status poll to an empty list with the reason", async () => {
    const client = clientFor(
      fakeEngine({
        engineMediaStatus: async () => {
          throw new Error("media_unavailable: engine is not running");
        },
      }),
    );

    const status = await client.engineMediaStatus();

    expect(status.sources).toEqual([]);
    expect(status.error).toContain("not running");
  });
});
