// Unit tests for the WS-14 control-channel protocol client
// (src/main/engine-protocol.js). A real node:net Unix socket pair stands in
// for the engine, exercising framing, request/response correlation and the
// event path without the Rust binary.

import { describe, it, expect, afterEach } from "vitest";
import net from "node:net";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  EngineProtocolClient,
  encodeFrame,
  KIND_REQUEST,
  KIND_RESPONSE,
  KIND_EVENT,
} from "../../src/main/engine-protocol";

const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "modv-protocol-test-"));
let counter = 0;
const tempSocket = () =>
  path.join(tmpDir, `sock-${process.pid}-${counter++}.sock`);

function frame(kind, payload) {
  return encodeFrame(kind, Buffer.from(JSON.stringify(payload), "utf8"));
}

/** Start a stub engine on a fresh socket path. Returns { path, received, responses }. */
function startStubEngine({ onRequest, sendOnConnect } = {}) {
  const socketPath = tempSocket();
  const received = [];
  const sockets = new Set();

  const server = net.createServer((socket) => {
    sockets.add(socket);
    if (sendOnConnect) socket.write(sendOnConnect);

    let buffer = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);

      for (;;) {
        if (buffer.length < 5) break;
        const length = buffer.readUInt32LE(0);
        if (buffer.length < 5 + length) break;
        const kind = buffer.readUInt8(4);
        const payload = buffer.subarray(5, 5 + length);
        buffer = buffer.subarray(5 + length);
        received.push({ kind, payload: JSON.parse(payload.toString("utf8")) });
        if (onRequest) {
          for (const response of onRequest(
            received[received.length - 1].payload,
          )) {
            socket.write(response);
          }
        }
      }
    });
  });

  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(socketPath, () =>
      resolve({ server, socketPath, received, sockets }),
    );
  });
}

describe("EngineProtocolClient", () => {
  const clients = [];
  const servers = [];

  afterEach(async () => {
    for (const client of clients.splice(0)) client.disconnect();
    await Promise.all(
      servers
        .splice(0)
        .map(({ server }) => new Promise((r) => server.close(r))),
    );
  });

  async function connectClient(socketPath) {
    const client = new EngineProtocolClient();
    clients.push(client);
    await client.connect(socketPath);
    return client;
  }

  it("sends a framed request and resolves with the result", async () => {
    const { server, socketPath, received } = await startStubEngine({
      onRequest: (req) => [
        frame(KIND_RESPONSE, {
          id: req.id,
          ok: true,
          result: { echoed: req.params },
        }),
      ],
    });
    servers.push({ server });

    const client = await connectClient(socketPath);
    const result = await client.request("scenes/applyScene", {
      sceneId: "abc",
    });

    expect(result).toEqual({ echoed: { sceneId: "abc" } });
    expect(received[0].kind).toBe(KIND_REQUEST);
    expect(received[0].payload.method).toBe("scenes/applyScene");
    expect(received[0].payload.params).toEqual({ sceneId: "abc" });
  });

  it("rejects with the engine's error code and message on ok:false", async () => {
    const { server, socketPath } = await startStubEngine({
      onRequest: (req) => [
        frame(KIND_RESPONSE, {
          id: req.id,
          ok: false,
          error: { code: "unknown_scene", message: "no such scene" },
        }),
      ],
    });
    servers.push({ server });

    const client = await connectClient(socketPath);
    await expect(
      client.request("scenes/applyScene", { sceneId: "nope" }),
    ).rejects.toMatchObject({
      code: "unknown_scene",
      message: "no such scene",
    });
  });

  it("times out when the engine never responds", async () => {
    const { server, socketPath } = await startStubEngine();
    servers.push({ server });

    const client = await connectClient(socketPath);
    await expect(client.request("state/snapshot", null, 50)).rejects.toThrow(
      /timed out/,
    );
  });

  it("buffers a response split mid-header without crashing", async () => {
    const { server, socketPath } = await startStubEngine({
      onRequest: (req) => {
        const full = frame(KIND_RESPONSE, {
          id: req.id,
          ok: true,
          result: "done",
        });
        // Deliver only the first 3 bytes — the client must buffer, not parse.
        return [full.subarray(0, 3)];
      },
    });
    servers.push({ server });

    const client = await connectClient(socketPath);
    const request = client.request("engine/shutdown");
    void request.catch(() => {}); // handled: the assertion below is the point
    await new Promise((r) => setTimeout(r, 150));
    expect(client.pending.size).toBe(1); // still waiting, not crashed
  });

  it("emits engine-event for events and tracks preview/ready", async () => {
    const { server, socketPath } = await startStubEngine({
      sendOnConnect: frame(KIND_EVENT, {
        event: "preview/ready",
        data: { path: "/dev/shm/modv-preview-x", slots: 3, slotBytes: 8192 },
      }),
    });
    servers.push({ server });

    const client = await connectClient(socketPath);
    await new Promise((r) => client.once("preview/ready", r));
    expect(client.previewInfo).toEqual({
      path: "/dev/shm/modv-preview-x",
      slots: 3,
      slotBytes: 8192,
    });
  });

  it("emits engine-event for state/delta events", async () => {
    const delta = {
      type: "groups/UPDATE_GROUP",
      payload: { id: "g1", alpha: 0.5 },
    };
    const { server, socketPath } = await startStubEngine({
      sendOnConnect: frame(KIND_EVENT, { event: "state/delta", data: delta }),
    });
    servers.push({ server });

    const client = await connectClient(socketPath);
    const received = await new Promise((r) => client.once("state/delta", r));
    expect(received).toEqual(delta);
  });

  it("rejects pending requests when the socket closes", async () => {
    const { server, socketPath, sockets } = await startStubEngine();
    servers.push({ server });

    const client = await connectClient(socketPath);
    const pending = client.request("state/snapshot");
    for (const socket of sockets) socket.destroy();
    await expect(pending).rejects.toThrow(/disconnected/);
  });
});

describe("encodeFrame", () => {
  it("writes the spec header: u32 LE length followed by u8 kind", () => {
    const payload = Buffer.from("{}", "utf8");
    const encoded = encodeFrame(KIND_REQUEST, payload);
    expect(encoded.readUInt32LE(0)).toBe(payload.length);
    expect(encoded.readUInt8(4)).toBe(0);
    expect(encoded.subarray(5).equals(payload)).toBe(true);
  });
});
