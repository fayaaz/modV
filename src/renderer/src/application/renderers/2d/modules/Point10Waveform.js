import { createPoint10Module } from "../../../point10/adapter.js";
import { Waveform } from "../../../point10/scenes/Waveform.js";

export default createPoint10Module({
  name: "Point10 Waveform",
  SceneClass: Waveform,
});
