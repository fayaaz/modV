#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, extname, join } from "node:path";

import {
  WORD_COUNT,
  encodeWords,
  parseWordFile,
  reencodeShader,
} from "./rave-wordcodec.mjs";

function usage() {
  console.log(`Usage: node tools/rave-reencode.mjs <words.txt> <rave.fs> [output.fs]

Reads the editable word list and patches the packed txtLeaf/wordPair tables
in an existing rave.exe ISF shader.

  words.txt   editable word list (for example tools/rave-words.txt)
  rave.fs     the shader to patch
  output.fs   defaults to <rave>.reencoded.fs next to the input shader

Run it through the Docker toolchain with:
  make rave-reencode ARGS="tools/rave-words.txt <rave.fs> <output.fs>"`);
}

const args = process.argv.slice(2);

if (args.length === 0 || args.includes("-h") || args.includes("--help")) {
  usage();
  process.exit(args.length === 0 ? 1 : 0);
}

if (args.length < 2 || args.length > 3) {
  usage();
  process.exit(1);
}

const [wordsPath, shaderPath, outputArg] = args;
const extension = extname(shaderPath) || ".fs";
const outputPath =
  outputArg ??
  join(
    dirname(shaderPath),
    `${basename(shaderPath, extname(shaderPath))}.reencoded${extension}`,
  );

try {
  const words = parseWordFile(readFileSync(wordsPath, "utf8"));
  const source = readFileSync(shaderPath, "utf8");
  const output = reencodeShader(source, words);
  const encoded = encodeWords(words);

  writeFileSync(outputPath, output);
  console.log(`Read  ${wordsPath}`);
  console.log(`Read  ${shaderPath}`);
  console.log(`Wrote ${outputPath}`);
  console.log(
    `${encoded.codes.length} characters, ${encoded.leaves.length} text leaves, ${Math.ceil(WORD_COUNT / 2)} word pairs`,
  );
} catch (error) {
  console.error(`rave-reencode: ${error.message}`);
  process.exit(1);
}
