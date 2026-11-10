import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, test } from "@playwright/test";
import {
  ipcRendererInvoke,
  parseElectronApp,
} from "electron-playwright-helpers";
import { _electron as electron } from "playwright";
import { resolveLatestBuild } from "../../pageObjectModel/resolveLatestBuild";

const EXIT_TIMEOUT_MS = 15000;

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function expectBoundsClose(actual, expected, tolerance = 16) {
  expect(Math.abs(actual.x - expected.x)).toBeLessThanOrEqual(tolerance);
  expect(Math.abs(actual.y - expected.y)).toBeLessThanOrEqual(tolerance);
  expect(Math.abs(actual.width - expected.width)).toBeLessThanOrEqual(
    tolerance,
  );
  expect(Math.abs(actual.height - expected.height)).toBeLessThanOrEqual(
    tolerance,
  );
}

async function waitUntilModVReady(page, timeout = 60000) {
  const deadline = Date.now() + timeout;

  while (Date.now() < deadline) {
    if (page.isClosed()) {
      break;
    }

    if (await ipcRendererInvoke(page, "is-modv-ready")) {
      return;
    }

    await page.waitForTimeout(250);
  }

  throw new Error("VmodV did not become ready before the window state test");
}

async function launchApp(buildDir, configDir) {
  const appInfo = parseElectronApp(buildDir);
  const launchArgs = [];

  if (process.env.MODV_E2E_NO_SANDBOX === "1") {
    launchArgs.push("--no-sandbox");
  }

  launchArgs.push(appInfo.main);

  return electron.launch({
    args: launchArgs,
    executablePath: appInfo.executable,
    env: {
      ...process.env,
      CI: "e2e",
      ELECTRON_ENABLE_LOGGING: "1",
      XDG_CONFIG_HOME: configDir,
    },
  });
}

async function waitForOutputWindow(electronApp, timeout = 30000) {
  const deadline = Date.now() + timeout;

  while (Date.now() < deadline) {
    const windows = await electronApp.evaluate(({ BrowserWindow }) =>
      BrowserWindow.getAllWindows().map((window) => ({
        url: window.webContents.getURL(),
        bounds: window.getBounds(),
      })),
    );
    const outputWindow = windows.find((window) =>
      window.url.includes("output-window.html"),
    );

    if (outputWindow) {
      return outputWindow;
    }

    await delay(100);
  }

  throw new Error("VmodV did not create the output window");
}

test("remembers window screens and resolutions between restarts", async () => {
  test.setTimeout(180000);

  const latestBuild = resolveLatestBuild(path.resolve("./dist"));
  const configDir = fs.mkdtempSync(path.join(os.tmpdir(), "vmodv-e2e-config-"));
  const statePath = path.join(configDir, "VmodV", "window-state.json");
  let firstApp;
  let secondApp;

  try {
    firstApp = await launchApp(latestBuild.buildDir, configDir);
    const firstPage = await firstApp.firstWindow();

    await waitUntilModVReady(firstPage);
    await waitForOutputWindow(firstApp);

    const placed = await firstApp.evaluate(
      async ({ BrowserWindow, screen }) => {
        const area = screen.getPrimaryDisplay().workArea;
        const displayId = screen.getPrimaryDisplay().id;
        const mainWindow = BrowserWindow.getAllWindows().find((window) =>
          window.webContents.getURL().includes("index.html"),
        );
        const outputWindow = BrowserWindow.getAllWindows().find((window) =>
          window.webContents.getURL().includes("output-window.html"),
        );

        if (!mainWindow || !outputWindow) {
          throw new Error("Could not find the main and output windows");
        }

        const mainBounds = {
          x: area.x + 40,
          y: area.y + 30,
          width: Math.min(1000, area.width - 120),
          height: Math.min(700, area.height - 120),
        };
        const outputBounds = {
          x: area.x + 80,
          y: area.y + 60,
          width: Math.min(640, area.width - 200),
          height: Math.min(480, area.height - 200),
        };

        mainWindow.unmaximize();
        mainWindow.setBounds(mainBounds);
        outputWindow.setBounds(outputBounds);

        // Let the window manager finish unmaximising/repositioning before
        // reading back what was actually applied.
        await new Promise((resolve) => setTimeout(resolve, 400));

        return {
          displayId,
          mainBounds: mainWindow.getBounds(),
          outputBounds: outputWindow.getBounds(),
        };
      },
    );

    // Give the debounced placement save time to observe the move/resize
    // events before quitting.
    await delay(750);

    const exited = new Promise((resolve) => {
      firstApp.process().once("exit", resolve);
    });

    await firstApp
      .evaluate(({ app }) => {
        app.quit();
      })
      .catch(() => {
        // The Playwright connection can drop as the app exits.
      });

    await Promise.race([exited, delay(EXIT_TIMEOUT_MS)]);
    firstApp = null;

    const savedState = JSON.parse(fs.readFileSync(statePath, "utf8"));

    expect(savedState.mainWindow).toMatchObject({
      ...placed.mainBounds,
      displayId: placed.displayId,
      maximized: false,
      fullscreen: false,
    });
    expect(savedState.windows["output-0"]).toMatchObject({
      ...placed.outputBounds,
      displayId: placed.displayId,
      maximized: false,
      fullscreen: false,
    });

    secondApp = await launchApp(latestBuild.buildDir, configDir);
    const secondPage = await secondApp.firstWindow();

    await waitUntilModVReady(secondPage);
    await waitForOutputWindow(secondApp);
    // Window managers apply a show-time placement asynchronously.
    await delay(400);

    const restored = await secondApp.evaluate(({ BrowserWindow }) =>
      BrowserWindow.getAllWindows().map((window) => ({
        url: window.webContents.getURL(),
        bounds: window.getBounds(),
        maximized: window.isMaximized(),
      })),
    );
    const restoredMain = restored.find((window) =>
      window.url.includes("index.html"),
    );
    const restoredOutput = restored.find((window) =>
      window.url.includes("output-window.html"),
    );

    expect(restoredMain.maximized).toBe(false);
    expectBoundsClose(restoredMain.bounds, placed.mainBounds);
    expectBoundsClose(restoredOutput.bounds, placed.outputBounds);
  } finally {
    for (const app of [firstApp, secondApp]) {
      try {
        app?.process()?.kill("SIGKILL");
      } catch (e) {
        // The process may already be gone.
      }
    }

    for (const directory of [latestBuild.cleanupDir, configDir]) {
      if (directory) {
        fs.rmSync(directory, { recursive: true, force: true });
      }
    }
  }
});
