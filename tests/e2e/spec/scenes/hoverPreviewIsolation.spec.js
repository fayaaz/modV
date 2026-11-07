import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

/**
 * Hover previews render Scene row modules through the same renderer code the
 * live composition uses. ISF modules all share one WebGL context and one unit
 * quad binding, so disposing a preview's ISF renderer used to delete the
 * vertex buffer every live ISF module draws through. The ISF layers (MBOX3
 * backgrounds, Bloom/Vignette effects, digital-crystal-tunnel, ...) then
 * silently rendered nothing - the hovered foreground "took over" the output
 * and effects only reappeared while an ISF cell was being previewed.
 */
test("hovering an ISF grid cell keeps the live ISF layers rendering", async () => {
  test.setTimeout(120000);

  const setup = await modVApp.evaluateWorker(async () => {
    const waitFrames = async (count) => {
      const nextFrame = () =>
        new Promise((resolve) => requestAnimationFrame(resolve));

      for (let i = 0; i < count; i++) {
        await nextFrame();
      }
    };

    const coverage = (context) => {
      const { width, height } = context.canvas;
      const data = context.getImageData(0, 0, width, height).data;
      let covered = 0;

      for (let i = 3; i < data.length; i += 4) {
        if (data[i] > 8) {
          covered += 1;
        }
      }

      return covered / (width * height);
    };

    const groupCoverage = (name) => {
      const group = self.store.state.groups.groups.find(
        (item) => item && item.name === name && item.sceneId,
      );

      if (!group || !group.context) {
        return null;
      }

      return coverage(group.context.context);
    };

    const isfCenterPixel = () => {
      const outputs = self.store.state.outputs.auxillary;
      const output = Object.values(outputs).find(
        (item) => item && item.name === "isf-buffer",
      );

      if (!output || typeof output.context.readPixels !== "function") {
        return null;
      }

      const pixels = new Uint8Array(4);

      output.context.readPixels(
        Math.floor(output.context.canvas.width / 2),
        Math.floor(output.context.canvas.height / 2),
        1,
        1,
        output.context.RGBA,
        output.context.UNSIGNED_BYTE,
        pixels,
      );

      return Array.from(pixels);
    };

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
    await waitFrames(150);

    return {
      sceneId: scene.id,
      backgroundCoverage: groupCoverage("Background"),
      isfCenterPixel: isfCenterPixel(),
    };
  });

  expect(setup.backgroundCoverage).toBeGreaterThan(0.9);
  expect(setup.isfCenterPixel[3]).toBe(255);

  const { page } = modVApp;
  await page.locator('.lm_tab[title="Scenes"]').click();
  await expect(page.locator(".scenes__grid")).toBeVisible();

  const fxCell = page.locator(
    `.scenes__gridCell[data-scene-id="${setup.sceneId}"][data-row-index="2"]`,
  );

  await fxCell.hover();
  await page.waitForTimeout(300);
  await page.locator(".scenes__gridCorner").hover();
  await page.waitForTimeout(200);

  const after = await modVApp.evaluateWorker(async () => {
    const waitFrames = async (count) => {
      const nextFrame = () =>
        new Promise((resolve) => requestAnimationFrame(resolve));

      for (let i = 0; i < count; i++) {
        await nextFrame();
      }
    };

    const coverage = (context) => {
      const { width, height } = context.canvas;
      const data = context.getImageData(0, 0, width, height).data;
      let covered = 0;

      for (let i = 3; i < data.length; i += 4) {
        if (data[i] > 8) {
          covered += 1;
        }
      }

      return covered / (width * height);
    };

    const group = self.store.state.groups.groups.find(
      (item) => item && item.name === "Background" && item.sceneId,
    );

    const outputs = self.store.state.outputs.auxillary;
    const isfOutput = Object.values(outputs).find(
      (item) => item && item.name === "isf-buffer",
    );

    const pixels = new Uint8Array(4);

    isfOutput.context.readPixels(
      Math.floor(isfOutput.context.canvas.width / 2),
      Math.floor(isfOutput.context.canvas.height / 2),
      1,
      1,
      isfOutput.context.RGBA,
      isfOutput.context.UNSIGNED_BYTE,
      pixels,
    );

    await waitFrames(10);

    return {
      backgroundCoverage: coverage(group.context.context),
      isfCenterPixel: Array.from(pixels),
    };
  });

  expect(after.backgroundCoverage).toBeGreaterThan(0.9);
  expect(after.isfCenterPixel[3]).toBe(255);

  await modVApp.evaluateWorker(async () => {
    await self.store.dispatch("scenes/stopPerformance");
    await self.store.dispatch("scenes/setRowNames", { rowNames: [] });
  });
});
