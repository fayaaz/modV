const LOG_LEVELS = ["info", "warn", "error"];

function serialiseData(data) {
  if (data === undefined || data === null) {
    return "";
  }

  if (data instanceof Error) {
    return JSON.stringify({
      name: data.name,
      message: data.message,
      stack: data.stack,
    });
  }

  try {
    const json = JSON.stringify(data);

    return json === undefined ? String(data) : json;
  } catch (e) {
    return JSON.stringify({
      unserialisable: String(data),
      reason: e?.message || String(e),
    });
  }
}

function formatLogLine({ timestamp, level, scope, message, data } = {}) {
  const time =
    timestamp instanceof Date
      ? timestamp.toISOString()
      : String(timestamp ?? "");

  const normalisedLevel = String(level || "info").toUpperCase();
  const normalisedScope = scope ? `[${scope}]` : "[]";
  const normalisedMessage =
    typeof message === "string" ? message : String(message ?? "");
  const serialised = serialiseData(data);

  const line = `${time} ${normalisedLevel} ${normalisedScope} ${normalisedMessage}`;

  return serialised ? `${line} ${serialised}` : line;
}

export { formatLogLine, serialiseData, LOG_LEVELS };
