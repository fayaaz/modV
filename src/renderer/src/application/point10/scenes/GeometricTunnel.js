import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { ShaderPass } from "three/addons/postprocessing/ShaderPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { Line2 } from "three/addons/lines/Line2.js";
import { LineGeometry } from "three/addons/lines/LineGeometry.js";
import { LineMaterial } from "three/addons/lines/LineMaterial.js";
import { P9Audio } from "../p10Audio.js";

export const TUNNEL_BLOOM_LAYER = 1;
export const TUNNEL_HOOP_DEPTH = 0.05;
const OUTER_SIDES = [4, 8, 3, 32, 10, 12];
const INNER_SIDES = [32, 12, 4, 8, 3, 10];
const ringGeometries = new Map();

export const tunnelRingSides = (index, inner = false) => {
  const sides = inner ? INNER_SIDES : OUTER_SIDES;
  return sides[index % sides.length];
};

const roundedPolygon = (path, sides, radius, corner) => {
  const points = Array.from({ length: sides }, (_, i) => {
    const a = (i * Math.PI * 2) / sides;
    return { x: Math.cos(a) * radius, y: Math.sin(a) * radius };
  });
  const r = Math.min(corner, radius * Math.sin(Math.PI / sides) * 0.45);
  for (let i = 0; i < sides; i++) {
    const prev = points[(i + sides - 1) % sides];
    const point = points[i];
    const next = points[(i + 1) % sides];
    const inLen = Math.hypot(point.x - prev.x, point.y - prev.y);
    const outLen = Math.hypot(next.x - point.x, next.y - point.y);
    const startX = point.x - ((point.x - prev.x) / inLen) * r;
    const startY = point.y - ((point.y - prev.y) / inLen) * r;
    const endX = point.x + ((next.x - point.x) / outLen) * r;
    const endY = point.y + ((next.y - point.y) / outLen) * r;
    if (i === 0) path.moveTo(startX, startY);
    else path.lineTo(startX, startY);
    path.quadraticCurveTo(point.x, point.y, endX, endY);
  }
  path.closePath();
};

export const buildTunnelRing = (sides, outer, thick, depth) => {
  const key = `${sides}:${outer}:${thick}:${depth}`;
  if (ringGeometries.has(key)) return ringGeometries.get(key);
  const shape = new THREE.Shape();
  const hole = new THREE.Path();
  const corner = Math.min(0.24, thick * 0.7, outer * 0.08);
  roundedPolygon(shape, sides, outer, corner);
  roundedPolygon(hole, sides, outer - thick, corner);
  shape.holes.push(hole);
  const bevel = Math.min(0.08, thick * 0.3, depth * 0.2);
  const geometry = new THREE.ExtrudeGeometry(shape, {
    depth,
    curveSegments: 6,
    bevelEnabled: true,
    bevelSegments: 2,
    bevelSize: bevel,
    bevelThickness: bevel,
  });
  geometry.translate(0, 0, -depth / 2);
  ringGeometries.set(key, geometry);
  return geometry;
};

