/**
 * Engine-mode media capture plumbing (WS-10, renderer side).
 *
 * With `MODV_ENGINE=1` the Rust engine owns GStreamer capture for cameras,
 * video files and screen shares. The renderer keeps the device-management UI
 * and the saved-selection state (`mediaStream.selectedVideoSources`) but must
 * never open `getUserMedia`, an `ImageCapture` loop or a WebCodecs decoder.
 *
 * These helpers are pure (no DOM, no Electron) so they can be unit tested:
 *
 * - engine camera DTOs → the `mediaStream.video` device shape;
 * - stable source ids and the 1-based camera order the Webcam module's
 *   `camera` prop indexes;
 * - media path → absolute path for `media/start-file`;
 * - `media/status` normalisation/formatting and error-code mapping;
 * - the stateless reconciliation that stops removed sources and retries dead
 *   ones.
 *
 * `createEngineMediaController` is exercised with a fake `window.engine` so the
 * request shapes and notice de-duplication are pinned without Electron.
 */

import { describe, expect, it } from "vitest";

import {
  buildCameraStartRequest,
  buildFileStartRequest,
  buildScreenStartRequest,
  cameraCapabilities,
  cameraIndexForDevice,
  cameraSourceId,
  createEngineMediaController,
  describeEngineMediaError,
  engineCameraDevice,
  engineCameraDevices,
  engineMediaFailureNotice,
  formatDeviceStatus,
  formatMediaSourceStatus,
  isEngineMediaEnabled,
  mapEngineMediaError,
  mediaSourceStateLabel,
  normaliseMediaSource,
  normaliseMediaStatus,
  reconcileCameraIndex,
  reconcileEngineSources,
  screenNodeIdFromSource,
  screenSourceId,
  sourceStatusForDevice,
  toEngineMediaPath,
  videoFileSourceId,
} from "../../src/renderer/src/application/engine-media.js";

const silentLogger = { info() {}, warn() {}, error() {} };

const cameraDto = (overrides = {}) => ({
  id: "/dev/video0",
  name: "HD Pro Webcam C920",
  path: "/dev/video0",
  caps: [
    { width: 1280, height: 720, fps: 30 },
    { width: 640, height: 480, fps: 30 },
    { width: 1280, height: 720, fps: 60 },
  ],
  ...overrides,
});

describe("isEngineMediaEnabled", () => {
  it("is false without an engine surface", () => {
    expect(isEngineMediaEnabled(undefined)).toBe(false);
    expect(isEngineMediaEnabled(null)).toBe(false);
    expect(isEngineMediaEnabled({})).toBe(false);
  });

  it("is false when the engine reports it is disabled", () => {
    expect(isEngineMediaEnabled({ isEnabled: () => false })).toBe(false);
  });

  it("is true when the engine reports it is enabled", () => {
    expect(isEngineMediaEnabled({ isEnabled: () => true })).toBe(true);
  });

  it("treats a throwing isEnabled as disabled", () => {
    expect(
      isEngineMediaEnabled({
        isEnabled() {
          throw new Error("no main window");
        },
      }),
    ).toBe(false);
  });
});

describe("engineCameraDevice", () => {
  it("maps an engine camera into the mediaStream device shape", () => {
    const device = engineCameraDevice(cameraDto());

    expect(device).toMatchObject({
      deviceId: "/dev/video0",
      label: "HD Pro Webcam C920",
      kind: "videoinput",
      engine: true,
      engineCameraId: "/dev/video0",
    });
  });

  it("uses the engine id when the provider exposes no device path", () => {
    const device = engineCameraDevice(
      cameraDto({ id: "camera-1", path: undefined }),
    );

    expect(device.deviceId).toBe("camera-1");
    expect(device.engineCameraId).toBe("camera-1");
  });

  it("falls back to the id as the label", () => {
    expect(engineCameraDevice(cameraDto({ name: "" })).label).toBe(
      "/dev/video0",
    );
  });

  it("refuses cameras without any id", () => {
    expect(engineCameraDevice(undefined)).toBeNull();
    expect(engineCameraDevice({ name: "nope" })).toBeNull();
  });

  it("drops malformed entries but keeps the usable ones", () => {
    const devices = engineCameraDevices([
      cameraDto(),
      null,
      { name: "nameless" },
      cameraDto({ id: "/dev/video2", path: "/dev/video2", name: "Second" }),
    ]);

    expect(devices.map((device) => device.deviceId)).toEqual([
      "/dev/video0",
      "/dev/video2",
    ]);
  });

  it("accepts a bare array as well as { cameras }", () => {
    expect(engineCameraDevices({ cameras: [cameraDto()] })).toHaveLength(1);
    expect(engineCameraDevices([cameraDto()])).toHaveLength(1);
    expect(engineCameraDevices(undefined)).toEqual([]);
  });
});

