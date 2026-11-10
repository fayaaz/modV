import { describe, expect, it } from "vitest";
import fs from "fs";
import os from "os";
import path from "path";
import { writeFileAtomic } from "../../src/renderer/src/application/worker/atomic-write.cjs";

describe("writeFileAtomic", () => {
  it("writes the contents of a new file", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "vmodv-atomic-write-"));
    const filePath = path.join(dir, "scenes.json");

    try {
      await writeFileAtomic(filePath, '{"ok":true}');
      expect(fs.readFileSync(filePath, "utf8")).toBe('{"ok":true}');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("replaces an existing file with complete contents", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "vmodv-atomic-write-"));
    const filePath = path.join(dir, "scenes.json");

    try {
      fs.writeFileSync(filePath, '{"old":"a much longer previous payload"}');

      await writeFileAtomic(filePath, '{"ok":true}');

      expect(fs.readFileSync(filePath, "utf8")).toBe('{"ok":true}');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("leaves no temporary files behind", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "vmodv-atomic-write-"));
    const filePath = path.join(dir, "scenes.json");

    try {
      await writeFileAtomic(filePath, '{"ok":true}');
      const leftovers = fs
        .readdirSync(dir)
        .filter((name) => name !== "scenes.json");

      expect(leftovers).toEqual([]);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("creates missing parent directories", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "vmodv-atomic-write-"));
    const filePath = path.join(dir, "nested", "deep", "scenes.json");

    try {
      await writeFileAtomic(filePath, '{"ok":true}');
      expect(fs.readFileSync(filePath, "utf8")).toBe('{"ok":true}');
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
