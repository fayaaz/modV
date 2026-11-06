import { execFileSync } from "child_process";
import fs from "fs";
import os from "os";
import path from "path";
import { expect, test } from "@playwright/test";
import {
  ipcRendererInvoke,
  parseElectronApp,
} from "electron-playwright-helpers";
import { _electron as electron } from "playwright";
import { resolveLatestBuild } from "../../pageObjectModel/resolveLatestBuild";

const EXIT_TIMEOUT_MS = 5000;
const CHILD_EXIT_TIMEOUT_MS = 3000;
const POLL_INTERVAL_MS = 100;

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function readProcessChildren() {
  const output = execFileSync("ps", ["-e", "-o", "pid=,ppid="], {
    encoding: "utf8",
  });
  const childrenByParent = new Map();

  for (const line of output.trim().split("\n")) {
    const [pid, ppid] = line.trim().split(/\s+/).map(Number);

    if (!pid || !ppid) {
      continue;
    }

    const children = childrenByParent.get(ppid) || [];

    children.push(pid);
    childrenByParent.set(ppid, children);
  }

  return childrenByParent;
}

function collectDescendants(rootPid) {
  const childrenByParent = readProcessChildren();
  const descendants = [];
  const stack = [rootPid];

  while (stack.length) {
    const parent = stack.pop();

    for (const child of childrenByParent.get(parent) || []) {
      descendants.push(child);
      stack.push(child);
    }
  }

  return descendants;
}

function isProcessAlive(pid) {
  try {
    const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
    // The process name is wrapped in parentheses and may contain spaces, so
    // read the state after the final ")".
    const endOfName = stat.lastIndexOf(")");
    const state = stat.slice(endOfName + 2).split(" ")[0];

    // Zombies are already dead; the parent just has not reaped them yet.
    return state !== "Z";
  } catch (e) {
    return false;
  }
}

async function waitForGone(pids, timeout) {
  const deadline = Date.now() + timeout;

  while (Date.now() < deadline) {
    if (!pids.some(isProcessAlive)) {
      return;
    }

    await delay(POLL_INTERVAL_MS);
  }
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

  throw new Error("modV did not become ready before the quit test");
}

test("quitting the app leaves no orphaned process tree", async () => {
  test.setTimeout(90000);

  const latestBuild = resolveLatestBuild(path.resolve("./dist"));
  const configDir = fs.mkdtempSync(path.join(os.tmpdir(), "modv-e2e-config-"));
  const appInfo = parseElectronApp(latestBuild.buildDir);
  const launchArgs = [];

  if (process.env.MODV_E2E_NO_SANDBOX === "1") {
    launchArgs.push("--no-sandbox");
  }

  launchArgs.push(appInfo.main);

  const electronApp = await electron.launch({
    args: launchArgs,
    executablePath: appInfo.executable,
    env: {
      ...process.env,
      CI: "e2e",
      ELECTRON_ENABLE_LOGGING: "1",
      XDG_CONFIG_HOME: configDir,
    },
  });

  const appProcess = electronApp.process();
  const mainPid = appProcess.pid;
  let descendants = [];

  try {
    const page = await electronApp.firstWindow();

    await waitUntilModVReady(page);

    descendants = collectDescendants(mainPid);

    const exited = new Promise((resolve) => {
      appProcess.once("exit", resolve);
    });

    await electronApp
      .evaluate(({ app }) => {
        app.quit();
      })
      .catch(() => {
        // The Playwright connection can drop as the app exits.
      });

    await Promise.race([exited, delay(EXIT_TIMEOUT_MS)]);

    expect(isProcessAlive(mainPid)).toBe(false);

    await waitForGone(descendants, CHILD_EXIT_TIMEOUT_MS);

    const surviving = descendants.filter(isProcessAlive);

    expect(surviving).toEqual([]);
  } finally {
    if (isProcessAlive(mainPid)) {
      try {
        appProcess.kill("SIGKILL");
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
