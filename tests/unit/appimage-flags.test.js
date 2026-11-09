import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const builderConfig = readFileSync(
  new URL("../../electron-builder.yml", import.meta.url),
  "utf8",
);

function topLevelSection(name) {
  const lines = builderConfig.split(/\r?\n/);
  const start = lines.findIndex((line) => line.trimEnd() === `${name}:`);

  if (start === -1) {
    return "";
  }

  const body = [];

  for (const line of lines.slice(start + 1)) {
    if (line.trim() !== "" && !line.startsWith(" ")) {
      break;
    }

    body.push(line);
  }

  return body.join("\n");
}

describe("AppImage launch flags", () => {
  it("disables the Chromium sandbox by default", () => {
    const appImageSection = topLevelSection("appImage");

    expect(appImageSection).toMatch(/executableArgs:\s*\n\s*-\s*--no-sandbox/);
  });

  it("leaves the deb sandbox enabled", () => {
    const linuxSection = topLevelSection("linux");

    expect(linuxSection).not.toMatch(/--no-sandbox/);
    expect(topLevelSection("deb")).not.toMatch(/--no-sandbox/);
  });
});
