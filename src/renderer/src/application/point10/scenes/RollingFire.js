// RollingFire - port of three.js examples/webgpu_volume_fire (master, MIT):
// GPU 3D fluid simulation (semi-Lagrangian advection + curlNoise, buoyancy,
// Jacobi projection) rendered volumetrically, with the teapot replaced by a
// lava-textured sphere and the bloom strength driven by the bass band.
// Requires WebGPU.
//
// modV port of musicvis/src/scenes/RollingFire.js. Three things had to change
// for the worker context, everything else is upstream:
//   - the WebGPURenderer + OffscreenCanvas come from the adapter (`host`);
//     there is no DOM to append a canvas to and no window to read a size from
//   - the private requestAnimationFrame loop is gone: modV's worker loop calls
//     update(dt) / render(), and `init()` replaces onActivate()
//   - textures are Vite assets read through ../shared/textures.js
import * as THREE from "three/webgpu";
import {
  vec3,
  vec4,
  uvec3,
  float,
  Fn,
  uniform,
  texture3D,
  textureStore,
  instanceIndex,
  screenCoordinate,
  pass,
  smoothstep,
  mix,
  min,
  max,
  floor,
  mx_noise_float,
  storage,
  storageTexture,
  If,
  cameraPosition,
  hue,
  luminance,
  Loop,
  positionWorld,
  positionLocal,
  interleavedGradientNoise,
  frameId,
  fract,
  saturation,
  cos,
  sin,
  atan,
} from "three/tsl";
import { snoise, snoiseVec3 } from "three/addons/tsl/math/curlNoise.js";
import { ImprovedNoise } from "three/addons/math/ImprovedNoise.js";
import { gaussianBlur } from "three/addons/tsl/display/GaussianBlurNode.js";
import { bloom } from "three/addons/tsl/display/BloomNode.js";
import { BaseSceneWebGPU } from "../BaseSceneWebGPU.js";
// Phrase colour flip + palette, shared with SmokeRoom / AttractorParticles /
// AroundPlus (moved out of this file - it is scene-agnostic).
import { dropColorPulse, ROYGBIV_HUES } from "../shared/phraseColor.js";
import { loadTexture } from "../shared/textures.js";
import floorDiffUrl from "../textures/rubber002_diff_1k.jpg?url";
import floorRoughUrl from "../textures/rubber002_rough_1k.jpg?url";
import sphereDiffUrl from "../textures/moon_04_diff_1k.jpg?url";

const GRID_SIZE_X = 100;
const GRID_SIZE_Y = 100;
const GRID_SIZE_Z = 200;
const CELL_COUNT = GRID_SIZE_X * GRID_SIZE_Y * GRID_SIZE_Z;
const PRESSURE_ITERATIONS = 2;

// ponytail: world kept larger than the original 12x12x24 so walls stay far, but
// the voxel grid is back to 2M cells - the 128x128x256 bump doubled sim compute
// (2M -> 4.2M cells per pass) and tanked the frame rate.
const VOLUME_WORLD_SIZE_X = 16;
const VOLUME_WORLD_SIZE_Y = 16;
const VOLUME_WORLD_SIZE_Z = 32;
const VOLUME_WORLD_SIZE_DIAGONAL = Math.sqrt(
  VOLUME_WORLD_SIZE_X ** 2 +
    VOLUME_WORLD_SIZE_Y ** 2 +
    VOLUME_WORLD_SIZE_Z ** 2,
);
const TEXEL_X = 1 / GRID_SIZE_X;
const TEXEL_Y = 1 / GRID_SIZE_Y;
const TEXEL_Z = 1 / GRID_SIZE_Z;
const SPHERE_RADIUS = 0.9;
// ponytail: AudioEngine fftSize=1024 @48kHz -> first 5 bins ≈ 0-280Hz (bass).
const BASS_BINS = 5;
const BLOOM_BASE = 0.12;
const BLOOM_PER_BASS = 3.6; // bass -> bloom strength (the sphere's glow)

// Hue-rotate a fire colour and rescale it to the same luminance the GREEN hue
// would produce. three's `hue()` is a rotation about the white axis: it keeps
// each ramp stop's brightness unevenly (green lands the warm ramp on high-
// luminance channels, blue/violet on dark ones), so without this the green fire
// reads much brighter than the others. Equalizing per-sample luminance makes
// every ROYGBIV state as bright as the green one, and it stays smooth while the
// hue eases between states.
const fireHue = (color, adjustment) => {
  const rotated = hue(color, adjustment);
  const green = hue(color, ROYGBIV_HUES[4]);
  return rotated.mul(luminance(green).div(luminance(rotated).max(0.001)));
};

// ---- ball drive (pure, tested in vf-color-check.mjs) -------------------------
// The floor is a level group whose local plane is y=0 (no tilt - the ball is
// driven by music, see update()). The ball integrates on that plane: an
// acceleration vector (bass push + beat kick) drives the roll with the 5/7
// solid-sphere factor, damping bleeds momentum so silence stops it, and the
// rim pins it to PLAY_RADIUS (inside the fire volume's footprint) with a bounce.
const PLAY_RADIUS = Infinity; // rollBall's default: no invisible wall (the
// scene passes BALL_ROVE_RADIUS explicitly to
// keep the ball on the fixed floor)
const FLOOR_TILE = 10; // world metres per texture tile (was 20: a
// lot smaller so the lava detail reads
// close to the ball and camera)
const FLOOR_PLANE = 400; // plane extent - big, and it follows the ball
// so the edge is never seen
const BALL_ACCEL_K = 60; // smoothed bass -> ball acceleration (m/s² per unit bass)
const BALL_DIR_SPIN = 0.35; // autonomous steering rotation (rad/s): the ball roves
const BALL_DIR_SWAY = 2.0; // hi-hat centroid -> steering nudge
const BALL_BEAT_K = 3.0; // bass-beat velocity kick (m/s per unit beat)
// Bass "beat" gates: the smoothed bass must jump sharply (kick rises more than
// riseFrac of its current level, floored by the AGC-scaled floor) so a drone or
// vocal-only tail never nudges the ball; maxRise normalizes the kick to <= 1.
export const BALL_BEAT = { riseFrac: 0.3, floor: 0.04, maxRise: 0.1 };
const FIRE_HAT_K = 2.5; // hi-hat band -> uFireSize emission multiplier
const FIRE_SNARE_K = 8; // snare band -> uFireSize: big pulse per hit
const ROLL_FACTOR = 5 / 7; // solid sphere, rolling without slipping
const ROLL_DAMP = 0.3; // rolling resistance (1/s): enough to bleed the
// momentum so silence stops the ball
const ROLL_MAX_SPEED = 30; // m/s cap on the ball (keeps it from rocketing off)
export { ROLL_MAX_SPEED };
const BALL_ROVE_RADIUS = 40; // the ball roves within this radius of the origin. The
// floor stays FIXED (only the ball moves), so its
// motion over the floor texture is visible - a floor
// that tracked the ball froze every reference frame.
export { BALL_ROVE_RADIUS };
const FLOOR_AIM_DOWN = 1; // aim the camera below the ball so the floor stays in frame
const BALL_BOUNCE = 0.6; // rim restitution

// A bass "beat": 0..1 kick strength when the smoothed bass jumps sharply (see
// BALL_BEAT gates); flat, decaying or quiet bass reads 0.
export function beatKick(bass, prevBass, opts = BALL_BEAT) {
  const rise = bass - prevBass;
  if (rise <= Math.max(opts.riseFrac * bass, opts.floor)) return 0;
  return Math.min(rise / opts.maxRise, 1);
}

