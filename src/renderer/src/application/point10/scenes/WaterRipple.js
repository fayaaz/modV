import * as THREE from "three";
import { BaseScene } from "../BaseScene.js";
import { P9Audio } from "../p10Audio.js";
import { WaveSim } from "../shared/waterWave.js";
import { dropColorPulse, equalizedHue } from "../shared/phraseColor.js";

const WAVE_RES = 256;
const IMPACT_COUNT = 8;
const clamp01 = (value) =>
  Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0));

export function waterRippleDrive(bass = 0, pulse = 0) {
  const b = clamp01(bass);
  const p = clamp01(pulse);
  return {
    distortion: 0.55 + b * 1.1 + p * 0.55,
    speed: 1.3 + b * 0.5 + p * 0.2,
    radius: 0.9 + b * 0.1 + p * 0.03,
    brightness: 0.68 + b * 0.5 + p * 0.2,
    dropStrength: 0.012 + b * 0.022 + p * 0.025,
  };
}

const VERT = `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position, 1.0);
}
`;

const FRAG = `
precision highp float;
uniform vec2 uResolution;
uniform float uTime;
uniform float uBass;
uniform float uPulse;
uniform float uDistortion;
uniform float uSpeed;
uniform float uRadius;
uniform float uBrightness;
uniform sampler2D uWater;
uniform vec4 uImpacts[8];
uniform vec3 uColorA;
uniform vec3 uColorB;
uniform vec3 uColorC;
varying vec2 vUv;

float impactRipple(vec2 uv, vec4 impact) {
  float age = impact.z;
  vec2 delta = uv - impact.xy;
  float distanceToDrop = length(delta);
  float front = age * 0.28;
  float outerDistance = (distanceToDrop - front) / 0.014;
  float innerDistance = (distanceToDrop - front * 0.7) / 0.024;
  float ring = exp(-outerDistance * outerDistance) + exp(-innerDistance * innerDistance) * 0.28;
  float bead = exp(-distanceToDrop * distanceToDrop / 0.00008) * exp(-age * 10.0) * 0.2;
  return (ring * 0.55 + bead) * exp(-age * 0.8) * impact.w;
}

void main() {
  vec2 r = uResolution;
  vec2 fragment = gl_FragCoord.xy;
  vec2 waterUv = vUv;
  vec2 screenP = (fragment * 2.0 - r) / r.y;
  float waterHeight = texture2D(uWater, waterUv).r * (2.4 + uBass * 1.6);
  float waterInfluence = clamp(waterHeight, -0.65, 0.65);
  float impactInfluence = 0.0;
  for (int i = 0; i < 8; i++) impactInfluence += impactRipple(waterUv, uImpacts[i]);
  waterInfluence = clamp(waterInfluence + impactInfluence * 0.025, -0.8, 0.8);

  float audioInfluence = uBass * 0.4 + uPulse * 0.2;
  float circleRadius = uRadius + audioInfluence * 0.08 + abs(waterInfluence) * 0.1;
  float inCircle = 1.0 - smoothstep(circleRadius - 0.1, circleRadius + 0.1, length(screenP));
  vec2 p = screenP * 1.1;
  float angle = length(p) * 4.0 + audioInfluence * 2.0 + waterInfluence * 0.25;
  p *= mat2(cos(angle), -sin(angle), sin(angle), cos(angle));

  float l = length(p) - 0.7 + waterInfluence * 0.5 + audioInfluence * 0.12;
  float t = uTime * uSpeed + waterInfluence * 2.0 + audioInfluence * 1.5;
  float enhancedY = p.y + waterInfluence * 0.3 + audioInfluence * 0.15;
  float edge = max(max(l / 0.1, -l), 0.0001);
  float pattern1 = 0.5 + 0.5 * tanh(0.1 / edge - sin(l + enhancedY * max(1.0, -l / 0.1) + t));
  float pattern2 = 0.5 + 0.5 * tanh(0.1 / edge - sin(l + enhancedY * max(1.0, -l / 0.1) + t + 1.0));
  float pattern3 = 0.5 + 0.5 * tanh(0.1 / edge - sin(l + enhancedY * max(1.0, -l / 0.1) + t + 2.0));
  float body = 0.24 + 0.76 * smoothstep(-0.65, 0.0, l);
  float intensity = (0.78 + waterInfluence * 0.25 + audioInfluence * 0.22) * (0.9 + uBrightness * 0.12);
  vec2 centered = waterUv - 0.5;
  float polar = atan(centered.y, centered.x);
  float gradientSweep = 0.5 + 0.5 * sin(polar * 2.0 - uTime * 0.22
    + length(centered) * 4.0 + waterInfluence * 2.4);
  float gradientDrift = 0.5 + 0.5 * sin((waterUv.x + waterUv.y) * 5.0
    + uTime * 0.16 - waterInfluence * 1.5);
  vec3 gradientColor = mix(uColorA, uColorB, gradientSweep);
  gradientColor = mix(gradientColor, uColorC, gradientDrift * 0.28);

  vec3 patternColor = vec3(pattern1 * gradientColor.r, pattern2 * gradientColor.g, pattern3 * gradientColor.b);
  vec3 color = patternColor * body * intensity;
  color += gradientColor * body * (0.08 + gradientSweep * 0.12);
  float outer = smoothstep(0.02, 0.28, l);
  color = mix(color, gradientColor * 0.42, outer * 0.72);
  float rim = exp(-l * l / 0.0024);
  vec3 rimColor = mix(vec3(0.78, 0.96, 1.0), gradientColor, 0.2);
  color += rim * rimColor * (1.05 + audioInfluence * 0.3);
  color += impactInfluence * gradientColor * (0.018 + uBass * 0.025);

  vec3 background = vec3(0.02, 0.02, 0.05);
  gl_FragColor = vec4(mix(background, color, inCircle), 1.0);
}
`;

