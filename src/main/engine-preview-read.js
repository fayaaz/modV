// WS-14 — validated reads from the engine's VMP1 preview ring.
//
// The engine publishes each frame into a `/dev/shm` ring (`crates/modv-ipc/
// src/preview.rs`) and announces it with a `preview/frame` event carrying
// `{ slot, width, height, stride, format, sequence, frameId }`. Each ring slot
// is a 32-byte little-endian header followed by `stride * height` pixels:
//
//   | offset | type | field                         |
//   | -----: | ---- | ----------------------------- |
//   |      0 | u32  | magic `VMP1` (`0x3150_4D56`)  |
//   |      4 | u32  | width                         |
//   |      8 | u32  | height                        |
//   |     12 | u32  | stride (bytes per row)        |
//   |     16 | u32  | format (1 = RGBA8, 2 = BGRA8) |
//   |     20 | u32  | sequence number               |
//   |     24 | u64  | frame id                      |
//
// The sequence is a seqlock: the writer bumps it to an odd value before
// writing the pixels and to the following even value afterwards. A reader must
// therefore (a) bound the slot and the frame geometry from the handshake, (b)
// read the header, (c) copy exactly `stride * height` pixel bytes, (d) re-read
// the sequence and retry when it changed. Shipping a buffer allocated before
// the read (the old `Buffer.allocUnsafe` path) could hand the renderer
// uninitialised memory on a short read — and a mid-publish read produced a torn
// half-old/half-new frame (review R-12).
//
// Pure Node (no Electron), so `tests/unit/engine-preview-read.test.js` drives it
// against a real ring file.

import fs from "node:fs";

/** The per-slot header size in bytes (`modv_ipc::preview::HEADER_BYTES`). */
export const PREVIEW_HEADER_BYTES = 32;

/** Magic number `VMP1` little-endian (`'V' | 'M' | 'P' | '1'`). */
export const PREVIEW_MAGIC = 0x31504d56;

/** Pixel format: 4 × u8, R, G, B, A byte order. */
export const PREVIEW_FORMAT_RGBA8 = 1;

/** Pixel format: 4 × u8, B, G, R, A byte order. */
export const PREVIEW_FORMAT_BGRA8 = 2;

/**
 * Upper bound for one preview frame's pixels.
 *
 * The engine's main preview is 320 × 180 RGBA8 (230 KiB) and the ring caps at
 * the session window size; 64 MiB matches the control socket's frame cap and
 * stops a hostile/corrupt handshake from requesting a huge allocation.
 */
const MAX_PREVIEW_BYTES = 64 * 1024 * 1024;

/** How many seqlock read attempts before a frame is treated as torn. */
const READ_ATTEMPTS = 3;

const isPositiveInteger = (value) =>
  Number.isInteger(value) && value > 0 && value <= MAX_PREVIEW_BYTES;

const isNonNegativeInteger = (value) => Number.isInteger(value) && value >= 0;

/**
 * Validate a `preview/ready` handshake + `preview/frame` event pair and derive
 * the slot geometry to read.
 *
 * Returns `null` for anything that cannot describe a safe read: a missing/bad
 * path, a `slot` outside the ring, a non-integer/oversized `slotBytes`, a
 * frame whose geometry does not fit its slot, or a geometry that is larger than
 * the pixel cap.
 *
 * @param {object} info `preview/ready` payload (`{ path, slots, slotBytes, … }`)
 * @param {object} frame `preview/frame` payload (`{ slot, width, height, stride, format, … }`)
 * @returns {{ offset: number, slotBytes: number, width: number, height: number,
 *   stride: number, format: number, pixelBytes: number, sequence: number } | null}
 */
