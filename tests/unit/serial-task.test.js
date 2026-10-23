import { describe, expect, it } from "vitest";
import { createSerialTask } from "../../src/renderer/src/application/worker/serial-task.js";

const deferred = () => {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });

  return { promise, resolve, reject };
};

describe("createSerialTask", () => {
  it("runs concurrent tasks one after another", async () => {
    const schedule = createSerialTask();
    const first = deferred();
    const order = [];

    const a = schedule(async () => {
      order.push("a:start");
      await first.promise;
      order.push("a:end");
      return "a";
    });

    const b = schedule(async () => {
      order.push("b:start");
      return "b";
    });

    await Promise.resolve();
    expect(order).toEqual(["a:start"]);

    first.resolve();
    await expect(a).resolves.toBe("a");
    await expect(b).resolves.toBe("b");
    expect(order).toEqual(["a:start", "a:end", "b:start"]);
  });

  it("keeps running after a task rejects", async () => {
    const schedule = createSerialTask();

    await expect(
      schedule(async () => {
        throw new Error("boom");
      }),
    ).rejects.toThrow("boom");

    await expect(schedule(async () => "ok")).resolves.toBe("ok");
  });
});
