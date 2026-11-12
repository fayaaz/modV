// WS-14 — restart-budget accounting for the engine supervisor (review R-24).
//
// Kept pure so the budget/window semantics are unit-testable without spawning
// an engine. The supervisor stores the returned timestamps on itself and
// clears them on a clean `start()`/`shutdown()` so a stop/start cycle never
// inherits the previous crash budget.

const RESTART_BUDGET = 5;
const RESTART_WINDOW_MS = 60_000;

/**
 * Record one restart attempt, pruning attempts that fell out of the window.
 *
 * The budget means "more than N attempts inside the window is fatal": with
 * `RESTART_BUDGET = 5`, the sixth attempt exhausts it. This mirrors the
 * supervisor's original `length > restartBudget` check.
 *
 * @param {number[] | null | undefined} timestamps previous attempt times
 * @param {number} [now] current time (injectable for tests)
 * @param {{ budget?: number, windowMs?: number }} [options]
 * @returns {{ timestamps: number[], count: number, exhausted: boolean }}
 */
function recordRestartAttempt(
  timestamps,
  now = Date.now(),
  { budget = RESTART_BUDGET, windowMs = RESTART_WINDOW_MS } = {},
) {
  const previous = Array.isArray(timestamps) ? timestamps : [];
  const recent = previous.filter(
    (timestamp) => Number.isFinite(timestamp) && now - timestamp < windowMs,
  );
  const next = [...recent, now];

  return {
    timestamps: next,
    count: next.length,
    exhausted: next.length > budget,
  };
}

/** A clean stop/start starts a fresh crash budget. */
function resetRestartBudget() {
  return [];
}

export {
  RESTART_BUDGET,
  RESTART_WINDOW_MS,
  recordRestartAttempt,
  resetRestartBudget,
};
