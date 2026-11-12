/**
 * Renderer-side preview plumbing for engine mode (WS-14, M6).
 *
 * With `MODV_ENGINE=1` the renderer creates no GPU context, so the main-window
 * preview panel and the Gallery thumbnails draw frames the engine composited:
 *
 * - the output preview travels through the `VMP1` shared-memory ring and the
 *   main process hands the renderer `{ info, frame, pixels }` (raw bytes);
 * - the per-module Gallery preview travels as base64 RGBA8 over the control
 *   socket and is decoded here, in the renderer.
 *
 * These helpers are pure (no DOM): they normalise both shapes into
 * `{ width, height, data }` with a tightly packed `Uint8ClampedArray`, which is
 * exactly what `new ImageData(data, width, height)` wants. `shouldRefresh` is
 * the pure throttle both components use.
 */

import { describe, expect, it } from "vitest";

import {
  decodeBase64Pixels,
  modulePreviewFromResponse,
  outputPreviewFromResponse,
  shouldRefresh,
  toFrameData,
} from "../../src/renderer/src/application/engine-client.js";

const FORMAT_RGBA8 = 1;
const FORMAT_BGRA8 = 2;

/** 2×2 RGBA frame: red, green, blue, white. */
const rgba2x2 = Uint8Array.from([
  255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
]);

const base64 = (bytes) => Buffer.from(bytes).toString("base64");

describe("decodeBase64Pixels", () => {
  it("decodes a base64 RGBA frame to bytes", () => {
    const decoded = decodeBase64Pixels(base64(rgba2x2));

    expect(decoded).toBeInstanceOf(Uint8Array);
    expect(Array.from(decoded)).toEqual(Array.from(rgba2x2));
  });

  it("returns null for anything that is not a non-empty string", () => {
    expect(decodeBase64Pixels(null)).toBeNull();
    expect(decodeBase64Pixels(undefined)).toBeNull();
    expect(decodeBase64Pixels("")).toBeNull();
    expect(decodeBase64Pixels(Uint8Array.from([1, 2, 3]))).toBeNull();
  });
});

describe("toFrameData", () => {
  it("returns a tightly packed Uint8ClampedArray copy for a tight frame", () => {
    const source = rgba2x2;

    const frame = toFrameData(source, { width: 2, height: 2 });

    expect(frame).not.toBeNull();
    expect(frame.width).toBe(2);
    expect(frame.height).toBe(2);
    expect(frame.data).toBeInstanceOf(Uint8ClampedArray);
    expect(Array.from(frame.data)).toEqual(Array.from(source));
    // A copy, not a view of the IPC buffer.
    frame.data[0] = 7;
    expect(source[0]).toBe(255);
  });

  it("drops row padding when stride exceeds width * 4", () => {
    // 2×2 RGBA with 12-byte rows (4 bytes of padding per row).
    const row0 = [1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0];
    const row1 = [9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0];

    const frame = toFrameData(Uint8Array.from([...row0, ...row1]), {
      width: 2,
      height: 2,
      stride: 12,
    });

    expect(frame).not.toBeNull();
    expect(Array.from(frame.data)).toEqual([
      1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
    ]);
  });

  it("swaps B and R for a BGRA8 frame", () => {
    const bgra = Uint8Array.from([0, 0, 255, 255, 255, 0, 0, 255]);
    const frame = toFrameData(bgra, {
      width: 2,
      height: 1,
      format: FORMAT_BGRA8,
    });

    expect(frame).not.toBeNull();
    expect(Array.from(frame.data)).toEqual([255, 0, 0, 255, 0, 0, 255, 255]);
  });

  it("leaves RGBA8 frames untouched and rejects short buffers or bad geometry", () => {
    const rgba = toFrameData(rgba2x2, {
      width: 2,
      height: 2,
      format: FORMAT_RGBA8,
    });
    expect(Array.from(rgba.data)).toEqual(Array.from(rgba2x2));

    expect(toFrameData(rgba2x2.subarray(0, 12), { width: 2, height: 2 })).toBe(
      null,
    );
    expect(toFrameData(rgba2x2, { width: 0, height: 2 })).toBeNull();
    expect(toFrameData(rgba2x2, { width: 2.5, height: 2 })).toBeNull();
    expect(toFrameData(rgba2x2, {})).toBeNull();
    expect(toFrameData(null, { width: 2, height: 2 })).toBeNull();
  });
});

