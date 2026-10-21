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
import {
  createPipeWireCapture,
  interleavedToPlanar,
} from "../../src/renderer/src/application/system-audio-pipewire.js";

class FakeTrack {
  constructor(kind, label = "") {
    this.kind = kind;
    this.label = label;
    this.readyState = "live";
    this.stopped = false;
  }

  stop() {
    this.stopped = true;
    this.readyState = "ended";
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

const silentLogger = {
  info: vi.fn(),
  warn: vi.fn(),
  error: vi.fn(),
};

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

  it("prefers a monitor source over the display picker", async () => {
    const monitorTrack = new FakeTrack("audio", "Monitor of Built-in Audio");
    const monitorStream = new FakeStream([monitorTrack]);

    const monitorDevice = {
      deviceId: "mon-1",
      kind: "audioinput",
      label: "Monitor of Built-in Audio",
    };

    const getDisplayMedia = vi.fn();
    const getUserMedia = vi.fn(async () => monitorStream);

    const capture = new SystemAudioCapture({
      mediaDevices: {
        getDisplayMedia,
        enumerateDevices: vi.fn(async () => [monitorDevice]),
        getUserMedia,
      },
      platform: "darwin",
      logger: silentLogger,
    });

    const result = await capture.capture();

    expect(result.backend).toBe("monitor");
    expect(result.label).toBe("Monitor of Built-in Audio");
    expect(result.stream).toBe(monitorStream);
    expect(getDisplayMedia).not.toHaveBeenCalled();
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: {
        deviceId: { exact: "mon-1" },
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
    });
  });

