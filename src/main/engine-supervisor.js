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

const RESTART_BUDGET = 5;
const RESTART_WINDOW_MS = 60_000;
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

class EngineSupervisor extends EventEmitter {
  constructor({
    binaryPath,
    socketPath,
    statePath,
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
    this.log.info(
      `starting engine: ${this.binaryPath} --socket ${this.socketPath} --state ${this.statePath}`,
    );

    let child;
    try {
      child = spawn(
        this.binaryPath,
        ["--socket", this.socketPath, "--state", this.statePath],
        {
          stdio: ["ignore", "pipe", "pipe"],
          env: { ...process.env },
        },
      );
    } catch (error) {
      this.log.error(`failed to spawn the engine: ${error.message}`);
      this._scheduleRestart("spawn-failed");
      return;
    }
    this.child = child;
    this._exitHandled = false;
    attachEngineLogForwarder(child, this.log);

    child.on("error", (error) => {
      this.log.error(`engine process error: ${error.message}`);
    });
    child.on("exit", (code, signal) => {
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
      this._terminateChild();
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
    const now = Date.now();
    this.restartTimestamps = [
      ...this.restartTimestamps.filter((t) => now - t < RESTART_WINDOW_MS),
      now,
    ];

    if (this.restartTimestamps.length > this.restartBudget) {
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

  _terminateChild(signal = "SIGTERM") {
    if (this.child && this.child.exitCode === null) {
      try {
        this.child.kill(signal);
      } catch (_) {
        // already gone
      }
    }
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
