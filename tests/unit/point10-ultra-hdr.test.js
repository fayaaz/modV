import * as THREE from "three";
import { describe, expect, it } from "vitest";
import {
  CAMERA_POSITION,
  ENERGY,
  HEMI,
  KNOT,
  MORPH,
  PULSE,
  SPIN,
  buildUltraHDRKnotGeometry,
  createDriveState,
  driveUltraHDR,
  eightCurve,
  morphFromEnergy,
  stepEnergy,
  stepKick,
} from "../../src/renderer/src/application/point10/scenes/UltraHDR.js";

// UltraHDR renders a mirror torus knot with a WebGL environment, so the draw
// itself needs a GPU. The audio drive (kick pulse, smoothed energy, morph),
// the figure-8 morph target and the scene constants are plain arithmetic and
// are tested directly.
describe("UltraHDR smoothed energy", () => {
  it("stays at zero in silence", () => {
    expect(stepEnergy(0, 0, 0, 0, 0.016)).toBe(0);
  });

  it("follows the mean band level through the attack one-pole", () => {
    const dt = 0.016;
    const k = Math.min(1, (1 - Math.exp(-dt * ENERGY.rate)) * ENERGY.attack);
    const first = stepEnergy(1, 1, 1, 0, dt);
    const second = stepEnergy(1, 1, 1, first, dt);

    expect(first).toBeCloseTo(k);
    expect(second).toBeGreaterThan(first);
    expect(second).toBeLessThan(1);
  });

  it("clamps the band mean to 0..1 and sanitises bad input", () => {
    const dt = 0.016;
    const k = Math.min(1, (1 - Math.exp(-dt * ENERGY.rate)) * ENERGY.attack);

    expect(stepEnergy(10, 10, 10, 0, dt)).toBeCloseTo(k);
    expect(stepEnergy(-1, -2, -3, 0, dt)).toBe(0);
    expect(stepEnergy(Number.NaN, Number.NaN, Number.NaN, 0.5, dt)).toBeCloseTo(
      0.5 * (1 - k),
    );
    expect(stepEnergy(1, 1, 1, 0.25, Number.NaN)).toBe(0.25);
  });

  it("releases toward silence when the music stops", () => {
    expect(stepEnergy(0, 0, 0, 0.8, 0.05)).toBeLessThan(0.8);
  });
});

describe("UltraHDR kick pulse", () => {
  it("does not fire below the jump threshold", () => {
    const state = createDriveState();
    const next = stepKick(state, PULSE.jumpThreshold, 1, 0.016);

    expect(next.lastPulse).toBe(state.lastPulse);
    expect(next.pulseTarget).toBe(0);
    expect(next.pulse).toBe(0);
  });

  it("fires on a sharp attack and decays the target immediately", () => {
    const dt = 0.016;
    const next = stepKick(createDriveState(), 0.5, 1, dt);
    const target =
      Math.min(1, 0.5 * PULSE.targetGain) * Math.exp(-dt * PULSE.targetDecay);

    expect(next.lastPulse).toBe(1);
    expect(next.pulseTarget).toBeCloseTo(target);
    expect(next.pulse).toBeCloseTo(target * Math.min(1, dt * PULSE.rise));
    expect(next.kick).toBe(0.5);
  });

  it("gates re-fires behind the cooldown", () => {
    const fired = stepKick(createDriveState(), 0.5, 1, 0.016);
    const tooSoon = stepKick(fired, 0.9, 1.3, 0.01);
    const afterCooldown = stepKick(tooSoon, 1, 1.7, 0.01);

    expect(tooSoon.lastPulse).toBe(1);
    expect(afterCooldown.lastPulse).toBe(1.7);
    expect(afterCooldown.pulseTarget).toBeGreaterThan(
      tooSoon.pulseTarget * Math.exp(-0.01 * PULSE.targetDecay),
    );
  });

  it("never exceeds a full-scale kick target and sanitises bad input", () => {
    const full = stepKick(createDriveState(), 10, 1, 0.016);
    const bad = stepKick(
      createDriveState(),
      Number.NaN,
      Number.NaN,
      Number.NaN,
    );

    expect(full.kick).toBe(1);
    expect(full.pulseTarget).toBeLessThanOrEqual(Math.exp(-0.016 * 2.5));
    expect(bad.kick).toBe(0);
    expect(Number.isFinite(bad.pulse)).toBe(true);
    expect(Number.isFinite(bad.pulseTarget)).toBe(true);
  });
});

describe("UltraHDR figure-8 morph", () => {
  it("is a no-op in silence and saturates when busy", () => {
    expect(morphFromEnergy(0)).toBe(0);
    expect(morphFromEnergy(1)).toBe(1);
    expect(morphFromEnergy(1 / MORPH.energyGain)).toBe(1);
  });

  it("scales the smoothed energy sub-linearly", () => {
    const energy = 0.4;
    const expected = Math.pow(energy * MORPH.energyGain, MORPH.exponent);

    expect(morphFromEnergy(energy)).toBeCloseTo(expected);
    expect(morphFromEnergy(energy)).toBeLessThan(energy * MORPH.energyGain);
  });

  it("clamps and sanitises bad input", () => {
    expect(morphFromEnergy(-1)).toBe(0);
    expect(morphFromEnergy(Number.NaN)).toBe(0);
  });
});

