import { builtinModules } from "node:module";
import { resolve } from "path";
import { defineConfig, externalizeDepsPlugin } from "electron-vite";
import { esmExternalRequirePlugin } from "vite";
import vue from "@vitejs/plugin-vue";
import svgLoader from "vite-svg-loader";
import { nodePolyfills } from "vite-plugin-node-polyfills";

const nodeBuiltins = builtinModules.flatMap((moduleName) => [
  moduleName,
  `node:${moduleName}`,
]);

// The renderer worker runs with nodeIntegrationInWorker, so it must keep
// calling the real Node `require` for built-ins and native addons instead of
// receiving Vite's empty browser externals.
const workerExternals = [...nodeBuiltins, "grandiose"];

export default defineConfig({
  main: {
    plugins: [
      externalizeDepsPlugin({
        include: ["webpack"],
      }),
    ],
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
    build: {
      rollupOptions: {
        input: {
          index: resolve(__dirname, "src/preload/index.js"),
          colorPicker: resolve(__dirname, "src/preload/colorPicker.js"),
        },
      },
    },
  },
  renderer: {
    build: {
      rolldownOptions: {
        input: {
          index: resolve(__dirname, "src/renderer/index.html"),
          outputWindow: resolve(__dirname, "src/renderer/output-window.html"),
          colorPicker: resolve(__dirname, "src/renderer/color-picker.html"),
          splashScreen: resolve(__dirname, "src/renderer/splash-screen.html"),
        },
        external: ["grandiose"],
      },
    },
    resolve: {
      alias: [{ find: "@renderer", replacement: resolve("src/renderer/src") }],
    },
    plugins: [
      nodePolyfills({
        include: ["events"],
      }),
      vue({
        template: {
          compilerOptions: {
            isCustomElement: (tag) => ["grid", "c"].includes(tag),
          },
        },
      }),
      svgLoader(),
    ],
    worker: {
      format: "es",
      plugins: () => [
        esmExternalRequirePlugin({
          external: workerExternals,
          skipDuplicateCheck: true,
        }),
      ],
      rolldownOptions: {
        external: workerExternals,
      },
    },
  },
});
