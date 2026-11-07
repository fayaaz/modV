import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, it, expect } from "vitest";

import {
  WORD_COUNT,
  glyphIndex,
  normalizeWord,
  parseWordFile,
  encodeWords,
  packLeaf,
  buildQuadMaxBody,
  replaceFunction,
  reencodeShader,
} from "../../tools/rave-wordcodec.mjs";

function words(overrides = {}) {
  const list = Array.from({ length: WORD_COUNT }, (_, index) =>
    Object.prototype.hasOwnProperty.call(overrides, index)
      ? overrides[index]
      : "A",
  );
  list[155] = null;
  return list;
}

function wordFile(lines) {
  return ["# test words", "", ...lines].join("\n");
}

function allLines(overrides = {}) {
  return Array.from({ length: WORD_COUNT }, (_, index) => {
    if (index === 155) return null;
    const value = Object.prototype.hasOwnProperty.call(overrides, index)
      ? overrides[index]
      : "A";
    return `${index}: ${value}`;
  }).filter(Boolean);
}

function decode(codes) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  const punctuation = {
    36: " ",
    37: "!",
    38: "-",
    39: "'",
    41: ".",
    42: "/",
    43: ":",
    44: "+",
    45: "*",
    46: "?",
    47: ",",
  };
  return codes
    .map((code) => alphabet[code] ?? punctuation[code] ?? "?")
    .join("");
}

