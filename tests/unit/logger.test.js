import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it, vi } from "vitest";
import {
  createLogger,
  formatLogLine,
} from "../../src/renderer/src/application/logger.js";
import {
  createLogger as createMainLogger,
  getLogFileName,
} from "../../src/main/logger.js";

const silentConsole = {
  log: vi.fn(),
  info: vi.fn(),
  warn: vi.fn(),
  error: vi.fn(),
};

describe("formatLogLine", () => {
  it("puts the timestamp, level, scope and message on a single line", () => {
    const line = formatLogLine({
      timestamp: "2026-10-21T10:11:12.345Z",
      level: "info",
      scope: "main:display-media",
      message: "display media request received",
    });

    expect(line).toBe(
      "2026-10-21T10:11:12.345Z INFO [main:display-media] display media request received",
    );
    expect(line.split("\n")).toHaveLength(1);
  });

  it("serialises structured data as JSON on the same line", () => {
    const line = formatLogLine({
      timestamp: "2026-10-21T10:11:12.345Z",
      level: "error",
      scope: "renderer:system-audio",
      message: "capture failed",
      data: { message: "boom", code: 42 },
    });

    expect(line).toContain('{"message":"boom","code":42}');
    expect(line.split("\n")).toHaveLength(1);
  });

  it("omits the data when there is none", () => {
    const line = formatLogLine({
      timestamp: "2026-10-21T10:11:12.345Z",
      level: "warn",
      scope: "renderer:app",
      message: "no data here",
    });

    expect(line).toBe(
      "2026-10-21T10:11:12.345Z WARN [renderer:app] no data here",
    );
  });

  it("serialises errors with their message and stack", () => {
    const error = new Error("boom");
    const line = formatLogLine({
      timestamp: "2026-10-21T10:11:12.345Z",
      level: "error",
      scope: "main",
      message: "something broke",
      data: error,
    });

    expect(line).toContain('"message":"boom"');
    expect(line).toContain("stack");
  });

  it("does not throw on circular data", () => {
    const data = { name: "loop" };
    data.self = data;

    let line = "";

    expect(() => {
      line = formatLogLine({
        timestamp: "2026-10-21T10:11:12.345Z",
        level: "error",
        scope: "main",
        message: "circular",
        data,
      });
    }).not.toThrow();

    expect(line).toContain("unserialisable");
  });
});

describe("renderer createLogger", () => {
  it("sends the level, scope, message and data over the injected send function", () => {
    const send = vi.fn();
    const logger = createLogger("renderer:system-audio", {
      send,
      console: silentConsole,
    });

    logger.info("hello", { a: 1 });
    logger.warn("careful");
    logger.error("boom", { b: 2 });

    expect(send).toHaveBeenCalledTimes(3);
    expect(send).toHaveBeenNthCalledWith(1, {
      level: "info",
      scope: "renderer:system-audio",
      message: "hello",
      data: { a: 1 },
    });
    expect(send).toHaveBeenNthCalledWith(2, {
      level: "warn",
      scope: "renderer:system-audio",
      message: "careful",
      data: undefined,
    });
    expect(send).toHaveBeenNthCalledWith(3, {
      level: "error",
      scope: "renderer:system-audio",
      message: "boom",
      data: { b: 2 },
    });
  });

  it("never throws when the send function throws", () => {
    const send = vi.fn(() => {
      throw new Error("no ipc channel");
    });
    const logger = createLogger("renderer:app", {
      send,
      console: silentConsole,
    });

    expect(() => logger.info("hello")).not.toThrow();
    expect(() => logger.error("boom")).not.toThrow();
  });

  it("mirrors every line to the console", () => {
    const logger = createLogger("renderer:app", {
      send: vi.fn(),
      console: silentConsole,
    });

    logger.info("hello");
    logger.warn("careful");
    logger.error("boom");

    expect(silentConsole.info).toHaveBeenCalledTimes(1);
    expect(silentConsole.warn).toHaveBeenCalledTimes(1);
    expect(silentConsole.error).toHaveBeenCalledTimes(1);
    expect(silentConsole.info.mock.calls[0][0]).toContain(
      "INFO [renderer:app] hello",
    );
  });
});

describe("main createLogger", () => {
  it("writes the line to a daily file in the log directory", () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "modv-logs-"));

    const logger = createMainLogger("main:test", {
      directory,
      mirror: false,
      now: () => new Date("2026-10-21T10:11:12.345Z"),
    });

    logger.info("hello", { a: 1 });

    const files = fs.readdirSync(directory);

    expect(files).toEqual(["modv-2026-10-21.log"]);

    const contents = fs.readFileSync(path.join(directory, files[0]), "utf8");

    expect(contents).toContain("INFO [main:test] hello");
    expect(contents).toContain('{"a":1}');
    expect(contents.split("\n").filter(Boolean)).toHaveLength(1);

    fs.rmSync(directory, { recursive: true, force: true });
  });

  it("appends to the file instead of overwriting it", () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "modv-logs-"));

    const logger = createMainLogger("main:test", {
      directory,
      mirror: false,
      now: () => new Date("2026-10-21T10:11:12.345Z"),
    });

    logger.info("first");
    logger.error("second");

    const contents = fs.readFileSync(
      path.join(directory, "modv-2026-10-21.log"),
      "utf8",
    );

    expect(contents.split("\n").filter(Boolean)).toHaveLength(2);

    fs.rmSync(directory, { recursive: true, force: true });
  });

  it("keeps only the seven most recent daily files", () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "modv-logs-"));

    for (let day = 1; day <= 10; day += 1) {
      const name = `modv-2020-01-${String(day).padStart(2, "0")}.log`;
      fs.writeFileSync(path.join(directory, name), "old\n");
    }

    const logger = createMainLogger("main:test", {
      directory,
      mirror: false,
      now: () => new Date("2026-10-21T10:11:12.345Z"),
    });

    logger.info("hello");

    const files = fs.readdirSync(directory).sort();

    expect(files).toHaveLength(7);
    expect(files).toContain("modv-2026-10-21.log");
    expect(files).not.toContain("modv-2020-01-01.log");
    expect(files).not.toContain("modv-2020-01-02.log");
    expect(files).toContain("modv-2020-01-10.log");

    fs.rmSync(directory, { recursive: true, force: true });
  });

  it("never throws when the log directory cannot be written to", () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "modv-logs-"));
    const blocker = path.join(directory, "blocker");

    fs.writeFileSync(blocker, "not a directory");

    const logger = createMainLogger("main:test", {
      directory: path.join(blocker, "logs"),
      mirror: false,
    });

    expect(() => logger.info("hello")).not.toThrow();

    fs.rmSync(directory, { recursive: true, force: true });
  });

  it("names the log file after the date", () => {
    expect(getLogFileName(new Date(2026, 9, 21))).toBe("modv-2026-10-21.log");
    expect(getLogFileName(new Date(2026, 11, 5))).toBe("modv-2026-12-05.log");
  });
});
