import { describe, expect, it } from "vitest";
import {
  buildPipeWireRecordArgs,
  createPcmChunker,
  parseDefaultSink,
} from "../../src/main/system-audio-pipewire.js";

const DEFAULT_SINK_OUTPUT = `id 47, type PipeWire:Interface:Node
    alsa.card = "5"
    media.class = "Audio/Sink"
    node.description = "PCM2704 16-bit stereo audio DAC Analogue Stereo"
  * node.name = "alsa_output.usb-Vendor_strings_are_placed_here._ADAM_Audio_D3V-00.analog-stereo"
    object.serial = "73"
`;

describe("buildPipeWireRecordArgs", () => {
  it("records the monitor of the given sink as raw f32 PCM", () => {
    expect(buildPipeWireRecordArgs({ target: "sink.name" })).toEqual([
      "--target",
      "sink.name",
      "-P",
      '{"stream.capture.sink":true}',
      "--raw",
      "--rate",
      "48000",
      "--channels",
      "2",
      "--format",
      "f32",
      "--latency",
      "50ms",
      "-",
    ]);
  });

  it("omits the target when none was resolved", () => {
    const args = buildPipeWireRecordArgs({ target: null });

    expect(args).not.toContain("--target");
    expect(args[args.length - 1]).toBe("-");
  });

  it("uses the requested format", () => {
    const args = buildPipeWireRecordArgs({
      target: "sink.name",
      sampleRate: 44100,
      channels: 1,
    });

    expect(args).toContain("44100");
    expect(args).toContain("1");
    expect(args).toContain("f32");
  });
});

describe("parseDefaultSink", () => {
  it("parses the default sink id and node name", () => {
    expect(parseDefaultSink(DEFAULT_SINK_OUTPUT)).toEqual({
      id: 47,
      name: "alsa_output.usb-Vendor_strings_are_placed_here._ADAM_Audio_D3V-00.analog-stereo",
    });
  });

  it("returns null when wpctl gave no node", () => {
    expect(parseDefaultSink("")).toBeNull();
    expect(parseDefaultSink("id 47, type PipeWire:Interface:Node")).toBeNull();
    expect(parseDefaultSink(null)).toBeNull();
  });
});

describe("createPcmChunker", () => {
  it("emits whole frames and keeps the remainder buffered", () => {
    const chunks = [];
    const chunker = createPcmChunker({
      channels: 2,
      framesPerChunk: 2,
      onChunk: (chunk) => chunks.push(chunk),
    });

    chunker.push(Buffer.alloc(16, 1));
    chunker.push(Buffer.alloc(12, 2));
    chunker.push(Buffer.alloc(4, 3));

    expect(chunks.map((chunk) => chunk.length)).toEqual([16, 16]);
    expect(chunks[0][0]).toBe(1);
    expect(chunks[1][0]).toBe(2);
    expect(chunks[1][12]).toBe(3);
  });
});
