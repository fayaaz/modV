// Module inputs for the imported Point10 scenes. These live outside the
// vendored scene files (like the responsiveness levers in props.js), so a
// scene stays portable and the adapter owns the mapping from modV props to the
// scene's uniforms/fields.
//
// Each config pairs a prop schema with the function that writes the resolved
// values onto a live scene. The scene files read the named fields in their
// update loop, so changing a slider takes effect on the next frame.
const float = (label, defaultValue, min, max, step = 0.01) => ({
  label,
  type: "float",
  min,
  max,
  step,
  default: defaultValue,
});

const SMOKE_FIRE = {
  smokeDensity: float("Smoke Density", 28, 0, 100, 0.1),
  fireIntensity: float("Fire Intensity", 40, 0, 100, 0.1),
  smokeLifespan: float("Smoke Lifespan", 3.5, 0.1, 20, 0.1),
  fireLifespan: float("Fire Lifespan", 1.3, 0.1, 5, 0.1),
  // The sim loop divides the frame accumulator by this, so 0 would never
  // terminate; 0.1 is the slowest safe rate.
  simulationSpeed: float("Simulation Speed", 1.2, 0.1, 4),
};

export const SMOKE_ROOM_PROPS = {
  ...SMOKE_FIRE,
  // Defaults to the pre-boxScale size: the cameras sit outside the glass room
  // and the room fills the frame. Scaling it up puts the cameras inside the
  // room, which crops the scene ("zoomed in"), so that stays opt-in.
  boxScale: float("Box Scale", 1, 0.5, 5, 0.05),
  // X/Y multiplier on top of Box Scale: widens/tallies the glass room and the
  // sim grid only. The orb (sphere, point light) and the sim's sphere radius
  // stay on Box Scale alone.
  boxXYScale: float("Box Width & Height", 1, 0.5, 5, 0.05),
};

export const DVD_ROOM_PROPS = {
  ...SMOKE_ROOM_PROPS,
  // The DVD Room dims the burn so the logo's bounce colour stays visible.
  fireIntensity: float("Fire Intensity", 22, 0, 100, 0.1),
};

export const ROLLING_FIRE_PROPS = {
  ...SMOKE_FIRE,
  // Rolling Fire's bigger grid uses less emission per cell.
  smokeDensity: float("Smoke Density", 16, 0, 100, 0.1),
  fireIntensity: float("Fire Intensity", 26, 0, 100, 0.1),
  smokeLifespan: float("Smoke Lifespan", 1.5, 0.1, 20, 0.1),
  fireLifespan: float("Fire Lifespan", 0.9, 0.1, 5, 0.1),
  // Screen-space pan of the whole scene: added to both the camera position and
  // its lookAt target each frame, so the frame shifts without rotating.
  // World units; the sim world is ~32 m across.
  sceneOffsetX: float("Offset X", 0, -10, 10, 0.1),
  sceneOffsetY: float("Offset Y", 0, -10, 10, 0.1),
};

export const AROUND_PLUS_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  ringBrightness: float("Ring Brightness", 1, 0, 3),
  waterChurn: float("Water Churn", 1, 0, 3),
};

export const ATTRACTOR_PARTICLES_PROPS = {
  cameraDistance: float("Camera Distance", 9.9, 4, 30, 0.1),
  particleSize: float("Particle Size", 1, 0.1, 4),
  brightness: float("Particle Brightness", 1, 0, 3),
  maxSpeed: float("Max Speed", 1, 0.25, 3),
};

export const BOXY_PROPS = {
  scrollSpeed: float("Scroll Speed", 1, 0, 4),
  glow: float("Glow", 1, 0, 3),
  roughness: float("Roughness Floor", 0.3, 0, 1),
  bloom: float("Bloom", 1, 0, 3),
};

export const CHROMATIC_LINE_PROPS = {
  distortion: float("Chroma", 0.05, 0, 0.5, 0.001),
  amplitude: float("Wave Amplitude", 1, 0, 3),
  glow: float("Glow", 1, 0, 3),
};

