import { createPoint10Module } from "../../../point10/adapter.js";
import { SmokeRoom } from "../../../point10/scenes/SmokeRoom.js";

export default createPoint10Module({
  name: "Point10 Smoke Room",
  SceneClass: SmokeRoom,
  webgpu: true,
});
