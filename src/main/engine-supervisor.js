// WS-14 — supervisor for the Rust engine process (see docs/rust-hybrid.md,
// "Supervision contract").
//
// Owns the engine's lifecycle: spawn (with --socket/--state), 2 s connection
// retry, heartbeat watchdog (no heartbeat for 5 s → SIGTERM, SIGKILL 1 s
// later), restart budget (5 restarts in 60 s, then a fatal state the UI can
// surface), and graceful shutdown (engine/shutdown, wait ≤ 1 s, then kill).
//
// State restore: the engine loads its --state snapshot itself at startup and
// flushes it on engine/shutdown and periodically while running, so a
// crash-and-restart restores the last snapshot simply by re-spawning with the
// same --state path. No snapshot bytes travel through this process.
//
// The engine is a child of the Electron main process; Playwright/e2e override
// the binary with MODV_ENGINE_BIN.

import { spawn } from "node:child_process";
import path from "node:path";
import fs from "node:fs";
import os from "node:os";
import { EventEmitter } from "node:events";

import { EngineProtocolClient } from "./engine-protocol";
import { createLogger } from "./logger";
import { attachEngineLogForwarder } from "./engine-log-forwarder";
import {
  RESTART_BUDGET,
  RESTART_WINDOW_MS,
  recordRestartAttempt,
  resetRestartBudget,
} from "./engine-restart-budget";

const RESTART_DELAY_MS = 500;
const HEARTBEAT_TIMEOUT_MS = 5_000;
const HEARTBEAT_CHECK_MS = 1_000;
const CONNECT_RETRY_MS = 50;
const CONNECT_RETRY_LIMIT = 100; // 5 s to bind the socket after spawn
const SHUTDOWN_WAIT_MS = 1_000;
const SIGKILL_DELAY_MS = 1_000;

function defaultSocketPath() {
  return path.join(os.tmpdir(), `modv-engine-${process.pid}.sock`);
}

/**
 * Resolve the engine binary: MODV_ENGINE_BIN (tests) → packaged
 * process.resourcesPath/modv-engine → crates/target/release/modv-engine
 * (development).
 */
function defaultEngineBinary() {
  if (process.env.MODV_ENGINE_BIN) return process.env.MODV_ENGINE_BIN;
  const app = getElectronApp();
  if (app) {
    if (app.isPackaged) {
      return path.join(process.resourcesPath, "modv-engine");
    }
    return path.join(
      app.getAppPath(),
      "crates",
      "target",
      "release",
      "modv-engine",
    );
  }
  // Outside Electron (unit tests): fall back to the dev path relative to cwd.
  return path.join(process.cwd(), "crates", "target", "release", "modv-engine");
}

function getElectronApp() {
  try {
    // electron is external in the bundle; import lazily so unit tests can run
    // against plain Node without Electron installed.
    return require("electron").app;
  } catch (_) {
    return null;
  }
}

/**
 * Resolve the ISF corpus directory: packaged → `resources/isf`
 * (electron-builder `extraResources`), development → the vendored source tree.
 * Returns null outside Electron (unit tests), where the engine falls back to
 * `$MODV_ISF_DIR` or its own development lookup.
 */
function defaultAssetsDir() {
  const app = getElectronApp();
  if (!app) return null;
  if (app.isPackaged) {
    return path.join(process.resourcesPath, "textures");
  }
  return path.join(
    app.getAppPath(),
    "src",
    "renderer",
    "src",
    "application",
    "point10",
    "textures",
  );
}

