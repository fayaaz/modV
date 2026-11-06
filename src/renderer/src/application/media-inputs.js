export function isLiveVideoTrack(track) {
  return !!(track && track.readyState === "live");
}

export function shouldReconnectVideoCapture(capture) {
  return !capture || !isLiveVideoTrack(capture.track);
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