// ball: {x, z, vx, vz} on the local plane (y=0). grav is [gx,gy,gz] in the same
// frame. Level plane => grav x/z zero => no motion. Returns the surface point.
export function rollBall(ball, grav, dt, opts = {}) {
  const damping = opts.damping ?? ROLL_DAMP;
  const bounce = opts.bounce ?? BALL_BOUNCE;
  const radius = opts.radius ?? PLAY_RADIUS;
  const maxSpeed = opts.maxSpeed ?? ROLL_MAX_SPEED;
  let { x = 0, z = 0, vx = 0, vz = 0 } = ball;
  const damp = Math.max(0, 1 - damping * dt);
  vx = (vx + grav[0] * ROLL_FACTOR * dt) * damp;
  vz = (vz + grav[2] * ROLL_FACTOR * dt) * damp;
  const sp = Math.hypot(vx, vz);
  if (sp > maxSpeed) {
    vx = (vx / sp) * maxSpeed;
    vz = (vz / sp) * maxSpeed;
  }
  x += vx * dt;
  z += vz * dt;
  const r = Math.hypot(x, z);
  if (r > radius) {
    x *= radius / r;
    z *= radius / r;
    const rv = (vx * x + vz * z) / radius;
    if (rv > 0) {
      vx -= (1 + bounce) * rv * (x / radius);
      vz -= (1 + bounce) * rv * (z / radius);
    }
  }
  return { x, z, vx, vz };
}