describe("UltraHDR drive", () => {
  it("starts from the documented resting state", () => {
    expect(createDriveState()).toEqual({
      kick: 0,
      pulse: 0,
      pulseTarget: 0,
      lastPulse: -1,
      energy: 0,
    });
  });

  it("spins slowly and keeps the resting scale in silence", () => {
    const drive = driveUltraHDR(createDriveState(), {
      bass: 0,
      mid: 0,
      high: 0,
      beat: 0,
      dt: 0.1,
      time: 0,
      spinScale: 1,
      pulseScale: 1,
    });

    expect(drive.morph).toBe(0);
    expect(drive.spin).toBeCloseTo(0.1 * SPIN.base);
    expect(drive.scale).toBe(1);
  });

  it("combines active energy, spin and pulse scale into the frame values", () => {
    const dt = 0.016;
    const state = createDriveState();
    const drive = driveUltraHDR(state, {
      bass: 1,
      mid: 1,
      high: 1,
      beat: 1,
      dt,
      time: 1,
      spinScale: 2,
      pulseScale: 0.5,
    });
    const energy = Math.min(
      1,
      (1 - Math.exp(-dt * ENERGY.rate)) * ENERGY.attack,
    );
    const pulseTarget = Math.exp(-dt * PULSE.targetDecay);
    const pulse = pulseTarget * Math.min(1, dt * PULSE.rise);

    expect(drive.energy).toBeCloseTo(energy);
    expect(drive.morph).toBeCloseTo(
      Math.pow(Math.min(1, energy * MORPH.energyGain), MORPH.exponent),
    );
    expect(drive.pulse).toBeCloseTo(pulse);
    expect(drive.spin).toBeCloseTo(dt * (SPIN.base + energy * SPIN.energy) * 2);
    expect(drive.scale).toBeCloseTo(1 + pulse * PULSE.scaleAmount * 0.5);
    expect(drive.state).not.toBe(state);
    expect(drive.state.lastPulse).toBe(1);
  });

  it("applies the pulse and spin scale inputs independently", () => {
    const input = {
      bass: 1,
      mid: 0,
      high: 0,
      beat: 1,
      dt: 0.016,
      time: 1,
      spinScale: 0,
      pulseScale: 0,
    };
    const drive = driveUltraHDR(createDriveState(), input);

    expect(drive.spin).toBe(0);
    expect(drive.scale).toBe(1);
    expect(drive.morph).toBeGreaterThan(0);
  });

  it("survives non-finite input without producing NaN", () => {
    const drive = driveUltraHDR(createDriveState(), {
      bass: Number.NaN,
      mid: Number.NaN,
      high: Number.NaN,
      beat: Number.NaN,
      dt: Number.NaN,
      time: Number.NaN,
      spinScale: Number.NaN,
      pulseScale: Number.NaN,
    });

    expect(Number.isFinite(drive.energy)).toBe(true);
    expect(Number.isFinite(drive.morph)).toBe(true);
    expect(Number.isFinite(drive.spin)).toBe(true);
    expect(Number.isFinite(drive.scale)).toBe(true);
  });
});

describe("UltraHDR figure-8 morph target", () => {
  it("draws a Gerono lemniscate with the z-lift", () => {
    const out = new THREE.Vector3();

    eightCurve(0, out);
    expect(out.x).toBeCloseTo(1.15);
    expect(out.y).toBeCloseTo(0);
    expect(out.z).toBeCloseTo(0);

    eightCurve(Math.PI / 2, out);
    expect(out.x).toBeCloseTo(0);
    expect(out.y).toBeCloseTo(0);
    expect(out.z).toBeCloseTo(0.85);

    eightCurve(Math.PI, out);
    expect(out.x).toBeCloseTo(-1.15);
    expect(out.z).toBeCloseTo(0);
  });

  it("matches the torus knot's vertex layout for morphing", () => {
    const geometry = buildUltraHDRKnotGeometry();
    const position = geometry.getAttribute("position");
    const morphPosition = geometry.morphAttributes.position[0];
    const morphNormal = geometry.morphAttributes.normal[0];

    expect(position.count).toBe(
      (KNOT.tubularSegments + 1) * (KNOT.radialSegments + 1),
    );
    expect(morphPosition.count).toBe(position.count);
    expect(morphNormal.count).toBe(position.count);
    expect(Number.isFinite(morphPosition.getX(0))).toBe(true);
    expect(Number.isFinite(morphNormal.getX(0))).toBe(true);
  });
});

describe("UltraHDR constants", () => {
  it("uses the three.js UltraHDR example's torus knot", () => {
    expect(KNOT).toEqual({
      radius: 1,
      tube: 0.4,
      tubularSegments: 128,
      radialSegments: 128,
      p: 1,
      q: 3,
    });
    expect(CAMERA_POSITION).toEqual([0, 0, 7]);
  });

  it("keeps the hemisphere fallback and response tuning", () => {
    expect(HEMI).toEqual({ sky: 0xffffff, ground: 0x202020, intensity: 0.6 });
    expect(PULSE.jumpThreshold).toBe(0.08);
    expect(PULSE.cooldown).toBe(0.6);
    expect(PULSE.scaleAmount).toBe(0.18);
    expect(MORPH.energyGain).toBe(1.6);
    expect(MORPH.exponent).toBe(1.2);
    expect(SPIN.base).toBe(0.3);
    expect(SPIN.energy).toBe(4);
  });
});