describe("cameraCapabilities", () => {
  it("builds unique, sorted resolution and frame-rate options", () => {
    const capabilities = cameraCapabilities(cameraDto().caps);

    expect(capabilities.resolutions).toEqual([
      { width: 640, height: 480 },
      { width: 1280, height: 720 },
    ]);
    expect(capabilities.frameRates).toEqual([
      { value: 30, label: "30 fps" },
      { value: 60, label: "60 fps" },
    ]);
  });

  it("ignores unusable caps entries", () => {
    const capabilities = cameraCapabilities([
      { width: 0, height: 480, fps: 0 },
      { width: 640, fps: 0 },
      { width: 640, height: 480, fps: 0 },
      { width: 800, height: 600, fps: 25 },
      "nope",
    ]);

    expect(capabilities.resolutions).toEqual([
      { width: 640, height: 480 },
      { width: 800, height: 600 },
    ]);
    expect(capabilities.frameRates).toEqual([{ value: 25, label: "25 fps" }]);
  });

  it("returns empty lists for missing caps", () => {
    expect(cameraCapabilities(undefined)).toEqual({
      resolutions: [],
      frameRates: [],
    });
  });
});

describe("cameraSourceId / cameraIndexForDevice", () => {
  it("prefixes the device id to keep source ids unique per kind", () => {
    expect(cameraSourceId("/dev/video0")).toBe("camera:/dev/video0");
    expect(cameraSourceId("")).toBeNull();
    expect(cameraSourceId(undefined)).toBeNull();
  });

  it("derives the 1-based order from the saved selection", () => {
    const selected = ["/dev/video2", "/dev/video0", "/dev/video4"];

    expect(cameraIndexForDevice(selected, "/dev/video2")).toBe(1);
    expect(cameraIndexForDevice(selected, "/dev/video4")).toBe(3);
  });

  it("returns null for a camera outside the saved selection", () => {
    expect(cameraIndexForDevice(["/dev/video0"], "/dev/video9")).toBeNull();
    expect(cameraIndexForDevice(undefined, "/dev/video0")).toBeNull();
    expect(cameraIndexForDevice(["/dev/video0"], "")).toBeNull();
  });
});

describe("buildCameraStartRequest", () => {
  it("carries the source id, camera id, order, size and frame rate", () => {
    expect(
      buildCameraStartRequest({
        deviceId: "/dev/video0",
        selectedIds: ["/dev/video0", "/dev/video2"],
        resolution: { width: 1280, height: 720 },
        frameRate: 60,
      }),
    ).toEqual({
      sourceId: "camera:/dev/video0",
      cameraId: "/dev/video0",
      index: 1,
      width: 1280,
      height: 720,
      fps: 60,
    });
  });

  it("uses 0 for an unset resolution/frame rate (engine default)", () => {
    expect(
      buildCameraStartRequest({
        deviceId: "/dev/video0",
        selectedIds: ["/dev/video0"],
      }),
    ).toEqual({
      sourceId: "camera:/dev/video0",
      cameraId: "/dev/video0",
      index: 1,
      width: 0,
      height: 0,
      fps: 0,
    });
  });

  it("keeps a distinct engine camera id from the device id", () => {
    const request = buildCameraStartRequest({
      deviceId: "engine-cam-1",
      cameraId: "/dev/video2",
      selectedIds: ["engine-cam-1"],
    });

    expect(request.sourceId).toBe("camera:engine-cam-1");
    expect(request.cameraId).toBe("/dev/video2");
  });

  it("omits the index when the camera is not in the saved order", () => {
    const request = buildCameraStartRequest({
      deviceId: "/dev/video9",
      selectedIds: ["/dev/video0"],
    });

    expect(request).not.toHaveProperty("index");
  });

  it("returns null without a device id", () => {
    expect(buildCameraStartRequest({ selectedIds: [] })).toBeNull();
    expect(buildCameraStartRequest()).toBeNull();
  });
});

