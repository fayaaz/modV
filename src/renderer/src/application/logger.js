import { formatLogLine } from "../../../shared/logger-format.js";

function getIpcRenderer() {
  if (typeof window === "undefined") {
    return null;
  }

  return window.electron?.ipcRenderer || null;
}

function defaultSend(payload) {
  const ipcRenderer = getIpcRenderer();

  if (!ipcRenderer) {
    return;
  }

  ipcRenderer.send("log", payload);
}

function createLogger(scope, options = {}) {
  const { send, console: consoleImpl } = options;
  const sendLine = typeof send === "function" ? send : defaultSend;
  const target = consoleImpl || console;

  const write = (level, message, data) => {
    const payload = { level, scope, message, data };

    try {
      const line = formatLogLine({
        timestamp: new Date(),
        level,
        scope,
        message,
        data,
      });
      const method =
        level === "error"
          ? target.error
          : level === "warn"
            ? target.warn
            : target.info || target.log;

      if (typeof method === "function") {
        method.call(target, line);
      }
    } catch (e) {
      // Logging must never throw.
    }

    try {
      sendLine(payload);
    } catch (e) {
      // The main process may not be listening yet; the console copy is enough.
    }
  };

  return {
    info(message, data) {
      write("info", message, data);
    },
    warn(message, data) {
      write("warn", message, data);
    },
    error(message, data) {
      write("error", message, data);
    },
  };
}

export { createLogger, formatLogLine };
