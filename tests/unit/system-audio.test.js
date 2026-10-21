import { describe, expect, it, vi } from "vitest";
import {
  SYSTEM_AUDIO_SOURCE_ID,
  SYSTEM_AUDIO_SOURCE_LABEL,
  SystemAudioCapture,
  extractAudioStream,
  findMonitorDevice,
  isMonitorDeviceLabel,
  isSystemAudioSource,
} from "../../src/renderer/src/application/system-audio.js";

class FakeTrack {
  constructor(kind, label = "") {
    this.kind = kind;
    this.label = label;
    this.stopped = false;
  }

  stop() {
    this.stopped = true;
  }
}

class FakeStream {
  constructor(tracks = []) {
    this.tracks = tracks;
  }

  getTracks() {
    return this.tracks;
  }

  getAudioTracks() {
    return this.tracks.filter((track) => track.kind === "audio");
  }

  getVideoTracks() {
    return this.tracks.filter((track) => track.kind === "video");
  }
}

class FakeMediaStream extends FakeStream {}

describe("system audio source id", () => {
  it("matches only the exact system audio id", () => {
    expect(SYSTEM_AUDIO_SOURCE_ID).toBe("modv:system-audio");
    expect(SYSTEM_AUDIO_SOURCE_LABEL).toBe("System Audio (loopback)");
    expect(isSystemAudioSource(SYSTEM_AUDIO_SOURCE_ID)).toBe(true);
    expect(isSystemAudioSource("modv:system-audio-2")).toBe(false);
    expect(isSystemAudioSource("default")).toBe(false);
    expect(isSystemAudioSource(undefined)).toBe(false);
    expect(isSystemAudioSource(null)).toBe(false);
  });
});

describe("isMonitorDeviceLabel", () => {
  it("detects monitor-ish device labels", () => {
    expect(isMonitorDeviceLabel("Monitor of Built-in Audio")).toBe(true);
    expect(isMonitorDeviceLabel("System Audio")).toBe(true);
    expect(isMonitorDeviceLabel("loopback")).toBe(true);
    expect(isMonitorDeviceLabel("Built-in Loopback Audio")).toBe(true);
  });

  it("ignores regular inputs and non-strings", () => {
    expect(isMonitorDeviceLabel("USB Microphone")).toBe(false);
    expect(isMonitorDeviceLabel("")).toBe(false);
    expect(isMonitorDeviceLabel(undefined)).toBe(false);
    expect(isMonitorDeviceLabel(null)).toBe(false);
  });
});

describe("findMonitorDevice", () => {
  it("picks the first monitor-ish audio input", () => {
    const monitor = {
      deviceId: "mon-1",
      kind: "audioinput",
      label: "Monitor of Built-in Audio",
    };

    const devices = [
      { deviceId: "cam-1", kind: "videoinput", label: "HD Webcam" },
      { deviceId: "mic-1", kind: "audioinput", label: "USB Microphone" },
      monitor,
      {
        deviceId: "mon-2",
        kind: "audioinput",
        label: "Monitor of USB Audio",
      },
    ];

    expect(findMonitorDevice(devices)).toBe(monitor);
  });

  it("returns null when there is no monitor device", () => {
    expect(
      findMonitorDevice([
        { deviceId: "mic-1", kind: "audioinput", label: "USB Microphone" },
        { deviceId: "cam-1", kind: "videoinput", label: "HD Webcam" },
      ]),
    ).toBeNull();
    expect(findMonitorDevice([])).toBeNull();
    expect(findMonitorDevice(null)).toBeNull();
  });
});