describe("reconcileCameraIndex", () => {
  it("confirms a matching engine index", () => {
    expect(
      reconcileCameraIndex({
        request: { cameraId: "/dev/video0", index: 2 },
        response: { index: 2 },
      }),
    ).toEqual({ ok: true, expected: 2, actual: 2 });
  });

  it("reports a mismatch so the Webcam module order can be fixed", () => {
    expect(
      reconcileCameraIndex({
        request: { cameraId: "/dev/video0", index: 2 },
        response: { index: 1 },
      }),
    ).toEqual({ ok: false, expected: 2, actual: 1 });
  });

  it("treats a missing response index as unknown, not a mismatch", () => {
    expect(
      reconcileCameraIndex({
        request: { cameraId: "/dev/video0", index: 2 },
        response: {},
      }),
    ).toEqual({ ok: true, expected: 2, actual: null });
  });
});

describe("toEngineMediaPath", () => {
  it("joins the media directory and the saved relative path", () => {
    expect(
      toEngineMediaPath("/home/u/.config/VmodV/media", "default/video/a.mp4"),
    ).toBe("/home/u/.config/VmodV/media/default/video/a.mp4");
  });

  it("normalises separators for the engine's platform", () => {
    expect(
      toEngineMediaPath(
        "C:\\Users\\u\\VmodV\\media",
        "default/video/a.mp4",
        "win32",
      ),
    ).toBe("C:\\Users\\u\\VmodV\\media\\default\\video\\a.mp4");
    expect(toEngineMediaPath("/media", "default\\video\\a.mp4", "posix")).toBe(
      "/media/default/video/a.mp4",
    );
  });

  it("passes an absolute path through unchanged", () => {
    expect(toEngineMediaPath("/home/u/VmodV/media", "/tmp/clip.mp4")).toBe(
      "/tmp/clip.mp4",
    );
    expect(toEngineMediaPath("C:\\media", "D:\\clips\\a.mp4", "win32")).toBe(
      "D:\\clips\\a.mp4",
    );
  });

  it("returns null when the path cannot be resolved", () => {
    expect(toEngineMediaPath(null, "default/video/a.mp4")).toBeNull();
    expect(toEngineMediaPath("/media", "")).toBeNull();
    expect(toEngineMediaPath("/media", "   ")).toBeNull();
    expect(toEngineMediaPath("/media", undefined)).toBeNull();
    expect(toEngineMediaPath("/media", "/")).toBeNull();
  });

  it("does not double the separator at the media directory root", () => {
    expect(toEngineMediaPath("/media/", "a.mp4")).toBe("/media/a.mp4");
    expect(toEngineMediaPath("/media///", "sub/a.mp4")).toBe(
      "/media/sub/a.mp4",
    );
  });
});

describe("buildFileStartRequest", () => {
  const mediaPath = "/home/u/.config/VmodV/media";

  it("sends the absolute media path with size, rate and loop", () => {
    expect(
      buildFileStartRequest({
        path: "default/video/a.mp4",
        mediaPath,
        textureId: "texture-1",
        resolution: { width: 1920, height: 1080 },
        frameRate: 30,
      }),
    ).toEqual({
      sourceId: "video:texture-1",
      path: "/home/u/.config/VmodV/media/default/video/a.mp4",
      width: 1920,
      height: 1080,
      fps: 30,
      loop: true,
    });
  });

  it("derives a stable source id from the path without a texture id", () => {
    expect(buildFileStartRequest({ path: "a.mp4", mediaPath }).sourceId).toBe(
      "video:/home/u/.config/VmodV/media/a.mp4",
    );
    expect(videoFileSourceId("texture-1")).toBe("video:texture-1");
    expect(videoFileSourceId("")).toBeNull();
  });

  it("honours an explicit source id and loop=false", () => {
    const request = buildFileStartRequest({
      path: "a.mp4",
      mediaPath,
      sourceId: "video:custom",
      loop: false,
    });

    expect(request.sourceId).toBe("video:custom");
    expect(request.loop).toBe(false);
  });

  it("returns null when the path cannot be resolved", () => {
    expect(buildFileStartRequest({ path: "", mediaPath })).toBeNull();
    expect(
      buildFileStartRequest({ path: "a.mp4", mediaPath: null }),
    ).toBeNull();
  });
});

