// Ultra HDR: the three.js UltraHDR-loader example recreated as a standalone
// native scene - no XML, no P9Scene node graph.
//
// A mirror torus knot (metalness 1, roughness 0) reflects an Ultra HDR equirect
// environment. Music drives it: kicks fire a quick scale swell (attack
// detection), overall energy sets the spin rate (smoothed band mean, so it
// swells instead of juddering), and busy music morphs the knot's geometry
// toward a figure-8 - each kick flexes it a little further along the morph,
// calm music lets it fall back to the knot.
//
// modV port of Point10/src/scenes/UltraHDR.js. The worker has no DOM, so the
// Point10 CameraFeed webcam augmentation (RectAreaLight screens, CubeCamera,
// canvas colour sampling) is dropped: without a camera the upstream scene keeps
// exactly this UltraHDR-image look, which is what is ported here. The scene
// renders through the adapter's own renderer/camera and composites
// transparently - the HDR map is the environment (reflections) only, never the
// background, so empty pixels stay transparent.

import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";
import { readAssetBuffer } from "../shared/assets.js";
import { UltraHDRLoader } from "three/addons/loaders/UltraHDRLoader.js";
import hdrUrl from "../textures/spruit_sunrise_2k.hdr.jpg?url";

// The three.js UltraHDR example's TorusKnotGeometry(1, 0.4, 128, 128, 1, 3).
export const KNOT = {
  radius: 1,
  tube: 0.4,
  tubularSegments: 128,
  radialSegments: 128,
  p: 1,
  q: 3,
};

// Screen1 / ultra_hdr example camera: (0, 0, 7) looking at the origin.
export const CAMERA_POSITION = [0, 0, 7];

// Kick pulse: fire on the beat envelope's attack (a jump), gate re-fires with
// a cooldown, then let the target decay while a fast one-pole follows it.
export const PULSE = {
  jumpThreshold: 0.08,
  cooldown: 0.6,
  targetGain: 1.2,
  targetDecay: 2.5,
  rise: 8,
  scaleAmount: 0.18,
};

// Smoothed band mean: a fast attack on top of the 3 Hz energy one-pole.
export const ENERGY = { rate: 3, attack: 1.5 };

// Morph influence: energy is gained up, then eased so calm music barely moves
// the shape and busy music surges toward the figure-8.
export const MORPH = { energyGain: 1.6, exponent: 1.2 };

// Knot spin, in radians per second: a slow idle plus the smoothed energy.
export const SPIN = { base: 0.3, energy: 4 };

// Keeps the knot visible before the environment texture lands (and from black
// in node smoke runs); the mirror reflection takes over once loaded.
export const HEMI = { sky: 0xffffff, ground: 0x202020, intensity: 0.6 };

// The adapter's frame clock already clamps to 0.1 s; clamp again so a direct
// update() call cannot leap the pulse/morph phases.
export const MAX_DT = 0.1;

const finiteOr = (value, fallback) =>
  typeof value === "number" && Number.isFinite(value) ? value : fallback;
const nonNegative = (value) => Math.max(0, finiteOr(value, 0));
const clamp01 = (value) => Math.min(1, Math.max(0, finiteOr(value, 0)));
const clampDt = (dt) => Math.min(MAX_DT, Math.max(0, finiteOr(dt, 0)));

export function createDriveState() {
  return {
    kick: 0,
    pulse: 0,
    pulseTarget: 0,
    lastPulse: -1,
    energy: 0,
  };
}

// One smoothed-energy step: the mean bass/mid/high level through the 3 Hz
// one-pole with the scene's fast attack factor.
export function stepEnergy(bass, mid, high, previousEnergy, dt) {
  const step = clampDt(dt);
  const previous = nonNegative(previousEnergy);
  const target = Math.min(
    1,
    (nonNegative(bass) + nonNegative(mid) + nonNegative(high)) / 3,
  );
  const beta = 1 - Math.exp(-step * ENERGY.rate);

  return previous + (target - previous) * Math.min(1, beta * ENERGY.attack);
}

// One kick-pulse step. `sceneTime` is the scene's accumulated time, used for
// the cooldown; `state` carries the previous kick/pulse/target/lastPulse.
export function stepKick(state, beat, sceneTime, dt) {
  const step = clampDt(dt);
  const kick = clamp01(beat);
  const previousKick = clamp01(state && state.kick);
  const previousPulse = nonNegative(state && state.pulse);
  const time = finiteOr(sceneTime, finiteOr(state && state.lastPulse, -1));
  let lastPulse = finiteOr(state && state.lastPulse, -1);
  let pulseTarget = nonNegative(state && state.pulseTarget);
  const jump = kick - previousKick;

  if (jump > PULSE.jumpThreshold && time - lastPulse > PULSE.cooldown) {
    lastPulse = time;
    pulseTarget = Math.min(1, kick * PULSE.targetGain);
  }

  pulseTarget *= Math.exp(-step * PULSE.targetDecay);
  const pulse =
    previousPulse +
    (pulseTarget - previousPulse) * Math.min(1, step * PULSE.rise);

  return { kick, pulse, pulseTarget, lastPulse };
}

