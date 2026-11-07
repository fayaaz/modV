/* eslint-env worker */

const MAX_ROW_CANVASES = 16;
const rowCanvases = new Map();

const clamp01 = (value) => Math.min(1, Math.max(0, value));

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
 * Smoothstep alpha for the outgoing and incoming halves of a row crossfade.
 * Kept separate from canvas drawing so it can be unit-tested.
 */
export function getRowFadeAlphas({
  progress,
  fromAlpha = 1,
  toAlpha = 1,
} = {}) {
  const numericProgress = Number(progress);
  const clamped = Number.isFinite(numericProgress)
    ? clamp01(numericProgress)
    : 0;
  const eased = clamped * clamped * (3 - 2 * clamped);
  const numericFromAlpha = Number(fromAlpha);
  const numericToAlpha = Number(toAlpha);
  const from = Number.isFinite(numericFromAlpha)
    ? clamp01(numericFromAlpha)
    : 1;
  const to = Number.isFinite(numericToAlpha) ? clamp01(numericToAlpha) : 1;

  return {
    progress: clamped,
    eased,
    fromAlpha: from * (1 - eased),
    toAlpha: to * eased,
  };
}

/**
 * Composites the live outgoing (ghost) and incoming row canvases for a
 * row-level Scene change. A row fade must not inherit just the incoming
 * Group's canvas: rows above a fading Group (Foreground, Effects) would then
 * jump to the new visuals immediately and hide the fade. Each row reuses a
 * scratch canvas so the fade does not allocate per frame.
 */
export function renderRowFade({
  fromCanvas,
  toCanvas,
  progress,
  width,
  height,
  rowIndex,
  fromAlpha = 1,
  toAlpha = 1,
}) {
  if (
    !toCanvas ||
    !(width > 0) ||
    !(height > 0) ||
    !Number.isFinite(Number(rowIndex))
  ) {
    return toCanvas || fromCanvas || null;
  }

  const { canvas, context } = getScratchCanvas(rowIndex, width, height);
  const alphas = getRowFadeAlphas({ progress, fromAlpha, toAlpha });

  context.clearRect(0, 0, width, height);
  context.globalCompositeOperation = "source-over";
  context.globalAlpha = 1;

  if (fromCanvas) {
    context.save();
    context.globalAlpha = alphas.fromAlpha;
    context.drawImage(fromCanvas, 0, 0, width, height);
    context.restore();
  }

  if (toCanvas) {
    context.save();
    context.globalAlpha = alphas.toAlpha;
    context.drawImage(toCanvas, 0, 0, width, height);
    context.restore();
  }

  context.globalAlpha = 1;
  context.globalCompositeOperation = "source-over";

  return canvas;
}
