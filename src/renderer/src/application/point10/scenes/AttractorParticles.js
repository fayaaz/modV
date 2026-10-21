// AttractorParticles - port of three.js examples/webgpu_tsl_compute_attractors_particles
// (master, MIT): 2^18 particles orbiting three gravity+spin attractors, computed on the
// GPU. The mouse TransformControls are dropped - the music drives it instead: bass pushes
// maxSpeed up and actuates the spinning strength hard, snare/hi-hat percussion nudges the
// attractor mass exponent (6..8), overall energy drives particleGlobalMassExponent (4..6),
// and the colorA/colorB ramp hue-rotates on bass drops exactly like Ball of Fire. The
// Ball-of-Fire lava ball sits on each attractor with its colour change and bloom, but the
// volumetric fire sim and camera rig are not imported. Requires WebGPU.
//
// modV port notes:
//   - BaseSceneWebGPU replaces the upstream BaseScene: the adapter owns the
//     WebGPURenderer + OffscreenCanvas, so init() no longer builds a renderer or
//     appends a DOM canvas.
//   - the private requestAnimationFrame loop is gone: modV's worker loop calls
//     update(dt, audioData) / render().
//   - the ball texture is a Vite asset read through ../shared/textures.js.
import * as THREE from "three/webgpu";
import {
  float,
  If,
  PI,
  color,
  cos,
  instanceIndex,
  Loop,
  mix,
  mod,
  sin,
  instancedArray,
  Fn,
  uint,
  uniform,
  uniformArray,
  hash,
  vec3,
  vec4,
  hue,
  luminance,
  saturation,
  smoothstep,
  positionLocal,
  mx_noise_float,
  pass,
} from "three/tsl";
import { bloom } from "three/addons/tsl/display/BloomNode.js";
import { BaseSceneWebGPU } from "../BaseSceneWebGPU.js";
import { dropColorPulse, DROP, ROYGBIV_HUES } from "../shared/phraseColor.js";
import { seededRand } from "../shared/random.js";
import { loadTexture } from "../shared/textures.js";
import moonDiffUrl from "../textures/moon_04_diff_1k.jpg?url";

// ponytail: AudioEngine fftSize=2048 @48kHz -> first 5 bins ≈ 0-117Hz (bass).
const BASS_BINS = 5;
const MAX_SPEED_BASE = 8; // particles' max speed at silence
const MAX_SPEED_PER_BASS = 6; // loud bass drives maxSpeed up (8 -> ~14)
const MASS_EXP_MIN = 6; // attractorMassExponent range: 6..8
const MASS_EXP_RANGE = 2;
const PERCUSSION_K = 1.5; // snare/hat energy -> exponent (clamped to 1)
const MASS_EXP_PER_BASS = 0.5; // bass gives the exponent a little extra push
const SPIN_BASE = 2.75; // spinningStrength at silence (the example's default)
const SPIN_PER_BASS = 10; // bass drives the spin hard (2.75 -> ~13)
const SPIN_PER_ENERGY = 2; // overall energy only nudges the spin further
const PARTICLE_MASS_EXP_MIN = 4; // particleGlobalMassExponent range: 4..6 (example default 1e4)
const PARTICLE_MASS_EXP_RANGE = 2;
// Ball-of-Fire lava ball (no fire sim): bloom strength driven by bass, lava ramp hue
// rotated on bass drops. Constants mirror RollingFire's sphere/emissive/bloom.
const BALL_RADIUS = 0.01;
const BLOOM_BASE = 0.12;
const BLOOM_PER_BASS = 3.6;
const FIRE_START = 0xffe68c;
const FIRE_MID = 0xff7305;
const FIRE_END = 0xff0000;
const SEED_A = 1; // colorA's ROYGBIV drop-pulse seed
const SEED_B = 2; // colorB's ROYGBIV drop-pulse seed
const SEED_BALLS = [3, 4, 5]; // the three lava balls' ROYGBIV drop-pulse seeds
// Distinct STARTING hues so the three balls are never all-red at once (red, yellow, indigo).
const BALL_START_HUES = [ROYGBIV_HUES[0], ROYGBIV_HUES[2], ROYGBIV_HUES[5]];

