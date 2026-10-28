import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { DvdRoom } from "../../src/renderer/src/application/point10/scenes/DvdRoom.js";
import { SmokeRoom } from "../../src/renderer/src/application/point10/scenes/SmokeRoom.js";

const GLASS_HALF_X = 6.3;
const GLASS_HALF_Z = 12.3;
const GLASS_HALF_Y = 6.3;

function makeCamera({ aspect = 16 / 9, position = [0, 6.5, 17] } = {}) {
  const camera = new THREE.PerspectiveCamera(60, aspect, 0.1, 100);
  camera.position.set(...position);
  camera.lookAt(0, 0, 0);
  camera.updateMatrixWorld();
  return camera;
}

function orbitCamera({ aspect = 16 / 9, y = 6.5, yaw = 0 } = {}) {
  return makeCamera({
    aspect,
    position: [Math.cos(yaw) * 17, y, Math.sin(yaw) * 17],
  });
}

function topCorners(top) {
  return [
    [-GLASS_HALF_X, top, -GLASS_HALF_Z],
    [GLASS_HALF_X, top, -GLASS_HALF_Z],
    [-GLASS_HALF_X, top, GLASS_HALF_Z],
    [GLASS_HALF_X, top, GLASS_HALF_Z],
  ];
}

describe.each([
  ["SmokeRoom", SmokeRoom],
  ["DvdRoom", DvdRoom],
])("%s glass fit", (_name, SceneClass) => {
  it("puts the box's top edge on the output's top edge", () => {
    const camera = orbitCamera();
    const top = SceneClass.requiredGlassTop(camera);

    const projected = topCorners(top).map((corner) =>
      new THREE.Vector3(...corner).project(camera),
    );

    for (const point of projected) {
      expect(point.y).toBeGreaterThanOrEqual(1 - 1e-6);
    }
    // the far top edge (the lowest of the four) sits exactly on the border
    expect(Math.min(...projected.map((point) => point.y))).toBeCloseTo(1, 5);
  });

  it("is a bottom-anchored height, not a centre scale", () => {
    const camera = orbitCamera();
    const top = SceneClass.requiredGlassTop(camera);
    const { height, centreY } = SceneClass.glassFit(top);

    expect(height).toBeCloseTo(top - -GLASS_HALF_Y, 6);
    expect(centreY).toBeCloseTo((top + -GLASS_HALF_Y) / 2, 6);
    expect(centreY - height / 2).toBeCloseTo(-GLASS_HALF_Y, 6);
  });

  it("tracks the output's vertical FOV, not its aspect ratio", () => {
    const wide = orbitCamera({ aspect: 16 / 9 });
    const tall = orbitCamera({ aspect: 4 / 3 });

    expect(SceneClass.requiredGlassTop(tall)).toBeCloseTo(
      SceneClass.requiredGlassTop(wide),
      6,
    );
  });
});

describe("DVD Room glass fit", () => {
  it("keeps the top edge off-screen while the camera orbits and bobs", () => {
    for (const yaw of [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2]) {
      for (const y of [5.3, 6.5, 7.7]) {
        const camera = orbitCamera({ yaw, y });
        const top = DvdRoom.requiredGlassTop(camera);

        for (const corner of topCorners(top)) {
          expect(
            new THREE.Vector3(...corner).project(camera).y,
          ).toBeGreaterThanOrEqual(1 - 1e-6);
        }
      }
    }
  });
});

describe("Smoke Room glass fit", () => {
  it("fits the fixed smoke-room camera", () => {
    const camera = makeCamera({ position: [14, 5.5, 4.4] });
    const top = SmokeRoom.requiredGlassTop(camera);

    expect(top).toBeGreaterThan(GLASS_HALF_Y);
    for (const corner of topCorners(top)) {
      expect(
        new THREE.Vector3(...corner).project(camera).y,
      ).toBeGreaterThanOrEqual(1 - 1e-6);
    }
  });
});