export class GeometricTunnel extends BaseScene {
  _init() {
    this.label = "Geometric Tunnel";
    this.category = "Tunnel";
    this.role = "foreground";
    this.screenClearAlpha = 0;
    this.ringCount = 12;
    this.spacing = 3.2;
    this.tunnelLen = this.ringCount * this.spacing;
    this.radius = 4.2;
    this.scroll = 0;
    this.flash = 0;
    this.smBass = 0;
    this.smMid = 0;

    // A real scene root so we can set fog -> the far end fades into a dark
    // blue haze, which is what makes it read as depth, not a flat camera fly.
    this.scene = new THREE.Scene();
    this.scene.fog = new THREE.Fog(0x07071a, 10, this.tunnelLen * 1.3);
    this.scene.add(this.group);

    this.braceGeo = new THREE.BoxGeometry(1, 1, 1);
    this.nodeGeo = new THREE.OctahedronGeometry(0.11, 0);

    this.rings = [];
    for (let i = 0; i < this.ringCount; i++) {
      const sides = tunnelRingSides(i);
      const innerSides = tunnelRingSides(i, true);
      const frameMaterial = this._neonMat();
      const innerMaterial = this._neonMat();
      const frame = new THREE.Mesh(
        buildTunnelRing(sides, this.radius, 0.34, TUNNEL_HOOP_DEPTH),
        frameMaterial,
      );
      const inner = new THREE.Mesh(
        buildTunnelRing(innerSides, this.radius * 0.42, 0.22, 0.035),
        innerMaterial,
      );
      frame.rotation.z = (((i % 3) - 1) * Math.PI) / sides;
      inner.rotation.z = Math.PI / innerSides + (i % 2) * 0.16;
      frame.layers.enable(TUNNEL_BLOOM_LAYER);
      inner.layers.enable(TUNNEL_BLOOM_LAYER);

      const group = new THREE.Group();
      const braces = new THREE.Group();
      const nodes = [];
      const braceCount = i % 2 ? 3 : 4;
      const innerR = this.radius * 0.42 + 0.18;
      const outerR = this.radius - 0.34;
      for (let k = 0; k < braceCount; k++) {
        const a = (k * Math.PI * 2) / braceCount + Math.PI / sides;
        const brace = new THREE.Mesh(this.braceGeo, frameMaterial);
        brace.position.set(
          (Math.cos(a) * (innerR + outerR)) / 2,
          (Math.sin(a) * (innerR + outerR)) / 2,
          0,
        );
        brace.rotation.z = a;
        brace.scale.set(outerR - innerR, 0.07, 0.16);
        brace.frustumCulled = false;
        brace.layers.enable(TUNNEL_BLOOM_LAYER);
        braces.add(brace);

        const node = new THREE.Mesh(this.nodeGeo, frameMaterial);
        node.position.set(Math.cos(a) * innerR, Math.sin(a) * innerR, 0);
        node.frustumCulled = false;
        node.layers.enable(TUNNEL_BLOOM_LAYER);
        braces.add(node);
        nodes.push(node);
      }

      group.frustumCulled = false;
      frame.frustumCulled = inner.frustumCulled = false;

      let ros = null;
      if (i % 3 === 0) {
        ros = new THREE.Group();
        const cluster = this.radius * 0.52;
        const tileMaterial = this._neonMat();
        for (let k = 0; k < sides; k++) {
          const a = (k * Math.PI * 2) / sides;
          const tile = new THREE.Mesh(
            buildTunnelRing(innerSides, this.radius * 0.16, 0.1, 0.02),
            tileMaterial,
          );
          tile.position.set(Math.cos(a) * cluster, Math.sin(a) * cluster, 0);
          tile.rotation.z = a + Math.PI / innerSides;
          tile.frustumCulled = false;
          tile.layers.enable(TUNNEL_BLOOM_LAYER);
          ros.add(tile);
        }
      }

      group.add(frame, inner, braces);
      if (ros) group.add(ros);
      group.position.z = -(((i + 0.5) * this.spacing) % this.tunnelLen);
      this.group.add(group);
      this.rings.push({
        group,
        frame,
        inner,
        ros,
        nodes,
        sides,
        sway: Math.random() * Math.PI * 2,
        swayAxis: Math.random() * Math.PI * 2,
        swayAmp: 0.6 + Math.random() * 0.9,
      });
    }

    const ambient = new THREE.AmbientLight(0x263258, 0.72);
    this.keyLight = new THREE.DirectionalLight(0xb8d5ff, 0.85);
    this.keyLight.position.set(3, 4, 6);
    this.l1 = new THREE.PointLight(0x88aaff, 1.5, 16, 1.4);
    this.l2 = new THREE.PointLight(0xff66aa, 1.0, 16, 1.4);
    this.group.add(ambient, this.keyLight, this.l1, this.l2);
    for (const light of [ambient, this.keyLight, this.l1, this.l2]) {
      light.layers.enable(TUNNEL_BLOOM_LAYER);
    }

    // Neon bolts: a pool of jagged lightning arcing across the FRONT rings (so
    // they read prominently). Each bolt has a thick core, a forked accent, a
    // wider halo, and a pooled colored light that flashes the nearby hoops.
    this.boltLights = [
      new THREE.PointLight(0x4d8cff, 0, 14, 1.6),
      new THREE.PointLight(0xff4fbd, 0, 12, 1.6),
    ];
    for (const light of this.boltLights) {
      light.layers.enable(TUNNEL_BLOOM_LAYER);
      this.group.add(light);
    }
    this.boltLightStrengths = [0, 0];
    this.boltLightPositions = this.boltLights.map(() => new THREE.Vector3());
    this.boltLineMaterials = [];
    this.bolts = [];
    const BOLT_PTS = 17;
    for (let b = 0; b < 10; b++) {
      const coreGeo = new LineGeometry();
      coreGeo.setPositions(new Float32Array(BOLT_PTS * 3));
      const coreMaterial = new LineMaterial({
        color: 0xcceeff,
        // three r131 LineMaterial has no `worldUnits` (added in r152): its
        // linewidth is screen-space pixels, so the world-unit widths of the
        // original are converted to their on-screen equivalent.
        linewidth: 1.6,
        transparent: true,
        opacity: 0,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
        toneMapped: false,
      });
      const core = new Line2(coreGeo, coreMaterial);
      const branchGeo = new LineGeometry();
      branchGeo.setPositions(new Float32Array(5 * 3));
      const branchMaterial = new LineMaterial({
        color: 0xff4fbd,
        linewidth: 1.1,
        transparent: true,
        opacity: 0,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
        toneMapped: false,
      });
      const branch = new Line2(branchGeo, branchMaterial);
      const glowGeo = new THREE.BufferGeometry();
      const glowAttr = new THREE.BufferAttribute(
        new Float32Array((BOLT_PTS - 1) * 6 * 3),
        3,
      );
      glowAttr.setUsage(THREE.DynamicDrawUsage);
      glowGeo.setAttribute("position", glowAttr);
      const glow = new THREE.Mesh(
        glowGeo,
        new THREE.MeshBasicMaterial({
          color: 0x315dff,
          transparent: true,
          opacity: 0,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
          side: THREE.DoubleSide,
        }),
      );
      core.visible = branch.visible = glow.visible = false;
      core.layers.enable(TUNNEL_BLOOM_LAYER);
      branch.layers.enable(TUNNEL_BLOOM_LAYER);
      glow.layers.enable(TUNNEL_BLOOM_LAYER);
      core.frustumCulled = branch.frustumCulled = glow.frustumCulled = false;
      this.group.add(glow, core, branch);
      this.boltLineMaterials.push(coreMaterial, branchMaterial);
      this.bolts.push({
        core,
        branch,
        glow,
        active: false,
        life: 0,
        maxLife: 0.2,
        a: null,
        b: null,
        lightSlot: b % 2,
        phase: Math.random() * 100,
        hue: Math.random(),
        lightPos: new THREE.Vector3(),
        lightStrength: 0,
        corePositions: new Float32Array(BOLT_PTS * 3),
        branchPositions: new Float32Array(5 * 3),
      });
    }
    this._resizeBoltLines();

    const dustCount = 280;
    const dustPositions = new Float32Array(dustCount * 3);
    for (let i = 0; i < dustCount; i++) {
      const a = Math.random() * Math.PI * 2;
      const r = Math.sqrt(Math.random()) * this.radius * 0.9;
      dustPositions[i * 3] = Math.cos(a) * r;
      dustPositions[i * 3 + 1] = Math.sin(a) * r;
      dustPositions[i * 3 + 2] = -Math.random() * this.tunnelLen;
    }
    const dustGeo = new THREE.BufferGeometry();
    dustGeo.setAttribute(
      "position",
      new THREE.BufferAttribute(dustPositions, 3),
    );
    this.dust = new THREE.Points(
      dustGeo,
      new THREE.PointsMaterial({
        color: 0xb8d4ff,
        size: 0.055,
        transparent: true,
        opacity: 0.7,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
        sizeAttenuation: true,
        toneMapped: false,
      }),
    );
    this.dust.frustumCulled = false;
    this.dust.layers.enable(TUNNEL_BLOOM_LAYER);
    this.group.add(this.dust);

    this.target = new THREE.Group();
    this.targetFrame = new THREE.Mesh(
      buildTunnelRing(32, this.radius * 0.82, 0.26, 0.045),
      this._neonMat(),
    );
    this.targetInner = new THREE.Mesh(
      buildTunnelRing(4, this.radius * 0.34, 0.18, 0.03),
      this._neonMat(),
    );
    this.targetInner.rotation.z = Math.PI / 10;
    this.targetFrame.layers.enable(TUNNEL_BLOOM_LAYER);
    this.targetInner.layers.enable(TUNNEL_BLOOM_LAYER);
    this.targetFrame.frustumCulled = this.targetInner.frustumCulled = false;
    this.targetLight = new THREE.PointLight(0x8f7dff, 1.5, 18, 1.5);
    this.targetLight.layers.enable(TUNNEL_BLOOM_LAYER);
    this.target.add(this.targetFrame, this.targetInner, this.targetLight);
    this.target.position.z = -this.tunnelLen - 2;
    this.group.add(this.target);

    if (this.renderer) {
      const size = this.renderer.getSize(new THREE.Vector2());
      const renderPass = new RenderPass(
        this.scene,
        this.camera,
        null,
        0x000000,
        0,
      );
      const bloomTarget = new THREE.WebGLRenderTarget(1, 1, {
        type: THREE.HalfFloatType,
      });
      this.bloomComposer = new EffectComposer(this.renderer, bloomTarget);
      this.bloomComposer.renderToScreen = false;
      this.bloomComposer.setPixelRatio(this.renderer.getPixelRatio());
      this.bloomPass = new UnrealBloomPass(size, 0.72, 1, 0.35);
      this.bloomComposer.addPass(renderPass);
      this.bloomComposer.addPass(this.bloomPass);

      const finalTarget = new THREE.WebGLRenderTarget(1, 1, {
        type: THREE.HalfFloatType,
      });
      this.finalComposer = new EffectComposer(this.renderer, finalTarget);
      this.finalComposer.setPixelRatio(this.renderer.getPixelRatio());
      this.bloomMixPass = new ShaderPass(
        new THREE.ShaderMaterial({
          uniforms: {
            baseTexture: { value: null },
            bloomTexture: { value: this.bloomComposer.renderTarget2.texture },
            bloomStrength: { value: 0.72 },
          },
          vertexShader: `
          varying vec2 vUv;
          void main() {
            vUv = uv;
            gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
          }
        `,
          fragmentShader: `
          uniform sampler2D baseTexture;
          uniform sampler2D bloomTexture;
          uniform float bloomStrength;
          varying vec2 vUv;
          void main() {
            vec3 color = texture2D(baseTexture, vUv).rgb
              + texture2D(bloomTexture, vUv).rgb * bloomStrength;
            float alpha = clamp(max(color.r, max(color.g, color.b)), 0.0, 1.0);
            gl_FragColor = vec4(color, alpha);
          }
        `,
        }),
        "baseTexture",
      );
      this.bloomMixPass.needsSwap = true;
      this.finalComposer.addPass(renderPass);
      this.finalComposer.addPass(this.bloomMixPass);
      this.finalComposer.addPass(new OutputPass());
      this.bloomComposer.setSize(size.x, size.y);
      this.finalComposer.setSize(size.x, size.y);
    }
  }

