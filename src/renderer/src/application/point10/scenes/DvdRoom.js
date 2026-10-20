// DVD Room: SmokeRoom with the lava sphere swapped for a bouncing DVD logo -
// the logo IS the smoke emitter. The room matches the sim grid (12x12x24,
// same fixed camera); its walls are thin GlassLens-style physical glass with
// a neon edge frame. Like the original screensaver the logo changes colour
// on every wall bounce (corner hits flash white).
// The logo is the Wikimedia DVD textlogo (public domain,
// textures/dvd_logo.svg) extruded via a small SVG path parser +
// ExtrudeGeometry (peak-flow/muffinman pattern, no new dependency); the
// hidden trademark sub-path is skipped before parsing.
// Music: bass pumps smoke emission (inherited) + logo speed, treble drives
// the logo glow, kicks fire a scale pop + speed burst.
//
// modV port of musicvis/src/scenes/DvdRoom.js - inherits the vendored
// SmokeRoom; the SVG is imported with `?raw` because the worker has no
// fetch-relative public dir and Vite inlines it for both dev and prod.
import * as THREE from "three/webgpu";
import { mergeGeometries } from "three/addons/utils/BufferGeometryUtils.js";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";
import {
  SmokeRoom,
  VOLUME_WORLD_SIZE_X,
  VOLUME_WORLD_SIZE_Y,
  VOLUME_WORLD_SIZE_Z,
} from "./SmokeRoom.js";
import dvdLogoSvg from "../textures/dvd_logo.svg?raw";

const LOGO_WIDTH = 4.2; // world units across after normalizing the SVG
const LOGO_DEPTH = 0.35; // world-unit thickness after normalizing
const SVG_DEPTH = 100; // extrusion depth in SVG units (rescaled to LOGO_DEPTH)
const BASE_SPEED = 3.5;
// Classic screensaver palette, cycled (never random) so every bounce changes colour.
const PALETTE = [
  0x00b3ff, 0xff8c1a, 0x59ff5e, 0xff3bff, 0xffe63b, 0xff3b3b, 0xffffff,
];

const NUMBER_RE = /-?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?/g;

function parseNumbers(data) {
  const found = data.match(NUMBER_RE);
  return found ? found.map(Number) : [];
}

