// Latent Backrooms: the standalone Point10 backrooms world (12 procedural
// rooms stitched into one endless corridor) ported to a native modV Point10
// scene. The source drove the camera with pointer-lock first person input;
// the worker has no DOM, so that path is removed and the scene's built-in
// demo camera walks the corridor automatically instead. Everything else -
// room kinds, palettes, glyph walls, chevrons, lights, fog and the
// Afterimage -> Bloom -> CRT -> Output composer chain - follows the source.
//
// Canvas textures are built with OffscreenCanvas because the worker has no
// document; in Node (unit tests) the canvas helpers return null and the
// materials simply fall back to their colour/emissive values.

import * as THREE from "three";
import { EffectComposer } from "three/examples/jsm/postprocessing/EffectComposer.js";
import { AfterimagePass } from "three/examples/jsm/postprocessing/AfterimagePass.js";
import { RenderPass } from "three/examples/jsm/postprocessing/RenderPass.js";
import { ShaderPass } from "three/examples/jsm/postprocessing/ShaderPass.js";
import { UnrealBloomPass } from "three/examples/jsm/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/examples/jsm/postprocessing/OutputPass.js";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../plane9/P9Audio.js";

export const TAU = Math.PI * 2;
export const ROOM_COUNT = 12;
export const latentRoomSpacing = 56;
export const START_Z = 10;
export const ROOM_NAMES = [
  "The Yellow Vestibule",
  "The Long Library",
  "Hall Of Unsaid Things",
  "Server Nave",
  "The Forked Archive",
  "The Room That Waits",
  "Cathedral Of Context",
  "The Blue Corridor",
  "Garden Of Recurrence",
  "The Empty Wing",
  "Prediction Chapel",
  "The Quiet Wing",
];
export const ROOM_KINDS = [
  "backrooms",
  "library",
  "hall",
  "server",
  "maze",
  "archive",
  "cathedral",
  "server",
  "garden",
  "void",
  "chapel",
  "backrooms",
];
export const ROOM_CODES = [
  "VESTIBULE / 00",
  "LIBRARY / 17",
  "HALL / 04",
  "SERVER NAVE / 88",
  "FORKED ARCHIVE / 31",
  "WAITING ROOM / 09",
  "CONTEXT CATHEDRAL / 77",
  "BLUE CORRIDOR / 12",
  "RECURRENCE GARDEN / 23",
  "EMPTY WING / 00",
  "PREDICTION CHAPEL / 41",
  "QUIET WING / 01",
];
const PALETTES = [
  {
    background: "#0d0b07",
    wall: "#393021",
    ink: "#e0c47e",
    light: "#ffe0a0",
    accent: "#a98cff",
    trim: "#17130d",
  },
  {
    background: "#050b12",
    wall: "#102431",
    ink: "#8ed6e5",
    light: "#a9eaff",
    accent: "#ff70b7",
    trim: "#081016",
  },
  {
    background: "#100609",
    wall: "#32151a",
    ink: "#efa39b",
    light: "#ffc09b",
    accent: "#7f9cff",
    trim: "#16090d",
  },
  {
    background: "#07100b",
    wall: "#14251c",
    ink: "#a1e2bb",
    light: "#caffd1",
    accent: "#ffb45e",
    trim: "#0a140e",
  },
];
const GLYPHS =
  "01<>{}[]/\\|=+*#@$%&abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ROOM_WORDS = [
  "LATENT",
  "CONTEXT",
  "MEMORY",
  "LOSS",
  "ATTENTION",
  "PREDICTION",
  "PATTERN",
  "SILENCE",
  "RELATION",
  "ORIGIN",
];
// Long tab-suspend frames must not teleport the camera through rooms.
const MAX_DRIVE_DT = 0.1;
const fract = (v) => v - Math.floor(v);
const mod = (v, n) => ((v % n) + n) % n;
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const hash = (seed) => fract(Math.sin(seed * 127.1 + 311.7) * 43758.5453);
const finite = (v, fallback = 0) =>
  typeof v === "number" && Number.isFinite(v) ? v : fallback;
const unit = (v) => clamp(finite(v), 0, 1);

export const createRoomPlan = (index) => {
  const i = Math.max(0, Math.floor(index));
  const a = hash(i * 17.13 + 1.7);
  const b = hash(i * 31.71 + 8.2);
  const c = hash(i * 47.29 + 3.4);
  const e = hash(i * 73.41 + 4.6);
  const f = hash(i * 89.73 + 6.8);
  const g = hash(i * 101.9 + 2.2);
  return {
    index: i,
    kind: ROOM_KINDS[i % ROOM_KINDS.length],
    name: ROOM_NAMES[i % ROOM_NAMES.length],
    code: ROOM_CODES[i % ROOM_CODES.length],
    width: 15 + a * 13,
    height: 8 + b * 14,
    depth: 40 + c * 10,
    palette: Math.floor(i / 3) % PALETTES.length,
    phase: e * TAU,
    complexity: 2 + Math.floor(f * 5),
    bend: (g - 0.5) * 0.7,
  };
};

export const roomCycle = (travel) =>
  Math.floor((travel + latentRoomSpacing * 0.5) / latentRoomSpacing);

export const walkSpeed = (bass = 0, mid = 0, beat = 0) =>
  6.2 +
  clamp(bass, 0, 1) * 6.5 +
  clamp(mid, 0, 1) * 2 +
  clamp(beat, 0, 1) * 3.2;

export const runBob = (travel, phase = 0) =>
  Math.sin(travel * 2.35 + phase) * 0.06 +
  Math.sin(travel * 4.7 + phase * 0.7) * 0.018;

export const runSway = (travel, phase = 0) =>
  Math.sin(travel * 1.18 + phase * 0.5) * 0.035;

export const hallwayRipple = (phase, z) => {
  const wave = Math.max(0, Math.sin(phase + z * 0.32));
  return wave * wave * wave;
};

export const chevronGridZ = (cameraZ, index, spacing = 3.4) => {
  const first = Math.ceil(-cameraZ / spacing) + 1;
  return -(first + index) * spacing;
};

export const rippleSpeed = (bass = 0, mid = 0, beat = 0) =>
  22 + clamp(bass, 0, 1) * 28 + clamp(mid, 0, 1) * 10 + clamp(beat, 0, 1) * 35;

export const lightingDrive = (bass = 0, mid = 0, high = 0, beat = 0) => {
  const b = clamp(bass, 0, 1);
  const m = clamp(mid, 0, 1);
  const h = clamp(high, 0, 1);
  const k = clamp(beat, 0, 1);
  return {
    walk: 1.2 + b * 15 + m * 5 + k * 10,
    rim: 0.8 + m * 10 + h * 8 + k * 9,
    wall: 0.22 + b * 1.1 + m * 0.7 + k * 0.55,
    panel: 0.08 + h * 0.55 + k * 0.28,
    ceiling: 0.28 + b * 0.65 + m * 0.2 + k * 0.35,
    line: 0.07 + h * 0.3 + m * 0.08,
    floor: 0.05 + b * 0.16 + k * 0.08,
    bloom: 0.45 + b * 1.25 + m * 0.65 + k,
  };
};

