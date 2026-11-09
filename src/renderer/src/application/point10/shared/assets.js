// Worker-safe binary asset loading for the Point10 scenes.
//
// The scenes run in modV's worker, where three's FileLoader is unreliable for
// bundled assets: `fetch()` works against the Vite dev server but the packaged
// app serves the worker from `file://`, which Chromium will not fetch. Vite
// emits imported assets as `?url` strings, so this helper reads those the same
// way `textures.js` does: `fs` for `file://`, `fetch` otherwise.
const fs = require("fs");
const { fileURLToPath } = require("url");

async function readAssetBuffer(url) {
  const abs = new URL(url, import.meta.url);

  if (abs.protocol === "file:") {
    const buffer = fs.readFileSync(fileURLToPath(abs));

    return buffer.buffer.slice(
      buffer.byteOffset,
      buffer.byteOffset + buffer.byteLength,
    );
  }

  const response = await fetch(abs.href);

  if (!response.ok) {
    throw new Error(`Point10 asset "${url}": ${response.status}`);
  }

  return await response.arrayBuffer();
}

export { readAssetBuffer };