// Pure audio -> uniform mapping (checked in scripts/ap-audio-check.mjs): bass drives
// maxSpeed up and spinningStrength hard, snare/hi-hat percussion nudges the
// attractorMassExponent (6..8) modestly with a little bass assist, overall energy drives
// particleGlobalMassExponent up (4..6) and tops up the spin.
export function audioUniforms(bass, snare, hi, energy) {
  const expoFrac = Math.min(
    1,
    (snare + hi) * PERCUSSION_K + bass * MASS_EXP_PER_BASS,
  );
  return {
    maxSpeed: MAX_SPEED_BASE + bass * MAX_SPEED_PER_BASS,
    attractorMass: Math.pow(10, MASS_EXP_MIN + expoFrac * MASS_EXP_RANGE),
    spinningStrength:
      SPIN_BASE + bass * SPIN_PER_BASS + energy * SPIN_PER_ENERGY,
    particleGlobalMass: Math.pow(
      10,
      PARTICLE_MASS_EXP_MIN + energy * PARTICLE_MASS_EXP_RANGE,
    ),
  };
}

export class AttractorParticles extends BaseSceneWebGPU {
  constructor(renderer, camera, host) {
    super(renderer, camera, host);
    this.label = "Attractor Particles";
    this.category = "Nature";
    // ponytail: same WebGPU capability probe as RollingFire (no top-level await).
    this._available = typeof navigator !== "undefined" && !!navigator.gpu;
    if (this._available) {
      navigator.gpu.requestAdapter().then((adapter) => {
        this._available = !!adapter;
      });
    }
    this._initialized = false;
    this._dt = 0;
    this._bass = 0; // smoothed bass band energy
    this._snare = 0; // smoothed snare band energy (percussion)
    this._hi = 0; // smoothed hi-hat band energy (percussion)
    this._energy = 0; // smoothed overall spectrum energy (drives spin)
    // Two independent ROYGBIV drop-pulses - one per color ramp end - each on its own
    // seeded rand stream, so colorA and colorB flip to DIFFERENT random hues while
    // sharing the exact same drop detector (same algorithm, different seed).
    this._pulseA = { hue: 0, t: 0 };
    this._pulseB = { hue: 0, t: 0 };
    this._randA = this._makeRand(SEED_A);
    this._randB = this._makeRand(SEED_B);
    // Each lava ball runs its own seeded drop-pulse so all three land on different
    // ROYGBIV hues from the same bass drops (same algorithm, different seed). They
    // also START on distinct hues so they're never all the same colour.
    this._pulseBalls = BALL_START_HUES.map((hue) => ({ hue, t: 0 }));
    this._randBalls = SEED_BALLS.map((seed) => this._makeRand(seed));
    // Slow camera orbit: drifts around a sphere centred on the attractor centroid.
    this._camAzimuth = 0;
    this._camPolar = 0;
    this._camRadius = 9.9;
    if (!this._available) {
      console.warn("AttractorParticles: WebGPU not available, scene disabled.");
      return;
    }
  }