export const walkVector = (
  travel,
  plan,
  energy = 0,
  mid = 0,
  high = 0,
  beat = 0,
  turn = 0,
) => {
  const bass = clamp(energy, 0, 1);
  const middle = clamp(mid, 0, 1);
  const treble = clamp(high, 0, 1);
  const kick = clamp(beat, 0, 1);
  const sway = Math.sin(travel * 0.14 + plan.phase) * plan.width * 0.075;
  const floatSway =
    Math.sin(travel * 0.39 + plan.phase * 1.7) * plan.width * 0.045;
  const drift =
    Math.sin(travel * 0.047 + plan.phase * 0.7) * plan.width * 0.035;
  const bassTurn =
    bass * Math.sin(travel * 0.29 + plan.phase) * plan.width * 0.035;
  const midTurn =
    middle * Math.sin(travel * 0.095 + plan.phase * 1.2) * plan.width * 0.05;
  const kickTurn =
    kick * Math.cos(travel * 0.18 + plan.phase) * plan.width * 0.035;
  const guidedTurn = turn * plan.width * 0.035;
  const roomBend =
    plan.bend * Math.sin(travel * 0.07 + plan.phase) * plan.width * 0.025;
  return {
    x: clamp(
      sway +
        floatSway +
        drift +
        bassTurn +
        midTurn +
        kickTurn +
        guidedTurn +
        roomBend,
      -plan.width * 0.22,
      plan.width * 0.22,
    ),
    y:
      1.72 +
      Math.sin(travel * 0.27 + plan.phase) * (0.08 + bass * 0.1) +
      Math.sin(travel * 0.51 + plan.phase * 0.6) * 0.035 +
      treble * 0.06,
  };
};

// The automatic demo camera: advance the corridor at walkSpeed, wrap the
// hallway ripple phase and report which room is current. Replaces the source's
// pointer-lock walking while keeping its non-first-person path untouched.
export const latentBackroomsDrive = ({
  travel = 0,
  ripplePhase = 0,
  dt = 0,
  bass = 0,
  mid = 0,
  beat = 0,
  speedScale = 1,
} = {}) => {
  const step = clamp(finite(dt), 0, MAX_DRIVE_DT);
  const b = unit(bass);
  const m = unit(mid);
  const k = unit(beat);
  const scale =
    typeof speedScale === "number" &&
    Number.isFinite(speedScale) &&
    speedScale >= 0
      ? speedScale
      : 1;
  const speed = walkSpeed(b, m, k) * scale;
  const nextTravel = finite(travel) + step * speed;
  const nextRipplePhase = mod(
    finite(ripplePhase) + step * rippleSpeed(b, m, k) * 0.32,
    TAU,
  );
  return {
    step,
    speed,
    travel: nextTravel,
    ripplePhase: nextRipplePhase,
    cycle: roomCycle(nextTravel),
    cameraZ: START_Z - nextTravel,
  };
};

// Where each live room sits relative to the travelling camera. The room slots
// are reused as the camera passes them, exactly like the source update loop.
export const latentRoomLayout = (travel, count = ROOM_COUNT) => {
  const t = finite(travel);
  const cycle = roomCycle(t);
  const rooms = [];
  for (let i = 0; i < count; i++) {
    const absolute = cycle + i;
    rooms.push({
      slot: i,
      absolute,
      index: mod(absolute, count),
      z: START_Z - absolute * latentRoomSpacing,
    });
  }
  return {
    cycle,
    current: mod(cycle, count),
    cameraZ: START_Z - t,
    rooms,
  };
};

const createCanvas = (width, height) => {
  if (typeof OffscreenCanvas === "undefined") return null;
  return new OffscreenCanvas(width, height);
};

const makeGlowTexture = () => {
  const canvas = createCanvas(64, 64);
  if (!canvas) return null;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  const gradient = ctx.createRadialGradient(32, 32, 0, 32, 32, 32);
  gradient.addColorStop(0, "rgba(255,255,255,1)");
  gradient.addColorStop(0.18, "rgba(255,255,255,.8)");
  gradient.addColorStop(1, "rgba(255,255,255,0)");
  ctx.fillStyle = gradient;
  ctx.fillRect(0, 0, 64, 64);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
};

const makeGlyphTexture = (paletteIndex, seed) => {
  const palette = PALETTES[paletteIndex % PALETTES.length];
  const canvas = createCanvas(512, 512);
  if (!canvas) return null;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.fillStyle = palette.background;
  ctx.fillRect(0, 0, 512, 512);
  ctx.font = "7px monospace";
  ctx.textBaseline = "top";
  for (let row = 0; row < 64; row++) {
    const y = row * 8;
    const offset = Math.floor(hash(seed * 19 + row) * 8);
    for (let col = -1; col < 65; col++) {
      const x = col * 8 + offset;
      const density = 0.42 + hash(seed * 53 + row * 11 + col) * 0.48;
      if (hash(seed * 97 + row * 17 + col * 7) > density) continue;
      const glyph =
        GLYPHS[
          Math.floor(hash(seed * 131 + row * 19 + col * 3) * GLYPHS.length)
        ];
      ctx.globalAlpha = 0.48 + hash(seed * 149 + row * 23 + col) * 0.5;
      ctx.fillStyle =
        hash(seed * 173 + row * 5 + col) > 0.84 ? palette.light : palette.ink;
      ctx.fillText(glyph, x, y + 1);
    }
    ctx.globalAlpha = 0.2;
    ctx.fillStyle = palette.ink;
    ctx.fillRect(0, y + 7, 512, 1);
  }
  ctx.globalAlpha = 0.18;
  for (let i = 0; i < 24; i++) {
    const x = Math.floor(hash(seed * 211 + i) * 512);
    const y = Math.floor(hash(seed * 223 + i * 3) * 512);
    const w = 8 + Math.floor(hash(seed * 227 + i * 5) * 40);
    const h = 2 + Math.floor(hash(seed * 229 + i * 7) * 5);
    ctx.fillStyle = i % 3 ? palette.ink : palette.light;
    ctx.fillRect(x, y, w, h);
  }
  ctx.globalAlpha = 1;
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.RepeatWrapping;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.magFilter = THREE.LinearFilter;
  return texture;
};

const cloneMap = (texture, repeatX = 1, repeatY = 1) => {
  if (!texture) return null;
  const map = texture.clone();
  map.needsUpdate = true;
  map.wrapS = THREE.RepeatWrapping;
  map.wrapT = THREE.RepeatWrapping;
  map.repeat.set(repeatX, repeatY);
  return map;
};

