import { existsSync, readFileSync, readdirSync } from "node:fs";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const SRC = join(ROOT, "src");
const TESTS = join(ROOT, "tests");
const THIS_FILE = fileURLToPath(import.meta.url);

const REMOVED_FILES = [
  "src/renderer/src/application/point10/scenes/DiscoBall.js",
  "src/renderer/src/application/renderers/2d/modules/Point10DiscoBall.js",
];

const FORBIDDEN = ["DiscoBall"];

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);

    return entry.isDirectory() ? walk(path) : [path];
  });
}

describe("Point10 disco ball removal", () => {
  it("removes the module and scene files", () => {
    for (const relativePath of REMOVED_FILES) {
      expect(existsSync(join(ROOT, relativePath)), relativePath).toBe(false);
    }
  });

  it("leaves no source or test reference behind", () => {
    const offenders = [];

    for (const path of [...walk(SRC), ...walk(TESTS)]) {
      if (path === THIS_FILE || !["", ".js", ".vue"].includes(extname(path))) {
        continue;
      }

      const contents = readFileSync(path, "utf8");

      for (const needle of FORBIDDEN) {
        if (contents.includes(needle)) {
          offenders.push(`${path.slice(ROOT.length + 1)}: ${needle}`);
        }
      }
    }

    expect(offenders).toEqual([]);
  });
});