export const GEOMETRIC_TUNNEL_PROPS = {
  scrollSpeed: float("Scroll Speed", 1, 0, 4),
  brightness: float("Brightness", 1, 0, 3),
  bloom: float("Bloom", 1, 0, 3),
};

export const IT_IS_COMING_PROPS = {
  speed: float("Cloud Speed", 1, 0, 4),
  glow: float("Glow", 1, 0, 3),
  bloom: float("Bloom", 1, 0, 3),
};

export const LIGHT_TUNNEL_PROPS = {
  scrollSpeed: float("Scroll Speed", 1, 0, 4),
  glow: float("Glow", 1, 0, 3),
  bloom: float("Bloom", 1, 0, 3),
};

export const MYCELIUM_PROPS = {
  growthSpeed: float("Growth Speed", 1, 0, 4),
  glow: float("Glow", 1, 0, 3),
  fogDensity: float("Fog Density", 1, 0, 2),
};

export const PARTICLE_FIELD_PROPS = {
  particleSize: float("Particle Size", 0.15, 0.02, 1),
  speed: float("Speed", 1, 0, 4),
  opacity: float("Opacity", 0.8, 0, 1),
};

export const SPIRAL_PARTICLES_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  particleSize: float("Particle Size", 1, 0.2, 4),
  turbulence: float("Turbulence", 0.2, 0, 1),
};

export const SQUARE_TUNNEL_PROPS = {
  scrollSpeed: float("Scroll Speed", 1, 0, 4),
  brightness: float("Brightness", 1, 0, 3),
};

export const WATER_RIPPLE_PROPS = {
  speed: float("Ripple Speed", 1, 0, 4),
  distortion: float("Distortion", 1, 0, 4),
  radius: float("Radius", 1, 0, 2),
  brightness: float("Brightness", 1, 0, 3),
};

export const WAVEFORM_PROPS = {
  height: float("Height", 1, 0, 4),
  speed: float("Wave Speed", 1, 0, 4),
  opacity: float("Opacity", 1, 0, 1),
};

export const WORMHOLE_PROPS = {
  warpSpeed: float("Warp Speed", 1, 0, 4),
  bloom: float("Bloom", 1, 0, 3),
};

export const MIRROR_CUBE_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  brightness: float("Brightness", 1, 0, 3),
};

export const SOUND_CUBE_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  pulse: float("Pulse", 1, 0, 4),
  trail: float("Trail", 1, 0, 3),
  brightness: float("Brightness", 1, 0, 3),
};

export const HOLLOW_CUBE_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  explode: float("Explosion", 1, 0, 3),
  color: float("Color", 1, 0, 3),
  bloom: float("Bloom", 1, 0, 3),
};

export const COLOR_CUBE_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  turbulence: float("Turbulence", 1, 0, 3),
  bloom: float("Bloom", 1, 0, 3),
  brightness: float("Brightness", 1, 0, 3),
};

export const LATENT_BACKROOMS_PROPS = {
  speed: float("Camera Speed", 1, 0, 4),
  bloom: float("Bloom", 1, 0, 3),
  trail: float("Afterimage Trail", 1, 0, 3),
  brightness: float("Brightness", 1, 0, 3),
};

export const ULTRA_HDR_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  pulse: float("Kick Pulse", 1, 0, 3),
  brightness: float("Brightness", 1, 0, 3),
};

export const GLASS_LENS_PROPS = {
  spinSpeed: float("Spin Speed", 1, 0, 4),
  pulse: float("Kick Pulse", 1, 0, 3),
  brightness: float("Brightness", 1, 0, 3),
};

const UNIFORM_BINDINGS = {
  smokeDensity: "_uEmitDensity",
  fireIntensity: "_uFireIntensity",
};

const FIELD_BINDINGS = {
  smokeLifespan: "_smokeLifespan",
  fireLifespan: "_fireLifespan",
  simulationSpeed: "_simSpeed",
  boxScale: "_boxScale",
  boxXYScale: "_boxXYScale",
  sceneOffsetX: "_sceneOffsetX",
  sceneOffsetY: "_sceneOffsetY",
};

const numeric = (value, fallback, min, max) => {
  const n =
    typeof value === "number" && Number.isFinite(value) ? value : fallback;
  return n < min ? min : n > max ? max : n;
};