  _neonMat() {
    return new THREE.MeshStandardMaterial({
      color: 0x080b18,
      emissive: 0xffffff,
      emissiveIntensity: 0.55,
      roughness: 0.24,
      metalness: 0.72,
      flatShading: true,
      side: THREE.DoubleSide,
      depthWrite: true,
    });
  }

  setAudio(audio) {
    super.setAudio(audio);
    this.p9 = new P9Audio(audio);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    this.p9?.update(dt);
    const bass = this.p9 ? this.p9.bands[0] : 0;
    const mid = this.p9 ? this.p9.bands[1] : 0;
    const kick = this.p9 ? this.p9.beat() : 0;

    // One-pole low-pass on bass/mid so the animation glides instead of riding
    // every raw sample (that was the judder). beta from dt gives a ~0.3s lag.
    const beta = 1 - Math.exp(-dt * 3.5);
    this.smBass += (bass - this.smBass) * beta;
    this.smMid += (mid - this.smMid) * beta;
    const b = this.smBass;
    const m = this.smMid;

    this.flash = Math.max(kick, this.flash * Math.exp(-dt * 3));

    // Tunnel speed rides the music: a slow cruise with the intensity driving
    // most of the motion (smoothed b/m already, so it's glidey not juddery).
    // Rings whoosh harder/faster on heavy bass; near-silence drifts gently.
    const turn = 2.6 + b * 5.0 + m * 2.5 + this.flash * 3.0;
    this.scroll += dt * turn;

    this.dust.position.z = 6 - (this.scroll % this.tunnelLen);
    this.dust.rotation.z = this.time * 0.035;
    this.dust.rotation.x = Math.sin(this.time * 0.13) * 0.04;
    this.dust.material.color
      .setHSL((0.58 + this.time * 0.025) % 1, 0.82, 0.72)
      .multiplyScalar(1.35);
    this.dust.material.size = 0.05 + b * 0.025 + this.flash * 0.025;
    this.dust.material.opacity = 0.58 + b * 0.22 + this.flash * 0.16;

    const lightPulse = 0.6 + b * 0.9 + this.flash * 1.6;
    this.keyLight.intensity = 0.65 + lightPulse * 0.28;
    this.l1.intensity = 0.8 + lightPulse * 0.45;
    this.l2.intensity = 0.55 + lightPulse * 0.3;
    this.l1.position.set(Math.sin(this.time * 0.14) * 1.5, 1, 5);
    this.l2.position.set(Math.cos(this.time * 0.2) * 1.5, 0, -30);
    this.targetLight.intensity = 1.25 + b * 0.9 + this.flash * 1.2;
    if (this.bloomPass) {
      this.bloomPass.strength = 0.68 + b * 0.28 + this.flash * 0.22;
      this.bloomMixPass.material.uniforms.bloomStrength.value =
        0.55 + b * 0.14 + this.flash * 0.12;
    }

    const c = new THREE.Color();
    for (let i = 0; i < this.rings.length; i++) {
      const { group, frame, inner, ros, nodes, sway, swayAxis, swayAmp } =
        this.rings[i];
      const PASSTHR = 6;
      const z =
        -(
          ((((i + 0.5) * this.spacing - this.scroll) % this.tunnelLen) +
            this.tunnelLen) %
          this.tunnelLen
        ) + PASSTHR;
      group.position.z = z;
      const depth = 1 + group.position.z / this.tunnelLen;

      const swayOn = (0.3 + b * 1.2) * swayAmp * (0.35 + depth * 0.65);
      group.position.x =
        Math.cos(this.time * 0.5 + sway) * swayOn * Math.cos(swayAxis);
      group.position.y =
        Math.sin(this.time * 0.4 + sway * 1.7) * swayOn * Math.sin(swayAxis);

      group.rotation.z =
        (i % 2 ? 1 : -1) *
        (i * 0.05 + this.scroll * 0.22 + Math.sin(this.time * 0.3 + i) * 0.09);
      const pulse = 1 + b * 0.06 + this.flash * 0.04;
      const near = THREE.MathUtils.clamp((3 - group.position.z) / 7, 0, 1);
      const p = pulse * (0.02 + 0.98 * near * near);
      group.scale.set(p, p, 0.82 + b * 0.22 + this.flash * 0.18);

      const hue = (0.62 + i * 0.045 + this.time * 0.04) % 1;
      const l = Math.min(0.48, 0.34 + m * 0.06 + this.flash * 0.08);
      c.setHSL(hue, 0.9, l);
      const bright = 0.48 + depth * 0.28 + b * 0.2 + this.flash * 0.38;
      frame.material.emissive.copy(c).multiplyScalar(0.42);
      frame.material.emissiveIntensity = bright * 0.78;
      inner.material.emissive.copy(c).multiplyScalar(0.34);
      inner.material.emissiveIntensity = bright * 0.62;

      for (let k = 0; k < nodes.length; k++) {
        const node = nodes[k];
        const nodePulse =
          1 +
          b * 0.65 +
          this.flash * 0.4 +
          Math.sin(this.time * 2.2 + k * 1.7 + i) * 0.08;
        node.scale.setScalar(nodePulse);
        node.rotation.set(this.time * 0.7 + k, this.time * 0.9 + k * 0.4, 0);
      }

      if (ros) {
        const swell = 1 + b * 0.35 + this.flash * 0.1;
        ros.scale.set(swell, swell, 0.9 + b * 0.2);
        ros.rotation.z =
          (i % 2 ? -1 : 1) * Math.sin(this.time * 0.7 + i * 1.7) * 0.35;
        const tileMaterial = ros.children[0].material;
        tileMaterial.emissive.copy(c).multiplyScalar(0.85);
        tileMaterial.emissiveIntensity = bright * 0.82 * near;
      }
    }

    // Lightning: spawn with probability scaled by (damped) music energy, arc
    // between two random polygon corners, live ~0.2s with flicker.
    const energy = Math.min(1, b * 1.2 + m * 0.8);
    if (Math.random() < (0.5 + energy * 2.5) * dt) this._spawnBolt();
    for (const bolt of this.bolts) if (bolt.active) this._updateBolt(bolt, dt);
    this._updateBoltLights();

    this.target.rotation.z = this.time * 0.25;
    this.targetInner.rotation.z = Math.PI / 10 - this.time * 0.2;
    const s = 1 + b * 0.12 + this.flash * 0.08;
    this.target.scale.set(s, s, 0.86 + b * 0.18);
    const tc = new THREE.Color().setHSL((0.55 + this.time * 0.02) % 1, 1, 0.38);
    this.targetFrame.material.emissive.copy(tc).multiplyScalar(1.9);
    this.targetFrame.material.emissiveIntensity =
      0.46 + b * 0.2 + this.flash * 0.35;
    this.targetInner.material.emissive.copy(tc).multiplyScalar(1.55);
    this.targetInner.material.emissiveIntensity =
      0.34 + b * 0.12 + this.flash * 0.24;
  }

