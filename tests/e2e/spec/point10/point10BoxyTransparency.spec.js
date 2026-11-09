import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

const CANVAS_SIZE = 96;
const MODULE_NAME = "Point10 Boxy";

test.setTimeout(60000);

test("Point10 Boxy renders on a transparent background", async () => {
  const result = await modVApp.evaluateWorker(
    async ({ moduleName, size }) => {
      const definition = self.store.state.modules.registered[moduleName];

      if (!definition) {
        throw new Error(`Module "${moduleName}" is not registered`);
      }

      const props = {};

      for (const [key, prop] of Object.entries(definition.props || {})) {
        props[key] =
          prop && typeof prop === "object" && "default" in prop
            ? prop.default
            : prop;
      }

      const canvas = new OffscreenCanvas(size, size);
      const context = canvas.getContext("2d");
      const features = { p10: null };
      const moduleId = `e2e-boxy-transparency-${Date.now()}`;

      const draw = () => {
        context.clearRect(0, 0, size, size);
        definition.draw({
          canvas,
          context,
          features,
          props,
          moduleId,
          isGallery: false,
        });
      };

      // MessageChannel yields a macrotask without timers, so scene draws still
      // settle async loads when the e2e window is occluded (which throttles
      // requestAnimationFrame).
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

      // The scene loads its textures and cubemap asynchronously, so draw
      // across several frames before sampling the composited result.
      for (let i = 0; i < 90; i++) {
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
        } else if (pixels[i + 3] === 255) {
          opaque++;
        }
      }

      // Composite at 50% over solid magenta: transparent pixels must leave
      // the magenta background untouched, which is what layer alpha needs.
      const composite = new OffscreenCanvas(size, size);
      const compositeContext = composite.getContext("2d");

      compositeContext.fillStyle = "#ff00ff";
      compositeContext.fillRect(0, 0, size, size);
      compositeContext.globalAlpha = 0.5;
      compositeContext.drawImage(canvas, 0, 0);

      const composited = compositeContext.getImageData(0, 0, size, size).data;
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
    },
    { moduleName: MODULE_NAME, size: CANVAS_SIZE },
  );

  expect(result.transparent).toBeGreaterThan(result.total * 0.2);
  expect(result.opaque).toBeGreaterThan(0);
  expect(result.preserved).toBeGreaterThan(result.total * 0.2);
});