export class RollingFire extends BaseSceneWebGPU {
  constructor(renderer, camera, host) {
    super(renderer, camera, host);
    this.label = "Rolling Fire";
    this.category = "Fire";
    // Foreground-only (BUGS.md): the transparent fire canvas always
    // composites OVER another scene as the perf foreground layer.
    this.role = "foreground";
    // Transparent overlay: empty pixels carry alpha = max(rgb) so the base
    // layer beneath shows through (same trick as Smoke Room). Ball of Fire
    // overrides this - it plays standalone and must stay opaque or the
    // stale shared WebGL frame would show behind it.
    this._transparentOverlay = true;
    // ponytail: three's capabilities/WebGPU.js addon uses top-level await
    // (rejected by vite's es2020 pre-bundle target) - same check, no import.
    this._available = typeof navigator !== "undefined" && !!navigator.gpu;
    if (this._available) {
      navigator.gpu.requestAdapter().then((adapter) => {
        this._available = !!adapter;
      });
    }
    this._initialized = false;
    this._active = false;
    this._rafId = null;
    this._bass = 0;
    this._hi = 0; // smoothed hi-hat band energy (fire/smoke pulses)
    this._snare = 0; // smoothed snare band energy (big fire/smoke pulses)
    this._ball = { x: 0, z: 0, vx: 0, vz: 0 }; // on the local plane
    this._freezeBall = false; // Ball of Fire pins the ball at its start position
    this._ballDir = 0; // steering azimuth (rad): the ball's accel direction
    this._prevBass = 0; // previous smoothed bass (beat-rise detect)
    this._accelX = 0;
    this._accelZ = 0; // bass-driven acceleration (m/s²)
    this._kickX = 0;
    this._kickZ = 0; // beat velocity impulse (m/s)
    this._camFollow = new THREE.Vector3(9, 3.5, 2.8); // smoothed offset from ball
    this._camPos = new THREE.Vector3();
    this._camYaw = 0; // slow music-driven orbit angle (see update)
    this._pulse = { hue: 0, t: 0 };
    if (!this._available) {
      console.warn("RollingFire: WebGPU not available, scene disabled.");
      return;
    }
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

    // Ball drive (replaces the old floor-tilt + gravity roll): the floor stays
    // level and the ball accelerates with the MUSIC. A continuous push scaled
    // by the smoothed bass energy, plus a velocity kick on each sharp bass rise
    // (the beat - the "bpm counter"). The steering azimuth rotates slowly on
    // its own and is nudged by the hi-hat spectral centroid, so the ball roves
    // in a curving path instead of running in one straight line.
    const hiStart = fm.length - Math.floor(fm.length * 0.15); // ~8-20kHz hats
    let w = 0,
      c = 0;
    for (let i = hiStart; i < fm.length; i++) {
      w += fm[i];
      c += fm[i] * i;
    }
    const sway =
      w > 1e-6 ? ((c / w - hiStart) / (fm.length - hiStart) - 0.5) * 0.25 : 0;
    let hi = 0;
    for (let i = hiStart; i < fm.length; i++) hi += fm[i];
    hi /= fm.length - hiStart;
    this._hi += (hi - this._hi) * Math.min(1, dt * 10);
    this._ballDir += (BALL_DIR_SPIN + sway * BALL_DIR_SWAY) * dt;
    const kick = beatKick(this._bass, this._prevBass);
    this._prevBass = this._bass;
    const mag = this._bass * BALL_ACCEL_K;
    this._accelX = Math.cos(this._ballDir) * mag;
    this._accelZ = Math.sin(this._ballDir) * mag;
    this._kickX = Math.cos(this._ballDir) * kick * BALL_BEAT_K;
    this._kickZ = Math.sin(this._ballDir) * kick * BALL_BEAT_K;

    // Camera yaw: a slow, SUBTLE orbit from BASS - reduced gain/slew so a
    // bass swell doesn't pan the camera in an arc around the ball (the
    // left/right jerks). Full gain 0.5 @ 0.4/s swung ±15°; this keeps ~±8°
    // at a gentle drift so the orbit reads as ambience, not motion.
    this._camYaw += (this._bass * 0.15 - this._camYaw) * Math.min(1, dt * 0.15);

    // Hi-hats puff the fire/smoke: a smoothed hat energy scales the emission
    // volume (uFireSize), so every hat hit gives a small pulse of fire/smoke.
    // Snares (the 1-6kHz crack band, distinct from hats' 8k+) add a BIG pulse.
    if (this._uFireSize) {
      const sLo = Math.floor(fm.length * 0.02); // ~1kHz
      const sHi = Math.floor(fm.length * 0.13); // ~6kHz
      let sn = 0;
      for (let i = sLo; i < sHi; i++) sn += fm[i];
      sn /= sHi - sLo;
      this._snare += (sn - this._snare) * Math.min(1, dt * 12);
      this._uFireSize.value =
        1 + this._hi * FIRE_HAT_K + this._snare * FIRE_SNARE_K;
    }

    // Colour change on the music itself: the bass "drop detector". When the
    // smoothed sub-bass band's rise out of its rest level crests (a new 16-32
    // bar phrase arrives, at any tempo) and rearmSec has passed, the ROYGBIV
    // hue flips. Nothing counts beats, bars or time-to-fire; the flip lands on
    // the crest of the drop wherever it happens.
    this._pulse = dropColorPulse(this._pulse, this._bass, dt);
    let dh = this._pulse.hue - this._uFireHue.value;
    dh = Math.atan2(Math.sin(dh), Math.cos(dh)); // shortest hue arc (hue node is 2π-periodic)
    this._uFireHue.value += dh * Math.min(1, dt * 1.2);
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
    this._width = w;
    this._height = h;
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
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 2;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.transmitted = true;
    this.canvas = renderer.domElement;

    const scene = new THREE.Scene();
    // No background: the fire is emitted onto transparent black, so empty
    // pixels stay clear when this canvas is a foreground layer.
    scene.background = null;
    this.scene = scene;

    const camera = new THREE.PerspectiveCamera(60, width / height, 0.1, 100);
    camera.position.set(9, 3.5, 2.8);
    this.camera = camera;
    // ponytail: no OrbitControls - the camera is the music. It keeps the ball
    // centred on screen at all times (lookAt the ball), offset behind it.
    this._camBase = new THREE.Vector3(9, 3.5, 2.8);

    // ---- sim resources ---------------------------------------------------
    const createStorage3D = (name) => {
      const texture = new THREE.Storage3DTexture(
        GRID_SIZE_X,
        GRID_SIZE_Y,
        GRID_SIZE_Z,
      );
      texture.name = name;
      texture.format = THREE.RGBAFormat;
      texture.type = THREE.HalfFloatType;
      texture.minFilter = THREE.LinearFilter;
      texture.magFilter = THREE.LinearFilter;
      texture.wrapS = THREE.ClampToEdgeWrapping;
      texture.wrapT = THREE.ClampToEdgeWrapping;
      texture.wrapR = THREE.ClampToEdgeWrapping;
      return texture;
    };

    const velTexA = createStorage3D("velocity A");
    const velTexB = createStorage3D("velocity B");
    const dyeTexA = createStorage3D("dye A");
    const dyeTexB = createStorage3D("dye B");
    const divTex = createStorage3D("divergence");
    const pressTexA = createStorage3D("pressure A");
    const pressTexB = createStorage3D("pressure B");
    const curlNoiseTex = createStorage3D("curlNoise");
    curlNoiseTex.wrapS = THREE.RepeatWrapping;
    curlNoiseTex.wrapT = THREE.RepeatWrapping;
    curlNoiseTex.wrapR = THREE.RepeatWrapping;

    const dyeTexNode = texture3D(dyeTexA);
    const dyeTexWriteNode = storageTexture(dyeTexB).toWriteOnly();
    const curlNoiseTexNode = texture3D(curlNoiseTex);

    // uniforms
    const uVolumeWorldSize = uniform(
      new THREE.Vector3(
        VOLUME_WORLD_SIZE_X,
        VOLUME_WORLD_SIZE_Y,
        VOLUME_WORLD_SIZE_Z,
      ),
    );
    const uDt = uniform(0.016);
    const uTime = uniform(0);
    const uBuoyancy = uniform(3.0);
    const uWeight = uniform(0.15);
    const uTurbulence = uniform(3.2);
    const uTurbulenceDecay = uniform(0.1);
    const uTurbFrequency = uniform(10.0);
    const uVelDamping = uniform(0.25);
    const uCooling = uniform(1.0); // 1.0s fire lifespan
    const uDissipation = uniform(0.4); // 2.5s smoke lifespan
    const uEmitDensity = uniform(16.0); // less smoke volume after the grid bump
    const uEmitTemperature = uniform(14.0); // less fire
    const uSphereMatrix = uniform(new THREE.Matrix4());
    const uSphereSpeed = uniform(0.0);
    const uMotionBoost = uniform(0.25);
    const uSphereVelocity = uniform(new THREE.Vector3());
    const uWindStrength = uniform(6.5);
    const uSpherePosition = uniform(new THREE.Vector3());
    const uFireIntensity = uniform(26.0);
    const uBassGlow = uniform(0.0); // smoothed bass -> sphere self-glow
    const uFireGlowSpread = uniform(5.0);
    const uShadowAbsorption = uniform(2.0);
    const uShadowAmbient = uniform(0.5);
    const uFireStartColor = uniform(new THREE.Color(0xffe68c));
    const uFireMidColor = uniform(new THREE.Color(0xff7305));
    const uFireEndColor = uniform(new THREE.Color(0xff0000));
    const uFireHue = uniform(0.0);
    const uAsymmetry = uniform(0.0);
    const uPowderStrength = uniform(0.59);
    const uMultiScattering = uniform(1.0);
    const uPointLightVolumeIntensity = uniform(1.4);
    const uPointLightSurfaceIntensity = uniform(7.0);
    const uLightNearIntensity = uniform(10.0);
    const uLightFarIntensity = uniform(15.0);
    const uLightFarDistance = uniform(10.0);
    const uPointLightProjectionRadius = uniform(20.0);
    const uPointLightProjectionFrequency = uniform(0.2);
    const uPointLightProjectionNoiseFade = uniform(17.0);
    const uPointLightProjectionCenterFade = uniform(3.25);
    const uFlameHeight = uniform(3.5);
    const uFireSize = uniform(1.0); // hi-hat pulse: scales the emission volume
    const uSway = uniform(new THREE.Vector3());
    const uFlicker = uniform(1.0);
    const uColorNoise = uniform(0.0);
    const uKeyLightPos = uniform(new THREE.Vector3());
    const uGridOrigin = uniform(new THREE.Vector3()); // world pos of the sim grid's
    // bottom-center: the grid follows
    // the ball so the fire rides with it
    const cpuNoise = new ImprovedNoise();

    // ---- sphere emitter geometry (needed by emitSpherePass below) ---------
    const sphereGeometry = new THREE.SphereGeometry(SPHERE_RADIUS, 64, 40);
    this._sphereVertexCount = sphereGeometry.attributes.position.count;
    this._sphereVertices = storage(
      sphereGeometry.attributes.position,
      "vec3",
      this._sphereVertexCount,
    ).toReadOnly();

    // ---- TSL helpers ------------------------------------------------------
    const getVoxelCoord = (id) => {
      const x = id.mod(GRID_SIZE_X);
      const y = id.div(GRID_SIZE_X).mod(GRID_SIZE_Y);
      const z = id.div(GRID_SIZE_X * GRID_SIZE_Y);
      return uvec3(x, y, z);
    };

    const coordToUVW = (coord) =>
      vec3(coord)
        .add(0.5)
        .div(vec3(GRID_SIZE_X, GRID_SIZE_Y, GRID_SIZE_Z));

    // ---- compute passes ---------------------------------------------------
    const computeCurlNoisePass = Fn(() => {
      const coord = getVoxelCoord(instanceIndex);
      const uvw = coordToUVW(coord);

      const freq = uTurbFrequency;
      const e = float(0.1).div(freq);
      const dx = vec3(e, 0.0, 0.0);
      const dy = vec3(0.0, e, 0.0);
      const dz = vec3(0.0, 0.0, e);

      const p = uvw.mul(
        vec3(
          VOLUME_WORLD_SIZE_X / VOLUME_WORLD_SIZE_Y,
          1.0,
          VOLUME_WORLD_SIZE_Z / VOLUME_WORLD_SIZE_Y,
        ),
      );
      const p_x0 = snoiseVec3(p.sub(dx).mul(freq));
      const p_x1 = snoiseVec3(p.add(dx).mul(freq));
      const p_y0 = snoiseVec3(p.sub(dy).mul(freq));
      const p_y1 = snoiseVec3(p.add(dy).mul(freq));
      const p_z0 = snoiseVec3(p.sub(dz).mul(freq));
      const p_z1 = snoiseVec3(p.add(dz).mul(freq));

      const x = p_y1.z.sub(p_y0.z).sub(p_z1.y).add(p_z0.y);
      const y = p_z1.x.sub(p_z0.x).sub(p_x1.z).add(p_x0.z);
      const z = p_x1.y.sub(p_x0.y).sub(p_y1.x).add(p_y0.x);

      const noiseVal = vec3(x, y, z).mul(5.0);
      textureStore(curlNoiseTex, coord, vec4(noiseVal, 0.0)).toWriteOnly();
    })()
      .compute(CELL_COUNT)
      .setName("computeCurlNoise");

    const advectVelocityPass = Fn(() => {
      const coord = getVoxelCoord(instanceIndex);
      const uvw = coordToUVW(coord);

      const vel = texture3D(velTexA, uvw, 0).xyz;
      const velUVW = vel.div(uVolumeWorldSize);
      const prevPos = uvw.sub(velUVW.mul(uDt));
      const newVel = texture3D(velTexA, prevPos, 0).xyz.toVar();

      const dye = dyeTexNode.sample(uvw).level(0);
      const density = dye.r;
      const temperature = dye.g;
      const age = dye.b;

      const thermalNoisePos = uvw.add(
        vec3(0, age.negate().mul(0.6), age.mul(0.13)).div(uTurbFrequency),
      );
      const decay = age.mul(uTurbulenceDecay.negate()).exp();
      const thermalTurbulence = curlNoiseTexNode
        .sample(thermalNoisePos)
        .level(0)
        .xyz.mul(uTurbulence)
        .mul(temperature)
        .mul(decay);

      const ambientNoisePos = uvw
        .mul(0.5)
        .add(vec3(0, uTime.mul(0.25), uTime.mul(0.06)).div(uTurbFrequency));
      const ambientTurbulence = curlNoiseTexNode
        .sample(ambientNoisePos)
        .level(0)
        .xyz.mul(uTurbulence.mul(0.2))
        .mul(density);

      const turbulence = thermalTurbulence
        .add(ambientTurbulence)
        .mul(VOLUME_WORLD_SIZE_Y);
      newVel.addAssign(turbulence.mul(uDt));

      newVel.mulAssign(max(float(1).sub(uVelDamping.mul(uDt)), 0));

      const worldPos = uvw
        .sub(0.5)
        .mul(uVolumeWorldSize)
        .add(vec3(0, VOLUME_WORLD_SIZE_Y / 2, 0))
        .add(uGridOrigin);
      const dist = worldPos.distance(uSpherePosition);
      const sphereRadius = float(SPHERE_RADIUS);

      // Radial "buoyancy": push fire/smoke OUTWARD from the sphere centre in
      // all directions (instead of always rising with convection). Magnitude
      // is the usual temperature-vs-weight balance; direction is away from
      // the emitter, so the fire puffs outwards like an explosion.
      const buoyancyForce = temperature
        .mul(uBuoyancy)
        .sub(density.mul(uWeight))
        .mul(VOLUME_WORLD_SIZE_Y);
      If(dist.greaterThan(0.01), () => {
        const dir = worldPos.sub(uSpherePosition).div(dist);
        newVel.addAssign(dir.mul(buoyancyForce).mul(uDt));
      });

      If(dist.lessThan(sphereRadius), () => {
        const ratio = dist.div(sphereRadius);
        const falloff = smoothstep(0.0, 1.0, float(1.0).sub(ratio));
        const windNoisePos = uvw.add(
          vec3(0.0, uTime.mul(0.5), 0.0).div(uTurbFrequency),
        );
        const windTurbulence = curlNoiseTexNode
          .sample(windNoisePos)
          .level(0)
          .xyz.mul(uTurbulence)
          .mul(uSphereSpeed);
        const windVel = uSphereVelocity
          .mul(uWindStrength)
          .add(windTurbulence)
          .mul(uDt)
          .mul(falloff);
        newVel.addAssign(windVel);
      });

      const edge = min(uvw, vec3(1).sub(uvw));
      const boundary = smoothstep(0.0, 0.08, min(edge.x, min(edge.y, edge.z)));
      newVel.mulAssign(boundary);

      textureStore(velTexB, coord, vec4(newVel, 0)).toWriteOnly();
    })()
      .compute(CELL_COUNT)
      .setName("advectVelocity");

    const divergencePass = Fn(() => {
      const coord = getVoxelCoord(instanceIndex);
      const uvw = coordToUVW(coord);
      const vR = texture3D(velTexB, uvw.add(vec3(TEXEL_X, 0, 0)), 0).x;
      const vL = texture3D(velTexB, uvw.sub(vec3(TEXEL_X, 0, 0)), 0).x;
      const vU = texture3D(velTexB, uvw.add(vec3(0, TEXEL_Y, 0)), 0).y;
      const vD = texture3D(velTexB, uvw.sub(vec3(0, TEXEL_Y, 0)), 0).y;
      const vF = texture3D(velTexB, uvw.add(vec3(0, 0, TEXEL_Z)), 0).z;
      const vB = texture3D(velTexB, uvw.sub(vec3(0, 0, TEXEL_Z)), 0).z;
      const divergence = vR.sub(vL).add(vU.sub(vD)).add(vF.sub(vB)).mul(0.5);
      textureStore(divTex, coord, vec4(divergence, 0, 0, 0)).toWriteOnly();
    })()
      .compute(CELL_COUNT)
      .setName("divergence");

    const jacobi = (pressRead, pressWrite, name) =>
      Fn(() => {
        const coord = getVoxelCoord(instanceIndex);
        const uvw = coordToUVW(coord);
        const pR = texture3D(pressRead, uvw.add(vec3(TEXEL_X, 0, 0)), 0).x;
        const pL = texture3D(pressRead, uvw.sub(vec3(TEXEL_X, 0, 0)), 0).x;
        const pU = texture3D(pressRead, uvw.add(vec3(0, TEXEL_Y, 0)), 0).x;
        const pD = texture3D(pressRead, uvw.sub(vec3(0, TEXEL_Y, 0)), 0).x;
        const pF = texture3D(pressRead, uvw.add(vec3(0, 0, TEXEL_Z)), 0).x;
        const pB = texture3D(pressRead, uvw.sub(vec3(0, 0, TEXEL_Z)), 0).x;
        const divergence = texture3D(divTex, uvw, 0).x;
        const pressure = pR
          .add(pL)
          .add(pU)
          .add(pD)
          .add(pF)
          .add(pB)
          .sub(divergence)
          .div(6);
        textureStore(pressWrite, coord, vec4(pressure, 0, 0, 0)).toWriteOnly();
      })()
        .compute(CELL_COUNT)
        .setName(name);

    const jacobiPassAB = jacobi(pressTexA, pressTexB, "jacobiAB");
    const jacobiPassBA = jacobi(pressTexB, pressTexA, "jacobiBA");

    const projectPass = Fn(() => {
      const coord = getVoxelCoord(instanceIndex);
      const uvw = coordToUVW(coord);
      const pR = texture3D(pressTexA, uvw.add(vec3(TEXEL_X, 0, 0)), 0).x;
      const pL = texture3D(pressTexA, uvw.sub(vec3(TEXEL_X, 0, 0)), 0).x;
      const pU = texture3D(pressTexA, uvw.add(vec3(0, TEXEL_Y, 0)), 0).x;
      const pD = texture3D(pressTexA, uvw.sub(vec3(0, TEXEL_Y, 0)), 0).x;
      const pF = texture3D(pressTexA, uvw.add(vec3(0, 0, TEXEL_Z)), 0).x;
      const pB = texture3D(pressTexA, uvw.sub(vec3(0, 0, TEXEL_Z)), 0).x;
      const gradient = vec3(pR.sub(pL), pU.sub(pD), pF.sub(pB)).mul(0.5);
      const vel = texture3D(velTexB, uvw, 0).xyz.sub(gradient);
      textureStore(velTexA, coord, vec4(vel, 0)).toWriteOnly();
    })()
      .compute(CELL_COUNT)
      .setName("project");

    const advectDyePass = Fn(() => {
      const coord = getVoxelCoord(instanceIndex);
      const uvw = coordToUVW(coord);
      const vel = texture3D(velTexA, uvw, 0).xyz;
      const velUVW = vel.div(uVolumeWorldSize);
      const prevPos = uvw.sub(velUVW.mul(uDt));
      const dye = dyeTexNode.sample(prevPos).level(0);
      const density = dye.r
        .mul(max(float(1).sub(uDissipation.mul(uDt)), 0))
        .toVar();
      const temperature = dye.g
        .mul(max(float(1).sub(uCooling.mul(uDt)), 0))
        .toVar();
      const gridDims = vec3(GRID_SIZE_X, GRID_SIZE_Y, GRID_SIZE_Z);
      const nearestUVW = floor(prevPos.mul(gridDims)).add(0.5).div(gridDims);
      const age = dyeTexNode.sample(nearestUVW).level(0).b.add(uDt).toVar();

      temperature.assign(temperature.clamp(0, 12));

      If(density.lessThanEqual(0.01), () => {
        age.assign(0.0);
      });

      textureStore(
        dyeTexWriteNode,
        coord,
        vec4(density, temperature, age, 1.0),
      ).toWriteOnly();
    })()
      .compute(CELL_COUNT)
      .setName("advectDye");

    const emitSpherePass = Fn(() => {
      const vertexPos = this._sphereVertices.element(instanceIndex);
      const worldPos = uSphereMatrix.mul(vec4(vertexPos, 1.0)).xyz;

      const uvw = worldPos
        .sub(uGridOrigin)
        .sub(vec3(0, VOLUME_WORLD_SIZE_Y / 2, 0))
        .div(uVolumeWorldSize)
        .add(0.5);

      If(
        uvw.x
          .greaterThanEqual(0)
          .and(uvw.x.lessThanEqual(1))
          .and(uvw.y.greaterThanEqual(0))
          .and(uvw.y.lessThanEqual(1))
          .and(uvw.z.greaterThanEqual(0))
          .and(uvw.z.lessThanEqual(1)),
        () => {
          const coord = uvec3(
            uvw.mul(vec3(GRID_SIZE_X, GRID_SIZE_Y, GRID_SIZE_Z)),
          );

          const flicker = mx_noise_float(
            vertexPos
              .mul(9.0)
              .add(vec3(0.0, uTime.negate().mul(2.5), uTime.mul(0.7))),
          )
            .mul(0.5)
            .add(0.5);
          const baseEmission = uEmitTemperature
            .greaterThan(0.0)
            .select(float(1.0), float(0.0));
          const movementEmission = uSphereSpeed.mul(uMotionBoost);
          const emissionFactor = baseEmission.add(movementEmission);
          const densityVal = uEmitDensity
            .mul(float(1 / 120))
            .mul(flicker.mul(0.85).add(0.15))
            .mul(emissionFactor)
            .mul(uFireSize);

          If(densityVal.greaterThan(0.0), () => {
            const tempVal = uEmitTemperature
              .mul(float(1 / 120))
              .mul(flicker.mul(0.85).add(0.15))
              .mul(emissionFactor)
              .mul(uFireSize);
            const currentDye = dyeTexNode.sample(uvw).level(0);
            const newDensity = currentDye.r.add(densityVal);
            const newTemp = currentDye.g.add(tempVal).clamp(0.0, 12.0);
            const currentAge = currentDye.b;
            const newAge = mix(
              currentAge,
              float(0.0),
              densityVal.div(max(newDensity, 0.001)),
            );
            textureStore(
              dyeTexWriteNode,
              coord,
              vec4(newDensity, newTemp, newAge, 1.0),
            ).toWriteOnly();
          });
        },
      );
    })()
      .compute(this._sphereVertexCount)
      .setName("emitSphere");

    // Precompute curl noise on the GPU
    renderer.computeAsync(computeCurlNoisePass);

    // ---- volumetric material ---------------------------------------------
    const volumetricMaterial = new THREE.VolumeNodeMaterial();
    volumetricMaterial.steps = 18;
    volumetricMaterial.transparent = true;
    volumetricMaterial.blending = THREE.AdditiveBlending;
    volumetricMaterial.depthWrite = false;
    volumetricMaterial.offsetNode = fract(
      interleavedGradientNoise(screenCoordinate).add(
        float(frameId).mul(0.618033988749895),
      ),
    );

    const fireRamp = Fn(([t]) => {
      const color = vec3(0).toVar();
      color.assign(
        mix(vec3(0.0, 0.0, 0.0), uFireEndColor, smoothstep(0.05, 0.35, t)),
      );
      color.assign(mix(color, uFireMidColor, smoothstep(0.35, 0.65, t)));
      color.assign(mix(color, uFireStartColor, smoothstep(0.65, 1.0, t)));
      return color;
    });

    const henyeyGreenstein = Fn(([cosTheta, g]) => {
      const g2 = g.mul(g);
      const denom = float(1.0).add(g2).sub(float(2.0).mul(g).mul(cosTheta));
      const oneMinusG2 = float(1.0).sub(g2);
      return oneMinusG2.div(denom.pow(1.5)).mul(0.079577);
    });

    const getVolumeSample = ({ positionRay }) => {
      const uvw = positionRay
        .sub(uGridOrigin)
        .sub(vec3(0, VOLUME_WORLD_SIZE_Y / 2, 0))
        .div(uVolumeWorldSize)
        .add(0.5)
        .toVar();
      const noiseDistortion = texture3D(velTexA, uvw, 0)
        .xyz.div(uVolumeWorldSize)
        .mul(0.15);
      const distortedUVW = uvw.add(noiseDistortion).clamp(0.0, 1.0).toVar();
      const sample = dyeTexNode.sample(distortedUVW).level(0);
      const density = sample.r;
      const age = sample.b;
      const temperature = sample.g;
      const detailNoise = snoise(
        positionRay.mul(5.5).add(vec3(0, age.mul(0.8).negate(), 0)),
      );
      density.mulAssign(detailNoise.mul(0.35).add(0.85));
      const edge = min(distortedUVW, vec3(1).sub(distortedUVW));
      density.mulAssign(
        smoothstep(0.0, 0.06, min(edge.x, min(edge.y, edge.z))),
      );
      return { density, temperature, age, distortedUVW };
    };

    volumetricMaterial.scatteringNode = Fn(({ positionRay }) => {
      const { density } = getVolumeSample({ positionRay });

      const lightDir = uKeyLightPos.sub(positionRay).normalize();
      const shadowDensitySum = float(0.0).toVar();
      const shadowStepSize = 0.35;

      for (let i = 0; i < 2; i++) {
        const stepDist = (i + 0.5) * shadowStepSize;
        const shadowPos = positionRay.add(lightDir.mul(stepDist));
        const shadowUVW = shadowPos
          .sub(uGridOrigin)
          .sub(vec3(0, VOLUME_WORLD_SIZE_Y / 2, 0))
          .div(uVolumeWorldSize)
          .add(0.5);
        const shadowEdge = min(shadowUVW, vec3(1).sub(shadowUVW));
        const shadowFade = smoothstep(
          0.0,
          0.06,
          min(shadowEdge.x, min(shadowEdge.y, shadowEdge.z)),
        );
        const shadowSample = texture3D(dyeTexA, shadowUVW, 0).r.mul(shadowFade);
        shadowDensitySum.addAssign(shadowSample);
      }

      const tau = shadowDensitySum.mul(shadowStepSize).mul(uShadowAbsorption);
      const beer = tau.negate().exp();
      const multiScatter = tau.mul(0.25).negate().exp().mul(0.5);
      const baseTransmittance = mix(
        beer,
        beer.add(multiScatter),
        uMultiScattering,
      );
      const powder = float(1.0).sub(tau.mul(2.0).negate().exp());
      const finalTransmittance = mix(
        baseTransmittance,
        baseTransmittance.mul(powder),
        uPowderStrength,
      );
      const lightTransmittance = finalTransmittance
        .add(uShadowAmbient)
        .clamp(0.0, 1.0);

      const viewDir = cameraPosition.sub(positionRay).normalize();
      const cosTheta = viewDir.dot(lightDir).clamp(-1.0, 1.0);
      const phase = henyeyGreenstein(cosTheta, uAsymmetry);

      return vec3(density).mul(lightTransmittance).mul(phase.mul(12.56637));
    });

    volumetricMaterial.scatteringEmissiveNode = Fn(({ positionRay }) => {
      const { density, temperature } = getVolumeSample({ positionRay });

      const firePower = float(6.0).sub(uFireGlowSpread);
      const fire = fireRamp(temperature.clamp(0, 1))
        .mul(temperature.pow(firePower))
        .mul(uFireIntensity);
      const fireColor = fireHue(fire, uFireHue);
      const distance = positionRay.sub(uKeyLightPos).length();
      // attenuation tuned for the spotlight at ~20m (25/d² ≈ same per-unit
      // falloff as the old 400/d² when the light sat at 80m)
      const attenuation = float(25.0).div(distance.pow(2.0));
      return fireColor.mul(density.add(0.15)).mul(attenuation);
    });

    const volumeCastShadow = Fn(() => {
      const startPos = positionWorld;
      const lightDir = positionWorld.sub(cameraPosition).normalize();
      const steps = uniform("int").onRenderUpdate(
        ({ material }) => material.steps || volumetricMaterial.steps,
      );
      const maxDistance = float(VOLUME_WORLD_SIZE_DIAGONAL);
      const stepSize = maxDistance.div(steps).toVar();
      const rayDir = lightDir.toVar();
      const distTravelled = float(0.0).toVar();
      const transmittance = float(1.0).toVar();

      Loop(steps, () => {
        const positionRay = startPos.add(rayDir.mul(distTravelled));
        const { density } = getVolumeSample({ positionRay });
        const absorption = density.mul(uShadowAbsorption).mul(0.01);
        const falloff = absorption.negate().mul(stepSize).exp();
        transmittance.mulAssign(falloff);
        distTravelled.addAssign(stepSize);
      });

      transmittance.greaterThanEqual(0.99).discard();
      return vec4(vec3(0), transmittance.oneMinus().mul(5));
    });

    const volumetricMesh = new THREE.Mesh(
      new THREE.BoxGeometry(
        VOLUME_WORLD_SIZE_X,
        VOLUME_WORLD_SIZE_Y,
        VOLUME_WORLD_SIZE_Z,
      ),
      volumetricMaterial,
    );
    volumetricMesh.position.y = VOLUME_WORLD_SIZE_Y / 2 + 0.4;
    volumetricMesh.receiveShadow = true;
    this._volumetricMesh = volumetricMesh;
    scene.add(volumetricMesh);

    const shadowMaterial = new THREE.VolumeNodeMaterial();
    shadowMaterial.steps = volumetricMaterial.steps;
    shadowMaterial.offsetNode = volumetricMaterial.offsetNode;
    shadowMaterial.castShadowNode = volumeCastShadow();
    shadowMaterial.shadowSide = THREE.FrontSide;
    shadowMaterial.colorWrite = false;
    shadowMaterial.depthWrite = false;
    shadowMaterial.blending = THREE.CustomBlending;
    shadowMaterial.blendEquation = THREE.AddEquation;
    shadowMaterial.blendSrc = THREE.ZeroFactor;
    shadowMaterial.blendDst = THREE.OneMinusSrcAlphaFactor;
    shadowMaterial.blendEquationAlpha = THREE.AddEquation;
    shadowMaterial.blendSrcAlpha = THREE.OneFactor;
    shadowMaterial.blendDstAlpha = THREE.OneMinusSrcAlphaFactor;

    const volumetricShadowMesh = new THREE.Mesh(
      new THREE.BoxGeometry(
        VOLUME_WORLD_SIZE_X,
        VOLUME_WORLD_SIZE_Y,
        VOLUME_WORLD_SIZE_Z,
      ),
      shadowMaterial,
    );
    volumetricShadowMesh.position.y = VOLUME_WORLD_SIZE_Y / 2 + 0.4;
    volumetricShadowMesh.castShadow = true;
    this._volumetricShadowMesh = volumetricShadowMesh;
    scene.add(volumetricShadowMesh);

    // ---- floor (tiltable group: energy tilts it, the ball rolls on it) -----
    const floorGroup = new THREE.Group();
    floorGroup.position.set(
      0,
      -VOLUME_WORLD_SIZE_Y / 2 + 0.4 + VOLUME_WORLD_SIZE_Y / 2 + 0.4,
      0,
    );
    this._floorGroup = floorGroup;
    // rubber floor (Rubber002 from ambientCG), semi-transparent so the void
    // shows through behind it.
    const maxAniso = this.renderer?.capabilities?.getMaxAnisotropy?.() ?? 8;
    const floorMat = new THREE.MeshStandardMaterial({
      color: 0xffffff,
      roughness: 0.9,
      metalness: 0,
      transparent: true,
      opacity: 0.45,
    });
    loadTexture(floorDiffUrl).then((tex) => {
      tex.colorSpace = THREE.SRGBColorSpace;
      tex.wrapS = THREE.RepeatWrapping;
      tex.wrapT = THREE.RepeatWrapping;
      tex.anisotropy = maxAniso;
      tex.repeat.set(FLOOR_PLANE / FLOOR_TILE, FLOOR_PLANE / FLOOR_TILE);
      floorMat.map = tex;
      floorMat.needsUpdate = true;
    });
    loadTexture(floorRoughUrl).then((tex) => {
      tex.wrapS = THREE.RepeatWrapping;
      tex.wrapT = THREE.RepeatWrapping;
      tex.anisotropy = maxAniso;
      tex.repeat.set(FLOOR_PLANE / FLOOR_TILE, FLOOR_PLANE / FLOOR_TILE);
      floorMat.roughnessMap = tex;
      floorMat.needsUpdate = true;
    });
    const floorPlane = new THREE.Mesh(
      new THREE.PlaneGeometry(FLOOR_PLANE, FLOOR_PLANE),
      floorMat,
    );
    floorPlane.rotation.x = -Math.PI / 2;
    floorPlane.receiveShadow = true;
    floorGroup.add(floorPlane);
    scene.add(floorGroup);
    // The floor stays FIXED at the origin - the one world-anchored reference
    // in the scene (the ball is rim-bounded to BALL_ROVE_RADIUS, well inside
    // the 400m plane), so the ball's music-driven roving reads as it moving
    // across the floor texture.

    // ---- lava sphere (replaces the teapot) --------------------------------
    const sphereMaterial = new THREE.MeshStandardMaterial({
      color: 0x000000,
      roughness: 1.0,
      metalness: 0.0,
    });
    loadTexture(sphereDiffUrl).then((tex) => {
      tex.colorSpace = THREE.SRGBColorSpace;
      sphereMaterial.map = tex;
      sphereMaterial.needsUpdate = true;
    });

    sphereMaterial.emissiveNode = Fn(() => {
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
        uFireHue,
      );
      const tempScale = uEmitTemperature.div(33.36).max(0.0);
      const tempFactor = tempScale.pow(4.0);
      const densityScale = uEmitDensity.div(44.08).max(0.0);
      const fadeIn = smoothstep(0.0, 3.0, uTime);
      return (
        coloredFire
          .mul(tempFactor)
          .mul(densityScale)
          .mul(uFireIntensity)
          .mul(uFlicker)
          .mul(fadeIn)
          // bass-driven self-glow: the base emissive (0.06) sits below the
          // bloom threshold (0.5), so without this the sphere never blooms.
          // 1.5 + 5.3*bass lifts it to ~0.44 at silence and ~2.0 at full bass.
          .mul(float(1.5).add(uBassGlow.mul(5.3)))
      );
    })(); // ponytail: emissiveNode is a value node; an uninvoked Fn() would warn per build