  _spawnBolt() {
    const free = this.bolts.find((b) => !b.active);
    if (!free) return;
    // Anchor on the front rings (largest z) so the bolt arcs near the camera.
    const byZ = this.rings
      .map((_, i) => i)
      .sort(
        (x, y) =>
          this.rings[y].group.position.z - this.rings[x].group.position.z,
      );
    const i = byZ[Math.floor(Math.random() * 6)];
    const j = byZ[2 + Math.floor(Math.random() * Math.min(6, byZ.length - 2))];
    free.a = {
      ring: i,
      corner: Math.floor(Math.random() * this.rings[i].sides),
    };
    free.b = {
      ring: j,
      corner: Math.floor(Math.random() * this.rings[j].sides),
    };
    free.life = 0;
    free.maxLife = 0.22 + Math.random() * 0.18;
    free.phase = Math.random() * 100;
    free.hue = Math.random();
    free.core.material.color
      .setHSL(0.54 + free.hue * 0.08, 0.72, 0.82)
      .multiplyScalar(1.05);
    free.glow.material.color
      .setHSL(0.6 + free.hue * 0.08, 0.9, 0.56)
      .multiplyScalar(0.8);
    free.branch.material.color
      .setHSL(0.88 + free.hue * 0.08, 0.9, 0.62)
      .multiplyScalar(0.9);
    free.active = true;
    free.lightStrength = 0;
    free.core.visible = free.branch.visible = free.glow.visible = true;
  }

