import constants from "../constants";
import { switchChance } from "./phrase-detector.js";

const isGalleryGroup = (group) =>
  !!group && group.name === constants.GALLERY_GROUP_NAME;

const hasModules = (group) => {
  const modules = group && group.modules;

  if (Array.isArray(modules)) {
    return modules.length > 0;
  }

  if (modules && typeof modules === "object") {
    return Object.keys(modules).length > 0;
  }

  return false;
};

export const getSceneGroups = (scene) =>
  scene && Array.isArray(scene.groups) ? scene.groups : [];

export const isSceneEnabled = (scene) => !!scene && scene.enabled !== false;

export const getRowNameIndex = (rowNames, rowIndex) => {
  const list = Array.isArray(rowNames) ? rowNames : [];
  const name = list[rowIndex];

  return typeof name === "string" ? name.trim() : "";
};

/**
 * Strips a trailing number (and any separator before it) from a name, so the
 * matching below treats "Effects", "Effects 2" and "effects2" as one family.
 */
const baseName = (value) =>
  String(value)
    .trim()
    .toLowerCase()
    .replace(/ ?\d+$/, "");

/**
 * A row matches a Group whose name is exactly the row name (case-insensitive)
 * or starts with it plus a space, so "webcam" also matches "webcam 2".
 */
export const strictMatchesRowName = (group, rowName) => {
  if (!group || typeof group.name !== "string") {
    return false;
  }

  const key = group.name.trim().toLowerCase();
  const rowKey = String(rowName).trim().toLowerCase();

  if (!rowKey) {
    return false;
  }

  return key === rowKey || key.startsWith(`${rowKey} `);
};

/**
 * Same as `strictMatchesRowName`, but trailing numbers are ignored after the
 * strict match, so "effects2" also matches the "Effects" group and a
 * "2d Scenes" row keeps its own family.
 */
export const matchesRowName = (group, rowName) => {
  if (strictMatchesRowName(group, rowName)) {
    return true;
  }

  if (!group || typeof group.name !== "string") {
    return false;
  }

  const keyBase = baseName(group.name);
  const rowBase = baseName(rowName);

  return !!keyBase && !!rowBase && keyBase === rowBase;
};

/**
 * A digit-suffixed row ("effects2") only falls back to the base-named group
 * ("Effects") when no sibling row already owns that family — otherwise two
 * rows would claim the same Group twice.
 */
export const rowFamilyIsShared = (rowIndex, rowNames) => {
  const list = Array.isArray(rowNames) ? rowNames : [];
  const rowKey = getRowNameIndex(list, rowIndex);

  if (!rowKey) {
    return false;
  }

  const family = baseName(rowKey);

  if (!family) {
    return false;
  }

  for (let i = 0, len = list.length; i < len; i++) {
    if (i === rowIndex || typeof list[i] !== "string" || !list[i].trim()) {
      continue;
    }

    if (baseName(list[i]) === family) {
      return true;
    }
  }

  return false;
};

/**
 * Named rows match their Scene Group by name (case-insensitive) instead of by
 * position. Rows without a name keep the legacy positional lookup.
 */
export const getRowGroup = (scene, rowIndex, rowNames) => {
  const groups = getSceneGroups(scene);

  const rowName = getRowNameIndex(rowNames, rowIndex);

  if (!rowName) {
    return groups[rowIndex] || null;
  }

  const rowKey = String(rowName).trim().toLowerCase();

  // Strict matches (exact name or word-prefixed name) always win, so a
  // "2"-suffixed row cannot grab its base-named Group from the base row.
  for (let i = 0, len = groups.length; i < len; i++) {
    if (strictMatchesRowName(groups[i], rowKey)) {
      return groups[i];
    }
  }

  // The loose digit-suffix fallback ("effects2" → "Effects") only applies
  // when no sibling row already owns that name family.
  if (rowFamilyIsShared(rowIndex, rowNames)) {
    return null;
  }

  for (let i = 0, len = groups.length; i < len; i++) {
    if (matchesRowName(groups[i], rowKey)) {
      return groups[i];
    }
  }

  return null;
};

export const isRowCellEmpty = (scene, rowIndex, rowNames) => {
  const group = getRowGroup(scene, rowIndex, rowNames);

  return !group || !hasModules(group);
};

export const normalizeRowNames = (value) => {
  if (!Array.isArray(value)) {
    return [];
  }

  // Index-exact: names are positions, so blanks and duplicates are kept and
  // only whitespace is cleaned up.
  return value.map((name) => (typeof name === "string" ? name.trim() : ""));
};

/**
 * Assigns each Scene Group snapshot the grid row it should live on. Named
 * rows win over the Group's position in the document, exactly like
 * `getRowGroup` does for the grid, so applying a Scene places its Groups on
 * the cells that show them. Groups with no matching named row are slotted
 * below the named rows, keeping their document order.
 *
 * Returns one row index per Group (holes stay `null`). Without any named rows
 * the historical positional mapping is returned unchanged.
 */