/**
 * Resolves raw modV props against a scene prop schema, filling missing values
 * with their defaults and clamping everything to its declared range.
 */
export function readSceneProps(props = {}, definitions = SMOKE_ROOM_PROPS) {
  const values = {};

  for (const [key, definition] of Object.entries(definitions)) {
    if (definition.type === "bool") {
      values[key] = props[key] !== false;
    } else {
      values[key] = numeric(
        props[key],
        definition.default,
        definition.min,
        definition.max,
      );
    }
  }

  return values;
}

function defineScene(definitions, bind) {
  return {
    props: definitions,
    apply(scene, props) {
      const values = readSceneProps(props, definitions);
      bind(scene, values);
      return values;
    },
  };
}

const bindSmoke = (scene, values) => {
  for (const [key, field] of Object.entries(UNIFORM_BINDINGS)) {
    if (scene[field]) {
      scene[field].value = values[key];
    }
  }

  for (const [key, field] of Object.entries(FIELD_BINDINGS)) {
    if (values[key] !== undefined) scene[field] = values[key];
  }
};

/**
 * Applies the resolved smoke/fire controls to a Smoke Room (or subclass) or
 * Rolling Fire instance. Safe to call before the WebGPU volume exists: the
 * uniforms are simply skipped, while the scalar fields are ready for the
 * animation loop.
 */
export function applySmokeRoomProps(
  scene,
  props,
  definitions = SMOKE_ROOM_PROPS,
) {
  const values = readSceneProps(props, definitions);
  bindSmoke(scene, values);
  return values;
}

