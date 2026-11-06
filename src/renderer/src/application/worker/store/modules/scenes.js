/* eslint-env worker */
import { reactive } from "vue";
import store from "../";
import constants, { GROUP_ENABLED } from "../../../constants";
import { v4 as uuidv4 } from "uuid";
import {
  createPhraseState,
  switchChance,
  updatePhrase,
} from "../../phrase-detector";
import {
  FADE_TRANSITION,
  NONE_TRANSITION,
  NATIVE_TRANSITION_NAMES,
} from "../../../scene-transition-defs";
import DEFAULT_SCENES, {
  DEFAULTS_VERSION,
  LEGACY_DEFAULTS_VERSION,
  getDefaultSceneNamesUpTo,
  getScenesToSeed,
} from "../default-scenes";
import {
  normalizePerformanceTiming,
  normalizeSceneTiming,
  normalizeTransitionDuration,
  resolveSceneTiming,
} from "../../scene-timing";
import {
  createPresetData,
  SCENES_PLAYLIST_TYPE,
} from "../../preset-serializer";
import { createSerialTask } from "../../serial-task";
import {
  assignSceneRowIndexes,
  getRowCount,
  getRowGroup,
  getRowLabel,
  isRowCellEmpty,
  isSceneEnabled,
  matchesRowName,
  normalizeRowNames,
  pickNextRowScene,
  rowFamilyIsShared,
  shouldAdvanceRow,
  strictMatchesRowName,
} from "../../scene-rows";
import { buildCapturedGroups } from "../../scene-capture";
import {
  DEFAULT_SCENE_BPM_TWEEN,
  isAutoLinkExcluded,
} from "../../scene-link-defaults";

const fs = require("fs");
const path = require("path");

/**
 * @typedef {Object} Scene
 *
 * A Scene is a preset document plus Scene metadata. It snapshots the whole
 * preset-aware store (Groups, Modules, Input Links, Expressions, Tweens and
 * MIDI devices) and adds performance timing/crossfade information.
 *
 * @property {String} id       UUID of the Scene
 * @property {String} name     Name of the Scene
 * @property {Array}  groups   Group snapshots, including their Module ids
 * @property {Object} modules  Preset data for each Module, keyed by Module id
 * @property {Object} inputs   Input state, including Input Links
 * @property {Object} expressions Expression assignments
 * @property {Array}  tweens   Tween definitions
 * @property {Object} midi     MIDI device state
 */

/**
 * @returns {Object}
 */
function getSceneNameKey(name) {
  return String(name || "")
    .trim()
    .toLowerCase();
}

function normalizeDefaultSceneNames(value) {
  if (!Array.isArray(value)) {
    return [];
  }

  const names = [];
  const seen = new Set();

  for (let i = 0, len = value.length; i < len; i++) {
    const name = value[i];

    if (typeof name !== "string" || !name.trim()) {
      continue;
    }

    const key = getSceneNameKey(name);

    if (!seen.has(key)) {
      seen.add(key);
      names.push(name);
    }
  }

  return names;
}

function normalizeDefaultsVersion(value) {
  const version = Number(value);

  if (!Number.isFinite(version) || version < 0) {
    return null;
  }

  return Math.floor(version);
}

const GROUP_INPUT_NAMES = [
  "alpha",
  "enabled",
  "clearing",
  "inherit",
  "compositeOperation",
  "pipeline",
];

const MODULE_META_INPUT_NAMES = ["alpha", "enabled", "compositeOperation"];

const BASS_PROP_PREFERENCE = [
  "intensity",
  "amount",
  "brightness",
  "colorize",
  "bleedAmount",
  "grainLevel",
  "level",
  "strength",
  "saturation",
  "softness",
  "blurAmount",
  "spacing",
  "line_width",
  "scale",
  "radius",
  "size",
  "strobeRate",
  "gain",
];

const BPM_PROP_PREFERENCE = [
  "rotation",
  "rotationSpeed",
  "rotateSpeed",
  "angle",
  "hue",
  "shift",
  "offset",
  "phase",
  "speed",
  "rate",
  "count",
  "sides",
  "numColors",
  "width",
];

function getDataTypeInputKeys(type) {
  const dataType = store.state.dataTypes && store.state.dataTypes[type];

  if (dataType && typeof dataType.inputs === "function") {
    return Object.keys(dataType.inputs());
  }

  return [];
}

function addModuleInputIds(module, inputIds) {
  for (let i = 0, len = MODULE_META_INPUT_NAMES.length; i < len; i++) {
    const inputId =
      module.meta && module.meta[`${MODULE_META_INPUT_NAMES[i]}InputId`];

    if (inputId) {
      inputIds.add(inputId);
    }
  }

  const props = module.$props || {};

  for (const prop of Object.values(props)) {
    if (!prop || !prop.id) {
      continue;
    }

    inputIds.add(prop.id);

    const inputKeys = getDataTypeInputKeys(prop.type);

    for (let i = 0, len = inputKeys.length; i < len; i++) {
      inputIds.add(`${prop.id}-${inputKeys[i]}`);
    }
  }
}

function collectSceneInputIds(groups, modules) {
  const inputIds = new Set();

  for (let i = 0, len = groups.length; i < len; i++) {
    const group = groups[i];

    for (let j = 0, inputLen = GROUP_INPUT_NAMES.length; j < inputLen; j++) {
      const inputId = group[`${GROUP_INPUT_NAMES[j]}InputId`];

      if (inputId) {
        inputIds.add(inputId);
      }
    }
  }

  const moduleValues = Object.values(modules || {});

  for (let i = 0, len = moduleValues.length; i < len; i++) {
    addModuleInputIds(moduleValues[i], inputIds);
  }

  return inputIds;
}

function serializeTween(tween) {
  return JSON.parse(
    JSON.stringify({
      id: tween.id,
      data: tween.data,
      duration: tween.duration,
      direction: tween.direction,
      easing: tween.easing,
      loop: tween.loop,
      useBpm: tween.useBpm,
      bpmDivision: tween.bpmDivision,
      durationAsTotalTime: tween.durationAsTotalTime,
      steps: tween.steps,
    }),
  );
}

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

const PLAYLIST_VERSION = 1;

const PLAYLIST_PERFORMANCE_KEYS = [
  "minSeconds",
  "maxSeconds",
  "transitionSeconds",
  "allowedTransitions",
  "order",
  "rowNames",
];

function getPlaylistPerformance(performance) {
  const normalized = normalizePerformanceTiming(performance);
  const data = {};

  for (let i = 0, len = PLAYLIST_PERFORMANCE_KEYS.length; i < len; i++) {
    const key = PLAYLIST_PERFORMANCE_KEYS[i];

    if (normalized[key] !== undefined) {
      data[key] = clone(normalized[key]);
    }
  }

  return data;
}

function getImportedPerformance(performance) {
  if (!performance || typeof performance !== "object") {
    return null;
  }

  const data = {};

  for (let i = 0, len = PLAYLIST_PERFORMANCE_KEYS.length; i < len; i++) {
    const key = PLAYLIST_PERFORMANCE_KEYS[i];

    if (performance[key] !== undefined) {
      data[key] = performance[key];
    }
  }

  return Object.keys(data).length ? data : null;
}

function getSceneInputLinks(scene) {
  if (scene.inputs && scene.inputs.inputLinks) {
    return scene.inputs.inputLinks;
  }

  return scene.inputLinks || {};
}

function getSceneTweenEntries(scene) {
  if (Array.isArray(scene.tweens)) {
    return scene.tweens.map((tween) => [tween.id, tween]);
  }

  if (scene.tweens && typeof scene.tweens === "object") {
    return Object.entries(scene.tweens);
  }

  return [];
}

/**
 * Builds a Scene snapshot for Scene definitions that are constructed from a
 * temporary Group/Module set (the default Scenes). The snapshot uses the same
 * shape as a preset so Scenes and presets remain interchangeable.
 */
function createScopedSceneData(groups, modules) {
  const inputIds = collectSceneInputIds(groups, modules);
  const inputs = {};
  const inputLinks = {};
  const tweens = [];

  for (const inputId of inputIds) {
    const input = store.state.inputs.inputs[inputId];

    if (input) {
      inputs[inputId] = clone(input);
    }

    const link = store.state.inputs.inputLinks[inputId];

    if (!link) {
      continue;
    }

    inputLinks[inputId] = clone(link);

    if (link.source === "tween") {
      const tween =
        store.state.tweens.tweens[link.id] ||
        store.state.tweens.tweens[inputId];

      if (tween) {
        tweens.push(serializeTween(tween));
      }
    }
  }

  const assignments = {};
  const currentAssignments = store.state.expressions.assignments || {};

  for (const assignment of Object.values(currentAssignments)) {
    if (inputIds.has(assignment.inputId)) {
      assignments[assignment.id] = clone(assignment);
    }
  }

  return {
    groups,
    modules,
    inputs: {
      focusedInput: { id: null, title: null },
      inputs,
      inputLinks,
    },
    expressions: { assignments },
    tweens,
    midi: clone(store.state.midi),
  };
}

function mapGroupInputIds(oldGroup, newGroup, inputIdMap) {
  for (let i = 0, len = GROUP_INPUT_NAMES.length; i < len; i++) {
    const key = `${GROUP_INPUT_NAMES[i]}InputId`;
    const oldInputId = oldGroup[key];
    const newInputId = newGroup[key];

    if (oldInputId && newInputId) {
      inputIdMap[oldInputId] = newInputId;
    }
  }
}

function mapModuleInputIds(oldModule, newModule, inputIdMap) {
  for (let i = 0, len = MODULE_META_INPUT_NAMES.length; i < len; i++) {
    const key = `${MODULE_META_INPUT_NAMES[i]}InputId`;
    const oldInputId = oldModule.meta && oldModule.meta[key];
    const newInputId = newModule.meta && newModule.meta[key];

    if (oldInputId && newInputId) {
      inputIdMap[oldInputId] = newInputId;
    }
  }

  const oldProps = oldModule.$props || {};
  const newProps = newModule.$props || {};
  const propNames = Object.keys(oldProps);

  for (let i = 0, len = propNames.length; i < len; i++) {
    const propName = propNames[i];
    const oldProp = oldProps[propName];
    const newProp = newProps[propName];

    if (!oldProp || !oldProp.id || !newProp || !newProp.id) {
      continue;
    }

    inputIdMap[oldProp.id] = newProp.id;

    const inputKeys = new Set([
      ...getDataTypeInputKeys(oldProp.type),
      ...getDataTypeInputKeys(newProp.type),
    ]);

    for (const key of inputKeys) {
      inputIdMap[`${oldProp.id}-${key}`] = `${newProp.id}-${key}`;
    }
  }
}

async function restoreSceneInputs(scene, inputIdMap, { scoped = false } = {}) {
  const inputLinks = getSceneInputLinks(scene);
  const tweenEntries = getSceneTweenEntries(scene);
  const tweenMap = new Map();
  const createdTweenIds = new Set();

  for (let i = 0, len = tweenEntries.length; i < len; i++) {
    const [tweenKey, tween] = tweenEntries[i];

    if (!tween) {
      continue;
    }

    const tweenId = tween.id || tweenKey;

    tweenMap.set(tweenKey, tween);
    tweenMap.set(tweenId, tween);

    const newTweenId = inputIdMap[tweenId];

    if (!newTweenId) {
      continue;
    }

    await store.dispatch("dataTypes/createType", {
      type: "tween",
      args: { ...tween, id: newTweenId },
    });

    createdTweenIds.add(newTweenId);
  }

  const inputIds = Object.keys(inputLinks);

  for (let i = 0, len = inputIds.length; i < len; i++) {
    const oldInputId = inputIds[i];
    const newInputId = inputIdMap[oldInputId];

    if (!newInputId) {
      continue;
    }

    const link = { ...inputLinks[oldInputId] };

    if (link.source === "tween") {
      const tween = tweenMap.get(oldInputId) || tweenMap.get(link.id);

      if (tween && !createdTweenIds.has(newInputId)) {
        await store.dispatch("dataTypes/createType", {
          type: "tween",
          args: { ...tween, id: newInputId },
        });

        createdTweenIds.add(newInputId);
      } else if (!store.state.tweens.tweens[newInputId]) {
        continue;
      }

      // Tween input links point at the Tween by its input ID, which changes
      // every time the Scene is applied.
      link.location = `tweens.tweens['${newInputId}'].value[0]`;
    }

    await store.dispatch("inputs/createInputLink", {
      inputId: newInputId,
      ...link,
    });
  }

  if (scoped) {
    return;
  }

  if (!scene.inputs || typeof scene.inputs !== "object") {
    return;
  }

  const focusedInput = scene.inputs.focusedInput;

  if (focusedInput && focusedInput.id) {
    const newInputId = inputIdMap[focusedInput.id];

    if (newInputId) {
      await store.dispatch("inputs/setFocusedInput", {
        id: newInputId,
        title: focusedInput.title,
      });

      return;
    }
  }

  await store.dispatch("inputs/clearFocusedInput");
}

