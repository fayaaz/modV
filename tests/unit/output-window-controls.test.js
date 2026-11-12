import { describe, expect, it, vi } from "vitest";

import {
  exitFullscreen,
  isFullscreen,
  isFullscreenExitEvent,
  isFullscreenToggleEvent,
  requestFullscreen,
  toggleFullscreen,
} from "../../src/renderer/src/application/output-window-controls.js";

const keyEvent = (key, modifiers = {}) => ({
  key,
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  ...modifiers,
});

function fakeDocument({ fullscreen = false, api = "standard" } = {}) {
  const root = {};
  const doc = {
    documentElement: root,
    fullscreenElement: fullscreen ? root : null,
    webkitFullscreenElement: null,
  };

  if (api === "standard" || api === "both") {
    root.requestFullscreen = vi.fn(async () => {
      doc.fullscreenElement = root;
    });
    doc.exitFullscreen = vi.fn(async () => {
      doc.fullscreenElement = null;
    });
  }

  if (api === "webkit" || api === "both") {
    root.webkitRequestFullscreen = vi.fn(() => {
      doc.webkitFullscreenElement = root;
    });
    doc.webkitExitFullscreen = vi.fn(() => {
      doc.webkitFullscreenElement = null;
    });
  }

  return doc;
}

describe("output window fullscreen controls", () => {
  it("accepts F and Shift+F but not command-modified F", () => {
    expect(isFullscreenToggleEvent(keyEvent("f"))).toBe(true);
    expect(isFullscreenToggleEvent(keyEvent("F", { shiftKey: true }))).toBe(
      true,
    );
    expect(isFullscreenToggleEvent(keyEvent("f", { ctrlKey: true }))).toBe(
      false,
    );
    expect(isFullscreenToggleEvent(keyEvent("f", { metaKey: true }))).toBe(
      false,
    );
    expect(isFullscreenToggleEvent(keyEvent("f", { altKey: true }))).toBe(
      false,
    );
    expect(isFullscreenToggleEvent(keyEvent("g"))).toBe(false);
    expect(isFullscreenToggleEvent(null)).toBe(false);
  });

  it("only treats Escape as the exit key", () => {
    expect(isFullscreenExitEvent(keyEvent("Escape"))).toBe(true);
    expect(isFullscreenExitEvent(keyEvent("f"))).toBe(false);
    expect(isFullscreenExitEvent(null)).toBe(false);
  });

  it("fullscreens the document element, not the transferred canvas", async () => {
    const doc = fakeDocument();
    const root = doc.documentElement;

    expect(isFullscreen(doc)).toBe(false);
    await expect(requestFullscreen(doc)).resolves.toBe(true);
    expect(root.requestFullscreen).toHaveBeenCalledTimes(1);
    expect(isFullscreen(doc)).toBe(true);

    await expect(exitFullscreen(doc)).resolves.toBe(true);
    expect(doc.exitFullscreen).toHaveBeenCalledTimes(1);
    expect(isFullscreen(doc)).toBe(false);
  });

  it("falls back to the WebKit-prefixed API", async () => {
    const doc = fakeDocument({ api: "webkit" });

    await expect(requestFullscreen(doc)).resolves.toBe(true);
    expect(doc.documentElement.webkitRequestFullscreen).toHaveBeenCalledTimes(
      1,
    );
    expect(isFullscreen(doc)).toBe(true);

    await expect(exitFullscreen(doc)).resolves.toBe(true);
    expect(doc.webkitExitFullscreen).toHaveBeenCalledTimes(1);
    expect(isFullscreen(doc)).toBe(false);
  });

  it("reports false when the request is rejected or unavailable", async () => {
    const rejected = fakeDocument();
    rejected.documentElement.requestFullscreen = vi.fn(async () => {
      throw new Error("gesture required");
    });

    await expect(requestFullscreen(rejected)).resolves.toBe(false);
    await expect(requestFullscreen({ documentElement: {} })).resolves.toBe(
      false,
    );
    await expect(requestFullscreen(null)).resolves.toBe(false);
    await expect(exitFullscreen(null)).resolves.toBe(false);
  });

  it("toggles between fullscreen and windowed", async () => {
    const doc = fakeDocument();

    await expect(toggleFullscreen(doc)).resolves.toBe(true);
    expect(isFullscreen(doc)).toBe(true);

    await expect(toggleFullscreen(doc)).resolves.toBe(true);
    expect(isFullscreen(doc)).toBe(false);
    expect(doc.documentElement.requestFullscreen).toHaveBeenCalledTimes(1);
    expect(doc.exitFullscreen).toHaveBeenCalledTimes(1);
  });
});
