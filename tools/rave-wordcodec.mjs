export const WORD_COUNT = 157;

const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const PUNCTUATION = {
  " ": 36,
  "!": 37,
  "-": 38,
  "'": 39,
  ".": 41,
  "/": 42,
  ":": 43,
  "+": 44,
  "*": 45,
  "?": 46,
  ",": 47,
};

const GLYPHS = new Map([...ALPHABET].map((char, index) => [char, index]));
for (const [char, index] of Object.entries(PUNCTUATION)) {
  GLYPHS.set(char, index);
}

const SPACE = glyphIndex(" ");

function float(value) {
  if (!Number.isInteger(value)) {
    throw new Error(`Expected an integer value, got ${value}`);
  }
  return `${value}.0`;
}

export function glyphIndex(char) {
  const index = GLYPHS.get(char);
  if (index === undefined) {
    throw new Error(`Unsupported character "${char}"`);
  }
  return index;
}

export function normalizeWord(word) {
  if (typeof word !== "string" || word.length === 0) {
    throw new Error("Word is empty");
  }

  const upper = word.toUpperCase();
  for (const char of upper) glyphIndex(char);
  return upper;
}

export function parseWordFile(text) {
  const words = new Array(WORD_COUNT).fill(null);
  const lines = String(text).split(/\r?\n/);

  for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
    const line = lines[lineIndex].trim();
    if (line === "" || line.startsWith("#")) continue;

    const match = line.match(/^(\d+)\s*:\s*(.*)$/);
    if (!match) {
      throw new Error(`Line ${lineIndex + 1}: expected "<index>: <word>"`);
    }

    const index = Number(match[1]);
    if (!Number.isInteger(index) || index < 0 || index >= WORD_COUNT) {
      throw new Error(
        `Line ${lineIndex + 1}: index ${index} is out of range 0-${WORD_COUNT - 1}`,
      );
    }
    if (index === 155) {
      throw new Error(
        `Line ${lineIndex + 1}: word 155 is generated from the DJ ticker (80-126)`,
      );
    }

    let value = match[2];
    if (value.startsWith('"')) {
      if (!value.endsWith('"')) {
        throw new Error(`Line ${lineIndex + 1}: unterminated quoted word`);
      }
      value = value.slice(1, -1);
    } else {
      value = value.trim();
    }

    if (words[index] !== null) {
      throw new Error(`Line ${lineIndex + 1}: Duplicate index ${index}`);
    }

    words[index] = normalizeWord(value);
  }

  for (let index = 0; index < WORD_COUNT; index++) {
    if (index === 155) continue;
    if (words[index] === null) {
      throw new Error(`Missing word index ${index}`);
    }
  }

  words[155] = null;
  return words;
}

export function packLeaf(codes) {
  if (!Array.isArray(codes) || codes.length !== 16) {
    throw new Error("packLeaf expects exactly 16 glyph indices");
  }

  const packed = [];
  for (let component = 0; component < 4; component++) {
    const offset = component * 4;
    const value =
      codes[offset] +
      codes[offset + 1] * 64 +
      codes[offset + 2] * 4096 +
      codes[offset + 3] * 262144;

    if (!Number.isInteger(value) || value < 0 || value > 16777215) {
      throw new Error("Packed glyph value is out of range");
    }
    packed.push(value);
  }
  return packed;
}

function buildLeaves(codes) {
  const leafCount = Math.ceil(codes.length / 15);
  const leaves = [];

  for (let leaf = 0; leaf < leafCount; leaf++) {
    const glyphs = [];
    for (let offset = 0; offset < 16; offset++) {
      glyphs.push(codes[leaf * 15 + offset] ?? 0);
    }
    leaves.push(packLeaf(glyphs));
  }

  return leaves;
}

export function buildTxtLeafBody(leaves) {
  const lines = [];

  function emit(lo, hi, indent) {
    if (lo === hi) {
      const [a, b, c, d] = leaves[lo];
      lines.push(
        `${indent}return vec4(${float(a)}, ${float(b)}, ${float(c)}, ${float(d)});`,
      );
      return;
    }

    const mid = Math.floor((lo + hi) / 2);
    lines.push(`${indent}if (k < ${float(mid + 1)}) {`);
    emit(lo, mid, `${indent}    `);
    lines.push(`${indent}} else {`);
    emit(mid + 1, hi, `${indent}    `);
    lines.push(`${indent}}`);
  }

  emit(0, leaves.length - 1, "    ");
  return lines.join("\n");
}

