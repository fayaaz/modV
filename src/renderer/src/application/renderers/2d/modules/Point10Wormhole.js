import { createPoint10Module } from "../../../point10/adapter.js";
import { Wormhole } from "../../../point10/scenes/Wormhole.js";

export default createPoint10Module({
  name: "Point10 Wormhole",
  SceneClass: Wormhole,
});
