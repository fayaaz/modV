// Glass Lens: Sabo Sugi's "3D Glass Photo Lens" (CodePen YPWPoRJ) as a music
// visual - a thick glass RoundedBoxGeometry (physical transmission/ior/
// thickness) with a PCB photo plane suspended inside it. The glass bevels
// reflect the mountains photo environment; the Chromatic Wave shader is the
// live backdrop rendered in this same scene, so the physical material's
// transmission pass re-renders it through the glass instead of showing black.
//
// modV port of Point10/src/scenes/GlassLens.js. The worker has no document, so
// three's TextureLoader is replaced with the Vite ?url assets loaded through
// ../shared/textures.js, and the Point10 CameraFeed webcam augmentation is
// dropped (the adapter does not pass webcam feeds yet) - the static PCB photo
// is always the fallback.
//
// Audio: smoothed band energy sets the idle spin and sway rate, the beat
// envelope's sharp upward jumps fire a cooldown-limited scale swell, and the
// beat is low-passed into a slight roll + vertical float.

import * as THREE from "three";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../p10Audio.js";
import { ChromaticWave } from "./ChromaticWave.js";
import { loadTexture } from "../shared/textures.js";
import pcbUrl from "../textures/pcb.jpg?url";
import mountainsUrl from "../textures/mountains.png?url";

export const GLASS_BOX = { size: 2.1, depth: 1.0, segments: 32, radius: 0.25 };

export const PHOTO_SCALE = 1.2;

export const CAMERA = { fov: 45, positionZ: 3 };

export const GLASS_MATERIAL = {
  transmission: 1,
  ior: 1.5,
  thickness: 1.2,
  roughness: 0,
  metalness: 0,
  specularIntensity: 1,
  envMapIntensity: 1,
  clearcoat: 1,
  clearcoatRoughness: 0,
};

// Clear1 has no opaque fill of its own; the wave quad is the backdrop and the
// scene still clears transparent so a missing frame composites cleanly.
export const CLEAR_ALPHA = 0;

export const ENERGY = { tau: 3, gain: 1.5 };

// Kick attack detection: a rise above the previous beat envelope, at least
// 0.6 s after the previous pulse, swells the glass by up to 12%.
export const KICK = { jump: 0.08, cooldown: 0.6, gain: 1.2 };

export const KICK_SMOOTH = { rise: 12, fall: 2.5 };

export const PULSE = { decay: 2.5, attack: 8 };

// Motion constants from the pen/source: idle spin 0.5 rad/s rising to ~2 with
// band energy, an x-sway that breathes with the energy, a slow z-roll and a
// vertical float driven by the smoothed beat.
export const MOTION = {
  spinBase: 0.5,
  spinGain: 1.5,
  swayTimeBase: 0.4,
  swayTimeGain: 0.5,
  swayBase: 0.2,
  swayGain: 0.15,
  rollTime: 0.3 * 0.7,
  rollBase: 0.1,
  rollGain: 0.2,
  floatTime: 1.3,
  floatBase: 0.08,
  floatGain: 0.15,
  kickSwell: 0.12,
};

const finiteOr = (value, fallback) =>
  typeof value === "number" && Number.isFinite(value) ? value : fallback;

const nonNegative = (value) => Math.max(0, value);

const clamp01 = (value) => Math.min(1, Math.max(0, value));

// One-pole smoothing of the three-band mean (the source's beta * 1.5).
export function glassLensEnergy(energy, bass, mid, high, dt) {
  const current = clamp01(finiteOr(energy, 0));
  const level = clamp01(
    (finiteOr(bass, 0) + finiteOr(mid, 0) + finiteOr(high, 0)) / 3,
  );
  const step = nonNegative(finiteOr(dt, 0));
  const beta = 1 - Math.exp(-step * ENERGY.tau);

  return current + (level - current) * Math.min(1, beta * ENERGY.gain);
}

