import { createPoint10Module } from "../../../point10/adapter.js";
import { MirrorCube } from "../../../point10/scenes/MirrorCube.js";

export default createPoint10Module({
  name: "Point10 Mirror Cube",
  SceneClass: MirrorCube,
});
