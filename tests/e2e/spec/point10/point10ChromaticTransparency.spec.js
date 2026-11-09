import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

const CANVAS_SIZE = 96;
const MODULE_NAMES = ["Point10 Chromatic Line", "Point10 Chromatic Wave"];

test.setTimeout(60000);

for (const moduleName of MODULE_NAMES) {
  test(`${moduleName} renders on a transparent background`, async () => {
    const result = await modVApp.evaluateWorker(
      async ({ name, size }) => {
        const module = await self.store.dispatch("modules/makeActiveModule", {
          moduleName: name,
        });

        if (!module) {
          throw new Error(`Module "${name}" is not registered`);
        }

        const definition = self.store.state.modules.registered[name];
        const canvas = new OffscreenCanvas(size, size);
        const context = canvas.getContext("2d", { willReadFrequently: true });

        // MessageChannel yields a macrotask without timers, so scene draws
        // settle when the e2e window is occluded (which throttles rAF).
        const nextFrame = () =>
          new Promise((resolve) => {
            const { port1, port2 } = new MessageChannel();
            port1.onmessage = () => {
              port1.close();
              port2.close();
              resolve();
            };
            port2.postMessage(0);
          });

        const draw = () => {
          context.clearRect(0, 0, size, size);
          definition.draw({
            canvas,
            context,
            features: { p10: null },
            props: module.props,
            moduleId: module.$id,
            isGallery: false,
          });
        };

        try {
          for (let i = 0; i < 10; i++) {
            draw();
            await nextFrame();
          }

          const pixels = context.getImageData(0, 0, size, size).data;
          const total = size * size;
          let transparent = 0;
          let opaque = 0;

          for (let i = 0; i < pixels.length; i += 4) {
            if (pixels[i + 3] === 0) {
              transparent++;
            } else if (pixels[i + 3] > 200) {
              opaque++;
            }
          }

          // Composite at 50% over solid magenta: transparent pixels must leave
          // the magenta background untouched.
          const composite = new OffscreenCanvas(size, size);
          const compositeContext = composite.getContext("2d");

          compositeContext.fillStyle = "#ff00ff";
          compositeContext.fillRect(0, 0, size, size);
          compositeContext.globalAlpha = 0.5;
          compositeContext.drawImage(canvas, 0, 0);

          const composited = compositeContext.getImageData(
            0,
            0,
            size,
            size,
          ).data;
          let preserved = 0;

          for (let i = 0; i < composited.length; i += 4) {
            if (
              composited[i] >= 240 &&
              composited[i + 1] <= 15 &&
              composited[i + 2] >= 240 &&
              composited[i + 3] === 255
            ) {
              preserved++;
            }
          }

          return { transparent, opaque, preserved, total };
        } finally {
          await self.store.dispatch("modules/removeActiveModule", {
            moduleId: module.$id,
          });
        }
      },
      { name: moduleName, size: CANVAS_SIZE },
    );

    expect(result.transparent).toBeGreaterThan(result.total * 0.2);
    expect(result.opaque).toBeGreaterThan(0);
    expect(result.preserved).toBeGreaterThan(result.total * 0.2);
  });
}
