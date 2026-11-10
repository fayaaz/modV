import { createPoint10Module } from "../../../point10/adapter.js";
import { Earth } from "../../../point10/scenes/Earth.js";

export default createPoint10Module({
  name: "Point10 Earth",
  SceneClass: Earth,
});