describe("extractAudioStream", () => {
  it("keeps the audio tracks and stops the video tracks", () => {
    const audioTrack = new FakeTrack("audio", "System Audio");
    const videoTrack = new FakeTrack("video", "Screen");
    const displayStream = new FakeStream([videoTrack, audioTrack]);

    const stream = extractAudioStream(displayStream, {
      MediaStream: FakeMediaStream,
    });

    expect(stream).toBeInstanceOf(FakeMediaStream);
    expect(stream.getTracks()).toEqual([audioTrack]);
    expect(audioTrack.stopped).toBe(false);
    expect(videoTrack.stopped).toBe(true);
  });

  it("returns null and stops the tracks when there is no audio track", () => {
    const videoTrack = new FakeTrack("video", "Screen");
    const displayStream = new FakeStream([videoTrack]);

    const stream = extractAudioStream(displayStream, {
      MediaStream: FakeMediaStream,
    });

    expect(stream).toBeNull();
    expect(videoTrack.stopped).toBe(true);
  });
});

describe("SystemAudioCapture", () => {
  it("reports support based on getDisplayMedia", () => {
    expect(SystemAudioCapture.isSupported({ getDisplayMedia: () => {} })).toBe(
      true,
    );
    expect(SystemAudioCapture.isSupported({})).toBe(false);
    expect(SystemAudioCapture.isSupported(undefined)).toBe(false);
  });

  it("captures system audio through the display capture path", async () => {
    const audioTrack = new FakeTrack("audio", "System Audio");
    const videoTrack = new FakeTrack("video", "Screen");
    const displayStream = new FakeStream([videoTrack, audioTrack]);

    const getDisplayMedia = vi.fn(async () => displayStream);
    const capture = new SystemAudioCapture({
      mediaDevices: { getDisplayMedia },
    });

    const result = await capture.capture();

    expect(getDisplayMedia).toHaveBeenCalledWith({
      video: true,
      audio: true,
    });
    expect(result.backend).toBe("display");
    expect(result.label).toBe(SYSTEM_AUDIO_SOURCE_LABEL);
    expect(result.stream.getTracks()).toEqual([audioTrack]);
    expect(videoTrack.stopped).toBe(true);
  });

  it("falls back to a monitor device when the display capture has no audio", async () => {
    const videoTrack = new FakeTrack("video", "Screen");
    const displayStream = new FakeStream([videoTrack]);
    const monitorTrack = new FakeTrack("audio", "Monitor of Built-in Audio");
    const monitorStream = new FakeStream([monitorTrack]);

    const monitorDevice = {
      deviceId: "mon-1",
      kind: "audioinput",
      label: "Monitor of Built-in Audio",
    };

    const getUserMedia = vi.fn(async () => monitorStream);
    const mediaDevices = {
      getDisplayMedia: vi.fn(async () => displayStream),
      enumerateDevices: vi.fn(async () => [monitorDevice]),
      getUserMedia,
    };

    const capture = new SystemAudioCapture({ mediaDevices });
    const result = await capture.capture();

    expect(result.backend).toBe("monitor");
    expect(result.label).toBe("Monitor of Built-in Audio");
    expect(result.stream).toBe(monitorStream);
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: {
        deviceId: { exact: "mon-1" },
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
    });
  });

  it("rejects with a descriptive message when no capture path works", async () => {
    const videoTrack = new FakeTrack("video", "Screen");
    const displayStream = new FakeStream([videoTrack]);

    const mediaDevices = {
      getDisplayMedia: vi.fn(async () => displayStream),
      enumerateDevices: vi.fn(async () => [
        { deviceId: "mic-1", kind: "audioinput", label: "USB Microphone" },
      ]),
      getUserMedia: vi.fn(),
    };

    const capture = new SystemAudioCapture({ mediaDevices });

    await expect(capture.capture()).rejects.toThrow(
      /System audio capture is not available on this system/,
    );
    await expect(capture.capture()).rejects.toThrow(/pactl load-module/);
  });

  it("stops the tracks of the captured stream", async () => {
    const audioTrack = new FakeTrack("audio", "System Audio");
    const displayStream = new FakeStream([new FakeTrack("video"), audioTrack]);

    const capture = new SystemAudioCapture({
      mediaDevices: {
        getDisplayMedia: vi.fn(async () => displayStream),
      },
    });

    const { stream } = await capture.capture();

    capture.stop(stream);

    expect(audioTrack.stopped).toBe(true);
  });
});
