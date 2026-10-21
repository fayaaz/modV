import { BaseScene } from "../BaseScene.js";
import { createCanvas } from "../canvas.js";
import * as THREE from "three";

export class SpiralParticles extends BaseScene {
  constructor(renderer, camera) {
    super(renderer, camera);
    this.label = "Spiral Particles";
    this.category = "Particle";
  }

  _init() {
    this.armCount = 4;
    this.particleCount = 8000;
    this.geometry = new THREE.BufferGeometry();
    this.positions = new Float32Array(this.particleCount * 3);
    this.colors = new Float32Array(this.particleCount * 3);

    for (let i = 0; i < this.particleCount; i++) {
      const arm = i % this.armCount;
      const armAngle = (arm / this.armCount) * Math.PI * 2;
      const distance = Math.random() * 15;
      const spiralAngle = armAngle + distance * 0.3;
      const spread = (Math.random() - 0.5) * (1 + distance * 0.2);

      this.positions[i * 3] = Math.cos(spiralAngle) * distance + spread;
      this.positions[i * 3 + 1] =
        (Math.random() - 0.5) * (0.5 + distance * 0.05);
      this.positions[i * 3 + 2] = Math.sin(spiralAngle) * distance + spread;

      const hue = 0.6 + distance * 0.02;
      const color = new THREE.Color().setHSL(
        hue % 1,
        0.8,
        0.5 + Math.random() * 0.3,
      );
      this.colors[i * 3] = color.r;
      this.colors[i * 3 + 1] = color.g;
      this.colors[i * 3 + 2] = color.b;
    }

    this.geometry.setAttribute(
      "position",
      new THREE.BufferAttribute(this.positions, 3),
    );
    this.geometry.setAttribute(
      "color",
      new THREE.BufferAttribute(this.colors, 3),
    );

    const canvas = createCanvas(32, 32);
    canvas.width = canvas.height = 32;
    const ctx = canvas.getContext("2d");
    const grad = ctx.createRadialGradient(16, 16, 0, 16, 16, 16);
    grad.addColorStop(0, "rgba(255,255,255,1)");
    grad.addColorStop(1, "rgba(255,255,255,0)");
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, 32, 32);

    this.material = new THREE.PointsMaterial({
      size: 0.1,
      vertexColors: true,
      transparent: true,
      opacity: 0.9,
      sizeAttenuation: true,
      map: new THREE.CanvasTexture(canvas),
      depthWrite: false,
    });

    this.points = new THREE.Points(this.geometry, this.material);
    this.group.add(this.points);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    if (!audioData || audioData.length === 0) return;

    this.points.rotation.y += dt * (0.1 + (audioData[0] / 255) * 0.5);

    const energy = audioData[0] / 255;
    const positions = this.geometry.attributes.position.array;

    for (let i = 0; i < this.particleCount; i++) {
      const x = positions[i * 3];
      const z = positions[i * 3 + 2];
      const dist = Math.sqrt(x * x + z * z);

      if (energy > 0.6) {
        positions[i * 3 + 1] += (Math.random() - 0.5) * energy * 0.2;
      }

      const hue = 0.6 + dist * 0.02;
      const color = new THREE.Color().setHSL(hue % 1, 0.8, 0.5 + energy * 0.3);
      this.colors[i * 3] = color.r;
      this.colors[i * 3 + 1] = color.g;
      this.colors[i * 3 + 2] = color.b;
    }

    this.geometry.attributes.position.needsUpdate = true;
    this.geometry.attributes.color.needsUpdate = true;
  }
}