export function morphFromEnergy(energy) {
  return Math.pow(
    Math.min(1, nonNegative(energy) * MORPH.energyGain),
    MORPH.exponent,
  );
}

// Pure frame drive: returns the next state plus the values the scene applies
// (geometry morph influence, this frame's spin delta and scale).
export function driveUltraHDR(state, input) {
  const previous = state || createDriveState();
  const source = input || {};
  const dt = clampDt(source.dt);
  const energy = stepEnergy(
    source.bass,
    source.mid,
    source.high,
    previous.energy,
    dt,
  );
  const kick = stepKick(previous, source.beat, source.time, dt);
  const spinScale = finiteOr(source.spinScale, 1);
  const pulseScale = finiteOr(source.pulseScale, 1);

  return {
    state: {
      ...kick,
      energy,
    },
    energy,
    kick: kick.kick,
    pulse: kick.pulse,
    morph: morphFromEnergy(energy),
    spin: dt * (SPIN.base + energy * SPIN.energy) * spinScale,
    scale: 1 + kick.pulse * PULSE.scaleAmount * pulseScale,
  };
}

// Sweeps a tube along `curve(u)` with the exact vertex loop, order and the
// same ad-hoc orthonormal basis as three's TorusKnotGeometry (P1/P2 ahead,
// B = T x N, N = B x T), so the result is a valid morph target for it - same
// vertex count, matching i-over-j layout, radial (vertex - P1) normals.
export function sweptTube(curve, tubularSegments, radialSegments, tube) {
  const positions = [];
  const normals = [];
  const v = new THREE.Vector3();
  const N = new THREE.Vector3();
  const B = new THREE.Vector3();
  const T = new THREE.Vector3();
  const P1 = new THREE.Vector3();
  const P2 = new THREE.Vector3();

  for (let i = 0; i <= tubularSegments; i++) {
    const u = (i / tubularSegments) * Math.PI * 2;
    curve(u, P1);
    curve(u + 0.01, P2);
    T.subVectors(P2, P1);
    N.addVectors(P2, P1);
    B.crossVectors(T, N);
    N.crossVectors(B, T);
    B.normalize();
    N.normalize();

    for (let j = 0; j <= radialSegments; j++) {
      const a = (j / radialSegments) * Math.PI * 2;
      const cx = -tube * Math.cos(a);
      const cy = tube * Math.sin(a);
      v.set(
        P1.x + cx * N.x + cy * B.x,
        P1.y + cx * N.y + cy * B.y,
        P1.z + cx * N.z + cy * B.z,
      );
      positions.push(v.x, v.y, v.z);
      const dx = v.x - P1.x;
      const dy = v.y - P1.y;
      const dz = v.z - P1.z;
      const l = Math.hypot(dx, dy, dz) || 1;
      normals.push(dx / l, dy / l, dz / l);
    }
  }

  return { positions, normals };
}

// Figure-8: Gerono lemniscate in the xy-plane (reads as an infinity sign from
// the front) with a sine z-lift so the two arms cross at z = +/-0.85 instead of
// colliding tube-through-tube at the waist.
export function eightCurve(u, out) {
  out.x = 1.15 * Math.cos(u);
  out.y = 0.65 * Math.sin(u) * Math.cos(u);
  out.z = 0.85 * Math.sin(u);
}

export function buildUltraHDRKnotGeometry() {
  const geometry = new THREE.TorusKnotGeometry(
    KNOT.radius,
    KNOT.tube,
    KNOT.tubularSegments,
    KNOT.radialSegments,
    KNOT.p,
    KNOT.q,
  );
  const eight = sweptTube(
    eightCurve,
    KNOT.tubularSegments,
    KNOT.radialSegments,
    KNOT.tube,
  );

  geometry.morphAttributes.position = [
    new THREE.Float32BufferAttribute(eight.positions, 3),
  ];
  geometry.morphAttributes.normal = [
    new THREE.Float32BufferAttribute(eight.normals, 3),
  ];

  return geometry;
}

// UltraHDRLoader's XMP path uses DOMParser and a DOM canvas to unpack the gain
// map, so it cannot run in modV's worker. Try it only where DOMParser exists;
// otherwise the caller falls back to the JPEG's SDR base.
function loadUltraHDRTexture(buffer) {
  if (typeof DOMParser === "undefined") {
    return Promise.reject(
      new Error(
        "UltraHDRLoader requires DOM APIs (DOMParser/canvas) and is not worker-safe",
      ),
    );
  }

  const loader = new UltraHDRLoader();
  loader.setDataType(THREE.HalfFloatType);

  return new Promise((resolve, reject) => {
    try {
      loader.parse(buffer, (texture) => resolve(texture));
    } catch (error) {
      reject(error);
    }
  });
}

