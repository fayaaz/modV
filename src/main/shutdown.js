const WATCHDOG_TIMEOUT_MS = 3000;
const CLEANUP_TIMEOUT_MS = 1000;

let quitting = false;
let cleanupPromise = null;
let watchdogTimer = null;
let handlersRegistered = false;
let dependencies = {
  getWindows: () => ({}),
  stopMediaWatcher: () => {},
  stopSystemAudio: () => {},
};

function getApp() {
  // Required lazily so this module can be imported outside of Electron.
  const { app } = require("electron");

  return app;
}

function getBrowserWindow() {
  const { BrowserWindow } = require("electron");

  return BrowserWindow;
}

function isQuitting() {
  return quitting;
}

function markQuitting() {
  quitting = true;
}

function withTimeout(promise, ms = 1500, fallback = undefined) {
  let timer;

  const timeout = new Promise((resolve) => {
    timer = setTimeout(() => resolve(fallback), ms);

    if (typeof timer.unref === "function") {
      timer.unref();
    }
  });

  const work = Promise.resolve(promise).catch(() => fallback);

  return Promise.race([work, timeout]).finally(() => {
    clearTimeout(timer);
  });
}

function destroyWindow(window, destroyed) {
  if (!window || destroyed.has(window)) {
    return;
  }

  destroyed.add(window);

  try {
    if (typeof window.isDestroyed === "function" && window.isDestroyed()) {
      return;
    }
  } catch (e) {
    // Fall through and try to destroy it anyway.
  }

  try {
    window.destroy();
  } catch (e) {
    // The window may already be gone.
  }
}

function destroyWindows() {
  const destroyed = new Set();
  let windowsMap = {};

  try {
    windowsMap = dependencies.getWindows() || {};
  } catch (e) {
    windowsMap = {};
  }

  for (const name of Object.keys(windowsMap)) {
    destroyWindow(windowsMap[name], destroyed);
  }

  // Output windows are opened with window.open and are not in the windows map.
  try {
    const BrowserWindow = getBrowserWindow();
    const allWindows = BrowserWindow?.getAllWindows?.() || [];

    for (const window of allWindows) {
      destroyWindow(window, destroyed);
    }
  } catch (e) {
    // Electron may already be shutting down.
  }
}

async function doCleanup() {
  markQuitting();

  destroyWindows();

  const tasks = [];

  try {
    const result = dependencies.stopMediaWatcher();

    if (result && typeof result.then === "function") {
      tasks.push(result.catch(() => {}));
    }
  } catch (e) {
    // A broken watcher must not stop the quit.
  }

  try {
    const result = dependencies.stopSystemAudio();

    if (result && typeof result.then === "function") {
      tasks.push(result.catch(() => {}));
    }
  } catch (e) {
    // A broken audio capture must not stop the quit.
  }

  if (tasks.length) {
    await withTimeout(Promise.all(tasks), CLEANUP_TIMEOUT_MS);
  }
}

function runCleanup() {
  markQuitting();

  if (!cleanupPromise) {
    cleanupPromise = doCleanup();
  }

  return cleanupPromise;
}

function clearWatchdog() {
  if (!watchdogTimer) {
    return;
  }

  clearTimeout(watchdogTimer);
  watchdogTimer = null;
}

function startWatchdog(app) {
  if (watchdogTimer) {
    return;
  }

  watchdogTimer = setTimeout(() => {
    try {
      app.exit(0);
    } catch (e) {
      process.exit(0);
    }
  }, WATCHDOG_TIMEOUT_MS);

  if (typeof watchdogTimer.unref === "function") {
    watchdogTimer.unref();
  }
}

function registerShutdownHandlers(nextDependencies = {}) {
  dependencies = { ...dependencies, ...nextDependencies };

  if (handlersRegistered) {
    return;
  }

  handlersRegistered = true;

  const app = getApp();

  if (!app) {
    return;
  }

  app.on("before-quit", () => {
    markQuitting();
    void runCleanup();
    startWatchdog(app);
  });

  app.on("will-quit", () => {
    clearWatchdog();
  });
}

export {
  CLEANUP_TIMEOUT_MS,
  WATCHDOG_TIMEOUT_MS,
  isQuitting,
  markQuitting,
  registerShutdownHandlers,
  runCleanup,
  withTimeout,
};