  // Seeded [0,1) stream: each call advances the counter, so two channels seeded
  // differently pick different ROYGBIV hues on the same bass drops.
  _makeRand(seed) {
    let n = 0;
    return () => seededRand(seed, n++);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this._dt = dt;
    if (!this._initialized || !this.audio?.analyser || !this.audio.freqMag)
      return;
    const fm = this.audio.freqMag;

    let bass = 0;
    for (let i = 0; i < BASS_BINS; i++) bass += fm[i];
    bass /= BASS_BINS;
    this._bass += (bass - this._bass) * Math.min(1, dt * 8);

    // snares (1-6kHz crack band) and hi-hats (top 15% bins): percussion drives the
    // attractor mass exponent up toward 9.
    const sLo = Math.floor(fm.length * 0.02);
    const sHi = Math.floor(fm.length * 0.13);
    let sn = 0;
    for (let i = sLo; i < sHi; i++) sn += fm[i];
    sn /= sHi - sLo;
    this._snare += (sn - this._snare) * Math.min(1, dt * 12);

    const hiStart = fm.length - Math.floor(fm.length * 0.15);
    let hi = 0;
    for (let i = hiStart; i < fm.length; i++) hi += fm[i];
    hi /= fm.length - hiStart;
    this._hi += (hi - this._hi) * Math.min(1, dt * 10);

    // overall spectrum energy (mean magnitude across all bins) drives the spin.
    let e = 0;
    for (let i = 0; i < fm.length; i++) e += fm[i];
    e /= fm.length;
    this._energy += (e - this._energy) * Math.min(1, dt * 10);

    const u = audioUniforms(this._bass, this._snare, this._hi, this._energy);
    this._uMaxSpeed.value = u.maxSpeed;
    this._uAttractorMass.value = u.attractorMass;
    this._uSpinningStrength.value = u.spinningStrength;
    this._uParticleGlobalMass.value = u.particleGlobalMass;

    // Colour change on bass phrase boundaries (same drop detector as Ball of Fire):
    // TWO independent pulses, one per ramp end, each on its own seeded rand stream.
    // Same algorithm, different seed -> colorA and colorB land on different ROYGBIV
    // hues; each eases smoothly toward its own target.
    this._pulseA = dropColorPulse(
      this._pulseA,
      this._bass,
      dt,
      DROP,
      this._randA,
    );
    this._pulseB = dropColorPulse(
      this._pulseB,
      this._bass,
      dt,
      DROP,
      this._randB,
    );
    this._uHueA.value += this._easeHue(this._uHueA.value, this._pulseA.hue, dt);
    this._uHueB.value += this._easeHue(this._uHueB.value, this._pulseB.hue, dt);

    // Each lava ball: hue rotates on the same bass drops via its own seeded pulse.
    for (let i = 0; i < this._pulseBalls.length; i++) {
      this._pulseBalls[i] = dropColorPulse(
        this._pulseBalls[i],
        this._bass,
        dt,
        DROP,
        this._randBalls[i],
      );
      this._uBallHues[i].value += this._easeHue(
        this._uBallHues[i].value,
        this._pulseBalls[i].hue,
        dt,
      );
    }
  }

  // Shortest-arc eased step toward a target hue (mirrors RollingFire's ease).
  _easeHue(current, target, dt) {
    const dh = Math.atan2(
      Math.sin(target - current),
      Math.cos(target - current),
    );
    return dh * Math.min(1, dt * 1.2);
  }

  // modV: the adapter awaits this before it starts drawing frames.
  async init() {
    if (!this._available) return false;
    await this._initVolume();
    this._initialized = true;
    return true;
  }

  // modV's worker loop drives the frame (update(dt) then render()) - the
  // upstream private requestAnimationFrame loop is gone.
  render() {
    if (!this._initialized) return;
    return this._animate();
  }

  get ready() {
    return this._initialized;
  }

  get available() {
    return this._available;
  }

