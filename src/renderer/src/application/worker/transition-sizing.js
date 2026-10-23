/**
 * Shader transitions composite two full-resolution Scene canvases and upload
 * both to the GPU every frame. At 4K that is a lot of extra fill rate and
 * memory bandwidth on top of rendering both Scenes, which is enough to push a
 * weak GPU into a permanent stall. Rendering the transition into a bounded
 * canvas and scaling it up to the output keeps the effect affordable.
 */
export const MAX_TRANSITION_PIXELS = 1920 * 1080;

function toDimension(value) {
  return Number.isFinite(value) ? Math.max(1, Math.floor(value)) : 1;
}

export function getTransitionRenderSize(
  width,
  height,
  maxPixels = MAX_TRANSITION_PIXELS,
) {
  const sourceWidth = toDimension(width);
  const sourceHeight = toDimension(height);
  const sourcePixels = sourceWidth * sourceHeight;
  const budget =
    Number.isFinite(maxPixels) && maxPixels > 0 ? maxPixels : sourcePixels;

  if (sourcePixels <= budget) {
    return {
      width: sourceWidth,
      height: sourceHeight,
      scaled: false,
    };
  }

  const scale = Math.sqrt(budget / sourcePixels);

  return {
    width: Math.max(1, Math.floor(sourceWidth * scale)),
    height: Math.max(1, Math.floor(sourceHeight * scale)),
    scaled: true,
  };
}
