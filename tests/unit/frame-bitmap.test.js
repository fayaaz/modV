import { describe, expect, it, vi } from "vitest";
import { drawBitmapAndClose } from "../../src/renderer/src/application/worker/frame-bitmap.js";

describe("drawBitmapAndClose", () => {
  it("draws the bitmap before closing it", () => {
    const calls = [];
    const context = {
      drawImage: vi.fn(() => calls.push("draw")),
    };
    const bitmap = {
      close: vi.fn(() => calls.push("close")),
    };

    drawBitmapAndClose(context, bitmap);

    expect(context.drawImage).toHaveBeenCalledWith(bitmap, 0, 0);
    expect(bitmap.close).toHaveBeenCalledTimes(1);
    expect(calls).toEqual(["draw", "close"]);
  });

  it("closes a bitmap whose context was lost", () => {
    const bitmap = { close: vi.fn() };

    drawBitmapAndClose(undefined, bitmap);

    expect(bitmap.close).toHaveBeenCalledTimes(1);
  });

  it("tolerates bitmaps without close", () => {
    const context = { drawImage: vi.fn() };

    expect(() =>
      drawBitmapAndClose(context, { width: 1, height: 1 }),
    ).not.toThrow();
    expect(context.drawImage).toHaveBeenCalledTimes(1);
  });

  it("does nothing without a bitmap", () => {
    const context = { drawImage: vi.fn() };

    drawBitmapAndClose(context, undefined);

    expect(context.drawImage).not.toHaveBeenCalled();
  });
});
