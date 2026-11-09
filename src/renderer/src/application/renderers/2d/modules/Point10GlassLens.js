import { createPoint10Module } from "../../../point10/adapter.js";
import { GlassLens } from "../../../point10/scenes/GlassLens.js";

export default createPoint10Module({
  name: "Point10 Glass Lens",
  SceneClass: GlassLens,
});