async function restoreSceneExpressions(
  scene,
  inputIdMap,
  { merge = false } = {},
) {
  if (!scene.expressions || typeof scene.expressions !== "object") {
    return;
  }

  const source = scene.expressions.assignments || {};
  const assignments = {};

  for (const assignment of Object.values(source)) {
    const newInputId = inputIdMap[assignment.inputId];

    if (!newInputId) {
      continue;
    }

    assignments[assignment.id] = {
      ...assignment,
      inputId: newInputId,
    };
  }

  if (merge) {
    for (const assignment of Object.values(assignments)) {
      if (typeof assignment.expression !== "string") {
        continue;
      }

      const createdId = await store.dispatch("expressions/create", {
        id: assignment.id,
        inputId: assignment.inputId,
        expression: assignment.expression,
      });

      if (createdId && assignment.evaluateEveryFrame) {
        await store.dispatch("expressions/updateEvaluation", {
          id: createdId,
          evaluateEveryFrame: true,
        });
      }
    }

    return;
  }

  await store.dispatch("expressions/loadPresetData", { assignments });
  store.commit("expressions/SWAP");

  for (const assignment of Object.values(assignments)) {
    if (assignment.evaluateEveryFrame) {
      await store.dispatch("expressions/updateEvaluation", {
        id: assignment.id,
        evaluateEveryFrame: true,
      });
    }
  }
}

