import fs from "fs";
import path from "path";
import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

const effectsSource = fs.readFileSync(
  path.resolve(
    "src/renderer/src/application/renderers/shader/modules/Point10Effects.js",
  ),
  "utf8",
);

const effectNames = [...effectsSource.matchAll(/effect\(\s*"([^"]+)"/g)].map(
  (match) => `Point10 ${match[1]}`,
);

const isfDirectory = path.resolve(
  "src/renderer/src/application/renderers/isf/modules/isf",
);

const transitionNames = fs
  .readdirSync(isfDirectory)
  .filter(
    (fileName) => fileName.startsWith("Point10 ") && fileName.endsWith(".fs"),
  )
  .map((fileName) => fileName.slice(0, -".fs".length));

test("Point10 effects and transitions are registered", async () => {
  expect(effectNames.length).toBeGreaterThan(0);
  expect(transitionNames.length).toBeGreaterThan(0);

  const registered = await modVApp.evaluateWorkerState("modules.registered");

  for (let i = 0; i < effectNames.length; i++) {
    expect(registered[effectNames[i]], effectNames[i]).toBeTruthy();
  }

  for (let i = 0; i < transitionNames.length; i++) {
    expect(registered[transitionNames[i]], transitionNames[i]).toBeTruthy();
  }
});