export function validatePreviewGeometry(info, frame) {
  if (!info || typeof info !== "object") return null;
  if (!frame || typeof frame !== "object") return null;

  const { path, slots, slotBytes } = info;
  if (typeof path !== "string" || path.length === 0) return null;
  if (!isPositiveInteger(slots)) return null;
  if (!isPositiveInteger(slotBytes)) return null;
  if (slotBytes <= PREVIEW_HEADER_BYTES) return null;

  const { slot, width, height, stride, format } = frame;
  if (!isNonNegativeInteger(slot) || slot >= slots) return null;
  if (!isPositiveInteger(width) || !isPositiveInteger(height)) return null;
  if (!isPositiveInteger(stride) || stride < width * 4) return null;
  if (format !== PREVIEW_FORMAT_RGBA8 && format !== PREVIEW_FORMAT_BGRA8) {
    return null;
  }

  // `stride * height` must fit both the slot and the pixel cap (JS numbers
  // stay exact far beyond either bound).
  const pixelBytes = stride * height;
  if (pixelBytes > slotBytes - PREVIEW_HEADER_BYTES) return null;
  if (pixelBytes > MAX_PREVIEW_BYTES) return null;

  return {
    offset: slot * slotBytes,
    slotBytes,
    width,
    height,
    stride,
    format,
    pixelBytes,
    // The sequence the writer had reached when it announced this frame; the
    // header may be newer (another frame landed in the slot since) but never
    // older.
    sequence: isNonNegativeInteger(frame.sequence) ? frame.sequence : 0,
  };
}

/** Read one slot header, or `null` when the file cannot provide it. */
function readHeader(fd, offset) {
  const header = Buffer.alloc(PREVIEW_HEADER_BYTES);
  const bytesRead = fs.readSync(fd, header, 0, PREVIEW_HEADER_BYTES, offset);
  if (bytesRead !== PREVIEW_HEADER_BYTES) return null;
  if (header.readUInt32LE(0) !== PREVIEW_MAGIC) return null;

  return {
    width: header.readUInt32LE(4),
    height: header.readUInt32LE(8),
    stride: header.readUInt32LE(12),
    format: header.readUInt32LE(16),
    sequence: header.readUInt32LE(20),
    frameId: header.readBigUInt64LE(24),
  };
}

/**
 * Read the pixels of one published preview frame.
 *
 * Returns a `Buffer` of exactly `frame.stride * frame.height` bytes, or `null`
 * when no consistent frame can be read: invalid geometry (see
 * {@link validatePreviewGeometry}), a missing/truncated ring, a bad magic (the
 * ring was unlinked or the slot was never written), an odd sequence (a write
 * is in flight) or a sequence that changed while the pixels were copied (the
 * slot was overwritten mid-read). The caller must treat `null` as "no frame
 * this poll", never as a black frame.
 *
 * @param {object} info `preview/ready` payload (its `path` is the ring file)
 * @param {object} frame `preview/frame` payload
 * @returns {Buffer | null}
 */
export function readPreviewSlot(info, frame) {
  const geometry = validatePreviewGeometry(info, frame);
  if (!geometry) return null;

  const path = info.path;
  let fd;
  try {
    fd = fs.openSync(path, "r");
  } catch (_error) {
    return null;
  }

  try {
    for (let attempt = 0; attempt < READ_ATTEMPTS; attempt += 1) {
      const header = readHeader(fd, geometry.offset);
      if (!header) return null;
      // The ring is created at a fixed size and never re-created with a
      // different geometry while it is advertised; a mismatch means the file
      // is stale/torn, not a frame we can hand out.
      if (
        header.width !== geometry.width ||
        header.height !== geometry.height ||
        header.stride !== geometry.stride ||
        header.format !== geometry.format
      ) {
        continue;
      }
      // Odd: the writer is mid-publish; even: quiescent.
      if (header.sequence % 2 !== 0) continue;
      // A header older than the announced frame never came from the same
      // publication window (sequence and frame id only move forward).
      if (header.sequence < geometry.sequence) continue;
      if (
        Number.isSafeInteger(frame.frameId) &&
        header.frameId < BigInt(frame.frameId)
      ) {
        continue;
      }

      // Allocate *after* the header told us the size, and require a complete
      // read: a short read must never ship uninitialised bytes.
      const pixels = Buffer.alloc(geometry.pixelBytes);
      const bytesRead = fs.readSync(
        fd,
        pixels,
        0,
        geometry.pixelBytes,
        geometry.offset + PREVIEW_HEADER_BYTES,
      );
      if (bytesRead !== geometry.pixelBytes) continue;

      const sequenceAfter = Buffer.alloc(4);
      const seqBytes = fs.readSync(
        fd,
        sequenceAfter,
        0,
        4,
        geometry.offset + 20,
      );
      if (seqBytes !== 4) continue;
      if (sequenceAfter.readUInt32LE(0) !== header.sequence) continue;

      return pixels;
    }
  } catch (_error) {
    return null;
  } finally {
    try {
      fs.closeSync(fd);
    } catch (_error) {
      // Already closed.
    }
  }

  return null;
}