export class WaterRipple extends BaseScene {
  _init() {
    this.label = "Water Ripple";
    this.category = "Music";
    this.role = "background";
    this.p9audio = new P9Audio(this.audio);
    this._wave = this.renderer ? new WaveSim(this.renderer, WAVE_RES) : null;
    this._lastBass = 0;
    this._motionBass = 0;
    this._lastBassDrop = -10;
    this.impacts = Array.from(
      { length: IMPACT_COUNT },
      () => new THREE.Vector4(0.5, 0.5, 10, 0),
    );
    this._impactCursor = 0;
    this._dropQueue = [];
    this._phrase = { hue: 0, t: 0 };
    this._phraseHue = 0;
    this._baseColors = [
      new THREE.Color(0x287cff),
      new THREE.Color(0x55d7ff),
      new THREE.Color(0x8c62ff),
    ];
    this._phraseColors = [
      new THREE.Color(),
      new THREE.Color(),
      new THREE.Color(),
    ];
    this._pulse = 0;
    this._seeded = false;
    this._nextAmbient = 0.9;
    this._rw = 0;
    this._rh = 0;
    const initialDrive = waterRippleDrive();
    this.uniforms = {
      uResolution: { value: new THREE.Vector2(1, 1) },
      uTime: { value: 0 },
      uBass: { value: 0 },
      uPulse: { value: 0 },
      uDistortion: { value: initialDrive.distortion },
      uSpeed: { value: initialDrive.speed },
      uRadius: { value: initialDrive.radius },
      uBrightness: { value: initialDrive.brightness },
      uWater: { value: this._wave?.texture ?? this._emptyTexture() },
      uImpacts: { value: this.impacts },
      uColorA: { value: new THREE.Color() },
      uColorB: { value: new THREE.Color() },
      uColorC: { value: new THREE.Color() },
    };
    this._applyPhraseColor();
    this.mesh = new THREE.Mesh(
      new THREE.PlaneGeometry(2, 2),
      new THREE.ShaderMaterial({
        vertexShader: VERT,
        fragmentShader: FRAG,
        uniforms: this.uniforms,
        depthTest: false,
        depthWrite: false,
      }),
    );
    this.mesh.frustumCulled = false;
    this.group.add(this.mesh);
  }

  _applyPhraseColor() {
    for (let i = 0; i < this._baseColors.length; i++) {
      equalizedHue(this._baseColors[i], this._phraseHue, this._phraseColors[i]);
      const uniform = this.uniforms[`uColor${"ABC"[i]}`];
      uniform.value.copy(this._phraseColors[i]);
    }
  }

  _emptyTexture() {
    const texture = new THREE.DataTexture(
      new Uint8Array(4),
      1,
      1,
      THREE.RGBAFormat,
    );
    texture.needsUpdate = true;
    return texture;
  }

  setAudio(audio) {
    super.setAudio(audio);
    this.p9audio = new P9Audio(audio);
  }

  onActivate() {
    this._nextAmbient = this.time + 0.35;
  }

  _syncResolution() {
    const canvas = this.renderer?.domElement;
    const w = canvas?.width || 1;
    const h = canvas?.height || 1;
    if (w !== this._rw || h !== this._rh) {
      this._rw = w;
      this._rh = h;
      this.uniforms.uResolution.value.set(w, h);
    }
  }

  _spawnImpact(x, y, strength) {
    const impact = this.impacts[this._impactCursor];
    impact.set(x, y, 0, clamp01(strength));
    this._impactCursor = (this._impactCursor + 1) % this.impacts.length;
  }