// three's SVGLoader parses through DOMParser, which does not exist in modV's
// worker. This covers the command subset the DVD logo uses (M/L/H/V/C/S/Z
// plus Q/T) and returns a ShapePath that toShapes() can extrude.
function parseSvgPath(d) {
  const path = new THREE.ShapePath();
  const point = new THREE.Vector2();
  const control = new THREE.Vector2();
  const firstPoint = new THREE.Vector2();
  const commands = d.match(/[a-df-z][^a-df-z]*/gi) || [];

  for (const command of commands) {
    const type = command.charAt(0);
    const relative = type === type.toLowerCase();
    const numbers = parseNumbers(command.slice(1));

    switch (type.toUpperCase()) {
      case "M":
        for (let j = 0; j < numbers.length; j += 2) {
          point.x = relative ? point.x + numbers[j] : numbers[j];
          point.y = relative ? point.y + numbers[j + 1] : numbers[j + 1];
          control.copy(point);
          if (j === 0) {
            path.moveTo(point.x, point.y);
            firstPoint.copy(point);
          } else {
            path.lineTo(point.x, point.y);
          }
        }
        break;

      case "L":
        for (let j = 0; j < numbers.length; j += 2) {
          point.x = relative ? point.x + numbers[j] : numbers[j];
          point.y = relative ? point.y + numbers[j + 1] : numbers[j + 1];
          control.copy(point);
          path.lineTo(point.x, point.y);
        }
        break;

      case "H":
        for (let j = 0; j < numbers.length; j++) {
          point.x = relative ? point.x + numbers[j] : numbers[j];
          control.copy(point);
          path.lineTo(point.x, point.y);
        }
        break;

      case "V":
        for (let j = 0; j < numbers.length; j++) {
          point.y = relative ? point.y + numbers[j] : numbers[j];
          control.copy(point);
          path.lineTo(point.x, point.y);
        }
        break;

      case "C":
        for (let j = 0; j < numbers.length; j += 6) {
          const baseX = point.x;
          const baseY = point.y;
          const cp1x = relative ? baseX + numbers[j] : numbers[j];
          const cp1y = relative ? baseY + numbers[j + 1] : numbers[j + 1];
          const cp2x = relative ? baseX + numbers[j + 2] : numbers[j + 2];
          const cp2y = relative ? baseY + numbers[j + 3] : numbers[j + 3];
          point.x = relative ? baseX + numbers[j + 4] : numbers[j + 4];
          point.y = relative ? baseY + numbers[j + 5] : numbers[j + 5];
          path.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, point.x, point.y);
          control.set(cp2x, cp2y);
        }
        break;

      case "S":
        for (let j = 0; j < numbers.length; j += 4) {
          const baseX = point.x;
          const baseY = point.y;
          const cp1x = control.x * 2 - baseX;
          const cp1y = control.y * 2 - baseY;
          const cp2x = relative ? baseX + numbers[j] : numbers[j];
          const cp2y = relative ? baseY + numbers[j + 1] : numbers[j + 1];
          point.x = relative ? baseX + numbers[j + 2] : numbers[j + 2];
          point.y = relative ? baseY + numbers[j + 3] : numbers[j + 3];
          path.bezierCurveTo(cp1x, cp1y, cp2x, cp2y, point.x, point.y);
          control.set(cp2x, cp2y);
        }
        break;

      case "Q":
        for (let j = 0; j < numbers.length; j += 4) {
          const baseX = point.x;
          const baseY = point.y;
          const cpx = relative ? baseX + numbers[j] : numbers[j];
          const cpy = relative ? baseY + numbers[j + 1] : numbers[j + 1];
          point.x = relative ? baseX + numbers[j + 2] : numbers[j + 2];
          point.y = relative ? baseY + numbers[j + 3] : numbers[j + 3];
          path.quadraticCurveTo(cpx, cpy, point.x, point.y);
          control.set(cpx, cpy);
        }
        break;

      case "T":
        for (let j = 0; j < numbers.length; j += 2) {
          const baseX = point.x;
          const baseY = point.y;
          const cpx = control.x * 2 - baseX;
          const cpy = control.y * 2 - baseY;
          point.x = relative ? baseX + numbers[j] : numbers[j];
          point.y = relative ? baseY + numbers[j + 1] : numbers[j + 1];
          path.quadraticCurveTo(cpx, cpy, point.x, point.y);
          control.set(cpx, cpy);
        }
        break;

      case "Z":
        path.currentPath.autoClose = true;
        if (path.currentPath.curves.length > 0) {
          point.copy(firstPoint);
          path.currentPath.currentPoint.copy(point);
        }
        break;

      default:
        break;
    }
  }

  return path;
}

function parseSvgPaths(svgText) {
  const paths = [];
  const tagRe = /<path\b[^>]*>/gi;
  let tag;

  while ((tag = tagRe.exec(svgText))) {
    if (/display\s*=\s*"none"/i.test(tag[0])) continue;
    const d = /\bd\s*=\s*"([^"]+)"/i.exec(tag[0]);
    if (d) paths.push(d[1]);
  }

  return paths;
}

export class DvdRoom extends SmokeRoom {
  role = "standalone";

  constructor(renderer, camera, host) {
    super(renderer, camera, host);
    this.label = "DVD Room";
    this.category = "Game";
    // ponytail: DoubleSide - the SVG Y-flip mirrors the winding, so front
    // faces would cull otherwise. Upgrade path is re-winding the index.
    this.logoMat = new THREE.MeshStandardMaterial({
      color: PALETTE[0],
      emissive: PALETTE[0],
      emissiveIntensity: 0.55,
      roughness: 0.35,
      metalness: 0.1,
      side: THREE.DoubleSide,
    });
    // Fallback halves (thin box) until the extruded logo lands and re-measures.
    this.logoHalf = new THREE.Vector3(
      LOGO_WIDTH / 2,
      LOGO_WIDTH / 4,
      LOGO_DEPTH / 2,
    );
    this.pos = new THREE.Vector3(-2, 1, -4);
    this.vel = new THREE.Vector3(1, 0.6, 0.8)
      .normalize()
      .multiplyScalar(BASE_SPEED);
    this.colorIdx = 0;
    this.pop = 0;
    this.cornerFlash = 0;
    this.burst = 0;
    this.bounces = 0;
    this.smBass = 0;
    this.smTreble = 0;
    this.bassAvg = 0;
    this.lastBeat = -1;
    this._orbitYaw = 0; // slow bass-driven camera orbit (Ball of Fire style)
    this._emitGainValue = 1;
  }

