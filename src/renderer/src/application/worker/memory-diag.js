/**
 * Worker heap + store-size telemetry.
 *
 * The renderer dies silently when V8 runs out of heap: the only artifacts are
 * a `<--- Near heap limit --->` line on stderr (journald) and a renderer
 * crash with exitCode 5 in the app log. This module writes a once-every-5s
 * `[mem-diag]` line to the daily log pairing process heap statistics from
 * `v8.getHeapStatistics()`/`process.memoryUsage()` with counts of everything
 * the worker retains (Groups, Modules, ghosts, inputs, links, scenes, tweens,
 * windows, aux outputs, images, videos).
 *
 * Reading the two together distinguishes the classic failure modes:
 * - `heapPct` climbs while the counts stay flat  -> something outside the
 *   store (closures, caches, detached contexts) is retaining memory. Watch
 *   `detachedCtx` / `nativeCtx`: a leaking context keeps its globals alive.
 * - `heapPct` climbs with a matching count (e.g. `regMods` after every scene
 *   transition) -> the store itself is growing without bound.
 */

import get from "lodash.get";
import { writeDiagLine } from "./render-diag";

const MB = 1024 * 1024;
const NEAR_LIMIT_PCT = 85;
const MIN_GATE_INTERVAL_MS = 1000;

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function countObjectKeys(value) {
  return isObject(value) ? Object.keys(value).length : 0;
}

function asArray(value) {
  return Array.isArray(value) ? value.filter(Boolean) : [];
}

/**
 * Count the store entities worth watching for unbounded growth. Tolerates
 * missing or malformed slices (returns zeros) so diagnostics never break the
 * render loop.
 */
export function collectCounts(state) {
  const groups = asArray(get(state, "groups.groups", []));

  let groupMods = 0;
  let ghosts = 0;

  for (let i = 0; i < groups.length; i++) {
    const group = groups[i];
    groupMods += Array.isArray(group.modules) ? group.modules.length : 0;

    if (group.transitionGhost === true) {
      ghosts += 1;
    }
  }

  const registered = get(state, "modules.registered", {});
  const registeredObjects = isObject(registered)
    ? Object.values(registered)
    : [];

  const registeredModules = registeredObjects.length;
  const galleryReg = registeredObjects.filter(
    (module) => module && module.meta && module.meta.isGallery === true,
  ).length;

  return {
    groups: groups.length,
    groupMods,
    ghosts,
    regMods: registeredModules,
    activeMods: countObjectKeys(get(state, "modules.active", {})),
    galleryReg,
    propQ: countObjectKeys(get(state, "modules.propQueue", {})),
    metaQ: countObjectKeys(get(state, "modules.metaQueue", {})),
    inputs: countObjectKeys(get(state, "inputs.inputs", {})),
    links: countObjectKeys(get(state, "inputs.inputLinks", {})),
    expr: countObjectKeys(get(state, "expressions.assignments", {})),
    tweens: countObjectKeys(get(state, "tweens.tweens", {})),
    scenes: asArray(get(state, "scenes.scenes", [])).length,
    rows: countObjectKeys(get(state, "scenes.activeRows", {})),
    prog: countObjectKeys(get(state, "scenes.rowProgress", {})),
    transitioning: get(state, "scenes.transitioning", false) ? 1 : 0,
    perf: get(state, "scenes.performance.enabled", false) ? 1 : 0,
    windows: countObjectKeys(get(state, "windows", {})),
    aux: countObjectKeys(get(state, "outputs.auxillary", {})),
    images: countObjectKeys(get(state, "images", {})),
    videos: countObjectKeys(get(state, "videos", {})),
  };
}

function toMB(bytes) {
  return Number((bytes / MB).toFixed(1));
}

/**
 * Merge `v8.getHeapStatistics()` and `process.memoryUsage()` into a stable
 * shape. Either source may be missing (returns null when both are).
 */
