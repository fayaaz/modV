import { describe, expect, it } from "vitest";
import {
  LatentBackrooms,
  ROOM_CODES,
  ROOM_COUNT,
  ROOM_KINDS,
  ROOM_NAMES,
  START_Z,
  chevronGridZ,
  createRoomPlan,
  hallwayRipple,
  latentBackroomsDrive,
  latentRoomLayout,
  latentRoomSpacing,
  latentRoomWords,
  lightingDrive,
  rippleSpeed,
  roomCycle,
  runBob,
  runSway,
  walkSpeed,
  walkVector,
  TAU,
} from "../../src/renderer/src/application/point10/scenes/LatentBackrooms.js";

// The rooms, lights and composer need WebGL. The room plan, the automatic
// camera/room treadmill and the audio mapping are plain arithmetic - those are
// what the scene is built from, so they are tested in isolation.

describe("LatentBackrooms room plan", () => {
  it("builds 12 distinct rooms from the source name/kind/code tables", () => {
    const plans = Array.from({ length: ROOM_COUNT }, (_, i) =>
      createRoomPlan(i),
    );

    expect(ROOM_COUNT).toBe(12);
    expect(plans).toHaveLength(12);
    expect(new Set(plans.map((plan) => plan.name)).size).toBe(12);
    expect(new Set(plans.map((plan) => plan.kind)).size).toBeGreaterThanOrEqual(
      6,
    );
    expect(ROOM_NAMES).toHaveLength(ROOM_COUNT);
    expect(ROOM_KINDS).toHaveLength(ROOM_COUNT);
    expect(ROOM_CODES).toHaveLength(ROOM_COUNT);
    expect(plans.map((plan) => plan.name)).toEqual([...ROOM_NAMES]);
    expect(plans.map((plan) => plan.kind)).toEqual([...ROOM_KINDS]);
    expect(plans.map((plan) => plan.code)).toEqual([...ROOM_CODES]);
  });

  it("keeps every room inside the 56 unit corridor spacing", () => {
    expect(latentRoomSpacing).toBe(56);
    expect(START_Z).toBe(10);

    for (let i = 0; i < ROOM_COUNT; i++) {
      const plan = createRoomPlan(i);

      expect(plan.index).toBe(i);
      expect(plan.depth).toBeGreaterThanOrEqual(40);
      expect(plan.depth).toBeLessThan(latentRoomSpacing);
      expect(plan.width).toBeGreaterThan(14);
      expect(plan.height).toBeGreaterThan(7);
      expect(plan.complexity).toBeGreaterThanOrEqual(2);
      expect(plan.complexity).toBeLessThan(7);
      expect(plan.palette).toBeGreaterThanOrEqual(0);
      expect(plan.palette).toBeLessThan(4);
      expect(Number.isFinite(plan.phase)).toBe(true);
    }
  });

  it("clamps negative indexes and wraps the repeating room tables", () => {
    expect(createRoomPlan(-1).index).toBe(0);
    expect(createRoomPlan(12).name).toBe(ROOM_NAMES[0]);
    expect(createRoomPlan(13).kind).toBe(ROOM_KINDS[1]);
  });

  it("keeps the room words used by the Point10 checks", () => {
    expect(latentRoomWords).toContain("LATENT");
    expect(latentRoomWords).toContain("MEMORY");
  });
});

describe("LatentBackrooms room treadmill", () => {
  it("cycles a room every latentRoomSpacing of travel", () => {
    expect(roomCycle(0)).toBe(0);
    expect(roomCycle(27)).toBe(0);
    expect(roomCycle(28)).toBe(1);
    expect(roomCycle(84)).toBe(2);
  });

  it("places the live rooms at fixed spacing ahead of the camera", () => {
    const first = latentRoomLayout(0);

    expect(first.cycle).toBe(0);
    expect(first.current).toBe(0);
    expect(first.cameraZ).toBe(START_Z);
    expect(first.rooms).toHaveLength(ROOM_COUNT);
    expect(first.rooms[0]).toMatchObject({
      slot: 0,
      absolute: 0,
      index: 0,
      z: START_Z,
    });
    expect(first.rooms[1].z).toBe(START_Z - latentRoomSpacing);
  });

  it("slots the next room in front as the camera advances", () => {
    const later = latentRoomLayout(latentRoomSpacing);

    expect(later.cycle).toBe(1);
    expect(later.current).toBe(1);
    expect(later.cameraZ).toBe(START_Z - latentRoomSpacing);
    expect(later.rooms[0]).toMatchObject({
      absolute: 1,
      index: 1,
      z: START_Z - latentRoomSpacing,
    });
    expect(later.rooms[1].absolute).toBe(2);
  });
});

