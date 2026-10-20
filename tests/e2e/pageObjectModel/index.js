import { execFileSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";
import { expect, test } from "@playwright/test";
import {
  findLatestBuild,
  ipcRendererInvoke,
  parseElectronApp,
} from "electron-playwright-helpers";
import { _electron as electron } from "playwright";
import { groups } from "./groups";
import { gallery } from "./gallery";
import { modules } from "./modules";
import { tabs } from "./tabs";

/**
 * The canonical Docker build emits installers only, while `findLatestBuild`
 * expects an unpacked app directory. Fall back to extracting the newest
 * AppImage so the e2e suite exercises the artifact users actually run.
 */
function resolveLatestBuild(distDir) {
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

  const extractDir = fs.mkdtempSync(path.join(os.tmpdir(), "modv-e2e-"));
  execFileSync(path.join(distDir, appImages[0].name), ["--appimage-extract"], {
    cwd: extractDir,
    stdio: "ignore",
  });

  // parseElectronApp infers the platform from the directory name.
  const buildDir = path.join(extractDir, "modv-linux-unpacked");
  fs.renameSync(path.join(extractDir, "squashfs-root"), buildDir);
  // The AppImage's AppRun launcher is also executable and would otherwise be
  // picked as the Electron executable.
  fs.rmSync(path.join(buildDir, "AppRun"), { force: true });

  return { buildDir, cleanupDir: extractDir };
}

class ModVApp {
  groups = groups;
  gallery = gallery;
  modules = modules;
  tabs = tabs;

  /**
   * @param {import('@playwright/test').Page} page
   */
  constructor() {
    this.electronApp = null;
    this.page = null;

    test.beforeAll(async () => {
      // find the latest build in the out directory
      const latestBuild = resolveLatestBuild(path.resolve("./dist"));
      this.buildDir = latestBuild.buildDir;
      // Keep tests from reading or writing the user's real modV config.
      const configDir = fs.mkdtempSync(
        path.join(os.tmpdir(), "modv-e2e-config-"),
      );
      this.cleanupDirs = [latestBuild.cleanupDir, configDir].filter(Boolean);

      // parse the directory and find paths and other info
      const appInfo = parseElectronApp(latestBuild.buildDir);
      // set the CI environment variable to true
      process.env.CI = "e2e";

      this.electronApp = await electron.launch({
        args: [
          appInfo.main,
          // "--use-fake-device-for-media-stream",
          // "--use-fake-ui-for-media-stream",
        ],
        executablePath: appInfo.executable,
        env: {
          ...process.env,
          XDG_CONFIG_HOME: configDir,
        },
      });

      this.electronApp.on("window", async (page) => {
        // capture errors
        page.on("pageerror", (error) => {
          console.error(error);
        });

        // capture console messages
        page.on("console", (msg) => {
          console.log(msg.text());
        });
      });

      // console.info("  ℹ    Waiting for modV to become ready…");
      this.page = await modVApp.electronApp.firstWindow();
      await this.waitUntilModVReady();
    });

    // The app is launched once per worker and shared by every spec file in it,
    // so it must stay alive between files. Clean up when the worker exits.
    process.once("exit", () => {
      try {
        this.electronApp?.process()?.kill();
      } catch (e) {
        // The process may already be gone.
      }

      for (let i = 0; i < (this.cleanupDirs || []).length; i++) {
        fs.rmSync(this.cleanupDirs[i], { recursive: true, force: true });
      }
    });
  }

  async waitUntilModVReady(resolver) {
    if (!resolver) {
      let resolveReady;

      const promise = new Promise((resolve) => {
        resolveReady = resolve;
      });

      this.waitUntilModVReady(resolveReady);

      return promise;
    }

    const isReady = await ipcRendererInvoke(this.page, "is-modv-ready");

    if (!isReady) {
      setTimeout(() => {
        this.waitUntilModVReady(resolver);
      }, 500);

      return;
    }

    resolver();
  }

  async evaluateMainState(propertyPath) {
    const { page } = this;

    if (propertyPath) {
      return page.evaluate(
        (propertyPath) => window._get(window.modV.store.state, propertyPath),
        propertyPath,
      );
    }

    return page.evaluate(() => window.modV.store.state);
  }

  async evaluateUIState() {
    const { page } = this;

    return page.evaluate(() => window.Vue.config.globalProperties.$store.state);
  }

  async evaluateWorkerState(propertyPath) {
    const { page } = this;
    const worker = page.workers()[0];

    if (propertyPath) {
      return worker.evaluate(
        (propertyPath) => self._get(self.store.state, propertyPath),
        propertyPath,
      );
    }

    return worker.evaluate(() => self.store.state);
  }

  async evaluateWorker(fn, arg) {
    const { page } = this;
    return page.workers()[0].evaluate(fn, arg);
  }

  async checkWorkerAndMainState(checks = [], propertyPath) {
    for (let j = 0; j < 2; j += 1) {
      for (let i = 0; i < checks.length; i += 1) {
        const check = checks[i];

        await check[1](
          expect.poll(async () => {
            if (j === 0) {
              return check[0](await this.evaluateWorkerState(propertyPath));
            } else {
              return check[0](await this.evaluateMainState(propertyPath));
            }
          }),
        );
      }
    }
  }

  async generatePreset() {
    const { page } = this;

    return page.evaluate(async () => await window.modV.generatePreset());
  }
}

const modVApp = new ModVApp();
export { modVApp };
