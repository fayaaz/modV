export const FADE_TRANSITION = "Fade";
export const NONE_TRANSITION = "None";

const ISF_HEADER = `/*{
  "ISFVSN": "2",
  "INPUTS": [
    { "NAME": "startImage", "TYPE": "image" },
    { "NAME": "endImage", "TYPE": "image" },
    { "NAME": "progress", "TYPE": "float", "MIN": 0, "MAX": 1, "DEFAULT": 0 }
  ]
}*/`;

const makeTransition = (name, body) => ({
  name,
  fragmentShader: `${ISF_HEADER}\n${body}`,
});

// Native ports of the three.js transitions already used by musicvis.
export const NATIVE_TRANSITIONS = [
  makeTransition(
    "Three Crossfade",
    `void main() {
      vec2 uv = isf_FragNormCoord;
      float p = smoothstep(0.0, 1.0, clamp(progress, 0.0, 1.0));
      gl_FragColor = mix(
        IMG_NORM_PIXEL(startImage, uv),
        IMG_NORM_PIXEL(endImage, uv),
        p
      );
    }`,
  ),
  makeTransition(
    "Three Dissolve",
    `float hash(vec2 p) {
      return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
    }

    void main() {
      vec2 uv = isf_FragNormCoord;
      vec2 cell = floor(uv * vec2(32.0, 18.0));
      float noise = hash(cell);
      float edge = clamp(progress, 0.0, 1.0) * 1.4 - 0.2;
      float incoming = smoothstep(edge, edge + 0.14, noise);
      gl_FragColor = mix(
        IMG_NORM_PIXEL(startImage, uv),
        IMG_NORM_PIXEL(endImage, uv),
        1.0 - incoming
      );
    }`,
  ),
  makeTransition(
    "Three Pixelate",
    `vec2 cells(vec2 uv, vec2 size) {
      vec2 count = max(vec2(1.0), floor(size / 9.0));
      return (floor(uv * count) + 0.5) / count;
    }

    void main() {
      vec2 uv = isf_FragNormCoord;
      vec2 fromUv = cells(uv, IMG_SIZE(startImage));
      vec2 toUv = cells(uv, IMG_SIZE(endImage));
      float p = smoothstep(0.0, 1.0, clamp(progress, 0.0, 1.0));
      gl_FragColor = mix(
        IMG_NORM_PIXEL(startImage, fromUv),
        IMG_NORM_PIXEL(endImage, toUv),
        p
      );
    }`,
  ),
  makeTransition(
    "Three Glitch",
    `float hash(vec2 p) {
      return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
    }

    void main() {
      float p = clamp(progress, 0.0, 1.0);
      vec2 uv = isf_FragNormCoord;
      float band = floor(uv.y * 24.0);
      float seed = floor(progress * 12.0);
      float amount = step(0.72, hash(vec2(band, seed))) *
        (1.0 - abs(p * 2.0 - 1.0)) * 0.08;
      uv.x += (hash(vec2(band, seed + 1.0)) - 0.5) * amount;

      vec4 from = IMG_NORM_PIXEL(startImage, uv);
      vec4 incoming = IMG_NORM_PIXEL(endImage, uv);
      incoming.r = IMG_NORM_PIXEL(endImage, uv + vec2(amount, 0.0)).r;
      incoming.b = IMG_NORM_PIXEL(endImage, uv - vec2(amount, 0.0)).b;

      gl_FragColor = mix(from, incoming, smoothstep(0.0, 1.0, p));
    }`,
  ),
  makeTransition(
    "Three Wipe",
    `void main() {
      vec2 uv = isf_FragNormCoord;
      float p = smoothstep(0.0, 1.0, clamp(progress, 0.0, 1.0));
      float incoming = step(uv.x, p);
      gl_FragColor = mix(
        IMG_NORM_PIXEL(startImage, uv),
        IMG_NORM_PIXEL(endImage, uv),
        incoming
      );
    }`,
  ),
  makeTransition(
    "Three Circle",
    `void main() {
      vec2 uv = isf_FragNormCoord;
      float aspect = RENDERSIZE.x / max(RENDERSIZE.y, 1.0);
      vec2 p = (uv - 0.5) * vec2(aspect, 1.0);
      float radius = smoothstep(0.0, 1.0, clamp(progress, 0.0, 1.0)) * 1.2;
      float incoming = 1.0 - smoothstep(radius - 0.08, radius + 0.08, length(p));
      gl_FragColor = mix(
        IMG_NORM_PIXEL(startImage, uv),
        IMG_NORM_PIXEL(endImage, uv),
        incoming
      );
    }`,
  ),
];

export const NATIVE_TRANSITION_NAMES = NATIVE_TRANSITIONS.map(
  (transition) => transition.name,
);
