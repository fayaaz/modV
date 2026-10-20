import { createPoint10Module } from "../../../point10/adapter.js";
import { DvdRoom } from "../../../point10/scenes/DvdRoom.js";

export default createPoint10Module({
  name: "Point10 DVD Room",
  SceneClass: DvdRoom,
  webgpu: true,
});
