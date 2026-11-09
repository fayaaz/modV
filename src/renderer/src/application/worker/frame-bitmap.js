// A transferred ImageBitmap is owned by the worker. It must be closed once it
// has been drawn, otherwise every frame (60 fps per camera) leaks until the
// worker is restarted.
export function drawBitmapAndClose(context, bitmap) {
  if (!bitmap) {
    return false;
  }

  try {
    if (context) {
      context.drawImage(bitmap, 0, 0);
    }
  } finally {
    try {
      bitmap.close?.();
    } catch (e) {
      // The bitmap may already be closed.
    }
  }

  return true;
}
