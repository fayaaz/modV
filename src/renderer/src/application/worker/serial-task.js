/**
 * Serialises async work so a slow task cannot overlap with the next one. Used
 * to keep Scene applications sequential: applying Scene B while Scene A is
 * still rebuilding Groups leaves the old Groups orphaned in the store, and
 * those are then rendered every frame until another transition happens to
 * remove them.
 */
export function createSerialTask() {
  let tail = Promise.resolve();

  return function schedule(task) {
    const run = tail.then(task, task);

    // Keep the chain alive when a task rejects; the rejection is still
    // delivered to the caller through `run`.
    tail = run.catch(() => {});

    return run;
  };
}
