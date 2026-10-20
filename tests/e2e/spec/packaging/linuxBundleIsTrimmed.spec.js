import fs from "fs";
import path from "path";
import { expect, test } from "@playwright/test";
import { modVApp } from "../../pageObjectModel";

test("the Linux bundle omits macOS-only and build-only payloads", () => {
  const { buildDir } = modVApp;
  const unpacked = path.join(
    buildDir,
    "resources",
    "app.asar.unpacked",
    "node_modules",
  );

  expect(fs.existsSync(path.join(unpacked, "hapticjs", "hapticJS"))).toBe(
    false,
  );
  expect(fs.existsSync(path.join(unpacked, "grandiose", "ndi"))).toBe(false);

  expect(
    fs.existsSync(
      path.join(unpacked, "grandiose", "build", "Release", "grandiose.node"),
    ),
  ).toBe(true);
  expect(
    fs.existsSync(
      path.join(unpacked, "grandiose", "build", "Release", "libndi.so.5.1.1"),
    ),
  ).toBe(true);
  expect(
    fs.existsSync(
      path.join(unpacked, "grandiose", "build", "Release", "libndi.so.5"),
    ),
  ).toBe(true);

  expect(fs.existsSync(path.join(buildDir, "locales", "en-US.pak"))).toBe(true);
  expect(fs.existsSync(path.join(buildDir, "locales", "ja.pak"))).toBe(false);
});
