import { describe, expect, it } from "vitest";
import {
  BallOfFire,
  RollingFire,
} from "../../src/renderer/src/application/point10/scenes/RollingFire.js";

const construct = (SceneClass) => new SceneClass(null, null, {});

describe("Rolling Fire transparency", () => {
  it("starts with the scene offset at the origin", () => {
    for (const SceneClass of [RollingFire, BallOfFire]) {
      const scene = construct(SceneClass);

      expect(scene._sceneOffsetX).toBe(0);
      expect(scene._sceneOffsetY).toBe(0);
    }
  });

  it("composites Rolling Fire as a transparent foreground overlay", () => {
    const scene = construct(RollingFire);

    expect(scene._transparentOverlay).toBe(true);
  });

  it("renders Ball of Fire without a background too", () => {
    const scene = construct(BallOfFire);

    expect(scene._transparentOverlay).toBe(true);
    expect(scene._freezeBall).toBe(true);
    expect(scene.role).toBe("standalone");
  });
});