    const sphere = new THREE.Mesh(sphereGeometry, sphereMaterial);
    sphere.receiveShadow = true;
    sphere.position.set(0, SPHERE_RADIUS, 0);
    // ponytail: the sphere is a SCENE child (not a floor-group child), so the
    // floor group's tilt rotation can't swing its world position; its x/z and
    // height are set directly in _animate from the ball's roll coords and the
    // floor surface under it. The fire sim grid origin follows the ball.
    scene.add(sphere);
    this.sphere = sphere;

    // ---- fire point light (attached to sphere) ----------------------------
    const isVolume = Fn(({ material }) => {
      return float(material && material.isVolumeNodeMaterial ? 1.0 : 0.0);
    })();

    const pointLightColor = Fn(() => {
      const P = positionWorld;
      const A = uSpherePosition;
      const H = vec3(0.0, uFlameHeight, 0.0);
      const V = P.sub(A);
      const t = V.dot(H).div(H.dot(H)).clamp(0.0, 1.0);
      const C = A.add(uSway).add(H.mul(t));
      const distToSegment = P.sub(C).length();

      const r = float(1.2);
      const softAttenuation = float(1.0).div(
        distToSegment.pow(2.0).add(r.pow(2.0)),
      );
      const distToLight = P.sub(A).length();
      const decayExponent = float(2.0);
      const defaultAttenuation = distToLight
        .pow(decayExponent)
        .max(0.01)
        .reciprocal();
      const attenuationCorrection = isVolume
        .equal(1.0)
        .select(softAttenuation.div(defaultAttenuation), float(1.0));
      const currentIntensity = isVolume
        .equal(1.0)
        .select(uPointLightVolumeIntensity, uPointLightSurfaceIntensity);

      const colorT = uEmitTemperature
        .div(33.36)
        .mul(0.5)
        .add(0.2)
        .add(uColorNoise)
        .clamp(0.0, 1.0);
      const fireColor = fireRamp(colorT);
      const coloredFire = fireHue(
        saturation(fireColor, uniform(1.1)),
        uFireHue,
      );

      const relP = P.xz.sub(A.xz);
      const angle = atan(relP.y, relP.x);
      const distXZ = relP.length();

      const freqScale = uPointLightProjectionFrequency;
      const angleNoise = mx_noise_float(
        vec3(
          cos(angle).mul(float(1.5).mul(freqScale)),
          sin(angle).mul(float(1.5).mul(freqScale)),
          uTime.mul(0.6),
        ),
      )
        .mul(0.5)
        .add(0.5);
      const centerFadeFactor = smoothstep(
        0.0,
        uPointLightProjectionCenterFade,
        distXZ,
      );
      const cleanAngleNoise = mix(float(1.0), angleNoise, centerFadeFactor);

      const noiseCoord1 = vec3(
        P.x.mul(float(0.6).mul(freqScale)),
        uTime.mul(1.2),
        P.z.mul(float(0.6).mul(freqScale)),
      );
      const projN1 = mx_noise_float(noiseCoord1).mul(0.5).add(0.5);
      const noiseCoord2 = vec3(
        P.x.mul(float(1.5).mul(freqScale)),
        uTime.mul(2.5),
        P.z.mul(float(1.5).mul(freqScale)),
      );
      const projN2 = mx_noise_float(noiseCoord2).mul(0.5).add(0.5);
      const projNoise = projN1.mul(0.65).add(projN2.mul(0.35));
      const projectionIntensity = projNoise.mul(
        cleanAngleNoise.mul(0.5).add(0.5),
      );
      const noiseFadeFactor = distToSegment
        .div(uPointLightProjectionNoiseFade)
        .clamp(0.0, 1.0);
      const finalIntensity = mix(
        projectionIntensity,
        float(1.0),
        noiseFadeFactor,
      );

      const radialTemp = float(1.0)
        .sub(distToSegment.div(uPointLightProjectionRadius))
        .clamp(0.0, 1.0);
      const colorTProj = radialTemp.mul(finalIntensity).clamp(0.0, 1.0);
      const fireColorProj = fireRamp(colorTProj);
      const coloredFireProj = fireHue(
        saturation(fireColorProj, uniform(1.1)),
        uFireHue,
      );

      const finalFireColor = isVolume
        .equal(1.0)
        .select(coloredFire, coloredFireProj);

      const tempScale = uEmitTemperature.div(33.36).max(0.0);
      const tempFactor = tempScale.pow(4.0);
      const densityScale = uEmitDensity.div(44.08).max(0.0);
      const fadeIn = smoothstep(0.0, 3.0, uTime);

      const baseColor = finalFireColor
        .mul(tempFactor)
        .mul(densityScale)
        .mul(uFireIntensity)
        .mul(currentIntensity)
        .mul(uFlicker)
        .mul(fadeIn);

      const distRatio = distToSegment.div(uLightFarDistance).clamp(0.0, 1.0);
      const distanceScale = mix(
        uLightNearIntensity,
        uLightFarIntensity,
        smoothstep(0.0, 1.0, distRatio),
      );
      const finalScale = isVolume.equal(1.0).select(distanceScale, float(1.0));

      return baseColor.mul(attenuationCorrection).mul(finalScale);
    })();

