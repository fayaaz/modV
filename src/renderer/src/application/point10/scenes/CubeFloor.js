import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";

export class CubeFloor extends BaseScene {
  _init() {
    this.gridSize = 16;
    this.cubes = [];
    const geometry = new THREE.BoxGeometry(0.9, 0.9, 0.9);

    for (let x = 0; x < this.gridSize; x++) {
      for (let z = 0; z < this.gridSize; z++) {
        const hue = (x + z) / (this.gridSize * 2);
        const color = new THREE.Color().setHSL(hue, 0.7, 0.4);

        const material = new THREE.MeshStandardMaterial({
          color,
          emissive: color,
          emissiveIntensity: 0.1,
          roughness: 0.5,
          metalness: 0.5,
        });

        const cube = new THREE.Mesh(geometry, material);
        cube.position.set(
          (x - this.gridSize / 2) * 1.1,
          0,
          (z - this.gridSize / 2) * 1.1,
        );
        this.group.add(cube);
        this.cubes.push({ mesh: cube, baseY: 0, x, z });
      }
    }

    this.group.add(new THREE.DirectionalLight(0xffffff, 1));
    this.group.add(new THREE.AmbientLight(0x404040));
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    if (!audioData) return;

    for (let i = 0; i < this.cubes.length; i++) {
      const cube = this.cubes[i];
      const freqIndex = Math.floor(
        (cube.x / this.gridSize) * 32 + (cube.z / this.gridSize) * 32,
      );
      const value = audioData[Math.min(freqIndex, audioData.length - 1)] / 255;

      const height = value * 4;
      cube.mesh.position.y = cube.baseY + height / 2;
      cube.mesh.scale.y = Math.max(height, 0.05);

      const hue = (cube.x + cube.z + this.time) / (this.gridSize * 2);
      const color = new THREE.Color().setHSL(hue % 1, 0.7, 0.3 + value * 0.5);
      cube.mesh.material.color.copy(color);
      cube.mesh.material.emissive.copy(color).multiplyScalar(value * 0.6);
    }
  }
}
