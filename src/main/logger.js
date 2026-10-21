/* eslint-disable no-console */
import fs from "node:fs";
import path from "node:path";
import { formatLogLine } from "../shared/logger-format.js";

const LOG_FILE_PREFIX = "modv-";
const LOG_FILE_SUFFIX = ".log";
const MAX_LOG_FILES = 7;

function getElectronApp() {
  // Required lazily so this module can be imported outside of Electron.
  const { app } = require("electron");

  return app;
}

function getLogDirectory() {
  return path.join(getElectronApp().getPath("userData"), "logs");
}

function pad(value) {
  return String(value).padStart(2, "0");
}

function getLogFileName(date = new Date()) {
  const year = date.getFullYear();
  const month = pad(date.getMonth() + 1);
  const day = pad(date.getDate());

  return `${LOG_FILE_PREFIX}${year}-${month}-${day}${LOG_FILE_SUFFIX}`;
}

function getLogFilePath(date = new Date()) {
  return path.join(getLogDirectory(), getLogFileName(date));
}

function pruneLogFiles(directory) {
  try {
    const files = fs
      .readdirSync(directory)
      .filter(
        (name) =>
          name.startsWith(LOG_FILE_PREFIX) && name.endsWith(LOG_FILE_SUFFIX),
      )
      .sort();

    const excess = files.length - MAX_LOG_FILES;

    for (let i = 0; i < excess; i += 1) {
      fs.rmSync(path.join(directory, files[i]), { force: true });
    }
  } catch (e) {
    // Pruning is best effort, a failure here must not break logging.
  }
}

function mirrorToConsole(level, line) {
  try {
    if (level === "error") {
      console.error(line);
    } else if (level === "warn") {
      console.warn(line);
    } else {
      console.log(line);
    }
  } catch (e) {
    // The console may not be available in every context.
  }
}

function writeLogLine(payload = {}, options = {}) {
  const { directory, now, mirror = true } = options;
  const timestamp = now ? now() : new Date();
  const line = formatLogLine({ timestamp, ...payload });

  try {
    const target = directory || getLogDirectory();

    fs.mkdirSync(target, { recursive: true });
    fs.appendFileSync(
      path.join(target, getLogFileName(timestamp)),
      `${line}\n`,
      "utf8",
    );
    pruneLogFiles(target);
  } catch (e) {
    console.error(`[logger] could not write the log file: ${e?.message}`);
  }

  if (mirror) {
    mirrorToConsole(payload.level, line);
  }

  return line;
}

function createLogger(scope, options = {}) {
  return {
    info(message, data) {
      return writeLogLine({ level: "info", scope, message, data }, options);
    },
    warn(message, data) {
      return writeLogLine({ level: "warn", scope, message, data }, options);
    },
    error(message, data) {
      return writeLogLine({ level: "error", scope, message, data }, options);
    },
  };
}

export {
  createLogger,
  formatLogLine,
  getLogDirectory,
  getLogFileName,
  getLogFilePath,
  writeLogLine,
  MAX_LOG_FILES,
};
