// WS-14 — forwards the engine's `tracing` output (stderr) into the shared
// daily log, mirroring how the renderer forwards its lines over the "log"
// IPC channel (src/renderer/src/application/logger.js → src/main/window-prefs.js).
//
// The engine writes tracing's default format: `<timestamp> LEVEL target: message`.
// We map ERROR/WARN to those levels and everything else to info, preserving
// the raw line so no information is lost.

const LEVEL_RE = /\b(TRACE|DEBUG|INFO|WARN|ERROR)\b/;

/**
 * Attach a line-buffered stderr → logger pipe to a spawned engine child.
 * @param {import("child_process").ChildProcess} child
 * @param {{ info: Function, warn: Function, error: Function }} logger
 */
function attachEngineLogForwarder(child, logger) {
  let remainder = "";

  const handleChunk = (chunk) => {
    remainder += chunk.toString("utf8");
    const lines = remainder.split("\n");
    remainder = lines.pop() || "";
    for (const raw of lines) {
      const line = raw.trimEnd();
      if (!line) continue;
      const match = LEVEL_RE.exec(line);
      const level = match ? match[1] : "INFO";
      if (level === "ERROR") logger.error(line);
      else if (level === "WARN") logger.warn(line);
      else logger.info(line);
    }
  };

  if (child.stderr) child.stderr.on("data", handleChunk);
  if (child.stdout) child.stdout.on("data", handleChunk); // tracing may go to stdout with some fmt configs
  child.on("exit", () => {
    if (remainder.trim()) {
      logger.info(`engine exited with unterminated line: ${remainder.trim()}`);
    }
    remainder = "";
  });
}

export { attachEngineLogForwarder, LEVEL_RE };
