import { createLogger } from "./logger";

export const SYSTEM_AUDIO_SOURCE_ID = "modv:system-audio";
export const SYSTEM_AUDIO_SOURCE_LABEL = "System Audio (loopback)";

export const SYSTEM_AUDIO_UNAVAILABLE_MESSAGE =
  "System audio capture is not available on this system. On Linux, expose a PulseAudio/PipeWire monitor source as an input device (for example with `pactl load-module module-remap-source master=<sink>.monitor source_name=modv_loopback source_properties=device.description=modV_Loopback`).";

const MONITOR_LABEL_PATTERN = /monitor|system audio|loopback/i;

// Electron's desktop audio loopback only works reliably on macOS and
// Windows. On Linux it hands back a stream without an audio track and has
// been observed to take the browser process down, so the system audio is
// captured from a PulseAudio/PipeWire monitor source there instead.
const DISPLAY_LOOPBACK_PLATFORMS = ["darwin", "win32"];

function getDefaultMediaDevices() {
  if (typeof navigator === "undefined") {
    return undefined;
  }

  return navigator.mediaDevices;
}

function getDefaultMediaStream() {
  if (typeof window === "undefined") {
    return undefined;
  }

  return window.MediaStream;
}

function getDefaultPlatform() {
  if (typeof process !== "undefined" && process.platform) {
    return process.platform;
  }

  return "unknown";
}

function describeTracks(stream) {
  const tracks = stream?.getTracks?.() || [];

  return tracks.map((track) => ({
    kind: track.kind,
    label: track.label,
    readyState: track.readyState,
    settings: track.getSettings ? track.getSettings() : undefined,
  }));
}

export function isSystemAudioSource(sourceId) {
  return sourceId === SYSTEM_AUDIO_SOURCE_ID;
}

export function isMonitorDeviceLabel(label) {
  if (typeof label !== "string") {
    return false;
  }

  return MONITOR_LABEL_PATTERN.test(label);
}

export function findMonitorDevice(devices) {
  if (!Array.isArray(devices)) {
    return null;
  }

  for (const device of devices) {
    if (!device || device.kind !== "audioinput") {
      continue;
    }

    if (isMonitorDeviceLabel(device.label)) {
      return device;
    }
  }

  return null;
}

export function extractAudioStream(
  displayStream,
  { MediaStream: Ctor = getDefaultMediaStream() } = {},
) {
  if (!displayStream) {
    return null;
  }

  const audioTracks = displayStream.getAudioTracks() || [];
  const videoTracks = displayStream.getVideoTracks() || [];

  for (const track of videoTracks) {
    track.stop();
  }

  if (!audioTracks.length) {
    const remainingTracks = displayStream.getTracks() || [];

    for (const track of remainingTracks) {
      track.stop();
    }

    return null;
  }

  const Constructor = Ctor || displayStream.constructor;

  return new Constructor(audioTracks);
}

export class SystemAudioCapture {
  constructor({
    mediaDevices = getDefaultMediaDevices(),
    monitorDeviceId = null,
    platform = getDefaultPlatform(),
    logger = createLogger("renderer:system-audio"),
  } = {}) {
    this.mediaDevices = mediaDevices;
    this.monitorDeviceId = monitorDeviceId;
    this.platform = platform;
    this.logger = logger;
    this._displayStream = null;
  }

  static isSupported(mediaDevices) {
    return typeof mediaDevices?.getDisplayMedia === "function";
  }

  async capture() {
    const logger = this.logger;

    logger.info("system audio capture requested", {
      platform: this.platform,
    });

    try {
      const device = await this._findMonitorDevice();

      if (device) {
        logger.info("capturing the system audio from a monitor source", {
          deviceId: device.deviceId,
          label: device.label,
        });

        const stream = await this._captureMonitorDevice(device);

        logger.info("monitor source captured", {
          tracks: describeTracks(stream),
        });

        return {
          stream,
          label: device.label || SYSTEM_AUDIO_SOURCE_LABEL,
          backend: "monitor",
        };
      }

      logger.info("no monitor source was found", {
        devices: await this._describeDevices(),
      });

      if (!SystemAudioCapture.isSupported(this.mediaDevices)) {
        logger.warn("getDisplayMedia is not available");
      } else if (!DISPLAY_LOOPBACK_PLATFORMS.includes(this.platform)) {
        logger.warn("skipping the display capture fallback on this platform", {
          platform: this.platform,
        });
      } else {
        const displayStream = await this.mediaDevices.getDisplayMedia({
          video: true,
          audio: true,
        });

        this._displayStream = displayStream;

        logger.info("display capture returned a stream", {
          tracks: describeTracks(displayStream),
        });

        const stream = extractAudioStream(displayStream);

        if (stream) {
          logger.info("display capture provided system audio", {
            tracks: describeTracks(stream),
          });

          return {
            stream,
            label: SYSTEM_AUDIO_SOURCE_LABEL,
            backend: "display",
          };
        }

        logger.warn("the display capture returned no audio track");

        this._stopStream(displayStream);
        this._displayStream = null;
      }
    } catch (e) {
      logger.error("system audio capture failed", e);

      this._stopStream(this._displayStream);
      this._displayStream = null;

      throw e;
    }

    logger.error("no usable system audio source was found");

    throw new Error(SYSTEM_AUDIO_UNAVAILABLE_MESSAGE);
  }

  stop(stream) {
    this._stopStream(stream);

    if (this._displayStream && this._displayStream !== stream) {
      this._stopStream(this._displayStream);
    }

    this._displayStream = null;
  }

  async _captureMonitorDevice(device) {
    const stream = await this.mediaDevices.getUserMedia({
      audio: {
        deviceId: { exact: device.deviceId },
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
    });

    const tracks = stream?.getAudioTracks?.() || [];
    const live = tracks.filter((track) => track.readyState !== "ended");

    if (!live.length) {
      this._stopStream(stream);

      throw new Error(
        `The system audio source "${device.label || device.deviceId}" did not provide a live audio track.`,
      );
    }

    return stream;
  }

  async _describeDevices() {
    if (typeof this.mediaDevices?.enumerateDevices !== "function") {
      return [];
    }

    const devices = await this.mediaDevices.enumerateDevices();

    return devices.map((device) => ({
      deviceId: device.deviceId,
      kind: device.kind,
      label: device.label,
    }));
  }

  async _findMonitorDevice() {
    if (this.monitorDeviceId) {
      if (typeof this.mediaDevices?.enumerateDevices === "function") {
        const devices = await this.mediaDevices.enumerateDevices();
        const match = (devices || []).find(
          (device) => device.deviceId === this.monitorDeviceId,
        );

        if (match) {
          return match;
        }
      }

      return {
        deviceId: this.monitorDeviceId,
        label: SYSTEM_AUDIO_SOURCE_LABEL,
      };
    }

    if (typeof this.mediaDevices?.enumerateDevices !== "function") {
      return null;
    }

    const devices = await this.mediaDevices.enumerateDevices();

    return findMonitorDevice(devices);
  }

  _stopStream(stream) {
    const tracks = stream?.getTracks?.() || [];

    for (const track of tracks) {
      track.stop();
    }
  }
}
