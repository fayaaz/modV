import { describe, expect, it } from "vitest";
import {
  isLiveVideoTrack,
  restoreAudioSource,
  restoreVideoSources,
  selectWebcamSource,
  shouldReconnectVideoCapture,
} from "../../src/renderer/src/application/media-inputs.js";

const SYSTEM_AUDIO_ID = "modv:system-audio";

describe("isLiveVideoTrack", () => {
  it("is true for a live track even when it is muted", () => {
    expect(isLiveVideoTrack({ readyState: "live", muted: true })).toBe(true);
    expect(isLiveVideoTrack({ readyState: "live", muted: false })).toBe(true);
  });

  it("is false for an ended track", () => {
    expect(isLiveVideoTrack({ readyState: "ended" })).toBe(false);
  });

  it("is false when the track is missing", () => {
    expect(isLiveVideoTrack(undefined)).toBe(false);
    expect(isLiveVideoTrack(null)).toBe(false);
    expect(isLiveVideoTrack({})).toBe(false);
  });
});

describe("shouldReconnectVideoCapture", () => {
  it("reconnects when there is no capture at all", () => {
    expect(shouldReconnectVideoCapture(undefined)).toBe(true);
    expect(shouldReconnectVideoCapture(null)).toBe(true);
  });

  it("reconnects when the capture has no track", () => {
    expect(shouldReconnectVideoCapture({})).toBe(true);
  });

  it("reconnects when the track has ended", () => {
    expect(
      shouldReconnectVideoCapture({ track: { readyState: "ended" } }),
    ).toBe(true);
  });

  it("does not reconnect a live track, muted or not", () => {
    expect(
      shouldReconnectVideoCapture({
        track: { readyState: "live", muted: true },
      }),
    ).toBe(false);
    expect(
      shouldReconnectVideoCapture({
        track: { readyState: "live", muted: false },
      }),
    ).toBe(false);
  });
});

describe("restoreAudioSource", () => {
  const devices = [{ deviceId: "mic-a" }, { deviceId: "mic-b" }];

  it("returns a saved device id that is still present", () => {
    expect(restoreAudioSource("mic-b", devices, SYSTEM_AUDIO_ID)).toBe("mic-b");
  });

  it("returns null for a saved device id that is missing", () => {
    expect(restoreAudioSource("mic-gone", devices, SYSTEM_AUDIO_ID)).toBe(null);
  });

  it("returns null for an empty or missing saved id", () => {
    expect(restoreAudioSource(null, devices, SYSTEM_AUDIO_ID)).toBe(null);
    expect(restoreAudioSource("", devices, SYSTEM_AUDIO_ID)).toBe(null);
    expect(restoreAudioSource(undefined, devices, SYSTEM_AUDIO_ID)).toBe(null);
  });

  it("accepts the synthetic system audio id", () => {
    expect(restoreAudioSource(SYSTEM_AUDIO_ID, devices, SYSTEM_AUDIO_ID)).toBe(
      SYSTEM_AUDIO_ID,
    );
    expect(restoreAudioSource(SYSTEM_AUDIO_ID, [], SYSTEM_AUDIO_ID)).toBe(
      SYSTEM_AUDIO_ID,
    );
  });
});

describe("restoreVideoSources", () => {
  const devices = [{ deviceId: "cam-a" }, { deviceId: "cam-b" }];

  it("preserves the saved order", () => {
    expect(restoreVideoSources(["cam-b", "cam-a"], devices)).toEqual([
      "cam-b",
      "cam-a",
    ]);
  });

  it("filters out devices that are not available", () => {
    expect(restoreVideoSources(["cam-a", "cam-gone"], devices)).toEqual([
      "cam-a",
    ]);
  });

  it("de-duplicates while keeping the first occurrence's position", () => {
    expect(restoreVideoSources(["cam-a", "cam-b", "cam-a"], devices)).toEqual([
      "cam-a",
      "cam-b",
    ]);
  });

  it("ignores empty and non-string entries", () => {
    expect(
      restoreVideoSources([null, "", "cam-a", undefined, "cam-b"], devices),
    ).toEqual(["cam-a", "cam-b"]);
  });

  it("returns an empty array for missing or non-array inputs", () => {
    expect(restoreVideoSources(undefined, devices)).toEqual([]);
    expect(restoreVideoSources("cam-a", devices)).toEqual([]);
    expect(restoreVideoSources(["cam-a"], undefined)).toEqual([]);
  });
});

describe("selectWebcamSource", () => {
  const first = { id: "first" };
  const second = { id: "second" };
  const fallback = { id: "fallback" };

  it("selects 1-based from an ordered list", () => {
    expect(selectWebcamSource([first, second], 1, fallback)).toBe(first);
    expect(selectWebcamSource([first, second], 2, fallback)).toBe(second);
  });

  it("falls back to the first entry when the index is out of range", () => {
    expect(selectWebcamSource([first, second], 5, fallback)).toBe(first);
  });

  it("clamps zero and negative indexes to the first entry", () => {
    expect(selectWebcamSource([first, second], 0, fallback)).toBe(first);
    expect(selectWebcamSource([first, second], -3, fallback)).toBe(first);
  });

  it("falls back to the first entry when the index is not a number", () => {
    expect(selectWebcamSource([first, second], undefined, fallback)).toBe(
      first,
    );
    expect(selectWebcamSource([first, second], "nope", fallback)).toBe(first);
  });

  it("rounds fractional indexes", () => {
    expect(selectWebcamSource([first, second], 1.6, fallback)).toBe(second);
    expect(selectWebcamSource([first, second], "2", fallback)).toBe(second);
  });

  it("ignores null entries in the list", () => {
    expect(selectWebcamSource([null, second], 1, fallback)).toBe(second);
    expect(selectWebcamSource([undefined, null, second], 2, fallback)).toBe(
      second,
    );
  });

  it("returns the fallback when no usable entry exists", () => {
    expect(selectWebcamSource([], 1, fallback)).toBe(fallback);
    expect(selectWebcamSource([null, undefined], 1, fallback)).toBe(fallback);
    expect(selectWebcamSource(undefined, 1, fallback)).toBe(fallback);
  });
});
