import { beforeEach, describe, expect, it } from "vitest";
import mediaStream from "../../src/renderer/src/application/worker/store/modules/mediaStream.js";

function freshState() {
  return JSON.parse(JSON.stringify(mediaStream.state));
}

describe("mediaStream video devices", () => {
  let state;

  beforeEach(() => {
    state = freshState();
  });

  it("supports several cameras at once without duplicates", () => {
    mediaStream.mutations.SET_CURRENT_VIDEO_SOURCE(state, { videoId: "cam-a" });
    mediaStream.mutations.SET_CURRENT_VIDEO_SOURCE(state, { videoId: "cam-b" });
    mediaStream.mutations.SET_CURRENT_VIDEO_SOURCE(state, { videoId: "cam-a" });

    expect(state.selectedVideoSources).toEqual(["cam-a", "cam-b"]);
    expect(state.currentVideoSource).toBe("cam-a");
  });

  it("clears the current camera when it is removed", () => {
    mediaStream.mutations.SET_CURRENT_VIDEO_SOURCE(state, { videoId: "cam-a" });
    mediaStream.mutations.SET_CURRENT_VIDEO_SOURCE(state, { videoId: "cam-b" });
    mediaStream.mutations.REMOVE_SELECTED_VIDEO_SOURCE(state, {
      videoId: "cam-b",
    });

    expect(state.selectedVideoSources).toEqual(["cam-a"]);
    expect(state.currentVideoSource).toBe("cam-a");
  });

  it("stores resolution and frame rate per camera", () => {
    mediaStream.mutations.SET_VIDEO_SETTINGS(state, {
      deviceId: "cam-a",
      resolution: { width: 1280, height: 720 },
      frameRate: 60,
    });
    mediaStream.mutations.SET_VIDEO_SETTINGS(state, {
      deviceId: "cam-b",
      resolution: { width: 640, height: 480 },
    });

    expect(state.videoSettings["cam-a"]).toEqual({
      resolution: { width: 1280, height: 720 },
      frameRate: 60,
    });
    expect(state.videoSettings["cam-b"]).toEqual({
      resolution: { width: 640, height: 480 },
    });
  });

  it("merges a later per-camera change", () => {
    mediaStream.mutations.SET_VIDEO_SETTINGS(state, {
      deviceId: "cam-a",
      resolution: { width: 1280, height: 720 },
      frameRate: 60,
    });
    mediaStream.mutations.SET_VIDEO_SETTINGS(state, {
      deviceId: "cam-a",
      frameRate: 30,
    });

    expect(state.videoSettings["cam-a"]).toEqual({
      resolution: { width: 1280, height: 720 },
      frameRate: 30,
    });
  });

  it("stores the modes reported by each camera", () => {
    mediaStream.mutations.SET_VIDEO_CAPABILITIES(state, {
      deviceId: "cam-a",
      resolutions: [{ width: 1280, height: 720 }],
      frameRates: [{ value: 60 }],
    });
    mediaStream.mutations.SET_VIDEO_CAPABILITIES(state, {
      deviceId: "cam-b",
      resolutions: [{ width: 640, height: 480 }],
      frameRates: [{ value: 30 }],
    });

    expect(state.videoCapabilities["cam-a"]).toEqual({
      resolutions: [{ width: 1280, height: 720 }],
      frameRates: [{ value: 60 }],
    });
    expect(state.videoCapabilities["cam-b"]).toEqual({
      resolutions: [{ width: 640, height: 480 }],
      frameRates: [{ value: 30 }],
    });
  });

  it("stores the settings each camera actually applied", () => {
    mediaStream.mutations.SET_APPLIED_VIDEO_SETTINGS(state, {
      deviceId: "cam-a",
      width: 1280,
      height: 720,
      frameRate: 60,
    });

    expect(state.appliedVideoSettings["cam-a"]).toEqual({
      width: 1280,
      height: 720,
      frameRate: 60,
    });
  });
});
