import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";

export class DiscoBall extends BaseScene {
  _init() {
    const ballGeometry = new THREE.SphereGeometry(3, 32, 32);
    const ballMaterial = new THREE.MeshStandardMaterial({
      color: 0xffffff,
      emissive: 0x444444,
      roughness: 0.1,
      metalness: 1,
    });
    this.ball = new THREE.Mesh(ballGeometry, ballMaterial);
    this.ball.position.y = 5;
    this.group.add(this.ball);

    const mirrorCount = 200;
    const mirrorGeometry = new THREE.PlaneGeometry(0.3, 0.3);
    const mirrorMaterial = new THREE.MeshBasicMaterial({ color: 0xffffff });

    for (let i = 0; i < mirrorCount; i++) {
      const phi = Math.acos(-1 + (2 * i) / mirrorCount);
      const theta = Math.sqrt(mirrorCount * Math.PI) * phi;

      const mirror = new THREE.Mesh(mirrorGeometry, mirrorMaterial.clone());
      mirror.position.set(
        3.05 * Math.cos(theta) * Math.sin(phi),
        5 + 3.05 * Math.cos(phi),
        3.05 * Math.sin(theta) * Math.sin(phi),
      );
      mirror.lookAt(0, 5, 0);
      this.group.add(mirror);
    }

    this.spotlights = [];
    for (let i = 0; i < 6; i++) {
      const light = new THREE.PointLight(
        new THREE.Color().setHSL(i / 6, 1, 0.5),
        2,
        20,
      );
      this.spotlights.push(light);
      this.group.add(light);
    }

    this.group.add(new THREE.AmbientLight(0x202020));
  }

  update(dt, audioData) {
    super.update(dt, audioData);
    if (!audioData) return;

    this.ball.rotation.y += dt * (0.5 + audioData[0] / 255);

    const beat = audioData[0] > 200 ? 1 : 0;
    const energy = audioData[0] / 255;

    for (let i = 0; i < this.spotlights.length; i++) {
      const angle = this.time * (0.5 + i * 0.1) + (i * Math.PI) / 3;
      const radius = 8;
      this.spotlights[i].position.set(
        Math.cos(angle) * radius,
        2 + energy * 3,
        Math.sin(angle) * radius,
      );

      const hue = (i / 6 + this.time * 0.1) % 1;
      this.spotlights[i].color.setHSL(hue, 1, 0.5);
      this.spotlights[i].intensity = 2 + energy * 5 + beat * 3;
    }
  }
}
