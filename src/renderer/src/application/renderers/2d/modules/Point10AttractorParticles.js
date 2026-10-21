import { createPoint10Module } from "../../../point10/adapter.js";
import { AttractorParticles } from "../../../point10/scenes/AttractorParticles.js";

export default createPoint10Module({
  name: "Point10 Attractor Particles",
  SceneClass: AttractorParticles,
  webgpu: true,
});