async function restoreSceneMidi(scene) {
  if (!scene.midi || typeof scene.midi !== "object") {
    return;
  }

  await store.dispatch("midi/loadPresetData", scene.midi);
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

function getPropRange(module, propName) {
  const prop = (module.$props && module.$props[propName]) || {};
  const min = Number(prop.min);
  const max = Number(prop.max);

  if (Number.isFinite(min) && Number.isFinite(max) && max > min) {
    return { min, max };
  }

  return { min: 0, max: 1 };
}

function getBassLinkRange(module, propName) {
  const { min, max } = getPropRange(module, propName);
  const span = max - min;
  const current = Number(module.props && module.props[propName]);
  let linkMin;
  let linkMax;

  if (Number.isFinite(current) && current > 0) {
    linkMin = clamp(current * 0.6, min, max);
    linkMax = clamp(current * 1.4, min, max);
  } else {
    linkMin = min + span * 0.05;
    linkMax = min + span * 0.35;
  }

  if (linkMax <= linkMin) {
    return { min, max };
  }

  return { min: linkMin, max: linkMax };
}

function getBpmLinkRange(module, propName) {
  const { min, max } = getPropRange(module, propName);
  const span = max - min;
  const current = Number(module.props && module.props[propName]);
  let linkMin;
  let linkMax;

  if (Number.isFinite(current)) {
    linkMin = clamp(current - span * 0.2, min, max);
    linkMax = clamp(current + span * 0.2, min, max);
  } else {
    linkMin = min;
    linkMax = max;
  }

  if (linkMax <= linkMin) {
    return { min, max };
  }

  return { min: linkMin, max: linkMax };
}

function isNumericProp(prop) {
  return !!prop && (prop.type === "int" || prop.type === "float");
}

function getModuleName(module) {
  return (
    (module && (module.$moduleName || (module.meta && module.meta.name))) || ""
  );
}

function pickLinkProp(module, preference, excludedInputIds, mode) {
  const props = module.$props || {};
  const moduleName = getModuleName(module);
  const isAvailable = (name) =>
    isNumericProp(props[name]) &&
    !excludedInputIds.has(props[name].id) &&
    !isAutoLinkExcluded(moduleName, name, mode);

  for (let i = 0, len = preference.length; i < len; i++) {
    if (isAvailable(preference[i])) {
      return preference[i];
    }
  }

  const propNames = Object.keys(props);

  for (let i = 0, len = propNames.length; i < len; i++) {
    if (isAvailable(propNames[i])) {
      return propNames[i];
    }
  }

  return null;
}

function getGroupsInPriority(groups, priority) {
  const ordered = [];

  for (let i = 0, len = priority.length; i < len; i++) {
    for (let j = 0, groupLen = groups.length; j < groupLen; j++) {
      if (groups[j].name === priority[i] && !ordered.includes(groups[j])) {
        ordered.push(groups[j]);
      }
    }
  }

  for (let i = 0, len = groups.length; i < len; i++) {
    if (!ordered.includes(groups[i])) {
      ordered.push(groups[i]);
    }
  }

  return ordered;
}

function findModuleLinkTarget(
  groups,
  moduleEntries,
  priority,
  preference,
  excludedInputIds,
  mode,
) {
  const orderedGroups = getGroupsInPriority(groups, priority);

  for (let i = 0, len = orderedGroups.length; i < len; i++) {
    const group = orderedGroups[i];

    for (let j = 0, entryLen = moduleEntries.length; j < entryLen; j++) {
      const entry = moduleEntries[j];

      if (!group.modules.includes(entry.module.$id)) {
        continue;
      }

      const propName = pickLinkProp(
        entry.module,
        preference,
        excludedInputIds,
        mode,
      );

      if (!propName) {
        continue;
      }

      return {
        inputId: entry.module.$props[propName].id,
        module: entry.module,
        propName,
      };
    }
  }

  return null;
}

async function createBassLink({
  inputId,
  module,
  propName,
  group,
  feature = "bass",
  range: rangeOverride,
}) {
  const range =
    rangeOverride ||
    (module ? getBassLinkRange(module, propName) : { min: 0.55, max: 1 });

  await store.dispatch("inputs/createInputLink", {
    inputId: inputId || group.alphaInputId,
    type: "getter",
    location: "meyda/getFeature",
    source: "meyda",
    args: [feature],
    min: range.min,
    max: range.max,
  });
}

async function createBpmTweenLink({
  inputId,
  module,
  propName,
  group,
  range: rangeOverride,
}) {
  const targetInputId = inputId || group.alphaInputId;
  const range =
    rangeOverride ||
    (module ? getBpmLinkRange(module, propName) : { min: 0.4, max: 1 });

  await store.dispatch("dataTypes/createType", {
    type: "tween",
    args: {
      id: targetInputId,
      data: [[0], [1]],
      duration: 1000,
      direction: "alternate",
      easing: DEFAULT_SCENE_BPM_TWEEN.easing,
      loop: true,
      useBpm: true,
      bpmDivision: DEFAULT_SCENE_BPM_TWEEN.bpmDivision,
      durationAsTotalTime: false,
      steps: 0,
    },
  });

  await store.dispatch("inputs/createInputLink", {
    inputId: targetInputId,
    type: "state",
    location: `tweens.tweens['${targetInputId}'].value[0]`,
    source: "tween",
    min: range.min,
    max: range.max,
  });
}

const GROUP_CONTROL_INPUTS = {
  alpha: "alphaInputId",
  enabled: "enabledInputId",
  compositeOperation: "compositeOperationInputId",
};

const MODULE_CONTROL_INPUTS = {
  alpha: "alphaInputId",
  enabled: "enabledInputId",
  compositeOperation: "compositeOperationInputId",
};

function getGroupControlInputId(group, control) {
  const key = GROUP_CONTROL_INPUTS[control];

  return key ? group[key] : null;
}

function getModuleControlInputId(module, control) {
  const metaKey = MODULE_CONTROL_INPUTS[control];

  if (metaKey) {
    return module.meta && module.meta[metaKey];
  }

  return module.$props && module.$props[control]
    ? module.$props[control].id
    : null;
}

function findModuleEntryInGroup(moduleEntries, group, moduleName) {
  for (let i = 0, len = moduleEntries.length; i < len; i++) {
    const entry = moduleEntries[i];
    const module = entry.module;

    if (!group.modules.includes(module.$id)) {
      continue;
    }

    if (
      module.$moduleName === moduleName ||
      (module.meta && module.meta.name === moduleName)
    ) {
      return entry;
    }
  }

  return null;
}

/**
 * Resolves a declarative Scene link (`group`, optional `module` and `control`)
 * to the live Input id it targets. Group-only links control the Group's alpha
 * by default, while module links control a named Module control (alpha,
 * enabled, compositeOperation) or one of its numeric props.
 */
function resolveSceneLinkTarget(groups, moduleEntries, link) {
  const group = groups.find((item) => item.name === link.group);

  if (!group) {
    return null;
  }

  const control = link.control || "alpha";

  if (!link.module) {
    const inputId = getGroupControlInputId(group, control);

    return inputId ? { inputId, group } : null;
  }

  const entry = findModuleEntryInGroup(moduleEntries, group, link.module);

  if (!entry) {
    return null;
  }

  const inputId = getModuleControlInputId(entry.module, control);

  if (!inputId) {
    return null;
  }

  return { inputId, module: entry.module, propName: control, group };
}

function getExplicitLinkRange(link, target) {
  const fallback = target.module
    ? getBassLinkRange(target.module, target.propName)
    : { min: 0.2, max: 1 };

  return {
    min: Number.isFinite(link.min) ? link.min : fallback.min,
    max: Number.isFinite(link.max) ? link.max : fallback.max,
  };
}

async function applyExplicitSceneInputLinks(
  groups,
  moduleEntries,
  definition,
  excludedInputIds,
) {
  const links = Array.isArray(definition && definition.inputLinks)
    ? definition.inputLinks
    : [];

  for (let i = 0, len = links.length; i < len; i++) {
    const link = links[i];
    const target = resolveSceneLinkTarget(groups, moduleEntries, link);

    if (!target) {
      console.warn(
        `Default Scene "${definition.name}" could not link "${link.group}${
          link.module ? `/${link.module}` : ""
        }" to ${link.feature || "bass"}`,
      );
      continue;
    }

    const range = getExplicitLinkRange(link, target);

    if (link.source === "tween") {
      await createBpmTweenLink({ ...target, range });
    } else {
      await createBassLink({
        ...target,
        feature: link.feature || "bass",
        range,
      });
    }

    excludedInputIds.add(target.inputId);
  }
}

/**
 * Gives the ready-made Scenes a musical starting point: at least one control
 * follows the bass band and one follows a BPM-synced Tween. A Scene may pin
 * specific controls (for example a Foreground filter's alpha to bass) with its
 * own `inputLinks`; those are applied first and the automatic picks skip them.
 * Remaining targets are picked from the Modules the Scene already uses,
 * falling back to a Group's alpha when a Scene has no suitable numeric control.
 */
async function applyDefaultSceneInputLinks(groups, moduleEntries, definition) {
  const excludedInputIds = new Set();

  await applyExplicitSceneInputLinks(
    groups,
    moduleEntries,
    definition || {},
    excludedInputIds,
  );

  let bassTarget = findModuleLinkTarget(
    groups,
    moduleEntries,
    ["Effects", "Foreground", "Background"],
    BASS_PROP_PREFERENCE,
    excludedInputIds,
    "bass",
  );

  if (!bassTarget) {
    const candidateGroups = ["Effects", "Foreground", "Background"]
      .map((name) => groups.find((group) => group.name === name))
      .filter(Boolean)
      .concat(groups);
    const group = candidateGroups.find(
      (item) => !excludedInputIds.has(item.alphaInputId),
    );

    if (group) {
      bassTarget = { inputId: group.alphaInputId, group };
    }
  }

  if (bassTarget) {
    await createBassLink(bassTarget);
    excludedInputIds.add(bassTarget.inputId);
  }

  let bpmTarget = findModuleLinkTarget(
    groups,
    moduleEntries,
    ["Effects", "Foreground", "Background"],
    BPM_PROP_PREFERENCE,
    excludedInputIds,
    "bpm",
  );

  if (!bpmTarget) {
    const candidateGroups = ["Foreground", "Background", "Effects"]
      .map((name) => groups.find((group) => group.name === name))
      .filter(Boolean)
      .concat(groups);
    const group = candidateGroups.find(
      (item) => !excludedInputIds.has(item.alphaInputId),
    );

    if (group) {
      bpmTarget = { inputId: group.alphaInputId, group };
    }
  }

  if (bpmTarget) {
    await createBpmTweenLink(bpmTarget);
  }
}

function getDefaultState() {
  return {
    scenes: [],
    defaultsSeeded: false,
    defaultsVersion: 0,
    seededDefaultScenes: [],
    performance: {
      enabled: false,
      currentSceneId: null,
      minSeconds: 8,
      maxSeconds: 24,
      transitionSeconds: 2,
      allowedTransitions: [],
      order: "in-order",
      rowNames: [],
    },
    transitioning: false,
    activeRows: {},
    rowProgress: {},
  };
}

const state = getDefaultState();

/**
 * Phrase detection, dwell timing and crossfade bookkeeping live outside the
 * store so the per-frame tick doesn't flood the commitQueue with mutations.
 * Only meaningful changes (scene applied, performance config) are committed.
 */
let transition = null;
let pendingCleanup = [];
const scheduleApplyScene = createSerialTask();

const activeRows = reactive({});
const rowTransitions = reactive({});
const rowRuntime = new Map();
let lastRowProgressCommit = 0;
let legacyDwell = 0;
let legacyPhrase = createPhraseState();

function getLiveRowGroups() {
  return store.state.groups.groups.filter(
    (group) => !isGalleryGroup(group) && group.transitionGhost !== true,
  );
}

function assignMissingSceneRows() {
  const groups = getLiveRowGroups();

  for (let i = 0, len = groups.length; i < len; i++) {
    if (getGroupSceneRow(groups[i]) === null) {
      store.commit("groups/UPDATE_GROUP", {
        groupId: groups[i].id,
        data: { sceneRow: i },
      });
    }
  }
}

function findLiveRowGroup(rowIndex) {
  const row = Number(rowIndex);
  const groups = getLiveRowGroups();
  const match = groups.find(
    (group) =>
      group.sceneRow !== undefined &&
      group.sceneRow !== null &&
      Number(group.sceneRow) === row,
  );

  return match || groups[row] || null;
}

function groupExists(groupId) {
  return store.state.groups.groups.some((group) => group.id === groupId);
}

/**
 * Removes a row transition and its outgoing (ghost) Group. The ghost Group
 * still owns the old Modules, so removing it is what releases them once the
 * fade has finished.
 */
async function removeRowTransition(rowIndex) {
  const row = Number(rowIndex);
  const rowTransition = rowTransitions[row];

  delete rowTransitions[row];
  store.commit("scenes/CLEAR_ROW_PROGRESS", { rowIndex: row });

  if (!rowTransition || !rowTransition.ghostGroupId) {
    return;
  }

  if (!groupExists(rowTransition.ghostGroupId)) {
    return;
  }

  try {
    await store.dispatch("groups/removeGroup", {
      groupId: rowTransition.ghostGroupId,
    });
  } catch (e) {
    console.error("Could not remove a fading Scene Group", e);
  }
}

/**
 * Finishes a row fade straight away: the incoming Group gets its target alpha
 * and the outgoing ghost Group (and its Modules) is removed.
 */
async function completeRowTransition(rowIndex) {
  const row = Number(rowIndex);
  const rowTransition = rowTransitions[row];

  if (!rowTransition) {
    return;
  }

  const group = store.state.groups.groups.find(
    (item) => item.id === rowTransition.groupId,
  );

  if (group) {
    store.commit("groups/UPDATE_GROUP", {
      groupId: group.id,
      data: { alpha: rowTransition.targetAlpha },
    });
  }

  await removeRowTransition(row);
}

async function completeAllRowTransitions() {
  const rows = Object.keys(rowTransitions);

  for (let i = 0, len = rows.length; i < len; i++) {
    await completeRowTransition(rows[i]);
  }

  store.commit("scenes/CLEAR_ALL_ROW_PROGRESS");
}

function clearRowTransition(rowIndex) {
  const row = Number(rowIndex);

  delete rowTransitions[row];
  store.commit("scenes/CLEAR_ROW_PROGRESS", { rowIndex: row });
}

function clearAllRowTransitions() {
  for (const rowIndex of Object.keys(rowTransitions)) {
    delete rowTransitions[rowIndex];
  }

  store.commit("scenes/CLEAR_ALL_ROW_PROGRESS");
}

function resetLegacyRuntime() {
  legacyDwell = 0;
  legacyPhrase = createPhraseState();
}

function hasAnyRowCandidates(scenes, rowCount, rowNames) {
  const enabledScenes = getEnabledScenes(scenes);

  for (let rowIndex = 0; rowIndex < rowCount; rowIndex++) {
    for (let i = 0, len = enabledScenes.length; i < len; i++) {
      if (!isRowCellEmpty(enabledScenes[i], rowIndex, rowNames)) {
        return true;
      }
    }
  }

  return false;
}

function resetRowRuntime(rowIndex, dwell = 0) {
  rowRuntime.set(Number(rowIndex), {
    dwell,
    phrase: createPhraseState(),
  });
}

function resetAllRowRuntime(rowCount = 0, minSeconds = 0) {
  rowRuntime.clear();

  for (let rowIndex = 0; rowIndex < rowCount; rowIndex++) {
    const initialDwell = rowCount > 0 ? (rowIndex / rowCount) * minSeconds : 0;

    resetRowRuntime(rowIndex, initialDwell);
  }
}

function getRowRuntime(rowIndex, minSeconds = 0) {
  const row = Number(rowIndex);
  let runtime = rowRuntime.get(row);

  if (!runtime) {
    const rowCount = Math.max(
      1,
      getRowCount(
        store.state.scenes,
        store.state.groups.groups,
        store.state.scenes.performance &&
          store.state.scenes.performance.rowNames,
      ),
    );
    const initialDwell = minSeconds > 0 ? (row / rowCount) * minSeconds : 0;

    runtime = {
      dwell: initialDwell,
      phrase: createPhraseState(),
    };
    rowRuntime.set(row, runtime);
  }

  return runtime;
}

function getGroupSceneRow(group) {
  if (
    !group ||
    group.sceneRow === undefined ||
    group.sceneRow === null ||
    group.sceneRow === ""
  ) {
    return null;
  }

  const sceneRow = Number(group.sceneRow);

  return Number.isFinite(sceneRow) ? sceneRow : null;
}

function orderRowGroups() {
  const groups = store.state.groups.groups;
  const galleryGroup = groups.find(isGalleryGroup) || null;
  const ordered = groups
    .filter((group) => !isGalleryGroup(group))
    .map((group, index) => ({
      group,
      index,
      sceneRow: getGroupSceneRow(group) ?? index,
    }))
    .sort(
      (a, b) =>
        a.sceneRow - b.sceneRow ||
        Number(b.group.transitionGhost === true) -
          Number(a.group.transitionGhost === true) ||
        a.index - b.index,
    )
    .map((item) => item.group);

  if (galleryGroup) {
    ordered.push(galleryGroup);
  }

  return ordered;
}

/**
 * Reassigns live Group sceneRow values so named rows keep their Groups: the
 * first unclaimed Group matching a row's name is pinned to that row, and the
 * remaining Groups keep their relative order in the unclaimed rows. This is
 * what makes renaming a row snap the grid (and the live Group order) into the
 * right position.
 */
function remapRowGroupsForNames(names) {
  const groups = getLiveRowGroups();
  const takenGroups = new Set();
  const takenRows = new Set();
  const updates = [];

  for (let i = 0, len = names.length; i < len; i++) {
    const name = names[i];

    if (!name) {
      continue;
    }

    // Match strictly first so a "2" row cannot steal its base group from the
    // base row; the loose digit-suffix fallback only applies to rows whose
    // family no sibling row claims.
    let group = groups.find(
      (item) => !takenGroups.has(item.id) && strictMatchesRowName(item, name),
    );

    if (!group && !rowFamilyIsShared(i, names)) {
      group = groups.find(
        (item) => !takenGroups.has(item.id) && matchesRowName(item, name),
      );
    }

    if (group) {
      takenGroups.add(group.id);
      takenRows.add(i);
      updates.push({ groupId: group.id, sceneRow: i });
    }
  }

  const remaining = groups
    .filter((group) => !takenGroups.has(group.id))
    .map((group, index) => ({
      group,
      index,
      sceneRow: getGroupSceneRow(group) ?? index,
    }))
    .sort((a, b) => a.sceneRow - b.sceneRow || a.index - b.index);

  let nextFree = -1;

  for (const item of remaining) {
    // Unclaimed Groups slot below the named rows so a named row always owns
    // its position at the top of the grid.
    let target = Math.max(item.sceneRow, names.length);

    while (takenRows.has(target)) {
      target++;
    }

    takenRows.add(target);
    nextFree = Math.max(nextFree, target);
    updates.push({ groupId: item.group.id, sceneRow: target });
  }

  for (const update of updates) {
    if (
      update.sceneRow !==
      getGroupSceneRow(
        store.state.groups.groups.find((group) => group.id === update.groupId),
      )
    ) {
      store.commit("groups/UPDATE_GROUP", {
        groupId: update.groupId,
        data: { sceneRow: update.sceneRow },
      });
    }
  }
}

const getters = {
  getById: (state) => (sceneId) =>
    state.scenes.find((scene) => scene.id === sceneId),

  groupBySceneId: (state, getters, rootState) => (sceneId) =>
    rootState.groups.groups.find((group) => group.sceneId === sceneId),

  // `transition` lives outside the store so the per-frame tick doesn't flood
  // the commitQueue. The transition object is reactive and these getters also
  // read `state.transitioning`, so Vuex invalidates their cached values when
  // progress advances or the transition starts/finishes.
  transitionProgress: (state) => {
    if (!state.transitioning || !transition) {
      return 1;
    }

    return Math.min(1, transition.elapsed / transition.duration);
  },

  currentTransitionName: (state) =>
    state.transitioning && transition ? transition.name : null,

  transitionGroupIds: (state) => {
    if (!state.transitioning || !transition) {
      return null;
    }

    return {
      from: transition.from.map((item) => item.groupId),
      to: transition.to.map((item) => item.groupId),
    };
  },

  rowCount: (state, getters, rootState) =>
    getRowCount(
      state.scenes,
      rootState.groups.groups,
      state.performance.rowNames,
    ),

  activeRows: (state) => state.activeRows,

  activeRow: (state) => (rowIndex) =>
    state.activeRows[Number(rowIndex)] || null,

  // The render loop reads the live transition for a smooth fade. The renderer
  // store has no access to worker module state, so it falls back to the
  // throttled `rowProgress` mirror committed by `tick`.
  rowTransition: () => (rowIndex) => rowTransitions[Number(rowIndex)] || null,

  rowTransitionProgress: (state) => (rowIndex) => {
    const row = Number(rowIndex);
    const rowTransition = rowTransitions[row];

    if (rowTransition) {
      const duration =
        Number.isFinite(rowTransition.duration) && rowTransition.duration > 0
          ? rowTransition.duration
          : normalizeTransitionDuration(undefined);

      return Math.min(1, rowTransition.elapsed / duration);
    }

    const progress = Number(state.rowProgress[row]);

    return Number.isFinite(progress) ? Math.min(1, progress) : 1;
  },

  performanceRows: (state, getters) => {
    const rows = [];

    for (let rowIndex = 0; rowIndex < getters.rowCount; rowIndex++) {
      rows.push({
        index: rowIndex,
        label: getRowLabel(state.scenes, rowIndex, state.performance.rowNames),
        active: getters.activeRows[rowIndex] || null,
        progress: getters.rowTransitionProgress(rowIndex),
      });
    }

    return rows;
  },
};

function isGalleryGroup(group) {
  return group.name === constants.GALLERY_GROUP_NAME;
}

function getEnabledScenes(scenes) {
  return scenes.filter(isSceneEnabled);
}

function normalizeTransitionList(value) {
  if (!Array.isArray(value)) {
    return [];
  }

  const names = [];

  for (let i = 0, len = value.length; i < len; i++) {
    const name = value[i];

    if (typeof name === "string" && name && !names.includes(name)) {
      names.push(name);
    }
  }

  return names;
}

function normalizePerformanceData(data) {
  const normalized = { ...(data || {}) };
  const hasAllowedTransitions = Object.prototype.hasOwnProperty.call(
    normalized,
    "allowedTransitions",
  );
  const legacyTransition = normalized.transition;

  delete normalized.transition;

  const allowedTransitions = hasAllowedTransitions
    ? normalizeTransitionList(normalized.allowedTransitions)
    : [];

  if (allowedTransitions.length) {
    normalized.allowedTransitions = allowedTransitions;
  } else if (typeof legacyTransition === "string" && legacyTransition) {
    normalized.allowedTransitions = [legacyTransition];
  } else if (hasAllowedTransitions) {
    normalized.allowedTransitions = [];
  }

  if (normalized.rowNames !== undefined) {
    normalized.rowNames = normalizeRowNames(normalized.rowNames);
  }

  return normalized;
}

function getAvailableTransitionNames() {
  const names = [FADE_TRANSITION, NONE_TRANSITION, ...NATIVE_TRANSITION_NAMES];
  const registered = store.state.modules.registered || {};

  for (const module of Object.values(registered)) {
    if (
      module.props &&
      module.props.startImage &&
      module.props.endImage &&
      module.props.progress
    ) {
      names.push(module.meta.name);
    }
  }

  return names;
}

function pickTransitionName(performance) {
  const available = new Set(getAvailableTransitionNames());
  const allowed = normalizeTransitionList(
    performance.allowedTransitions,
  ).filter((name) => available.has(name));

  if (!allowed.length) {
    return FADE_TRANSITION;
  }

  return allowed[Math.floor(Math.random() * allowed.length)];
}

function pickNextScene(scenes, currentSceneId, order) {
  const enabledScenes = getEnabledScenes(scenes);

  if (!enabledScenes.length) {
    return null;
  }

  if (enabledScenes.length === 1) {
    return enabledScenes[0];
  }

  if (order === "random") {
    const pool = enabledScenes.filter((scene) => scene.id !== currentSceneId);

    return pool[Math.floor(Math.random() * pool.length)];
  }

  const currentIndex = enabledScenes.findIndex(
    (scene) => scene.id === currentSceneId,
  );
  const nextIndex =
    currentIndex === -1 ? 0 : (currentIndex + 1) % enabledScenes.length;

  return enabledScenes[nextIndex];
}

/**
 * Scenes saved before the Groups snapshot format used a single Group and a
 * flat `layers` array. Turn those into the current shape when loading.
 */
function normalizeScene(scene) {
  if (!scene || !scene.id) {
    return null;
  }

  if (Array.isArray(scene.groups)) {
    return scene;
  }

  if (!Array.isArray(scene.layers)) {
    return null;
  }

  const modules = {};

  for (let i = 0, len = scene.layers.length; i < len; i++) {
    const layer = scene.layers[i];

    if (layer && layer.$id) {
      modules[layer.$id] = layer;
    }
  }

  return {
    id: scene.id,
    name: scene.name,
    groups: [
      {
        ...(scene.group || {}),
        name: scene.name,
        modules: Object.keys(modules),
      },
    ],
    modules,
  };
}

const SCENES_STORAGE_FILENAME = "scenes.json";

function getScenesStoragePath() {
  if (!self.__userData) {
    return null;
  }

  return path.join(self.__userData, SCENES_STORAGE_FILENAME);
}

async function readStoredScenes() {
  const filePath = getScenesStoragePath();

  if (!filePath) {
    return null;
  }

  try {
    const fileBuffer = await fs.promises.readFile(filePath);
    const stored = JSON.parse(fileBuffer.toString());

    return stored && typeof stored === "object" ? stored : null;
  } catch (e) {
    if (e.code !== "ENOENT") {
      console.error("Could not read the saved Scene list", e);
    }

    return null;
  }
}

async function writeStoredScenes(state) {
  const filePath = getScenesStoragePath();

  if (!filePath) {
    return;
  }

  const performance = { ...state.performance };
  delete performance.currentSceneId;

  const payload = {
    version: 1,
    defaultsSeeded: !!state.defaultsSeeded,
    defaultsVersion: state.defaultsVersion || 0,
    seededDefaultScenes: [...state.seededDefaultScenes],
    scenes: JSON.parse(JSON.stringify(state.scenes)),
    performance,
  };

  try {
    await fs.promises.mkdir(path.dirname(filePath), { recursive: true });
    await fs.promises.writeFile(filePath, JSON.stringify(payload));
  } catch (e) {
    console.error("Could not save the Scene list", e);
  }
}

/**
 * Builds a Scene snapshot from a declarative definition by briefly creating the
 * Groups and Modules in the live store, then tearing them down again. This
 * keeps the default Scenes independent of the internal Module/Group snapshot
 * shape: they are produced by the same code path as a user-saved Scene.
 */
async function buildSceneSnapshot(definition) {
  const createdGroupIds = [];
  const createdModules = [];

  try {
    for (let i = 0, len = definition.groups.length; i < len; i++) {
      const groupDefinition = definition.groups[i];
      const group = await store.dispatch("groups/createGroup", {
        name: groupDefinition.name,
        enabled: GROUP_ENABLED,
        inherit: groupDefinition.inherit ?? true,
        inheritFrom: -1,
        clearing: groupDefinition.clearing ?? true,
        alpha: groupDefinition.alpha ?? 1,
        compositeOperation: groupDefinition.compositeOperation || "normal",
        pipeline: groupDefinition.pipeline ?? false,
      });

      createdGroupIds.push(group.id);

      const moduleDefinitions = groupDefinition.modules || [];

      for (
        let j = 0, moduleLen = moduleDefinitions.length;
        j < moduleLen;
        j++
      ) {
        const moduleDefinition = moduleDefinitions[j];

        if (!store.state.modules.registered[moduleDefinition.name]) {
          console.warn(
            `Default Scene "${definition.name}" could not find Module "${moduleDefinition.name}"`,
          );
          continue;
        }

        const module = await store.dispatch("modules/makeActiveModule", {
          moduleName: moduleDefinition.name,
          moduleMeta: {
            enabled: true,
            ...(moduleDefinition.meta || {}),
          },
        });

        if (!module) {
          continue;
        }

        store.commit("groups/ADD_MODULE_TO_GROUP", {
          moduleId: module.$id,
          groupId: group.id,
        });

        const propNames = Object.keys(moduleDefinition.props || {});

        for (let k = 0, propLen = propNames.length; k < propLen; k++) {
          const propName = propNames[k];

          if (!module.$props[propName]) {
            continue;
          }

          await store.dispatch("modules/updateProp", {
            moduleId: module.$id,
            prop: propName,
            data: moduleDefinition.props[propName],
          });
        }

        createdModules.push({ module });
      }
    }

    const createdGroups = createdGroupIds
      .map((groupId) => store.getters["groups/getById"](groupId))
      .filter(Boolean);

    await applyDefaultSceneInputLinks(
      createdGroups,
      createdModules,
      definition,
    );

    const groupSnapshots = [];
    const moduleIds = [];

    for (let i = 0, len = createdGroupIds.length; i < len; i++) {
      const group = store.getters["groups/getById"](createdGroupIds[i]);

      if (!group) {
        continue;
      }

      const snapshot = { ...group };
      delete snapshot.context;
      delete snapshot.sceneId;
      delete snapshot.writeToSwap;
      delete snapshot.readFromSwap;
      groupSnapshots.push(JSON.parse(JSON.stringify(snapshot)));
      moduleIds.push(...group.modules);
    }

    const presetData = await store.dispatch("modules/createPresetData");
    const moduleSnapshots = {};

    for (let i = 0, len = moduleIds.length; i < len; i++) {
      const moduleId = moduleIds[i];

      if (presetData[moduleId]) {
        moduleSnapshots[moduleId] = presetData[moduleId];
      }
    }

    const sceneData = createScopedSceneData(groupSnapshots, moduleSnapshots);

    return {
      ...sceneData,
      id: uuidv4(),
      name: definition.name,
      enabled: definition.enabled !== false,
      minSeconds: definition.minSeconds,
      maxSeconds: definition.maxSeconds,
      timingOverride: false,
    };
  } finally {
    for (let i = 0, len = createdGroupIds.length; i < len; i++) {
      try {
        await store.dispatch("groups/removeGroup", {
          groupId: createdGroupIds[i],
        });
      } catch (e) {
        console.error("Could not clean up a default Scene Group", e);
      }
    }
  }
}

async function cleanupPendingGroups() {
  const groupIds = pendingCleanup.splice(0);

  for (let i = 0, len = groupIds.length; i < len; i++) {
    const groupId = groupIds[i];

    try {
      const exists = store.state.groups.groups.some(
        (group) => group.id === groupId,
      );

      if (exists) {
        await store.dispatch("groups/removeGroup", { groupId });
      }
    } catch (e) {
      // One broken Group must not abort cleanup for the rest, otherwise the
      // old Scene keeps rendering alongside the new one forever.
      console.error("Could not clean up a transitioning Scene Group", e);
    }
  }
}

async function completeTransition(commit) {
  try {
    if (transition) {
      for (let i = 0, len = transition.to.length; i < len; i++) {
        const item = transition.to[i];

        store.commit("groups/UPDATE_GROUP", {
          groupId: item.groupId,
          data: { alpha: item.alpha },
        });
      }

      transition = null;
    }

    await cleanupPendingGroups();
  } finally {
    commit("SET_TRANSITIONING", false);
  }
}

const actions = {
  /**
   * Snapshots the whole preset-aware store using the same serializer as a
   * preset, then adds Scene metadata.
   */
  async saveCurrentAsScene({ commit, state }, args = {}) {
    const name = args.name || `Scene ${state.scenes.length + 1}`;

    await completeTransition(commit);
    await completeAllRowTransitions();

    const timing = normalizePerformanceTiming(state.performance);
    const presetData = await createPresetData(store);

    const scene = {
      ...presetData,
      id: uuidv4(),
      name,
      enabled: true,
      minSeconds: timing.minSeconds,
      maxSeconds: timing.maxSeconds,
      timingOverride: false,
    };

    commit("ADD_SCENE", scene);

    return JSON.parse(JSON.stringify(scene));
  },

  /**
   * Writes the current live state of an applied Scene back into its Scene
   * document. Only the live Groups stamped with the Scene's `sceneId` are
   * captured (never transition ghosts, the Gallery group or other Scenes'
   * groups), and capture is refused while performance mode is running.
   */
  async captureLiveStateToScene(
    { commit, dispatch, state, rootState },
    { sceneId } = {},
  ) {
    if (state.performance.enabled) {
      return { error: "performance" };
    }

    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (!scene) {
      return { error: "not-found" };
    }

    await completeTransition(commit);
    await completeAllRowTransitions();

    const captured = buildCapturedGroups(scene, rootState.groups.groups);

    if (!captured) {
      return { error: "not-applied" };
    }

    const moduleIds = [];
    for (let i = 0, len = captured.groups.length; i < len; i++) {
      const group = captured.groups[i];

      if (group) {
        moduleIds.push(...group.modules);
      }
    }

    const presetData = await store.dispatch("modules/createPresetData");
    const moduleSnapshots = {};

    for (let i = 0, len = moduleIds.length; i < len; i++) {
      const moduleId = moduleIds[i];

      if (presetData[moduleId]) {
        moduleSnapshots[moduleId] = presetData[moduleId];
      }
    }

    const scoped = createScopedSceneData(captured.groups, moduleSnapshots);

    const updated = {
      ...JSON.parse(JSON.stringify(scene)),
      groups: scoped.groups,
      modules: scoped.modules,
      inputs: scoped.inputs,
      expressions: scoped.expressions,
      tweens: scoped.tweens,
      midi: scoped.midi,
    };

    commit("ADD_SCENE", updated);

    await dispatch("persistScenes");

    return { scene: updated };
  },

  /**
   * Loads a Scene document (a preset document plus Scene metadata) from JSON,
   * registers it and applies it. Scene files and presets use the same
   * serializer, so a Scene can also be opened through the preset loader.
   */
  async loadSceneFromJson(
    { commit, dispatch, state },
    { scene, apply = true, persist = true } = {},
  ) {
    const normalized = normalizeScene(scene);

    if (!normalized) {
      return null;
    }

    const imported = {
      ...normalized,
      ...normalizeSceneTiming(normalized, state.performance),
    };

    commit("ADD_SCENE", imported);

    if (apply) {
      await dispatch("applyScene", { sceneId: imported.id });
    }

    if (persist) {
      await dispatch("persistScenes");
    }

    return JSON.parse(JSON.stringify(imported));
  },

  /**
   * Exports every Scene as a portable playlist JSON document. Performance
   * settings travel with the playlist; runtime state (enabled/current scene)
   * does not.
   */
  exportPlaylist({ state }) {
    return {
      type: SCENES_PLAYLIST_TYPE,
      version: PLAYLIST_VERSION,
      scenes: clone(state.scenes),
      performance: getPlaylistPerformance(state.performance),
    };
  },

  /**
   * Imports a playlist, merging Scenes by id by default. Set `replace` to
   * replace the whole Scene list.
   */
  async importPlaylist(
    { commit, dispatch, state },
    { playlist, replace = false } = {},
  ) {
    if (!playlist || !Array.isArray(playlist.scenes)) {
      return null;
    }

    await completeTransition(commit);

    const importedPerformance = getImportedPerformance(playlist.performance);

    if (importedPerformance) {
      commit(
        "UPDATE_PERFORMANCE",
        normalizePerformanceTiming({
          ...state.performance,
          ...normalizePerformanceData(importedPerformance),
        }),
      );
    }

    const importedScenes = [];

    for (let i = 0, len = playlist.scenes.length; i < len; i++) {
      const rawScene = playlist.scenes[i];

      if (!rawScene || typeof rawScene !== "object") {
        continue;
      }

      const normalized = normalizeScene({
        ...rawScene,
        id: rawScene.id || uuidv4(),
      });

      if (!normalized) {
        continue;
      }

      importedScenes.push({
        ...normalized,
        ...normalizeSceneTiming(normalized, state.performance),
      });
    }

    if (replace) {
      commit("REPLACE_SCENES", importedScenes);
    } else {
      for (let i = 0, len = importedScenes.length; i < len; i++) {
        commit("ADD_SCENE", importedScenes[i]);
      }
    }

    if (
      state.performance.currentSceneId &&
      !state.scenes.some(
        (scene) => scene.id === state.performance.currentSceneId,
      )
    ) {
      commit("UPDATE_PERFORMANCE", { currentSceneId: null });
    }

    await dispatch("persistScenes");

    return {
      scenes: clone(state.scenes),
      performance: clone(state.performance),
    };
  },

  updateSceneName({ commit, state }, { sceneId, name }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);
    const newName = (name || "").trim();

    if (!scene || !newName) {
      return null;
    }

    const duplicate = state.scenes.some(
      (other) =>
        other.id !== sceneId &&
        other.name.toLowerCase() === newName.toLowerCase(),
    );

    if (duplicate) {
      return null;
    }

    commit("UPDATE_SCENE", { sceneId, data: { name: newName } });

    return JSON.parse(JSON.stringify(scene));
  },

  updateSceneTiming({ commit, state }, { sceneId, minSeconds, maxSeconds }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (!scene) {
      return null;
    }

    const current = resolveSceneTiming(scene, state.performance);
    const timing = normalizeSceneTiming(
      {
        minSeconds: minSeconds === undefined ? current.minSeconds : minSeconds,
        maxSeconds: maxSeconds === undefined ? current.maxSeconds : maxSeconds,
        timingOverride: true,
      },
      state.performance,
    );

    commit("UPDATE_SCENE", { sceneId, data: timing });

    return JSON.parse(JSON.stringify(scene));
  },

  resetSceneTiming({ commit, state }, { sceneId }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (!scene) {
      return null;
    }

    const timing = normalizePerformanceTiming(state.performance);

    commit("UPDATE_SCENE", {
      sceneId,
      data: {
        minSeconds: timing.minSeconds,
        maxSeconds: timing.maxSeconds,
        timingOverride: false,
      },
    });

    return JSON.parse(JSON.stringify(scene));
  },

  /**
   * Applies a Scene by restoring the whole preset-aware store. Groups and
   * Modules are rebuilt with fresh IDs so the previous Groups can be
   * crossfaded out while the new Groups fade in.
   *
   * Applications are serialised: applying a second Scene while the first is
   * still rebuilding Groups would leave the first Scene's Groups orphaned in
   * the store and rendered every frame.
   */
  applyScene(context, payload) {
    return scheduleApplyScene(() => context.dispatch("applySceneNow", payload));
  },

  applySceneRow(context, payload) {
    return scheduleApplyScene(() =>
      context.dispatch("applySceneRowNow", payload),
    );
  },

  async applySceneRowNow({ commit, state }, { sceneId, rowIndex }) {
    const scene = state.scenes.find((item) => item.id === sceneId);

    if (!scene) {
      return null;
    }

    const normalizedScene = normalizeScene(scene);

    if (!normalizedScene) {
      return null;
    }

    const row = Number(rowIndex);

    if (!Number.isInteger(row) || row < 0) {
      return null;
    }

    const rowNames = state.performance.rowNames;
    const snapshot = getRowGroup(normalizedScene, row, rowNames);

    if (!snapshot || isRowCellEmpty(normalizedScene, row, rowNames)) {
      return null;
    }

    await completeTransition(commit);
    await completeRowTransition(row);
    assignMissingSceneRows();

    const oldGroup = findLiveRowGroup(row);
    const oldModuleIds = oldGroup ? [...(oldGroup.modules || [])] : [];
    const oldInputIds = new Set();
    const oldGroupInputIds = new Set();

    for (let i = 0, len = oldModuleIds.length; i < len; i++) {
      const module = store.state.modules.active[oldModuleIds[i]];

      if (module) {
        addModuleInputIds(module, oldInputIds);
      }
    }

    if (oldGroup) {
      for (let i = 0, len = GROUP_INPUT_NAMES.length; i < len; i++) {
        const inputId = oldGroup[`${GROUP_INPUT_NAMES[i]}InputId`];

        if (inputId) {
          oldInputIds.add(inputId);
          oldGroupInputIds.add(inputId);
        }
      }
    }

    const groupData = { ...snapshot };
    delete groupData.id;
    delete groupData.modules;
    delete groupData.context;
    delete groupData.sceneId;
    delete groupData.sceneRow;
    delete groupData.alphaInputId;
    delete groupData.enabledInputId;
    delete groupData.clearingInputId;
    delete groupData.inheritInputId;
    delete groupData.compositeOperationInputId;
    delete groupData.pipelineInputId;
    delete groupData.writeToSwap;
    delete groupData.readFromSwap;

    const targetAlpha = snapshot.alpha ?? 1;
    // Keep the outgoing Group and its Modules alive while the row crossfades.
    // Capturing its canvas instead (the old behaviour) froze the outgoing
    // visuals for the whole fade.
    const fade = !!(oldGroup && oldModuleIds.length > 0 && targetAlpha > 0);
    const fromAlpha = oldGroup ? oldGroup.alpha ?? 1 : 0;
    let ghostGroup = null;
    let group;

    if (fade) {
      ghostGroup = await store.dispatch("groups/createGroup", {
        name: oldGroup.name,
        enabled: oldGroup.enabled,
        hidden: oldGroup.hidden,
        inherit: oldGroup.inherit,
        inheritFrom: oldGroup.inheritFrom,
        clearing: oldGroup.clearing,
        pipeline: oldGroup.pipeline,
        compositeOperation: oldGroup.compositeOperation,
        drawToCanvasId: oldGroup.drawToCanvasId,
        modules: oldModuleIds,
        alpha: fromAlpha,
        sceneId: oldGroup.sceneId,
        sceneRow: row,
        transitionGhost: true,
      });

      store.commit("groups/UPDATE_GROUP", {
        groupId: oldGroup.id,
        data: {
          ...groupData,
          sceneId: scene.id,
          sceneRow: row,
          alpha: 0,
        },
      });

      group =
        store.state.groups.groups.find((item) => item.id === oldGroup.id) ||
        oldGroup;
    } else if (oldGroup) {
      store.commit("groups/UPDATE_GROUP", {
        groupId: oldGroup.id,
        data: {
          ...groupData,
          sceneId: scene.id,
          sceneRow: row,
          alpha: targetAlpha,
        },
      });

      group =
        store.state.groups.groups.find((item) => item.id === oldGroup.id) ||
        oldGroup;
    } else {
      group = await store.dispatch("groups/createGroup", {
        ...groupData,
        sceneId: scene.id,
        sceneRow: row,
        alpha: targetAlpha,
      });
    }

    const inputIdMap = {};
    mapGroupInputIds(snapshot, group, inputIdMap);

    const moduleSnapshots = normalizedScene.modules || {};
    const snapshotModuleIds = snapshot.modules || [];
    const newModuleIds = [];

    for (let i = 0, len = snapshotModuleIds.length; i < len; i++) {
      const moduleSnapshot = moduleSnapshots[snapshotModuleIds[i]];

      if (!moduleSnapshot) {
        continue;
      }

      const module = await store.dispatch("modules/makeActiveModule", {
        moduleName: moduleSnapshot.$moduleName,
        existingModule: moduleSnapshot,
        generateNewIds: true,
      });

      if (!module) {
        continue;
      }

      mapModuleInputIds(moduleSnapshot, module, inputIdMap);
      newModuleIds.push(module.$id);
    }

    store.commit("groups/REPLACE_GROUP_MODULES", {
      groupId: group.id,
      modules: newModuleIds,
    });

    // The ghost Group owns the old Modules while it fades out, so they must
    // stay active until the transition finishes and removes it.
    if (!fade) {
      for (let i = 0, len = oldModuleIds.length; i < len; i++) {
        const moduleId = oldModuleIds[i];

        if (!store.state.modules.active[moduleId]) {
          continue;
        }

        try {
          await store.dispatch("modules/removeActiveModule", { moduleId });
        } catch (e) {
          console.error("Could not remove a replaced row Module", e);
        }
      }
    }

    // Expressions on the replaced Group's own inputs belong to the incoming
    // Group, so they are always cleared. Module expressions only go when the
    // old Modules are actually removed.
    const staleInputIds = fade ? oldGroupInputIds : oldInputIds;
    const oldAssignments = Object.values(
      store.state.expressions.assignments || {},
    );

    for (let i = 0, len = oldAssignments.length; i < len; i++) {
      const assignment = oldAssignments[i];

      if (staleInputIds.has(assignment.inputId)) {
        await store.dispatch("expressions/remove", { id: assignment.id });
      }
    }

    for (let i = 0, len = GROUP_INPUT_NAMES.length; i < len; i++) {
      const inputId = group[`${GROUP_INPUT_NAMES[i]}InputId`];

      if (inputId) {
        await store.dispatch("inputs/removeInputLink", { inputId });
      }
    }

    await restoreSceneInputs(normalizedScene, inputIdMap, { scoped: true });
    await restoreSceneExpressions(normalizedScene, inputIdMap, { merge: true });

    const sceneGroupIdMap = {};

    for (let i = 0, len = normalizedScene.groups.length; i < len; i++) {
      const sceneGroup = normalizedScene.groups[i];
      const liveGroup = findLiveRowGroup(i);

      if (sceneGroup && sceneGroup.id && liveGroup) {
        sceneGroupIdMap[sceneGroup.id] = liveGroup.id;
      }
    }

    const inheritFrom =
      typeof snapshot.inheritFrom === "string" &&
      sceneGroupIdMap[snapshot.inheritFrom] !== undefined
        ? sceneGroupIdMap[snapshot.inheritFrom]
        : -1;

    store.commit("groups/UPDATE_GROUP", {
      groupId: group.id,
      data: { inheritFrom },
    });

    activeRows[row] = { sceneId: scene.id, groupId: group.id };
    store.commit("scenes/SET_ACTIVE_ROW", {
      rowIndex: row,
      sceneId: scene.id,
      groupId: group.id,
    });

    // Keep the UI's "Current scene" (and the legacy whole-Scene autopilot)
    // aligned with the top row when a performance grid cell is triggered.
    if (row === 0) {
      commit("UPDATE_PERFORMANCE", { currentSceneId: scene.id });
    }

    const orderedGroups = orderRowGroups();

    await store.dispatch("groups/orderByIds", {
      ids: orderedGroups.map((item) => item.id),
    });

    if (fade && ghostGroup) {
      rowTransitions[row] = reactive({
        groupId: group.id,
        ghostGroupId: ghostGroup.id,
        fromAlpha,
        targetAlpha,
        elapsed: 0,
        duration: normalizeTransitionDuration(
          state.performance.transitionSeconds,
        ),
      });
      store.commit("scenes/SET_ROW_PROGRESS", { rowIndex: row, progress: 0 });
    } else {
      clearRowTransition(row);
    }

    resetRowRuntime(row, 0);

    return { sceneId: scene.id, groupId: group.id };
  },

  async applySceneNow({ commit, state }, { sceneId }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (!scene) {
      return;
    }

    const normalizedScene = normalizeScene(scene);

    if (!normalizedScene) {
      return;
    }

    // Finish any in-flight crossfade first so nothing is left orphaned.
    await completeTransition(commit);
    await completeAllRowTransitions();

    const oldGroups = store.state.groups.groups.filter(
      (group) => !isGalleryGroup(group),
    );
    const oldGroupModuleIds = new Set(
      oldGroups.flatMap((group) => group.modules),
    );
    const oldOrphanModuleIds = Object.values(store.state.modules.active)
      .filter(
        (module) =>
          !module.meta.isGallery && !oldGroupModuleIds.has(module.$id),
      )
      .map((module) => module.$id);
    const groupSnapshots = normalizedScene.groups || [];
    const moduleSnapshots = normalizedScene.modules || {};
    // Only Modules referenced by Groups are rebuilt. Unreferenced Modules stay
    // in the serialized store for preset parity but are not instantiated while
    // crossfading Scenes.
    const createdGroups = [];
    const groupIdMap = {};
    const inputIdMap = {};
    // Named rows own their Groups by name, not by the Group's position in the
    // Scene document, so work out the row each Group belongs on before the
    // Groups are created. Otherwise the grid lights up the wrong cells and a
    // later cell apply would replace the wrong live Group.
    const rowIndexes = assignSceneRowIndexes(
      groupSnapshots,
      state.performance.rowNames,
    );

    for (let i = 0, len = groupSnapshots.length; i < len; i++) {
      const snapshot = groupSnapshots[i];

      if (!snapshot) {
        // Row holes (from Ctrl+drag copies) contain no Group.
        continue;
      }

      const sceneRow = rowIndexes[i] ?? i;
      const oldGroupId = snapshot.id;
      const moduleIds = snapshot.modules || [];

      const groupData = { ...snapshot };
      delete groupData.id;
      delete groupData.modules;
      delete groupData.context;
      delete groupData.sceneId;
      delete groupData.sceneRow;
      delete groupData.alphaInputId;
      delete groupData.enabledInputId;
      delete groupData.clearingInputId;
      delete groupData.inheritInputId;
      delete groupData.compositeOperationInputId;
      delete groupData.pipelineInputId;
      delete groupData.writeToSwap;
      delete groupData.readFromSwap;

      const targetAlpha = snapshot.alpha ?? 1;

      const group = await store.dispatch("groups/createGroup", {
        ...groupData,
        alpha: 0,
        sceneId: scene.id,
        sceneRow,
      });

      if (oldGroupId) {
        groupIdMap[oldGroupId] = group.id;
      }

      mapGroupInputIds(snapshot, group, inputIdMap);

      createdGroups.push({ groupId: group.id, alpha: targetAlpha, sceneRow });

      for (
        let j = 0, moduleIdsLength = moduleIds.length;
        j < moduleIdsLength;
        j++
      ) {
        const moduleId = moduleIds[j];
        const moduleSnapshot = moduleSnapshots[moduleId];

        if (!moduleSnapshot) {
          continue;
        }

        const module = await store.dispatch("modules/makeActiveModule", {
          moduleName: moduleSnapshot.$moduleName,
          existingModule: moduleSnapshot,
          generateNewIds: true,
        });

        if (!module) {
          continue;
        }

        mapModuleInputIds(moduleSnapshot, module, inputIdMap);

        store.commit("groups/ADD_MODULE_TO_GROUP", {
          moduleId: module.$id,
          groupId: group.id,
        });
      }
    }

    if (normalizeRowNames(state.performance.rowNames).some((name) => !!name)) {
      await store.dispatch("groups/orderByIds", {
        ids: orderRowGroups().map((item) => item.id),
      });
    }

    for (let i = 0, len = oldOrphanModuleIds.length; i < len; i++) {
      await store.dispatch("modules/removeActiveModule", {
        moduleId: oldOrphanModuleIds[i],
      });
    }

    await restoreSceneInputs(normalizedScene, inputIdMap);
    await restoreSceneExpressions(normalizedScene, inputIdMap);
    await restoreSceneMidi(normalizedScene);

    // Remap inheritFrom now that every Group has its new id, so Groups that
    // appear later in the snapshot still resolve correctly.
    for (let i = 0, len = groupSnapshots.length; i < len; i++) {
      const snapshot = groupSnapshots[i];

      if (!snapshot) {
        continue;
      }

      if (typeof snapshot.inheritFrom !== "string") {
        continue;
      }

      const groupId = groupIdMap[snapshot.id];

      if (groupId) {
        store.commit("groups/UPDATE_GROUP", {
          groupId,
          data: { inheritFrom: groupIdMap[snapshot.inheritFrom] ?? -1 },
        });
      }
    }

    clearAllRowTransitions();
    resetLegacyRuntime();

    for (const rowIndex of Object.keys(activeRows)) {
      delete activeRows[rowIndex];
    }

    const nextActiveRows = {};

    for (let i = 0, len = createdGroups.length; i < len; i++) {
      const { groupId, sceneRow } = createdGroups[i];

      activeRows[sceneRow] = { sceneId: scene.id, groupId };
      nextActiveRows[sceneRow] = { sceneId: scene.id, groupId };
    }

    commit("SET_ACTIVE_ROWS", nextActiveRows);

    const performanceMinSeconds = normalizePerformanceTiming(
      state.performance,
    ).minSeconds;
    resetAllRowRuntime(
      getRowCount(
        state.scenes,
        store.state.groups.groups,
        state.performance.rowNames,
      ),
      performanceMinSeconds,
    );

    commit("UPDATE_PERFORMANCE", { currentSceneId: scene.id });

    const transitionName = pickTransitionName(state.performance);

    const applyTargetAlphas = () => {
      for (let i = 0, len = createdGroups.length; i < len; i++) {
        const { groupId, alpha } = createdGroups[i];

        store.commit("groups/UPDATE_GROUP", {
          groupId,
          data: { alpha },
        });
      }
    };

    if (!oldGroups.length) {
      applyTargetAlphas();
      return;
    }

    if (transitionName === "None") {
      applyTargetAlphas();

      const oldGroupIds = oldGroups.map((group) => group.id);

      for (let i = 0, len = oldGroupIds.length; i < len; i++) {
        await store.dispatch("groups/removeGroup", {
          groupId: oldGroupIds[i],
        });
      }

      return;
    }

    const duration = normalizeTransitionDuration(
      state.performance.transitionSeconds,
    );
    const shader = transitionName !== "Fade";

    // Shader transitions composite the two Scenes themselves, so the Groups
    // need their final opacity straight away.
    if (shader) {
      applyTargetAlphas();
    }

    transition = reactive({
      name: transitionName,
      shader,
      from: oldGroups.map((group) => ({
        groupId: group.id,
        alpha: group.alpha,
      })),
      to: createdGroups,
      elapsed: 0,
      duration,
    });

    pendingCleanup.push(...oldGroups.map((group) => group.id));
    commit("SET_TRANSITIONING", true);
  },

  async removeScene({ commit, state }, { sceneId }) {
    await completeTransition(commit);
    await completeAllRowTransitions();

    for (const rowIndex of Object.keys(activeRows)) {
      const row = Number(rowIndex);
      const active = activeRows[rowIndex];

      if (active && active.sceneId === sceneId) {
        delete activeRows[rowIndex];
        commit("CLEAR_ACTIVE_ROW", { rowIndex: row });
        clearRowTransition(row);
        rowRuntime.delete(row);
      }
    }

    const groups = store.state.groups.groups.filter(
      (group) => group.sceneId === sceneId,
    );

    for (let i = 0, len = groups.length; i < len; i++) {
      await store.dispatch("groups/removeGroup", {
        groupId: groups[i].id,
      });
    }

    if (state.performance.currentSceneId === sceneId) {
      commit("UPDATE_PERFORMANCE", { currentSceneId: null });
    }

    commit("REMOVE_SCENE", { sceneId });
  },

  /**
   * Restores the Scene list that was persisted to the app's user data folder
   * by a previous session.
   */
  async hydrateScenes({ commit, state }) {
    const stored = await readStoredScenes();

    if (!stored) {
      return;
    }

    const storedPerformance =
      stored.performance && typeof stored.performance === "object"
        ? stored.performance
        : stored.autopilot && typeof stored.autopilot === "object"
          ? stored.autopilot
          : null;

    const performanceForTiming = normalizePerformanceTiming({
      ...state.performance,
      ...(storedPerformance ? normalizePerformanceData(storedPerformance) : {}),
    });

    if (Array.isArray(stored.scenes)) {
      for (let i = 0, len = stored.scenes.length; i < len; i++) {
        const scene = normalizeScene(stored.scenes[i]);

        if (scene) {
          commit("ADD_SCENE", {
            ...scene,
            ...normalizeSceneTiming(scene, performanceForTiming),
          });
        }
      }
    }

    if (storedPerformance) {
      commit("UPDATE_PERFORMANCE", {
        ...performanceForTiming,
        currentSceneId: state.performance.currentSceneId,
      });
    }

    const storedDefaultsVersion = normalizeDefaultsVersion(
      stored.defaultsVersion,
    );
    const defaultsVersion =
      storedDefaultsVersion === null
        ? stored.defaultsSeeded
          ? LEGACY_DEFAULTS_VERSION
          : 0
        : storedDefaultsVersion;
    let seededDefaultScenes = normalizeDefaultSceneNames(
      stored.seededDefaultScenes,
    );

    // Files written before versioned defaults only have `defaultsSeeded`.
    // Assume every default available at that time was seeded, so a default the
    // user deleted is not resurrected by the first versioned run.
    if (!seededDefaultScenes.length && defaultsVersion > 0) {
      seededDefaultScenes = getDefaultSceneNamesUpTo(defaultsVersion);
    }

    if (defaultsVersion > 0) {
      commit("SET_DEFAULTS_VERSION", defaultsVersion);
    }

    if (seededDefaultScenes.length) {
      commit("SET_SEEDED_DEFAULT_SCENES", seededDefaultScenes);
    }

    if (
      stored.defaultsSeeded ||
      defaultsVersion > 0 ||
      seededDefaultScenes.length
    ) {
      commit("SET_DEFAULTS_SEEDED", true);
    }
  },

  /**
   * Seeds the ready-to-go music reactive Scenes on first run and appends any
   * defaults introduced since the user's last seeding pass. Seeded names are
   * remembered so deleting a default Scene does not bring it back.
   */
  async ensureDefaultScenes({ commit, dispatch, state }) {
    const currentVersion = normalizeDefaultsVersion(state.defaultsVersion) || 0;
    const seededDefaultScenes = normalizeDefaultSceneNames(
      state.seededDefaultScenes,
    );
    const seededKeys = new Set(seededDefaultScenes.map(getSceneNameKey));
    const existingNames = state.scenes.map((scene) => scene.name);
    const existingKeys = new Set(existingNames.map(getSceneNameKey));
    const scenesToSeed = getScenesToSeed({
      defaultsVersion: currentVersion,
      seededNames: seededDefaultScenes,
      existingNames,
    });
    let failed = false;

    for (let i = 0, len = scenesToSeed.length; i < len; i++) {
      const definition = scenesToSeed[i];

      try {
        const scene = await buildSceneSnapshot(definition);

        if (scene) {
          commit("ADD_SCENE", scene);

          const key = getSceneNameKey(definition.name);

          if (!seededKeys.has(key)) {
            seededKeys.add(key);
            seededDefaultScenes.push(definition.name);
          }
        }
      } catch (e) {
        failed = true;
        console.error(`Could not create default Scene "${definition.name}"`, e);
      }
    }

    // A user Scene may already use a default's name. Treat it as seeded so
    // deleting it later doesn't cause the default to reappear.
    for (let i = 0, len = DEFAULT_SCENES.length; i < len; i++) {
      const definition = DEFAULT_SCENES[i];
      const key = getSceneNameKey(definition.name);

      if (!seededKeys.has(key) && existingKeys.has(key)) {
        seededKeys.add(key);
        seededDefaultScenes.push(definition.name);
      }
    }

    commit("SET_SEEDED_DEFAULT_SCENES", seededDefaultScenes);
    commit("SET_DEFAULTS_SEEDED", true);
    commit(
      "SET_DEFAULTS_VERSION",
      failed ? currentVersion : Math.max(DEFAULTS_VERSION, currentVersion),
    );

    await dispatch("persistScenes");
  },

  async persistScenes({ state }) {
    await writeStoredScenes(state);
  },

  /**
   * Reads saved Scene JSON files from the current project's scene folder.
   * The Electron main process (media manager watcher) discovers the files;
   * this reads their contents, mirroring how presets are loaded.
   */
  async loadScenesFromDisk({ commit, state, rootState }) {
    const { media, path: mediaPath } = rootState.media;
    const project = rootState.projects.currentProject;

    if (!mediaPath || !media[project] || !media[project].scene) {
      return;
    }

    const items = Object.values(media[project].scene);

    for (let i = 0, len = items.length; i < len; i++) {
      const item = items[i];

      try {
        const fileBuffer = await fs.promises.readFile(
          path.join(mediaPath, item.path),
        );
        const scene = normalizeScene(JSON.parse(fileBuffer.toString()));

        if (scene) {
          commit("ADD_SCENE", {
            ...scene,
            ...normalizeSceneTiming(scene, state.performance),
          });
        }
      } catch (e) {
        console.error(`Could not load scene "${item.name}"`, e);
      }
    }
  },

  setPerformanceEnabled({ commit, state }, { enabled }) {
    resetLegacyRuntime();
    resetAllRowRuntime(
      getRowCount(
        state.scenes,
        store.state.groups.groups,
        state.performance.rowNames,
      ),
      normalizePerformanceTiming(state.performance).minSeconds,
    );

    commit("UPDATE_PERFORMANCE", { enabled });
  },

  async startPerformance({ commit, state, dispatch }) {
    resetLegacyRuntime();

    commit("UPDATE_PERFORMANCE", { enabled: true });

    const enabledScenes = getEnabledScenes(state.scenes);

    if (!enabledScenes.length) {
      return;
    }

    const rowCount = getRowCount(
      state.scenes,
      store.state.groups.groups,
      state.performance.rowNames,
    );
    const rowCandidates = hasAnyRowCandidates(
      state.scenes,
      rowCount,
      state.performance.rowNames,
    );

    if (!rowCandidates) {
      const currentScene = state.scenes.find(
        (scene) => scene.id === state.performance.currentSceneId,
      );

      if (!currentScene || !isSceneEnabled(currentScene)) {
        await dispatch("nextScene");
      }

      return;
    }

    const activeRowIndexes = [];

    for (let rowIndex = 0; rowIndex < rowCount; rowIndex++) {
      const active = activeRows[rowIndex];
      const scene =
        active && state.scenes.find((item) => item.id === active.sceneId);
      const group = findLiveRowGroup(rowIndex);

      if (
        active &&
        scene &&
        isSceneEnabled(scene) &&
        !isRowCellEmpty(scene, rowIndex, state.performance.rowNames) &&
        group &&
        group.id === active.groupId
      ) {
        activeRowIndexes.push(rowIndex);
      }
    }

    if (!activeRowIndexes.length) {
      await dispatch("nextScene");
    } else {
      for (let rowIndex = 0; rowIndex < rowCount; rowIndex++) {
        if (activeRowIndexes.includes(rowIndex)) {
          continue;
        }

        const active = activeRows[rowIndex];
        const scene = pickNextRowScene({
          scenes: state.scenes,
          rowIndex,
          currentSceneId: active ? active.sceneId : null,
          order: state.performance.order,
          rowNames: state.performance.rowNames,
        });

        if (scene) {
          await dispatch("applySceneRow", {
            sceneId: scene.id,
            rowIndex,
          });
        }
      }
    }

    resetAllRowRuntime(
      rowCount,
      normalizePerformanceTiming(state.performance).minSeconds,
    );
  },

  async stopPerformance({ commit }) {
    resetLegacyRuntime();
    await completeTransition(commit);
    await completeAllRowTransitions();
    commit("UPDATE_PERFORMANCE", { enabled: false });
  },

  async nextScene({ state, dispatch }) {
    const scene = pickNextScene(
      state.scenes,
      state.performance.currentSceneId,
      state.performance.order,
    );

    if (!scene) {
      return;
    }

    await dispatch("applyScene", { sceneId: scene.id });
  },

  async nextSceneForRow({ state, dispatch }, { rowIndex }) {
    const row = Number(rowIndex);

    if (!Number.isInteger(row) || row < 0) {
      return;
    }

    const active = activeRows[row];
    const scene = pickNextRowScene({
      scenes: state.scenes,
      rowIndex: row,
      currentSceneId: active ? active.sceneId : null,
      order: state.performance.order,
      rowNames: state.performance.rowNames,
    });

    if (!scene) {
      return;
    }

    await dispatch("applySceneRow", { sceneId: scene.id, rowIndex: row });
  },

  setSceneEnabled({ commit, state }, { sceneId, enabled }) {
    const scene = state.scenes.find((item) => item.id === sceneId);

    if (!scene) {
      return null;
    }

    commit("UPDATE_SCENE", {
      sceneId,
      data: { enabled: !!enabled },
    });

    return JSON.parse(JSON.stringify(scene));
  },

  updatePerformance({ commit, state }, data) {
    commit(
      "UPDATE_PERFORMANCE",
      normalizePerformanceTiming({
        ...state.performance,
        ...normalizePerformanceData(data),
      }),
    );
  },

  /**
   * Sets the named performance rows (an array of strings, null/"" meaning an
   * unnamed legacy positional row). Naming/reordering a row also reorders the
   * live Groups so the grid and the layer stack snap into place.
   */
  async setRowNames({ commit, state, dispatch }, { rowNames }) {
    const normalized = normalizeRowNames(rowNames);

    commit("UPDATE_PERFORMANCE", { rowNames: normalized });
    remapRowGroupsForNames(normalized);

    await store.dispatch("groups/orderByIds", {
      ids: orderRowGroups().map((item) => item.id),
    });

    resetAllRowRuntime(
      getRowCount(state.scenes, store.state.groups.groups, normalized),
      normalizePerformanceTiming(state.performance).minSeconds,
    );

    await dispatch("persistScenes");

    return normalized;
  },

  async copySceneCellToScene(
    { commit, state },
    { fromSceneId, toSceneId, fromRowIndex, toRowIndex } = {},
  ) {
    const fromScene = state.scenes.find((scene) => scene.id === fromSceneId);
    const toScene = state.scenes.find((scene) => scene.id === toSceneId);

    if (
      !fromScene ||
      !toScene ||
      fromScene.id === toScene.id ||
      !Number.isInteger(fromRowIndex) ||
      !Number.isInteger(toRowIndex) ||
      fromRowIndex < 0 ||
      toRowIndex < 0
    ) {
      return null;
    }

    const rowNames = state.performance.rowNames;
    const sourceScene = normalizeScene(fromScene);

    if (!sourceScene) {
      return null;
    }

    const sourceGroup = getRowGroup(sourceScene, fromRowIndex, rowNames);

    if (!sourceGroup || isRowCellEmpty(sourceScene, fromRowIndex, rowNames)) {
      return null;
    }

    const moduleSnapshots = sourceScene.modules || {};

    const copiedModuleIds = (sourceGroup.modules || []).filter(
      (moduleId) => !!moduleSnapshots[moduleId],
    );

    // Saved input ids the copy carries with it, so inputs, input links,
    // expression assignments and tweens ride along exactly.
    const copiedInputIds = new Set();

    for (let i = 0, len = GROUP_INPUT_NAMES.length; i < len; i++) {
      const inputId = sourceGroup[`${GROUP_INPUT_NAMES[i]}InputId`];

      if (inputId) {
        copiedInputIds.add(inputId);
      }
    }

    for (const moduleId of copiedModuleIds) {
      addModuleInputIds(moduleSnapshots[moduleId], copiedInputIds);
    }

    const data = {};

    data.modules = { ...(toScene.modules || {}) };

    for (const moduleId of copiedModuleIds) {
      data.modules[moduleId] = clone(moduleSnapshots[moduleId]);
    }

    // Row holes (null entries) are how copied rows leave space in scenes that
    // have fewer groups; trailing holes are meaningless, so trim them.
    const groups = Array.isArray(toScene.groups) ? clone(toScene.groups) : [];

    while (groups.length <= toRowIndex) {
      groups.push(null);
    }

    groups[toRowIndex] = clone(sourceGroup);

    while (groups.length && groups[groups.length - 1] === null) {
      groups.pop();
    }

    data.groups = groups;

    const sourceInputs = sourceScene.inputs || {};
    const sourceInputLinks = getSceneInputLinks(sourceScene);
    const toInputs =
      toScene.inputs && typeof toScene.inputs === "object"
        ? clone(toScene.inputs)
        : {};
    const inputs = {
      ...(sourceInputs.inputs || {}),
      ...(toInputs.inputs || {}),
    };
    const inputLinks = {
      ...(toInputs.inputLinks || {}),
      ...getSceneInputLinks(toScene),
    };

    for (const inputId of copiedInputIds) {
      const input = sourceInputs.inputs && sourceInputs.inputs[inputId];

      if (input) {
        inputs[inputId] = clone(input);
      }

      const link = sourceInputLinks[inputId];

      if (link) {
        inputLinks[inputId] = clone(link);
      }
    }

    data.inputs = {
      focusedInput: toInputs.focusedInput || { id: null, title: null },
      inputs,
      inputLinks,
    };

    // Tweens (id->tween) merged back into whatever shape the target uses.
    const tweenMap = new Map();
    const toTween = toScene.tweens || {};

    if (Array.isArray(toTween)) {
      for (const tween of toTween) {
        if (tween && tween.id) {
          tweenMap.set(tween.id, tween);
        }
      }
    } else if (typeof toTween === "object") {
      for (const [id, tween] of getSceneTweenEntries({ tweens: toTween })) {
        tweenMap.set(id, tween);
      }
    }

    for (const inputId of Object.keys(inputLinks)) {
      if (!copiedInputIds.has(inputId)) {
        continue;
      }

      const link = inputLinks[inputId] || {};

      if (link.source === "tween") {
        const sourceTweenMap = new Map(getSceneTweenEntries(sourceScene));
        const tween =
          sourceTweenMap.get(inputId) || sourceTweenMap.get(link.id);

        if (tween && tween.id) {
          tweenMap.set(tween.id, clone(tween));
        } else {
          delete inputLinks[inputId];
        }
      }
    }

    if (Array.isArray(toTween)) {
      data.tweens = Array.from(tweenMap.values());
    } else if (typeof toTween === "object") {
      data.tweens = Object.fromEntries(tweenMap.entries());
    }

    const toAssignments =
      toScene.expressions && typeof toScene.expressions === "object"
        ? { ...(toScene.expressions.assignments || {}) }
        : {};

    const sourceAssignments =
      (sourceScene.expressions && sourceScene.expressions.assignments) || {};

    for (const assignment of Object.values(sourceAssignments)) {
      if (assignment && copiedInputIds.has(assignment.inputId)) {
        toAssignments[assignment.id] = clone(assignment);
      }
    }

    data.expressions = { assignments: toAssignments };

    commit("UPDATE_SCENE", { sceneId: toScene.id, data });

    return JSON.parse(JSON.stringify(toScene));
  },

  /**
   * Called by the render loop when a shader transition cannot be created at
   * runtime. The Scene change has already happened, so keep the same groups
   * and complete the change with the group-alpha crossfade instead of leaving
   * the output blank.
   */
  fallbackToFade() {
    if (!transition || !transition.shader) {
      return false;
    }

    transition.name = FADE_TRANSITION;
    transition.shader = false;

    return true;
  },

  /**
   * Runs once per frame from the render loop: advances any active crossfade
   * and, when performance mode is on, feeds the phrase detector with the bass
   * band and switches scenes on phrase changes (or when dwell passes
   * maxSeconds).
   *
   * With no audio the bass band sits at 0, so the detector can never flip;
   * the dwell timer alone cycles scenes at maxSeconds.
   */
  async tick({ commit, state, dispatch }, { delta }) {
    const numericDelta = Number(delta);
    const rawDt = Number.isFinite(numericDelta)
      ? Math.max(0, numericDelta / 1000)
      : 0;
    // Phrase detection smooths over long frames, but a transition must advance
    // in real time. Clamping the transition delta made a 2s transition last
    // many seconds on a slow frame rate, which kept the two Scene render load
    // on screen for the whole stall.
    const dt = Math.min(0.1, rawDt);

    const rowTransitionIndexes = Object.keys(rowTransitions);

    for (let i = 0, len = rowTransitionIndexes.length; i < len; i++) {
      const rowIndex = rowTransitionIndexes[i];
      const rowTransition = rowTransitions[rowIndex];

      if (!rowTransition) {
        continue;
      }

      rowTransition.elapsed += rawDt;

      const duration =
        Number.isFinite(rowTransition.duration) && rowTransition.duration > 0
          ? rowTransition.duration
          : normalizeTransitionDuration(undefined);
      const progress = Math.min(1, rowTransition.elapsed / duration);
      const eased = progress * progress * (3 - 2 * progress);
      const targetAlpha = Number.isFinite(rowTransition.targetAlpha)
        ? rowTransition.targetAlpha
        : 1;
      const fromAlpha = Number.isFinite(rowTransition.fromAlpha)
        ? rowTransition.fromAlpha
        : 1;

      // Fade the incoming Group in and the still-rendering outgoing Group out
      // together, so the outgoing visuals keep moving through the fade.
      if (
        store.state.groups.groups.some((g) => g.id === rowTransition.groupId)
      ) {
        store.commit("groups/UPDATE_GROUP", {
          groupId: rowTransition.groupId,
          data: { alpha: targetAlpha * eased },
        });
      }

      if (
        rowTransition.ghostGroupId &&
        store.state.groups.groups.some(
          (g) => g.id === rowTransition.ghostGroupId,
        )
      ) {
        store.commit("groups/UPDATE_GROUP", {
          groupId: rowTransition.ghostGroupId,
          data: { alpha: fromAlpha * (1 - eased) },
        });
      }

      if (progress >= 1) {
        await completeRowTransition(rowIndex);
      }
    }

    if (rowTransitionIndexes.length) {
      const now = Date.now();

      if (now - lastRowProgressCommit >= 100) {
        for (const rowIndex of Object.keys(rowTransitions)) {
          const rowTransition = rowTransitions[rowIndex];

          if (!rowTransition) {
            continue;
          }

          const duration =
            Number.isFinite(rowTransition.duration) &&
            rowTransition.duration > 0
              ? rowTransition.duration
              : normalizeTransitionDuration(undefined);

          commit("SET_ROW_PROGRESS", {
            rowIndex: Number(rowIndex),
            progress: Math.min(1, rowTransition.elapsed / duration),
          });
        }

        lastRowProgressCommit = now;
      }
    }

    if (transition) {
      transition.elapsed += rawDt;

      const duration =
        Number.isFinite(transition.duration) && transition.duration > 0
          ? transition.duration
          : normalizeTransitionDuration(undefined);
      const progress = Math.min(1, transition.elapsed / duration);

      if (!transition.shader) {
        const eased = progress * progress * (3 - 2 * progress);

        for (let i = 0, len = transition.to.length; i < len; i++) {
          const item = transition.to[i];

          store.commit("groups/UPDATE_GROUP", {
            groupId: item.groupId,
            data: { alpha: item.alpha * eased },
          });
        }

        for (let i = 0, len = transition.from.length; i < len; i++) {
          const item = transition.from[i];

          store.commit("groups/UPDATE_GROUP", {
            groupId: item.groupId,
            data: { alpha: item.alpha * (1 - eased) },
          });
        }
      }

      if (progress >= 1) {
        for (let i = 0, len = transition.to.length; i < len; i++) {
          const item = transition.to[i];

          store.commit("groups/UPDATE_GROUP", {
            groupId: item.groupId,
            data: { alpha: item.alpha },
          });
        }

        transition = null;

        try {
          await cleanupPendingGroups();
        } finally {
          commit("SET_TRANSITIONING", false);
        }
      }

      // Dwell only starts once the crossfade has finished, so time spent
      // transitioning never counts towards the next Scene's timing window.
      return;
    }

    const performance = state.performance;

    if (!performance.enabled) {
      return;
    }

    const rowCount = getRowCount(
      state.scenes,
      store.state.groups.groups,
      performance.rowNames,
    );

    if (!rowCount) {
      return;
    }

    const { bands } = store.state.meyda;
    const bass = bands ? bands.bass : 0;
    const rowCandidates = hasAnyRowCandidates(
      state.scenes,
      rowCount,
      performance.rowNames,
    );

    // Legacy Scenes can contain Groups with no Modules. There is nothing to
    // swap per row, so keep the original whole-Scene autopilot for them.
    if (!rowCandidates) {
      const currentScene = state.scenes.find(
        (scene) => scene.id === performance.currentSceneId,
      );
      const enabledScenes = getEnabledScenes(state.scenes);

      if (!currentScene || !isSceneEnabled(currentScene)) {
        await dispatch("nextScene");
        return;
      }

      if (enabledScenes.length < 2) {
        return;
      }

      const phraseUpdate = updatePhrase(legacyPhrase, bass, dt);
      legacyPhrase = phraseUpdate.state;

      const { minSeconds, maxSeconds } = resolveSceneTiming(
        currentScene,
        performance,
      );

      legacyDwell += dt;

      const chance = switchChance(legacyDwell, minSeconds, maxSeconds);
      const musical = phraseUpdate.flipped && Math.random() < chance;
      const forced = legacyDwell >= maxSeconds;

      if (musical || forced) {
        legacyDwell = 0;
        legacyPhrase = { ...legacyPhrase, t: 0 };
        await dispatch("nextScene");
      }

      return;
    }

    for (let rowIndex = 0; rowIndex < rowCount; rowIndex++) {
      if (rowTransitions[rowIndex]) {
        continue;
      }

      const active = activeRows[rowIndex];
      const currentScene =
        active && state.scenes.find((item) => item.id === active.sceneId);
      const currentGroup = findLiveRowGroup(rowIndex);

      if (
        !active ||
        !currentScene ||
        !isSceneEnabled(currentScene) ||
        isRowCellEmpty(currentScene, rowIndex, performance.rowNames) ||
        !currentGroup ||
        currentGroup.id !== active.groupId
      ) {
        const nextScene = pickNextRowScene({
          scenes: state.scenes,
          rowIndex,
          currentSceneId: active ? active.sceneId : null,
          order: performance.order,
          rowNames: performance.rowNames,
        });

        if (nextScene) {
          await dispatch("applySceneRow", {
            sceneId: nextScene.id,
            rowIndex,
          });
        }

        continue;
      }

      const candidateCount = getEnabledScenes(state.scenes).reduce(
        (count, scene) =>
          count +
          (isRowCellEmpty(scene, rowIndex, performance.rowNames) ? 0 : 1),
        0,
      );

      if (candidateCount < 2) {
        continue;
      }

      const runtime = getRowRuntime(rowIndex);
      const phraseUpdate = updatePhrase(runtime.phrase, bass, dt);
      runtime.phrase = phraseUpdate.state;

      const { minSeconds, maxSeconds } = resolveSceneTiming(
        currentScene,
        performance,
      );

      runtime.dwell += dt;

      const { advance } = shouldAdvanceRow({
        dwell: runtime.dwell,
        minSeconds,
        maxSeconds,
        phraseFlipped: phraseUpdate.flipped,
      });

      if (!advance) {
        continue;
      }

      runtime.dwell = 0;
      runtime.phrase = { ...runtime.phrase, t: 0 };

      const nextScene = pickNextRowScene({
        scenes: state.scenes,
        rowIndex,
        currentSceneId: currentScene.id,
        order: performance.order,
        rowNames: performance.rowNames,
      });

      if (nextScene) {
        await dispatch("applySceneRow", {
          sceneId: nextScene.id,
          rowIndex,
        });
      }
    }
  },
};

