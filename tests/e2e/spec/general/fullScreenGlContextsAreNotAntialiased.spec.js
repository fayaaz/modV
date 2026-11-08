import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

/**
 * The shared ISF, shader and three WebGL contexts only ever draw full-screen
 * quads (ISF/pex) or are used by a THREE renderer configured with
 * antialias:false. Multisampling is pure fill-rate and resolve cost on every
 * pass, so these contexts must request antialias:false.
 */
test("full-screen-quad GL contexts do not request antialiasing", async () => {
  const attributes = await modVApp.evaluateWorker(async () => {
    const result = {};
    const names = [
      "isf-buffer",
      "isf-buffer-gallery",
      "shader-buffer",
      "three-buffer",
    ];

    for (const name of names) {
      const aux = Object.values(self.store.state.outputs.auxillary).find(
        (item) => item && item.name === name,
      );

      if (
        !aux ||
        !aux.context ||
        typeof aux.context.getContextAttributes !== "function"
      ) {
        continue;
      }

      result[name] = aux.context.getContextAttributes();
    }

    return result;
  });

  for (const name of [
    "isf-buffer",
    "isf-buffer-gallery",
    "shader-buffer",
    "three-buffer",
  ]) {
    expect(attributes[name], name).toBeTruthy();
    expect(attributes[name].antialias, name).toBe(false);
  }
});