export function normaliseHeapStats(sources) {
  if (!sources) {
    return null;
  }

  const memoryUsage = isObject(sources.memoryUsage)
    ? sources.memoryUsage
    : null;
  const heapStatistics = isObject(sources.heapStatistics)
    ? sources.heapStatistics
    : null;

  if (!memoryUsage && !heapStatistics) {
    return null;
  }

  const heap = {};

  heap.usedHeapMB =
    heapStatistics && typeof heapStatistics.used_heap_size === "number"
      ? toMB(heapStatistics.used_heap_size)
      : memoryUsage && typeof memoryUsage.heapUsed === "number"
        ? toMB(memoryUsage.heapUsed)
        : null;

  heap.heapLimitMB =
    heapStatistics && typeof heapStatistics.heap_size_limit === "number"
      ? toMB(heapStatistics.heap_size_limit)
      : null;

  heap.totalHeapMB =
    heapStatistics && typeof heapStatistics.total_heap_size === "number"
      ? toMB(heapStatistics.total_heap_size)
      : memoryUsage && typeof memoryUsage.heapTotal === "number"
        ? toMB(memoryUsage.heapTotal)
        : null;

  heap.physicalMB =
    heapStatistics && typeof heapStatistics.total_physical_size === "number"
      ? toMB(heapStatistics.total_physical_size)
      : null;

  heap.mallocMB =
    heapStatistics && typeof heapStatistics.malloced_memory === "number"
      ? toMB(heapStatistics.malloced_memory)
      : null;

  heap.externalMB =
    heapStatistics && typeof heapStatistics.external_memory === "number"
      ? toMB(heapStatistics.external_memory)
      : memoryUsage && typeof memoryUsage.external === "number"
        ? toMB(memoryUsage.external)
        : null;

  heap.rssMB =
    memoryUsage && typeof memoryUsage.rss === "number"
      ? toMB(memoryUsage.rss)
      : null;

  heap.arrayBuffersMB =
    memoryUsage && typeof memoryUsage.arrayBuffers === "number"
      ? toMB(memoryUsage.arrayBuffers)
      : null;

  heap.nativeCtx =
    heapStatistics &&
    typeof heapStatistics.number_of_native_contexts === "number"
      ? heapStatistics.number_of_native_contexts
      : null;

  heap.detachedCtx =
    heapStatistics &&
    typeof heapStatistics.number_of_detached_contexts === "number"
      ? heapStatistics.number_of_detached_contexts
      : null;

  heap.heapPct =
    heap.usedHeapMB !== null && heap.heapLimitMB
      ? Number(((heap.usedHeapMB / heap.heapLimitMB) * 100).toFixed(1))
      : null;

  return heap;
}

/**
 * Read heap statistics from the worker's Node integration. Returns null when
 * unavailable; never throws.
 */
export function readHeapStats() {
  let memoryUsage = null;
  let heapStatistics = null;

  try {
    const proc =
      typeof process !== "undefined"
        ? process
        : typeof global !== "undefined"
          ? global.process
          : null;

    if (proc && typeof proc.memoryUsage === "function") {
      memoryUsage = proc.memoryUsage();
    }
  } catch (error) {
    memoryUsage = null;
  }

  try {
    const v8 = require("v8");

    if (v8 && typeof v8.getHeapStatistics === "function") {
      heapStatistics = v8.getHeapStatistics();
    }
  } catch (error) {
    heapStatistics = null;
  }

  try {
    return normaliseHeapStats({ memoryUsage, heapStatistics });
  } catch (error) {
    return null;
  }
}

export function isHeapNearLimit(heap, threshold = NEAR_LIMIT_PCT) {
  if (!heap || typeof heap.heapPct !== "number") {
    return false;
  }

  return heap.heapPct >= threshold;
}

function field(value, suffix = "") {
  return value === null || value === undefined
    ? "unknown"
    : `${value}${suffix}`;
}

export function formatMemDiag({ heap, counts }) {
  const countsLine = Object.keys(counts)
    .map((key) => `${key}=${counts[key]}`)
    .join(" ");

  if (!heap) {
    return `mem heap=unavailable counts ${countsLine}`;
  }

  const mb = (value) =>
    field(value == null ? null : Number(value).toFixed(1), "MB");

  const parts = [
    "mem",
    `used=${mb(heap.usedHeapMB)}`,
    `limit=${mb(heap.heapLimitMB)}`,
    `pct=${heap.heapPct === null ? "unknown" : `${heap.heapPct}%`}`,
    `total=${mb(heap.totalHeapMB)}`,
    `rss=${mb(heap.rssMB)}`,
    `malloc=${mb(heap.mallocMB)}`,
    `external=${mb(heap.externalMB)}`,
    `arrBuf=${mb(heap.arrayBuffersMB)}`,
    `phys=${mb(heap.physicalMB)}`,
    `nativeCtx=${field(heap.nativeCtx)}`,
    `detachedCtx=${field(heap.detachedCtx)}`,
    "counts",
    countsLine,
  ];

  if (isHeapNearLimit(heap)) {
    parts.push("WARN=near-heap-limit");
  }

  return parts.join(" ");
}

/**
 * Rate-limit helper. Returns a function that yields true immediately on the
 * first call and then only once per `intervalMs`.
 */
export function createMemoryGate(intervalMs = 5000) {
  const interval = Math.max(MIN_GATE_INTERVAL_MS, Number(intervalMs) || 0);
  let nextAt = -Infinity;

  return function gate(now) {
    if (now < nextAt) {
      return false;
    }

    nextAt = now + interval;
    return true;
  };
}

/**
 * Collect a memory snapshot and append it to the daily log as `[mem-diag]`.
 * Diagnostics must never break rendering, so all failures are swallowed.
 */
export function writeMemorySnapshot(state) {
  try {
    const heap = readHeapStats();

    writeDiagLine(
      formatMemDiag({ heap, counts: collectCounts(state) }),
      "mem-diag",
    );
  } catch (error) {
    // Diagnostics must never break rendering.
  }
}
