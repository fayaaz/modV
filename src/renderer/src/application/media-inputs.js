export const VIDEO_CAPTURE_MAX_GRAB_FAILURES = 3;
export const VIDEO_CAPTURE_MUTED_RECONNECT_MS = 5000;

export function isLiveVideoTrack(track) {
  return !!(track && track.readyState === "live");
}

export function shouldReconnectVideoCapture(
  capture,
  { allowMuted = false } = {},
) {
  if (!capture || !isLiveVideoTrack(capture.track)) {
    return true;
  }

  return allowMuted && !!capture.track.muted;
}

// A plain deviceId is only an "ideal" constraint, so Chromium is free to
// silently open a different camera when the requested one is busy. That makes
// two selected cameras share one physical device, which in turn wedges
// ImageCapture and freezes both feeds. Pin the camera with an exact constraint
// so getUserMedia either opens the requested device or rejects.
export function buildVideoConstraints({
  deviceId,
  resolution,
  frameRate,
} = {}) {
  if (!deviceId) {
    return {};
  }

  const { width, height } = resolution || {};
  const constraints = {
    deviceId: { exact: deviceId },
    frameRate: { ideal: frameRate || 60 },
  };

  if (width && height) {
    constraints.width = { ideal: width };
    constraints.height = { ideal: height };
  }

  return constraints;
}

// Decides what the per-camera capture loop should do next. Recreating the
// ImageCapture (without reopening the device) recovers an object whose
// grabFrame has started rejecting; reopening the stream is the fallback.
export function nextVideoCaptureAction({
  capture,
  consecutiveFailures = 0,
  mutedForMs = null,
  recreateAttempted = false,
} = {}) {
  if (shouldReconnectVideoCapture(capture)) {
    return "reconnect";
  }

  if (capture.track && capture.track.muted) {
    if (
      Number.isFinite(mutedForMs) &&
      mutedForMs >= VIDEO_CAPTURE_MUTED_RECONNECT_MS
    ) {
      return "reconnect";
    }

    return "wait";
  }

  if (consecutiveFailures >= VIDEO_CAPTURE_MAX_GRAB_FAILURES) {
    return recreateAttempted ? "reconnect" : "recreate";
  }

  return "grab";
}

export function describeCaptureError(error) {
  if (error === undefined || error === null) {
    return "unknown";
  }

  if (typeof error === "string") {
    return error;
  }

  if (typeof error === "object") {
    const name = error.name ? String(error.name) : "";
    const message = error.message ? String(error.message) : "";
    const description = [name, message].filter(Boolean).join(": ");

    if (description) {
      return description;
    }

    try {
      return JSON.stringify(error);
    } catch (e) {
      return String(error);
    }
  }

  return String(error);
}

export function restoreAudioSource(savedId, audioDevices, systemAudioId) {
  if (typeof savedId !== "string" || savedId.length === 0) {
    return null;
  }

  if (savedId === systemAudioId) {
    return savedId;
  }

  const devices = Array.isArray(audioDevices) ? audioDevices : [];

  return devices.some((device) => device?.deviceId === savedId)
    ? savedId
    : null;
}

export function restoreVideoSources(savedIds, videoDevices) {
  if (!Array.isArray(savedIds)) {
    return [];
  }

  const devices = Array.isArray(videoDevices) ? videoDevices : [];
  const available = new Set(
    devices.map((device) => device?.deviceId).filter(Boolean),
  );
  const seen = new Set();
  const restored = [];

  for (const id of savedIds) {
    if (typeof id !== "string" || id.length === 0) {
      continue;
    }

    if (!available.has(id) || seen.has(id)) {
      continue;
    }

    seen.add(id);
    restored.push(id);
  }

  return restored;
}

export function selectWebcamSource(videos, index, fallback) {
  const list = Array.isArray(videos)
    ? videos.filter((video) => video != null)
    : [];

  const i = Math.max(1, Math.round(Number(index) || 1));

  return list[i - 1] || list[0] || fallback;
}