export class LatentBackrooms extends BaseScene {
  _init() {
    this.label = "Latent Backrooms";
    this.category = "Latent";
    this.role = "standalone";
    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color(0x050505);
    this.scene.fog = new THREE.FogExp2(0x050505, 0.011);
    this.scene.add(this.group);
    this.travel = 0;
    this.walkX = 0;
    this.walkY = 1.72;
    this.walkVX = 0;
    this.walkVY = 0;
    this.lookX = 0;
    this.lookY = 1.72;
    this.lookVX = 0;
    this.lookVY = 0;
    this.gazeX = 0;
    this.gazeY = 0;
    this.gazeVX = 0;
    this.gazeVY = 0;
    this.turn = 0;
    this.roll = 0;
    this.speed = 0;
    this.ripplePhase = 0;
    this.rippleSpeed = 0;
    this.headBob = 0;
    this.shoulderSway = 0;
    this.smBass = 0;
    this.smMid = 0;
    this.smHigh = 0;
    this.flash = 0;
    this.morph = 0;
    this.rooms = [];
    this.chevrons = [];
    this.chevronGroup = new THREE.Group();
    this.chevronGroup.renderOrder = 3;
    this.chevronGroup.frustumCulled = false;
    this.p9 = null;
    this.currentRoom = null;
    this.glowTexture = makeGlowTexture();
    this.glyphTextures = PALETTES.map((_, i) =>
      makeGlyphTexture(i, i * 101 + 17),
    );
    this.boxGeometry = new THREE.BoxGeometry(1, 1, 1);
    this.planeGeometry = new THREE.PlaneGeometry(1, 1);
    this.orbGeometry = new THREE.IcosahedronGeometry(1, 1);
    this.columnGeometry = new THREE.CylinderGeometry(1, 1, 1, 10);
    this._makeChevrons();
    this.hemisphereLight = new THREE.HemisphereLight(0xb0aa8c, 0x080706, 0.5);
    this.ambientLight = new THREE.AmbientLight(0x77736a, 0.12);
    this.fillLight = new THREE.DirectionalLight(0xd0c8aa, 0.45);
    this.fillLight.position.set(2, 8, 4);
    this.scene.add(this.hemisphereLight, this.ambientLight, this.fillLight);
    this.walkLight = new THREE.PointLight(0xffd28c, 2, 48, 1.8);
    this.rimLight = new THREE.PointLight(0x6fa8c4, 1, 36, 1.7);
    this.scene.add(this.walkLight, this.rimLight);
    for (let i = 0; i < ROOM_COUNT; i++) {
      this.rooms.push(this._makeRoom(createRoomPlan(i)));
    }
    this._makeComposer();

    // modV-facing levers. All default to 1 so the untouched scene matches the
    // source: speedScale = automatic camera speed, bloomScale = bloom
    // strength, trailScale = afterimage persistence, brightnessScale = final
    // CRT grade exposure.
    this.speedScale = 1;
    this.bloomScale = 1;
    this.trailScale = 1;
    this.brightnessScale = 1;
  }

  onActivate() {
    if (!this.afterimagePass || !this.renderer) return;
    const target = this.renderer.getRenderTarget();
    for (const texture of [
      this.afterimagePass.textureComp,
      this.afterimagePass.textureOld,
    ]) {
      if (!texture) continue;
      this.renderer.setRenderTarget(texture);
      this.renderer.clear(true, true, true);
    }
    this.renderer.setRenderTarget(target);
  }

  _addMesh(parent, geometry, material, x, y, z, sx, sy, sz) {
    const mesh = new THREE.Mesh(geometry, material);
    mesh.position.set(x, y, z);
    mesh.scale.set(sx, sy, sz);
    parent.add(mesh);
    return mesh;
  }

