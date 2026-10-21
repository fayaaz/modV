import Meyda from "meyda";
import constants from "./constants";
import { isSystemAudioSource } from "./system-audio";

let floatFrequencyDataArray;
let byteFrequencyDataArray;
let byteTimeDomainDataArray;
let analyserNode;

async function getMediaStream({
  audioSourceId,
  videoSourceId,
  videoResolution,
  videoFrameRate,
  audioStream,
}) {
  const audioConstraints = {};
  const videoConstraints = {};

  if (audioSourceId && !audioStream && !isSystemAudioSource(audioSourceId)) {
    audioConstraints.audio = {
      echoCancellation: { exact: false },
      deviceId: audioSourceId,
    };
  }

  if (videoSourceId) {
    const { width, height } = videoResolution || {};

    videoConstraints.video = {
      deviceId: videoSourceId,
      frameRate: {
        ideal: videoFrameRate || 60,
      },
    };

    if (width && height) {
      videoConstraints.video.width = { ideal: width };
      videoConstraints.video.height = { ideal: height };
    }
  }

  /* Ask for user media access */
  return [
    audioStream ||
      (audioConstraints.audio
        ? navigator.mediaDevices.getUserMedia(audioConstraints)
        : false),
    videoSourceId && navigator.mediaDevices.getUserMedia(videoConstraints),
  ];
}