  _updateBolt(b, dt) {
    b.life += dt;
    const t = b.life / b.maxLife;
    if (t >= 1) {
      b.active = false;
      b.core.visible = b.branch.visible = b.glow.visible = false;
      b.lightStrength = 0;
      return;
    }
    const a = this._cornerWorld(b.a);
    const c = this._cornerWorld(b.b);
    const dist = a.distanceTo(c);
    const pts = this._boltPath(a, c, dist * 0.14, b.phase);
    const branchPts = this._branchPath(pts, dist, b.phase);

    const cPos = b.corePositions;
    for (let i = 0; i < pts.length; i++) {
      cPos[i * 3] = pts[i].x;
      cPos[i * 3 + 1] = pts[i].y;
      cPos[i * 3 + 2] = pts[i].z;
    }
    b.core.geometry.setPositions(cPos);
    const branchPos = b.branchPositions;
    for (let i = 0; i < branchPts.length; i++) {
      branchPos[i * 3] = branchPts[i].x;
      branchPos[i * 3 + 1] = branchPts[i].y;
      branchPos[i * 3 + 2] = branchPts[i].z;
    }
    b.branch.geometry.setPositions(branchPos);

    const gPos = b.glow.geometry.attributes.position.array;
    let v = 0;
    const halfW = 0.14;
    for (let i = 0; i < pts.length - 1; i++) {
      const p = pts[i],
        q = pts[i + 1];
      const dx = q.x - p.x,
        dy = q.y - p.y,
        dz = q.z - p.z;
      const len = Math.hypot(dx, dy, dz) || 1;
      const px = -dy / len,
        py = dx / len;
      const ox = px * halfW,
        oy = py * halfW;
      const quad = [
        p.x - ox,
        p.y - oy,
        p.z,
        p.x + ox,
        p.y + oy,
        p.z,
        q.x - ox,
        q.y - oy,
        q.z,
        q.x - ox,
        q.y - oy,
        q.z,
        q.x + ox,
        q.y + oy,
        q.z,
        p.x + ox,
        p.y + oy,
        p.z,
      ];
      for (const f of quad) gPos[v++] = f;
    }
    b.glow.geometry.attributes.position.needsUpdate = true;

    const flick = Math.min(
      1,
      (1 - t) *
        (0.72 +
          Math.sin(this.time * 72 + b.phase) * 0.16 +
          Math.random() * 0.12) *
        (0.85 + this.flash * 0.45),
    );
    b.glow.material.opacity = flick * 0.78;
    b.core.material.opacity = Math.min(1, flick * 1.2);
    b.branch.material.opacity = flick * 0.65;
    b.lightPos.copy(pts[Math.floor(pts.length * 0.5)]);
    b.lightStrength = flick * (1.2 + this.smBass * 2.4 + this.flash * 1.4);
  }

