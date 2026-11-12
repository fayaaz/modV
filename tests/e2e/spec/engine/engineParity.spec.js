// WS-9 / M4 / M7 behaviour-parity gate, against the packaged app with
// MODV_ENGINE=1:
//  - the gallery is populated from the engine registry (registered modules);
//  - a Scene's bass Input Link drives the linked prop between its min and max;
//  - a deliberately transparent Point10 scene keeps an alpha-0 background;
//  - the composited output preview publishes real pixels.
//
// Launches its own Electron instance (like engineSkeleton.spec.js) so it can
// run with MODV_ENGINE=1 without touching the shared legacy page object.

import { test, expect } from "@playwright/test";
import { _electron as electron } from "playwright";
import {
  ipcRendererInvoke,
  parseElectronApp,
} from "electron-playwright-helpers";
import fs from "fs";
import os from "os";
import path from "path";
import { resolveLatestBuild } from "../../pageObjectModel/resolveLatestBuild";

async function waitForReady(page, timeout = 60_000) {
  const deadline = Date.now() + timeout;

  while (Date.now() < deadline) {
    if (page.isClosed()) return false;
    if (await ipcRendererInvoke(page, "is-modv-ready")) return true;
    await page.waitForTimeout(250);
  }

  return false;
}

test.describe("engine behaviour parity (MODV_ENGINE=1)", () => {
  test.setTimeout(120_000);

  test("gallery, audio links, transparency and previews", async () => {
    const latestBuild = resolveLatestBuild(path.resolve("./dist"));
    const appInfo = parseElectronApp(latestBuild.buildDir);
    const configDir = fs.mkdtempSync(
      path.join(os.tmpdir(), "vmodv-e2e-parity-"),
    );
    const cleanupDirs = [latestBuild.cleanupDir, configDir].filter(Boolean);

    const app = await electron.launch({
      args: ["--no-sandbox", appInfo.main],
      executablePath: appInfo.executable,
      env: {
        ...process.env,
        MODV_ENGINE: "1",
        ELECTRON_ENABLE_LOGGING: "1",
        XDG_CONFIG_HOME: configDir,
      },
    });

    try {
      const page = await app.firstWindow();
      expect(await waitForReady(page)).toBe(true);

      // 1. Gallery registry: the engine's state/sync must carry every
      // registered module so the Gallery renders tiles (it only exists in the
      // worker in legacy mode).
      const registered = await page.evaluate(
        () =>
          Object.keys(window.modV.store.state.modules.registered || {}).length,
      );
      expect(registered).toBeGreaterThan(300);

      // 2. Audio Input Link: apply a default Scene with a bass link, stop the
      // live analyser pump overwriting the synthetic frames, then drive the
      // source between 0 and 1 and read the linked prop back from the mirror.
      const linkResult = await page.evaluate(async () => {
        const store = window.modV.store;
        const scene = store.state.scenes.scenes.find(
          (item) => item.name === "Lone Chrome",
        );
        if (!scene) return { error: "scene missing" };

        await store.dispatch("scenes/applyScene", { sceneId: scene.id });
        await new Promise((resolve) => setTimeout(resolve, 800));

        window.__parityMeyda = window.modV.meyda;
        window.modV.meyda = null;

        const read = () => {
          const state = store.state;
          const link = Object.values(state.inputs.inputLinks || {}).find(
            (candidate) =>
              candidate.source === "meyda" && candidate.type === "getter",
          );
          if (!link) return { error: "no meyda link" };
          const input = state.inputs.inputs[link.id];
          const module = state.modules.active[input.data.moduleId];
          return {
            min: link.min,
            max: link.max,
            value: module.props[input.data.prop],
          };
        };

        const base = {
          sampleRate: 48000,
          wave: Array(1024).fill(0),
          freqMag: Array(1024).fill(0),
          byteFrequencyData: Array(512).fill(0),
          byteTimeDomainData: Array(512).fill(128),
        };
        const silent = {
          ...base,
          bass: 0,
          mid: 0,
          treble: 0,
          beat: 0,
          lanes: Array(12).fill(0),
          cells: Array(16).fill(0),
          rawLanes: Array(12).fill(0),
          rawCells: Array(16).fill(0),
        };
        const loud = {
          ...base,
          bass: 1,
          mid: 1,
          treble: 1,
          beat: 1,
          lanes: Array(12).fill(1),
          cells: Array(16).fill(1),
          rawLanes: Array(12).fill(1),
          rawCells: Array(16).fill(1),
        };

        store.commit("meyda/SET_BANDS", silent);
        await new Promise((resolve) => setTimeout(resolve, 700));
        const quiet = read();

        store.commit("meyda/SET_BANDS", loud);
        await new Promise((resolve) => setTimeout(resolve, 700));
        const loudRead = read();

        window.modV.meyda = window.__parityMeyda;
        return { quiet, loud: loudRead };
      });

      expect(linkResult.error).toBeUndefined();
      expect(linkResult.quiet.value).toBeCloseTo(linkResult.quiet.min, 2);
      expect(linkResult.loud.value).toBeCloseTo(linkResult.loud.max, 2);

      // 3. Transparency: an isolated preview of the deliberately
      // background-free scene has alpha-0 background pixels and opaque pixels.
      const transparency = await page.evaluate(async () => {
        const frame = await window.modV.requestModulePreview(
          "Point10 It Is Coming For You",
          192,
          108,
        );
        if (!frame) return { error: "no preview" };
        let alphaZero = 0;
        let alphaFull = 0;
        let lit = 0;
        for (let index = 0; index < frame.data.length; index += 4) {
          const alpha = frame.data[index + 3];
          if (alpha === 0) alphaZero += 1;
          if (alpha === 255) alphaFull += 1;
          if (
            frame.data[index] + frame.data[index + 1] + frame.data[index + 2] >
            8
          ) {
            lit += 1;
          }
        }
        return {
          bytes: frame.data.length,
          width: frame.width,
          height: frame.height,
          pixels: frame.data.length / 4,
          alphaZero,
          alphaFull,
          lit,
        };
      });

      expect(transparency.error).toBeUndefined();
      expect(transparency.bytes).toBe(192 * 108 * 4);
      expect(transparency.alphaZero).toBeGreaterThan(transparency.pixels * 0.1);
      expect(transparency.alphaFull).toBeGreaterThan(
        transparency.pixels * 0.05,
      );
      expect(transparency.lit).toBeGreaterThan(0);

      // 4. The composited output preview ring publishes real pixels.
      const output = await page.evaluate(async () => {
        const frame = await window.modV.requestOutputPreview();
        if (!frame) return { error: "no output preview" };
        let min = 255;
        let max = 0;
        for (let index = 0; index < frame.data.length; index += 4) {
          const value = Math.max(
            frame.data[index],
            frame.data[index + 1],
            frame.data[index + 2],
          );
          if (value < min) min = value;
          if (value > max) max = value;
        }
        return {
          bytes: frame.data.length,
          width: frame.width,
          height: frame.height,
          min,
          max,
        };
      });

      expect(output.error).toBeUndefined();
      expect(output.bytes).toBe(output.width * output.height * 4);
      expect(output.max).toBeGreaterThan(output.min);
    } finally {
      await app.close().catch(() => {});
      for (const dir of cleanupDirs) {
        fs.rmSync(dir, { recursive: true, force: true });
      }
    }
  });
});
