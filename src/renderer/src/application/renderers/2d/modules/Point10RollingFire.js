import { createPoint10Module } from "../../../point10/adapter.js";
import { RollingFire } from "../../../point10/scenes/RollingFire.js";

export default createPoint10Module({
  name: "Point10 Rolling Fire",
  SceneClass: RollingFire,
  webgpu: true,
});
