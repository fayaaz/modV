/* eslint-env worker */

const MAX_ROW_CANVASES = 16;
const rowCanvases = new Map();

function getScratchCanvas(rowIndex, width, height) {
  const key = Number(rowIndex);
  let entry = rowCanvases.get(key);

  if (entry) {
    // Refresh recency so the cache evicts rows that are no longer fading.
    rowCanvases.delete(key);
  } else {
    const canvas = new OffscreenCanvas(width, height);

    entry = { canvas, context: canvas.getContext("2d") };
  }

  if (entry.canvas.width !== width || entry.canvas.height !== height) {
    entry.canvas.width = width;
    entry.canvas.height = height;
  }

  rowCanvases.set(key, entry);

  while (rowCanvases.size > MAX_ROW_CANVASES) {
    const oldestKey = rowCanvases.keys().next().value;

    rowCanvases.delete(oldestKey);
  }

  return entry;
}

/**
 * Composites an outgoing row canvas over the incoming one for a row-level
 * Scene change. Each row reuses a scratch canvas so a fade does not allocate
 * per frame.
 */
export function renderRowFade({
  fromCanvas,
  toCanvas,
  progress,
  width,
  height,
  rowIndex,
  fromAlpha = 1,
}) {
  if (
    !toCanvas ||
    !(width > 0) ||
    !(height > 0) ||
    !Number.isFinite(Number(rowIndex))
  ) {
    return toCanvas || fromCanvas || null;
  }

  const numericProgress = Number(progress);
  const clamped = Number.isFinite(numericProgress)
    ? Math.min(1, Math.max(0, numericProgress))
    : 0;
  const eased = clamped * clamped * (3 - 2 * clamped);
  const numericFromAlpha = Number(fromAlpha);
  const alpha = Number.isFinite(numericFromAlpha)
    ? Math.min(1, Math.max(0, numericFromAlpha))
    : 1;

  const { canvas, context } = getScratchCanvas(rowIndex, width, height);

  context.clearRect(0, 0, width, height);
  context.globalCompositeOperation = "source-over";

  if (fromCanvas) {
    context.save();
    context.globalAlpha = alpha;
    context.drawImage(fromCanvas, 0, 0, width, height);
    context.restore();
  }

  context.save();
  context.globalAlpha = eased;
  context.drawImage(toCanvas, 0, 0, width, height);
  context.restore();

  context.globalAlpha = 1;
  context.globalCompositeOperation = "source-over";

  return canvas;
}