  _band(lo, hi) {
    const m = this.audio?.freqMag;
    if (!m) return 0;
    let s = 0;
    for (let i = lo; i < hi && i < m.length; i++) s += m[i];
    return s / Math.max(1, hi - lo);
  }

  // Pure geometry transform, headless-testable: centre extruded SVG
  // geometries on the origin and scale to (width x depth). The old code
  // scaled the SVG-unit extrude depth by the XY factor, leaving a ~0.001
  // world-unit sliver - depth gets its own rescale here.
  static normalizeExtrude(geos, width, depth) {
    const full = new THREE.Box3();
    for (const g of geos) {
      g.computeBoundingBox();
      full.union(g.boundingBox);
    }
    const size = full.getSize(new THREE.Vector3());
    const center = full.getCenter(new THREE.Vector3());
    const s = width / size.x;
    for (const g of geos) {
      g.translate(-center.x, -center.y, -center.z);
      g.scale(s, -s, depth / size.z); // -s: SVG Y runs downwards
    }
    return s;
  }

  static _logoGeometry(svgText) {
    const geos = [];
    for (const d of parseSvgPaths(svgText)) {
      for (const shape of parseSvgPath(d).toShapes()) {
        geos.push(
          new THREE.ExtrudeGeometry(shape, {
            depth: SVG_DEPTH,
            bevelEnabled: true,
            bevelThickness: 3,
            bevelSize: 3,
            bevelSegments: 1,
            curveSegments: 6,
          }),
        );
      }
    }
    if (!geos.length) return null;
    DvdRoom.normalizeExtrude(geos, LOGO_WIDTH, LOGO_DEPTH);
    const merged = geos.length > 1 ? mergeGeometries(geos, false) : geos[0];
    if (!merged) return null;
    geos.forEach((g) => {
      if (g !== merged && g.dispose) g.dispose();
    });
    return merged;
  }

  async _emitterGeometry() {
    // Reference count for emission normalization (no constant duplication).
    const ref = await super._emitterGeometry();
    const refCount = ref.attributes.position.count;
    if (ref.dispose) ref.dispose();
    let geo = null;
    try {
      geo = DvdRoom._logoGeometry(dvdLogoSvg);
    } catch {
      /* fallback box below */
    }
    geo ??= new THREE.BoxGeometry(LOGO_WIDTH, LOGO_WIDTH / 2, LOGO_DEPTH);
    geo.computeBoundingBox();
    this.logoHalf.copy(
      geo.boundingBox.getSize(new THREE.Vector3()).multiplyScalar(0.5),
    );
    // Emission is per-vertex: rescale so the logo seeds like the sphere, then
    // boost - the flat logo's verts share few distinct voxels vs the sphere's
    // 3D spread, so count-parity alone leaves the room thin. ponytail: crude
    // global knob; upgrade path is per-area emission weighting.
    this._emitGainValue =
      (refCount / Math.max(1, geo.attributes.position.count)) * 6;
    return geo;
  }

  _emitterMaterial() {
    return this.logoMat;
  }

  _updateEmitter() {
    if (!this.emitter) return;
    // Grid stays fixed (the room doesn't follow the logo) - only the
    // emission matrix/position/velocity track it.
    this.emitter.updateMatrixWorld();
    this._uSphereMatrix.value.copy(this.emitter.matrixWorld);
    this._uSpherePosition.value.copy(
      this.emitter.getWorldPosition(this._camPos),
    );
    this._uSphereVelocity.value.set(this.vel.x, this.vel.y, this.vel.z);
    this._uSphereSpeed.value = this.vel.length();
  }