describe("LatentBackrooms automatic camera drive", () => {
  it("creeps forward at the silent walk speed", () => {
    const drive = latentBackroomsDrive();

    expect(drive.speed).toBeCloseTo(6.2);
    expect(drive.travel).toBe(0);
    expect(drive.ripplePhase).toBe(0);
    expect(drive.cycle).toBe(0);
    expect(drive.cameraZ).toBe(START_Z);
  });

  it("advances travel at walkSpeed and clamps long frames", () => {
    const drive = latentBackroomsDrive({ dt: 1 });

    expect(drive.step).toBeCloseTo(0.1);
    expect(drive.travel).toBeCloseTo(0.62);
    expect(drive.cameraZ).toBeCloseTo(START_Z - 0.62);
  });

  it("scales the demo motion with speedScale", () => {
    const drive = latentBackroomsDrive({ dt: 0.1, speedScale: 2 });

    expect(drive.speed).toBeCloseTo(12.4);
    expect(drive.travel).toBeCloseTo(1.24);
  });

  it("follows the bass, mid and beat", () => {
    const drive = latentBackroomsDrive({
      dt: 0.1,
      bass: 0.9,
      mid: 0.8,
      beat: 0.9,
    });

    expect(drive.speed).toBeCloseTo(walkSpeed(0.9, 0.8, 0.9));
    expect(drive.travel).toBeCloseTo(0.1 * walkSpeed(0.9, 0.8, 0.9));
    expect(drive.ripplePhase).toBeCloseTo(
      0.1 * rippleSpeed(0.9, 0.8, 0.9) * 0.32,
    );
  });

  it("wraps the ripple phase into 0..TAU", () => {
    const drive = latentBackroomsDrive({
      dt: 0.1,
      ripplePhase: TAU - 0.01,
      bass: 1,
    });

    expect(drive.ripplePhase).toBeGreaterThanOrEqual(0);
    expect(drive.ripplePhase).toBeLessThan(TAU);
  });

  it("sanitises negative and non-finite input", () => {
    const drive = latentBackroomsDrive({
      dt: Number.NaN,
      travel: Number.NaN,
      ripplePhase: Number.NaN,
      bass: Number.NaN,
      mid: Number.NaN,
      high: Number.NaN,
      beat: Number.NaN,
      speedScale: Number.NaN,
    });

    expect(drive.step).toBe(0);
    expect(drive.speed).toBeCloseTo(6.2);
    expect(drive.travel).toBe(0);
    expect(drive.ripplePhase).toBe(0);
    expect(latentBackroomsDrive({ dt: -1 }).step).toBe(0);
  });
});

describe("LatentBackrooms audio mapping", () => {
  it("scales the walk speed with bass, mid and beat", () => {
    expect(walkSpeed()).toBeCloseTo(6.2);
    expect(walkSpeed(0.9, 0.8, 0.9)).toBeGreaterThan(walkSpeed() + 8);
    expect(walkSpeed(2, 2, 2)).toBe(walkSpeed(1, 1, 1));
    expect(walkSpeed(-1, -1, -1)).toBe(walkSpeed());
  });

  it("drives the lights harder as the music gets louder", () => {
    const quiet = lightingDrive(0, 0, 0, 0);
    const loud = lightingDrive(0.9, 0.8, 0.7, 1);

    expect(quiet.wall).toBeLessThan(0.3);
    expect(loud.walk).toBeGreaterThan(quiet.walk * 3);
    expect(loud.bloom).toBeGreaterThan(quiet.bloom * 2);
  });

  it("propagates the hallway ripple faster than the walk", () => {
    expect(rippleSpeed(0.9, 0.8, 1)).toBeGreaterThan(
      walkSpeed(0.9, 0.8, 1) * 2,
    );
  });

  it("produces the head bob, shoulder sway and hallway pulse", () => {
    expect(Number.isFinite(runBob(0))).toBe(true);
    expect(Number.isFinite(runSway(0))).toBe(true);
    expect(runBob(0)).not.toBe(runBob(1));
    expect(hallwayRipple(Math.PI / 2, 0)).toBeCloseTo(1);
    expect(hallwayRipple(0, 0)).toBe(0);
    expect(hallwayRipple(-Math.PI / 2, 0)).toBe(0);
  });

  it("keeps the camera path inside the room and follows the audio", () => {
    const plan = createRoomPlan(0);
    const quiet = walkVector(0, plan);
    const musical = walkVector(0, plan, 0.9, 0.8, 0.7, 0.9, 1.4);

    expect(Number.isFinite(quiet.x)).toBe(true);
    expect(Number.isFinite(quiet.y)).toBe(true);
    expect(Number.isFinite(musical.x)).toBe(true);
    expect(Number.isFinite(musical.y)).toBe(true);
    expect(Math.abs(musical.x - quiet.x)).toBeGreaterThan(0.001);
    expect(Math.abs(musical.x)).toBeLessThanOrEqual(plan.width * 0.22 + 1e-9);
    expect(musical.y).toBeGreaterThan(1.6);
  });

  it("moves the chevron grid towards the camera as travel advances", () => {
    expect(Number.isFinite(chevronGridZ(0, 0))).toBe(true);
    expect(chevronGridZ(0, 0)).not.toBe(chevronGridZ(-10, 0));
  });
});

describe("LatentBackrooms scene shape", () => {
  it("builds the world without a DOM and starts every lever at 1", () => {
    const scene = new LatentBackrooms(null, null);

    expect(scene.rooms).toHaveLength(ROOM_COUNT);
    expect(scene.chevrons).toHaveLength(6);
    expect(scene.scene).toBeDefined();
    expect(scene.group).toBeDefined();
    expect(scene.paintsCanvas).toBe(true);
    expect(scene.captureTexture).toBeNull();
    expect(scene.speedScale).toBe(1);
    expect(scene.bloomScale).toBe(1);
    expect(scene.trailScale).toBe(1);
    expect(scene.brightnessScale).toBe(1);
  });
});
