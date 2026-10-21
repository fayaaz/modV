import { resolve } from "path";
import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: [{ find: "@renderer", replacement: resolve("src/renderer/src") }],
  },
  test: {
    environment: "node",
    include: ["tests/unit/**/*.test.js"],
  },
});
