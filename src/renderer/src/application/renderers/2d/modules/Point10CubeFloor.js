import { createPoint10Module } from "../../../point10/adapter.js";
import { CubeFloor } from "../../../point10/scenes/CubeFloor.js";

export default createPoint10Module({
  name: "Point10 Cube Floor",
  SceneClass: CubeFloor,
});