describe("outputPreviewFromResponse", () => {
  it("reads geometry from the preview/frame event payload", () => {
    const frame = outputPreviewFromResponse({
      info: { width: 320, height: 180, stride: 1280, format: FORMAT_RGBA8 },
      frame: { width: 2, height: 2, stride: 8, format: FORMAT_RGBA8 },
      pixels: rgba2x2,
    });

    expect(frame).toEqual({
      width: 2,
      height: 2,
      data: new Uint8ClampedArray(rgba2x2),
    });
  });

  it("falls back to the ring handshake geometry", () => {
    const frame = outputPreviewFromResponse({
      info: { width: 2, height: 2, stride: 8, format: FORMAT_RGBA8 },
      frame: null,
      pixels: rgba2x2,
    });

    expect(frame.width).toBe(2);
    expect(frame.height).toBe(2);
  });

  it("returns null when the engine has not produced a frame yet", () => {
    expect(outputPreviewFromResponse(null)).toBeNull();
    expect(outputPreviewFromResponse({ info: null, frame: null })).toBeNull();
    expect(
      outputPreviewFromResponse({ info: null, frame: null, pixels: null }),
    ).toBeNull();
  });
});

describe("modulePreviewFromResponse", () => {
  it("decodes the base64 RGBA8 frame the engine returns", () => {
    const frame = modulePreviewFromResponse({
      width: 2,
      height: 2,
      pixels: base64(rgba2x2),
    });

    expect(frame).toEqual({
      width: 2,
      height: 2,
      data: new Uint8ClampedArray(rgba2x2),
    });
  });

  it("accepts raw bytes as well as base64", () => {
    const frame = modulePreviewFromResponse({
      width: 2,
      height: 2,
      pixels: rgba2x2,
    });

    expect(Array.from(frame.data)).toEqual(Array.from(rgba2x2));
  });

  it("bails when the payload does not match width × height × 4", () => {
    expect(
      modulePreviewFromResponse({
        width: 4,
        height: 4,
        pixels: base64(rgba2x2),
      }),
    ).toBeNull();
    expect(
      modulePreviewFromResponse({ width: 0, height: 0, pixels: "" }),
    ).toBeNull();
    expect(modulePreviewFromResponse({ pixels: base64(rgba2x2) })).toBeNull();
    expect(modulePreviewFromResponse(null)).toBeNull();
  });
});

describe("shouldRefresh", () => {
  it("allows a refresh once the interval has elapsed", () => {
    // 10 fps → 100 ms.
    expect(shouldRefresh(1000, 990, 10)).toBe(false);
    expect(shouldRefresh(1000, 950, 10)).toBe(false);
    expect(shouldRefresh(1000, 900, 10)).toBe(true);
    expect(shouldRefresh(1000, 0, 10)).toBe(true);
  });

  it("treats the very first pull (no previous frame) as due", () => {
    expect(shouldRefresh(1234, null, 15)).toBe(true);
    expect(shouldRefresh(1234, undefined, 15)).toBe(true);
  });

  it("never refreshes for a non-positive or invalid rate", () => {
    expect(shouldRefresh(10_000, 0, 0)).toBe(false);
    expect(shouldRefresh(10_000, 0, -15)).toBe(false);
    expect(shouldRefresh(10_000, 0, Number.NaN)).toBe(false);
    expect(shouldRefresh(Number.NaN, 0, 10)).toBe(false);
  });
});