async function setupMedia({
  audioId,
  videoId,
  useDefaultDevices = false,
  videoResolution,
  videoFrameRate,
  audioStream,
} = {}) {
  const { mediaDeviceManager } = this;
  const mediaStreamState = this._store.state.mediaStream;

  if (videoResolution === undefined) {
    videoResolution = mediaStreamState.videoResolution;
  }

  if (videoFrameRate === undefined) {
    videoFrameRate = mediaStreamState.videoFrameRate;
  }

  const mediaStreamDevices = await mediaDeviceManager.enumerateDevices();

  let audioSourceId = audioId;
  let videoSourceId = videoId;
  const isSystemAudio = isSystemAudioSource(audioId);

  if (!audioId && useDefaultDevices && mediaStreamDevices.audio.length > 0) {
    const defaultAudioDevice = mediaStreamDevices.audio.find(
      (device) => !device.isSystemAudio,
    );

    if (defaultAudioDevice) {
      audioSourceId = defaultAudioDevice.deviceId;
    }
  }

  if (!videoId && useDefaultDevices && mediaStreamDevices.video.length > 0) {
    videoSourceId = mediaStreamDevices.video[0].deviceId;
  }

  const streams = [];

  if (audioId) {
    // When switching audio device, stop the previous audio stream
    streams.push(this._audioMediaStream);

    // A previously captured system audio stream is not a device, so it is
    // tracked separately and stopped here too.
    if (this._systemAudioStream) {
      streams.push(this._systemAudioStream);
    }
  }

  // For video, do not stop existing streams when adding a new device
  // If the requested video device already exists, short-circuit
  if (videoId && this._imageCaptures && this._imageCaptures[videoSourceId]) {
    mediaDeviceManager.selectVideoDevice(videoSourceId);
    return [undefined, undefined];
  }

  for (let i = 0, len = streams.length; i < len; i++) {
    const stream = streams[i];

    if (stream) {
      const tracks = stream.getTracks();
      for (let j = 0, jLen = tracks.length; j < jLen; j++) {
        const track = tracks[j];
        track.stop();
      }
    }
  }

  if (audioId && this._systemAudioStream) {
    this._systemAudioStream = null;
  }

  const [audioMediaStream, videoMediaStream] = await Promise.all(
    await getMediaStream({
      audioSourceId,
      videoSourceId,
      videoResolution,
      videoFrameRate,
      audioStream,
    }),
  );

  // This video element is required to keep the camera alive for the ImageCapture API
  // (this._imageCapture, ./index.js)
  if (videoMediaStream) {
    // Create a hidden video element per device to keep streams alive
    const videoEl = document.createElement("video");
    videoEl.autoplay = true;
    videoEl.muted = true;
    videoEl.srcObject = videoMediaStream;
    videoEl.onloadedmetadata = () => {
      videoEl.play();
    };

    const [track] = videoMediaStream.getVideoTracks();
    if (track) {
      // Store ImageCapture per device
      if (!this._imageCaptures) this._imageCaptures = {};
      this._imageCaptures[videoSourceId] = new ImageCapture(track);

      // Report back what the device actually gave us
      if (typeof track.getSettings === "function") {
        const { width, height, frameRate } = track.getSettings();

        this._store.commit("mediaStream/SET_APPLIED_VIDEO_SETTINGS", {
          width,
          height,
          frameRate: frameRate ? Math.round(frameRate) : null,
        });
      }

      // Start async capture loop for this deviceId
      this.startCaptureForDevice && this.startCaptureForDevice(videoSourceId);
    }

    // Keep reference to the video element per deviceId for possible cleanup
    if (!this.videoStreams) this.videoStreams = {};
    // Stop and replace any existing stream for the same deviceId
    if (this.videoStreams[videoSourceId]) {
      try {
        this.videoStreams[videoSourceId].pause();
      } catch (e) {
        // Ignore
      }
      this.videoStreams[videoSourceId].srcObject = null;
    }
    this.videoStreams[videoSourceId] = videoEl;

    mediaDeviceManager.selectVideoDevice(videoSourceId);
  }

  if (audioMediaStream) {
    if (this.audioContext) {
      this.audioContext.close();
    }

    // Create new Audio Context
    this.audioContext = new window.AudioContext({
      latencyHint: "playback",
    });

    // Create new Audio Analyser
    analyserNode = this.audioContext.createAnalyser();
    analyserNode.smoothingTimeConstant = 0;
    analyserNode.fftSize = 2048;

    // Set up arrays for analyser. The time-domain buffer is the analyser's
    // fftSize (not frequencyBinCount): a shorter array would leave the rest of
    // AudioBands' wave buffer stale (see audio-bands.js).
    floatFrequencyDataArray = new Float32Array(analyserNode.frequencyBinCount);
    byteFrequencyDataArray = new Uint8Array(analyserNode.frequencyBinCount);
    byteTimeDomainDataArray = new Uint8Array(analyserNode.fftSize);

    // Create a gain node
    this.gainNode = this.audioContext.createGain();

    // Default gain
    this.gainNode.gain.value = 1;

    // Create the audio input stream (audio)
    this.audioStream =
      this.audioContext.createMediaStreamSource(audioMediaStream);

    // Connect the audio stream to the analyser (this is a passthru) (audio->(analyser))
    this.audioStream.connect(analyserNode);

    // Connect the audio stream to the gain node (audio->(analyser)->gain)
    this.audioStream.connect(this.gainNode);

    // Set up Meyda
    // eslint-disable-next-line new-cap
    this.meyda = new Meyda.createMeydaAnalyzer({
      audioContext: this.audioContext,
      source: this.gainNode,
      bufferSize: constants.AUDIO_BUFFER_SIZE,
      windowingFunction: "rect",
      featureExtractors: ["complexSpectrum"],
    });

    mediaDeviceManager.selectAudioDevice(audioSourceId);
  }

  this._audioMediaStream = audioMediaStream || this._audioMediaStream;
  this._videoMediaStream = videoMediaStream || this._videoMediaStream;

  if (isSystemAudio) {
    if (audioMediaStream) {
      this._systemAudioStream = audioMediaStream;
    } else {
      // No stream was captured, so there is no audio to feed the analyser.
      this._audioMediaStream = null;
    }
  }

  return [audioMediaStream, videoMediaStream];
}

function getFloatFrequencyData() {
  analyserNode.getFloatFrequencyData(floatFrequencyDataArray);

  return floatFrequencyDataArray;
}

function getByteFrequencyData() {
  analyserNode.getByteFrequencyData(byteFrequencyDataArray);
  return byteFrequencyDataArray;
}

function getByteTimeDomainData() {
  analyserNode.getByteTimeDomainData(byteTimeDomainDataArray);
  return byteTimeDomainDataArray;
}

export {
  setupMedia,
  getFloatFrequencyData,
  getByteFrequencyData,
  getByteTimeDomainData,
};
