import { execFile, spawn } from "node:child_process";

export const PIPEWIRE_SAMPLE_RATE = 48000;
export const PIPEWIRE_CHANNELS = 2;
export const PIPEWIRE_FRAMES_PER_CHUNK = 480;

const DEFAULT_SINK_INSPECT_ARGS = ["inspect", "@DEFAULT_AUDIO_SINK@"];

export function buildPipeWireRecordArgs({
  target = null,
  sampleRate = PIPEWIRE_SAMPLE_RATE,
  channels = PIPEWIRE_CHANNELS,
} = {}) {
  const args = [];

  if (target) {
    args.push("--target", target);
  }

  args.push(
    "-P",
    '{"stream.capture.sink":true}',
    "--raw",
    "--rate",
    String(sampleRate),
    "--channels",
    String(channels),
    "--format",
    "f32",
    "--latency",
    "50ms",
    "-",
  );

  return args;
}

export function parseDefaultSink(output) {
  if (typeof output !== "string" || !output) {
    return null;
  }

  const name = /^\s*(?:\*\s*)?node\.name\s*=\s*"([^"]+)"/m.exec(output);

  if (!name) {
    return null;
  }

  const id = /^\s*id\s+(\d+),/m.exec(output);

  return {
    id: id ? Number(id[1]) : null,
    name: name[1],
  };
}

export function resolveDefaultSink({
  execFileImpl = execFile,
  timeout = 2000,
} = {}) {
  return new Promise((resolve) => {
    execFileImpl(
      "wpctl",
      DEFAULT_SINK_INSPECT_ARGS,
      { timeout },
      (error, stdout) => {
        if (error) {
          resolve(null);
          return;
        }

        resolve(parseDefaultSink(stdout)?.name || null);
      },
    );
  });
}

export function createPcmChunker({
  channels = PIPEWIRE_CHANNELS,
  framesPerChunk = PIPEWIRE_FRAMES_PER_CHUNK,
  onChunk,
} = {}) {
  const frameBytes = channels * 4;
  const chunkBytes = framesPerChunk * frameBytes;
  let pending = Buffer.alloc(0);

  return {
    push(data) {
      if (!data || !data.length) {
        return;
      }

      const buffer = Buffer.isBuffer(data) ? data : Buffer.from(data);

      pending =
        pending.length === 0 ? buffer : Buffer.concat([pending, buffer]);

      while (pending.length >= chunkBytes) {
        const chunk = pending.subarray(0, chunkBytes);

        pending = pending.subarray(chunkBytes);
        onChunk(chunk);
      }
    },
  };
}

export function startPipeWireCapture({
  target = null,
  sampleRate = PIPEWIRE_SAMPLE_RATE,
  channels = PIPEWIRE_CHANNELS,
  framesPerChunk = PIPEWIRE_FRAMES_PER_CHUNK,
  spawnImpl = spawn,
  onData = () => {},
  onError = () => {},
} = {}) {
  const args = buildPipeWireRecordArgs({ target, sampleRate, channels });
  const child = spawnImpl("pw-record", args, {
    stdio: ["ignore", "pipe", "pipe"],
  });
  const chunker = createPcmChunker({
    channels,
    framesPerChunk,
    onChunk: onData,
  });
  let stopped = false;

  const stop = () => {
    if (stopped) {
      return;
    }

    stopped = true;
    child.kill("SIGTERM");
  };

  child.stdout?.on("data", (data) => {
    if (!stopped) {
      chunker.push(data);
    }
  });

  child.stderr?.on("data", (data) => {
    const message = String(data).trim();

    if (message) {
      onError(new Error(message));
    }
  });

  child.on("error", (error) => {
    if (!stopped) {
      onError(error);
    }
  });

  child.on("exit", (code, signal) => {
    if (!stopped && code !== 0) {
      onError(
        new Error(
          `pw-record exited with code ${code}${signal ? ` (${signal})` : ""}`,
        ),
      );
    }
  });

  const ready = new Promise((resolve, reject) => {
    child.once("spawn", resolve);
    child.once("error", reject);
  });

  return {
    args,
    child,
    ready,
    sampleRate,
    channels,
    stop,
  };
}
