/**
 * Main-process validation for the engine media channels (WS-10).
 *
 * The renderer can only reach the control socket through `engine/media-*`
 * invokes. `engine-ipc.js` is bound to Electron, so the request building and
 * validation live in `src/main/engine-media-request.js` and are unit tested
 * here: every channel maps to exactly one engine method, optional numbers are
 * clamped/dropped, missing required fields are rejected before a socket write,
 * and engine errors keep their `media_*` code in the message that crosses
 * `ipcRenderer.invoke` (which only carries text).
 */

import { describe, expect, it } from "vitest";

import {
  buildEngineMediaRequest,
  ENGINE_MEDIA_CHANNEL_NAMES,
  mediaInvokeError,
  mediaRequestTimeoutMs,
} from "../../src/main/engine-media-request.js";

describe("buildEngineMediaRequest", () => {
  it("maps every channel to its engine method", () => {
    expect(buildEngineMediaRequest("engine/media-list")).toEqual({
      command: "media/list-cameras",
      payload: null,
      timeoutMs: 8000,
    });

    expect(buildEngineMediaRequest("engine/media-status")).toEqual({
      command: "media/status",
      payload: null,
      timeoutMs: 4000,
    });

    expect(
      buildEngineMediaRequest("engine/media-stop", { sourceId: "camera:x" }),
    ).toEqual({
      command: "media/stop-source",
      payload: { sourceId: "camera:x" },
      timeoutMs: 8000,
    });
  });

  it("builds a camera start request", () => {
    expect(
      buildEngineMediaRequest("engine/media-start-camera", {
        sourceId: "camera:/dev/video0",
        cameraId: "/dev/video0",
        index: 1,
        width: 1280,
        height: 720,
        fps: 60,
      }),
    ).toEqual({
      command: "media/start-camera",
      payload: {
        sourceId: "camera:/dev/video0",
        cameraId: "/dev/video0",
        index: 1,
        width: 1280,
        height: 720,
        fps: 60,
      },
      timeoutMs: 15000,
    });
  });

  it("builds a file start request with loop", () => {
    expect(
      buildEngineMediaRequest("engine/media-start-file", {
        sourceId: "video:texture-1",
        path: "/media/a.mp4",
        width: 1920,
        height: 1080,
        fps: 30,
        loop: false,
      }),
    ).toEqual({
      command: "media/start-file",
      payload: {
        sourceId: "video:texture-1",
        path: "/media/a.mp4",
        width: 1920,
        height: 1080,
        fps: 30,
        loop: false,
      },
      timeoutMs: 15000,
    });
  });

  it("builds a screen start request", () => {
    expect(
      buildEngineMediaRequest("engine/media-start-screen", {
        sourceId: "screen:42",
        nodeId: 42,
      }),
    ).toEqual({
      command: "media/start-screen",
      payload: { sourceId: "screen:42", nodeId: 42 },
      timeoutMs: 15000,
    });
  });

  it("drops unset optional numbers and defaults loop to true", () => {
    const { payload } = buildEngineMediaRequest("engine/media-start-file", {
      sourceId: "video:x",
      path: "/media/a.mp4",
    });

    expect(payload).toEqual({
      sourceId: "video:x",
      path: "/media/a.mp4",
      width: 0,
      height: 0,
      fps: 0,
      loop: true,
    });
  });

  it("requires a non-empty source id", () => {
    expect(() =>
      buildEngineMediaRequest("engine/media-start-camera", {
        cameraId: "/dev/video0",
      }),
    ).toThrow(/sourceId/);
    expect(() =>
      buildEngineMediaRequest("engine/media-stop", { sourceId: "" }),
    ).toThrow(/sourceId/);
  });

  it("requires a camera id and a file path", () => {
    expect(() =>
      buildEngineMediaRequest("engine/media-start-camera", {
        sourceId: "camera:x",
      }),
    ).toThrow(/cameraId/);
    expect(() =>
      buildEngineMediaRequest("engine/media-start-file", {
        sourceId: "video:x",
      }),
    ).toThrow(/path/);
  });

  it("requires a positive integer PipeWire node id", () => {
    expect(() =>
      buildEngineMediaRequest("engine/media-start-screen", {
        sourceId: "screen:x",
        nodeId: 0,
      }),
    ).toThrow(/nodeId/);
    expect(() =>
      buildEngineMediaRequest("engine/media-start-screen", {
        sourceId: "screen:x",
        nodeId: "42",
      }),
    ).toThrow(/nodeId/);
  });

  it("rejects unknown channels", () => {
    expect(() => buildEngineMediaRequest("engine/nope")).toThrow(
      /unknown engine media channel/,
    );
  });

  it("includes an ordered list of handled channel names", () => {
    expect(ENGINE_MEDIA_CHANNEL_NAMES).toEqual([
      "engine/media-list",
      "engine/media-start-camera",
      "engine/media-start-file",
      "engine/media-start-screen",
      "engine/media-stop",
      "engine/media-status",
    ]);
  });

  it("exposes the request timeouts it uses", () => {
    expect(mediaRequestTimeoutMs("engine/media-start-camera")).toBe(15000);
    expect(mediaRequestTimeoutMs("engine/media-status")).toBe(4000);
    expect(mediaRequestTimeoutMs("engine/media-list")).toBe(8000);
  });
});

describe("mediaInvokeError", () => {
  it("keeps the engine code in the message (invoke drops custom fields)", () => {
    const error = Object.assign(new Error("device busy"), {
      code: "media_start_failed",
    });

    const invokeError = mediaInvokeError(error, "media/start-camera");

    expect(invokeError.message).toBe("media_start_failed: device busy");
  });

  it("defaults to media_unavailable for connection failures", () => {
    const invokeError = mediaInvokeError(
      new Error("engine is not connected"),
      "media/list-cameras",
    );

    expect(invokeError.message).toBe(
      "media_unavailable: engine is not connected",
    );
  });

  it("defaults to media_start_failed for start commands without a code", () => {
    expect(
      mediaInvokeError(new Error("no such element"), "media/start-file")
        .message,
    ).toBe("media_start_failed: no such element");
  });

  it("does not duplicate an already-coded message", () => {
    expect(
      mediaInvokeError(
        new Error("media_plugin_missing: gst is missing"),
        "media/start-camera",
      ).message,
    ).toBe("media_plugin_missing: gst is missing");
  });
});