// Worker-safe fallback: decode the Ultra HDR JPEG's SDR base rendition with
// createImageBitmap (no DOM) and use it as the equirect environment. The knot
// still mirrors the sunrise, just without the gain-map HDR headroom.
async function loadBaseImageTexture(buffer) {
  const blob = new Blob([new Uint8Array(buffer)], { type: "image/jpeg" });
  const bitmap = await createImageBitmap(blob);
  const texture = new THREE.Texture(bitmap);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.needsUpdate = true;

  return texture;
}

export class UltraHDR extends BaseScene {
  _init() {
    this.label = "Ultra HDR";
    this.role = "standalone";
    this.p9 = null;
    this._driveState = createDriveState();
    this._kick = 0;
    this._pulse = 0;
    this._pulseTarget = 0;
    this._lastPulse = -1;
    this._smEnergy = 0;
    this._envApplied = false;
    this.envTexture = null;

    // Module inputs (sceneProps.js owns the sliders): all neutral by default.
    this.spinScale = 1;
    this.pulseScale = 1;
    this.brightnessScale = 1;

    if (this.renderer) {
      // The example's look depends on ACES filmic - env values > 1 would clip
      // to white with the default NoToneMapping. Each Point10 scene owns its
      // renderer (see adapter.js), so this does not leak to other scenes.
      this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
      this.renderer.setClearColor(0x000000, 0);
    }

    this.scene = new THREE.Scene();
    this.scene.add(this.group);

    this.knot = new THREE.Mesh(
      buildUltraHDRKnotGeometry(),
      new THREE.MeshStandardMaterial({
        metalness: 1,
        roughness: 0,
      }),
    );
    this.knot.morphTargetInfluences = [0];
    this.group.add(this.knot);

    this._hemi = new THREE.HemisphereLight(
      HEMI.sky,
      HEMI.ground,
      HEMI.intensity,
    );
    this.scene.add(this._hemi);

    this._loadEnvironment();

    this.size = { w: 2, h: 2 };
    if (this.renderer) {
      const s = this.renderer.getSize(new THREE.Vector2());
      this.size = { w: s.x, h: s.y };
    }
  }

  async _loadEnvironment() {
    let buffer;

    try {
      // readAssetBuffer reads the Vite ?url asset through fs on file:// and
      // fetch in dev, the same worker-safe path the other Point10 scenes use.
      buffer = await readAssetBuffer(hdrUrl);
    } catch (error) {
      console.warn(
        "UltraHDR environment asset failed; keeping the hemisphere-lit knot:",
        error,
      );
      return;
    }

    let texture = null;

    try {
      texture = await loadUltraHDRTexture(buffer);
    } catch (error) {
      console.warn(
        "UltraHDR gain map unavailable in the worker; using the SDR base image:",
        error,
      );
    }

    if (!texture) {
      try {
        texture = await loadBaseImageTexture(buffer);
      } catch (error) {
        console.warn(
          "UltraHDR environment decode failed; keeping the hemisphere-lit knot:",
          error,
        );
        return;
      }
    }

    texture.mapping = THREE.EquirectangularReflectionMapping;
    this.envTexture = texture;
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init builds against a null engine, so recreate P9Audio with the real
    // audio bridge or every band reads 0.
    this.p9 = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9?.update(dt);

    const bands = this.p9?.bands ?? [0, 0, 0];
    const drive = driveUltraHDR(this._driveState, {
      bass: bands[0],
      mid: bands[1],
      high: bands[2],
      beat: this.p9 ? this.p9.beat() : 0,
      dt,
      time: this.time,
      spinScale: this.spinScale,
      pulseScale: this.pulseScale,
    });

    this._driveState = drive.state;
    this._kick = drive.state.kick;
    this._pulse = drive.state.pulse;
    this._pulseTarget = drive.state.pulseTarget;
    this._lastPulse = drive.state.lastPulse;
    this._smEnergy = drive.energy;

    if (this.knot) {
      this.knot.morphTargetInfluences[0] = drive.morph;
      this.knot.rotation.y += drive.spin;
      this.knot.scale.setScalar(drive.scale);
    }

    if (this.scene) this.scene.environmentIntensity = this.brightnessScale;
    if (this._hemi) {
      this._hemi.intensity = HEMI.intensity * this.brightnessScale;
    }
  }

  render() {
    if (!this.renderer || !this.scene) return;

    if (this.envTexture && !this._envApplied) {
      this.scene.environment = this.envTexture;
      this.scene.environmentIntensity = this.brightnessScale;
      this._envApplied = true;
    }

    this.camera.position.set(
      CAMERA_POSITION[0],
      CAMERA_POSITION[1],
      CAMERA_POSITION[2],
    );
    this.camera.lookAt(0, 0, 0);
    this.renderer.render(this.scene, this.camera);
  }

  resize(w, h) {
    this.size = { w, h };
    if (this.camera) {
      this.camera.aspect = w / Math.max(1, h);
      this.camera.updateProjectionMatrix();
    }
  }
}