export function buildWordPairBody(records) {
  const pairs = [];

  for (let pair = 0; pair < Math.ceil(WORD_COUNT / 2); pair++) {
    const even = records[pair * 2];
    const odd = records[pair * 2 + 1] ?? { start: 0, len: 1 };
    pairs.push([even.start, even.len, odd.start, odd.len]);
  }

  const lines = [];

  function emit(lo, hi, indent) {
    if (lo === hi) {
      const [a, b, c, d] = pairs[lo];
      lines.push(
        `${indent}return vec4(${float(a)}, ${float(b)}, ${float(c)}, ${float(d)});`,
      );
      return;
    }

    const mid = Math.floor((lo + hi) / 2);
    lines.push(`${indent}if (j < ${float(mid + 1)}) {`);
    emit(lo, mid, `${indent}    `);
    lines.push(`${indent}} else {`);
    emit(mid + 1, hi, `${indent}    `);
    lines.push(`${indent}}`);
  }

  emit(0, pairs.length - 1, "    ");
  return lines.join("\n");
}

export function buildQuadMaxBody(words) {
  const maxima = [];
  for (let phrase = 0; phrase < 8; phrase++) {
    const start = 24 + phrase * 4;
    const lengths = words.slice(start, start + 4).map((word) => word.length);
    maxima.push(Math.max(...lengths));
  }

  const lines = ["    float q = float(qi);"];

  function emit(lo, hi, indent) {
    if (lo === hi) {
      lines.push(`${indent}return ${float(maxima[lo])};`);
      return;
    }

    const mid = Math.floor((lo + hi) / 2);
    lines.push(`${indent}if (q < ${float(mid + 1)}) {`);
    emit(lo, mid, `${indent}    `);
    lines.push(`${indent}} else {`);
    emit(mid + 1, hi, `${indent}    `);
    lines.push(`${indent}}`);
  }

  emit(0, maxima.length - 1, "    ");
  return lines.join("\n");
}

export function encodeWords(words) {
  if (!Array.isArray(words) || words.length !== WORD_COUNT) {
    throw new Error(`Expected an array of ${WORD_COUNT} words`);
  }

  const codes = [];
  const records = new Array(WORD_COUNT);

  function appendWord(index) {
    const word = words[index];
    if (word === null || word === undefined) {
      throw new Error(`Missing word index ${index}`);
    }

    const start = codes.length;
    for (const char of word) codes.push(glyphIndex(char));
    records[index] = { start, len: word.length };
  }

  for (let index = 0; index <= 79; index++) appendWord(index);

  const lineupStart = codes.length;
  for (let index = 80; index <= 126; index++) {
    appendWord(index);
    codes.push(SPACE, SPACE);
  }
  records[155] = { start: lineupStart, len: codes.length - lineupStart };
  codes.push(glyphIndex(words[80][0]));

  for (let index = 127; index <= 150; index++) appendWord(index);

  for (let index = 151; index <= 154; index++) {
    appendWord(index);
    codes.push(glyphIndex(words[index][0]));
  }

  appendWord(156);

  const leaves = buildLeaves(codes);
  return {
    codes,
    records,
    leaves,
    txtLeafBody: buildTxtLeafBody(leaves),
    wordPairBody: buildWordPairBody(records),
    quadMaxBody: buildQuadMaxBody(words),
  };
}

export function replaceFunction(source, name, body) {
  const signature = new RegExp(
    `\\b(?:vec2|vec4|float)\\s+${name}\\s*\\([^)]*\\)\\s*\\{`,
  );
  const match = signature.exec(source);
  if (!match) {
    throw new Error(`Could not find ${name}(...) in shader`);
  }

  const start = match.index;
  const openBrace = source.indexOf("{", start);
  let depth = 0;
  let end = -1;

  for (let index = openBrace; index < source.length; index++) {
    const char = source[index];
    if (char === "{") {
      depth++;
    } else if (char === "}") {
      depth--;
      if (depth === 0) {
        end = index;
        break;
      }
    }
  }

  if (end === -1) {
    throw new Error(`Unclosed ${name}(...) in shader`);
  }

  const header = source.slice(start, openBrace).trimEnd();
  const replacement = `${header} {\n${body}\n}`;
  return source.slice(0, start) + replacement + source.slice(end + 1);
}

function hasFunction(source, name) {
  return new RegExp(
    `\\b(?:vec2|vec4|float)\\s+${name}\\s*\\([^)]*\\)\\s*\\{`,
  ).test(source);
}

export function reencodeShader(source, words) {
  const { txtLeafBody, wordPairBody, quadMaxBody } = encodeWords(words);
  let output = replaceFunction(source, "txtLeaf", txtLeafBody);
  output = replaceFunction(output, "wordPair", wordPairBody);
  if (hasFunction(output, "quadMax")) {
    output = replaceFunction(output, "quadMax", quadMaxBody);
  }
  return output;
}
