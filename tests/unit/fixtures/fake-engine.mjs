// Fake engine used by tests/unit/engine-supervisor.test.js.
//
// Speaks just enough of the WS-14 protocol for the supervisor to connect,
// receive heartbeats and drive a graceful shutdown.
//
// Modes (FAKE_ENGINE_MODE env var):
//   "ok"               — bind, heartbeat 10 Hz, answer requests (default)
//   "silent-heartbeat" — bind and answer requests, never send heartbeats
//   "crash"            — exit(1) immediately
//
// Usage: node fake-engine.js --socket <path> --state <path>

import net from "node:net";
import fs from "node:fs";

const argv = process.argv.slice(2);
const args = {};
for (let i = 0; i < argv.length; i += 1) {
  if (argv[i].startsWith("--")) args[argv[i].slice(2)] = argv[i + 1];
}

const socketPath = args.socket;
const statePath = args.state;
const mode = process.env.FAKE_ENGINE_MODE || "ok";

if (mode === "crash") {
  process.exit(1);
}

fs.rmSync(socketPath, { force: true });

const sockets = new Set();
const server = net.createServer((socket) => {
  sockets.add(socket);
  socket.on("close", () => sockets.delete(socket));
  socket.on("error", () => sockets.delete(socket));

  let buffer = Buffer.alloc(0);
  socket.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);

    for (;;) {
      if (buffer.length < 5) return;
      const length = buffer.readUInt32LE(0);
      if (buffer.length < 5 + length) return;
      const kind = buffer.readUInt8(4);
      const payload = buffer.subarray(5, 5 + length);
      buffer = buffer.subarray(5 + length);

      if (kind !== 0) continue;

      const request = JSON.parse(payload.toString("utf8"));
      let body = { id: request.id, ok: true, result: null };
      if (request.method === "state/snapshot") {
        body = { id: request.id, ok: true, result: { fake: true, snapshot: true } };
      } else if (request.method === "unknown/method") {
        body = {
          id: request.id,
          ok: false,
          error: { code: "unknown_method", message: "no such method" },
        };
      }

      const out = Buffer.from(JSON.stringify(body), "utf8");
      const header = Buffer.allocUnsafe(5);
      header.writeUInt32LE(out.length, 0);
      header.writeUInt8(1, 4);
      socket.write(Buffer.concat([header, out]));
    }
  });
});

const heartbeat = setInterval(() => {
  if (mode === "silent-heartbeat") return;
  const event = Buffer.from(
    JSON.stringify({ event: "engine/heartbeat", data: { uptime: process.uptime(), frames: 1 } }),
    "utf8",
  );
  const header = Buffer.allocUnsafe(5);
  header.writeUInt32LE(event.length, 0);
  header.writeUInt8(2, 4);
  for (const socket of sockets) {
    socket.write(Buffer.concat([header, event]));
  }
}, 100);
heartbeat.unref();

server.listen(socketPath);

process.on("SIGTERM", () => {
  try {
    if (statePath) fs.writeFileSync(statePath, JSON.stringify({ fake: true }));
  } catch (_) {
    // best effort
  }
  server.close();
  process.exit(0);
});
process.on("SIGINT", () => process.exit(0));
