import { createPoint10Module } from "../../../point10/adapter.js";
import { Ferrofluid } from "../../../point10/scenes/Ferrofluid.js";

export default createPoint10Module({
  name: "Point10 Ferrofluid",
  SceneClass: Ferrofluid,
});
