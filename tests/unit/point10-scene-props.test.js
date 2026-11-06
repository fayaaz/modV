import { describe, expect, it } from "vitest";
import {
  DVD_ROOM_PROPS,
  ROLLING_FIRE_PROPS,
  SCENE_PROP_CONFIGS,
  SMOKE_ROOM_PROPS,
  applySmokeRoomProps,
  readSceneProps,
  scenePropsFor,
} from "../../src/renderer/src/application/point10/sceneProps.js";

const moduleFiles = import.meta.glob(
  "../../src/renderer/src/application/renderers/2d/modules/Point10*.js",
  { eager: true },
);

const sceneModules = Object.entries(moduleFiles).map(([path, module]) => ({
  path,
  definition: module.default,
}));

function fakeScene(fields = {}) {
  return {
    _uEmitDensity: { value: 0 },
    _uFireIntensity: { value: 0 },
    _smokeLifespan: 0,
    _fireLifespan: 0,
    _simSpeed: 0,
    ...fields,
  };
}

describe("Point10 scene prop registry", () => {
  it("covers every imported Point10 module", () => {
    expect(sceneModules.length).toBeGreaterThan(0);

    for (const { path, definition } of sceneModules) {
      expect(definition, path).toBeTruthy();
      expect(SCENE_PROP_CONFIGS[definition.meta.name], path).toBeDefined();
    }
  });

  it("gives every scene at least one input and wires it into the module", () => {
    for (const { definition } of sceneModules) {
      const config = scenePropsFor(definition.meta.name);

      for (const [key, prop] of Object.entries(config.props)) {
        expect(definition.props[key], `${definition.meta.name}.${key}`).toEqual(
          prop,
        );
      }
      expect(Object.keys(config.props).length).toBeGreaterThan(0);
      for (const prop of Object.values(config.props)) {
        expect(prop.label).toBeTruthy();
        expect(typeof prop.default).not.toBe("undefined");
      }
    }
  });

  it("applies defaults safely before a scene has built its GPU state", () => {
    for (const { definition } of sceneModules) {
      const config = scenePropsFor(definition.meta.name);

      expect(() => config.apply({}, {})).not.toThrow();

      const values = config.apply({}, {});
      for (const [key, prop] of Object.entries(config.props)) {
        expect(values[key]).toBe(prop.default);
      }
    }
  });

  it("clamps values to their declared ranges", () => {
    const values = readSceneProps(
      {
        smokeDensity: 500,
        fireIntensity: -10,
        smokeLifespan: 0,
        fireLifespan: 99,
        simulationSpeed: 100,
      },
      SMOKE_ROOM_PROPS,
    );

    expect(values.smokeDensity).toBe(100);
    expect(values.fireIntensity).toBe(0);
    expect(values.smokeLifespan).toBe(SMOKE_ROOM_PROPS.smokeLifespan.min);
    expect(values.fireLifespan).toBe(SMOKE_ROOM_PROPS.fireLifespan.max);
    expect(values.simulationSpeed).toBe(4);
  });

  it("ignores non-numeric values", () => {
    const values = readSceneProps({
      smokeDensity: "thick",
      fireIntensity: null,
      simulationSpeed: Number.NaN,
    });

    expect(values.smokeDensity).toBe(28);
    expect(values.fireIntensity).toBe(40);
    expect(values.simulationSpeed).toBe(1.2);
  });
});

describe("Smoke scenes", () => {
  it("exposes smoke density alongside the other smoke room controls", () => {
    expect(Object.keys(SMOKE_ROOM_PROPS)).toEqual([
      "smokeDensity",
      "fireIntensity",
      "smokeLifespan",
      "fireLifespan",
      "simulationSpeed",
    ]);
    expect(SMOKE_ROOM_PROPS.smokeDensity.label).toBe("Smoke Density");
  });

  it("keeps the DVD Room's dimmer fire default", () => {
    expect(SMOKE_ROOM_PROPS.fireIntensity.default).toBe(40);
    expect(DVD_ROOM_PROPS.fireIntensity.default).toBe(22);
    expect(DVD_ROOM_PROPS.smokeDensity).toEqual(SMOKE_ROOM_PROPS.smokeDensity);
  });

  it("uses the Rolling Fire's smaller grid defaults", () => {
    expect(ROLLING_FIRE_PROPS.smokeDensity.default).toBe(16);
    expect(ROLLING_FIRE_PROPS.fireIntensity.default).toBe(26);
    expect(ROLLING_FIRE_PROPS.smokeLifespan.default).toBe(1.5);
    expect(ROLLING_FIRE_PROPS.fireLifespan.default).toBe(0.9);
  });

  it("writes the values onto a live scene", () => {
    const scene = fakeScene();
    const values = applySmokeRoomProps(scene, {
      smokeDensity: 12,
      fireIntensity: 55,
      smokeLifespan: 9,
      fireLifespan: 2,
      simulationSpeed: 2.5,
    });

    expect(values.smokeDensity).toBe(12);
    expect(scene._uEmitDensity.value).toBe(12);
    expect(scene._uFireIntensity.value).toBe(55);
    expect(scene._smokeLifespan).toBe(9);
    expect(scene._fireLifespan).toBe(2);
    expect(scene._simSpeed).toBe(2.5);
  });

  it("survives the frame before the WebGPU volume exists", () => {
    expect(() => applySmokeRoomProps({}, {})).not.toThrow();
  });
});

describe("Attractor Particles inputs", () => {
  it("exposes distance and brightness as usable levers", () => {
    const config = scenePropsFor("Point10 Attractor Particles");

    expect(config.props.cameraDistance.label).toBe("Camera Distance");
    expect(config.props.brightness.label).toBe("Particle Brightness");
    expect(config.props.particleSize.default).toBe(1);
  });

  it("applies the camera distance, size and brightness", () => {
    const config = scenePropsFor("Point10 Attractor Particles");
    const scaleUniform = { value: 0 };
    const scene = {
      _uScale: scaleUniform,
      _camRadius: 9.9,
      _speedScale: 1,
      _bloomScale: 1,
    };

    const values = config.apply(scene, {
      cameraDistance: 18,
      particleSize: 2,
      brightness: 1.5,
      maxSpeed: 0.5,
    });

    expect(values.cameraDistance).toBe(18);
    expect(scene._camRadius).toBe(18);
    expect(scene._particleSize).toBe(2);
    expect(scaleUniform.value).toBeCloseTo(0.016);
    expect(scene._bloomScale).toBe(1.5);
    expect(scene._speedScale).toBe(0.5);
  });
});

describe("Scene-specific bindings", () => {
  it("wires Around Plus water and ring controls", () => {
    const config = scenePropsFor("Point10 Around Plus");
    const scene = {};

    config.apply(scene, { spinSpeed: 2, ringBrightness: 1.5, waterChurn: 0.5 });

    expect(scene.spinSpeed).toBe(2);
    expect(scene.ringBrightness).toBe(1.5);
    expect(scene.waterChurn).toBe(0.5);
  });

  it("wires Chromatic Line and Wave to the same amp/distortion levers", () => {
    for (const name of ["Point10 Chromatic Line", "Point10 Chromatic Wave"]) {
      const config = scenePropsFor(name);
      const scene = {};

      config.apply(scene, { distortion: 0.2, amplitude: 2, glow: 1.5 });

      expect(scene.distortion).toBe(0.2);
      expect(scene.ampScale).toBe(2);
      expect(scene.glowScale).toBe(1.5);
    }
  });
});