  _afterInitVolume(scene) {
    this._uGridOrigin.value.set(0, -VOLUME_WORLD_SIZE_Y / 2, 0);
    if (this._uEmitGain && this._emitGainValue) {
      this._uEmitGain.value = this._emitGainValue;
    }
    // Diffuse fill from high above the room (also the main light on the
    // camera-facing panes - the key comes straight down and gives them ~0).
    scene.add(new THREE.HemisphereLight(0xbfd4ff, 0x0a0a12, 0.9));
    // The fire light only feeds standard materials (logo + glass - the smoke
    // is shader-lit), and at full blast it white-outs the bounce colours.
    if (this._pointLight) this._pointLight.intensity = 0.35;
    // Dimmer fire core than the lava sphere: the burn sits inside the logo and
    // would bury its bounce colour - smoke density is untouched.
    if (this._uFireIntensity) this._uFireIntensity.value = 22.0;
    this._webgpuScene = scene;
    // Thin glass room: plain standard material (physical/clearcoat shaders
    // come out black on this WebGPU backend - verified opaque) with a dark
    // PMREM studio for streaks; the fire light plays across the panes.
    // Dark studio for the panes: a few bright bars on black, PMREM'd with the
    // scene's own renderer. (Raw equirect env maps sample black in this
    // WebGPU pipeline - A/B proved the glass contributes zero - and
    // RoomEnvironment floods the whole room gray.) Falls back to a gradient
    // canvas if PMREM ever throws.
    try {
      if (!this._webgpuRenderer) throw new Error("no renderer");
      const envScene = new THREE.Scene();
      const bar = (w, h, x, y) => {
        const m = new THREE.Mesh(
          new THREE.PlaneGeometry(w, h),
          new THREE.MeshBasicMaterial({
            color: 0xffffff,
            side: THREE.DoubleSide,
          }),
        );
        m.position.set(x, y, 0);
        envScene.add(m);
      };
      bar(1.2, 4, -3.5, 0);
      bar(1.2, 4, 3.5, 0);
      bar(5, 1, 0, 2.5);
      const pmrem = new THREE.PMREMGenerator(this._webgpuRenderer);
      scene.environment = pmrem.fromScene(envScene, 0.04).texture;
      pmrem.dispose();
    } catch {
      if (typeof document !== "undefined") {
        const c = document.createElement("canvas");
        c.width = 512;
        c.height = 256;
        const g = c.getContext("2d");
        const grad = g.createLinearGradient(0, 0, 0, 256);
        grad.addColorStop(0, "#3a4666");
        grad.addColorStop(0.5, "#0a0d16");
        grad.addColorStop(1, "#020307");
        g.fillStyle = grad;
        g.fillRect(0, 0, 512, 256);
        g.fillStyle = "rgba(255,255,255,1.0)";
        g.fillRect(0, 0, 512, 70);
        g.fillRect(0, 186, 512, 70);
        const env = new THREE.CanvasTexture(c);
        env.mapping = THREE.EquirectangularReflectionMapping;
        env.colorSpace = THREE.SRGBColorSpace;
        scene.environment = env;
      }
    }
    this._glassMat = new THREE.MeshStandardMaterial({
      color: 0x9fc4e8,
      metalness: 0.1,
      roughness: 0.15,
      transparent: true,
      opacity: 0.35,
      side: THREE.FrontSide,
      depthWrite: false,
      envMapIntensity: 2.5,
    });
    const glass = new THREE.Mesh(
      // ponytail: slightly larger than the 12x12x24 sim grid - coplanar with
      // the volume boxes the panes z-fight and lose; offset off their faces.
      new RoundedBoxGeometry(
        VOLUME_WORLD_SIZE_X + 0.6,
        VOLUME_WORLD_SIZE_Y + 0.6,
        VOLUME_WORLD_SIZE_Z + 0.6,
        3,
        1.2,
      ),
      this._glassMat,
    );
    this._glassMesh = glass;
    scene.add(glass);
  }

  _bounce() {
    this.bounces++;
    this.colorIdx = (this.colorIdx + 1) % PALETTE.length;
    const c = new THREE.Color(PALETTE[this.colorIdx]);
    this.logoMat.color.copy(c);
    this.logoMat.emissive.copy(c);
    this.pop = 1;
  }