  _branchPath(points, span, phase) {
    const start = points[Math.floor(points.length * 0.55)];
    const angle = phase * 1.37;
    const end = start
      .clone()
      .add(
        new THREE.Vector3(
          Math.cos(angle) * span * 0.24,
          Math.sin(angle) * span * 0.24,
          Math.sin(phase * 2.1) * span * 0.08,
        ),
      );
    let pts = [start, end];
    for (let level = 0; level < 2; level++) {
      const next = [];
      for (let i = 0; i < pts.length - 1; i++) {
        const p = pts[i],
          q = pts[i + 1];
        next.push(p);
        const mx = (p.x + q.x) / 2,
          my = (p.y + q.y) / 2,
          mz = (p.z + q.z) / 2;
        const dx = q.x - p.x,
          dy = q.y - p.y;
        const len = Math.hypot(dx, dy) || 1;
        const off = Math.sin((level + 1) * 17.3 + phase) * span * 0.06;
        next.push(
          new THREE.Vector3(
            mx - (dy / len) * off,
            my + (dx / len) * off,
            mz + Math.sin((level + 2) * 9.1 + phase) * span * 0.025,
          ),
        );
      }
      next.push(pts[pts.length - 1]);
      pts = next;
    }
    return pts;
  }

  _updateBoltLights() {
    this.boltLightStrengths.fill(0);
    for (const bolt of this.bolts) {
      if (
        !bolt.active ||
        bolt.lightStrength <= this.boltLightStrengths[bolt.lightSlot]
      )
        continue;
      this.boltLightStrengths[bolt.lightSlot] = bolt.lightStrength;
      this.boltLightPositions[bolt.lightSlot].copy(bolt.lightPos);
    }
    for (let i = 0; i < this.boltLights.length; i++) {
      this.boltLights[i].intensity = this.boltLightStrengths[i] * 0.7;
      this.boltLights[i].position.copy(this.boltLightPositions[i]);
    }
  }

