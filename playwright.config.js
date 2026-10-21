import { defineConfig, expect } from "@playwright/test";
import extensions from "./tests/e2e/extentions";

expect.extend(extensions);

export default defineConfig({
  testDir: "./tests/e2e/spec",
  // Each worker launches its own Electron app and the suite is GPU-bound: a
  // second app over-subscribes /dev/dri and Chromium aborts the renderer
  // (SIGTRAP), which fails whichever specs were running. One worker is as
  // fast as two (~49s for the full suite) and far more stable. Override with
  // MODV_E2E_WORKERS only if the GPU can take it.
  workers: Number(process.env.MODV_E2E_WORKERS || 1),
});