  it("captures system audio through the display capture path", async () => {
    const audioTrack = new FakeTrack("audio", "System Audio");
    const videoTrack = new FakeTrack("video", "Screen");
    const displayStream = new FakeStream([videoTrack, audioTrack]);

    const getDisplayMedia = vi.fn(async () => displayStream);
    const capture = new SystemAudioCapture({
      mediaDevices: {
        getDisplayMedia,
        enumerateDevices: vi.fn(async () => []),
      },
      platform: "darwin",
      logger: silentLogger,
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

  it("skips the display capture fallback on linux", async () => {
    const getDisplayMedia = vi.fn();

    const capture = new SystemAudioCapture({
      mediaDevices: {
        getDisplayMedia,
        enumerateDevices: vi.fn(async () => [
          { deviceId: "mic-1", kind: "audioinput", label: "USB Microphone" },
        ]),
        getUserMedia: vi.fn(),
      },
      platform: "linux",
      logger: silentLogger,
    });

    await expect(capture.capture()).rejects.toThrow(
      /System audio capture is not available on this system/,
    );
    expect(getDisplayMedia).not.toHaveBeenCalled();
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

    const capture = new SystemAudioCapture({
      mediaDevices,
      platform: "darwin",
      logger: silentLogger,
    });

    await expect(capture.capture()).rejects.toThrow(
      /System audio capture is not available on this system/,
    );
    await expect(capture.capture()).rejects.toThrow(/pw-record/);
  });

  it("rejects when the monitor source has no live audio track", async () => {
    const monitorTrack = new FakeTrack("audio", "Monitor of Built-in Audio");
    monitorTrack.readyState = "ended";
    const monitorStream = new FakeStream([monitorTrack]);

    const monitorDevice = {
      deviceId: "mon-1",
      kind: "audioinput",
      label: "Monitor of Built-in Audio",
    };

    const capture = new SystemAudioCapture({
      mediaDevices: {
        enumerateDevices: vi.fn(async () => [monitorDevice]),
        getUserMedia: vi.fn(async () => monitorStream),
      },
      platform: "linux",
      logger: silentLogger,
    });

    await expect(capture.capture()).rejects.toThrow(/live audio track/);
    expect(monitorTrack.stopped).toBe(true);
  });

  it("propagates the original error when the capture throws", async () => {
    const failure = new Error("the picker was cancelled");

    const capture = new SystemAudioCapture({
      mediaDevices: {
        enumerateDevices: vi.fn(async () => []),
        getDisplayMedia: vi.fn(async () => {
          throw failure;
        }),
      },
      platform: "darwin",
      logger: silentLogger,
    });

    await expect(capture.capture()).rejects.toThrow(failure);
  });

  it("stops the tracks of the captured stream", async () => {
    const audioTrack = new FakeTrack("audio", "System Audio");
    const displayStream = new FakeStream([new FakeTrack("video"), audioTrack]);

    const capture = new SystemAudioCapture({
      mediaDevices: {
        getDisplayMedia: vi.fn(async () => displayStream),
        enumerateDevices: vi.fn(async () => []),
      },
      platform: "darwin",
      logger: silentLogger,
    });

    const { stream } = await capture.capture();

    capture.stop(stream);

    expect(audioTrack.stopped).toBe(true);
  });
});

describe("interleavedToPlanar", () => {
  it("deinterleaves f32 PCM into per-channel planes", () => {
    const interleaved = new Float32Array([1, 2, 3, 4, 5, 6]);
    const planar = interleavedToPlanar(new Uint8Array(interleaved.buffer), 2);

    expect(Array.from(planar)).toEqual([1, 3, 5, 2, 4, 6]);
  });

  it("ignores a trailing partial frame", () => {
    const interleaved = new Float32Array([1, 2, 3, 4, 5]);
    const planar = interleavedToPlanar(new Uint8Array(interleaved.buffer), 2);

    expect(Array.from(planar)).toEqual([1, 3, 2, 4]);
  });
});

describe("SystemAudioCapture native PipeWire fallback", () => {
  it("uses a native PipeWire capture when no monitor device is exposed", async () => {
    const track = new FakeTrack("audio", "System Audio (PipeWire)");
    const stream = new FakeStream([track]);
    const nativeCapture = {
      start: vi.fn(async () => ({
        stream,
        sampleRate: 48000,
        channels: 2,
        label: "System Audio (PipeWire monitor)",
        backend: "pipewire",
      })),
      stop: vi.fn(),
    };

    const capture = new SystemAudioCapture({
      mediaDevices: {
        enumerateDevices: vi.fn(async () => [
          { deviceId: "mic-1", kind: "audioinput", label: "USB Microphone" },
        ]),
      },
      platform: "linux",
      logger: silentLogger,
      nativeCapture,
    });

    const result = await capture.capture();

    expect(nativeCapture.start).toHaveBeenCalledTimes(1);
    expect(result.stream).toBe(stream);
    expect(result.backend).toBe("pipewire");
    expect(result.label).toBe("System Audio (PipeWire monitor)");
  });

  it("falls back to the native label when the capture omits one", async () => {
    const stream = new FakeStream([new FakeTrack("audio")]);
    const nativeCapture = {
      start: vi.fn(async () => ({ stream })),
    };

    const capture = new SystemAudioCapture({
      mediaDevices: {
        enumerateDevices: vi.fn(async () => []),
      },
      platform: "linux",
      logger: silentLogger,
      nativeCapture,
    });

    const result = await capture.capture();

    expect(result.label).toBe(SYSTEM_AUDIO_SOURCE_LABEL);
  });

  it("does not try the native capture on other platforms", async () => {
    const nativeCapture = { start: vi.fn() };

    const capture = new SystemAudioCapture({
      mediaDevices: {
        enumerateDevices: vi.fn(async () => []),
        getDisplayMedia: vi.fn(async () => {
          throw new Error("cancelled");
        }),
      },
      platform: "darwin",
      logger: silentLogger,
      nativeCapture,
    });

    await expect(capture.capture()).rejects.toThrow("cancelled");
    expect(nativeCapture.start).not.toHaveBeenCalled();
  });

  it("keeps the unavailable error when native capture is unavailable", async () => {
    const nativeCapture = {
      start: vi.fn(async () => ({
        available: false,
        reason: "pw-record is not installed",
      })),
    };

    const capture = new SystemAudioCapture({
      mediaDevices: {
        enumerateDevices: vi.fn(async () => []),
      },
      platform: "linux",
      logger: silentLogger,
      nativeCapture,
    });

    await expect(capture.capture()).rejects.toThrow(
      /System audio capture is not available on this system/,
    );
  });

  it("stops the native capture with the stream", async () => {
    const track = new FakeTrack("audio");
    const stream = new FakeStream([track]);
    const stop = vi.fn();
    const nativeCapture = {
      start: vi.fn(async () => ({ stream })),
      stop,
    };

    const capture = new SystemAudioCapture({
      mediaDevices: {
        enumerateDevices: vi.fn(async () => []),
      },
      platform: "linux",
      logger: silentLogger,
      nativeCapture,
    });

    const result = await capture.capture();

    capture.stop(result.stream);

    expect(track.stopped).toBe(true);
    expect(stop).toHaveBeenCalledTimes(1);
  });
});

describe("createPipeWireCapture", () => {
  it("streams native PCM chunks into a generated audio track", async () => {
    const listeners = new Map();
    const invoke = vi.fn(async (channel) =>
      channel === "system-audio:start"
        ? {
            available: true,
            backend: "pipewire",
            sampleRate: 48000,
            channels: 2,
          }
        : { stopped: true },
    );
    const ipcRenderer = {
      invoke,
      on: (channel, listener) => listeners.set(channel, listener),
      removeListener: (channel) => listeners.delete(channel),
    };
    const track = new FakeTrack("audio");
    const stream = new FakeStream([track]);
    const pcm = { stream, push: vi.fn(), close: vi.fn() };
    const capture = createPipeWireCapture({
      ipcRenderer,
      createStream: () => pcm,
    });

    const captured = await capture.start();

    expect(captured.stream).toBe(stream);
    expect(captured.backend).toBe("pipewire");
    expect(captured.label).toBe("System Audio (PipeWire monitor)");

    const chunk = new Uint8Array([1, 2, 3, 4]);

    listeners.get("system-audio:chunk")(null, chunk);
    expect(pcm.push).toHaveBeenCalledWith(chunk);

    capture.stop();

    expect(pcm.close).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("system-audio:stop");
    expect(listeners.has("system-audio:chunk")).toBe(false);
  });

  it("keeps chunks that arrive before the stream is ready", async () => {
    const listeners = new Map();
    const early = new Uint8Array([9, 9, 9, 9]);
    const invoke = vi.fn(async (channel) => {
      if (channel === "system-audio:start") {
        listeners.get("system-audio:chunk")(null, early);

        return {
          available: true,
          sampleRate: 48000,
          channels: 2,
        };
      }

      return { stopped: true };
    });
    const ipcRenderer = {
      invoke,
      on: (channel, listener) => listeners.set(channel, listener),
      removeListener: (channel) => listeners.delete(channel),
    };
    const stream = new FakeStream([new FakeTrack("audio")]);
    const pcm = { stream, push: vi.fn(), close: vi.fn() };
    const capture = createPipeWireCapture({
      ipcRenderer,
      createStream: () => pcm,
    });

    await capture.start();

    expect(pcm.push).toHaveBeenCalledWith(early);
  });

  it("does not create a stream when native capture is unavailable", async () => {
    const createStream = vi.fn();
    const capture = createPipeWireCapture({
      ipcRenderer: {
        invoke: vi.fn(async () => ({
          available: false,
          reason: "pw-record is not installed",
        })),
        on: vi.fn(),
        removeListener: vi.fn(),
      },
      createStream,
    });

    const result = await capture.start();

    expect(result).toEqual({
      available: false,
      reason: "pw-record is not installed",
    });
    expect(createStream).not.toHaveBeenCalled();
  });
});
