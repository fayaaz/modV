import { createPoint10Module } from "../../../point10/adapter.js";
import { UltraHDR } from "../../../point10/scenes/UltraHDR.js";

export default createPoint10Module({
  name: "Point10 Ultra HDR",
  SceneClass: UltraHDR,
});