  resize(w, h) {
    if (!this._initialized) return;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  async _initVolume() {
    // modV: the adapter built the WebGPURenderer around an OffscreenCanvas
    // (no DOM in the worker). The backend still initializes asynchronously
    // (adapter+device) - rendering before that throws every frame, which is
    // why init() awaits this.
    const renderer = this.renderer;
    await renderer.init();
    const width = this.host.width;
    const height = this.host.height;
    renderer.setPixelRatio(1);
    renderer.setSize(width, height, false);
    this.canvas = renderer.domElement;

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x000000);
    this.scene = scene;

    const camera = new THREE.PerspectiveCamera(25, width / height, 0.1, 100);
    camera.position.set(3, 5, 8);
    camera.lookAt(0, 0, 0);
    this.camera = camera;

    // ---- attractors (static; TransformControls from the example are dropped) -----
    const attractorsPositions = uniformArray([
      new THREE.Vector3(-1, 0, 0),
      new THREE.Vector3(1, 0, -0.5),
      new THREE.Vector3(0, 0.5, 1),
    ]);
    // Camera orbit centre = the centroid of the three attractors. The camera starts
    // at (3,5,8) which is ~9.9 away, so the orbit radius matches that starting view.
    this._camCenter = new THREE.Vector3();
    for (const p of attractorsPositions.array) this._camCenter.add(p);
    this._camCenter.multiplyScalar(1 / attractorsPositions.array.length);
    // seed azimuth/polar from the starting camera position so there's no jump.
    this._camRadius = this.camera.position.distanceTo(this._camCenter);
    this._camAzimuth = Math.atan2(
      this.camera.position.z - this._camCenter.z,
      this.camera.position.x - this._camCenter.x,
    );
    this._camPolar = Math.acos(
      (this.camera.position.y - this._camCenter.y) / this._camRadius,
    );
    const attractorsRotationAxes = uniformArray([
      new THREE.Vector3(0, 1, 0),
      new THREE.Vector3(0, 1, 0),
      new THREE.Vector3(1, 0, -0.5).normalize(),
    ]);
    const attractorsLength = uniform(attractorsPositions.array.length, "uint");

    // ---- Ball-of-Fire lava balls on the attractor points (colour + bloom only) ---
    // No volumetric fire, no camera rig - just the sphere's lava emissive ramp, its
    // bass-driven hue rotation and the bloom that makes it glow.
    const uTime = uniform(0);
    const uBassGlow = uniform(0);
    const uFireStartColor = uniform(new THREE.Color(FIRE_START));
    const uFireMidColor = uniform(new THREE.Color(FIRE_MID));
    const uFireEndColor = uniform(new THREE.Color(FIRE_END));

    const fireRamp = Fn(([t]) => {
      const c = vec3(0).toVar();
      c.assign(
        mix(vec3(0.0, 0.0, 0.0), uFireEndColor, smoothstep(0.05, 0.35, t)),
      );
      c.assign(mix(c, uFireMidColor, smoothstep(0.35, 0.65, t)));
      c.assign(mix(c, uFireStartColor, smoothstep(0.65, 1.0, t)));
      return c;
    });
    // Same luminance-equalized ROYGBIV rotation as Ball of Fire.
    const fireHue = (c, adjustment) => {
      const rotated = hue(c, adjustment);
      const green = hue(c, ROYGBIV_HUES[4]);
      return rotated.mul(luminance(green).div(luminance(rotated).max(0.001)));
    };

    const ballGeometry = new THREE.SphereGeometry(BALL_RADIUS, 64, 40);
    const ballHues = [];
    for (let i = 0; i < attractorsPositions.array.length; i++) {
      const ballHue = uniform(BALL_START_HUES[i]);
      ballHues.push(ballHue);
      // per-ball material so each ball's hue rotates independently on its own seed.
      // ponytail: the three materials are structurally identical, so three.js gives
      // them the SAME material cache key and the WebGPU backend reuses ONE built
      // node-builder state - all three balls then read the same (last-written)
      // hue uniform. Splitting customProgramCacheKey per ball forces a separate
      // builder/uniform buffer per ball.
      const ballMaterial = new THREE.MeshStandardMaterial({
        color: 0x000000,
        roughness: 1.0,
        metalness: 0.0,
      });
      ballMaterial.customProgramCacheKey = () => `ball${i}`;
      loadTexture(moonDiffUrl)
        .then((tex) => {
          tex.colorSpace = THREE.SRGBColorSpace;
          ballMaterial.map = tex;
          ballMaterial.needsUpdate = true;
        })
        .catch((error) => {
          console.warn(
            "AttractorParticles: ball texture failed to load:",
            error,
          );
        });
      // ponytail: emissive is RollingFire's sphere ramp minus the fire-sim factors
      // (temp/density/intensity folded into the 3.5 constant); bloom lights it up.
      ballMaterial.emissiveNode = Fn(() => {
        const p = positionLocal.mul(0.5);
        const flow = vec3(0.0, uTime.negate(), 0.0);
        const n1 = mx_noise_float(p.add(flow)).mul(0.5).add(0.5);
        const p2 = p.mul(2.0).sub(flow.mul(1.5));
        const n2 = mx_noise_float(p2.add(vec3(n1.mul(0.4))))
          .mul(0.5)
          .add(0.5);
        const p3 = p.mul(4.0).add(flow.mul(2.5));
        const n3 = mx_noise_float(p3).mul(0.5).add(0.5);
        const noiseVal = n1.mul(0.5).add(n2.mul(0.35)).add(n3.mul(0.15));
        const lavaT = noiseVal.pow(2.5).clamp(0.0, 1.0);
        const fireColor = fireRamp(lavaT.add(0.1));
        const coloredFire = fireHue(
          saturation(fireColor, uniform(1.1)),
          ballHue,
        );
        return coloredFire.mul(3.5).mul(float(1.5).add(uBassGlow.mul(5.3)));
      })();
      const ball = new THREE.Mesh(ballGeometry, ballMaterial);
      ball.position.copy(attractorsPositions.array[i]);
      scene.add(ball);
    }
    // The balls' moon texture needs light to read (RollingFire's ambient+directional).
    scene.add(new THREE.AmbientLight("#ffffff", 0.5));
    const directionalLight = new THREE.DirectionalLight("#ffffff", 1.5);
    directionalLight.position.set(4, 2, 0);
    scene.add(directionalLight);

    // ---- particles --------------------------------------------------------------
    const count = Math.pow(2, 18);
    const material = new THREE.SpriteNodeMaterial({
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    const attractorMass = uniform(Number(`1e${7}`));
    const particleGlobalMass = uniform(Number(`1e${4}`));
    const timeScale = uniform(1);
    const spinningStrength = uniform(SPIN_BASE);
    const maxSpeed = uniform(MAX_SPEED_BASE);
    const gravityConstant = 6.67e-11;
    const velocityDamping = uniform(0.1);
    const scale = uniform(0.008);
    const boundHalfExtent = uniform(8);
    const colorA = uniform(color("#5900ff"));
    const colorB = uniform(color("#ffa575"));
    const uHueA = uniform(0);
    const uHueB = uniform(0);

    const positionBuffer = instancedArray(count, "vec3");
    const velocityBuffer = instancedArray(count, "vec3");

    const sphericalToVec3 = Fn(([phi, theta]) => {
      const sinPhiRadius = sin(phi);
      return vec3(
        sinPhiRadius.mul(sin(theta)),
        cos(phi),
        sinPhiRadius.mul(cos(theta)),
      );
    });

    // init compute: seed the particle field once (deterministic per instance).
    const init = Fn(() => {
      const position = positionBuffer.element(instanceIndex);
      const velocity = velocityBuffer.element(instanceIndex);
      const basePosition = vec3(
        hash(instanceIndex.add(uint(Math.random() * 0xffffff))),
        hash(instanceIndex.add(uint(Math.random() * 0xffffff))),
        hash(instanceIndex.add(uint(Math.random() * 0xffffff))),
      )
        .sub(0.5)
        .mul(vec3(5, 0.2, 5));
      position.assign(basePosition);
      const phi = hash(instanceIndex.add(uint(Math.random() * 0xffffff)))
        .mul(PI)
        .mul(2);
      const theta = hash(instanceIndex.add(uint(Math.random() * 0xffffff))).mul(
        PI,
      );
      const baseVelocity = sphericalToVec3(phi, theta).mul(0.05);
      velocity.assign(baseVelocity);
    });
    const initCompute = init().compute(count);
    renderer.compute(initCompute);

    // update compute: gravity + spin toward each attractor, clamped by maxSpeed.
    const particleMassMultiplier = hash(
      instanceIndex.add(uint(Math.random() * 0xffffff)),
    )
      .remap(0.25, 1)
      .toVar();
    const particleMass = particleMassMultiplier.mul(particleGlobalMass).toVar();
    const update = Fn(() => {
      const delta = float(1 / 60)
        .mul(timeScale)
        .toVar();
      const position = positionBuffer.element(instanceIndex);
      const velocity = velocityBuffer.element(instanceIndex);

      const force = vec3(0).toVar();
      Loop(attractorsLength, ({ i }) => {
        const attractorPosition = attractorsPositions.element(i);
        const attractorRotationAxis = attractorsRotationAxes.element(i);
        const toAttractor = attractorPosition.sub(position);
        const distance = toAttractor.length();
        const direction = toAttractor.normalize();

        const gravityStrength = attractorMass
          .mul(particleMass)
          .mul(gravityConstant)
          .div(distance.pow(2))
          .toVar();
        const gravityForce = direction.mul(gravityStrength);
        force.addAssign(gravityForce);

        const spinningForce = attractorRotationAxis
          .mul(gravityStrength)
          .mul(spinningStrength);
        const spinningVelocity = spinningForce.cross(toAttractor);
        force.addAssign(spinningVelocity);
      });

      velocity.addAssign(force.mul(delta));
      const speed = velocity.length();
      If(speed.greaterThan(maxSpeed), () => {
        velocity.assign(velocity.normalize().mul(maxSpeed));
      });
      velocity.mulAssign(velocityDamping.oneMinus());

      position.addAssign(velocity.mul(delta));

      // box loop
      const halfHalfExtent = boundHalfExtent.div(2).toVar();
      position.assign(
        mod(position.add(halfHalfExtent), boundHalfExtent).sub(halfHalfExtent),
      );
    });
    const updateCompute = update().compute(count).setName("Update Particles");

    // nodes
    material.positionNode = positionBuffer.toAttribute();
    material.colorNode = Fn(() => {
      const velocity = velocityBuffer.toAttribute();
      const speed = velocity.length();
      const colorMix = speed.div(maxSpeed).smoothstep(0, 0.5);
      // Each ramp end rotates independently on its own seeded drop-pulse, so
      // colorA and colorB drift apart into different random hues.
      const finalColor = mix(hue(colorA, uHueA), hue(colorB, uHueB), colorMix);
      return vec4(finalColor, 1);
    })();
    material.scaleNode = particleMassMultiplier.mul(scale);

    const geometry = new THREE.PlaneGeometry(1, 1);
    const mesh = new THREE.InstancedMesh(geometry, material, count);
    scene.add(mesh);

    // stash refs for the audio update()
    this._uMaxSpeed = maxSpeed;
    this._uAttractorMass = attractorMass;
    this._uSpinningStrength = spinningStrength;
    this._uParticleGlobalMass = particleGlobalMass;
    this._uHueA = uHueA;
    this._uHueB = uHueB;
    this._uBallHues = ballHues;
    this._uBassGlow = uBassGlow;
    this._uTime = uTime;
    this._updateCompute = updateCompute;

    // ---- bloom pipeline (Ball of Fire's glow; bass drives the strength) ---------
    const renderPipeline = new THREE.RenderPipeline(renderer);
    const scenePass = pass(scene, camera);
    scenePass.name = "Scene Pass";
    const bloomPass = bloom(scenePass);
    bloomPass.threshold.value = 0.5;
    bloomPass.radius.value = 1.0;
    bloomPass.strength.value = BLOOM_BASE;
    renderPipeline.outputNode = scenePass.add(bloomPass);
    this._renderPipeline = renderPipeline;
    this._bloomPass = bloomPass;
  }

  _animate() {
    // ponytail: async backend init - no-op until renderer.init() resolved.
    if (!this._initialized) return;
    const delta = Math.min(this._dt || 1 / 60, 1 / 30);
    this._uTime.value += delta;
    this._uBassGlow.value = this._bass;
    // bass -> bloom strength (the ball's glow, like Ball of Fire)
    this._bloomPass.strength.value = Math.min(
      BLOOM_BASE + this._bass * BLOOM_PER_BASS,
      4.0,
    );

    // Camera drifts around a sphere centred on the attractor centroid. A slow
    // azimuth sweep orbits the scene; the polar angle sways a little so the view
    // tilts up/down rather than staying in one flat plane. lookAt keeps the centre
    // dead-centre at all times.
    this._camAzimuth += delta * 0.12;
    this._camPolar = Math.PI / 2 + Math.sin(this._uTime.value * 0.05) * 0.2;
    const r = this._camRadius;
    this.camera.position.set(
      this._camCenter.x +
        r * Math.sin(this._camPolar) * Math.cos(this._camAzimuth),
      this._camCenter.y + r * Math.cos(this._camPolar),
      this._camCenter.z +
        r * Math.sin(this._camPolar) * Math.sin(this._camAzimuth),
    );
    this.camera.lookAt(this._camCenter);

    this.renderer.compute(this._updateCompute);
    return this._renderPipeline.render();
  }
}
