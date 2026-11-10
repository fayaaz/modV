import fs from "node:fs";
import path from "node:path";

// Window placement is stored in the userData directory so packaged builds keep
// it between restarts. Bounds are in device-independent pixels, the same unit
// Electron's BrowserWindow and screen APIs use, so the saved position stays
// correct when the desktop scale factor changes.

export const WINDOW_STATE_FILENAME = "window-state.json";
export const WINDOW_STATE_VERSION = 1;

const MODE_EVENTS = [
  "resize",
  "move",
  "maximize",
  "unmaximize",
  "enter-full-screen",
  "leave-full-screen",
];

function isFiniteNumber(value) {
  return typeof value === "number" && Number.isFinite(value);
}

function isUsableRect(rect) {
  return (
    rect &&
    isFiniteNumber(rect.x) &&
    isFiniteNumber(rect.y) &&
    isFiniteNumber(rect.width) &&
    isFiniteNumber(rect.height) &&
    rect.width > 0 &&
    rect.height > 0
  );
}

function sanitiseBounds(bounds) {
  if (!isUsableRect(bounds)) {
    return null;
  }

  return {
    x: Math.round(bounds.x),
    y: Math.round(bounds.y),
    width: Math.round(bounds.width),
    height: Math.round(bounds.height),
  };
}

function fitsWithin(bounds, area, tolerance = 1) {
  return (
    bounds.x >= area.x - tolerance &&
    bounds.y >= area.y - tolerance &&
    bounds.x + bounds.width <= area.x + area.width + tolerance &&
    bounds.y + bounds.height <= area.y + area.height + tolerance
  );
}

function intersectionArea(a, b) {
  const width = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
  const height = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);

  if (width <= 0 || height <= 0) {
    return 0;
  }

  return width * height;
}

function clampBoundsToArea(bounds, area) {
  const width = Math.min(bounds.width, area.width);
  const height = Math.min(bounds.height, area.height);

  return {
    x: Math.min(Math.max(bounds.x, area.x), area.x + area.width - width),
    y: Math.min(Math.max(bounds.y, area.y), area.y + area.height - height),
    width,
    height,
  };
}

function normaliseSavedWindow(value) {
  if (!value || typeof value !== "object") {
    return null;
  }

  const bounds = sanitiseBounds(value);

  if (!bounds) {
    return null;
  }

  return {
    ...bounds,
    displayId: isFiniteNumber(value.displayId) ? value.displayId : null,
    maximized: value.maximized === true,
    fullscreen: value.fullscreen === true,
  };
}

export function createEmptyWindowState() {
  return {
    version: WINDOW_STATE_VERSION,
    mainWindow: null,
    windows: {},
  };
}

export function normalizeWindowState(value) {
  const state = createEmptyWindowState();

  if (
    !value ||
    typeof value !== "object" ||
    value.version !== WINDOW_STATE_VERSION
  ) {
    return state;
  }

  state.mainWindow = normaliseSavedWindow(value.mainWindow);

  if (value.windows && typeof value.windows === "object") {
    for (const [key, saved] of Object.entries(value.windows)) {
      const normalised = normaliseSavedWindow(saved);

      if (normalised) {
        state.windows[key] = normalised;
      }
    }
  }

  return state;
}

export function readWindowState(filePath) {
  try {
    return normalizeWindowState(JSON.parse(fs.readFileSync(filePath, "utf8")));
  } catch (e) {
    return createEmptyWindowState();
  }
}

export function writeWindowState(filePath, state) {
  const temporaryPath = `${filePath}.tmp`;

  try {
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(temporaryPath, JSON.stringify(state, null, 2));
    fs.renameSync(temporaryPath, filePath);

    return true;
  } catch (e) {
    try {
      fs.rmSync(temporaryPath, { force: true });
    } catch (cleanupError) {
      // The temporary file may never have been written.
    }

    return false;
  }
}

export function snapshotWindowState({
  bounds,
  displayId = null,
  maximized = false,
  fullscreen = false,
}) {
  const sanitised = sanitiseBounds(bounds);

  if (!sanitised) {
    return null;
  }

  return {
    ...sanitised,
    displayId: isFiniteNumber(displayId) ? displayId : null,
    maximized: maximized === true,
    fullscreen: fullscreen === true,
  };
}

