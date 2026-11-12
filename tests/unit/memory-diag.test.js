import { describe, expect, it } from "vitest";
import {
  collectCounts,
  createMemoryGate,
  formatMemDiag,
  isHeapNearLimit,
  normaliseHeapStats,
  readHeapStats,
} from "../../src/renderer/src/application/worker/memory-diag";

const MB = 1024 * 1024;

function makeState() {
  return {
    groups: {
      groups: [
        { id: "g1", modules: ["m1", "m2"], transitionGhost: false },
        null,
        { id: "g2", modules: ["m3"], transitionGhost: true },
        { id: "g3", modules: [], transitionGhost: false },
      ],
    },
    modules: {
      registered: {
        r1: { $id: "r1", meta: { isGallery: false } },
        r2: { $id: "r2", meta: { isGallery: true } },
        r3: { $id: "r3", meta: { isGallery: true } },
      },
      active: { a1: {}, a2: {} },
      propQueue: { p1: {} },
      metaQueue: {},
    },
    inputs: {
      inputs: { i1: {}, i2: {} },
      inputLinks: { i1: {} },
    },
    expressions: { assignments: { e1: {}, e2: {} } },
    tweens: { tweens: { t1: {}, t2: {}, t3: {} } },
    scenes: {
      scenes: [{ id: "s1" }, { id: "s2" }],
      transitioning: true,
      activeRows: { 0: {}, 1: {} },
      rowProgress: { 0: 0.4 },
      performance: { enabled: true },
    },
    windows: { w1: {}, w2: {} },
    outputs: { auxillary: { o1: {} } },
    images: { img1: {}, img2: {} },
    videos: { vid1: {} },
  };
}

describe("collectCounts", () => {
  it("counts store entities, tolerating null Group holes", () => {
    expect(collectCounts(makeState())).toEqual({
      groups: 3,
      groupMods: 3,
      ghosts: 1,
      regMods: 3,
      activeMods: 2,
      galleryReg: 2,
      propQ: 1,
      metaQ: 0,
      inputs: 2,
      links: 1,
      expr: 2,
      tweens: 3,
      scenes: 2,
      rows: 2,
      prog: 1,
      transitioning: 1,
      perf: 1,
      windows: 2,
      aux: 1,
      images: 2,
      videos: 1,
    });
  });

  it("returns zeros for an empty state", () => {
    const counts = collectCounts({});
    expect(Object.values(counts).every((value) => value === 0)).toBe(true);
    expect(Object.keys(counts)).toHaveLength(21);
  });

  it("returns zeros for null/undefined state and broken slices", () => {
    expect(collectCounts(null).groups).toBe(0);
    expect(collectCounts(undefined).groups).toBe(0);

    const broken = collectCounts({
      groups: { groups: "not-an-array" },
      modules: { registered: null, active: 42, propQueue: [], metaQueue: null },
      inputs: null,
      scenes: null,
      outputs: { auxillary: "nope" },
      windows: 7,
    });
    expect(broken.groups).toBe(0);
    expect(broken.regMods).toBe(0);
    expect(broken.inputs).toBe(0);
    expect(broken.scenes).toBe(0);
    expect(broken.aux).toBe(0);
    expect(broken.windows).toBe(0);
  });

  it("treats missing modules arrays on Groups as zero", () => {
    const counts = collectCounts({
      groups: { groups: [{ id: "g" }, { id: "g2", modules: null }] },
    });
    expect(counts.groups).toBe(2);
    expect(counts.groupMods).toBe(0);
  });
});

describe("readHeapStats", () => {
  it("never throws and returns heap stats or a fallback object", () => {
    expect(() => readHeapStats()).not.toThrow();

    const heap = readHeapStats();

    if (heap !== null) {
      // In plain Node (no v8 binding wired up) the memoryUsage fallback
      // still yields real numbers.
      expect(typeof heap.rssMB).toBe("number");
      expect(typeof heap.usedHeapMB).toBe("number");
    }
  });
});

