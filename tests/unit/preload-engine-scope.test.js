/**
 * Which renderer windows receive the mutating `window.engine` API (review
 * R-25). Every window used to get dispatch/commit/preview/snapshot; only the
 * main window (`index.html`) drives the engine. Output windows inherit the
 * same preload but run no script of their own, splash has no preload and the
 * color picker uses its own; unknown pages must get nothing.
 */

import { describe, expect, it } from "vitest";

import {
  engineApiScope,
  isMainWindowPath,
} from "../../src/preload/engine-api-scope";

describe("isMainWindowPath", () => {
  it.each([
    "/index.html",
    "index.html",
    "/home/user/VmodV/resources/app.asar/renderer/index.html",
    "file:///home/user/VmodV/resources/app.asar/renderer/index.html",
    "/",
    "/index.html?foo=bar",
    "/index.html#section",
    "C:\\VmodV\\resources\\renderer\\index.html",
  ])("treats %s as the main window", (pathname) => {
    expect(isMainWindowPath(pathname)).toBe(true);
  });

  it.each([
    "/output-window.html",
    "/color-picker.html",
    "/splash-screen.html",
    "/renderer/output-window.html",
    "/some/other/index.html.bak",
    "/index.htm",
    "",
    undefined,
    null,
    42,
  ])("does not treat %s as the main window", (pathname) => {
    expect(isMainWindowPath(pathname)).toBe(false);
  });
});

describe("engineApiScope", () => {
  it("only grants the full engine surface to the main window", () => {
    expect(engineApiScope("/index.html")).toBe("main");
    expect(engineApiScope("/output-window.html")).toBe("none");
    expect(engineApiScope("/color-picker.html")).toBe("none");
    expect(engineApiScope("/splash-screen.html")).toBe("none");
    expect(engineApiScope("/totally-unknown.html")).toBe("none");
    expect(engineApiScope(undefined)).toBe("none");
  });
});