    const pointLight = new THREE.PointLight(0xffffff, 1, 100, 2);
    pointLight.colorNode = pointLightColor;
    pointLight.position.set(0, 0, 0);
    pointLight.castShadow = false;
    sphere.add(pointLight);

    // ---- key light (overhead SPOTLIGHT on the ball) -----------------------
    // A cone of light from ~20m above the ball, aimed straight down at it.
    // Intensity is candela (physical units): 700 / 20² ≈ 1.75 at the ball,
    // matching the old directional's 1.8. The cone is visible through the
    // fire volume as a shaft.
    const keyLight = new THREE.SpotLight(0xffffff, 700, 0, 0.4, 0.6, 2);
    keyLight.position.set(0, 0, 0); // repositioned above the ball each frame
    keyLight.castShadow = true;
    keyLight.shadow.intensity = 0.6;
    keyLight.shadow.mapSize.width = 1024;
    keyLight.shadow.mapSize.height = 1024;
    keyLight.shadow.camera.near = 1;
    keyLight.shadow.camera.far = 60;
    keyLight.shadow.bias = -0.001;
    keyLight.shadow.focus = 1;
    scene.add(keyLight);
    scene.add(keyLight.target);
    this._keyLight = keyLight;

    uKeyLightPos.value.copy(keyLight.position);

