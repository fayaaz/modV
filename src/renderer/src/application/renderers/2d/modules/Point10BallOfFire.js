import { createPoint10Module } from "../../../point10/adapter.js";
import { BallOfFire } from "../../../point10/scenes/RollingFire.js";

export default createPoint10Module({
  name: "Point10 Ball Of Fire",
  SceneClass: BallOfFire,
  webgpu: true,
});
