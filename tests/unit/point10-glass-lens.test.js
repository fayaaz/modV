import { describe, expect, it } from "vitest";
import {
  CAMERA,
  CLEAR_ALPHA,
  ENERGY,
  GLASS_BOX,
  GLASS_MATERIAL,
  KICK,
  KICK_SMOOTH,
  MOTION,
  PHOTO_SCALE,
  PULSE,
  glassLensEnergy,
  glassLensKick,
  glassLensPulse,
  glassLensSway,
} from "../../src/renderer/src/application/point10/scenes/GlassLens.js";

// Glass Lens renders a physical glass block with a transmission pass, so the
// draw needs WebGL. The audio drive (energy smoothing, kick pulse, sway) and
// the rig constants are plain arithmetic - those are what the controls map to,
// so they are tested directly.
describe("GlassLens energy", () => {
  it("smooths the three bands toward their mean", () => {
    expect(glassLensEnergy(0, 0, 0, 0, 1)).toBe(0);

    const oneSecond = glassLensEnergy(0, 0.5, 0.5, 0.5, 1);
    expect(oneSecond).toBeCloseTo(0.5);

    const beta = 1 - Math.exp(-0.1 * ENERGY.tau);
    expect(glassLensEnergy(0, 1, 1, 1, 0.1)).toBeCloseTo(
      Math.min(1, beta * ENERGY.gain),
    );
  });

  it("clamps levels, sanitises bad input and holds still at dt 0", () => {
    expect(glassLensEnergy(0, 10, 10, 10, 1)).toBe(1);
    expect(glassLensEnergy(0, -1, -1, -1, 1)).toBe(0);
    expect(glassLensEnergy(0.3, 1, 0, 0, 0)).toBeCloseTo(0.3);

    const bad = glassLensEnergy(0.2, Number.NaN, Infinity, -5, 1);
    expect(bad).toBeGreaterThanOrEqual(0);
    expect(bad).toBeLessThanOrEqual(1);
  });
});

describe("GlassLens kick", () => {
  it("fires only on a sharp rise after the cooldown", () => {
    const idle = glassLensKick(0.3, 0.25, 0, -1, 10, 0.1);
    expect(idle.fire).toBe(false);
    expect(idle.target).toBeNull();

    const fire = glassLensKick(0.6, 0.2, 0, -1, 10, 0.1);
    expect(fire.fire).toBe(true);
    expect(fire.target).toBeCloseTo(Math.min(1, 0.6 * KICK.gain));
    expect(fire.jump).toBeCloseTo(0.4);

    const cooling = glassLensKick(0.6, 0.2, 0, 9.8, 10, 0.1);
    expect(cooling.fire).toBe(false);
  });

  it("tracks the beat with a fast rise and a slower fall", () => {
    const rise = glassLensKick(1, 0, 0, -1, 0, 0.1);
    expect(rise.kickSm).toBeCloseTo(1);

    const fall = glassLensKick(0, 1, 1, -1, 0, 0.1);
    expect(fall.kickSm).toBeCloseTo(1 - Math.min(1, 0.1 * KICK_SMOOTH.fall));
  });
});

describe("GlassLens pulse", () => {
  it("decays the target and eases the swell toward it", () => {
    const first = glassLensPulse(0.8, 0, 0.1);
    expect(first.target).toBeCloseTo(0.8 * Math.exp(-0.1 * PULSE.decay));
    expect(first.pulse).toBeCloseTo(
      first.target * Math.min(1, 0.1 * PULSE.attack),
    );

    const rest = glassLensPulse(0, 0.5, 10);
    expect(rest.target).toBe(0);
    expect(rest.pulse).toBe(0);
  });

  it("sanitises bad targets and deltas", () => {
    const pulse = glassLensPulse(Number.NaN, 0.2, Number.NaN);
    expect(pulse.target).toBe(0);
    expect(pulse.pulse).toBeCloseTo(0.2);
  });
});

describe("GlassLens sway", () => {
  it("keeps the pen's idle pose in silence", () => {
    const silent = glassLensSway(0, 0, 0, 0);

    expect(silent.spin).toBeCloseTo(MOTION.spinBase);
    expect(silent.rotationX).toBeCloseTo(MOTION.swayBase);
    expect(silent.rotationZ).toBe(0);
    expect(silent.scale).toBe(1);
    expect(silent.positionY).toBe(0);
  });

  it("scales the spin and swell with energy and the kick pulse", () => {
    const loud = glassLensSway(0, 1, 1, 1);

    expect(loud.spin).toBeCloseTo(MOTION.spinBase + MOTION.spinGain);
    expect(loud.rotationX).toBeCloseTo(MOTION.swayBase + MOTION.swayGain);
    expect(loud.scale).toBeCloseTo(1 + MOTION.kickSwell);
  });

  it("sways and floats over time", () => {
    const t = 0.25;
    const pose = glassLensSway(t, 0, 0, 0);

    expect(pose.rotationX).toBeCloseTo(
      Math.cos(t * MOTION.swayTimeBase) * MOTION.swayBase,
    );
    expect(pose.rotationZ).toBeCloseTo(
      Math.sin(t * MOTION.rollTime) * MOTION.rollBase,
    );
    expect(pose.positionY).toBeCloseTo(
      Math.sin(t * MOTION.floatTime) * MOTION.floatBase,
    );
  });
});

describe("GlassLens rig constants", () => {
  it("uses the source's rounded glass block and photo scale", () => {
    expect(GLASS_BOX).toEqual({
      size: 2.1,
      depth: 1.0,
      segments: 32,
      radius: 0.25,
    });
    expect(PHOTO_SCALE).toBe(1.2);
  });

  it("keeps the physical glass material parameters", () => {
    expect(GLASS_MATERIAL.transmission).toBe(1);
    expect(GLASS_MATERIAL.ior).toBe(1.5);
    expect(GLASS_MATERIAL.thickness).toBe(1.2);
    expect(GLASS_MATERIAL.roughness).toBe(0);
    expect(GLASS_MATERIAL.clearcoat).toBe(1);
  });

  it("keeps the camera, clear alpha and drive thresholds", () => {
    expect(CAMERA.fov).toBe(45);
    expect(CAMERA.positionZ).toBe(3);
    expect(CLEAR_ALPHA).toBe(0);
    expect(ENERGY.tau).toBe(3);
    expect(KICK.jump).toBe(0.08);
    expect(KICK.cooldown).toBe(0.6);
    expect(PULSE.decay).toBe(2.5);
    expect(KICK_SMOOTH.rise).toBe(12);
  });
});
