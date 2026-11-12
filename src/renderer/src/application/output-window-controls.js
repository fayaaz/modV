// Fullscreen controls for the legacy Output Window popup.
//
// The engine has its own native window (winit) with the same F / double-click
// shortcuts; this module keeps the Electron popup path testable without a DOM.
//
// The canvas in an Output Window has been transferred to the worker
// (`transferControlToOffscreen`), so it is a placeholder rather than a normal
// canvas. Requesting fullscreen on the placeholder is unreliable, which is why
// the popup fullscreens its `documentElement` instead.

const FULLSCREEN_KEY = "f";

/**
 * Whether a keydown event should toggle the Output Window's fullscreen state.
 *
 * `F`/`f` without Ctrl/Meta/Alt; Shift is allowed (it is how `F` is typed).
 * Ctrl is rejected on purpose: AltGr reports as Ctrl+Alt on some layouts.
 *
 * @param {KeyboardEvent | { key?: string, ctrlKey?: boolean, metaKey?: boolean, altKey?: boolean }} event
 * @returns {boolean}
 */
export function isFullscreenToggleEvent(event) {
  if (!event || typeof event.key !== "string") {
    return false;
  }

  if (event.key.toLowerCase() !== FULLSCREEN_KEY) {
    return false;
  }

  return !event.ctrlKey && !event.metaKey && !event.altKey;
}

/**
 * Whether a keydown event should leave fullscreen.
 *
 * @param {{ key?: string } | null | undefined} event
 * @returns {boolean}
 */
export function isFullscreenExitEvent(event) {
  return Boolean(event) && event.key === "Escape";
}

/**
 * Whether the document is currently in HTML fullscreen.
 *
 * @param {Document | null | undefined} doc
 * @returns {boolean}
 */
export function isFullscreen(doc) {
  return Boolean(doc && (doc.fullscreenElement || doc.webkitFullscreenElement));
}

/**
 * Enter fullscreen on the document root.
 *
 * Uses the standard API first and the WebKit-prefixed one as a fallback (the
 * Electron/Chromium version here supports both). Returns `false` when neither
 * is available or the request is rejected (browsers reject without a user
 * gesture, which callers always have).
 *
 * @param {Document | null | undefined} doc
 * @returns {Promise<boolean>}
 */
export async function requestFullscreen(doc) {
  const target = doc?.documentElement;

  if (!target) {
    return false;
  }

  try {
    if (typeof target.requestFullscreen === "function") {
      await target.requestFullscreen();
      return true;
    }

    if (typeof target.webkitRequestFullscreen === "function") {
      target.webkitRequestFullscreen();
      return true;
    }
  } catch (error) {
    return false;
  }

  return false;
}

/**
 * Leave fullscreen.
 *
 * @param {Document | null | undefined} doc
 * @returns {Promise<boolean>}
 */
export async function exitFullscreen(doc) {
  if (!doc) {
    return false;
  }

  try {
    if (typeof doc.exitFullscreen === "function") {
      await doc.exitFullscreen();
      return true;
    }

    if (typeof doc.webkitExitFullscreen === "function") {
      doc.webkitExitFullscreen();
      return true;
    }
  } catch (error) {
    return false;
  }

  return false;
}

/**
 * Toggle the document between fullscreen and windowed.
 *
 * @param {Document | null | undefined} doc
 * @returns {Promise<boolean>}
 */
export function toggleFullscreen(doc) {
  return isFullscreen(doc) ? exitFullscreen(doc) : requestFullscreen(doc);
}
