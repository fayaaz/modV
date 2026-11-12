/**
 * Main-process reads from the engine's VMP1 preview ring (review R-12).
 *
 * The old handler allocated `Buffer.allocUnsafe(length)` *before* the read and
 * ignored `readSync`'s return value, so a short read shipped uninitialised
 * pages into `putImageData`; it also never bounded `slot` by the handshake or
 * checked the ring's seqlock, so a mid-publish slot could be handed out as a
 * torn half-old/half-new frame.
 *
 * `src/main/engine-preview-read.js` is the pure replacement: it validates the
 * `preview/ready`/`preview/frame` pair, allocates only after the header proves
 * the size, requires a complete read and re-checks the sequence. These tests
 * drive it against a real ring file with the Rust writer's layout
 * (`crates/modv-ipc/src/preview.rs`).
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  PREVIEW_FORMAT_RGBA8,
  PREVIEW_HEADER_BYTES,
  PREVIEW_MAGIC,
  readPreviewSlot,
  validatePreviewGeometry,
} from "../../src/main/engine-preview-read.js";

const WIDTH = 2;
const HEIGHT = 2;
const STRIDE = WIDTH * 4;
const SLOT_BYTES = PREVIEW_HEADER_BYTES + STRIDE * HEIGHT;
const SLOTS = 3;

const PIXELS = Buffer.from([
  1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255,
]);

function makeInfo(overrides = {}) {
  return {
    path: "/dev/shm/modv-preview-test",
    slots: SLOTS,
    slotBytes: SLOT_BYTES,
    width: WIDTH,
    height: HEIGHT,
    stride: STRIDE,
    format: PREVIEW_FORMAT_RGBA8,
    ...overrides,
  };
}

function makeFrame(overrides = {}) {
  return {
    slot: 1,
    width: WIDTH,
    height: HEIGHT,
    stride: STRIDE,
    format: PREVIEW_FORMAT_RGBA8,
    sequence: 2,
    frameId: 0,
    ...overrides,
  };
}

/** Encode a slot header the way `PreviewHeader::encode` does. */
function header({
  width = WIDTH,
  height = HEIGHT,
  stride = STRIDE,
  format = PREVIEW_FORMAT_RGBA8,
  sequence = 2,
  frameId = 0,
} = {}) {
  const bytes = Buffer.alloc(PREVIEW_HEADER_BYTES);
  bytes.writeUInt32LE(PREVIEW_MAGIC, 0);
  bytes.writeUInt32LE(width, 4);
  bytes.writeUInt32LE(height, 8);
  bytes.writeUInt32LE(stride, 12);
  bytes.writeUInt32LE(format, 16);
  bytes.writeUInt32LE(sequence, 20);
  bytes.writeBigUInt64LE(BigInt(frameId), 24);
  return bytes;
}

/** Write a complete ring file with one published frame in `slot`. */
function writeRing(dir, { slot = 1, pixels = PIXELS, ...rest } = {}) {
  const file = path.join(dir, "ring");
  const bytes = Buffer.alloc(SLOT_BYTES * SLOTS);
  header(rest).copy(bytes, slot * SLOT_BYTES);
  pixels.copy(bytes, slot * SLOT_BYTES + PREVIEW_HEADER_BYTES);
  fs.writeFileSync(file, bytes);
  return file;
}

let dir;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "modv-preview-read-"));
});

afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
  vi.restoreAllMocks();
});