    // ---- render pipeline (scene + volumetric + denoise + bloom) -----------
    const renderPipeline = new THREE.RenderPipeline(renderer);

    const LAYER_VOLUMETRIC_LIGHTING = 10;
    const volumetricLayer = new THREE.Layers();
    volumetricLayer.disableAll();
    volumetricLayer.enable(LAYER_VOLUMETRIC_LIGHTING);

    volumetricMesh.layers.disableAll();
    volumetricMesh.layers.enable(LAYER_VOLUMETRIC_LIGHTING);
    keyLight.layers.enable(LAYER_VOLUMETRIC_LIGHTING);
    pointLight.layers.enable(LAYER_VOLUMETRIC_LIGHTING);

    const scenePass = pass(scene, camera);
    scenePass.name = "Scene Pass";

    const volumetricPass = pass(scene, camera);
    volumetricPass.name = "Volumetric Lighting";
    volumetricPass.setLayers(volumetricLayer);
    volumetricPass.setResolutionScale(0.5);

    const blurredVolumetricPass = gaussianBlur(volumetricPass, uniform(0.5), 1);

    const volumetricRGB = blurredVolumetricPass.rgb;
    const sRGB = saturation(volumetricRGB, uniform(1.1));
    const adjustedVolumetric = vec4(sRGB, blurredVolumetricPass.a).mul(0.5);
    const scenePassColor = scenePass
      .max(adjustedVolumetric)
      .add(adjustedVolumetric);