  _cornerWorld(anchor) {
    const { ring: index, corner } = anchor;
    const ring = this.rings[index];
    const a = ring.frame.rotation.z + (corner * Math.PI * 2) / ring.sides;
    const v = new THREE.Vector3(
      Math.cos(a) * this.radius,
      Math.sin(a) * this.radius,
      0,
    );
    return ring.group.localToWorld(v);
  }

  _boltPath(a, b, amp, phase) {
    let pts = [a, b];
    for (let level = 0; level < 4; level++) {
      const next = [];
      for (let i = 0; i < pts.length - 1; i++) {
        const p = pts[i],
          q = pts[i + 1];
        next.push(p);
        const mx = (p.x + q.x) / 2,
          my = (p.y + q.y) / 2,
          mz = (p.z + q.z) / 2;
        const dx = q.x - p.x,
          dy = q.y - p.y;
        const len = Math.hypot(dx, dy) || 1;
        const seed = phase + this.time * 17 + level * 19.7 + i * 7.3;
        const off = Math.sin(seed) * amp;
        next.push(
          new THREE.Vector3(
            mx - (dy / len) * off,
            my + (dx / len) * off,
            mz + Math.sin(seed * 1.37) * amp * 0.12,
          ),
        );
      }
      next.push(pts[pts.length - 1]);
      pts = next;
      amp *= 0.5;
    }
    return pts;
  }