describe("screen sources", () => {
  it("builds a screen start request from a PipeWire node id", () => {
    expect(buildScreenStartRequest({ nodeId: 42 })).toEqual({
      sourceId: "screen:42",
      nodeId: 42,
    });
    expect(screenSourceId(42)).toBe("screen:42");
  });

  it("refuses invalid node ids", () => {
    expect(buildScreenStartRequest({ nodeId: 0 })).toBeNull();
    expect(buildScreenStartRequest({ nodeId: -1 })).toBeNull();
    expect(buildScreenStartRequest({ nodeId: "42" })).toBeNull();
    expect(buildScreenStartRequest({ nodeId: 1.5 })).toBeNull();
    expect(screenSourceId("42")).toBeNull();
  });

  it("finds no node id in Electron's desktopCapturer sources", () => {
    // Electron 45 exposes `screen:<display>:<n>` / `window:<id>:<n>` ids only;
    // the xdg-desktop-portal ScreenCast node id is not surfaced to the app.
    expect(
      screenNodeIdFromSource({ id: "screen:0:0", name: "Screen 1" }),
    ).toBeNull();
    expect(
      screenNodeIdFromSource({ id: "window:123:0", name: "Window" }),
    ).toBeNull();
    // A future Electron that does expose it is picked up without a code change.
    expect(screenNodeIdFromSource({ id: "screen:0:0", nodeId: 42 })).toBe(42);
    expect(
      screenNodeIdFromSource({ id: "screen:0:0", pipewireNodeId: "42" }),
    ).toBe(42);
  });
});

describe("media status normalisation", () => {
  const raw = {
    sourceId: "camera:/dev/video0",
    kind: "camera",
    index: 1,
    state: "running",
    width: 1280,
    height: 720,
    frames: 120,
    dropped: 3,
    lastError: null,
  };

  it("normalises a source record", () => {
    expect(normaliseMediaSource(raw)).toEqual(raw);
  });

  it("returns null for a record without a source id", () => {
    expect(normaliseMediaSource({ state: "running" })).toBeNull();
    expect(normaliseMediaSource(undefined)).toBeNull();
  });

  it("coerces numbers and keeps an error string", () => {
    expect(
      normaliseMediaSource({
        sourceId: "camera:x",
        state: "error",
        width: "640",
        height: 480,
        frames: "7",
        dropped: undefined,
        lastError: { message: "boom" },
      }),
    ).toEqual({
      sourceId: "camera:x",
      kind: "",
      index: null,
      state: "error",
      width: 640,
      height: 480,
      frames: 7,
      dropped: 0,
      lastError: "boom",
    });
  });

  it("normalises a status payload", () => {
    expect(normaliseMediaStatus({ sources: [raw, null] })).toEqual({
      sources: [raw],
      error: null,
    });
    expect(normaliseMediaStatus(undefined)).toEqual({
      sources: [],
      error: null,
    });
  });

  it("picks the source for a camera device", () => {
    const status = normaliseMediaStatus({ sources: [raw] });

    expect(sourceStatusForDevice(status, "/dev/video0")).toEqual(raw);
    expect(sourceStatusForDevice(status, "/dev/video2")).toBeNull();
  });

  it("labels known and unknown states", () => {
    expect(mediaSourceStateLabel("running")).toBe("Live");
    expect(mediaSourceStateLabel("ERROR")).toBe("Error");
    expect(mediaSourceStateLabel("stopped")).toBe("Stopped");
    expect(mediaSourceStateLabel("bogus")).toBe("Unknown");
    expect(mediaSourceStateLabel(undefined)).toBe("Unknown");
  });

  it("formats a status line for the panel", () => {
    expect(formatMediaSourceStatus(raw)).toBe("Live 1280×720 · 3 dropped");
    expect(
      formatMediaSourceStatus({
        ...raw,
        state: "error",
        lastError: "device busy",
        dropped: 0,
      }),
    ).toBe("Error 1280×720 — device busy");
    expect(formatMediaSourceStatus(null)).toBe("");
  });

  it("formats a device status without a matching source", () => {
    const status = normaliseMediaStatus({ sources: [raw] });

    expect(formatDeviceStatus(status, "/dev/video0")).toBe(
      "Live 1280×720 · 3 dropped",
    );
    expect(formatDeviceStatus(status, "/dev/video2")).toBe("");
  });
});