    const bloomPass = bloom(scenePassColor);
    // ponytail: these stay fixed (no GUI); the audio drives strength per-frame.
    bloomPass.threshold.value = 0.5;
    bloomPass.radius.value = 1.0;
    bloomPass.strength.value = BLOOM_BASE;

    // Transparent overlay: alpha = max(rgb) keeps empty pixels clear so the
    // base layer shows through; Ball of Fire forces alpha 1 (opaque).
    const outColor = scenePassColor.add(bloomPass);
    renderPipeline.outputNode = this._transparentOverlay
      ? vec4(outColor.rgb, max(outColor.r, max(outColor.g, outColor.b)))
      : vec4(outColor.rgb, 1.0);

    // stash refs for the animation loop
    this.renderPipeline = renderPipeline;
    this._uTime = uTime;
    this._uDt = uDt;
    this._uFlameHeight = uFlameHeight;
    this._uFireSize = uFireSize;
    this._uSway = uSway;
    this._uGridOrigin = uGridOrigin;
    this._uKeyLightPos = uKeyLightPos;
    this._uFlicker = uFlicker;
    this._uColorNoise = uColorNoise;
    this._uSphereMatrix = uSphereMatrix;
    this._uSphereSpeed = uSphereSpeed;
    this._uSphereVelocity = uSphereVelocity;
    this._uSpherePosition = uSpherePosition;
    this._uEmitTemperature = uEmitTemperature;
    this._uEmitDensity = uEmitDensity;
    this._uFireIntensity = uFireIntensity;
    this._uBassGlow = uBassGlow;
    this._uTurbulence = uTurbulence;
    this._uDissipation = uDissipation;
    this._uCooling = uCooling;
    this._uFireStartColor = uFireStartColor;
    this._uFireMidColor = uFireMidColor;
    this._uFireEndColor = uFireEndColor;
    this._uFireHue = uFireHue;
    this._pointLight = pointLight;
    this._bloomPass = bloomPass;
    this._cpuNoise = cpuNoise;
    this._emitSpherePass = emitSpherePass;

