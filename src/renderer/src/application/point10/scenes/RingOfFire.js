import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";

export class RingOfFire extends BaseScene {
  _init() {
    this.ringCount = 8;
    this.rings = [];

    for (let i = 0; i < this.ringCount; i++) {
      const geometry = new THREE.TorusGeometry(
        3 + i * 0.8,
        0.05 + Math.random() * 0.1,
        8,
        64,
      );
      const hue = i / this.ringCount;
      const color = new THREE.Color().setHSL(hue, 1, 0.5);

      const material = new THREE.MeshStandardMaterial({
        color,
        emissive: color,
        emissiveIntensity: 0.3,
        roughness: 0.4,
        metalness: 0.8,
      });

      const ring = new THREE.Mesh(geometry, material);
      ring.rotation.x = Math.PI / 2;
      this.group.add(ring);
      this.rings.push({
        mesh: ring,
        baseRadius: 3 + i * 0.8,
        speed: (i + 1) * 0.2,
      });
    }

    this.group.add(new THREE.DirectionalLight(0xffffff, 1));
    this.group.add(new THREE.AmbientLight(0x404040));
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    if (!audioData) return;

    for (let i = 0; i < this.rings.length; i++) {
      const ring = this.rings[i];
      const value = audioData[i * 4] / 255;

      ring.mesh.rotation.z += dt * ring.speed * (1 + value * 3);
      ring.mesh.rotation.x = Math.PI / 2 + Math.sin(this.time * 0.5 + i) * 0.3;

      const scale = 1 + value * 0.5;
      ring.mesh.scale.set(scale, scale, scale);

      const hue = (i / this.ringCount + this.time * 0.05) % 1;
      const color = new THREE.Color().setHSL(hue, 1, 0.4 + value * 0.4);
      ring.mesh.material.color.copy(color);
      ring.mesh.material.emissive.copy(color).multiplyScalar(value);
    }
  }
}
