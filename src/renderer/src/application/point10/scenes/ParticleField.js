import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";

export class ParticleField extends BaseScene {
  constructor(renderer, camera) {
    super(renderer, camera);
    this.label = "Particle Field";
  }

  _init() {
    this.count = 5000;
    this.geometry = new THREE.BufferGeometry();
    this.positions = new Float32Array(this.count * 3);
    this.velocities = new Float32Array(this.count * 3);
    this.colors = new Float32Array(this.count * 3);

    for (let i = 0; i < this.count; i++) {
      this.positions[i * 3] = (Math.random() - 0.5) * 40;
      this.positions[i * 3 + 1] = (Math.random() - 0.5) * 40;
      this.positions[i * 3 + 2] = (Math.random() - 0.5) * 40;

      this.velocities[i * 3] = (Math.random() - 0.5) * 0.02;
      this.velocities[i * 3 + 1] = (Math.random() - 0.5) * 0.02;
      this.velocities[i * 3 + 2] = (Math.random() - 0.5) * 0.02;

      const hue = Math.random();
      const color = new THREE.Color().setHSL(hue, 0.8, 0.6);
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

    this.material = new THREE.PointsMaterial({
      size: 0.15,
      vertexColors: true,
      transparent: true,
      opacity: 0.8,
      sizeAttenuation: true,
    });

    this.points = new THREE.Points(this.geometry, this.material);
    this.group.add(this.points);
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    if (!audioData) return;

    const energy = audioData[0] / 255;
    const bass = audioData[0] / 255;

    for (let i = 0; i < this.count; i++) {
      let speed = 1;
      if (bass > 0.6) {
        speed = 1 + bass * 3;
      }

      this.positions[i * 3] += this.velocities[i * 3] * speed;
      this.positions[i * 3 + 1] += this.velocities[i * 3 + 1] * speed;
      this.positions[i * 3 + 2] += this.velocities[i * 3 + 2] * speed;

      for (let j = 0; j < 3; j++) {
        if (Math.abs(this.positions[i * 3 + j]) > 20) {
          this.velocities[i * 3 + j] *= -1;
        }
      }

      const hue = (i / this.count + this.time * 0.1) % 1;
      const color = new THREE.Color().setHSL(hue, 0.8, 0.5 + energy * 0.3);
      this.colors[i * 3] = color.r;
      this.colors[i * 3 + 1] = color.g;
      this.colors[i * 3 + 2] = color.b;
    }

    this.geometry.attributes.position.needsUpdate = true;
    this.geometry.attributes.color.needsUpdate = true;
  }
}
