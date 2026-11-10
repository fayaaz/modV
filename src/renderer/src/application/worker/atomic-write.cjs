// CommonJS on purpose: the render worker externalizes Node built-ins through
// the `__require` shim, and an ESM `import "fs"` survives the bundle as a bare
// specifier Chromium cannot resolve at runtime. Unit tests import the default
// export through the standard CJS->ESM interop.
const fs = require("fs");
const path = require("path");

/**
 * Writes a file atomically: the contents go to a temporary file in the same
 * directory first and are then renamed over the target, so a concurrent
 * reader never observes an empty or partially written file. Used for the
 * persisted Scene list, which the worker saves in the background while the
 * user (or a test) may be reading it.
 */
async function writeFileAtomic(filePath, contents) {
  const directory = path.dirname(filePath);
  const temporary = path.join(
    directory,
    `.${path.basename(filePath)}.${process.pid}.${Date.now()}.${Math.random()
      .toString(36)
      .slice(2)}.tmp`,
  );

  await fs.promises.mkdir(directory, { recursive: true });

  try {
    await fs.promises.writeFile(temporary, contents);
    await fs.promises.rename(temporary, filePath);
  } catch (e) {
    try {
      await fs.promises.unlink(temporary);
    } catch {
      // The temporary file was never created or the rename already moved it;
      // nothing to clean up.
    }

    throw e;
  }
}

module.exports = { writeFileAtomic };