    this._computePasses = {
      advectVelocityPass,
      divergencePass,
      jacobiPassAB,
      jacobiPassBA,
      projectPass,
      advectDyePass,
      emitSpherePass,
    };
    this._dyeTexNode = dyeTexNode;
    this._dyeTexWriteNode = dyeTexWriteNode;
    this._simulationTime = 0;
    this._simAccumulator = 0;
    // ponytail: preset defaults from the example's params (simSpeed 1.2,
    // smokeLifespan 3.5s, fireLifespan 1.3s).
    this._simSpeed = 1.2;
    // ponytail: lifespans cut ~2x so smoke dies before reaching the (now much
    // larger) grid walls - the anti-"stuck in a room" tuning. smokeLifespan
    // feeds uDissipation (1/life), fireLifespan feeds uCooling (1/life).
    this._smokeLifespan = 1.5;
    this._fireLifespan = 0.9;
  }

  _updateTemporalUniforms(time) {
    const { _cpuNoise: noise } = this;
    this._uTime.value = time % 1000;
    this._uFlameHeight.value = 3.5 + noise.noise(0, time * 2.5, 0) * 0.8;
    this._uSway.value.set(
      noise.noise(time * 3.5, 0, 0) * 0.4,
      0,
      noise.noise(0, 0, time * 3.5) * 0.4,
    );
    this._uFlicker.value =
      noise.noise(0, time * 0.8, 0) * 0.12 +
      noise.noise(0, time * 15.0, 0) * 0.06 +
      0.82;
    this._uColorNoise.value = noise.noise(time * 5.0, time * 5.0, 0) * 0.08;

    // The sphere world matrix is set in _animate AFTER the floor tilt + roll are
    // applied (so the ball always sits exactly on the current floor); here we
    // only feed the sim/emission from the sphere's matrix for this substep.
    this._uSphereMatrix.value.copy(this.sphere.matrixWorld);
    this._uSpherePosition.value.copy(
      this.sphere.getWorldPosition(this._camPos),
    );
    // Centre the ball in the grid VERTICALLY: uGridOrigin is the grid's
    // bottom-centre, so offset it VY/2 below the ball - then the ball maps
    // to uvw.y=0.5 and the radial force can push fire both above AND below.
    this._uGridOrigin.value.set(
      this.sphere.position.x,
      this.sphere.position.y - VOLUME_WORLD_SIZE_Y / 2,
      this.sphere.position.z,
    );
  }

  _animate() {
    // ponytail: async backend init - no-op until renderer.init() resolved.
    if (!this._initialized) return;
    // modV: the worker loop hands us the frame delta instead of a clock.
    const delta = Math.min(this._dt || 1 / 60, 1 / 30);

    this._simAccumulator += delta * this._simSpeed;
    const simStep = (1 / 120) * this._simSpeed;
    const maxAccumulator = simStep * 8;
    if (this._simAccumulator > maxAccumulator)
      this._simAccumulator = maxAccumulator;

    this._uDt.value = simStep;
    this._uTurbulence.value =
      this._simSpeed > 0 ? 3.2 / Math.sqrt(this._simSpeed) : 0;
    this._uDissipation.value =
      this._smokeLifespan >= 100 ? 0 : 1 / this._smokeLifespan;
    this._uCooling.value = 1 / this._fireLifespan;

    while (this._simAccumulator >= simStep) {
      this._simulationTime += simStep;
      this._updateTemporalUniforms(this._simulationTime);

      this.renderer.compute(this._computePasses.advectVelocityPass);
      this.renderer.compute(this._computePasses.divergencePass);
      for (let i = 0; i < PRESSURE_ITERATIONS; i++) {
        this.renderer.compute(
          i % 2 === 0
            ? this._computePasses.jacobiPassAB
            : this._computePasses.jacobiPassBA,
        );
      }
      this.renderer.compute(this._computePasses.projectPass);
      this.renderer.compute(this._computePasses.advectDyePass);
      this.renderer.compute(this._emitSpherePass);

      const temp = this._dyeTexNode.value;
      this._dyeTexNode.value = this._dyeTexWriteNode.value;
      this._dyeTexWriteNode.value = temp;

      this._simAccumulator -= simStep;
    }

    // bass -> bloom strength (the musical response)
    if (this._bloomPass) {
      this._bloomPass.strength.value = Math.min(
        BLOOM_BASE + this._bass * BLOOM_PER_BASS,
        4.0,
      );
    }
    this._uBassGlow.value = this._bass;

    // floor tilt + ball rolling: apply the audio-smoothed tilt to the floor
    // group, roll the ball down the incline in the floor's local frame, then
    // the sphere rides the plane (placed in _updateTemporalUniforms each sim
    // step from this._ball).
    // The floor stays level and FIXED at the origin - it is the only world-
    // anchored reference, so the ball's music-driven roving shows up as it
    // moving over the floor texture (everything else - camera, fire grid, key
    // light - follows the ball). The ball is rim-bounded to BALL_ROVE_RADIUS
    // so it stays on the plane and bounces instead of running off.
    this._floorGroup.updateMatrixWorld();
    // the fire volume boxes ride the grid origin (which follows the ball)
    const gy = this._uGridOrigin.value.y;
    this._volumetricMesh.position.set(
      this._uGridOrigin.value.x,
      gy + VOLUME_WORLD_SIZE_Y / 2,
      this._uGridOrigin.value.z,
    );
    this._volumetricShadowMesh.position.copy(this._volumetricMesh.position);
    // The ball accelerates with the music: a bass-driven push plus a beat
    // velocity kick (from update()); rollBall applies the damping, speed cap
    // and the rim that keeps it roving on the fixed floor. Skipped when frozen
    // (Ball of Fire pins the ball in place).
    if (!this._freezeBall) {
      this._ball.vx += this._kickX;
      this._ball.vz += this._kickZ;
      this._ball = rollBall(
        this._ball,
        [this._accelX, 0, this._accelZ],
        delta,
        { radius: BALL_ROVE_RADIUS },
      );
    }
    this._kickX = 0;
    this._kickZ = 0;

    // The ball floats at a constant height above the level floor: the floor's
    // surface is at the group's y, so the gap is always SPHERE_RADIUS - the
    // same distance from the floor wherever it roves.
    this.sphere.position.set(
      this._ball.x,
      this._floorGroup.position.y + SPHERE_RADIUS,
      this._ball.z,
    );
    this.sphere.rotation.y += delta * 0.25;
    this.sphere.updateMatrixWorld();

    // camera: hold a constant WORLD offset from the ball (so its on-screen size
    // never changes), with a slow music-driven orbit (bass yaw) and a gentle
    // lerp so the music-driven ball doesn't jerk the view.
    const n2 = this._cpuNoise;
    const swayT = this.time;
    const camera = this.camera;
    camera.up.set(0, 1, 0);
    const ballWorld = this.sphere.getWorldPosition(this._camPos);
    const cy = Math.cos(this._camYaw),
      sy = Math.sin(this._camYaw);
    const desired = this._camFollow.set(
      ballWorld.x + this._camBase.x * cy - this._camBase.z * sy,
      ballWorld.y + this._camBase.y,
      ballWorld.z + this._camBase.x * sy + this._camBase.z * cy,
    );
    camera.position.lerp(desired, Math.min(1, delta * 1.2));
    camera.position.x += n2.noise(swayT * 0.25, 0, 0) * 0.15;
    camera.position.z += n2.noise(0, 0, swayT * 0.3) * 0.15;
    // keep the ball near centre but aim slightly BELOW it so the floor always
    // stays in frame.
    camera.lookAt(ballWorld.x, ballWorld.y - FLOOR_AIM_DOWN, ballWorld.z);

    // key light: overhead spotlight, aimed straight down at the ball (tracks
    // the ball's X/Z so the fire is always lit).
    this._keyLight.position.set(ballWorld.x, ballWorld.y + 20, ballWorld.z);
    this._keyLight.target.position.copy(ballWorld);
    this._uKeyLightPos.value.copy(this._keyLight.position);

    // point light range tracks fire size (from the example's animate loop)
    const tempRatio = this._uEmitTemperature.value / 33.36;
    const densityRatio = this._uEmitDensity.value / 44.08;
    const intensityRatio = this._uFireIntensity.value / 5.63;
    const sizeFactor = Math.sqrt(tempRatio * densityRatio * intensityRatio);
    const t = Math.min(Math.max(this._simulationTime / 3.0, 0.0), 1.0);
    const fadeIn = t * t * (3.0 - 2.0 * t);
    this._pointLight.distance = Math.max(
      0.01,
      40.0 * Math.max(0.2, sizeFactor) * fadeIn,
    );

    return this.renderPipeline.render();
  }
}

// Ball of Fire - RollingFire with the ball pinned at its start position, floating
// above the floor at a constant height; the fire, bloom, colour drops and floor
// tilt stay fully audio-reactive.
export class BallOfFire extends RollingFire {
  constructor(renderer, camera, host) {
    super(renderer, camera, host);
    this.label = "Ball of Fire";
    this.category = "Fire";
    this._freezeBall = true;
    // Not a layer: plays standalone (only Rolling Fire itself is
    // foreground-pinned). Opaque canvas - nothing beneath it in that role.
    this._transparentOverlay = false;
    this.role = "standalone";
  }
}
