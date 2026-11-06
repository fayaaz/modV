import map from "../utils/map";

/**
 * Resolves an Input Link's raw source value into the value written to the
 * linked input. Audio features are undefined until the first audio frame
 * arrives (and stay undefined when no input is selected), so a ranged link
 * must not turn that into NaN and overwrite the input's saved value.
 */
export function resolveInputLinkValue(value, min, max) {
  if (value === undefined || value === null) {
    return undefined;
  }

  const resolved =
    min != null || max != null ? map(value, 0, 1, min, max) : value;

  if (typeof resolved === "number" && !Number.isFinite(resolved)) {
    return undefined;
  }

  return resolved;
}