/**
 * Work out where a window should open.
 *
 * A window that still fits entirely on one of the connected displays is
 * restored exactly. Otherwise its remembered size is clamped into the display
 * it was on (resolution shrunk) or the nearest/primary display (monitor
 * removed), so a window can never open entirely off-screen.
 */
export function resolveWindowBounds(saved, displays = [], fallbackBounds) {
  const usableDisplays = (displays || []).filter(
    (display) => display && isUsableRect(display.workArea),
  );
  const savedBounds = saved ? sanitiseBounds(saved) : null;
  const primary =
    usableDisplays.find((display) => display.isPrimary) || usableDisplays[0];
  const fallback =
    sanitiseBounds(fallbackBounds) ||
    (primary ? sanitiseBounds(primary.workArea) : null);

  if (!savedBounds) {
    return {
      bounds: fallback || { x: 0, y: 0, width: 800, height: 600 },
      restored: false,
    };
  }

  // No connected display information (e.g. during shutdown): trust the saved
  // bounds rather than inventing a position.
  if (!usableDisplays.length) {
    return { bounds: savedBounds, restored: true };
  }

  const displayItFitsOn = usableDisplays.find((display) =>
    fitsWithin(savedBounds, display.workArea),
  );

  if (displayItFitsOn) {
    return { bounds: savedBounds, restored: true };
  }

  const target =
    usableDisplays.find((display) => display.id === saved.displayId) ||
    usableDisplays.reduce(
      (best, display) =>
        intersectionArea(savedBounds, display.workArea) >
        intersectionArea(savedBounds, best.workArea)
          ? display
          : best,
      primary,
    ) ||
    primary;

  return {
    bounds: clampBoundsToArea(savedBounds, target.workArea),
    restored: true,
  };
}

export function nextOutputWindowKey(activeKeys = []) {
  const active = new Set(activeKeys);
  let index = 0;

  while (active.has(`output-${index}`)) {
    index += 1;
  }

  return `output-${index}`;
}

// ---------------------------------------------------------------------------
// Electron-facing helpers. Electron is required lazily so the placement
// helpers above can be unit-tested in plain Node.

function getElectron() {
  return require("electron");
}

function getScreen() {
  return getElectron().screen;
}

export function getWindowStatePath() {
  const { app } = getElectron();

  return path.join(app.getPath("userData"), WINDOW_STATE_FILENAME);
}

let cachedState = null;
let mainWindowPlacement = null;
let flushTimer = null;
let warnedAboutSaveFailure = false;

export function loadWindowState() {
  if (!cachedState) {
    cachedState = readWindowState(getWindowStatePath());
  }

  return cachedState;
}

export function scheduleWindowStateFlush(delay = 400) {
  if (!cachedState) {
    return;
  }

  if (flushTimer) {
    clearTimeout(flushTimer);
  }

  flushTimer = setTimeout(() => {
    flushTimer = null;
    flushWindowState();
  }, delay);

  // Never keep the app alive just to write window placement.
  if (typeof flushTimer.unref === "function") {
    flushTimer.unref();
  }
}

export function flushWindowState() {
  if (flushTimer) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }

  if (!cachedState) {
    return false;
  }

  const saved = writeWindowState(getWindowStatePath(), cachedState);

  if (!saved && !warnedAboutSaveFailure) {
    warnedAboutSaveFailure = true;

    try {
      const { createLogger } = require("./logger");
      createLogger("main:window-state").warn(
        "could not save the window placement",
        { path: getWindowStatePath() },
      );
    } catch (e) {
      // Logging must never stop the save path from finishing.
    }
  }

  return saved;
}

function snapshotBrowserWindow(window) {
  if (!window || (window.isDestroyed && window.isDestroyed())) {
    return null;
  }

  const bounds =
    typeof window.getNormalBounds === "function"
      ? window.getNormalBounds()
      : window.getBounds();
  const display = getScreen().getDisplayMatching(bounds);

  return snapshotWindowState({
    bounds,
    displayId: display?.id,
    maximized: window.isMaximized?.() === true,
    fullscreen: window.isFullScreen?.() === true,
  });
}

export function saveWindowSnapshot(state, key, snapshot) {
  if (!snapshot) {
    return state;
  }

  // The main window is a single, well-known window; output windows live in
  // their own slots so a new output window can take the next free one.
  if (key === "mainWindow") {
    state.mainWindow = snapshot;
  } else {
    state.windows[key] = snapshot;
  }

  return state;
}

