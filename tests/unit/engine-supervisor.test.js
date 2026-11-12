// Unit tests for the WS-14 engine supervisor (src/main/engine-supervisor.js).
// The "engine" is tests/unit/fixtures/fake-engine.mjs — a real spawned
// process speaking the protocol, so lifecycle behaviour (restart budget,
// heartbeat watchdog, graceful shutdown) is exercised for real.

import { describe, it, expect, afterEach } from "vitest";
import path from "node:path";
import fs from "node:fs";
import os from "node:os";
import { fileURLToPath } from "node:url";

import { EngineSupervisor } from "../../src/main/engine-supervisor";

const fakeEngine = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "fixtures",
  "fake-engine.mjs",
);

const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "modv-engine-test-"));
let testCount = 0;

function tempPaths() {
  testCount += 1;
  return {
    socketPath: path.join(tmpDir, `engine-${process.pid}-${testCount}.sock`),
    statePath: path.join(tmpDir, `engine-${process.pid}-${testCount}.json`),
  };
}

const silentLogger = { info() {}, warn() {}, error() {} };

/**
 * EngineSupervisor spawns a single binary path with --socket/--state args.
 * Node cannot be spawned with the fixture as argv[1] via that path alone, so
 * this wrapper creates a shell launcher script per test.
 */
let launcherCount = 0;
function makeLauncher(mode) {
  launcherCount += 1;
  const launcher = path.join(
    tmpDir,
    `engine-launcher-${process.pid}-${launcherCount}.sh`,
  );
  fs.writeFileSync(
    launcher,
    `#!/bin/sh\nexport FAKE_ENGINE_MODE=${mode}\nexec ${JSON.stringify(process.execPath)} ${JSON.stringify(fakeEngine)} "$@"\n`,
  );
  fs.chmodSync(launcher, 0o755);
  return launcher;
}

function makeTestSupervisor({ mode = "ok", ...overrides } = {}) {
  const { socketPath, statePath } = tempPaths();
  const supervisor = new EngineSupervisor({
    binaryPath: makeLauncher(mode),
    socketPath,
    statePath,
    logger: silentLogger,
    restartDelayMs: 20,
    restartBudget: 3,
    heartbeatTimeoutMs: 300,
    connectRetryLimit: 50,
    connectRetryMs: 20,
    ...overrides,
  });
  return { supervisor, socketPath, statePath };
}

async function waitForState(supervisor, wanted, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (supervisor.getState() === wanted) return wanted;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(
    `timed out waiting for engine state "${wanted}" (got "${supervisor.getState()}")`,
  );
}

describe("EngineSupervisor", () => {
  const cleanup = [];

  afterEach(async () => {
    await Promise.all(cleanup.splice(0).map((fn) => fn()));
  });

  it("spawns the engine, reaches running, serves requests and stops cleanly", async () => {
    const { supervisor } = makeTestSupervisor({ mode: "ok" });
    cleanup.push(() => supervisor.shutdown());

    const running = new Promise((resolve) =>
      supervisor.on("engine-state", (t) => t.state === "running" && resolve()),
    );
    void supervisor.start();
    await running;

    const snapshot = await supervisor.requestSnapshot(1000);
    expect(snapshot).toEqual({ fake: true, snapshot: true });

    await supervisor.shutdown();
    expect(supervisor.getState()).toBe("stopped");
  }, 15_000);

  it("writes the snapshot path on graceful shutdown so a relaunch can restore it", async () => {
    const { supervisor, statePath } = makeTestSupervisor({ mode: "ok" });
    cleanup.push(() => supervisor.shutdown());

    void supervisor.start();
    await waitForState(supervisor, "running");
    await supervisor.shutdown();

    expect(fs.existsSync(statePath)).toBe(true);
    expect(JSON.parse(fs.readFileSync(statePath, "utf8"))).toEqual({
      fake: true,
    });
  }, 15_000);

  it("restarts a crashing engine and gives up after the restart budget", async () => {
    const { supervisor } = makeTestSupervisor({
      mode: "crash",
      restartBudget: 3,
      restartDelayMs: 20,
    });
    cleanup.push(() => supervisor.shutdown());

    void supervisor.start();
    await waitForState(supervisor, "fatal", 15_000);
    expect(supervisor.getState()).toBe("fatal");
  }, 20_000);

  it("terminates the engine when heartbeats stop", async () => {
    const { supervisor } = makeTestSupervisor({
      mode: "silent-heartbeat",
      restartBudget: 1,
      restartDelayMs: 20,
      heartbeatTimeoutMs: 300,
    });
    cleanup.push(() => supervisor.shutdown());

    void supervisor.start();
    await waitForState(supervisor, "running");

    // No heartbeat ever arrives; the watchdog must SIGTERM it. The fake
    // engine exits 0 on SIGTERM, gets restarted within the budget of 1, and
    // the second silent run exhausts the budget → fatal.
    await waitForState(supervisor, "fatal", 15_000);
  }, 20_000);

  it("goes fatal immediately when the binary does not exist", async () => {
    const { socketPath, statePath } = tempPaths();
    const supervisor = new EngineSupervisor({
      binaryPath: path.join(tmpDir, "definitely-not-a-binary"),
      socketPath,
      statePath,
      logger: silentLogger,
    });
    cleanup.push(() => supervisor.shutdown());

    await supervisor.start();
    expect(supervisor.getState()).toBe("fatal");
  });
});

describe("EngineSupervisor state transitions", () => {
  it("emits engine-state events in order: starting → running → stopped", async () => {
    const { supervisor } = makeTestSupervisor({ mode: "ok" });
    const transitions = [];
    supervisor.on("engine-state", (t) => transitions.push(t.state));

    await supervisor.start();
    await waitForState(supervisor, "running");
    await supervisor.shutdown();

    expect(transitions[0]).toBe("starting");
    expect(transitions).toContain("running");
    expect(transitions[transitions.length - 1]).toBe("stopped");
  }, 15_000);
});