describe("rave word codec", () => {
  it("maps the supported 5x5 font glyphs", () => {
    expect(glyphIndex("A")).toBe(0);
    expect(glyphIndex("Z")).toBe(25);
    expect(glyphIndex("0")).toBe(26);
    expect(glyphIndex("9")).toBe(35);
    expect(glyphIndex(" ")).toBe(36);
    expect(glyphIndex("'")).toBe(39);
    expect(glyphIndex("+")).toBe(44);
    expect(glyphIndex("*")).toBe(45);
    expect(() => glyphIndex("&")).toThrow(/Unsupported character/);
  });

  it("normalises words and rejects unsupported characters", () => {
    expect(normalizeWord("no sleep")).toBe("NO SLEEP");
    expect(normalizeWord("ACID")).toBe("ACID");
    expect(() => normalizeWord("")).toThrow(/empty/);
    expect(() => normalizeWord("A&B")).toThrow(/Unsupported character "&"/);
  });

  it("parses indexed words, comments, blank lines and quoted spaces", () => {
    const parsed = parseWordFile(
      wordFile(
        allLines({
          0: "RAVE",
          151: '"NO SLEEP TILL SUNRISE * RAVE ON * "',
        }),
      ),
    );

    expect(parsed).toHaveLength(WORD_COUNT);
    expect(parsed[0]).toBe("RAVE");
    expect(parsed[151]).toBe("NO SLEEP TILL SUNRISE * RAVE ON * ");
    expect(parsed[155]).toBeNull();
  });

  it("rejects duplicate, missing and malformed word entries", () => {
    const duplicate = allLines();
    duplicate.push("0: AGAIN");
    expect(() => parseWordFile(wordFile(duplicate))).toThrow(/Duplicate/);

    expect(() => parseWordFile(wordFile(allLines().slice(1)))).toThrow(
      /Missing word/,
    );

    expect(() =>
      parseWordFile(wordFile([...allLines().slice(1), "not a word"])),
    ).toThrow(/expected "<index>: <word>"/);

    expect(() =>
      parseWordFile(wordFile([...allLines(), "155: TICKER"])),
    ).toThrow(/155 is generated/);
  });

  it("packs sixteen six-bit glyphs into four exact floats", () => {
    expect(packLeaf(Array.from({ length: 16 }, (_, index) => index))).toEqual([
      794688, 1859908, 2925128, 3990348,
    ]);
  });

  it("lays out the DJ ticker, marquees and NO SLEEP word", () => {
    const encoded = encodeWords(
      words({
        80: "DJ ONE",
        81: "DJ TWO",
        126: "MC LAST",
        151: "MARQ * ",
        156: "NO SLEEP",
      }),
    );

    expect(encoded.records[80]).toEqual({ start: 80, len: 6 });
    expect(encoded.records[81]).toEqual({ start: 88, len: 6 });

    const ticker = encoded.records[155];
    expect(ticker.start).toBe(80);
    const tickerText = decode(
      encoded.codes.slice(ticker.start, ticker.start + ticker.len),
    );
    expect(tickerText.startsWith("DJ ONE  DJ TWO  ")).toBe(true);
    expect(tickerText.endsWith("MC LAST  ")).toBe(true);

    expect(encoded.codes[ticker.start + ticker.len]).toBe(glyphIndex("D"));

    const marquee = encoded.records[151];
    expect(marquee.len).toBe("MARQ * ".length);
    expect(encoded.codes[marquee.start + marquee.len]).toBe(glyphIndex("M"));

    expect(encoded.records[156]).toEqual({
      start: encoded.codes.length - "NO SLEEP".length,
      len: "NO SLEEP".length,
    });
  });

  it("recomputes quadMax from the four words in each quad phrase", () => {
    const body = buildQuadMaxBody(
      words({
        24: "SUPERCALIFRAGILISTIC",
      }),
    );

    expect(body).toContain("float q = float(qi);");
    expect(body).toContain("return 20.0;");
    expect(body.match(/return (\d+)\.0;/)[1]).toBe("20");
  });

  it("parses the shipped rave word list", () => {
    const path = fileURLToPath(
      new URL("../../tools/rave-words.txt", import.meta.url),
    );
    const parsed = parseWordFile(readFileSync(path, "utf8"));

    expect(parsed[0]).toBe("RAVE");
    expect(parsed[155]).toBeNull();
    expect(parsed[156]).toBe("NO SLEEP");
  });

  it("generates a balanced txtLeaf tree and a wordPair table", () => {
    const encoded = encodeWords(words());
    const txtLeaf = encoded.txtLeafBody;
    const wordPair = encoded.wordPairBody;

    expect(txtLeaf).toContain("if (k < ");
    expect(txtLeaf.match(/return vec4\(/g)).toHaveLength(encoded.leaves.length);
    expect(wordPair).toContain("if (j < ");
    expect(wordPair.match(/return vec4\(/g)).toHaveLength(
      Math.ceil(WORD_COUNT / 2),
    );
  });

  it("replaces only the requested function body", () => {
    const source = [
      "int before = 1;",
      "vec4 txtLeaf(float k) {",
      "    return vec4(0.0);",
      "}",
      "int after = 2;",
    ].join("\n");

    const output = replaceFunction(
      source,
      "txtLeaf",
      "    return vec4(1.0, 2.0, 3.0, 4.0);",
    );

    expect(output).toContain("int before = 1;");
    expect(output).toContain("int after = 2;");
    expect(output).toContain("return vec4(1.0, 2.0, 3.0, 4.0);");
    expect(output).not.toContain("return vec4(0.0);");
  });

  it("re-encodes both generated functions in a shader", () => {
    const source = [
      "vec4 txtLeaf(float k) {",
      "    return vec4(0.0);",
      "}",
      "vec4 wordPair(float j) {",
      "    return vec4(0.0);",
      "}",
      "float quadMax(int qi) {",
      "    return 8.0;",
      "}",
      "float keep = 1.0;",
    ].join("\n");

    const output = reencodeShader(
      source,
      words({ 24: "SUPERCALIFRAGILISTIC" }),
    );

    expect(output).toContain("vec4 txtLeaf(float k) {");
    expect(output).toContain("vec4 wordPair(float j) {");
    expect(output).toContain("float quadMax(int qi) {");
    expect(output).toContain("return 20.0;");
    expect(output).toContain("float keep = 1.0;");
    expect(output).toMatch(/if \(k < /);
    expect(output).toMatch(/if \(j < /);
  });

  it("throws when a generated function is missing", () => {
    expect(() => reencodeShader("vec4 txtLeaf(float k) { }", words())).toThrow(
      /wordPair/,
    );
  });
});
