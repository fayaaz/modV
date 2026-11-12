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