const mutations = {
  ADD_SCENE(state, scene) {
    const index = state.scenes.findIndex((item) => item.id === scene.id);

    if (index > -1) {
      state.scenes.splice(index, 1, scene);
      return;
    }

    state.scenes.push(scene);
  },

  REPLACE_SCENES(state, scenes) {
    state.scenes.splice(0, state.scenes.length, ...scenes);
  },

  REMOVE_SCENE(state, { sceneId }) {
    const index = state.scenes.findIndex((scene) => scene.id === sceneId);

    if (index > -1) {
      state.scenes.splice(index, 1);
    }
  },

  UPDATE_SCENE(state, { sceneId, data }) {
    const scene = state.scenes.find((scene) => scene.id === sceneId);

    if (scene) {
      Object.assign(scene, data);
    }
  },

  UPDATE_PERFORMANCE(state, data) {
    Object.assign(state.performance, data);
  },

  SET_TRANSITIONING(state, transitioning) {
    state.transitioning = transitioning;
  },

  SET_ACTIVE_ROWS(state, activeRows) {
    state.activeRows = { ...activeRows };
  },

  SET_ACTIVE_ROW(state, { rowIndex, sceneId, groupId }) {
    state.activeRows[rowIndex] = { sceneId, groupId };
  },

  CLEAR_ACTIVE_ROW(state, { rowIndex }) {
    delete state.activeRows[rowIndex];
  },

  SET_ROW_PROGRESS(state, { rowIndex, progress }) {
    state.rowProgress[rowIndex] = progress;
  },

  CLEAR_ROW_PROGRESS(state, { rowIndex }) {
    delete state.rowProgress[rowIndex];
  },

  CLEAR_ALL_ROW_PROGRESS(state) {
    state.rowProgress = {};
  },

  SET_DEFAULTS_SEEDED(state, defaultsSeeded) {
    state.defaultsSeeded = defaultsSeeded;
  },

  SET_DEFAULTS_VERSION(state, defaultsVersion) {
    state.defaultsVersion = normalizeDefaultsVersion(defaultsVersion) || 0;
  },

  SET_SEEDED_DEFAULT_SCENES(state, names) {
    state.seededDefaultScenes = normalizeDefaultSceneNames(names);
  },

  RESET_STATE(state) {
    Object.assign(state, getDefaultState());

    for (const rowIndex of Object.keys(activeRows)) {
      delete activeRows[rowIndex];
    }

    for (const rowIndex of Object.keys(rowTransitions)) {
      delete rowTransitions[rowIndex];
    }

    rowRuntime.clear();
  },
};

export default {
  namespaced: true,
  state,
  getters,
  actions,
  mutations,
};
