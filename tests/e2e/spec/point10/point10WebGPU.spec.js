import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

const CANVAS_SIZE = 96;
const MODULE_NAMES = ["Point10 Rolling Fire", "Point10 Smoke Room"];

test.describe.configure({ mode: "serial" });

for (const moduleName of MODULE_NAMES) {
  test(`${moduleName} presents through the WebGPU backend`, async () => {
    test.setTimeout(120000);

    const result = await modVApp.evaluateWorker(
      async ({ name, size }) => {
        const originalWarn = console.warn;
        const originalError = console.error;
        const warnings = [];
        const capture = (...args) => {
          warnings.push(args.map((value) => String(value)).join(" "));
        };

        const module = await self.store.dispatch("modules/makeActiveModule", {
          moduleName: name,
        });

        if (!module) {
          return { missing: true, warnings };
        }

        const canvas = new OffscreenCanvas(size, size);
        const context = canvas.getContext("2d", { willReadFrequently: true });

        const yieldTask = () =>
          new Promise((resolve) => {
            const { port1, port2 } = new MessageChannel();
            port1.onmessage = () => {
              port1.close();
              port2.close();
              resolve();
            };
            port2.postMessage(0);
          });

        const drawAndCount = () => {
          context.clearRect(0, 0, size, size);
          self.store.state.modules.registered[name].draw({
            canvas,
            context,
            features: { p10: null },
            props: module.props,
            moduleId: module.$id,
            isGallery: false,
          });

          const pixels = context.getImageData(0, 0, size, size).data;
          let lit = 0;

          for (let p = 0; p < pixels.length; p += 4) {
            if (pixels[p + 3] > 8) {
              lit++;
            }
          }

          return lit;
        };

        console.warn = capture;
        console.error = capture;

        try {
          const deadline = performance.now() + 30000;
          let lit = 0;

          while (performance.now() < deadline) {
            // WebGPU scenes build their compute/render pipelines asynchronously.
            for (let i = 0; i < 25; i++) {
              await yieldTask();
            }

            lit = drawAndCount();

            if (lit > 0) {
              break;
            }
          }

          return { lit, total: size * size, warnings };
        } catch (error) {
          return { error: String(error), warnings };
        } finally {
          console.warn = originalWarn;
          console.error = originalError;
          await self.store.dispatch("modules/removeActiveModule", {
            moduleId: module.$id,
          });
        }
      },
      { name: moduleName, size: CANVAS_SIZE },
    );

    expect(result.missing, moduleName).not.toBe(true);
    expect(result.error, moduleName).toBeUndefined();
    expect(
      result.warnings.filter((line) =>
        /storageTexture|WebGPU not available/i.test(line),
      ),
      moduleName,
    ).toEqual([]);
    expect(result.lit, moduleName).toBeGreaterThan(0);
  });
}
