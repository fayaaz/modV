import { createPoint10Module } from "../../../point10/adapter.js";
import { Mycelium } from "../../../point10/scenes/Mycelium.js";

export default createPoint10Module({
  name: "Point10 Mycelium",
  SceneClass: Mycelium,
  meta: { category: "2d-scenes" },
});
