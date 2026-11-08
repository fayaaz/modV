import constants from "../constants";

const isGalleryGroup = (group) =>
  !!group && group.name === constants.GALLERY_GROUP_NAME;

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function stripGroup(group) {
  delete group.context;
  delete group.writeToSwap;
  delete group.readFromSwap;

  return group;
}

function fillRowHoles(rows) {
  for (let i = 0; i < rows.length; i++) {
    if (!rows[i]) {
      rows[i] = null;
    }
  }

  return rows;
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

  return {
    groups: fillRowHoles(rows).map((group) => group && stripGroup(group)),
  };
}

/**
 * Collects every live user Group (no Gallery, no transition ghosts). This is
 * used for an empty Scene column, which has no stamped Groups to match: the
 * current live layers are the snapshot being saved into it.
 */
export function buildCapturedGroupsFromLive(liveGroups) {
  const groups = Array.isArray(liveGroups) ? liveGroups : [];
  const captured = groups.filter(
    (group) =>
      !!group && group.transitionGhost !== true && !isGalleryGroup(group),
  );

  if (captured.length === 0) {
    return null;
  }

  const rows = [];
  for (let i = 0; i < captured.length; i++) {
    const group = captured[i];
    const row =
      Number.isFinite(group.sceneRow) && group.sceneRow >= 0
        ? Math.trunc(group.sceneRow)
        : i;

    rows[row] = clone(group);
  }

  return {
    groups: fillRowHoles(rows).map((group) => group && stripGroup(group)),
  };
}

/**
 * A Scene with no Groups at all is safe to fill from the live layer stack,
 * unlike a populated Scene that is not currently applied (capturing that
 * would overwrite it with another Scene's Groups).
 */
export function sceneHasGroups(scene) {
  if (Array.isArray(scene && scene.groups)) {
    return scene.groups.some((group) => !!group);
  }

  if (Array.isArray(scene && scene.layers)) {
    return scene.layers.length > 0;
  }

  return false;
}

/**
 * Picks the Groups to save into a Scene. Applied Scenes capture only their own
 * stamped Groups (`fromLive: false`). An empty Scene captures the whole live
 * layer stack instead (`fromLive: true`), so a newly added column can be
 * filled from the current setup. Returns `null` when the Scene must not be
 * overwritten.
 */
export function collectSceneGroups(scene, liveGroups) {
  const stamped = buildCapturedGroups(scene, liveGroups);

  if (stamped) {
    return { groups: stamped.groups, fromLive: false };
  }

  if (sceneHasGroups(scene)) {
    return null;
  }

  const live = buildCapturedGroupsFromLive(liveGroups);

  if (!live) {
    return null;
  }

  return { groups: live.groups, fromLive: true };
}

export function canCaptureScene(scene, liveGroups) {
  return collectSceneGroups(scene, liveGroups) !== null;
}
