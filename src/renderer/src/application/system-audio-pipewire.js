const PIPEWIRE_LABEL = "System Audio (PipeWire monitor)";
const PCM_CHUNK_CHANNELS = 2;
const PCM_CHUNK_SAMPLE_RATE = 48000;

function getBrowserScope() {
  return typeof window !== "undefined" ? window : {};
}

export function interleavedToPlanar(input, channels) {
  if (!input) {
    return new Float32Array(0);
  }

  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  const sampleCount = Math.floor(bytes.byteLength / 4);
  const frames = Math.floor(sampleCount / channels);

  if (!frames) {
    return new Float32Array(0);
  }

  const samples = new Float32Array(frames * channels);
  const view = new DataView(
    bytes.buffer,
    bytes.byteOffset,
    frames * channels * 4,
  );

  for (let i = 0; i < samples.length; i += 1) {
    samples[i] = view.getFloat32(i * 4, true);
  }

  const planar = new Float32Array(samples.length);

  for (let channel = 0; channel < channels; channel += 1) {
    const offset = channel * frames;

    for (let frame = 0; frame < frames; frame += 1) {
      planar[offset + frame] = samples[frame * channels + channel];
    }
  }

  return planar;
}

export function createPcmStream({
  sampleRate = PCM_CHUNK_SAMPLE_RATE,
  channels = PCM_CHUNK_CHANNELS,
  MediaStream: MediaStreamCtor = getBrowserScope().MediaStream,
  MediaStreamTrackGenerator: TrackGenerator = getBrowserScope()
    .MediaStreamTrackGenerator,
  AudioData: AudioDataCtor = getBrowserScope().AudioData,
  maxQueuedChunks = 50,
} = {}) {
  if (!MediaStreamCtor || !TrackGenerator || !AudioDataCtor) {
    throw new Error(
      "The browser does not support generating an audio track from PCM data.",
    );
  }

  const generator = new TrackGenerator({ kind: "audio" });
  const writer = generator.writable.getWriter();
  const stream = new MediaStreamCtor([generator]);
  const queue = [];
  let writing = false;
  let closed = false;
  let timestamp =
    typeof performance !== "undefined" ? performance.now() * 1000 : 0;

  function pump() {
    if (writing || closed || queue.length === 0) {
      return;
    }

    const audioData = queue.shift();

    writing = true;

    writer
      .write(audioData)
      .then(() => {
        audioData.close();
        writing = false;
        pump();
      })
      .catch(() => {
        audioData.close();
        writing = false;
        closed = true;
      });
  }

  function push(bytes) {
    if (closed) {
      return;
    }

    const data = interleavedToPlanar(bytes, channels);
    const frames = data.length / channels;

    if (!frames) {
      return;
    }

    if (queue.length >= maxQueuedChunks) {
      queue.shift().close();
    }

    queue.push(
      new AudioDataCtor({
        format: "f32-planar",
        sampleRate,
        numberOfFrames: frames,
        numberOfChannels: channels,
        timestamp,
        data,
      }),
    );

    timestamp += (frames / sampleRate) * 1e6;

    pump();
  }

  async function close() {
    if (closed) {
      return;
    }

    closed = true;

    while (queue.length) {
      queue.shift().close();
    }

    try {
      await writer.close();
    } catch (e) {
      // The writer can already be closed by the track being stopped.
    }
  }

  return { stream, push, close };
}

export function createPipeWireCapture({
  ipcRenderer,
  createStream = createPcmStream,
  label = PIPEWIRE_LABEL,
} = {}) {
  let active = null;

  function stopActive() {
    if (!active) {
      return;
    }

    const current = active;

    active = null;
    ipcRenderer.removeListener("system-audio:chunk", current.onChunk);
    ipcRenderer.invoke("system-audio:stop").catch(() => {});
    current.pcm.close();
  }

  return {
    async start() {
      if (!ipcRenderer) {
        return {
          available: false,
          reason: "The system audio IPC bridge is not available.",
        };
      }

      stopActive();

      let earlyChunks = [];
      const onEarlyChunk = (_event, chunk) => earlyChunks.push(chunk);

      ipcRenderer.on("system-audio:chunk", onEarlyChunk);

      let info;

      try {
        info = await ipcRenderer.invoke("system-audio:start");
      } catch (e) {
        ipcRenderer.removeListener("system-audio:chunk", onEarlyChunk);
        earlyChunks = [];

        return { available: false, reason: e.message };
      }

      if (!info?.available) {
        ipcRenderer.removeListener("system-audio:chunk", onEarlyChunk);
        earlyChunks = [];

        return info || { available: false };
      }

      let pcm;

      try {
        pcm = createStream({
          sampleRate: info.sampleRate,
          channels: info.channels,
        });
      } catch (e) {
        ipcRenderer.removeListener("system-audio:chunk", onEarlyChunk);
        earlyChunks = [];
        ipcRenderer.invoke("system-audio:stop").catch(() => {});

        return { available: false, reason: e.message };
      }

      const onChunk = (_event, chunk) => pcm.push(chunk);

      ipcRenderer.removeListener("system-audio:chunk", onEarlyChunk);
      ipcRenderer.on("system-audio:chunk", onChunk);
      active = { pcm, onChunk };

      for (const chunk of earlyChunks) {
        onChunk(null, chunk);
      }

      earlyChunks = [];

      const [track] = pcm.stream.getAudioTracks();

      if (track) {
        const originalStop = track.stop.bind(track);

        track.stop = () => {
          stopActive();
          originalStop();
        };
      }

      return {
        ...info,
        stream: pcm.stream,
        label,
        backend: "pipewire",
      };
    },

    stop() {
      stopActive();
    },
  };
}
