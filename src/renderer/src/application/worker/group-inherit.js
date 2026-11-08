/**
 * Resolve the canvas a Group should inherit at the start of its draw.
 *
 * `inheritFrom: -1` means "the previous Group in render order". There is no
 * previous Group for the first Group of the frame, so it inherits nothing
 * (`null`). It must not fall back to the root output canvas: that canvas still
 * holds the previous frame's composite until the frame is cleared, so feeding
 * it back into a Group makes a transparent scene accumulate trails.
 */
export function resolveInheritCanvas({
  inheritFrom,
  lastCanvas,
  inheritedGroup,
  renderedCanvasByGroupId,
}) {
  if (inheritFrom === -1) {
    return lastCanvas || null;
  }

  if (!inheritedGroup || !inheritedGroup.context) {
    return null;
  }

  return (
    renderedCanvasByGroupId[inheritedGroup.id] ||
    inheritedGroup.context.context.canvas
  );
}