describe("engine media errors", () => {
  it("keeps the engine error code and message", () => {
    const error = Object.assign(new Error("no plugin gst.video4linux2"), {
      code: "media_plugin_missing",
    });

    expect(mapEngineMediaError(error)).toMatchObject({
      code: "media_plugin_missing",
      message: "no plugin gst.video4linux2",
    });
    expect(describeEngineMediaError(error)).toContain("GStreamer plugin");
  });

  it("recovers the code from an ipc invoke error message", () => {
    // `ipcRenderer.invoke` only carries the message across the bridge.
    expect(
      mapEngineMediaError(
        new Error(
          "Error invoking remote method 'engine/media-start-camera': Error: media_start_failed: device busy",
        ),
      ),
    ).toMatchObject({ code: "media_start_failed", message: "device busy" });
  });

  it("falls back to a generic code for unknown failures", () => {
    expect(mapEngineMediaError(new Error("kaboom"))).toMatchObject({
      code: "media_start_failed",
      message: "kaboom",
    });
    expect(mapEngineMediaError(undefined).code).toBe("media_start_failed");
  });

  it("describes a failure once per source", () => {
    const notice = engineMediaFailureNotice({
      name: "HD Pro Webcam C920",
      error: Object.assign(new Error("device busy"), {
        code: "media_start_failed",
      }),
    });

    expect(notice).toContain("HD Pro Webcam C920");
    expect(notice).toContain("device busy");
  });
});

describe("reconcileEngineSources", () => {
  const tracked = {
    "camera:/dev/video0": {
      kind: "camera",
      deviceId: "/dev/video0",
      request: { sourceId: "camera:/dev/video0" },
    },
    "camera:/dev/video2": {
      kind: "camera",
      deviceId: "/dev/video2",
      request: { sourceId: "camera:/dev/video2" },
    },
  };

  it("stops a source whose device disappeared", () => {
    expect(
      reconcileEngineSources({
        tracked,
        status: normaliseMediaStatus({
          sources: [{ sourceId: "camera:/dev/video0", state: "running" }],
        }),
        selectedIds: ["/dev/video0", "/dev/video2"],
        availableIds: ["/dev/video0"],
        nowMs: 10_000,
        lastAttempt: {},
      }),
    ).toEqual({ stopSources: ["camera:/dev/video2"], restartDevices: [] });
  });

  it("stops a source the user unchecked", () => {
    expect(
      reconcileEngineSources({
        tracked,
        status: normaliseMediaStatus({
          sources: [{ sourceId: "camera:/dev/video0", state: "running" }],
        }),
        selectedIds: ["/dev/video0"],
        availableIds: ["/dev/video0", "/dev/video2"],
        nowMs: 10_000,
        lastAttempt: {},
      }),
    ).toEqual({ stopSources: ["camera:/dev/video2"], restartDevices: [] });
  });

  it("restarts a selected camera that reported an error after the backoff", () => {
    expect(
      reconcileEngineSources({
        tracked,
        status: normaliseMediaStatus({
          sources: [
            {
              sourceId: "camera:/dev/video0",
              kind: "camera",
              state: "error",
              lastError: "device busy",
            },
          ],
        }),
        selectedIds: ["/dev/video0", "/dev/video2"],
        availableIds: ["/dev/video0", "/dev/video2"],
        nowMs: 10_000,
        lastAttempt: { "/dev/video0": 4_000, "/dev/video2": 9_500 },
        retryIntervalMs: 5000,
      }),
    ).toEqual({ stopSources: [], restartDevices: ["/dev/video0"] });
  });

  it("does not restart before the backoff has elapsed", () => {
    expect(
      reconcileEngineSources({
        tracked,
        status: normaliseMediaStatus({
          sources: [{ sourceId: "camera:/dev/video0", state: "stopped" }],
        }),
        selectedIds: ["/dev/video0", "/dev/video2"],
        availableIds: ["/dev/video0", "/dev/video2"],
        nowMs: 10_000,
        lastAttempt: { "/dev/video0": 8_000, "/dev/video2": 9_500 },
        retryIntervalMs: 5000,
      }).restartDevices,
    ).toEqual([]);
  });

  it("restarts a selected camera the engine no longer tracks", () => {
    expect(
      reconcileEngineSources({
        tracked,
        status: normaliseMediaStatus({ sources: [] }),
        selectedIds: ["/dev/video0"],
        availableIds: ["/dev/video0"],
        nowMs: 10_000,
        lastAttempt: {},
      }).restartDevices,
    ).toEqual(["/dev/video0"]);
  });

  it("leaves healthy sources alone", () => {
    expect(
      reconcileEngineSources({
        tracked,
        status: normaliseMediaStatus({
          sources: [
            { sourceId: "camera:/dev/video0", state: "running" },
            { sourceId: "camera:/dev/video2", state: "running" },
          ],
        }),
        selectedIds: ["/dev/video0", "/dev/video2"],
        availableIds: ["/dev/video0", "/dev/video2"],
        nowMs: 10_000,
        lastAttempt: {},
      }),
    ).toEqual({ stopSources: [], restartDevices: [] });
  });

  it("only reconciles camera sources (files/screens are UI driven)", () => {
    expect(
      reconcileEngineSources({
        tracked: {
          "video:a.mp4": { kind: "file", deviceId: null },
          "screen:42": { kind: "screen", deviceId: null },
        },
        status: normaliseMediaStatus({ sources: [] }),
        selectedIds: [],
        availableIds: [],
        nowMs: 10_000,
        lastAttempt: {},
      }),
    ).toEqual({ stopSources: [], restartDevices: [] });
  });
});

