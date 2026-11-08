import { describe, expect, it } from "vitest";
import { shouldLoadSceneFromDisk } from "../../src/renderer/src/application/worker/scene-disk.js";

const existing = (...ids) => new Set(ids);

describe("shouldLoadSceneFromDisk", () => {
  it("imports a Scene file that is not in the store yet", () => {
    expect(
      shouldLoadSceneFromDisk({
        sceneId: "new-scene",
        existingSceneIds: existing("known-scene"),
        fileMtimeMs: 1000,
        storeMtimeMs: 2000,
      }),
    ).toBe(true);
  });

  it("does not overwrite an existing Scene with an older file", () => {
    expect(
      shouldLoadSceneFromDisk({
        sceneId: "known-scene",
        existingSceneIds: existing("known-scene"),
        fileMtimeMs: 1000,
        storeMtimeMs: 2000,
      }),
    ).toBe(false);
  });

  it("does not overwrite an existing Scene when the file is not newer", () => {
    expect(
      shouldLoadSceneFromDisk({
        sceneId: "known-scene",
        existingSceneIds: existing("known-scene"),
        fileMtimeMs: 2000,
        storeMtimeMs: 2000,
      }),
    ).toBe(false);
  });

  it("reloads an existing Scene when its file changed after the list was saved", () => {
    expect(
      shouldLoadSceneFromDisk({
        sceneId: "known-scene",
        existingSceneIds: existing("known-scene"),
        fileMtimeMs: 3000,
        storeMtimeMs: 2000,
      }),
    ).toBe(true);
  });

  it("does not overwrite an existing Scene without a saved list timestamp", () => {
    expect(
      shouldLoadSceneFromDisk({
        sceneId: "known-scene",
        existingSceneIds: existing("known-scene"),
        fileMtimeMs: 3000,
        storeMtimeMs: null,
      }),
    ).toBe(false);
  });

  it("ignores files without a Scene id", () => {
    expect(
      shouldLoadSceneFromDisk({
        sceneId: null,
        existingSceneIds: existing(),
        fileMtimeMs: 3000,
        storeMtimeMs: 2000,
      }),
    ).toBe(false);
  });
});
