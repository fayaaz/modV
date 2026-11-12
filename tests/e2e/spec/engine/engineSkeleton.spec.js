// WS-14 / M1 exit criteria, against the packaged app with MODV_ENGINE=1:
//  - the engine presents frames (preview ring publishes, counter grows);
//  - killing the engine leaves the UI alive and is-modv-ready still true;
//  - the supervisor restarts the engine (new pid) within a few seconds.
//
// This spec launches its own Electron instance (not the shared modVApp one)
// so it can run with MODV_ENGINE=1 without affecting the legacy suite.

import { test, expect } from "@playwright/test";
import { _electron as electron } from "playwright";
import {
  ipcRendererInvoke,
  parseElectronApp,
} from "electron-playwright-helpers";
import { execSync } from "node:child_process";
import fs from "fs";
import os from "os";
import path from "path";
import { resolveLatestBuild } from "../../pageObjectModel/resolveLatestBuild";

function enginePids() {
  try {
    const out = execSync(
      `pgrep -u ${process.getuid()} -f "modv-engine.*--socket"`,
      { encoding: "utf8" },
    ).trim();
    return out.split("\n").filter(Boolean).map(Number);
  } catch {
    return [];
  }
}

async function waitForReady(page, timeout = 60_000) {
  const deadline = Date.now() + timeout;

  while (Date.now() < deadline) {
    if (page.isClosed()) return false;
    if (await ipcRendererInvoke(page, "is-modv-ready")) return true;
    await page.waitForTimeout(250);
  }

  return false;
}

test.describe("engine skeleton (MODV_ENGINE=1)", () => {
  test("engine presents frames, survives a kill and restarts", async () => {
    const latestBuild = resolveLatestBuild(path.resolve("./dist"));
    const appInfo = parseElectronApp(latestBuild.buildDir);
    const configDir = fs.mkdtempSync(
      path.join(os.tmpdir(), "vmodv-e2e-engine-"),
    );
    const cleanupDirs = [latestBuild.cleanupDir, configDir].filter(Boolean);

    const app = await electron.launch({
      args: ["--no-sandbox", appInfo.main],
      executablePath: appInfo.executable,
      env: {
        ...process.env,
        MODV_ENGINE: "1",
        ELECTRON_ENABLE_LOGGING: "1",
        XDG_CONFIG_HOME: configDir,
      },
    });

    try {
      const page = await app.firstWindow();

      // M1 exit: the engine-mode renderer must not create GPU contexts.
      await page.addInitScript(() => {
        window.__gpuContexts = [];
        const originalGetContext = HTMLCanvasElement.prototype.getContext;
        HTMLCanvasElement.prototype.getContext = function (type, ...rest) {
          if (/webgl|webgpu/i.test(String(type))) {
            window.__gpuContexts.push(String(type));
          }
          return originalGetContext.call(this, type, ...rest);
        };
        if (navigator.gpu?.requestAdapter) {
          const originalRequestAdapter = navigator.gpu.requestAdapter.bind(
            navigator.gpu,
          );
          navigator.gpu.requestAdapter = (...args) => {
            window.__gpuContexts.push("webgpu");
            return originalRequestAdapter(...args);
          };
        }
      });
      await page.reload();

      expect(await waitForReady(page)).toBe(true);
      expect(await page.evaluate(() => window.__gpuContexts || [])).toEqual([]);

      // The engine publishes preview frames on its own cadence; the renderer
      // counts them into window.__enginePreviewCount (engine-client.js).
      const before = await page.evaluate(
        () => window.__enginePreviewCount || 0,
      );
      await page.waitForFunction(
        (count) => (window.__enginePreviewCount || 0) > count,
        before,
        { timeout: 15_000 },
      );

      const pids = enginePids();
      expect(pids.length).toBeGreaterThan(0);

      // Hard kill the engine; the UI must stay alive and the supervisor
      // must restart it.
      for (const pid of pids) {
        try {
          process.kill(pid, "SIGKILL");
        } catch {
          // already gone
        }
      }

      await page.waitForTimeout(1_500);
      expect(await ipcRendererInvoke(page, "is-modv-ready")).toBe(true);

      let respawned = false;
      const deadline = Date.now() + 10_000;

      while (Date.now() < deadline) {
        if (enginePids().some((pid) => !pids.includes(pid))) {
          respawned = true;
          break;
        }
        await page.waitForTimeout(100);
      }

      expect(respawned).toBe(true);

      // The engine must keep publishing after the restart.
      const afterKill = await page.evaluate(
        () => window.__enginePreviewCount || 0,
      );
      await page.waitForFunction(
        (count) => (window.__enginePreviewCount || 0) > count,
        afterKill,
        { timeout: 15_000 },
      );
    } finally {
      await app.close().catch(() => {});
      for (const dir of cleanupDirs) {
        fs.rmSync(dir, { recursive: true, force: true });
      }
    }
  });
});
