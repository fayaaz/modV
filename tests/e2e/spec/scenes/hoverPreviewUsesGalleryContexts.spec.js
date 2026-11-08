import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

/**
 * Scene hover previews used to render through the live composition's shared
 * ISF/shader WebGL canvases, resizing them down to preview size and back to
 * output size on every frame of a hover. That invalidates the live drawing
 * buffers mid-frame. Previews must use the gallery contexts instead, so the
 * live ISF canvas keeps its output size while a cell is hovered.
 */
test("hovering an ISF cell keeps the live ISF canvas at output size", async () => {
  test.setTimeout(120000);

  const setup = await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/stopPerformance");
    await self.store.dispatch("scenes/setRowNames", {
      rowNames: ["Background", "Foreground", "Effects"],
    });

    const scene = self.store.state.scenes.scenes.find(
      (item) => item.name === "MBOX Light Tunnel",
    );

    if (!scene) {
      throw new Error("MBOX Light Tunnel scene not seeded");
    }

    await self.store.dispatch("scenes/applyScene", { sceneId: scene.id });

    const auxByName = (name) =>
      Object.values(self.store.state.outputs.auxillary).find(
        (item) => item && item.name === name,
      );

    const nextFrame = () =>
      new Promise((resolve) => requestAnimationFrame(resolve));

    for (let i = 0; i < 60; i++) {
      await nextFrame();
    }

    const isfLive = auxByName("isf-buffer");

    return {
      sceneId: scene.id,
      liveSize: {
        width: isfLive.context.canvas.width,
        height: isfLive.context.canvas.height,
      },
    };
  });

  // The live composition canvas is sized to the output before the hover.
  expect(setup.liveSize.width).toBeGreaterThan(0);
  expect(setup.liveSize.height).toBeGreaterThan(0);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const fxCell = page.locator(
    `.scenes__gridCell[data-scene-id="${setup.sceneId}"][data-row-index="2"]`,
  );

  await fxCell.hover();
  await page.waitForTimeout(500);

  const during = await modVApp.evaluateWorker(async () => {
    const auxByName = (name) =>
      Object.values(self.store.state.outputs.auxillary).find(
        (item) => item && item.name === name,
      );

    const isfLive = auxByName("isf-buffer");
    const isfGallery = auxByName("isf-buffer-gallery");
    const canvas = isfLive.context.canvas;

    // Count actual width changes on the live ISF canvas: the preview used to
    // shrink it to the cell's size and the next live render grew it back
    // every preview frame.
    let widthChanges = 0;
    const descriptor = Object.getOwnPropertyDescriptor(
      OffscreenCanvas.prototype,
      "width",
    );
    const originalSetter = descriptor && descriptor.set;

    const observe = () => {
      if (originalSetter) {
        Object.defineProperty(OffscreenCanvas.prototype, "width", {
          configurable: true,
          set(value) {
            if (this === canvas && this.width !== value) {
              widthChanges += 1;
            }
            originalSetter.call(this, value);
          },
          get: descriptor.get,
        });
      }
    };

    observe();

    const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

    // Long enough for several 30fps preview frames.
    await wait(1200);

    Object.defineProperty(OffscreenCanvas.prototype, "width", descriptor);

    return {
      liveSize: {
        width: canvas.width,
        height: canvas.height,
      },
      gallerySize: {
        width: isfGallery.context.canvas.width,
        height: isfGallery.context.canvas.height,
      },
      widthChanges,
    };
  });

  // The live composition canvas never changes size to follow the preview.
  expect(during.liveSize.width).toBe(setup.liveSize.width);
  expect(during.liveSize.height).toBe(setup.liveSize.height);
  expect(during.widthChanges).toBe(0);

  await page.locator(".scenes__gridCorner").hover();
  await page.waitForTimeout(200);

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/stopPerformance");
    await self.store.dispatch("scenes/setRowNames", { rowNames: [] });
  });
});
