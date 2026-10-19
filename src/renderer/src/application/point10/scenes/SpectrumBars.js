import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";

export class SpectrumBars extends BaseScene {
  constructor(renderer, camera) {
    super(renderer, camera);
    this.label = "Spectrum Bars";
  }

  _init() {
    this.barCount = 64;
    this.barGeometry = new THREE.BoxGeometry(0.6, 1, 0.6);
    this.barMaterial = new THREE.MeshStandardMaterial({
      color: 0xffffff,
      emissive: 0x000000,
      roughness: 0.3,
      metalness: 0.7,
    });

    this.bars = [];

    for (let i = 0; i < this.barCount; i++) {
      const mesh = new THREE.Mesh(this.barGeometry, this.barMaterial.clone());
      const angle = (i / this.barCount) * Math.PI * 2;
      const radius = 8;
      mesh.position.set(Math.cos(angle) * radius, 0, Math.sin(angle) * radius);
      mesh.lookAt(0, 0, 0);
      this.group.add(mesh);
      this.bars.push(mesh);
    }

    this.group.add(new THREE.DirectionalLight(0xffffff, 1));
    this.group.add(new THREE.AmbientLight(0x404040));
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    if (!audioData) return;

    for (let i = 0; i < this.bars.length; i++) {
      const value = audioData[i] / 255;
      const height = Math.max(value * 8, 0.1);
      const mesh = this.bars[i];
      mesh.position.y = height / 2;
      mesh.scale.y = height;

      const hue = i / this.barCount;
      const color = new THREE.Color().setHSL(hue, 1, 0.5);
      mesh.material.emissive.copy(color).multiplyScalar(value * 0.8);
    }
  }
}
