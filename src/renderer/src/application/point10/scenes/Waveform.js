import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";

export class Waveform extends BaseScene {
  constructor(renderer, camera) {
    super(renderer, camera);
    this.label = "Waveform";
    this.category = "Music";
  }

  _init() {
    this.points = 256;
    const geometry = new THREE.BufferGeometry();
    const positions = new Float32Array(this.points * 3);
    geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));

    const material = new THREE.LineBasicMaterial({
      color: 0x00ffff,
      transparent: true,
      opacity: 0.8,
    });

    this.line = new THREE.Line(geometry, material);
    this.group.add(this.line);

    const material2 = new THREE.LineBasicMaterial({
      color: 0xff00ff,
      transparent: true,
      opacity: 0.5,
    });

    this.line2 = new THREE.Line(geometry.clone(), material2);
    this.group.add(this.line2);

    this.group.rotation.x = -0.3;
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    if (!audioData) return;

    const positions = this.line.geometry.attributes.position.array;
    const wave = audioData;

    for (let i = 0; i < this.points; i++) {
      const x = (i / this.points - 0.5) * 30;
      const value = wave[i % wave.length] / 255;
      const y = value * 6 * Math.sin(i * 0.1 + this.time);

      positions[i * 3] = x;
      positions[i * 3 + 1] = y;
      positions[i * 3 + 2] = 0;
    }

    this.line.geometry.attributes.position.needsUpdate = true;

    const positions2 = this.line2.geometry.attributes.position.array;
    for (let i = 0; i < this.points; i++) {
      const x = (i / this.points - 0.5) * 30;
      const value = wave[i % wave.length] / 255;
      const y = value * 4 * Math.cos(i * 0.15 + this.time * 0.7);

      positions2[i * 3] = x;
      positions2[i * 3 + 1] = y;
      positions2[i * 3 + 2] = 2;
    }

    this.line2.geometry.attributes.position.needsUpdate = true;

    const hue = (this.time * 0.1) % 1;
    this.line.material.color.setHSL(hue, 1, 0.6);
    this.line2.material.color.setHSL((hue + 0.5) % 1, 1, 0.4);
  }
}