export const SCENE_PROP_CONFIGS = {
  "Point10 Mirror Cube": defineScene(MIRROR_CUBE_PROPS, (scene, v) => {
    scene.spinScale = v.spinSpeed;
    scene.brightnessScale = v.brightness;
  }),
  "Point10 Sound Cube": defineScene(SOUND_CUBE_PROPS, (scene, v) => {
    scene.spinScale = v.spinSpeed;
    scene.pulseScale = v.pulse;
    scene.trailScale = v.trail;
    scene.brightnessScale = v.brightness;
  }),
  "Point10 Hollow Cube": defineScene(HOLLOW_CUBE_PROPS, (scene, v) => {
    scene.spinScale = v.spinSpeed;
    scene.explodeScale = v.explode;
    scene.colorScale = v.color;
    scene.bloomScale = v.bloom;
  }),
  "Point10 Color Cube": defineScene(COLOR_CUBE_PROPS, (scene, v) => {
    scene.spinScale = v.spinSpeed;
    scene.turbulenceScale = v.turbulence;
    scene.bloomScale = v.bloom;
    scene.brightnessScale = v.brightness;
  }),
  "Point10 Latent Backrooms": defineScene(
    LATENT_BACKROOMS_PROPS,
    (scene, v) => {
      scene.speedScale = v.speed;
      scene.bloomScale = v.bloom;
      scene.trailScale = v.trail;
      scene.brightnessScale = v.brightness;
    },
  ),
  "Point10 Ultra HDR": defineScene(ULTRA_HDR_PROPS, (scene, v) => {
    scene.spinScale = v.spinSpeed;
    scene.pulseScale = v.pulse;
    scene.brightnessScale = v.brightness;
  }),
  "Point10 Glass Lens": defineScene(GLASS_LENS_PROPS, (scene, v) => {
    scene.spinScale = v.spinSpeed;
    scene.pulseScale = v.pulse;
    scene.brightnessScale = v.brightness;
  }),
  "Point10 Around Plus": defineScene(AROUND_PLUS_PROPS, (scene, v) => {
    scene.spinSpeed = v.spinSpeed;
    scene.ringBrightness = v.ringBrightness;
    scene.waterChurn = v.waterChurn;
  }),

  "Point10 Attractor Particles": defineScene(
    ATTRACTOR_PARTICLES_PROPS,
    (scene, v) => {
      scene._camRadius = v.cameraDistance;
      scene._particleSize = v.particleSize;
      scene._bloomScale = v.brightness;
      scene._speedScale = v.maxSpeed;
      if (scene._uScale) {
        scene._uScale.value = 0.008 * v.particleSize;
      }
    },
  ),

  "Point10 Ball Of Fire": defineScene(ROLLING_FIRE_PROPS, bindSmoke),
  "Point10 Boxy": defineScene(BOXY_PROPS, (scene, v) => {
    scene.scrollScale = v.scrollSpeed;
    scene.glowScale = v.glow;
    scene.roughnessFloor = v.roughness;
    scene.bloomScale = v.bloom;
  }),
  "Point10 Chromatic Line": defineScene(CHROMATIC_LINE_PROPS, (scene, v) => {
    scene.distortion = v.distortion;
    scene.ampScale = v.amplitude;
    scene.glowScale = v.glow;
  }),
  "Point10 Chromatic Wave": defineScene(CHROMATIC_LINE_PROPS, (scene, v) => {
    scene.distortion = v.distortion;
    scene.ampScale = v.amplitude;
    scene.glowScale = v.glow;
  }),
  "Point10 DVD Room": defineScene(DVD_ROOM_PROPS, bindSmoke),
  "Point10 Geometric Tunnel": defineScene(
    GEOMETRIC_TUNNEL_PROPS,
    (scene, v) => {
      scene.speedScale = v.scrollSpeed;
      scene.brightnessScale = v.brightness;
      scene.bloomScale = v.bloom;
    },
  ),
  "Point10 It Is Coming For You": defineScene(
    IT_IS_COMING_PROPS,
    (scene, v) => {
      scene.speedScale = v.speed;
      scene.glowScale = v.glow;
      scene.bloomScale = v.bloom;
    },
  ),
  "Point10 Light Tunnel": defineScene(LIGHT_TUNNEL_PROPS, (scene, v) => {
    scene.speedScale = v.scrollSpeed;
    scene.glowScale = v.glow;
    scene.bloomScale = v.bloom;
  }),
  "Point10 Mycelium": defineScene(MYCELIUM_PROPS, (scene, v) => {
    scene.speedScale = v.growthSpeed;
    scene.glowScale = v.glow;
    scene.fogScale = v.fogDensity;
  }),
  "Point10 Particle Field": defineScene(PARTICLE_FIELD_PROPS, (scene, v) => {
    scene.particleSize = v.particleSize;
    scene.speedScale = v.speed;
    scene.opacity = v.opacity;
  }),
  "Point10 Rolling Fire": defineScene(ROLLING_FIRE_PROPS, bindSmoke),
  "Point10 Smoke Room": defineScene(SMOKE_ROOM_PROPS, bindSmoke),
  "Point10 Spiral Particles": defineScene(
    SPIRAL_PARTICLES_PROPS,
    (scene, v) => {
      scene.spinScale = v.spinSpeed;
      scene.sizeScale = v.particleSize;
      scene.turbulence = v.turbulence;
    },
  ),
  "Point10 Square Tunnel": defineScene(SQUARE_TUNNEL_PROPS, (scene, v) => {
    scene.speedScale = v.scrollSpeed;
    scene.brightnessScale = v.brightness;
  }),
  "Point10 Water Ripple": defineScene(WATER_RIPPLE_PROPS, (scene, v) => {
    scene.speedScale = v.speed;
    scene.distortionScale = v.distortion;
    scene.radiusScale = v.radius;
    scene.brightnessScale = v.brightness;
  }),
  "Point10 Waveform": defineScene(WAVEFORM_PROPS, (scene, v) => {
    scene.heightScale = v.height;
    scene.speedScale = v.speed;
    scene.opacity = v.opacity;
  }),
  "Point10 Wormhole": defineScene(WORMHOLE_PROPS, (scene, v) => {
    scene.speedScale = v.warpSpeed;
    scene.bloomScale = v.bloom;
  }),
};

export function scenePropsFor(name) {
  return SCENE_PROP_CONFIGS[name] ?? { props: {}, apply: () => ({}) };
}

export function applyDvdRoomProps(scene, props) {
  return applySmokeRoomProps(scene, props, DVD_ROOM_PROPS);
}
