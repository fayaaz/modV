import { expect, test } from "@playwright/test";
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
    const audioSources = await modVApp.evaluateMainState("mediaStream.audio");

    expect(Array.isArray(audioSources)).toBe(true);

    const systemAudioSource = audioSources.find(
      (source) => source.deviceId === SYSTEM_AUDIO_SOURCE_ID,
    );

    expect(systemAudioSource).toBeTruthy();
    expect(systemAudioSource.label).toBe(SYSTEM_AUDIO_SOURCE_LABEL);
    expect(systemAudioSource.kind).toBe("audioinput");
  });
});
