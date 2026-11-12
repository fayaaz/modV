// R-23 — the Rust engine accepts unknown commands with a catch-all, so
// Output Windows, MIDI, OSC, plugins and local fonts are silently dropped in
// engine mode. The UI uses these strings to say so instead of pretending the
// feature works. Legacy (worker) mode never reads them.

/** Short label/tooltip for a control that the engine cannot drive yet. */
export const ENGINE_UNSUPPORTED_TITLE =
  "Not available with the Rust engine yet";

/** One-line boot warning, also useful when a shortcut is attempted. */
export const ENGINE_UNSUPPORTED_BOOT_MESSAGE =
  "Rust engine mode: Output Windows, MIDI input, OSC, plugins and local fonts are not supported yet; unavailable controls are disabled";

/** Console/renderer warning for a feature the user tried to use anyway. */
export function engineUnsupportedMessage(feature) {
  return `${feature} is not available with the Rust engine yet`;
}

/**
 * WS-10 screen capture in engine mode.
 *
 * The portal/consent UI stays in Electron, but the engine captures through a
 * PipeWire node id, and Electron 45 never exposes the xdg-desktop-portal
 * `ScreenCast` node id to the app: `desktopCapturer.getSources()` yields only
 * `screen:<display>:<n>` / `window:<id>:<n>`, and
 * `setDisplayMediaRequestHandler(..., { useSystemPicker: true })` hands the
 * picker to the portal entirely. There is therefore no node id to forward to
 * `media/start-screen`, so engine mode says so instead of silently capturing
 * in the renderer (where nothing consumes the frames).
 */
export const ENGINE_SCREEN_CAPTURE_UNAVAILABLE =
  "Screen capture is not available with the Rust engine yet: the PipeWire node id from the screen picker is not exposed to the app, so the engine has nothing to capture. Use the default (non-engine) renderer for screen capture.";
