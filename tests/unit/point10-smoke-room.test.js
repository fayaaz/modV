import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { DvdRoom } from "../../src/renderer/src/application/point10/scenes/DvdRoom.js";
import { SmokeRoom } from "../../src/renderer/src/application/point10/scenes/SmokeRoom.js";
import { SMOKE_ROOM_PROPS } from "../../src/renderer/src/application/point10/sceneProps.js";

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

describe("Smoke Room box scale", () => {
  it("scales the glass extents with the box scale", () => {
    const base = SmokeRoom.boxHalfExtents();
    const scaled = SmokeRoom.boxHalfExtents(2.5);

    expect(scaled.x).toBeCloseTo(base.x * 2.5, 6);
    expect(scaled.y).toBeCloseTo(base.y * 2.5, 6);
    expect(scaled.z).toBeCloseTo(base.z * 2.5, 6);
  });

  it("puts the scaled box's top edge on the output's top edge", () => {
    const scale = 1.5;
    const camera = orbitCamera();
    const top = SmokeRoom.requiredGlassTop(camera, scale);
    const { x: halfX, z: halfZ } = SmokeRoom.boxHalfExtents(scale);

    const corners = [
      [-halfX, top, -halfZ],
      [halfX, top, -halfZ],
      [-halfX, top, halfZ],
      [halfX, top, halfZ],
    ];
    const projected = corners.map((corner) =>
      new THREE.Vector3(...corner).project(camera),
    );

    for (const point of projected) {
      expect(point.y).toBeGreaterThanOrEqual(1 - 1e-6);
    }
    expect(Math.min(...projected.map((point) => point.y))).toBeCloseTo(1, 5);
  });

  it("anchors the scaled glass fit to the scaled bottom edge", () => {
    const scale = 1.3;
    const top = 20;
    const { y: halfY } = SmokeRoom.boxHalfExtents(scale);
    const { height, centreY } = SmokeRoom.glassFit(top, scale);

    expect(centreY - height / 2).toBeCloseTo(-halfY, 6);
    expect(centreY + height / 2).toBeCloseTo(top, 6);
  });

  it("keeps the cameras inside the default box so no glass rim is visible", () => {
    const scale = SMOKE_ROOM_PROPS.boxScale.default;
    const { x: halfX, y: halfY, z: halfZ } = SmokeRoom.boxHalfExtents(scale);
    const cameras = [
      makeCamera({ position: [14, 5.5, 4.4] }),
      ...["5.3", "6.5", "7.7"].flatMap((y) =>
        [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2].map((yaw) =>
          orbitCamera({ y: Number(y), yaw }),
        ),
      ),
    ];

    for (const camera of cameras) {
      expect(Math.abs(camera.position.x)).toBeLessThan(halfX);
      expect(Math.abs(camera.position.y)).toBeLessThan(halfY);
      expect(Math.abs(camera.position.z)).toBeLessThan(halfZ);
    }
  });

  it("keeps the DVD Room's camera orbit within the default box", () => {
    const scale = SMOKE_ROOM_PROPS.boxScale.default;
    const { x: halfX, y: halfY, z: halfZ } = DvdRoom.boxHalfExtents(scale);

    for (const yaw of [0, Math.PI / 2, Math.PI, (3 * Math.PI) / 2]) {
      const camera = orbitCamera({ yaw });

      expect(Math.abs(camera.position.x)).toBeLessThan(halfX);
      expect(Math.abs(camera.position.y)).toBeLessThan(halfY);
      expect(Math.abs(camera.position.z)).toBeLessThan(halfZ);
    }
  });
});