  _resizeBoltLines() {
    if (!this.renderer) return;
    const size = this.renderer.getSize(new THREE.Vector2());
    for (const material of this.boltLineMaterials)
      material.resolution.copy(size);
  }

  get paintsCanvas() {
    return true;
  }

  get captureTexture() {
    return this.finalComposer?.readBuffer?.texture ?? null;
  }

  render() {
    const renderToScreen = !this._captureNextFrame;
    this._captureNextFrame = false;
    this.camera.position.set(
      Math.sin(this.time * 0.15) * 0.3,
      Math.cos(this.time * 0.12) * 0.2,
      3,
    );
    this.camera.lookAt(0, 0, -this.tunnelLen * 0.5);
    this.camera.rotateZ(
      Math.sin(this.time * 0.17) * 0.025 + Math.sin(this.time * 0.07) * 0.01,
    );
    this.camera.fov = 64 + this.smBass * 3;
    this.camera.updateProjectionMatrix();

    if (!this.bloomComposer || !this.finalComposer) {
      this.renderer.render(this.scene, this.camera);
      return;
    }
    const w = this.renderer.domElement.width;
    const h = this.renderer.domElement.height;
    if (w !== this._cw || h !== this._ch) {
      this._cw = w;
      this._ch = h;
      const pixelRatio = this.renderer.getPixelRatio();
      this.bloomComposer.setPixelRatio(pixelRatio);
      this.finalComposer.setPixelRatio(pixelRatio);
      this.bloomComposer.setSize(w / pixelRatio, h / pixelRatio);
      this.finalComposer.setSize(w / pixelRatio, h / pixelRatio);
      this._resizeBoltLines();
    }

    const cameraMask = this.camera.layers.mask;
    try {
      this.camera.layers.set(TUNNEL_BLOOM_LAYER);
      this.bloomComposer.render();
    } finally {
      this.camera.layers.mask = cameraMask;
    }
    this.finalComposer.renderToScreen = renderToScreen;
    this.finalComposer.render();
  }

  resize(w, h) {
    if (!this.bloomComposer || !this.finalComposer) return;
    const pixelRatio = this.renderer.getPixelRatio();
    this.bloomComposer.setPixelRatio(pixelRatio);
    this.finalComposer.setPixelRatio(pixelRatio);
    this.bloomComposer.setSize(w, h);
    this.finalComposer.setSize(w, h);
    this._resizeBoltLines();
  }
}