  _makeMaterials(plan) {
    const palette = PALETTES[plan.palette];
    const baseMap = this.glyphTextures[plan.palette];
    const wallMap = cloneMap(baseMap, plan.width / 5.5, plan.depth / 5.5);
    const floorMap = cloneMap(baseMap, plan.width / 7, plan.depth / 7);
    const wall = new THREE.MeshStandardMaterial({
      map: wallMap,
      color: 0xdddddd,
      emissive: palette.ink,
      emissiveMap: wallMap,
      emissiveIntensity: 0.22,
      roughness: 0.94,
      metalness: 0.04,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    const floor = new THREE.MeshBasicMaterial({
      map: floorMap,
      color: 0x999999,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    const floorWash = new THREE.MeshBasicMaterial({
      color: palette.wall,
      transparent: true,
      opacity: 0.05,
      depthWrite: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    const trim = new THREE.MeshStandardMaterial({
      color: palette.trim,
      roughness: 0.72,
      metalness: 0.18,
      side: THREE.DoubleSide,
    });
    const panel = new THREE.MeshBasicMaterial({
      map: cloneMap(baseMap, 1.5, 2.5),
      color: palette.light,
      transparent: true,
      opacity: 0.08,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    const light = new THREE.MeshBasicMaterial({
      color: palette.light,
      transparent: true,
      opacity: 0.28,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      toneMapped: false,
    });
    const accent = new THREE.MeshStandardMaterial({
      color: palette.accent,
      emissive: palette.accent,
      emissiveIntensity: 0.25,
      roughness: 0.38,
      metalness: 0.24,
      side: THREE.DoubleSide,
    });
    const runner = new THREE.MeshStandardMaterial({
      color: palette.accent,
      emissive: palette.accent,
      emissiveIntensity: 0.12,
      roughness: 0.98,
      metalness: 0,
      side: THREE.DoubleSide,
    });
    const bookMats = [0, 1, 2].map(
      (i) =>
        new THREE.MeshStandardMaterial({
          color: [palette.ink, palette.light, palette.accent][i],
          emissive: [palette.ink, palette.light, palette.accent][i],
          emissiveIntensity: 0.16 + i * 0.06,
          roughness: 0.86,
          metalness: 0.02,
        }),
    );
    const line = new THREE.LineBasicMaterial({
      color: palette.ink,
      transparent: true,
      opacity: 0.2,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
      toneMapped: false,
    });
    return {
      palette,
      wall,
      floor,
      floorWash,
      trim,
      panel,
      light,
      accent,
      runner,
      bookMats,
      line,
    };
  }

  _makeGrid(width, depth, y, material) {
    const positions = [];
    const xLines = Math.max(5, Math.floor(width / 2.8));
    const zLines = Math.max(8, Math.floor(depth / 2.8));
    for (let i = 0; i <= xLines; i++) {
      const x = -width / 2 + (width * i) / xLines;
      positions.push(x, 0, -depth / 2, x, 0, depth / 2);
    }
    for (let i = 0; i <= zLines; i++) {
      const z = -depth / 2 + (depth * i) / zLines;
      positions.push(-width / 2, 0, z, width / 2, 0, z);
    }
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(positions, 3),
    );
    const grid = new THREE.LineSegments(geometry, material);
    grid.position.y = y;
    return grid;
  }

  _makeChevrons() {
    const material = new THREE.MeshBasicMaterial({
      color: 0xffe0a0,
      transparent: true,
      opacity: 0.16,
      blending: THREE.NormalBlending,
      depthTest: true,
      depthWrite: false,
      side: THREE.DoubleSide,
    });
    const plateMaterial = new THREE.MeshBasicMaterial({
      color: 0x030407,
      transparent: true,
      opacity: 0.42,
      depthWrite: false,
      side: THREE.DoubleSide,
      toneMapped: false,
    });
    this.chevronMaterial = material;
    this.chevronPlateMaterial = plateMaterial;
    for (let i = 0; i < 6; i++) {
      const marker = new THREE.Group();
      const plate = this._addMesh(
        marker,
        this.boxGeometry,
        plateMaterial,
        0,
        0,
        -0.62,
        2.35,
        0.012,
        3.8,
      );
      plate.renderOrder = 1;
      const pixels = [];
      const addPixel = (x, z) => {
        const pixelMaterial = material.clone();
        pixelMaterial.opacity = 0.16;
        const pixel = this._addMesh(
          marker,
          this.boxGeometry,
          pixelMaterial,
          x,
          0,
          z,
          0.2,
          0.014,
          0.2,
        );
        pixel.userData.localZ = z;
        pixel.userData.material = pixelMaterial;
        pixel.userData.baseScale = pixel.scale.clone();
        pixel.renderOrder = 2;
        pixels.push(pixel);
      };
      addPixel(0, -0.62);
      for (const side of [-1, 1]) {
        for (let segment = 0; segment < 4; segment++) {
          addPixel(side * (0.2 + segment * 0.18), -0.38 + segment * 0.34);
        }
      }
      marker.userData.pixels = pixels;
      marker.position.y = 0.012;
      marker.frustumCulled = false;
      this.chevronGroup.add(marker);
      this.chevrons.push(marker);
    }
    this.scene.add(this.chevronGroup);
  }

  _makeRoom(plan) {
    const room = {
      plan,
      group: new THREE.Group(),
      bars: [],
      labels: [],
      movers: [],
      cores: [],
      grids: [],
      lights: [],
      lightFixtures: [],
    };
    Object.assign(room, this._makeMaterials(plan));
    room.group.position.z = -plan.index * latentRoomSpacing;
    this.group.add(room.group);

    const floor = this._addMesh(
      room.group,
      this.planeGeometry,
      room.floor,
      0,
      0,
      0,
      plan.width,
      plan.depth,
      1,
    );
    floor.rotation.x = -Math.PI / 2;
    const floorWash = this._addMesh(
      room.group,
      this.planeGeometry,
      room.floorWash,
      0,
      0.012,
      0,
      plan.width * 0.98,
      plan.depth * 0.98,
      1,
    );
    floorWash.rotation.x = -Math.PI / 2;
    const ceiling = this._addMesh(
      room.group,
      this.planeGeometry,
      room.wall,
      0,
      plan.height,
      0,
      plan.width,
      plan.depth,
      1,
    );
    ceiling.rotation.x = Math.PI / 2;
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.wall,
      -plan.width / 2,
      plan.height / 2,
      0,
      0.2,
      plan.height,
      plan.depth,
    );
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.wall,
      plan.width / 2,
      plan.height / 2,
      0,
      0.2,
      plan.height,
      plan.depth,
    );
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.trim,
      0,
      0.12,
      0,
      plan.width,
      0.24,
      plan.depth,
    );
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.trim,
      -plan.width / 2 + 0.12,
      plan.height / 2,
      0,
      0.24,
      plan.height,
      plan.depth,
    );
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.trim,
      plan.width / 2 - 0.12,
      plan.height / 2,
      0,
      0.24,
      plan.height,
      plan.depth,
    );

    const floorGrid = this._makeGrid(plan.width, plan.depth, 0.035, room.line);
    const ceilingGrid = this._makeGrid(
      plan.width,
      plan.depth,
      plan.height - 0.035,
      room.line,
    );
    room.grids.push(floorGrid, ceilingGrid);
    room.group.add(floorGrid, ceilingGrid);

    for (let i = 0; i < 7; i++) {
      const z = -plan.depth / 2 + ((i + 0.5) * plan.depth) / 7;
      for (const side of [-1, 1]) {
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.trim,
          side * (plan.width / 2 - 0.2),
          plan.height / 2,
          z,
          0.3,
          plan.height * 0.98,
          0.22,
        );
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.panel,
          side * (plan.width / 2 - 0.36),
          plan.height * 0.52,
          z,
          0.025,
          plan.height * 0.72,
          plan.depth * 0.075,
        );
      }
      this._addMesh(
        room.group,
        this.boxGeometry,
        room.trim,
        0,
        plan.height - 0.18,
        z,
        plan.width * 0.94,
        0.24,
        0.22,
      );
      const fixtureMaterial = room.light.clone();
      const fixture = this._addMesh(
        room.group,
        this.boxGeometry,
        fixtureMaterial,
        0,
        plan.height - 0.32,
        z,
        plan.width * 0.33,
        0.035,
        0.07,
      );
      room.lightFixtures.push({
        mesh: fixture,
        material: fixtureMaterial,
        localZ: z,
        baseColor: fixtureMaterial.color.clone(),
      });
    }

    for (const end of [-1, 1]) {
      const portal = new THREE.Group();
      portal.position.z = end * (plan.depth / 2 - 0.28);
      const side = plan.width * 0.32;
      this._addMesh(
        portal,
        this.boxGeometry,
        room.trim,
        -side,
        plan.height * 0.4,
        0,
        0.36,
        plan.height * 0.8,
        0.42,
      );
      this._addMesh(
        portal,
        this.boxGeometry,
        room.trim,
        side,
        plan.height * 0.4,
        0,
        0.36,
        plan.height * 0.8,
        0.42,
      );
      this._addMesh(
        portal,
        this.boxGeometry,
        room.trim,
        0,
        plan.height * 0.78,
        0,
        side * 2,
        0.36,
        0.42,
      );
      const wing = Math.max(0, plan.width * 0.5 - side);
      for (const wingSide of [-1, 1]) {
        this._addMesh(
          portal,
          this.boxGeometry,
          room.wall,
          wingSide * (side + wing * 0.5),
          plan.height * 0.4,
          0,
          wing,
          plan.height * 0.8,
          0.18,
        );
      }
      this._addMesh(
        portal,
        this.boxGeometry,
        room.wall,
        0,
        plan.height * 0.89,
        0,
        side * 2,
        plan.height * 0.22,
        0.18,
      );
      this._addMesh(
        portal,
        this.boxGeometry,
        room.trim,
        0,
        0.08,
        0,
        side * 2,
        0.16,
        0.24,
      );
      this._addMesh(
        portal,
        this.boxGeometry,
        room.panel,
        0,
        plan.height * 0.39,
        end * 0.03,
        side * 1.7,
        plan.height * 0.68,
        0.025,
      );
      room.group.add(portal);
    }

    if (plan.kind === "backrooms") this._addBackrooms(room);
    else if (plan.kind === "library") this._addLibrary(room);
    else if (plan.kind === "hall") this._addHall(room);
    else if (plan.kind === "server") this._addServer(room);
    else if (plan.kind === "maze") this._addMaze(room);
    else if (plan.kind === "archive") this._addArchive(room);
    else if (plan.kind === "cathedral") this._addCathedral(room);
    else if (plan.kind === "garden") this._addGarden(room);
    else if (plan.kind === "void") this._addVoid(room);
    else this._addChapel(room);

    const count = 90 + plan.complexity * 22;
    const dustPositions = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      const angle = hash(plan.index * 101 + i * 7.1) * TAU;
      const radius =
        Math.sqrt(hash(plan.index * 211 + i * 3.7)) * plan.width * 0.44;
      dustPositions[i * 3] = Math.cos(angle) * radius;
      dustPositions[i * 3 + 1] =
        0.4 + hash(plan.index * 307 + i * 9.2) * (plan.height - 0.7);
      dustPositions[i * 3 + 2] =
        (hash(plan.index * 401 + i * 5.3) - 0.5) * plan.depth * 0.9;
    }
    const dustGeometry = new THREE.BufferGeometry();
    dustGeometry.setAttribute(
      "position",
      new THREE.BufferAttribute(dustPositions, 3),
    );
    room.dust = new THREE.Points(
      dustGeometry,
      new THREE.PointsMaterial({
        color: room.palette.ink,
        size: 0.065,
        map: this.glowTexture,
        transparent: true,
        opacity: 0.3,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
        toneMapped: false,
      }),
    );
    room.group.add(room.dust);