describe("validatePreviewGeometry", () => {
  it("accepts the handshake/frame pair the engine publishes", () => {
    const geometry = validatePreviewGeometry(makeInfo(), makeFrame());

    expect(geometry).toEqual({
      offset: SLOT_BYTES,
      slotBytes: SLOT_BYTES,
      width: WIDTH,
      height: HEIGHT,
      stride: STRIDE,
      format: PREVIEW_FORMAT_RGBA8,
      pixelBytes: STRIDE * HEIGHT,
      sequence: 2,
    });
  });

  it("rejects a slot outside the ring", () => {
    expect(
      validatePreviewGeometry(makeInfo(), makeFrame({ slot: SLOTS })),
    ).toBeNull();
    expect(
      validatePreviewGeometry(makeInfo(), makeFrame({ slot: -1 })),
    ).toBeNull();
    expect(
      validatePreviewGeometry(makeInfo(), makeFrame({ slot: 1.5 })),
    ).toBeNull();
  });

  it("rejects geometry that does not fit the slot or is malformed", () => {
    const info = makeInfo();

    // Unknown format, bad stride, bad dimensions, missing ids.
    expect(validatePreviewGeometry(info, makeFrame({ format: 9 }))).toBeNull();
    expect(
      validatePreviewGeometry(info, makeFrame({ stride: STRIDE - 1 })),
    ).toBeNull();
    expect(validatePreviewGeometry(info, makeFrame({ width: 0 }))).toBeNull();
    expect(
      validatePreviewGeometry(info, makeFrame({ height: 2.5 })),
    ).toBeNull();
    expect(validatePreviewGeometry(null, makeFrame())).toBeNull();
    expect(validatePreviewGeometry(info, null)).toBeNull();
    expect(
      validatePreviewGeometry(makeInfo({ path: "" }), makeFrame()),
    ).toBeNull();
    expect(
      validatePreviewGeometry(makeInfo({ slots: 0 }), makeFrame()),
    ).toBeNull();
    expect(
      validatePreviewGeometry(makeInfo({ slotBytes: 16 }), makeFrame()),
    ).toBeNull();

    // A frame larger than the slot (a corrupt handshake) is refused.
    const big = makeFrame({ height: 8 });
    expect(validatePreviewGeometry(makeInfo(), big)).toBeNull();
  });
});

describe("readPreviewSlot", () => {
  it("returns exactly the published pixels", () => {
    const file = writeRing(dir);
    const pixels = readPreviewSlot(makeInfo({ path: file }), makeFrame());

    expect(pixels).toBeInstanceOf(Buffer);
    expect(pixels.length).toBe(STRIDE * HEIGHT);
    expect([...pixels]).toEqual([...PIXELS]);
  });

  it("returns null instead of uninitialised bytes on a short read", () => {
    const file = writeRing(dir);
    // Keep the 32-byte header so validation passes, drop the pixel bytes.
    fs.truncateSync(file, SLOT_BYTES);

    expect(readPreviewSlot(makeInfo({ path: file }), makeFrame())).toBeNull();
  });

  it("returns null for a missing ring file", () => {
    expect(
      readPreviewSlot(
        makeInfo({ path: path.join(dir, "does-not-exist") }),
        makeFrame(),
      ),
    ).toBeNull();
  });

  it("rejects a slot whose magic is not VMP1 (never written / rewound)", () => {
    const file = writeRing(dir);
    const fd = fs.openSync(file, "r+");
    fs.writeSync(fd, Buffer.alloc(4), 0, 4, SLOT_BYTES);
    fs.closeSync(fd);

    expect(readPreviewSlot(makeInfo({ path: file }), makeFrame())).toBeNull();
  });

  it("drops a slot with an odd sequence (a write is in flight)", () => {
    const file = writeRing(dir, { sequence: 3 });

    expect(readPreviewSlot(makeInfo({ path: file }), makeFrame())).toBeNull();
  });

  it("drops a header older than the announced frame", () => {
    const file = writeRing(dir, { sequence: 2, frameId: 1 });

    expect(
      readPreviewSlot(makeInfo({ path: file }), makeFrame({ sequence: 8 })),
    ).toBeNull();
    expect(
      readPreviewSlot(makeInfo({ path: file }), makeFrame({ frameId: 3 })),
    ).toBeNull();
  });

  it("drops a header whose geometry does not match the frame event", () => {
    const file = writeRing(dir, { stride: STRIDE + 4 });

    expect(readPreviewSlot(makeInfo({ path: file }), makeFrame())).toBeNull();
  });

  it("retries and gives up when the sequence changes mid-read", () => {
    const file = writeRing(dir);
    const readSync = fs.readSync.bind(fs);
    vi.spyOn(fs, "readSync").mockImplementation(
      (fd, buffer, offset, length, position) => {
        const bytesRead = readSync(fd, buffer, offset, length, position);
        // The second sequence read (after the pixels were copied) sees a newer
        // write: the frame is torn and must never be returned.
        if (position === SLOT_BYTES + 20) {
          buffer.writeUInt32LE(4, 0);
        }
        return bytesRead;
      },
    );

    expect(readPreviewSlot(makeInfo({ path: file }), makeFrame())).toBeNull();
  });
});