export const assignSceneRowIndexes = (groups, rowNames) => {
  const list = Array.isArray(groups) ? groups : [];
  const names = normalizeRowNames(rowNames);

  if (!names.some((name) => !!name)) {
    return list.map((_, index) => index);
  }

  const indexes = new Array(list.length).fill(null);
  const takenGroups = new Set();
  const takenRows = new Set();

  for (let rowIndex = 0; rowIndex < names.length; rowIndex++) {
    const name = names[rowIndex];

    if (!name) {
      continue;
    }

    let groupIndex = -1;

    for (let i = 0; i < list.length; i++) {
      if (!takenGroups.has(i) && strictMatchesRowName(list[i], name)) {
        groupIndex = i;
        break;
      }
    }

    if (groupIndex === -1 && !rowFamilyIsShared(rowIndex, names)) {
      for (let i = 0; i < list.length; i++) {
        if (!takenGroups.has(i) && matchesRowName(list[i], name)) {
          groupIndex = i;
          break;
        }
      }
    }

    if (groupIndex !== -1) {
      indexes[groupIndex] = rowIndex;
      takenGroups.add(groupIndex);
      takenRows.add(rowIndex);
    }
  }

  let nextRow = names.length;

  for (let i = 0; i < list.length; i++) {
    if (!list[i] || takenGroups.has(i)) {
      continue;
    }

    while (takenRows.has(nextRow)) {
      nextRow++;
    }

    indexes[i] = nextRow;
    takenRows.add(nextRow);
    nextRow++;
  }

  return indexes;
};

export const getRowCount = (scenes, liveGroups = [], rowNames) => {
  let rowCount = normalizeRowNames(rowNames).length;

  const sceneList = Array.isArray(scenes) ? scenes : [];

  for (let i = 0, len = sceneList.length; i < len; i++) {
    const groups = getSceneGroups(sceneList[i]).filter(
      (group) => !isGalleryGroup(group),
    );

    rowCount = Math.max(rowCount, groups.length);
  }

  const liveList = Array.isArray(liveGroups) ? liveGroups : [];
  let liveCount = 0;

  for (let i = 0, len = liveList.length; i < len; i++) {
    const group = liveList[i];

    if (isGalleryGroup(group) || group.transitionGhost === true) {
      continue;
    }

    liveCount += 1;

    const sceneRow = Number.isFinite(group && group.sceneRow)
      ? group.sceneRow + 1
      : 0;

    rowCount = Math.max(rowCount, sceneRow);
  }

  rowCount = Math.max(rowCount, liveCount);

  return Math.max(0, rowCount);
};

export const getRowLabel = (scenes, rowIndex, rowNames) => {
  const rowName = getRowNameIndex(rowNames, rowIndex);

  if (rowName) {
    return rowName;
  }

  const sceneList = Array.isArray(scenes) ? scenes : [];
  const counts = new Map();
  // Scenes without a group on this row count as unnamed too, so a name shared
  // by a handful of legacy Scenes cannot dress an unnamed row up as something
  // it is not.
  const denominator = sceneList.length || 1;

  for (let i = 0, len = sceneList.length; i < len; i++) {
    const groups = getSceneGroups(sceneList[i]);
    const group = groups[rowIndex];
    const name = group && group.name;

    if (typeof name !== "string" || !name) {
      continue;
    }

    if (!counts.has(name)) {
      counts.set(name, 0);
    }

    counts.set(name, counts.get(name) + 1);
  }

  let label = null;
  let bestCount = 0;

  for (const [name, count] of counts) {
    if (count * 2 > denominator && count > bestCount) {
      label = name;
      bestCount = count;
    }
  }

  return label || `Row ${rowIndex + 1}`;
};

const getCandidateScenes = (scenes, rowIndex, rowNames) =>
  (Array.isArray(scenes) ? scenes : []).filter(
    (scene) =>
      isSceneEnabled(scene) && !isRowCellEmpty(scene, rowIndex, rowNames),
  );

export const pickNextRowScene = ({
  scenes,
  rowIndex,
  currentSceneId,
  order = "in-order",
  random = Math.random,
  rowNames,
}) => {
  const candidates = getCandidateScenes(scenes, rowIndex, rowNames);

  if (candidates.length === 0) {
    return null;
  }

  if (candidates.length === 1) {
    return candidates[0];
  }

  if (order === "random") {
    const pool = candidates.filter((scene) => scene.id !== currentSceneId);
    const choices = pool.length > 0 ? pool : candidates;
    const index = Math.min(
      choices.length - 1,
      Math.floor(random() * choices.length),
    );

    return choices[index];
  }

  const currentIndex = candidates.findIndex(
    (scene) => scene.id === currentSceneId,
  );

  if (currentIndex === -1) {
    return candidates[0];
  }

  return candidates[(currentIndex + 1) % candidates.length];
};

export const shouldAdvanceRow = ({
  dwell,
  minSeconds,
  maxSeconds,
  phraseFlipped,
  random = Math.random,
}) => {
  const forced = dwell >= maxSeconds;
  const musical =
    !!phraseFlipped && random() < switchChance(dwell, minSeconds, maxSeconds);

  return {
    advance: forced || musical,
    musical,
    forced,
  };
};
