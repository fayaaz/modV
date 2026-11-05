import constants from "../constants";

const isGalleryGroup = (group) =>
  !!group && group.name === constants.GALLERY_GROUP_NAME;

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

/**
 * Collects the live Groups that belong to a Scene (stamped with `sceneId` /
 * `sceneRow` when the Scene was applied) and rebuilds the Scene's `groups`
 * array from them, keyed by row with `null` holes. Returns `null` when the
 * Scene is not applied, so a stale Scene document is never overwritten with
 * groups from other Scenes or transitions.
 */
export function buildCapturedGroups(scene, liveGroups) {
  const groups = Array.isArray(liveGroups) ? liveGroups : [];

  const captured = groups
    .filter(
      (group) =>
        !!group &&
        group.sceneId === scene.id &&
        group.transitionGhost !== true &&
        !isGalleryGroup(group) &&
        Number.isFinite(group.sceneRow),
    )
    .sort((a, b) => a.sceneRow - b.sceneRow);

  if (captured.length === 0) {
    return null;
  }

  const rows = [];
  for (const group of captured) {
    rows[group.sceneRow] = clone(group);
  }

  for (let i = 0; i < rows.length; i++) {
    if (!rows[i]) {
      rows[i] = null;
    }
  }

  for (const group of rows) {
    if (!group) {
      continue;
    }

    delete group.context;
    delete group.writeToSwap;
    delete group.readFromSwap;
  }

  return { groups: rows };
}
