export const SYSTEM_AUDIO_SOURCE_ID = "modv:system-audio";
export const SYSTEM_AUDIO_SOURCE_LABEL = "System Audio (loopback)";

export const SYSTEM_AUDIO_UNAVAILABLE_MESSAGE =
  "System audio capture is not available on this system. On Linux, expose a PulseAudio/PipeWire monitor source as an input device (for example with `pactl load-module module-remap-source master=<sink>.monitor source_name=modv_loopback source_properties=device.description=modV_Loopback`).";

const MONITOR_LABEL_PATTERN = /monitor|system audio|loopback/i;

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
  } = {}) {
    this.mediaDevices = mediaDevices;
    this.monitorDeviceId = monitorDeviceId;
    this._displayStream = null;
  }

  static isSupported(mediaDevices) {
    return typeof mediaDevices?.getDisplayMedia === "function";
  }

  async capture() {
    const mediaDevices = this.mediaDevices;

    if (!mediaDevices) {
      throw new Error(SYSTEM_AUDIO_UNAVAILABLE_MESSAGE);
    }

    if (SystemAudioCapture.isSupported(mediaDevices)) {
      const displayStream = await mediaDevices
        .getDisplayMedia({ video: true, audio: true })
        .catch(() => null);

      if (displayStream) {
        this._displayStream = displayStream;

        const stream = extractAudioStream(displayStream);

        if (stream) {
          return {
            stream,
            label: SYSTEM_AUDIO_SOURCE_LABEL,
            backend: "display",
          };
        }

        this._stopStream(displayStream);
        this._displayStream = null;
      }
    }

    const device = await this._findMonitorDevice();

    if (device) {
      const stream = await mediaDevices.getUserMedia({
        audio: {
          deviceId: { exact: device.deviceId },
          echoCancellation: false,
          noiseSuppression: false,
          autoGainControl: false,
        },
      });

      return {
        stream,
        label: device.label || SYSTEM_AUDIO_SOURCE_LABEL,
        backend: "monitor",
      };
    }

    throw new Error(SYSTEM_AUDIO_UNAVAILABLE_MESSAGE);
  }

  stop(stream) {
    this._stopStream(stream);

    if (this._displayStream && this._displayStream !== stream) {
      this._stopStream(this._displayStream);
    }

    this._displayStream = null;
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