// Beat-envelope attack detection plus the source's asymmetric one-pole: the
// smoothed kick rises fast (tau ~0.08 s) and falls at ~0.4 s, so the roll and
// float breathe instead of jerking on beat()'s sharp snap.
export function glassLensKick(kick, prevKick, kickSm, lastPulse, time, dt) {
  const value = clamp01(finiteOr(kick, 0));
  const previous = clamp01(finiteOr(prevKick, 0));
  const smoothed = clamp01(finiteOr(kickSm, 0));
  const last = finiteOr(lastPulse, -Infinity);
  const now = finiteOr(time, 0);
  const step = nonNegative(finiteOr(dt, 0));
  const jump = value - previous;
  const fire = jump > KICK.jump && now - last > KICK.cooldown;
  const target = fire ? Math.min(1, value * KICK.gain) : null;
  const rate = value > smoothed ? KICK_SMOOTH.rise : KICK_SMOOTH.fall;

  return {
    fire,
    jump,
    target,
    kick: value,
    kickSm: smoothed + (value - smoothed) * Math.min(1, step * rate),
  };
}

// The pulse target decays exponentially and the displayed swell eases toward
// it, so a kick is a quick swell that settles instead of a step.
export function glassLensPulse(pulseTarget, pulse, dt) {
  const step = nonNegative(finiteOr(dt, 0));
  const target =
    clamp01(finiteOr(pulseTarget, 0)) * Math.exp(-step * PULSE.decay);
  const current = clamp01(finiteOr(pulse, 0));

  return {
    target,
    pulse: current + (target - current) * Math.min(1, step * PULSE.attack),
  };
}

// Pure motion pose for the rotated glass group.
export function glassLensSway(time, energy, kickSm, pulse) {
  const t = finiteOr(time, 0);
  const e = clamp01(finiteOr(energy, 0));
  const kick = clamp01(finiteOr(kickSm, 0));
  const swell = clamp01(finiteOr(pulse, 0));

  return {
    spin: MOTION.spinBase + e * MOTION.spinGain,
    rotationX:
      Math.cos(t * (MOTION.swayTimeBase + e * MOTION.swayTimeGain)) *
      (MOTION.swayBase + e * MOTION.swayGain),
    rotationZ:
      Math.sin(t * MOTION.rollTime) *
      (MOTION.rollBase + kick * MOTION.rollGain),
    scale: 1 + swell * MOTION.kickSwell,
    positionY:
      Math.sin(t * MOTION.floatTime) *
      (MOTION.floatBase + kick * MOTION.floatGain),
  };
}