function defaultIsfDir() {
  const app = getElectronApp();
  if (!app) return null;
  if (app.isPackaged) {
    return path.join(process.resourcesPath, "isf");
  }
  return path.join(
    app.getAppPath(),
    "src",
    "renderer",
    "src",
    "application",
    "renderers",
    "isf",
    "modules",
    "isf",
  );
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

class EngineSupervisor extends EventEmitter {
  constructor({
    binaryPath,
    socketPath,
    statePath,
    isfDir,
    assetsDir,
    logger,
    clientFactory,
    // Test overrides (production uses the constants above).
    restartDelayMs = RESTART_DELAY_MS,
    restartBudget = RESTART_BUDGET,
    heartbeatTimeoutMs = HEARTBEAT_TIMEOUT_MS,
    connectRetryLimit = CONNECT_RETRY_LIMIT,
    connectRetryMs = CONNECT_RETRY_MS,
    shutdownWaitMs = SHUTDOWN_WAIT_MS,
  } = {}) {
    super();
    this.binaryPath = binaryPath || defaultEngineBinary();
    this.socketPath = socketPath || defaultSocketPath();
    // Resolved once: the packaged app ships `resources/isf`, development
    // reads the vendored corpus. `null` leaves the engine's own lookup
    // (`$MODV_ISF_DIR` / dev default) in charge.
    this.isfDir = isfDir === undefined ? defaultIsfDir() : isfDir;
    // Point10 scene textures (tinore maps, panoramas, PCB, ...). Packaged
    // builds ship them outside the asar; the engine resolves
    // `$MODV_ASSETS_DIR` before its source-tree dev fallback.
    this.assetsDir = assetsDir === undefined ? defaultAssetsDir() : assetsDir;
    this.restartBudget = restartBudget;
    this.restartDelayMs = restartDelayMs;
    this.heartbeatTimeoutMs = heartbeatTimeoutMs;
    this.connectRetryLimit = connectRetryLimit;
    this.connectRetryMs = connectRetryMs;
    this.shutdownWaitMs = shutdownWaitMs;
    this.statePath =
      statePath ||
      (() => {
        const app = getElectronApp();
        const userData = app ? app.getPath("userData") : os.tmpdir();
        return path.join(userData, "engine-state.json");
      })();
    this.log = logger || createLogger("engine");
    this._clientFactory = clientFactory || (() => new EngineProtocolClient());

    this.client = null;
    this.child = null;
    this.state = "stopped";
    this.stopRequested = false;
    this.restartTimestamps = [];
    this.lastHeartbeat = 0;
    this._watchdogTimer = null;
    this._sigkillTimer = null;
    this._startPromise = null;
    this._exitHandled = false;
  }

  /** Start (or no-op while a start is already in flight). */
  start() {
    if (this._startPromise) return this._startPromise;
    this.stopRequested = false;
    // A deliberate start is a new lifecycle: never inherit the crash budget
    // from a previous run (review R-24).
    this.restartTimestamps = resetRestartBudget();
    this._startPromise = this._spawn().finally(() => {
      this._startPromise = null;
    });
    return this._startPromise;
  }

  isRunning() {
    return this.state === "running";
  }

  getState() {
    return this.state;
  }

  /** Subscribe to engine lifecycle transitions: { state, reason? }. */
  onEngineState(callback) {
    this.on("engine-state", callback);
    return () => this.removeListener("engine-state", callback);
  }

  /** Graceful stop: engine/shutdown (≤ 1 s), then SIGTERM/SIGKILL. */
  async shutdown() {
    this.stopRequested = true;
    // A clean stop clears the crash history: the next start() gets a fresh
    // budget instead of inheriting the crashes of a previous session.
    this.restartTimestamps = resetRestartBudget();
    this._stopWatchdog();

    if (this.client) {
      try {
        await this.client.request("engine/shutdown", null, this.shutdownWaitMs);
      } catch (_) {
        // already gone or very slow — fall through to the kill path
      }
    }

    this._kill();

    // Give the engine a moment to exit, then force the issue if it has not.
    const deadline = Date.now() + this.shutdownWaitMs + 250;
    while (
      this.child &&
      this.child.exitCode === null &&
      Date.now() < deadline
    ) {
      await sleep(50);
    }
    if (this.child && this.child.exitCode === null) {
      this._stopWatchdog();
      this._kill("SIGKILL");
    }

    this._setState("stopped");
  }

  /** Ask the engine for a state snapshot (WS-14 state/snapshot). */
  requestSnapshot(timeoutMs = 2000) {
    if (!this.client) return Promise.reject(new Error("engine is not running"));
    return this.client.request("state/snapshot", null, timeoutMs);
  }

  async _spawn() {
    if (!fs.existsSync(this.binaryPath)) {
      this.log.error(`engine binary not found at ${this.binaryPath}`);
      this._setState("fatal", "binary-missing");
      return;
    }

    this._stopWatchdog();
    this._teardownClient();

    // NOTE: fs.rmSync throws in this Electron build when the path does not
    // exist; check first and unlink instead.
    try {
      if (fs.existsSync(this.socketPath)) {
        fs.unlinkSync(this.socketPath);
      }
    } catch (error) {
      this.log.warn(
        `engine: could not remove stale socket ${this.socketPath}: ${error.message}`,
      );
    }

    this._setState("starting");
    // The engine owns the snapshot file: it restores from it at startup and
    // flushes it on shutdown, so a restart needs no state bytes from us.
    const args = ["--socket", this.socketPath, "--state", this.statePath];
    if (this.isfDir) {
      args.push("--isf-dir", this.isfDir);
    }
    this.log.info(`starting engine: ${this.binaryPath} ${args.join(" ")}`);

    let child;
    try {
      const env = { ...process.env };
      if (this.assetsDir) {
        env.MODV_ASSETS_DIR = this.assetsDir;
      }
      child = spawn(this.binaryPath, args, {
        stdio: ["ignore", "pipe", "pipe"],
        env,
      });
    } catch (error) {
      this.log.error(`failed to spawn the engine: ${error.message}`);
      this._scheduleRestart("spawn-failed");
      return;
    }
    this.child = child;
    this._exitHandled = false;
    attachEngineLogForwarder(child, this.log);

    child.on("error", (error) => {
      // A spawn failure (ENOENT/EACCES/not-executable packaged binary) arrives
      // here without an 'exit' event. Route it through the same decision as an
      // exit so the restart budget handles it immediately instead of leaving
      // the supervisor in "starting" until the connect retries run out
      // (review R-24). Once the engine is running, an 'error' means a failed
      // signal/stdio operation: wait for the real 'exit' so a still-alive
      // process is not orphaned by a second spawn.
      if (this.child !== child) return;
      this.log.error(`engine process error: ${error.message}`);
      if (this.state !== "starting") return;
      this._onExit(null, `spawn-error: ${error.code || error.message}`);
    });
    child.on("exit", (code, signal) => {
      // A late exit from a child we already replaced must not reset the new
      // child's `_exitHandled`/restart accounting.
      if (this.child !== child) return;
      this._onExit(code, signal);
    });

    this.client = this._clientFactory();
    this.client.on("protocol-error", (error) =>
      this.log.error(`engine protocol error: ${error.message}`),
    );
    this.client.on("engine/heartbeat", () => {
      this.lastHeartbeat = Date.now();
    });

    const connected = await this._connectWithRetry();
    if (!connected) {
      // Treat a missing socket as an engine failure and go through the
      // restart budget rather than blocking startup forever.
      //
      // Stop the old engine first and wait (bounded) for it to actually exit:
      // the next `_spawn` unlinks the socket path, and doing that while the
      // dying engine still owns it leaves a window where neither process can
      // be connected to (review R-24).
      const oldChild = this.child;
      this.log.warn("engine did not connect; terminating it before restarting");
      this._terminateChild("SIGTERM", oldChild);

      let exited = await this._awaitChildExit(oldChild, this.shutdownWaitMs);
      if (!exited) {
        this.log.warn(
          `engine still running ${this.shutdownWaitMs} ms after SIGTERM; sending SIGKILL`,
        );
        this._terminateChild("SIGKILL", oldChild);
        exited = await this._awaitChildExit(oldChild, SIGKILL_DELAY_MS);
      }
      if (!exited) {
        // Give up waiting (the process is unkillable or a zombie): restart
        // anyway, but make the overlap explicit in the log.
        this.log.error(
          "engine did not exit after SIGKILL; restarting anyway (the socket may be replaced)",
        );
      }

      this._onExit(null, "connect-timeout");
    }
  }

  async _connectWithRetry() {
    for (let attempt = 0; attempt < this.connectRetryLimit; attempt += 1) {
      if (this.state !== "starting") return false;
      try {
        await this.client.connect(
          this.socketPath,
          Math.max(this.connectRetryMs * 2, 100),
        );
        this._onConnected();
        return true;
      } catch (_) {
        await sleep(this.connectRetryMs);
      }
    }
    return false;
  }

  _onConnected() {
    this._setState("running");
    this.lastHeartbeat = Date.now();
    this._startWatchdog();
    this.log.info("engine connected");
  }

  _onExit(code, signal) {
    // Both the child's 'exit' event and our own connect-timeout path funnel
    // here — only the first one counts for the restart budget.
    if (this._exitHandled) return;
    this._exitHandled = true;

    this._stopWatchdog();
    this._teardownClient();

    if (this.state === "fatal") return;
    if (this.stopRequested) {
      this._setState("stopped");
      return;
    }

    this.log.error(`engine exited (code=${code} signal=${signal})`);
    this._scheduleRestart("engine-exited");
  }

  _scheduleRestart(reason) {
    const { timestamps, exhausted } = recordRestartAttempt(
      this.restartTimestamps,
      Date.now(),
      { budget: this.restartBudget, windowMs: RESTART_WINDOW_MS },
    );
    this.restartTimestamps = timestamps;

    if (exhausted) {
      this.log.error(
        `engine restart budget exhausted (${this.restartBudget} in ${RESTART_WINDOW_MS / 1000}s)`,
      );
      this._setState("fatal", "restart-budget-exhausted");
      return;
    }

    this._setState("restarting", reason);
    setTimeout(() => {
      if (!this.stopRequested) void this._spawn();
    }, this.restartDelayMs).unref?.();
  }

  _startWatchdog() {
    this._stopWatchdog();
    this._watchdogTimer = setInterval(() => {
      if (this.state !== "running") return;
      if (Date.now() - this.lastHeartbeat > this.heartbeatTimeoutMs) {
        this.log.error("engine heartbeat lost; terminating");
        this._terminateChild("SIGTERM");
        this._sigkillTimer = setTimeout(
          () => this._terminateChild("SIGKILL"),
          SIGKILL_DELAY_MS,
        );
        this._sigkillTimer.unref?.();
      }
    }, HEARTBEAT_CHECK_MS);
    this._watchdogTimer.unref?.();
  }

  _stopWatchdog() {
    if (this._watchdogTimer) clearInterval(this._watchdogTimer);
    this._watchdogTimer = null;
    if (this._sigkillTimer) clearTimeout(this._sigkillTimer);
    this._sigkillTimer = null;
  }

  _teardownClient() {
    if (this.client) {
      try {
        // Strip listeners so a late close event cannot double-report.
        this.client.removeAllListeners();
        this.client.disconnect();
      } catch (_) {
        // already dead
      }
      this.client = null;
    }
    this.child = null;
  }

  _terminateChild(signal = "SIGTERM", child = this.child) {
    if (child && child.exitCode === null) {
      try {
        child.kill(signal);
      } catch (_) {
        // already gone
      }
    }
  }

  /**
   * Wait (bounded) for a spawned child to actually exit.
   *
   * `exitCode` stays null until the 'exit' event fires; a process that ignores
   * SIGTERM would otherwise let the next `_spawn` unlink the socket it still
   * holds. Returns true when the child is gone, false after the timeout.
   */
  async _awaitChildExit(child, timeoutMs) {
    const hasExited = () =>
      !child || child.exitCode !== null || child.signalCode !== null;

    if (hasExited()) return true;

    const deadline = Date.now() + Math.max(0, timeoutMs);
    while (Date.now() < deadline) {
      await sleep(20);
      if (hasExited()) return true;
    }

    return hasExited();
  }

  _kill(signal = "SIGTERM") {
    this._terminateChild(signal);
  }

  _setState(state, reason) {
    this.state = state;
    this.emit("engine-state", { state, reason });
  }
}

export {
  EngineSupervisor,
  defaultEngineBinary,
  defaultSocketPath,
  RESTART_BUDGET,
  RESTART_WINDOW_MS,
  HEARTBEAT_TIMEOUT_MS,
};
