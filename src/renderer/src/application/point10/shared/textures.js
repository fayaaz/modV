// Texture loading for the WebGPU Point10 scenes.
//
// Those scenes run in modV's worker, where three's TextureLoader is unusable
// (its ImageLoader builds a DOM <img>). Textures ship as Vite assets imported
// with `?url`, so the same code works in both builds:
//   dev  - the URL is served by the Vite dev server -> fetch()
//   prod - the URL is relative to the built worker chunk -> file:// -> fs
// and the decoded ImageBitmap goes straight into a three Texture.
import * as THREE from "three/webgpu";

const fs = require("fs");
const { fileURLToPath } = require("url");

async function loadImageBitmap(url) {
  const abs = new URL(url, import.meta.url);

  if (abs.protocol === "file:") {
    const buffer = fs.readFileSync(fileURLToPath(abs));

    return await createImageBitmap(
      new Blob([new Uint8Array(buffer)], { type: "image/jpeg" }),
    );
  }

  const response = await fetch(abs.href);

  if (!response.ok) {
    throw new Error(`Point10 texture "${url}": ${response.status}`);
  }

  return await createImageBitmap(await response.blob());
}

// Resolves to a THREE.Texture backed by an ImageBitmap. Callers configure the
// texture (colorSpace, wrapping, repeat) exactly as they would with a
// TextureLoader callback.
async function loadTexture(url) {
  const bitmap = await loadImageBitmap(url);
  const texture = new THREE.Texture(bitmap);
  texture.needsUpdate = true;

  return texture;
}

export { loadTexture, loadImageBitmap };
