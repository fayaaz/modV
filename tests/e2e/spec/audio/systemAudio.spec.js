import { expect, test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { modVApp } from "../../pageObjectModel";

const SYSTEM_AUDIO_SOURCE_ID = "modv:system-audio";
const SYSTEM_AUDIO_SOURCE_LABEL = "System Audio (loopback)";

test.describe("system audio (loopback)", () => {
  test("the worker store exposes the system audio state", async () => {
    const systemAudio = await modVApp.evaluateWorkerState(
      "mediaStream.systemAudio",
    );

    expect(systemAudio).toBeTruthy();
    expect(systemAudio).toHaveProperty("supported");
    expect(systemAudio).toHaveProperty("active");
    expect(systemAudio).toHaveProperty("label");
    expect(systemAudio).toHaveProperty("error");
  });

  test("the renderer store lists the system audio source", async () => {
    let systemAudioSource;

    await expect
      .poll(async () => {
        const audioSources =
          await modVApp.evaluateMainState("mediaStream.audio");

        if (!Array.isArray(audioSources)) {
          return null;
        }

        systemAudioSource = audioSources.find(
          (source) => source.deviceId === SYSTEM_AUDIO_SOURCE_ID,
        );

        return systemAudioSource?.label ?? null;
      })
      .toBe(SYSTEM_AUDIO_SOURCE_LABEL);

    expect(systemAudioSource.kind).toBe("audioinput");
  });

  test("the main process writes a log file into the config folder", async () => {
    const userDataPath = await modVApp.page.evaluate(() =>
      window.remote.app.getPath("userData"),
    );

    expect(typeof userDataPath).toBe("string");

    const logsDirectory = path.join(userDataPath, "logs");

    expect(fs.existsSync(logsDirectory)).toBe(true);

    const files = fs
      .readdirSync(logsDirectory)
      .filter((name) => name.startsWith("modv-") && name.endsWith(".log"));

    expect(files.length).toBeGreaterThan(0);

    const contents = files
      .map((name) => fs.readFileSync(path.join(logsDirectory, name), "utf8"))
      .join("\n");

    expect(contents).toContain("modV starting");
    expect(contents).toContain(logsDirectory);
  });
});
