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

export const isRowCellEmpty = (scene, rowIndex) => {
  const groups = getSceneGroups(scene);
  const group = groups[rowIndex];

  return !group || !hasModules(group);
};

export const getRowGroup = (scene, rowIndex) => {
  const groups = getSceneGroups(scene);

  return groups[rowIndex] || null;
};

export const getRowCount = (scenes, liveGroups = []) => {
  let rowCount = 0;

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

    if (isGalleryGroup(group)) {
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

export const getRowLabel = (scenes, rowIndex) => {
  const sceneList = Array.isArray(scenes) ? scenes : [];
  const counts = new Map();

  for (let i = 0, len = sceneList.length; i < len; i++) {
    const group = getSceneGroups(sceneList[i])[rowIndex];
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
    if (count > bestCount) {
      label = name;
      bestCount = count;
    }
  }

  return label || `Row ${rowIndex + 1}`;
};

const getCandidateScenes = (scenes, rowIndex) =>
  (Array.isArray(scenes) ? scenes : []).filter(
    (scene) => isSceneEnabled(scene) && !isRowCellEmpty(scene, rowIndex),
  );

export const pickNextRowScene = ({
  scenes,
  rowIndex,
  currentSceneId,
  order = "in-order",
  random = Math.random,
}) => {
  const candidates = getCandidateScenes(scenes, rowIndex);

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