export class GlassLens extends BaseScene {
  _init() {
    this.label = "Glass Lens";
    this.p9 = null;
    this._kick = 0;
    this._pulseTarget = 0;
    this._pulse = 0;
    this._lastPulse = -1;
    this._smEnergy = 0;
    this._kickSm = 0;

    this.spinScale = 1;
    this.pulseScale = 1;
    this.brightnessScale = 1;

    if (this.renderer) {
      // ACES filmic: the environment and light intensities go above 1 and
      // would clip to white with the default NoToneMapping.
      this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
      this.renderer.setClearColor(0x000000, CLEAR_ALPHA);
    }

    this.scene = new THREE.Scene();
    this.scene.add(this.group);

    // The live backdrop: the Chromatic Wave full-screen shader. It is added to
    // this scene (renderOrder -1, first opaque) so the glass transmission pass
    // sees it; a separate pre-pass would leave everything behind the glass
    // black. Its raw-NDC quad would otherwise occlude the glass, so it must
    // neither test nor write depth.
    this.bg = new ChromaticWave(this.renderer, this.camera);
    this.bg.mesh.renderOrder = -1;
    const bgMat = this.bg.mesh.material;
    bgMat.depthWrite = false;
    bgMat.depthTest = false;
    this.scene.add(this.bg.mesh);

    // Bevel reflections: a second quad sharing the backdrop material (so both
    // always show the same audio state) captured by a CubeCamera and PMREM'd.
    // The mountains photo below stays the fallback environment until the live
    // capture exists.
    this.envWave = new THREE.Mesh(this.bg.mesh.geometry, bgMat);
    this.envWave.frustumCulled = false;
    this.envScene = new THREE.Scene();
    this.envScene.add(this.envWave);
    this.cubeRT = new THREE.WebGLCubeRenderTarget(512);
    this.cubeCam = new THREE.CubeCamera(0.1, 10, this.cubeRT);
    this._pmrem = this.renderer
      ? new THREE.PMREMGenerator(this.renderer)
      : null;
    this._envRT = null;
    this._envTick = 0;

    // Scene rig: the lights keep the bevels sparkling on top of the env; the
    // glass only reflects, it never receives shadows.
    this.scene.add(new THREE.AmbientLight(0xffffff, 1.0));
    const backLight = new THREE.DirectionalLight(0xffffff, 3.0);
    backLight.position.set(-5, 2, -10);
    this.scene.add(backLight);
    const topLight = new THREE.DirectionalLight(0xffffff, 2.0);
    topLight.position.set(0, 10, 0);
    this.scene.add(topLight);
    const frontLight = new THREE.DirectionalLight(0xffffff, 1.0);
    frontLight.position.set(0, 2, 10);
    this.scene.add(frontLight);

    // Photo inside the glass (the PCB).
    this._photoAspect = 1;
    this.photoMesh = new THREE.Mesh(
      new THREE.PlaneGeometry(1, 1),
      new THREE.MeshStandardMaterial({
        side: THREE.DoubleSide,
        color: 0xffffff,
        roughness: 0.2,
        metalness: 0.1,
        alphaTest: 0.5,
      }),
    );
    this.photoMesh.renderOrder = 0;
    this.photoMat = this.photoMesh.material;
    this.group.add(this.photoMesh);

    // Thick glass block (the pen's Square + RoundedBoxGeometry default).
    this.glassMat = new THREE.MeshPhysicalMaterial({
      color: 0xffffff,
      transmission: GLASS_MATERIAL.transmission,
      opacity: 1.0,
      metalness: GLASS_MATERIAL.metalness,
      roughness: GLASS_MATERIAL.roughness,
      ior: GLASS_MATERIAL.ior,
      thickness: GLASS_MATERIAL.thickness,
      attenuationColor: 0xffffff,
      attenuationDistance: 9999.0,
      specularIntensity: GLASS_MATERIAL.specularIntensity,
      envMapIntensity: GLASS_MATERIAL.envMapIntensity,
      clearcoat: GLASS_MATERIAL.clearcoat,
      clearcoatRoughness: GLASS_MATERIAL.clearcoatRoughness,
      transparent: true,
      side: THREE.DoubleSide,
      depthWrite: false,
    });
    this.glassMesh = new THREE.Mesh(
      new RoundedBoxGeometry(
        GLASS_BOX.size,
        GLASS_BOX.size,
        GLASS_BOX.depth,
        GLASS_BOX.segments,
        GLASS_BOX.radius,
      ),
      this.glassMat,
    );
    this.glassMesh.renderOrder = 1;
    this.group.add(this.glassMesh);

    this._loadPCB();
    this._loadMountains();
  }

  _loadPCB() {
    loadTexture(pcbUrl)
      .then((texture) => {
        texture.colorSpace = THREE.SRGBColorSpace;
        this._pcbTex = texture;
        this._applyPhoto(
          texture,
          texture.image?.width ?? 1,
          texture.image?.height ?? 1,
        );
      })
      .catch((error) => {
        console.warn("GlassLens PCB texture failed:", error);
      });
  }

