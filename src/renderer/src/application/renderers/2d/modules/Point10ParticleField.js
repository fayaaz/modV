import { createPoint10Module } from "../../../point10/adapter.js";
import { ParticleField } from "../../../point10/scenes/ParticleField.js";

export default createPoint10Module({
  name: "Point10 Particle Field",
  SceneClass: ParticleField,
});
