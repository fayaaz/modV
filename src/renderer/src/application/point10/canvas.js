// modV runs its render loop in a worker, where `document` does not exist.
// Vendored Point10 scenes that need a 2D canvas (particle sprites, generated
// textures) go through this helper instead of document.createElement("canvas").
function createCanvas(width, height) {
  if (typeof OffscreenCanvas !== "undefined") {
    return new OffscreenCanvas(width, height);
  }

  if (typeof document !== "undefined") {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    return canvas;
  }

  throw new Error("No canvas implementation available");
}

export { createCanvas };
