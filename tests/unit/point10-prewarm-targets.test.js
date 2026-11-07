import { describe, expect, it, vi } from "vitest";
import {
  getPoint10PrewarmTargets,
  modulePropDefaults,
  prewarmSceneModules,
} from "../../src/renderer/src/application/worker/point10-prewarm.js";

const registered = () => ({
  "Point10 Smoke Room": {
    meta: { name: "Point10 Smoke Room" },
    webgpu: true,
    props: {
      smokeDensity: { default: 28 },
      boxScale: { default: 2.8 },
      noDefault: {},
    },
    prewarm: vi.fn(),
  },
  "Point10 Waveform": {
    meta: { name: "Point10 Waveform" },
    webgpu: true,
    props: {},
    prewarm: vi.fn(),
  },
  "Some Effect": {
    meta: { name: "Some Effect" },
    props: { intensity: { default: 1 } },
  },
});

const scene = (moduleNames) => {
  const modules = {};
  const groupModules = [];

  for (let i = 0; i < moduleNames.length; i++) {
    const id = `module-${i}`;
    groupModules.push(id);
    modules[id] = { $moduleName: moduleNames[i] };
  }

  return { groups: [{ modules: groupModules }], modules };
};

describe("modulePropDefaults", () => {
  it("collects the declared defaults and skips props without one", () => {
    expect(modulePropDefaults(registered()["Point10 Smoke Room"])).toEqual({
      smokeDensity: 28,
      boxScale: 2.8,
    });
  });
});

describe("getPoint10PrewarmTargets", () => {
  it("returns one target per distinct Point10 scene", () => {
    const definitions = registered();
    const targets = getPoint10PrewarmTargets(
      scene([
        "Point10 Smoke Room",
        "Some Effect",
        "Point10 Smoke Room",
        "Point10 Waveform",
      ]),
      definitions,
    );

    expect(targets.map((target) => target.name)).toEqual([
      "Point10 Smoke Room",
      "Point10 Waveform",
    ]);
    expect(targets[0].definition).toBe(definitions["Point10 Smoke Room"]);
    expect(targets[0].defaults).toEqual({ smokeDensity: 28, boxScale: 2.8 });
  });

  it("skips WebGL Point10 scenes to keep the context budget free", () => {
    const definitions = registered();
    definitions["Point10 WebGL Scene"] = {
      meta: { name: "Point10 WebGL Scene" },
      props: {},
      prewarm: vi.fn(),
    };

    expect(
      getPoint10PrewarmTargets(scene(["Point10 WebGL Scene"]), definitions),
    ).toEqual([]);
  });

  it("ignores modules without a prewarm hook and unknown modules", () => {
    const targets = getPoint10PrewarmTargets(
      scene(["Some Effect", "Missing Module"]),
      registered(),
    );

    expect(targets).toEqual([]);
  });

  it("uses a module snapshot's meta name when $moduleName is absent", () => {
    const definitions = registered();
    const targets = getPoint10PrewarmTargets(
      {
        groups: [{ modules: ["m"] }],
        modules: { m: { meta: { name: "Point10 Waveform" } } },
      },
      definitions,
    );

    expect(targets.map((target) => target.name)).toEqual(["Point10 Waveform"]);
  });

  it("accepts a modules object on a group", () => {
    const targets = getPoint10PrewarmTargets(
      {
        groups: [{ modules: { m: { $moduleName: "Point10 Waveform" } } }],
        modules: { m: { $moduleName: "Point10 Waveform" } },
      },
      registered(),
    );

    expect(targets).toHaveLength(1);
  });
});

describe("prewarmSceneModules", () => {
  it("warms every target with its defaults as a reusable template", async () => {
    const definitions = registered();
    const count = await prewarmSceneModules({
      scene: scene(["Point10 Smoke Room", "Point10 Waveform"]),
      registered: definitions,
      width: 640,
      height: 480,
    });

    expect(count).toBe(2);
    expect(definitions["Point10 Smoke Room"].prewarm).toHaveBeenCalledWith({
      module: {
        $id: "prewarm:Point10 Smoke Room",
        props: { smokeDensity: 28, boxScale: 2.8 },
        meta: {},
      },
      width: 640,
      height: 480,
      warmOnly: true,
    });
    expect(definitions["Point10 Waveform"].prewarm).toHaveBeenCalledTimes(1);
  });

  it("keeps warming after one scene throws", async () => {
    const definitions = registered();
    definitions["Point10 Smoke Room"].prewarm.mockRejectedValue(
      new Error("no gpu"),
    );

    await expect(
      prewarmSceneModules({
        scene: scene(["Point10 Smoke Room", "Point10 Waveform"]),
        registered: definitions,
        width: 640,
        height: 480,
      }),
    ).resolves.toBe(2);

    expect(definitions["Point10 Waveform"].prewarm).toHaveBeenCalledTimes(1);
  });
});
