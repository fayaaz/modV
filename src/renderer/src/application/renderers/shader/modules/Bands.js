export default {
  meta: {
    name: "Bands",
    author: "modV",
    version: 0.1,
    type: "shader",
    audioFeatures: ["bass", "mid", "treble"],
  },
  fragmentShader: `
    precision mediump float;

    uniform float u_bass;
    uniform float u_mid;
    uniform float u_treble;
    uniform float u_ringWidth;

    varying vec2 vUv;

    float ring(float radius, float width) {
      float d = length(vUv - 0.5) * 2.0;
      return 1.0 - smoothstep(0.0, width, abs(d - radius));
    }

    void main() {
      vec3 color = vec3(
        ring(u_bass, u_ringWidth),
        ring(u_mid, u_ringWidth),
        ring(u_treble, u_ringWidth)
      );

      gl_FragColor = vec4(color, 1.0);
    }`,
  props: {
    u_ringWidth: {
      type: "float",
      label: "Ring Width",
      min: 0.01,
      max: 0.5,
      step: 0.01,
      default: 0.05,
    },
  },
};
