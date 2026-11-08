/**
 * Decides whether a Scene JSON file should be applied over the in-memory Scene
 * list.
 *
 * The worker auto-persists the Scene list to `scenes.json` after every Scene
 * mutation, so that file is the authoritative copy. The per-Scene JSON files in
 * the media folder are only written when a Scene is explicitly saved, renamed
 * or re-timed, so they are often older than the in-memory list. Reloading one
 * of those over a newer Scene reverted the user's work (the "Reload from disk
 * wiped my Scenes" bug), so an existing Scene is only replaced when its file was
 * modified after the Scene list was last saved.
 */
export function shouldLoadSceneFromDisk({
  sceneId,
  existingSceneIds,
  fileMtimeMs,
  storeMtimeMs,
} = {}) {
  if (!sceneId) {
    return false;
  }

  if (!existingSceneIds || !existingSceneIds.has(sceneId)) {
    return true;
  }

  const fileMtime =
    typeof fileMtimeMs === "number" && Number.isFinite(fileMtimeMs)
      ? fileMtimeMs
      : null;
  const storeMtime =
    typeof storeMtimeMs === "number" && Number.isFinite(storeMtimeMs)
      ? storeMtimeMs
      : null;

  if (fileMtime === null || storeMtime === null) {
    return false;
  }

  return fileMtime > storeMtime;
}