  _loadMountains() {
    loadTexture(mountainsUrl)
      .then((texture) => {
        texture.colorSpace = THREE.SRGBColorSpace;
        texture.mapping = THREE.EquirectangularReflectionMapping;
        this._mountainsTex = texture;
        // The live wave capture replaces this on the next env tick; keep the
        // photo as the fallback environment when the capture is unavailable.
        if (!this._envRT) this.scene.environment = texture;
      })
      .catch((error) => {
        console.warn("GlassLens mountains texture failed:", error);
      });
  }

  _applyPhoto(texture, width, height) {
    this.photoMat.map = texture;
    this.photoMat.needsUpdate = true;
    this._photoAspect = width / Math.max(1, height);
    this._updatePhotoScale();
  }

  _updatePhotoScale() {
    const s = PHOTO_SCALE;
    if (this._photoAspect > 1) {
      this.photoMesh.scale.set(s, s / this._photoAspect, 1);
    } else {
      this.photoMesh.scale.set(s * this._photoAspect, s, 1);
    }
  }

  setAudio(audio) {
    super.setAudio(audio);
    this.p9 = new P9Audio(audio);
    this.bg?.setAudio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.bg?.update(dt, audioData);
    this.p9?.update(dt);
    this._applyBrightness();
    if (!this.p9) return;

    const energy = glassLensEnergy(
      this._smEnergy,
      this.p9.bands[0],
      this.p9.bands[1],
      this.p9.bands[2],
      dt,
    );
    this._smEnergy = energy;

    const kick = glassLensKick(
      this.p9.beat(),
      this._kick,
      this._kickSm,
      this._lastPulse,
      this.time,
      dt,
    );
    if (kick.fire) {
      this._lastPulse = this.time;
      this._pulseTarget = kick.target;
    }
    this._kick = kick.kick;
    this._kickSm = kick.kickSm;

    const pulse = glassLensPulse(this._pulseTarget, this._pulse, dt);
    this._pulseTarget = pulse.target;
    this._pulse = pulse.pulse;

    const sway = glassLensSway(
      this.time * this.spinScale,
      energy,
      this._kickSm,
      this._pulse,
    );
    this.group.rotation.y += dt * sway.spin * this.spinScale;
    this.group.rotation.x = sway.rotationX;
    this.group.rotation.z = sway.rotationZ;
    const s = 1 + (sway.scale - 1) * this.pulseScale;
    this.group.scale.set(s, s, s);
    this.group.position.y = sway.positionY * this.pulseScale;

    this._captureEnv();
  }

  _applyBrightness() {
    if (this.glassMat) this.glassMat.envMapIntensity = this.brightnessScale;
    if (this.photoMat) this.photoMat.color.setScalar(this.brightnessScale);
  }

  _captureEnv() {
    if (!this.renderer || !this._pmrem) return;

    // PMREM.fromCubemap allocates a target per call, so ~20 Hz is plenty for
    // the barely-visible bevel reflection delta.
    this._envTick += 1;
    if (this._envTick % 3 !== 0) return;

    const renderer = this.renderer;
    const previousToneMapping = renderer.toneMapping;
    renderer.toneMapping = THREE.NoToneMapping;
    this.cubeCam.update(renderer, this.envScene);
    renderer.toneMapping = previousToneMapping;

    const next = this._pmrem.fromCubemap(this.cubeRT.texture);
    if (this._envRT) this._envRT.dispose();
    this._envRT = next;
    this.scene.environment = next.texture;
  }

  render() {
    const camera = this.camera;
    if (camera) {
      camera.position.set(0, 0, CAMERA.positionZ);
      camera.lookAt(0, 0, 0);
      if (camera.isPerspectiveCamera) camera.fov = CAMERA.fov;
      camera.updateProjectionMatrix();
    }

    // The wave quad renders inside the scene now; only its resolution uniform
    // needs the per-frame sync its own render() would have done.
    this.bg?._syncResolution();
    this.renderer.render(this.scene, this.camera);
  }
}
