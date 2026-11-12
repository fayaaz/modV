// WS-14 — which window gets the mutating `window.engine` surface (review
// R-25).
//
// `src/preload/index.js` is shared by the main window and the output windows
// (they inherit the opener's webPreferences). Only the main window drives the
// engine: dispatch/commit mutate authoritative state and the preview/snapshot
// requests read its rings. Output windows have no script of their own, the
// splash window has no preload and the color picker uses `colorPicker.js`, so
// everything except the main entry gets no engine API at all.
//
// Kept dependency-free so it can be unit tested without Electron.

/** True for the main renderer entry (`/index.html`, dev `/`, packaged file URL). */
function isMainWindowPath(pathname) {
  if (typeof pathname !== "string" || pathname.length === 0) return false;

  const path = pathname.replace(/\\/g, "/").split("?")[0].split("#")[0];

  if (path === "/") return true;

  return /(^|\/)index\.html$/i.test(path);
}

/**
 * "main" → expose the full engine API; "none" → expose `window.engine` as
 * undefined. A string (rather than a boolean) leaves room for a read-only
 * subset if another window ever needs one.
 *
 * @param {string | undefined | null} pathname `window.location.pathname`
 * @returns {"main" | "none"}
 */
function engineApiScope(pathname) {
  return isMainWindowPath(pathname) ? "main" : "none";
}

export { engineApiScope, isMainWindowPath };
