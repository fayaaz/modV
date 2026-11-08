import { execFileSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";
import { findLatestBuild } from "electron-playwright-helpers";

/**
 * The canonical Docker build emits installers only, while `findLatestBuild`
 * expects an unpacked app directory. Fall back to extracting the newest
 * AppImage so the e2e suite exercises the artifact users actually run.
 */
export function resolveLatestBuild(distDir) {
  try {
    return { buildDir: findLatestBuild(distDir) };
  } catch (e) {
    // No unpacked build in dist; try an AppImage below.
  }

  const appImages = fs
    .readdirSync(distDir)
    .filter((name) => name.endsWith(".AppImage"))
    .map((name) => ({
      name,
      time: fs.statSync(path.join(distDir, name)).mtimeMs,
    }))
    .sort((a, b) => b.time - a.time);

  if (!appImages.length) {
    throw new Error(`No unpacked build or AppImage found in ${distDir}`);
  }

  const extractDir = fs.mkdtempSync(path.join(os.tmpdir(), "vmodv-e2e-"));
  execFileSync(path.join(distDir, appImages[0].name), ["--appimage-extract"], {
    cwd: extractDir,
    stdio: "ignore",
  });

  // parseElectronApp infers the platform from the directory name.
  const buildDir = path.join(extractDir, "vmodv-linux-unpacked");
  fs.renameSync(path.join(extractDir, "squashfs-root"), buildDir);
  // The AppImage's AppRun launcher is also executable and would otherwise be
  // picked as the Electron executable.
  fs.rmSync(path.join(buildDir, "AppRun"), { force: true });

  return { buildDir, cleanupDir: extractDir };
}
