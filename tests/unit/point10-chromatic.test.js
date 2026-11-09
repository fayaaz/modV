import { describe, expect, it } from "vitest";
import { ChromaticLine } from "../../src/renderer/src/application/point10/scenes/ChromaticLine.js";
import { ChromaticWave } from "../../src/renderer/src/application/point10/scenes/ChromaticWave.js";

// The line/wave are full-screen fragment shaders. Their output used to write
// alpha 1 everywhere, which painted a dark rectangle over the layers below.
// The line intensity itself now provides the alpha, so empty space composites
// transparently.
describe("Chromatic Line and Wave transparency", () => {
  it.each([
    ["Chromatic Line", ChromaticLine],
    ["Chromatic Wave", ChromaticWave],
  ])("%s derives its alpha from the line intensity", (_name, SceneClass) => {
    const scene = new SceneClass(null, null);
    const shader = scene.mesh.material.fragmentShader;

    expect(shader).toContain("float intensity = max(r, max(g, b))");
    expect(shader).toContain("float alpha = smoothstep(0.5, 1.0, intensity)");
    expect(shader).toContain("gl_FragColor = vec4(r, g, b, alpha)");
    expect(shader).not.toContain("gl_FragColor = vec4(r, g, b, 1.0)");
  });
});
