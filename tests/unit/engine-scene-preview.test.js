/**
 * Scene hover previews in engine mode (WS-8 / WS-14).
 *
 * With `MODV_ENGINE=1` no worker exists to render a Scene cell, so `Scenes.vue`
 * polls the Rust engine through `window.engine.requestScenePreview` and paints
 * the returned frame into the cell's own 2D canvas. The response has the same
 * wire shape as `preview/module`: `{ width, height, pixels }` with `pixels` the
 * base64 of a tightly packed RGBA8 frame.
 *
 * These helpers are pure (no DOM) so they run in plain Node:
 * - `scenePreviewFromResponse` normalises the reply into `{ width, height,
 *   data }` for `new ImageData(...)`, honouring length/stride/BGRA;
 * - `scenePreviewKey` formats the legacy `sceneId:rowIndex` runtime key;
 * - `scenePreviewPollDelay` is the ~30 fps throttle/backoff;
 * - `shouldTransferScenePreviewCanvas` keeps the engine path from transferring
 *   the cell canvas (a one-way operation that would make it undrawable).
 */

import { describe, expect, it } from "vitest";

import {
  decodeBase64Pixels,
  scenePreviewFromResponse,
  scenePreviewKey,
  scenePreviewPollDelay,
  shouldTransferScenePreviewCanvas,
} from "../../src/renderer/src/application/engine-client.js";

const FORMAT_RGBA8 = 1;
const FORMAT_BGRA8 = 2;

/** 2×2 RGBA frame: red, green, blue, white. */
const rgba2x2 = Uint8Array.from([
  255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
]);

const base64 = (bytes) => Buffer.from(bytes).toString("base64");

describe("scenePreviewFromResponse", () => {
  it("decodes the base64 RGBA8 frame the engine returns", () => {
    const frame = scenePreviewFromResponse({
      width: 2,
      height: 2,
      pixels: base64(rgba2x2),
      format: FORMAT_RGBA8,
    });

    expect(frame).not.toBeNull();
    expect(frame.width).toBe(2);
    expect(frame.height).toBe(2);
    expect(frame.data).toBeInstanceOf(Uint8ClampedArray);
    expect(Array.from(frame.data)).toEqual(Array.from(rgba2x2));
  });

  it("accepts raw bytes as well as base64", () => {
    const frame = scenePreviewFromResponse({
      width: 2,
      height: 2,
      pixels: rgba2x2,
    });

    expect(Array.from(frame.data)).toEqual(Array.from(rgba2x2));
  });

  it("drops row padding when the reply carries a stride", () => {
    // 2×2 RGBA with 8-byte rows would be too short; 12-byte rows carry 4
    // bytes of padding, which the frame must not include.
    const row0 = [1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0];
    const row1 = [9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0];

    const frame = scenePreviewFromResponse({
      width: 2,
      height: 2,
      stride: 12,
      pixels: Uint8Array.from([...row0, ...row1]),
    });

    expect(frame).not.toBeNull();
    expect(Array.from(frame.data)).toEqual([
      1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
    ]);
  });

  it("swaps B and R for a BGRA8 frame", () => {
    const bgra = Uint8Array.from([
      255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ]);

    const frame = scenePreviewFromResponse({
      width: 2,
      height: 2,
      format: FORMAT_BGRA8,
      pixels: bgra,
    });

    // Blue became red in the first pixel; red became blue in the third.
    expect(Array.from(frame.data.slice(0, 4))).toEqual([0, 0, 255, 255]);
    expect(Array.from(frame.data.slice(8, 12))).toEqual([255, 0, 0, 255]);
  });

  it("refuses a payload that does not match the declared geometry", () => {
    expect(
      scenePreviewFromResponse({
        width: 2,
        height: 2,
        pixels: base64(Uint8Array.from([1, 2, 3, 4])),
      }),
    ).toBeNull();

    // Tight frames must be exactly one frame, not a longer buffer.
    expect(
      scenePreviewFromResponse({
        width: 2,
        height: 2,
        pixels: Uint8Array.from([...rgba2x2, 0]),
      }),
    ).toBeNull();

    expect(
      scenePreviewFromResponse({ width: 0, height: 2, pixels: rgba2x2 }),
    ).toBeNull();
    expect(scenePreviewFromResponse(null)).toBeNull();
    expect(
      scenePreviewFromResponse({ width: 2, height: 2, pixels: "" }),
    ).toBeNull();
  });

  it("shares the decoder the Gallery path uses", () => {
    expect(decodeBase64Pixels(base64(rgba2x2))).toEqual(rgba2x2);
  });
});

describe("scenePreviewKey", () => {
  it("formats the legacy sceneId:rowIndex runtime key", () => {
    expect(scenePreviewKey("scene-1", 0)).toBe("scene-1:0");
    expect(scenePreviewKey("scene-1", 2)).toBe("scene-1:2");
  });

  it("returns null for a missing id or an invalid row", () => {
    expect(scenePreviewKey("", 0)).toBeNull();
    expect(scenePreviewKey(null, 0)).toBeNull();
    expect(scenePreviewKey("scene-1", -1)).toBeNull();
    expect(scenePreviewKey("scene-1", 1.5)).toBeNull();
    expect(scenePreviewKey("scene-1", null)).toBeNull();
  });
});

describe("scenePreviewPollDelay", () => {
  it("pulls immediately for the first frame and at ~30 fps afterwards", () => {
    expect(scenePreviewPollDelay(1000, null)).toBe(0);
    expect(scenePreviewPollDelay(1000, 1000)).toBeCloseTo(1000 / 30, 5);
    // Already due: never negative.
    expect(scenePreviewPollDelay(1100, 1000)).toBe(0);
  });

  it("skips while a request is in flight or the document is hidden", () => {
    expect(scenePreviewPollDelay(1000, null, { inFlight: true })).toBeNull();
    expect(scenePreviewPollDelay(1000, null, { hidden: true })).toBeNull();
  });

  it("backs off 500 ms after a null/failed frame", () => {
    expect(scenePreviewPollDelay(1000, null, { failures: 1 })).toBe(500);
    expect(scenePreviewPollDelay(1000, 1000, { failures: 3 })).toBe(500);
  });

  it("recovers once the failure counter is cleared", () => {
    expect(scenePreviewPollDelay(1000, null, { failures: 0 })).toBe(0);
  });
});

describe("shouldTransferScenePreviewCanvas", () => {
  it("never transfers the canvas in engine mode", () => {
    expect(
      shouldTransferScenePreviewCanvas({
        engineMode: true,
        alreadyTransferred: false,
      }),
    ).toBe(false);
    expect(
      shouldTransferScenePreviewCanvas({
        engineMode: true,
        alreadyTransferred: true,
      }),
    ).toBe(false);
  });

  it("keeps the legacy transfer-once behaviour", () => {
    expect(
      shouldTransferScenePreviewCanvas({
        engineMode: false,
        alreadyTransferred: false,
      }),
    ).toBe(true);
    expect(
      shouldTransferScenePreviewCanvas({
        engineMode: false,
        alreadyTransferred: true,
      }),
    ).toBe(false);
  });
});