    const plaque = this._makePlaque(
      plan.code,
      room.palette.light,
      Math.min(plan.width * 0.26, 3.4),
    );
    if (plaque) {
      plaque.position.set(-plan.width * 0.34, 2.25, -plan.depth * 0.2);
      room.group.add(plaque);
      room.labels.push({
        sprite: plaque,
        baseY: 2.25,
        phase: plan.phase,
        amplitude: 0.025,
      });
    }
    return room;
  }

  _makePlaque(text, color, width) {
    const canvas = createCanvas(768, 128);
    if (!canvas) return null;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    ctx.fillStyle = "rgba(0,0,0,.72)";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.strokeStyle = color;
    ctx.lineWidth = 3;
    ctx.strokeRect(8, 8, canvas.width - 16, canvas.height - 16);
    ctx.fillStyle = color;
    ctx.font = "600 32px monospace";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(text, canvas.width / 2, canvas.height / 2, canvas.width - 48);
    const texture = new THREE.CanvasTexture(canvas);
    texture.colorSpace = THREE.SRGBColorSpace;
    const material = new THREE.SpriteMaterial({
      map: texture,
      transparent: true,
      depthWrite: false,
      toneMapped: false,
    });
    const sprite = new THREE.Sprite(material);
    sprite.scale.set(width, width / 6, 1);
    return sprite;
  }

  _addBackrooms(room) {
    const { plan } = room;
    for (let i = 0; i < 8; i++) {
      const z = -plan.depth * 0.42 + i * plan.depth * 0.12;
      for (const side of [-1, 1]) {
        const x = side * plan.width * 0.42;
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.trim,
          x,
          1.2,
          z,
          0.16,
          2.4,
          0.34,
        );
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.trim,
          x,
          2.45,
          z,
          1.5,
          0.16,
          0.34,
        );
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.panel,
          x - side * 0.1,
          1.2,
          z,
          0.03,
          2.05,
          0.18,
        );
      }
    }
  }

  _addLibrary(room) {
    const { plan } = room;
    const runner = this._addMesh(
      room.group,
      this.boxGeometry,
      room.runner,
      0,
      0.04,
      0,
      plan.width * 0.34,
      0.04,
      plan.depth * 0.9,
    );
    runner.material = room.runner;
    for (const side of [-1, 1]) {
      for (let row = 0; row < 5; row++) {
        const y = 0.8 + (row * (plan.height - 1.6)) / 5;
        const x = side * plan.width * 0.33;
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.trim,
          x,
          y,
          0,
          plan.width * 0.28,
          0.12,
          plan.depth * 0.72,
        );
        for (let col = 0; col < 13; col++) {
          const z = -plan.depth * 0.32 + col * plan.depth * 0.053;
          const h = 0.42 + hash(plan.index * 37 + row * 13 + col) * 0.62;
          const book = this._addMesh(
            room.group,
            this.boxGeometry,
            room.bookMats[(row + col) % 3],
            x - side * 0.14,
            y + h / 2,
            z,
            0.12,
            h,
            0.34,
          );
          room.bars.push({
            mesh: book,
            baseY: y + h / 2,
            pos: col / 12,
            phase: hash(plan.index * 61 + row * 7 + col) * TAU,
          });
        }
      }
    }
    for (let i = 0; i < 6; i++) {
      const z = -plan.depth * 0.36 + i * plan.depth * 0.145;
      const lamp = this._addMesh(
        room.group,
        this.orbGeometry,
        room.light,
        0,
        plan.height * 0.82,
        z,
        0.12,
        0.12,
        0.12,
      );
      room.cores.push(lamp);
    }
  }

  _addHall(room) {
    const { plan } = room;
    const runner = this._addMesh(
      room.group,
      this.boxGeometry,
      room.runner,
      0,
      0.05,
      0,
      plan.width * 0.28,
      0.05,
      plan.depth * 0.94,
    );
    runner.material = room.runner;
    for (let i = 0; i < 6; i++) {
      const z = -plan.depth * 0.39 + i * plan.depth * 0.156;
      for (const side of [-1, 1]) {
        const x = side * plan.width * 0.31;
        const column = this._addMesh(
          room.group,
          this.columnGeometry,
          room.trim,
          x,
          plan.height * 0.43,
          z,
          0.52,
          plan.height * 0.86,
          0.52,
        );
        room.movers.push({
          object: column,
          base: column.position.clone(),
          phase: i * 1.1,
          amplitude: 0.025,
          axis: "y",
        });
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.trim,
          x,
          0.22,
          z,
          1.1,
          0.35,
          1.1,
        );
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.accent,
          x,
          plan.height * 0.78,
          z,
          0.18,
          plan.height * 0.36,
          0.18,
        );
      }
      const beam = this._addMesh(
        room.group,
        this.boxGeometry,
        room.trim,
        0,
        plan.height * 0.8,
        z,
        plan.width * 0.72,
        0.3,
        0.34,
      );
      room.movers.push({
        object: beam,
        base: beam.position.clone(),
        phase: i * 0.8,
        amplitude: 0.03,
        axis: "y",
      });
      this._addMesh(
        room.group,
        this.boxGeometry,
        room.light,
        0,
        plan.height * 0.7,
        z,
        plan.width * 0.12,
        0.05,
        0.12,
      );
    }
  }

  _addServer(room) {
    const { plan } = room;
    for (const side of [-1, 1]) {
      for (let row = 0; row < 4; row++) {
        const y = 0.75 + (row * (plan.height - 1.5)) / 4;
        const x = side * plan.width * 0.34;
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.trim,
          x,
          y,
          0,
          plan.width * 0.22,
          plan.height * 0.2,
          plan.depth * 0.68,
        );
        for (let col = 0; col < 9; col++) {
          const z = -plan.depth * 0.29 + col * plan.depth * 0.073;
          const strip = this._addMesh(
            room.group,
            this.boxGeometry,
            room.accent,
            x - side * 0.13,
            y,
            z,
            0.04,
            plan.height * 0.13,
            0.08,
          );
          room.bars.push({
            mesh: strip,
            baseY: y,
            pos: col / 8,
            phase: col * 0.8,
          });
        }
      }
    }
    for (let i = 0; i < 5; i++) {
      const z = -plan.depth * 0.38 + i * plan.depth * 0.19;
      this._addMesh(
        room.group,
        this.boxGeometry,
        room.accent,
        0,
        plan.height * 0.76,
        z,
        plan.width * 0.24,
        0.035,
        0.08,
      );
    }
  }

  _addMaze(room) {
    const { plan } = room;
    for (let i = 0; i < 11; i++) {
      const side = i % 2 ? 1 : -1;
      const x = side * plan.width * (0.12 + hash(plan.index * 43 + i) * 0.23);
      const z = -plan.depth * 0.4 + i * plan.depth * 0.08;
      const h = plan.height * (0.32 + hash(plan.index * 59 + i) * 0.48);
      const wall = this._addMesh(
        room.group,
        this.boxGeometry,
        room.trim,
        x,
        h / 2,
        z,
        0.24,
        h,
        plan.depth * 0.2,
      );
      room.movers.push({
        object: wall,
        base: wall.position.clone(),
        phase: i * 1.3,
        amplitude: 0.12,
        axis: "x",
      });
      this._addMesh(
        room.group,
        this.boxGeometry,
        room.panel,
        x - side * 0.14,
        h * 0.54,
        z,
        0.025,
        h * 0.76,
        plan.depth * 0.14,
      );
    }
  }

  _addArchive(room) {
    const { plan } = room;
    for (const side of [-1, 1]) {
      for (let row = 0; row < 6; row++) {
        const y = 0.7 + (row * (plan.height - 1.4)) / 6;
        const x = side * plan.width * 0.36;
        this._addMesh(
          room.group,
          this.boxGeometry,
          room.trim,
          x,
          y,
          0,
          plan.width * 0.19,
          0.1,
          plan.depth * 0.84,
        );
        for (let col = 0; col < 10; col++) {
          const z = -plan.depth * 0.37 + col * plan.depth * 0.082;
          const card = this._addMesh(
            room.group,
            this.boxGeometry,
            room.bookMats[col % 3],
            x - side * 0.1,
            y + 0.13,
            z,
            0.035,
            0.24,
            0.32,
          );
          room.bars.push({
            mesh: card,
            baseY: y + 0.13,
            pos: col / 9,
            phase: col * 0.7,
          });
        }
      }
    }
  }

  _addCathedral(room) {
    const { plan } = room;
    for (let i = 0; i < 4; i++) {
      const z = -plan.depth * 0.35 + i * plan.depth * 0.23;
      const width = plan.width * (0.25 + i * 0.035);
      const arch = new THREE.Group();
      this._addMesh(
        arch,
        this.boxGeometry,
        room.trim,
        -width,
        plan.height * 0.29,
        0,
        0.34,
        plan.height * 0.58,
        0.38,
      );
      this._addMesh(
        arch,
        this.boxGeometry,
        room.trim,
        width,
        plan.height * 0.29,
        0,
        0.34,
        plan.height * 0.58,
        0.38,
      );
      this._addMesh(
        arch,
        this.boxGeometry,
        room.light,
        0,
        plan.height * 0.58,
        0,
        width * 1.55,
        0.05,
        0.08,
      );
      arch.position.z = z;
      room.group.add(arch);
    }
    const altar = this._addMesh(
      room.group,
      this.orbGeometry,
      room.accent,
      0,
      1.35,
      -plan.depth * 0.08,
      0.9,
      1.2,
      0.9,
    );
    room.cores.push(altar);
  }

  _addGarden(room) {
    const { plan } = room;
    for (let i = 0; i < 16; i++) {
      const a = i * 2.399 + room.plan.phase;
      const x = Math.cos(a) * plan.width * (0.18 + (i % 3) * 0.1);
      const z = Math.sin(a) * plan.depth * 0.3;
      const stem = this._addMesh(
        room.group,
        this.columnGeometry,
        room.trim,
        x,
        plan.height * 0.3,
        z,
        0.06,
        plan.height * 0.5,
        0.06,
      );
      const bloom = this._addMesh(
        room.group,
        this.orbGeometry,
        i % 2 ? room.accent : room.light,
        x,
        plan.height * 0.6 + (i % 3) * 0.5,
        z,
        0.18,
        0.3,
        0.18,
      );
      room.movers.push({
        object: stem,
        base: stem.position.clone(),
        phase: a,
        amplitude: 0.09,
        axis: "x",
      });
      room.movers.push({
        object: bloom,
        base: bloom.position.clone(),
        phase: a + 1,
        amplitude: 0.16,
        axis: "y",
      });
    }
  }

  _addVoid(room) {
    const { plan } = room;
    const count = 120;
    const positions = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      positions[i * 3] =
        (hash(plan.index * 71 + i * 3) - 0.5) * plan.width * 1.8;
      positions[i * 3 + 1] =
        0.5 + hash(plan.index * 73 + i * 5) * plan.height * 1.4;
      positions[i * 3 + 2] =
        (hash(plan.index * 79 + i * 7) - 0.5) * plan.depth * 1.8;
    }
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    room.stars = new THREE.Points(
      geometry,
      new THREE.PointsMaterial({
        color: room.palette.light,
        size: 0.11,
        map: this.glowTexture,
        transparent: true,
        opacity: 0.7,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
        toneMapped: false,
      }),
    );
    room.group.add(room.stars);
    const core = this._addMesh(
      room.group,
      this.orbGeometry,
      room.accent,
      0,
      plan.height * 0.48,
      -plan.depth * 0.12,
      1.3,
      1.3,
      1.3,
    );
    room.cores.push(core);
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.accent,
      0,
      plan.height * 0.48,
      -plan.depth * 0.12,
      plan.width * 0.3,
      0.04,
      0.04,
    );
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.accent,
      0,
      plan.height * 0.48,
      -plan.depth * 0.12,
      0.04,
      plan.height * 0.3,
      0.04,
    );
  }

  _addChapel(room) {
    const { plan } = room;
    const runner = this._addMesh(
      room.group,
      this.boxGeometry,
      room.runner,
      0,
      0.05,
      0,
      plan.width * 0.26,
      0.05,
      plan.depth * 0.92,
    );
    runner.material = room.runner;
    for (let i = 0; i < 5; i++) {
      const z = -plan.depth * 0.35 + i * plan.depth * 0.18;
      const bench = this._addMesh(
        room.group,
        this.boxGeometry,
        room.trim,
        0,
        0.65,
        z,
        plan.width * 0.6,
        0.16,
        0.36,
      );
      room.movers.push({
        object: bench,
        base: bench.position.clone(),
        phase: i,
        amplitude: 0.025,
        axis: "y",
      });
      this._addMesh(
        room.group,
        this.boxGeometry,
        room.accent,
        -plan.width * 0.27,
        1.8,
        z,
        0.08,
        2.2,
        0.08,
      );
      this._addMesh(
        room.group,
        this.boxGeometry,
        room.accent,
        plan.width * 0.27,
        1.8,
        z,
        0.08,
        2.2,
        0.08,
      );
    }
    this._addMesh(
      room.group,
      this.boxGeometry,
      room.light,
      0,
      plan.height * 0.55,
      -plan.depth * 0.38,
      plan.width * 0.28,
      plan.height * 0.26,
      0.04,
    );
  }

  _makeComposer() {
    if (!this.renderer || !this.camera) return;
    const size = this.renderer.getSize(new THREE.Vector2());
    this.composer = new EffectComposer(this.renderer);
    this.composer.setPixelRatio(this.renderer.getPixelRatio());
    this.composer.addPass(new RenderPass(this.scene, this.camera));
    this.afterimagePass = new AfterimagePass(0.78);
    this.composer.addPass(this.afterimagePass);
    this.bloomPass = new UnrealBloomPass(size, 0.8, 0.9, 0.12);
    this.composer.addPass(this.bloomPass);
    this.crtPass = new ShaderPass({
      uniforms: {
        tDiffuse: { value: null },
        resolution: { value: new THREE.Vector2(size.x, size.y) },
        time: { value: 0 },
        intensity: { value: 0.72 },
        exposure: { value: 1 },
      },
      vertexShader: `
        varying vec2 vUv;
        void main() {
          vUv = uv;
          gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        }
      `,
      fragmentShader: `
        uniform sampler2D tDiffuse;
        uniform vec2 resolution;
        uniform float time;
        uniform float intensity;
        uniform float exposure;
        varying vec2 vUv;
        void main() {
          vec3 color = texture2D(tDiffuse, vUv).rgb;
          float scan = 0.91 + 0.09 * sin(vUv.y * resolution.y * 3.14159265);
          vec2 centered = (vUv - 0.5) * vec2(resolution.x / max(resolution.y, 1.0), 1.0);
          float vignette = 1.0 - smoothstep(0.32, 0.92, length(centered));
          float grain = fract(sin(dot(vUv * resolution + time, vec2(12.9898, 78.233))) * 43758.5453);
          color *= scan * mix(0.78, 1.0, vignette);
          color += (grain - 0.5) * 0.018 * intensity;
          gl_FragColor = vec4(max(color * exposure, 0.0), 1.0);
        }
      `,
    });
    this.composer.addPass(this.crtPass);
    this.composer.addPass(new OutputPass());
    this.composer.setSize(size.x, size.y);
  }

  setAudio(audio) {
    super.setAudio(audio);
    // _init built p9 against a null engine, so it must be recreated with the
    // real audio or every band reads 0 and the corridor stops responding.
    this.p9 = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9?.update(dt);
    const bass = clamp(this.p9?.bands[0] ?? 0, 0, 1);
    const mid = clamp(this.p9?.bands[1] ?? 0, 0, 1);
    const high = clamp(this.p9?.bands[2] ?? 0, 0, 1);
    const beat = clamp(this.p9?.beat() ?? 0, 0, 1);
    this.smBass +=
      (bass - this.smBass) *
      (1 - Math.exp(-dt * (bass > this.smBass ? 10 : 3.5)));
    this.smMid +=
      (mid - this.smMid) * (1 - Math.exp(-dt * (mid > this.smMid ? 9 : 3.5)));
    this.smHigh +=
      (high - this.smHigh) *
      (1 - Math.exp(-dt * (high > this.smHigh ? 8 : 3.5)));
    this.flash = Math.max(beat, this.flash * Math.exp(-dt * 3.8));
    this.morph = Math.max(beat * 0.8, this.morph * Math.exp(-dt * 2.8));
    const motionBeat = this.flash * 0.3;
    const turnTarget =
      Math.sin(this.time * 0.17) * 0.18 + this.smMid * 0.72 + this.flash * 0.18;
    this.turn += (turnTarget - this.turn) * (1 - Math.exp(-dt * 2.8));
    const drive = latentBackroomsDrive({
      travel: this.travel,
      ripplePhase: this.ripplePhase,
      dt,
      bass: this.smBass,
      mid: this.smMid,
      beat: this.flash,
      speedScale: this.speedScale,
    });
    this.speed = drive.speed;
    this.travel = drive.travel;
    this.rippleSpeed = rippleSpeed(this.smBass, this.smMid, this.flash);
    this.ripplePhase = drive.ripplePhase;
    this.headBob = runBob(this.travel, this.currentRoom?.plan.phase ?? 0);
    this.shoulderSway = runSway(this.travel, this.currentRoom?.plan.phase ?? 0);
    const layout = latentRoomLayout(this.travel);
    for (let i = 0; i < this.rooms.length; i++) {
      const placement = layout.rooms[i];
      const room = this.rooms[placement.index];
      room.absolute = placement.absolute;
      room.group.position.z = placement.z;
    }
    this.currentRoom = this.rooms[layout.current];
    const path = walkVector(
      this.travel,
      this.currentRoom.plan,
      this.smBass,
      this.smMid,
      this.smHigh,
      motionBeat,
      this.turn,
    );
    this.walkVX += ((path.x - this.walkX) * 8 - this.walkVX * 4.8) * dt;
    this.walkVY += ((path.y - this.walkY) * 8 - this.walkVY * 4.8) * dt;
    this.walkX += this.walkVX * dt;
    this.walkY += this.walkVY * dt;
    this.walkX = clamp(
      this.walkX,
      -this.currentRoom.plan.width * 0.24,
      this.currentRoom.plan.width * 0.24,
    );
    this.walkY = clamp(this.walkY, 1.42, 2.18);
    const ahead = walkVector(
      this.travel + 9,
      this.currentRoom.plan,
      this.smBass,
      this.smMid,
      this.smHigh,
      motionBeat,
      this.turn,
    );
    this.lookVX +=
      ((ahead.x + this.turn * 0.08 - this.lookX) * 6.2 - this.lookVX * 4.4) *
      dt;
    this.lookVY +=
      ((ahead.y + this.smHigh * 0.08 - 0.08 - this.lookY) * 6.2 -
        this.lookVY * 4.4) *
      dt;
    this.lookX += this.lookVX * dt;
    this.lookY += this.lookVY * dt;
    const gazeTargetX =
      Math.sin(this.time * 0.37) * 0.65 +
      Math.sin(this.travel * 0.16 + (this.currentRoom?.plan.phase ?? 0)) *
        0.35 +
      this.smMid * 1.4;
    const gazeTargetY =
      Math.sin(this.time * 0.29 + 1.2) * 0.32 +
      Math.sin(this.travel * 0.23) * 0.18 +
      this.smHigh * 0.8;
    this.gazeVX += ((gazeTargetX - this.gazeX) * 3.2 - this.gazeVX * 2.8) * dt;
    this.gazeVY += ((gazeTargetY - this.gazeY) * 3.2 - this.gazeVY * 2.8) * dt;
    this.gazeX += this.gazeVX * dt;
    this.gazeY += this.gazeVY * dt;
    const rollTarget =
      Math.sin(this.time * 0.13) * 0.012 +
      this.smHigh * 0.014 +
      this.shoulderSway * 0.18;
    this.roll += (rollTarget - this.roll) * (1 - Math.exp(-dt * 4));
    this._updateRooms(dt);
    this._updateChevrons();
    if (this.crtPass) {
      this.crtPass.uniforms.time.value = this.time;
      this.crtPass.uniforms.exposure.value = this.brightnessScale;
    }
  }

  _updateChevrons() {
    const hue = this.currentRoom?.plan.palette ?? 0;
    const cameraZ = START_Z - this.travel;
    const rippleGain =
      0.12 + this.smBass * 1.4 + this.smMid * 0.6 + this.flash * 0.9;
    for (let i = 0; i < this.chevrons.length; i++) {
      const marker = this.chevrons[i];
      const z = chevronGridZ(cameraZ, i, 3.4);
      if (marker.userData.gridZ !== z) {
        marker.position.set(Math.sin(z * 0.11) * 0.35, 0.012, z);
        marker.userData.gridZ = z;
      }
      for (const pixel of marker.userData.pixels) {
        const material = pixel.userData.material;
        const pulse = hallwayRipple(
          this.ripplePhase,
          z + pixel.userData.localZ,
        );
        material.color.set(PALETTES[hue].ink);
        material.opacity = clamp(
          0.02 + pulse * rippleGain * 0.18 + this.smBass * 0.012,
          0.012,
          0.24,
        );
        const scale = 0.94 + pulse * 0.1 + this.flash * 0.04;
        const baseScale = pixel.userData.baseScale;
        pixel.scale.set(baseScale.x * scale, baseScale.y, baseScale.z * scale);
      }
    }
  }

  _updateRooms(dt) {
    const cameraZ = START_Z - this.travel;
    const drive = lightingDrive(
      this.smBass,
      this.smMid,
      this.smHigh,
      this.flash,
    );
    for (const room of this.rooms) {
      const distance = Math.abs(room.group.position.z - cameraZ);
      const near = clamp(1 - distance / (latentRoomSpacing * 1.4), 0, 1);
      const pulse = 1 + this.morph * 0.018;
      room.group.scale.set(1, pulse, 1);
      room.floorWash.opacity = drive.floor;
      room.panel.opacity = drive.panel;
      room.light.opacity = drive.ceiling;
      const rippleGain =
        0.12 + this.smBass * 1.4 + this.smMid * 0.6 + this.flash * 0.9;
      for (const fixture of room.lightFixtures) {
        const pulse = hallwayRipple(
          this.ripplePhase,
          room.group.position.z + fixture.localZ,
        );
        fixture.material.opacity = clamp(0.035 + pulse * rippleGain, 0, 1);
      }
      room.line.opacity = drive.line + near * 0.04;
      room.wall.emissiveIntensity = drive.wall;
      room.accent.emissiveIntensity = drive.panel * 1.8 + this.smMid * 0.18;
      room.bookMats.forEach((material, i) => {
        material.emissiveIntensity =
          0.04 + this.smMid * (0.1 + i * 0.04) + this.flash * 0.1;
      });
      room.dust.material.opacity = 0.07 + this.smMid * 0.25 + this.flash * 0.18;
      room.dust.rotation.y += dt * (0.008 + this.smHigh * 0.025);
      for (const core of room.cores) {
        const base =
          core.userData.baseScale ??
          (core.userData.baseScale = core.scale.clone());
        core.rotation.y += dt * (0.05 + this.smHigh * 0.24);
        core.rotation.x = Math.sin(this.time * 0.17 + room.plan.phase) * 0.08;
        const corePulse = 1 + this.smBass * 0.08 + this.flash * 0.12;
        core.scale.set(
          base.x * corePulse,
          base.y * corePulse,
          base.z * corePulse,
        );
      }
      for (const bar of room.bars) {
        const value = clamp(this.p9?.spectrum(-1, bar.pos) ?? 0, 0, 1);
        bar.mesh.scale.y = bar.mesh.userData.baseScale?.[1] ?? bar.mesh.scale.y;
        if (!bar.mesh.userData.baseScale) {
          bar.mesh.userData.baseScale = bar.mesh.scale.clone();
        }
        const lift = 0.86 + value * 0.24 + this.smBass * 0.04;
        bar.mesh.scale.y *= lift;
        bar.mesh.position.y =
          bar.baseY + Math.sin(this.time * 1.2 + bar.phase) * 0.012;
      }
      for (const mover of room.movers) {
        const wave =
          Math.sin(this.time * (0.28 + this.smMid * 0.5) + mover.phase) *
          mover.amplitude;
        if (mover.axis === "x") mover.object.position.x = mover.base.x + wave;
        else mover.object.position.y = mover.base.y + wave;
      }
      if (room.stars) {
        room.stars.rotation.y += dt * (0.006 + this.smHigh * 0.018);
        room.stars.material.size = 0.08 + this.smHigh * 0.08;
      }
      for (const label of room.labels) {
        label.sprite.position.y =
          label.baseY +
          Math.sin(this.time * 0.25 + label.phase) * label.amplitude;
        label.sprite.material.opacity = 0.58 + this.smMid * 0.16;
      }
    }
    const hue = this.currentRoom?.plan.palette ?? 0;
    this.scene.background.set(PALETTES[hue].background);
    this.scene.fog.color.set(PALETTES[hue].background);
    this.scene.fog.density = 0.009 + this.smBass * 0.001 + this.flash * 0.001;
    this.hemisphereLight.intensity = 0.35 + drive.wall * 0.25;
    this.ambientLight.intensity = 0.06 + drive.panel * 0.08;
    this.fillLight.intensity = 0.25 + drive.wall * 0.35;
    this.walkLight.position.set(this.walkX, 3.3, cameraZ - 3);
    this.walkLight.color.set(PALETTES[hue].light);
    this.walkLight.intensity = drive.walk;
    this.rimLight.position.set(-this.walkX * 0.6, 2.6, cameraZ - 11);
    this.rimLight.color.set(PALETTES[hue].accent);
    this.rimLight.intensity = drive.rim;
    if (this.afterimagePass?.uniforms?.damp) {
      this.afterimagePass.uniforms.damp.value = clamp(
        (0.48 + this.smHigh * 0.08 + this.flash * 0.04) * this.trailScale,
        0,
        0.95,
      );
    }
    if (this.bloomPass) {
      this.bloomPass.strength = clamp(drive.bloom * this.bloomScale, 0.35, 2.2);
    }
  }

  render() {
    if (!this.renderer || !this.camera) return;
    const cameraZ = START_Z - this.travel;
    this.camera.position.set(
      this.walkX + this.shoulderSway,
      this.walkY + this.headBob,
      cameraZ,
    );
    this.camera.lookAt(
      this.lookX + this.gazeX,
      this.lookY + this.gazeY,
      cameraZ - 10,
    );
    this.camera.rotateZ(this.roll);
    const runFov = clamp((this.speed - 7) * 0.7, 0, 5);
    this.camera.fov = 68 + runFov + this.smBass * 2.4 + this.flash * 1.5;
    this.camera.updateProjectionMatrix();
    if (!this.composer) {
      this.renderer.render(this.scene, this.camera);
      return;
    }
    const renderToScreen = !this._captureNextFrame;
    this._captureNextFrame = false;
    const w = this.renderer.domElement.width;
    const h = this.renderer.domElement.height;
    if (w !== this._cw || h !== this._ch) {
      this._cw = w;
      this._ch = h;
      this.composer.setPixelRatio(this.renderer.getPixelRatio());
      this.composer.setSize(
        w / this.renderer.getPixelRatio(),
        h / this.renderer.getPixelRatio(),
      );
    }
    this.composer.renderToScreen = renderToScreen;
    this.composer.render();
  }

  get paintsCanvas() {
    return true;
  }

  get captureTexture() {
    return this.composer?.readBuffer?.texture ?? null;
  }

  resize(w, h) {
    if (!this.composer) return;
    this.composer.setSize(w, h);
    this.crtPass?.uniforms.resolution.value.set(w, h);
  }
}

export const latentRoomWords = ROOM_WORDS;