  _queueDrop(x, y, strength, radius, delay) {
    this._dropQueue.push({ at: this.time + delay, x, y, strength, radius });
  }

  _processDropQueue() {
    if (!this._wave) return;
    let index = 0;
    while (index < this._dropQueue.length) {
      const drop = this._dropQueue[index];
      if (drop.at > this.time) {
        index++;
        continue;
      }
      this._wave.drop(drop.x, drop.y, drop.strength, drop.radius);
      this._spawnImpact(drop.x, drop.y, Math.min(1, drop.strength * 6));
      this._dropQueue.splice(index, 1);
    }
  }

  _addBassRipple(bass, jump, pulse, beat) {
    if (
      !this._wave ||
      this.time - this._lastBassDrop < 0.16 ||
      (jump <= 0.02 && beat <= 0.08)
    )
      return;
    const level = Math.max(this._motionBass, beat * 0.45);
    const cursor = this._impactCursor;
    const angle = cursor * 2.399963 + this.time * 0.11;
    const baseRadius = 0.08 + ((cursor * 0.17) % 0.3);
    const count = level > 0.65 ? 4 : level > 0.3 ? 3 : 2;
    const drive = waterRippleDrive(this._motionBass, pulse);
    for (let i = 0; i < count; i++) {
      const along = count === 1 ? 0 : i / (count - 1) - 0.5;
      const pathAngle = angle + along * (0.22 + level * 0.18);
      const radius = baseRadius + Math.sin(i * 1.7 + cursor) * 0.018;
      const x = 0.5 + Math.cos(pathAngle) * radius;
      const y = 0.5 + Math.sin(pathAngle) * radius;
      const falloff = 1 - Math.abs(along) * 0.55;
      const strength = (0.2 + level * 0.45 + pulse * 0.08) * falloff;
      this._queueDrop(
        x,
        y,
        drive.dropStrength * (0.7 + strength),
        0.022 + level * 0.018 + i * 0.002,
        i * 0.042,
      );
    }
    this._lastBassDrop = this.time;
  }

  _addAmbientRipple() {
    if (!this._wave) return;
    const angle = this.time * 0.41;
    this._wave.drop(
      0.5 + Math.cos(angle) * 0.2,
      0.5 + Math.sin(angle * 1.31) * 0.2,
      0.008,
      0.065,
    );
  }

  update(dt) {
    super.update(dt);
    this.p9audio?.update(dt);
    const bass = clamp01(
      this.p9audio?.hasAudio ? this.p9audio.band(-1, 1, 0) : 0,
    );
    const beat = this.p9audio?.hasAudio ? this.p9audio.beat() : 0;
    this._motionBass += (bass - this._motionBass) * Math.min(1, dt * 7);
    const jump = Math.max(0, bass - this._lastBass);
    const attack = clamp01(jump * 8);
    this._pulse = Math.max(attack * 0.65, this._pulse * Math.exp(-dt * 3.5));
    this._phrase = dropColorPulse(this._phrase, bass, dt);
    let phraseDelta = this._phrase.hue - this._phraseHue;
    phraseDelta = Math.atan2(Math.sin(phraseDelta), Math.cos(phraseDelta));
    this._phraseHue += phraseDelta * Math.min(1, dt * 0.55);
    this._applyPhraseColor();
    const drive = waterRippleDrive(this._motionBass, this._pulse);
    this.uniforms.uTime.value = this.time;
    this.uniforms.uBass.value = this._motionBass;
    this.uniforms.uPulse.value = this._pulse;
    this.uniforms.uDistortion.value = drive.distortion;
    this.uniforms.uSpeed.value = drive.speed;
    this.uniforms.uRadius.value = drive.radius;
    this.uniforms.uBrightness.value = drive.brightness;
    for (const impact of this.impacts) {
      impact.z += dt;
      if (impact.z > 5) impact.w = 0;
    }

    if (this._wave) {
      if (!this._seeded) {
        this._wave.drop(0.5, 0.5, 0.04, 0.12);
        this._spawnImpact(0.5, 0.5, 0.24);
        this._seeded = true;
      }
      this._addBassRipple(bass, jump, this._pulse, beat);
      this._processDropQueue();
      if (this.time >= this._nextAmbient) {
        this._addAmbientRipple();
        this._nextAmbient = this.time + 1.8;
      }
      this._wave.step();
      this.uniforms.uWater.value = this._wave.texture;
    }

    this._lastBass = bass;
    this._syncResolution();
  }

  resize() {
    this._rw = 0;
    this._rh = 0;
    this._syncResolution();
  }

  render() {
    if (!this.renderer) return;
    this._syncResolution();
    super.render();
  }
}