export function rememberWindow(window, key) {
  const snapshot = snapshotBrowserWindow(window);

  if (!snapshot) {
    return null;
  }

  const state = loadWindowState();
  saveWindowSnapshot(state, key, snapshot);
  scheduleWindowStateFlush();

  return snapshot;
}

export function prepareMainWindowPlacement() {
  const state = loadWindowState();
  const displays = getScreen().getAllDisplays();
  const primary = getScreen().getPrimaryDisplay();
  const fallback = primary?.workArea || {
    x: 0,
    y: 0,
    width: 1280,
    height: 720,
  };

  mainWindowPlacement = {
    ...resolveWindowBounds(state.mainWindow, displays, fallback),
    maximized: state.mainWindow ? state.mainWindow.maximized : true,
    fullscreen: state.mainWindow ? state.mainWindow.fullscreen : false,
  };

  return mainWindowPlacement;
}

/**
 * The placement decided when the window was created. Re-reading the state at
 * show time would pick up the constructor's provisional position (the window
 * manager applies its title bar offset after creation), so the snapshot is
 * kept for the show-time restore.
 */
export function getMainWindowBounds() {
  return mainWindowPlacement || prepareMainWindowPlacement();
}

export function placeMainWindow(window) {
  if (!window || window.isDestroyed()) {
    return;
  }

  const placement = getMainWindowBounds();
  const saved = placement.restored ? placement : null;

  // Preserve the previous first-run behaviour: a maximised main window.
  if (!saved) {
    window.maximize();
    window.show();
    window.focus();
    return;
  }

  const applySavedPlacement = () => {
    if (window.isDestroyed()) {
      return;
    }

    // Apply the exact bounds after the window is mapped. Some window managers
    // ignore or later override placement requested while the window is still
    // hidden, which shifted restored windows by their title bar height.
    window.setBounds(placement.bounds);

    if (placement.fullscreen) {
      window.setFullScreen(true);
    } else if (placement.maximized) {
      window.maximize();
    }
  };

  if (window.isVisible()) {
    applySavedPlacement();
  } else {
    window.once("show", () => {
      // The show event fires before the window manager has finished mapping
      // the window; a short delay lets the exact bounds stick.
      setTimeout(applySavedPlacement, 100);
    });
    window.show();
  }

  window.focus();
}

export function trackMainWindow(window) {
  const remember = () => rememberWindow(window, "mainWindow");

  for (const event of MODE_EVENTS) {
    window.on(event, remember);
  }

  window.on("close", () => {
    remember();
    flushWindowState();
  });
}

const activeOutputWindows = new Map();

export function trackOutputWindow(window) {
  const { key, saved, savedBounds } = getNextOutputWindowPlacement();
  activeOutputWindows.set(window, key);

  if (saved) {
    // setBounds in did-create-window runs before the window manager has
    // decorated/mapped the child, which shifts it by the title bar height.
    // Re-apply once the renderer has painted, immediately before it is shown.
    const applySavedBounds = () => {
      if (window.isDestroyed()) {
        return;
      }

      window.setBounds(savedBounds);

      if (saved.fullscreen) {
        window.setFullScreen(true);
      } else if (saved.maximized) {
        window.maximize();
      }
    };

    window.once("ready-to-show", applySavedBounds);
  }

  const remember = () => rememberWindow(window, key);

  for (const event of MODE_EVENTS) {
    window.on(event, remember);
  }

  window.on("close", () => {
    remember();
    flushWindowState();
  });

  window.on("closed", () => {
    activeOutputWindows.delete(window);
  });

  return key;
}

export function getNextOutputWindowPlacement() {
  const key = nextOutputWindowKey([...activeOutputWindows.values()]);
  const saved = loadWindowState().windows[key];

  if (!saved) {
    return { key, saved: null, savedBounds: null };
  }

  const displays = getScreen().getAllDisplays();
  const primary = getScreen().getPrimaryDisplay()?.workArea || {
    x: 0,
    y: 0,
    width: 1280,
    height: 720,
  };

  return {
    key,
    saved,
    savedBounds: resolveWindowBounds(saved, displays, primary).bounds,
  };
}
