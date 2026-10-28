// Devices report their capture abilities through MediaStreamTrack
// getCapabilities(). Chromium usually exposes the width/height/frameRate as a
// min/max range (or, for some drivers, an explicit list of values), rather than
// a discrete mode list. Build the selectable options from that report so the
// picker only offers modes the active camera actually advertises.

export const RESOLUTION_PRESETS = Object.freeze([
  { width: 176, height: 144 },
  { width: 320, height: 240 },
  { width: 352, height: 288 },
  { width: 640, height: 360 },
  { width: 640, height: 480 },
  { width: 800, height: 600 },
  { width: 960, height: 540 },
  { width: 1024, height: 768 },
  { width: 1280, height: 720 },
  { width: 1280, height: 960 },
  { width: 1366, height: 768 },
  { width: 1600, height: 900 },
  { width: 1600, height: 1200 },
  { width: 1920, height: 1080 },
  { width: 2048, height: 1536 },
  { width: 2560, height: 1440 },
  { width: 2592, height: 1944 },
  { width: 3264, height: 2448 },
  { width: 3840, height: 2160 },
  { width: 4096, height: 2160 },
]);

export const FRAME_RATE_PRESETS = Object.freeze([
  15, 20, 24, 25, 30, 48, 50, 60, 72, 90, 120, 144, 240,
]);

function toNumber(value) {
  const number = Number(value);

  return Number.isFinite(number) ? number : null;
}

function readRange(value) {
  if (Array.isArray(value)) {
    const values = [
      ...new Set(value.map(toNumber).filter((entry) => entry !== null)),
    ].sort((a, b) => a - b);

    return {
      known: values.length > 0,
      values,
      min: values.length ? values[0] : null,
      max: values.length ? values[values.length - 1] : null,
    };
  }

  const number = toNumber(value);

  if (number !== null) {
    return { known: true, values: [], min: number, max: number };
  }

  if (value && typeof value === "object") {
    const min = toNumber(value.min);
    const max = toNumber(value.max);
    const known = min !== null || max !== null;

    return {
      known,
      values: [],
      min: min ?? max,
      max: max ?? min,
    };
  }

  return { known: false, values: [], min: null, max: null };
}

function isInRange(value, range) {
  if (!range.known) {
    return true;
  }

  if (range.values.length) {
    return range.values.includes(value);
  }

  if (range.min !== null && value < range.min) {
    return false;
  }

  if (range.max !== null && value > range.max) {
    return false;
  }

  return true;
}

function resolutionKey({ width, height }) {
  return `${width}x${height}`;
}

function addResolution(map, width, height) {
  const w = Math.round(toNumber(width) ?? 0);
  const h = Math.round(toNumber(height) ?? 0);

  if (w <= 0 || h <= 0) {
    return;
  }

  const resolution = { width: w, height: h };

  map.set(resolutionKey(resolution), resolution);
}

export function buildResolutionOptions(capabilities = {}, settings = {}) {
  const widthRange = readRange(capabilities.width);
  const heightRange = readRange(capabilities.height);
  const resolutions = new Map();

  for (const preset of RESOLUTION_PRESETS) {
    if (
      isInRange(preset.width, widthRange) &&
      isInRange(preset.height, heightRange)
    ) {
      addResolution(resolutions, preset.width, preset.height);
    }
  }

  if (settings.width && settings.height) {
    addResolution(resolutions, settings.width, settings.height);
  }

  if (widthRange.max !== null && heightRange.max !== null) {
    addResolution(resolutions, widthRange.max, heightRange.max);
  }

  return [...resolutions.values()].sort((a, b) => {
    const area = a.width * a.height - b.width * b.height;

    return area || a.width - b.width;
  });
}

function addFrameRate(map, value) {
  const frameRate = Math.round(toNumber(value) ?? 0);

  if (frameRate > 0) {
    map.set(frameRate, frameRate);
  }
}

export function buildFrameRateOptions(capabilities = {}, settings = {}) {
  const frameRateRange = readRange(capabilities.frameRate);
  const frameRates = new Map();

  for (const preset of FRAME_RATE_PRESETS) {
    if (isInRange(preset, frameRateRange)) {
      addFrameRate(frameRates, preset);
    }
  }

  if (frameRateRange.values.length) {
    for (const value of frameRateRange.values) {
      addFrameRate(frameRates, value);
    }
  } else if (frameRateRange.max !== null) {
    addFrameRate(frameRates, frameRateRange.max);
  }

  addFrameRate(frameRates, settings.frameRate);

  return [...frameRates.values()]
    .sort((a, b) => a - b)
    .map((value) => ({ value, label: `${value} fps` }));
}

export function buildVideoOptions(capabilities = {}, settings = {}) {
  return {
    resolutions: buildResolutionOptions(capabilities, settings),
    frameRates: buildFrameRateOptions(capabilities, settings),
  };
}