function createFakeEngine({ cameras = [], startError = null } = {}) {
  const calls = [];
  let statusSources = [];

  const engine = {
    calls,
    setStatus(sources) {
      statusSources = sources;
    },
    isEnabled: () => true,
    async listCameras() {
      calls.push({ method: "listCameras" });
      return { cameras };
    },
    async startEngineCamera(request) {
      calls.push({ method: "startEngineCamera", request });
      if (startError) throw startError;
      return { sourceId: request.sourceId, index: request.index ?? 1 };
    },
    async startEngineFile(request) {
      calls.push({ method: "startEngineFile", request });
      return { sourceId: request.sourceId, index: 0 };
    },
    async startEngineScreen(request) {
      calls.push({ method: "startEngineScreen", request });
      return { sourceId: request.sourceId, index: 0 };
    },
    async stopEngineSource(sourceId) {
      calls.push({ method: "stopEngineSource", sourceId });
      return { sourceId };
    },
    async engineMediaStatus() {
      calls.push({ method: "engineMediaStatus" });
      return { sources: statusSources };
    },
  };

  return engine;
}

const controllerFor = (engine, options = {}) =>
  createEngineMediaController({ engine, logger: silentLogger, ...options });

describe("createEngineMediaController", () => {
  it("reports whether it is enabled", () => {
    expect(controllerFor(createFakeEngine()).isEnabled()).toBe(true);
    expect(createEngineMediaController({ engine: undefined }).isEnabled()).toBe(
      false,
    );
  });

  it("lists engine cameras as mediaStream devices", async () => {
    const engine = createFakeEngine({ cameras: [cameraDto()] });
    const controller = controllerFor(engine);

    const devices = await controller.listCameras();

    expect(devices).toHaveLength(1);
    expect(devices[0].deviceId).toBe("/dev/video0");
    expect(controller.cameras).toHaveLength(1);
  });

  it("starts a camera with the saved selection order", async () => {
    const engine = createFakeEngine();
    const controller = controllerFor(engine);

    const result = await controller.startCamera({
      deviceId: "/dev/video2",
      selectedIds: ["/dev/video0", "/dev/video2"],
      resolution: { width: 640, height: 480 },
      frameRate: 30,
    });

    expect(engine.calls[0].request).toMatchObject({
      sourceId: "camera:/dev/video2",
      cameraId: "/dev/video2",
      index: 2,
      width: 640,
      height: 480,
      fps: 30,
    });
    expect(result).toEqual({ sourceId: "camera:/dev/video2", index: 2 });
    expect(controller.isTracked("camera:/dev/video2")).toBe(true);
  });

  it("records one notice per source when a start fails", async () => {
    const engine = createFakeEngine({
      startError: Object.assign(new Error("device busy"), {
        code: "media_start_failed",
      }),
    });
    const controller = controllerFor(engine);

    await expect(
      controller.startCamera({
        deviceId: "/dev/video0",
        selectedIds: ["/dev/video0"],
      }),
    ).rejects.toThrow("device busy");

    await expect(
      controller.startCamera({
        deviceId: "/dev/video0",
        selectedIds: ["/dev/video0"],
      }),
    ).rejects.toThrow("device busy");

    expect(controller.notices()).toHaveLength(1);
    expect(controller.notices()[0].sourceId).toBe("camera:/dev/video0");
    expect(controller.notices()[0].message).toContain("device busy");

    controller.clearNotice("camera:/dev/video0");
    expect(controller.notices()).toEqual([]);
  });

  it("stops a camera by its derived source id", async () => {
    const engine = createFakeEngine();
    const controller = controllerFor(engine);

    await controller.startCamera({
      deviceId: "/dev/video0",
      selectedIds: ["/dev/video0"],
    });
    await controller.stopCamera("/dev/video0");

    expect(engine.calls.at(-1)).toEqual({
      method: "stopEngineSource",
      sourceId: "camera:/dev/video0",
    });
    expect(controller.isTracked("camera:/dev/video0")).toBe(false);
  });

  it("stops the previous video file before starting a new one", async () => {
    const engine = createFakeEngine();
    const controller = controllerFor(engine);

    await controller.startVideoFile({
      path: "a.mp4",
      mediaPath: "/media",
      textureId: "texture-1",
    });
    await controller.startVideoFile({
      path: "b.mp4",
      mediaPath: "/media",
      textureId: "texture-2",
      previousSourceId: "video:texture-1",
    });

    expect(engine.calls.map((call) => call.method)).toEqual([
      "startEngineFile",
      "stopEngineSource",
      "startEngineFile",
    ]);
    expect(engine.calls[2].request.path).toBe("/media/b.mp4");
    expect(controller.isTracked("video:texture-2")).toBe(true);
    expect(controller.isTracked("video:texture-1")).toBe(false);
  });

  it("starts a screen source from a node id and refuses without one", async () => {
    const engine = createFakeEngine();
    const controller = controllerFor(engine);

    await controller.startScreen({ nodeId: 42 });
    expect(engine.calls[0].request).toEqual({
      sourceId: "screen:42",
      nodeId: 42,
    });

    await expect(controller.startScreen({})).resolves.toBeNull();
  });

  it("stops every tracked source", async () => {
    const engine = createFakeEngine();
    const controller = controllerFor(engine);

    await controller.startCamera({
      deviceId: "/dev/video0",
      selectedIds: ["/dev/video0"],
    });
    await controller.startVideoFile({
      path: "a.mp4",
      mediaPath: "/media",
      textureId: "texture-1",
    });
    await controller.stopAll();

    const stopped = engine.calls
      .filter((call) => call.method === "stopEngineSource")
      .map((call) => call.sourceId);

    expect(stopped).toEqual(["camera:/dev/video0", "video:texture-1"]);
  });

  it("stops removed cameras and restarts dead ones while polling", async () => {
    const engine = createFakeEngine();
    const controller = controllerFor(engine, { now: () => 1000 });

    await controller.startCamera({
      deviceId: "/dev/video0",
      selectedIds: ["/dev/video0", "/dev/video2"],
    });
    await controller.startCamera({
      deviceId: "/dev/video2",
      selectedIds: ["/dev/video0", "/dev/video2"],
    });

    engine.setStatus([
      { sourceId: "camera:/dev/video0", kind: "camera", state: "error" },
    ]);

    const result = await controller.pollStatus({
      selectedIds: ["/dev/video0", "/dev/video2"],
      availableIds: ["/dev/video0"],
      nowMs: 60_000,
    });

    expect(result.stopped).toEqual(["camera:/dev/video2"]);
    expect(result.restarted).toEqual(["/dev/video0"]);
    expect(result.sources).toHaveLength(1);
  });

  it("starts selected cameras that are not tracked yet (re-scan)", async () => {
    const engine = createFakeEngine();
    const controller = controllerFor(engine);

    await controller.startCamera({
      deviceId: "/dev/video0",
      selectedIds: ["/dev/video0", "/dev/video2"],
    });

    const started = await controller.ensureCameras({
      selectedIds: ["/dev/video0", "/dev/video2", "/dev/videoGone"],
      availableIds: ["/dev/video0", "/dev/video2"],
    });

    expect(started).toEqual(["/dev/video2"]);
    expect(controller.isTracked("camera:/dev/video2")).toBe(true);
    expect(
      engine.calls.filter((c) => c.method === "startEngineCamera"),
    ).toHaveLength(2);
  });

  it("returns an error string instead of throwing while polling", async () => {
    const engine = createFakeEngine();
    engine.engineMediaStatus = async () => {
      throw new Error("media_unavailable: engine not running");
    };
    const controller = controllerFor(engine);

    const result = await controller.pollStatus({});

    expect(result.sources).toEqual([]);
    expect(result.error).toContain("not running");
  });
});