describe("normaliseHeapStats", () => {
  it("returns null when neither source is available", () => {
    expect(normaliseHeapStats({})).toBeNull();
    expect(normaliseHeapStats(null)).toBeNull();
  });

  it("normalises v8.getHeapStatistics output", () => {
    const heap = normaliseHeapStats({
      heapStatistics: {
        heap_size_limit: 4192 * MB,
        used_heap_size: 3469 * MB,
        total_heap_size: 3800 * MB,
        total_physical_size: 3200 * MB,
        malloced_memory: 8 * MB,
        external_memory: 92 * MB,
        number_of_native_contexts: 3,
        number_of_detached_contexts: 7,
      },
    });

    expect(heap.usedHeapMB).toBe(3469);
    expect(heap.heapLimitMB).toBe(4192);
    expect(heap.totalHeapMB).toBe(3800);
    expect(heap.physicalMB).toBe(3200);
    expect(heap.mallocMB).toBe(8);
    expect(heap.externalMB).toBe(92);
    expect(heap.nativeCtx).toBe(3);
    expect(heap.detachedCtx).toBe(7);
    expect(heap.heapPct).toBe(82.8);
  });

  it("falls back to process.memoryUsage when v8 stats are missing", () => {
    const heap = normaliseHeapStats({
      memoryUsage: {
        rss: 512 * MB,
        heapTotal: 200 * MB,
        heapUsed: 150 * MB,
        external: 24 * MB,
        arrayBuffers: 6 * MB,
      },
    });

    expect(heap.usedHeapMB).toBe(150);
    expect(heap.totalHeapMB).toBe(200);
    expect(heap.externalMB).toBe(24);
    expect(heap.rssMB).toBe(512);
    expect(heap.arrayBuffersMB).toBe(6);
    expect(heap.heapLimitMB).toBeNull();
    expect(heap.heapPct).toBeNull();
  });

  it("prefers v8 used_heap_size but keeps memoryUsage rss/arrayBuffers", () => {
    const heap = normaliseHeapStats({
      memoryUsage: {
        rss: 512 * MB,
        heapUsed: 150 * MB,
        heapTotal: 200 * MB,
        external: 24 * MB,
        arrayBuffers: 6 * MB,
      },
      heapStatistics: {
        heap_size_limit: 4000 * MB,
        used_heap_size: 1000 * MB,
        total_heap_size: 1100 * MB,
        number_of_native_contexts: 1,
        number_of_detached_contexts: 0,
      },
    });

    expect(heap.usedHeapMB).toBe(1000);
    expect(heap.totalHeapMB).toBe(1100);
    expect(heap.rssMB).toBe(512);
    expect(heap.arrayBuffersMB).toBe(6);
    expect(heap.heapPct).toBe(25);
  });
});

describe("isHeapNearLimit", () => {
  it("is false without heap or limit information", () => {
    expect(isHeapNearLimit(null)).toBe(false);
    expect(isHeapNearLimit({ heapPct: null })).toBe(false);
    expect(isHeapNearLimit({})).toBe(false);
  });

  it("trips at or above 85% of the heap limit", () => {
    expect(isHeapNearLimit({ heapPct: 84.9 })).toBe(false);
    expect(isHeapNearLimit({ heapPct: 85 })).toBe(true);
    expect(isHeapNearLimit({ heapPct: 99.5 })).toBe(true);
  });
});

describe("formatMemDiag", () => {
  const counts = collectCounts(makeState());

  it("formats heap stats and counts on one line", () => {
    const line = formatMemDiag({
      heap: {
        usedHeapMB: 3469,
        heapLimitMB: 4192,
        heapPct: 82.8,
        totalHeapMB: 3800,
        rssMB: 5100.2,
        mallocMB: 8,
        externalMB: 92,
        arrayBuffersMB: 6,
        physicalMB: 3200,
        nativeCtx: 3,
        detachedCtx: 7,
      },
      counts,
    });

    expect(line).toContain("used=3469.0MB");
    expect(line).toContain("limit=4192.0MB");
    expect(line).toContain("pct=82.8%");
    expect(line).toContain("rss=5100.2MB");
    expect(line).toContain("detachedCtx=7");
    expect(line).toContain("groups=3");
    expect(line).toContain("ghosts=1");
    expect(line).toContain("regMods=3");
    expect(line).toContain("transitioning=1");
    expect(line).not.toContain("WARN=");
  });

  it("adds WARN when the heap is near the limit", () => {
    const line = formatMemDiag({
      heap: {
        usedHeapMB: 3900,
        heapLimitMB: 4192,
        heapPct: 93.0,
        totalHeapMB: 4090,
        rssMB: 5200,
        mallocMB: 8,
        externalMB: 92,
        arrayBuffersMB: 6,
        physicalMB: 4000,
        nativeCtx: 3,
        detachedCtx: 12,
      },
      counts,
    });

    expect(line).toContain("WARN=near-heap-limit");
  });

  it("degrades gracefully when heap stats are unavailable", () => {
    const line = formatMemDiag({ heap: null, counts });

    expect(line).toContain("heap=unavailable");
    expect(line).toContain("groups=3");
  });
});

describe("createMemoryGate", () => {
  it("fires immediately then suppresses until the interval passes", () => {
    const gate = createMemoryGate(5000);

    expect(gate(0)).toBe(true);
    expect(gate(1000)).toBe(false);
    expect(gate(4999)).toBe(false);
    expect(gate(5000)).toBe(true);
    expect(gate(6000)).toBe(false);
    expect(gate(10000)).toBe(true);
  });

  it("floors the interval to a sane minimum", () => {
    const gate = createMemoryGate(0);

    expect(gate(0)).toBe(true);
    expect(gate(1)).toBe(false);
  });
});
