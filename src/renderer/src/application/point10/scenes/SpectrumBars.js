import { BaseScene } from "../BaseScene.js";
import * as THREE from "three";

// Map the full byte FFT onto log-spaced bar levels (0..255). Each bar covers
// an equal slice of octaves instead of an equal number of bins, so the bars
// span the whole spectrum rather than only the lowest ~64 bins (~0-1.4 kHz).
// A slice takes its peak rather than its mean: the upper bars cover many more
// bins than the lower ones, and averaging would hide narrow high-frequency
// tones. `minBin` defaults to 1 to skip the inflated DC bin.
export function spectrumBarLevels(audioData, barCount, { minBin = 1 } = {}) {
  const count = Number.isFinite(barCount)
    ? Math.max(0, Math.floor(barCount))
    : 0;
  const levels = new Uint8Array(count);

  if (!audioData || !audioData.length || count === 0) return levels;

  const end = audioData.length;
  if (end < 2) return levels;

  const start = Math.max(1, Math.min(Math.floor(minBin) || 1, end - 1));
  const ratio = end / start;

  for (let i = 0; i < count; i++) {
    const lo = Math.floor(start * Math.pow(ratio, i / count));
    const hi = Math.floor(start * Math.pow(ratio, (i + 1) / count));
    const from = Math.min(end - 1, Math.max(start, lo));
    const to = Math.min(end, Math.max(from + 1, hi));
    let peak = 0;

    for (let bin = from; bin < to; bin++) {
      const value = audioData[bin];
      if (Number.isFinite(value) && value > peak) peak = value;
    }

    levels[i] = peak > 255 ? 255 : peak < 0 ? 0 : peak;
  }

  return levels;
}

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

    const levels = spectrumBarLevels(audioData, this.bars.length);

    for (let i = 0; i < this.bars.length; i++) {
      const value = levels[i] / 255;
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