  // Pure position/velocity integration against the room bounds; returns the
  // number of axes bounced this step (2+ = corner hit). Split out so the
  // headless check can drive it without audio or GL.
  stepBounce(dt, speedMul = 1) {
    const b = new THREE.Vector3(
      VOLUME_WORLD_SIZE_X / 2 - this.logoHalf.x,
      VOLUME_WORLD_SIZE_Y / 2 - this.logoHalf.y,
      VOLUME_WORLD_SIZE_Z / 2 - this.logoHalf.z,
    );
    this.pos.addScaledVector(this.vel, dt * speedMul);
    let hit = 0;
    for (const ax of ["x", "y", "z"]) {
      if (this.pos[ax] > b[ax]) {
        this.pos[ax] = b[ax];
        this.vel[ax] = -Math.abs(this.vel[ax]);
        hit++;
      } else if (this.pos[ax] < -b[ax]) {
        this.pos[ax] = -b[ax];
        this.vel[ax] = Math.abs(this.vel[ax]);
        hit++;
      }
    }
    if (hit > 0) {
      this._bounce();
      if (hit >= 2) this.cornerFlash = 1; // ponytail: the mythical corner hit - white flash
    }
    return hit;
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    const bass = this._band(1, 9);
    const m = this.audio?.freqMag;
    let treble = 0;
    if (m?.length) {
      const lo = Math.floor(m.length * 0.85);
      for (let i = lo; i < m.length; i++) treble += m[i];
      treble /= m.length - lo;
    }
    this.smBass +=
      (bass - this.smBass) * Math.min(1, dt * (bass > this.smBass ? 16 : 5));
    this.smTreble +=
      (treble - this.smTreble) *
      Math.min(1, dt * (treble > this.smTreble ? 16 : 5));

    // Kick attack (RocketLeague pattern): sharp jump of the bass envelope.
    this.bassAvg = this.bassAvg * 0.95 + bass * 0.05;
    if (
      bass > 0.07 &&
      bass > this.bassAvg * 1.4 &&
      this.time - this.lastBeat > 0.12
    ) {
      this.lastBeat = this.time;
      this.pop = 1;
      this.burst = Math.min(1.5, this.burst + 0.8);
    }
    this.burst *= Math.exp(-dt * 2);
    this.pop *= Math.exp(-dt * 6);
    this.cornerFlash *= Math.exp(-dt * 3);

    // Bass speeds the logo (smoke emission itself is bass-driven, inherited).
    this.stepBounce(dt, 1 + Math.min(this.smBass, 0.6) + this.burst * 0.5);

    if (this.emitter) {
      this.emitter.position.copy(this.pos);
      // Billboard the logo at the camera with a gentle sway so the extrusion
      // reads in 3D without ever going edge-on.
      const faceYaw = Math.atan2(
        this.camera.position.x - this.pos.x,
        this.camera.position.z - this.pos.z,
      );
      this.emitter.rotation.set(
        Math.sin(this.time * 0.3) * 0.25,
        faceYaw + Math.sin(this.time * 0.4) * 0.5,
        0,
      );
      const s = 1 + this.pop * 0.12;
      this.emitter.scale.set(s, s, s);
    }

    // Free float (Ball of Fire style): slow bass-driven orbit around the room,
    // focused on its centre; the logo bounces through frame.
    this._orbitYaw +=
      (this.smBass * 0.35 - this._orbitYaw) * Math.min(1, dt * 0.4);
    const k = Math.min(1, dt * 1.2);
    this.camera.position.x +=
      (Math.cos(this._orbitYaw) * 17 - this.camera.position.x) * k;
    this.camera.position.y +=
      (6.5 + Math.sin(this.time * 0.23) * 1.2 - this.camera.position.y) * k;
    this.camera.position.z +=
      (Math.sin(this._orbitYaw) * 17 - this.camera.position.z) * k;
    this.camera.lookAt(0, 0, 0);

    // Treble glows the logo (hot emissive so the bounce colour beats the fire
    // light); glass breathes with the bass.
    this.logoMat.emissiveIntensity =
      1.0 + this.smTreble * 2.5 + this.cornerFlash * 1.5;
    if (this._glassMat) this._glassMat.opacity = 0.34 + this.smBass * 0.06;
  }
}
