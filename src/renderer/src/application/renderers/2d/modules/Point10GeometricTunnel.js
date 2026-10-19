import { createPoint10Module } from "../../../point10/adapter.js";
import { GeometricTunnel } from "../../../point10/scenes/GeometricTunnel.js";

export default createPoint10Module({
  name: "Point10 Geometric Tunnel",
  SceneClass: GeometricTunnel,
});
